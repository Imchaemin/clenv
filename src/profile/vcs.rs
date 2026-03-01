// src/profile/vcs.rs
// Profile version control (git2-based)
//
// Each profile directory is a git repository.
// Uses libgit2 (git2 crate) to implement commit, log, checkout, etc.

use anyhow::{Context, Result};
use git2::{Repository, Signature, Status};
use std::path::PathBuf;

use crate::error::ClenvError;
use crate::profile::manager::CommitSummary;

/// File change status
#[derive(Debug)]
pub struct FileChange {
    pub path: String,
    pub status: String, // "added", "modified", "deleted", "renamed"
}

/// Detailed commit info
#[derive(Debug)]
pub struct CommitInfo {
    pub hash: String,
    pub message: String,
    pub author: String,
    pub date: String,
}

/// Profile version control
pub struct ProfileVcs {
    /// Path to profile directory
    path: PathBuf,
    /// git repository (None if not initialized)
    repo: Option<Repository>,
}

impl ProfileVcs {
    /// Create a new ProfileVcs
    /// Does not error if the repo does not exist (some commands work without a repo)
    pub fn new(path: PathBuf) -> Result<Self> {
        let repo = Repository::open(&path).ok();
        Ok(Self { path, repo })
    }

    /// Initialize the git repository
    /// Takes &mut self so self.repo can be updated after initialization
    /// Allows methods like commit() to be used immediately afterward
    pub fn init(&mut self) -> Result<()> {
        let repo = Repository::init(&self.path).with_context(|| {
            format!(
                "Failed to initialize git repository: {}",
                self.path.display()
            )
        })?;
        self.repo = Some(repo);
        Ok(())
    }

    /// Get a reference to the repository (error if not initialized)
    fn repo(&self) -> Result<&Repository> {
        self.repo.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "No git repository at '{}'. Create a profile first.",
                self.path.display()
            )
        })
    }

    /// List of current changes
    pub fn status(&self) -> Result<Vec<FileChange>> {
        let repo = self.repo()?;

        // Configure git status options
        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(true) // include new files
            .recurse_untracked_dirs(true) // recurse into subdirectories
            .include_ignored(false); // exclude .gitignore'd files

        let statuses = repo.statuses(Some(&mut opts))?;

        let mut changes = Vec::new();

        for entry in statuses.iter() {
            let path = entry.path().unwrap_or("").to_string();
            let status = entry.status();

            // Map each status code to a descriptive string
            let status_str = if status.contains(Status::INDEX_NEW)
                || status.contains(Status::WT_NEW)
            {
                "added"
            } else if status.contains(Status::INDEX_DELETED) || status.contains(Status::WT_DELETED)
            {
                "deleted"
            } else if status.contains(Status::INDEX_RENAMED) || status.contains(Status::WT_RENAMED)
            {
                "renamed"
            } else {
                "modified"
            };

            changes.push(FileChange {
                path,
                status: status_str.to_string(),
            });
        }

        Ok(changes)
    }

    /// Generate a diff of changes
    pub fn diff(&self, range: Option<&str>, name_only: bool) -> Result<String> {
        let repo = self.repo()?;

        let diff = if let Some(range) = range {
            // Parse "v1..v2" or "hash1..hash2" format
            if let Some((from, to)) = range.split_once("..") {
                let from_commit = resolve_reference(repo, from)?;
                let to_commit = resolve_reference(repo, to)?;

                let from_tree = from_commit.tree()?;
                let to_tree = to_commit.tree()?;
                repo.diff_tree_to_tree(Some(&from_tree), Some(&to_tree), None)?
            } else {
                // Single reference — compare with HEAD
                let commit = resolve_reference(repo, range)?;
                let tree = commit.tree()?;
                repo.diff_tree_to_workdir_with_index(Some(&tree), None)?
            }
        } else {
            // Default: compare with the last commit
            if let Ok(head) = repo.head() {
                if let Ok(commit) = head.peel_to_commit() {
                    let tree = commit.tree()?;
                    repo.diff_tree_to_workdir_with_index(Some(&tree), None)?
                } else {
                    // No commit history
                    return Ok("(no commit history)".to_string());
                }
            } else {
                return Ok("(커밋 히스토리 없음)".to_string());
            }
        };

        // Convert diff to string
        let mut output = String::new();

        diff.print(git2::DiffFormat::Patch, |delta, _hunk, line| {
            use git2::DiffLineType;

            if name_only {
                // Output filenames only
                if let Some(path) = delta.new_file().path() {
                    let path_str = path.to_string_lossy();
                    if !output.contains(path_str.as_ref()) {
                        output.push_str(&path_str);
                        output.push('\n');
                    }
                }
            } else {
                // Output full diff
                match line.origin_value() {
                    DiffLineType::Context => output.push(' '),
                    DiffLineType::Addition => output.push('+'),
                    DiffLineType::Deletion => output.push('-'),
                    DiffLineType::FileHeader => output.push_str("--- "),
                    DiffLineType::HunkHeader => {}
                    _ => {}
                }

                if let Ok(content) = std::str::from_utf8(line.content()) {
                    output.push_str(content);
                }
            }

            true // continue processing diff
        })?;

        Ok(output)
    }

    /// Commit changes
    ///
    /// If files is empty, stage all changes and commit.
    pub fn commit(&self, message: &str, files: &[String]) -> Result<String> {
        let repo = self.repo()?;

        // Get the index (staging area)
        let mut index = repo.index()?;

        if files.is_empty() {
            // Stage all changes (git add -A)
            index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
        } else {
            // Stage specific files only
            for file in files {
                index.add_path(std::path::Path::new(file))?;
            }
        }

        index.write()?;

        // Check if there are any changes
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;

        // Create commit author signature
        // Read name/email from git config or fall back to defaults
        let sig = get_signature(repo)?;

        // Find parent commit (None for the first commit)
        let parent_commit = repo.head().ok().and_then(|h| h.peel_to_commit().ok());

        let parents: Vec<&git2::Commit> = parent_commit.iter().collect();

        // Create the commit
        let commit_id = repo.commit(
            Some("HEAD"), // store at refs/heads/HEAD
            &sig,         // author
            &sig,         // committer (same person)
            message,      // commit message
            &tree,        // file tree
            &parents,     // parent commits
        )?;

        Ok(commit_id.to_string())
    }

    /// Retrieve commit history
    pub fn log(&self, limit: usize, file_path: Option<&str>) -> Result<Vec<CommitInfo>> {
        let repo = self.repo()?;

        // Start commit walk from HEAD
        let mut revwalk = repo.revwalk()?;
        revwalk.push_head()?;

        // Reverse chronological order (newest first)
        revwalk.set_sorting(git2::Sort::TIME)?;

        let mut commits = Vec::new();
        let mut count = 0;

        for oid in revwalk {
            if count >= limit {
                break;
            }

            let oid = oid?;
            let commit = repo.find_commit(oid)?;

            // Filter by specific file
            if let Some(file) = file_path {
                if !commit_touches_file(&commit, repo, file)? {
                    continue;
                }
            }

            let author = commit.author();
            let name = author.name().unwrap_or("Unknown");
            let email = author.email().unwrap_or("");

            // Convert timestamp to a readable format
            // Use from_timestamp_secs (from_timestamp is deprecated)
            let timestamp = commit.time().seconds();
            let datetime = chrono::DateTime::<chrono::Utc>::from_timestamp_secs(timestamp)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| timestamp.to_string());

            commits.push(CommitInfo {
                hash: oid.to_string(),
                message: commit.message().unwrap_or("").trim().to_string(),
                author: format!("{} <{}>", name, email),
                date: datetime,
            });

            count += 1;
        }

        Ok(commits)
    }

    /// Summary info for the last commit
    pub fn last_commit(&self) -> Result<CommitSummary> {
        let commits = self.log(1, None)?;
        commits
            .into_iter()
            .next()
            .map(|c| CommitSummary {
                hash: c.hash,
                message: c.message,
                date: c.date,
                author: c.author,
            })
            .ok_or_else(|| anyhow::anyhow!("No commits found"))
    }

    /// Move to a specific version/tag/hash
    pub fn checkout(&self, reference: &str) -> Result<()> {
        let repo = self.repo()?;

        // Parse reference (tag, hash, HEAD~N, etc.)
        let commit = resolve_reference(repo, reference)
            .map_err(|_| ClenvError::VersionNotFound(reference.to_string()))?;

        // Checkout (change filesystem)
        repo.checkout_tree(
            commit.as_object(),
            Some(git2::build::CheckoutBuilder::new().force()),
        )?;

        // Move HEAD to this commit (detached HEAD)
        repo.set_head_detached(commit.id())?;

        Ok(())
    }

    /// Revert a commit
    pub fn revert(&self, target: &str) -> Result<String> {
        let repo = self.repo()?;

        let commit = resolve_reference(repo, target)
            .map_err(|_| ClenvError::VersionNotFound(target.to_string()))?;

        // revert: create a new commit that undoes the changes of the target commit
        // git2's revert only modifies the index, so we must commit manually
        let mut revert_opts = git2::RevertOptions::new();
        repo.revert(&commit, Some(&mut revert_opts))?;

        let sig = get_signature(repo)?;
        let message = format!("Revert \"{}\"", commit.message().unwrap_or("").trim());
        let mut index = repo.index()?;
        index.write()?;
        let tree_id = index.write_tree()?;
        let tree = repo.find_tree(tree_id)?;
        let parent = repo.head()?.peel_to_commit()?;

        let commit_id = repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &[&parent])?;

        Ok(commit_id.to_string())
    }

    /// List of tags
    pub fn list_tags(&self) -> Result<Vec<String>> {
        let repo = self.repo()?;
        let mut tags = Vec::new();

        repo.tag_foreach(|_oid, name| {
            if let Ok(name_str) = std::str::from_utf8(name) {
                // Remove "refs/tags/" prefix
                let tag_name = name_str.trim_start_matches("refs/tags/");
                tags.push(tag_name.to_string());
            }
            true
        })?;

        tags.sort();
        Ok(tags)
    }

    /// Create a tag
    pub fn create_tag(&self, name: &str, message: Option<&str>) -> Result<()> {
        let repo = self.repo()?;

        // Check if already exists
        if repo.find_reference(&format!("refs/tags/{}", name)).is_ok() {
            return Err(ClenvError::TagAlreadyExists(name.to_string()).into());
        }

        let head = repo.head()?.peel_to_commit()?;
        let sig = get_signature(repo)?;

        if let Some(msg) = message {
            // Annotated tag (with message)
            repo.tag(name, head.as_object(), &sig, msg, false)?;
        } else {
            // Lightweight tag (no message)
            repo.tag_lightweight(name, head.as_object(), false)?;
        }

        Ok(())
    }

    /// Delete a tag
    pub fn delete_tag(&self, name: &str) -> Result<()> {
        let repo = self.repo()?;
        repo.tag_delete(name)?;
        Ok(())
    }
}

// ── Helper functions ──────────────────────────────────────────────────────────

/// Resolve a reference string (tag, hash, HEAD~N, etc.) to a commit
fn resolve_reference<'a>(repo: &'a Repository, reference: &str) -> Result<git2::Commit<'a>> {
    // Handle HEAD~N format
    if reference.starts_with("HEAD~") || reference.starts_with("HEAD^") {
        let spec = reference;
        let obj = repo.revparse_single(spec)?;
        return obj.peel_to_commit().map_err(|e| e.into());
    }

    // Try as tag name
    if let Ok(tag_ref) = repo.find_reference(&format!("refs/tags/{}", reference)) {
        if let Ok(commit) = tag_ref.peel_to_commit() {
            return Ok(commit);
        }
    }

    // Try as branch name
    if let Ok(branch_ref) = repo.find_reference(&format!("refs/heads/{}", reference)) {
        if let Ok(commit) = branch_ref.peel_to_commit() {
            return Ok(commit);
        }
    }

    // Try as commit hash (including short hash)
    let obj = repo.revparse_single(reference)?;
    obj.peel_to_commit().map_err(|e| e.into())
}

/// Get signature info from git config
fn get_signature(repo: &Repository) -> Result<Signature<'_>> {
    // Read user.name and user.email from git config
    if let Ok(sig) = repo.signature() {
        return Ok(sig);
    }

    // Fall back to defaults if git config is not available
    Signature::now("clenv", "clenv@local")
        .map_err(|e| anyhow::anyhow!("Failed to create signature: {}", e))
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Initialize repo and commit one file
    fn init_repo_with_commit(dir: &TempDir) -> ProfileVcs {
        let mut vcs = ProfileVcs::new(dir.path().to_path_buf()).unwrap();
        vcs.init().unwrap();
        fs::write(dir.path().join("CLAUDE.md"), "# 초기 설정\n").unwrap();
        vcs.commit("초기 커밋", &[]).unwrap();
        vcs
    }

    // ── Initialization ───────────────────────────────────────────────────────

    #[test]
    fn test_new_without_repo() {
        let dir = TempDir::new().unwrap();
        let vcs = ProfileVcs::new(dir.path().to_path_buf()).unwrap();
        assert!(vcs.repo.is_none());
    }

    #[test]
    fn test_init_creates_git_dir() {
        let dir = TempDir::new().unwrap();
        let mut vcs = ProfileVcs::new(dir.path().to_path_buf()).unwrap();
        assert!(!dir.path().join(".git").exists());
        vcs.init().unwrap();
        assert!(vcs.repo.is_some());
        assert!(dir.path().join(".git").is_dir());
    }

    #[test]
    fn test_no_repo_methods_return_error() {
        let dir = TempDir::new().unwrap();
        let vcs = ProfileVcs::new(dir.path().to_path_buf()).unwrap();
        assert!(vcs.status().is_err());
        assert!(vcs.commit("msg", &[]).is_err());
        assert!(vcs.log(10, None).is_err());
        assert!(vcs.list_tags().is_err());
    }

    // ── Commit ───────────────────────────────────────────────────────────────

    #[test]
    fn test_commit_returns_sha() {
        let dir = TempDir::new().unwrap();
        let mut vcs = ProfileVcs::new(dir.path().to_path_buf()).unwrap();
        vcs.init().unwrap();
        fs::write(dir.path().join("test.md"), "내용\n").unwrap();

        let hash = vcs.commit("테스트 커밋", &[]).unwrap();
        assert_eq!(hash.len(), 40, "SHA-1 해시는 40자여야 함");
    }

    // ── status ───────────────────────────────────────────────────────────────

    #[test]
    fn test_status_clean_after_commit() {
        let dir = TempDir::new().unwrap();
        let vcs = init_repo_with_commit(&dir);
        let changes = vcs.status().unwrap();
        assert!(
            changes.is_empty(),
            "커밋 후 변경사항 없어야 함: {:?}",
            changes.iter().map(|c| &c.path).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_status_shows_new_untracked_file() {
        let dir = TempDir::new().unwrap();
        let vcs = init_repo_with_commit(&dir);

        fs::write(dir.path().join("new.md"), "새 파일\n").unwrap();

        let changes = vcs.status().unwrap();
        assert!(!changes.is_empty(), "새 파일이 감지되어야 함");
        assert!(changes.iter().any(|c| c.path == "new.md"));
        assert!(changes.iter().any(|c| c.status == "added"));
    }

    #[test]
    fn test_status_shows_modified_file() {
        let dir = TempDir::new().unwrap();
        let vcs = init_repo_with_commit(&dir);

        fs::write(dir.path().join("CLAUDE.md"), "수정된 내용\n").unwrap();

        let changes = vcs.status().unwrap();
        assert!(!changes.is_empty());
        assert!(changes.iter().any(|c| c.path == "CLAUDE.md"));
        assert!(changes.iter().any(|c| c.status == "modified"));
    }

    // ── log ──────────────────────────────────────────────────────────────────

    #[test]
    fn test_log_single_commit() {
        let dir = TempDir::new().unwrap();
        let vcs = init_repo_with_commit(&dir);

        let commits = vcs.log(10, None).unwrap();
        assert_eq!(commits.len(), 1);
        assert!(commits[0].message.contains("초기 커밋"));
        assert!(!commits[0].hash.is_empty());
        assert!(!commits[0].date.is_empty());
    }

    #[test]
    fn test_log_multiple_commits_all_present() {
        let dir = TempDir::new().unwrap();
        let mut vcs = ProfileVcs::new(dir.path().to_path_buf()).unwrap();
        vcs.init().unwrap();

        for i in 1..=3 {
            fs::write(dir.path().join("file.md"), format!("v{}\n", i)).unwrap();
            vcs.commit(&format!("커밋 {}", i), &[]).unwrap();
        }

        let commits = vcs.log(10, None).unwrap();
        assert_eq!(commits.len(), 3);
        // Timestamps may be identical in fast tests, check presence instead of order
        assert!(commits.iter().any(|c| c.message.contains("커밋 1")));
        assert!(commits.iter().any(|c| c.message.contains("커밋 2")));
        assert!(commits.iter().any(|c| c.message.contains("커밋 3")));
    }

    #[test]
    fn test_log_respects_limit() {
        let dir = TempDir::new().unwrap();
        let mut vcs = ProfileVcs::new(dir.path().to_path_buf()).unwrap();
        vcs.init().unwrap();

        for i in 1..=5 {
            fs::write(dir.path().join("file.md"), format!("v{}\n", i)).unwrap();
            vcs.commit(&format!("커밋 {}", i), &[]).unwrap();
        }

        let commits = vcs.log(2, None).unwrap();
        // With limit=2, exactly 2 entries should be returned
        assert_eq!(commits.len(), 2);
    }

    #[test]
    fn test_last_commit_returns_latest() {
        let dir = TempDir::new().unwrap();
        let mut vcs = ProfileVcs::new(dir.path().to_path_buf()).unwrap();
        vcs.init().unwrap();

        fs::write(dir.path().join("f.md"), "v1").unwrap();
        vcs.commit("첫 번째", &[]).unwrap();

        fs::write(dir.path().join("f.md"), "v2").unwrap();
        vcs.commit("두 번째", &[]).unwrap();

        let summary = vcs.last_commit().unwrap();
        assert!(summary.message.contains("두 번째"));
    }

    // ── diff ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_diff_no_commits_returns_placeholder() {
        let dir = TempDir::new().unwrap();
        let mut vcs = ProfileVcs::new(dir.path().to_path_buf()).unwrap();
        vcs.init().unwrap();

        let diff = vcs.diff(None, false).unwrap();
        assert!(diff.contains("없음"), "커밋 없을 때 안내 메시지: {}", diff);
    }

    #[test]
    fn test_diff_clean_working_dir_is_empty() {
        let dir = TempDir::new().unwrap();
        let vcs = init_repo_with_commit(&dir);

        let diff = vcs.diff(None, false).unwrap();
        assert!(diff.is_empty(), "변경사항 없으면 빈 diff: {:?}", diff);
    }

    #[test]
    fn test_diff_shows_file_changes() {
        let dir = TempDir::new().unwrap();
        let vcs = init_repo_with_commit(&dir);

        fs::write(dir.path().join("CLAUDE.md"), "수정된 내용\n새 줄\n").unwrap();

        let diff = vcs.diff(None, false).unwrap();
        assert!(!diff.is_empty(), "변경사항이 diff에 나타나야 함");
    }

    #[test]
    fn test_diff_name_only_shows_filename() {
        let dir = TempDir::new().unwrap();
        let vcs = init_repo_with_commit(&dir);

        // New untracked files don't appear in diff, so modify an existing tracked file
        fs::write(dir.path().join("CLAUDE.md"), "수정된 내용\n변경됨\n").unwrap();

        let diff = vcs.diff(None, true).unwrap();
        assert!(diff.contains("CLAUDE.md"), "이름만 표시: {}", diff);
    }

    // ── Tags ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_tag_annotated_create_and_list() {
        let dir = TempDir::new().unwrap();
        let vcs = init_repo_with_commit(&dir);

        vcs.create_tag("v1.0.0", Some("첫 버전")).unwrap();

        let tags = vcs.list_tags().unwrap();
        assert!(tags.contains(&"v1.0.0".to_string()));
    }

    #[test]
    fn test_tag_lightweight_create_and_list() {
        let dir = TempDir::new().unwrap();
        let vcs = init_repo_with_commit(&dir);

        vcs.create_tag("v1.0-light", None).unwrap();

        let tags = vcs.list_tags().unwrap();
        assert!(tags.contains(&"v1.0-light".to_string()));
    }

    #[test]
    fn test_tag_list_sorted() {
        let dir = TempDir::new().unwrap();
        let vcs = init_repo_with_commit(&dir);

        vcs.create_tag("v2.0", None).unwrap();
        vcs.create_tag("v1.0", None).unwrap();
        vcs.create_tag("v3.0", None).unwrap();

        let tags = vcs.list_tags().unwrap();
        // Should be sorted
        assert_eq!(tags, vec!["v1.0", "v2.0", "v3.0"]);
    }

    #[test]
    fn test_tag_duplicate_fails() {
        let dir = TempDir::new().unwrap();
        let vcs = init_repo_with_commit(&dir);

        vcs.create_tag("v1.0", None).unwrap();
        let result = vcs.create_tag("v1.0", None);
        assert!(result.is_err(), "중복 태그는 실패해야 함");
    }

    #[test]
    fn test_tag_delete_removes_tag() {
        let dir = TempDir::new().unwrap();
        let vcs = init_repo_with_commit(&dir);

        vcs.create_tag("v1.0", None).unwrap();
        vcs.delete_tag("v1.0").unwrap();

        let tags = vcs.list_tags().unwrap();
        assert!(!tags.contains(&"v1.0".to_string()), "삭제 후 목록에서 제거");
    }

    // ── checkout ─────────────────────────────────────────────────────────────

    #[test]
    fn test_checkout_by_tag_restores_file_content() {
        let dir = TempDir::new().unwrap();
        let mut vcs = ProfileVcs::new(dir.path().to_path_buf()).unwrap();
        vcs.init().unwrap();

        // v1 commit + tag
        fs::write(dir.path().join("CLAUDE.md"), "v1 내용\n").unwrap();
        vcs.commit("v1", &[]).unwrap();
        vcs.create_tag("v1.0", None).unwrap();

        // v2 commit
        fs::write(dir.path().join("CLAUDE.md"), "v2 내용\n").unwrap();
        vcs.commit("v2", &[]).unwrap();

        // Checkout by v1 tag
        vcs.checkout("v1.0").unwrap();

        let content = fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap();
        assert!(
            content.contains("v1 내용"),
            "체크아웃 후 v1 내용이어야 함: {}",
            content
        );
        assert!(!content.contains("v2 내용"));
    }

    #[test]
    fn test_checkout_nonexistent_fails() {
        let dir = TempDir::new().unwrap();
        let vcs = init_repo_with_commit(&dir);

        let result = vcs.checkout("nonexistent-tag");
        assert!(result.is_err(), "없는 참조로 체크아웃은 실패해야 함");
    }

    // ── revert ───────────────────────────────────────────────────────────────

    #[test]
    fn test_revert_creates_new_commit() {
        let dir = TempDir::new().unwrap();
        let mut vcs = ProfileVcs::new(dir.path().to_path_buf()).unwrap();
        vcs.init().unwrap();

        fs::write(dir.path().join("CLAUDE.md"), "초기 내용\n").unwrap();
        vcs.commit("초기", &[]).unwrap();

        fs::write(dir.path().join("CLAUDE.md"), "변경된 내용\n").unwrap();
        vcs.commit("변경", &[]).unwrap();

        // Revert HEAD (the change commit)
        vcs.revert("HEAD").unwrap();

        let commits = vcs.log(10, None).unwrap();
        assert!(commits.len() >= 3, "revert 커밋이 추가되어야 함");
        assert!(
            commits[0].message.contains("Revert"),
            "최신 커밋이 Revert여야 함: {}",
            commits[0].message
        );
    }
}

/// Check whether a commit touched a specific file
fn commit_touches_file(commit: &git2::Commit, repo: &Repository, file_path: &str) -> Result<bool> {
    let tree = commit.tree()?;

    if commit.parent_count() == 0 {
        // Treat the first commit as adding the file
        return Ok(tree.get_path(std::path::Path::new(file_path)).is_ok());
    }

    let parent = commit.parent(0)?;
    let parent_tree = parent.tree()?;

    let diff = repo.diff_tree_to_tree(Some(&parent_tree), Some(&tree), None)?;

    let mut touched = false;
    diff.foreach(
        &mut |delta, _| {
            if delta
                .new_file()
                .path()
                .is_some_and(|p| p == std::path::Path::new(file_path))
            {
                touched = true;
            }
            true
        },
        None,
        None,
        None,
    )?;

    Ok(touched)
}
