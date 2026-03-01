// src/profile/manager.rs
// Profile manager (CRUD + symlink management)
//
// Manages profiles in the ~/.clenv/profiles/ directory.
// Profile switching changes the ~/.claude/ symlink to the target profile directory.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::config::ClenvConfig;
use crate::dirs;
use crate::error::ClenvError;
use crate::profile::vcs::ProfileVcs;

/// Profile info (for listing)
#[derive(Debug, Serialize, Deserialize)]
pub struct ProfileInfo {
    /// Profile name
    pub name: String,
    /// Last commit info
    pub last_commit: Option<CommitSummary>,
    /// Path to profile directory
    pub path: PathBuf,
}

/// Commit summary info
#[derive(Debug, Serialize, Deserialize)]
pub struct CommitSummary {
    pub hash: String,
    pub message: String,
    pub date: String,
    pub author: String,
}

/// Diagnostic result from doctor
pub struct DoctorIssue {
    pub severity: String, // "error", "warning", "info"
    pub title: String,
    pub description: String,
    pub fix_hint_opt: Option<String>,
    pub fix_hint: String,
    pub auto_fix: Option<String>, // name of auto-fix function
}

/// Profile manager
pub struct ProfileManager {
    config: ClenvConfig,
}

impl ProfileManager {
    /// Create a new ProfileManager
    pub fn new() -> Result<Self> {
        let config = ClenvConfig::load()?;
        Ok(Self { config })
    }

    /// Return the name of the currently active profile
    pub fn active_profile_name(&self) -> Option<String> {
        // Read from symlink or config
        dirs::active_profile_name().or_else(|| self.config.active_profile.clone())
    }

    /// Return the path to a profile directory
    pub fn profile_path(&self, name: &str) -> PathBuf {
        dirs::profile_dir(name)
    }

    /// Create a new profile
    ///
    /// 1. Create ~/.clenv/profiles/<name>/ directory
    /// 2. Initialize a git repo (for version control)
    /// 3. Copy default files (from source profile if specified, otherwise start with empty config)
    pub fn create(&self, name: &str, from: Option<&str>) -> Result<()> {
        // Validate name
        validate_profile_name(name)?;

        let profile_path = dirs::profile_dir(name);

        // Check if already exists
        if profile_path.exists() {
            return Err(ClenvError::ProfileAlreadyExists(name.to_string()).into());
        }

        if let Some(source) = from {
            // Copy from source profile
            let source_path = dirs::profile_dir(source);
            if !source_path.exists() {
                return Err(ClenvError::ProfileNotFound(source.to_string()).into());
            }

            // Copy excluding .git directory
            copy_profile_files(&source_path, &profile_path)?;
        } else {
            // Create empty profile (start with default settings)
            std::fs::create_dir_all(&profile_path).with_context(|| {
                format!(
                    "Failed to create profile directory: {}",
                    profile_path.display()
                )
            })?;

            // Create default CLAUDE.md
            create_default_files(&profile_path, name)?;
        }

        // Set up shared file symlinks before the initial git commit
        if let Err(e) = ensure_shared_symlinks(&profile_path) {
            log::warn!(
                "Failed to set up shared file symlinks for '{}': {}",
                name,
                e
            );
        }

        // Initialize git repo (for version control)
        // &mut self required: self.repo must be updated after init() before commit() can be used
        let mut vcs = ProfileVcs::new(profile_path.clone())?;
        vcs.init()?;

        // Initial commit
        let init_message = if let Some(source) = from {
            format!("Create '{}' from '{}' profile", name, source)
        } else {
            format!("Initial commit: create '{}' profile", name)
        };
        vcs.commit(&init_message, &[])?;

        Ok(())
    }

    /// Initialize clenv: back up original ~/.claude/, create 'default' profile, and activate it.
    ///
    /// On first call: backs up ~/.claude/ to ~/.clenv/backup/original/ and writes manifest.json.
    /// On --reinit: skips re-backing-up if backup already exists; recreates the default profile.
    pub fn initialize(&self, reinit: bool) -> Result<()> {
        if dirs::is_clenv_initialized() && !reinit {
            anyhow::bail!(
                "clenv is already initialized. Use 'clenv init --reinit' to reinitialize."
            );
        }

        let claude_home = dirs::claude_home();
        let backup_original = dirs::backup_original_dir();

        // Step 1: Back up original ~/.claude/ (only on first init)
        if !backup_original.exists() {
            std::fs::create_dir_all(dirs::backup_dir())
                .with_context(|| "Failed to create backup directory")?;

            let had_original = claude_home.exists() && !claude_home.is_symlink();

            if had_original {
                // Prefer rename (O(1) on same filesystem); fall back to copy if cross-device
                if std::fs::rename(&claude_home, &backup_original).is_err() {
                    copy_profile_files(&claude_home, &backup_original)
                        .with_context(|| "Failed to back up ~/.claude")?;
                    std::fs::remove_dir_all(&claude_home)
                        .with_context(|| "Failed to remove ~/.claude after backup")?;
                }

                // Migrate account-level files (history, stats, etc.) to the shared dir
                if let Err(e) = migrate_shared_from(&backup_original) {
                    log::warn!(
                        "Failed to migrate shared files from backup (continuing): {}",
                        e
                    );
                }
            } else {
                std::fs::create_dir_all(&backup_original)
                    .with_context(|| "Failed to create empty backup directory")?;
            }

            let manifest = serde_json::json!({
                "initialized_at": chrono::Utc::now().to_rfc3339(),
                "clenv_version": env!("CARGO_PKG_VERSION"),
                "had_original_claude": had_original,
            });
            std::fs::write(
                dirs::backup_manifest(),
                serde_json::to_string_pretty(&manifest)?,
            )
            .with_context(|| "Failed to write backup manifest")?;
        }

        // Step 2: Create 'default' profile
        let profile_path = dirs::profile_dir("default");

        if profile_path.exists() && reinit {
            std::fs::remove_dir_all(&profile_path)
                .with_context(|| "Failed to remove existing 'default' profile")?;
        }

        if !profile_path.exists() {
            let backup_has_content = backup_original.exists()
                && std::fs::read_dir(&backup_original)
                    .map(|mut d| d.next().is_some())
                    .unwrap_or(false);

            if backup_has_content {
                copy_profile_files(&backup_original, &profile_path)
                    .with_context(|| "Failed to create 'default' profile from backup")?;
            } else {
                std::fs::create_dir_all(&profile_path)
                    .with_context(|| "Failed to create 'default' profile directory")?;
                create_default_files(&profile_path, "default")?;
            }

            // Set up shared file symlinks before the initial git commit
            if let Err(e) = ensure_shared_symlinks(&profile_path) {
                log::warn!(
                    "Failed to set up shared file symlinks in default profile: {}",
                    e
                );
            }

            let mut vcs = ProfileVcs::new(profile_path.clone())?;
            vcs.init()?;
            vcs.commit("Initial commit: create 'default' profile", &[])?;

            if let Err(e) = Self::save_mcp_to_profile("default") {
                log::warn!("Failed to save existing MCP config (continuing): {}", e);
            }
        }

        // Step 3: Activate 'default' profile (creates symlink)
        self.use_profile("default")?;

        // Step 5: Write init marker
        std::fs::write(dirs::init_marker(), chrono::Utc::now().to_rfc3339())
            .with_context(|| "Failed to write init marker")?;

        Ok(())
    }

    /// Return the list of profiles
    pub fn list(&self) -> Result<Vec<ProfileInfo>> {
        let profiles_dir = dirs::profiles_dir();

        if !profiles_dir.exists() {
            return Ok(Vec::new());
        }

        let mut profiles = Vec::new();

        // Process each entry in the profiles directory as a profile
        for entry in std::fs::read_dir(&profiles_dir)? {
            let entry = entry?;
            let path = entry.path();

            // Process directories only
            if !path.is_dir() {
                continue;
            }

            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            // Exclude hidden folders such as .git
            if name.starts_with('.') {
                continue;
            }

            // Get last commit info
            let last_commit = ProfileVcs::new(path.clone())
                .ok()
                .and_then(|vcs| vcs.last_commit().ok());

            profiles.push(ProfileInfo {
                name,
                last_commit,
                path,
            });
        }

        // Sort by name
        profiles.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(profiles)
    }

    /// Switch to a profile
    ///
    /// Changes the ~/.claude/ symlink to the specified profile directory,
    /// and swaps mcpServers in ~/.claude.json per profile.
    pub fn use_profile(&self, name: &str) -> Result<()> {
        let profile_path = dirs::profile_dir(name);

        if !profile_path.exists() {
            return Err(ClenvError::ProfileNotFound(name.to_string()).into());
        }

        // Back up mcpServers of the current active profile to user-mcp.json
        if let Some(current_name) = self.active_profile_name() {
            if let Err(e) = Self::save_mcp_to_profile(&current_name) {
                log::warn!("Failed to save current profile MCP (ignoring): {}", e);
            }
        }

        let claude_home = dirs::claude_home();

        // Handle existing ~/.claude/
        if claude_home.exists() || claude_home.is_symlink() {
            if claude_home.is_symlink() {
                // Already a symlink — remove and recreate
                std::fs::remove_file(&claude_home)
                    .with_context(|| "Failed to remove existing ~/.claude symlink")?;
            } else {
                // Real directory — back up then replace
                let backup_path = claude_home.with_extension("backup");
                if backup_path.exists() {
                    // Backup already exists, just remove current
                    std::fs::remove_dir_all(&claude_home)
                        .with_context(|| "Failed to remove ~/.claude directory")?;
                } else {
                    std::fs::rename(&claude_home, &backup_path)
                        .with_context(|| "Failed to rename ~/.claude to ~/.claude.backup")?;
                    log::info!("Renamed existing ~/.claude to ~/.claude.backup");
                }
            }
        }

        // Create new symlink
        // std::os::unix::fs::symlink: create a Unix symlink
        #[cfg(unix)]
        std::os::unix::fs::symlink(&profile_path, &claude_home).with_context(|| {
            format!(
                "Failed to create symlink: {} → {}",
                claude_home.display(),
                profile_path.display()
            )
        })?;

        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&profile_path, &claude_home)
            .with_context(|| "Failed to create symlink (Windows)")?;

        // Apply the new profile's mcpServers to ~/.claude.json
        if let Err(e) = Self::apply_mcp_from_profile(name) {
            log::warn!("Failed to apply new profile MCP (ignoring): {}", e);
        }

        // Ensure shared file symlinks exist in the newly activated profile.
        // Also migrates any real files that may exist from before this feature was added.
        if let Err(e) = ensure_shared_symlinks(&profile_path) {
            log::warn!("Failed to set up shared file symlinks (ignoring): {}", e);
        }

        // Update config
        let mut config = ClenvConfig::load()?;
        config.set_active_profile(name)?;

        Ok(())
    }

    /// Delete a profile
    pub fn delete(&self, name: &str) -> Result<()> {
        // Cannot delete the active profile
        if self.active_profile_name().as_deref() == Some(name) {
            return Err(ClenvError::CannotDeleteActiveProfile(name.to_string()).into());
        }

        let profile_path = dirs::profile_dir(name);

        if !profile_path.exists() {
            return Err(ClenvError::ProfileNotFound(name.to_string()).into());
        }

        // Recursively delete the directory
        std::fs::remove_dir_all(&profile_path).with_context(|| {
            format!(
                "Failed to delete profile directory: {}",
                profile_path.display()
            )
        })?;

        Ok(())
    }

    /// Clone a profile
    pub fn clone_profile(&self, source: &str, destination: &str) -> Result<()> {
        validate_profile_name(destination)?;

        let source_path = dirs::profile_dir(source);
        let dest_path = dirs::profile_dir(destination);

        if !source_path.exists() {
            return Err(ClenvError::ProfileNotFound(source.to_string()).into());
        }

        if dest_path.exists() {
            return Err(ClenvError::ProfileAlreadyExists(destination.to_string()).into());
        }

        copy_profile_files(&source_path, &dest_path)?;

        // Set up shared file symlinks in the cloned profile
        if let Err(e) = ensure_shared_symlinks(&dest_path) {
            log::warn!(
                "Failed to set up shared file symlinks for clone '{}': {}",
                destination,
                e
            );
        }

        // Initialize a new git repo
        let mut vcs = ProfileVcs::new(dest_path)?;
        vcs.init()?;
        vcs.commit(
            &format!("Clone '{}' profile to '{}'", source, destination),
            &[],
        )?;

        Ok(())
    }

    /// Rename a profile
    pub fn rename(&self, old_name: &str, new_name: &str) -> Result<()> {
        validate_profile_name(new_name)?;

        let old_path = dirs::profile_dir(old_name);
        let new_path = dirs::profile_dir(new_name);

        if !old_path.exists() {
            return Err(ClenvError::ProfileNotFound(old_name.to_string()).into());
        }

        if new_path.exists() {
            return Err(ClenvError::ProfileAlreadyExists(new_name.to_string()).into());
        }

        std::fs::rename(&old_path, &new_path)?;

        // If this was the active profile, update the symlink
        if self.active_profile_name().as_deref() == Some(old_name) {
            self.use_profile(new_name)?;
        }

        Ok(())
    }

    /// Diagnose the configuration
    pub fn doctor(&self) -> Result<Vec<DoctorIssue>> {
        let mut issues = Vec::new();

        // 1. Check ~/.claude/ directory
        if !dirs::claude_home().exists() {
            issues.push(DoctorIssue {
                severity: "warning".to_string(),
                title: "~/.claude directory not found".to_string(),
                description: "Claude Code is not installed or has never been run".to_string(),
                fix_hint: "Install Claude Code".to_string(),
                fix_hint_opt: Some("Download from https://claude.ai/download".to_string()),
                auto_fix: None,
            });
        }

        // 2. Check whether clenv is managing ~/.claude/
        if dirs::claude_home().exists() && !dirs::is_managed_by_clenv() {
            issues.push(DoctorIssue {
                severity: "info".to_string(),
                title: "~/.claude is not managed by clenv".to_string(),
                description: "~/.claude is a plain directory. Run 'clenv init' to start managing it with clenv".to_string(),
                fix_hint: "clenv init".to_string(),
                fix_hint_opt: Some("clenv init".to_string()),
                auto_fix: None,
            });
        }

        // 3. Check active profile
        if let Some(active) = self.active_profile_name() {
            if !dirs::profile_dir(&active).exists() {
                issues.push(DoctorIssue {
                    severity: "error".to_string(),
                    title: format!("Active profile '{}' directory missing", active),
                    description: "Symlink exists but the target profile directory was deleted"
                        .to_string(),
                    fix_hint: format!("clenv profile create {}", active),
                    fix_hint_opt: Some(format!("clenv profile create {}", active)),
                    auto_fix: None,
                });
            }
        }

        // 4. Check for broken symlinks
        let claude_home = dirs::claude_home();
        if claude_home.is_symlink() {
            if let Ok(target) = std::fs::read_link(&claude_home) {
                if !target.exists() {
                    issues.push(DoctorIssue {
                        severity: "error".to_string(),
                        title: "Broken symlink".to_string(),
                        description: format!("~/.claude → {} does not exist", target.display()),
                        fix_hint: "clenv profile use <name>".to_string(),
                        fix_hint_opt: Some("clenv profile use <name>".to_string()),
                        auto_fix: None,
                    });
                }
            }
        }

        Ok(issues)
    }

    /// Unmanage clenv (restore symlink → real directory)
    ///
    /// 1. Copy active profile contents to a real ~/.claude/ directory
    /// 2. Remove the symlink
    /// 3. If purge=true, delete ~/.clenv/ entirely
    pub fn deactivate(&self, purge: bool) -> Result<()> {
        let claude_home = dirs::claude_home();

        if !dirs::is_managed_by_clenv() {
            return Err(anyhow::anyhow!("~/.claude is not managed by clenv"));
        }

        // Resolve active profile path
        let profile_path =
            std::fs::read_link(&claude_home).with_context(|| "Failed to read symlink target")?;

        if !profile_path.exists() {
            return Err(anyhow::anyhow!(
                "Active profile directory does not exist: {}",
                profile_path.display()
            ));
        }

        // Copy profile to a temporary path
        let tmp_path = claude_home.with_file_name(".claude-clenv-restore-tmp");
        if tmp_path.exists() {
            std::fs::remove_dir_all(&tmp_path)?;
        }
        copy_profile_files(&profile_path, &tmp_path)
            .with_context(|| "Failed to copy profile contents")?;

        // Also copy shared files as real files (not symlinks) into the restored directory
        if let Err(e) = copy_shared_to(&tmp_path) {
            log::warn!("Failed to copy shared files to restored directory: {}", e);
        }

        // Remove symlink
        std::fs::remove_file(&claude_home).with_context(|| "Failed to remove symlink")?;

        // Move temporary directory → ~/.claude/
        std::fs::rename(&tmp_path, &claude_home).with_context(|| "Failed to restore ~/.claude")?;

        if purge {
            let clenv_home = dirs::clenv_home();
            if clenv_home.exists() {
                std::fs::remove_dir_all(&clenv_home)
                    .with_context(|| "Failed to delete ~/.clenv")?;
            }
        }

        Ok(())
    }

    /// Restore original ~/.claude/ from backup (used by uninstall).
    ///
    /// Replaces the current ~/.claude symlink with the original backup from
    /// ~/.clenv/backup/original/ that was saved during 'clenv init'.
    pub fn restore_original(&self) -> Result<()> {
        let claude_home = dirs::claude_home();
        let backup_dir = dirs::backup_original_dir();

        if !dirs::is_managed_by_clenv() {
            anyhow::bail!("~/.claude is not currently managed by clenv");
        }

        if !backup_dir.exists() {
            anyhow::bail!(
                "Original backup not found at ~/.clenv/backup/original/.\n\
                 Cannot restore original ~/.claude/.\n\
                 Your profiles are still available in ~/.clenv/profiles/."
            );
        }

        let tmp_path = claude_home.with_file_name(".claude-clenv-restore-tmp");
        if tmp_path.exists() {
            std::fs::remove_dir_all(&tmp_path)?;
        }

        copy_profile_files(&backup_dir, &tmp_path)
            .with_context(|| "Failed to copy original backup")?;

        // Overlay current shared files (history, stats, etc.) so they are not lost
        if let Err(e) = copy_shared_to(&tmp_path) {
            log::warn!("Failed to copy shared files during restore: {}", e);
        }

        std::fs::remove_file(&claude_home).with_context(|| "Failed to remove ~/.claude symlink")?;

        std::fs::rename(&tmp_path, &claude_home).with_context(|| "Failed to restore ~/.claude")?;

        Ok(())
    }

    /// Auto-fix issues found by doctor
    pub fn apply_fix(&self, fix_fn: &str) -> Result<()> {
        // Auto-fix capability is currently limited
        // Will be expanded in the future
        Err(anyhow::anyhow!("Auto-fix '{}' is not supported", fix_fn))
    }
}

// ── MCP-related helpers (static) ──────────────────────────────────────────────

impl ProfileManager {
    /// Read mcpServers from ~/.claude.json (returns empty object if absent)
    fn read_claude_json_mcp() -> serde_json::Value {
        let path = dirs::claude_json();
        if !path.exists() {
            return serde_json::Value::Object(Default::default());
        }
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();
        json.get("mcpServers")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Object(Default::default()))
    }

    /// Replace only the mcpServers key in ~/.claude.json (preserving the rest of app state).
    ///
    /// Skips the write entirely when mcpServers hasn't changed, so the file's mtime is
    /// not bumped unnecessarily — Claude Code watches this file and may force re-login
    /// when it detects an external modification.
    fn write_claude_json_mcp(mcp: &serde_json::Value) -> Result<()> {
        let path = dirs::claude_json();
        if !path.exists() {
            return Ok(());
        }
        let content = std::fs::read_to_string(&path)?;
        let mut json: serde_json::Value = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}: not valid JSON", path.display()))?;

        // Skip write when nothing changed to avoid bumping mtime.
        let current = json
            .get("mcpServers")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Object(Default::default()));
        if current == *mcp {
            return Ok(());
        }

        json["mcpServers"] = mcp.clone();
        std::fs::write(&path, serde_json::to_string_pretty(&json)?)?;
        Ok(())
    }

    /// Save the current mcpServers from ~/.claude.json to the profile's user-mcp.json
    fn save_mcp_to_profile(profile_name: &str) -> Result<()> {
        let mcp = Self::read_claude_json_mcp();
        let mcp_file = dirs::user_mcp_file(profile_name);
        std::fs::write(&mcp_file, serde_json::to_string_pretty(&mcp)?)?;
        Ok(())
    }

    /// Read the profile's user-mcp.json and apply it to ~/.claude.json (empty object if absent)
    fn apply_mcp_from_profile(profile_name: &str) -> Result<()> {
        let mcp_file = dirs::user_mcp_file(profile_name);
        let mcp = if mcp_file.exists() {
            let content = std::fs::read_to_string(&mcp_file)?;
            serde_json::from_str(&content)
                .unwrap_or_else(|_| serde_json::Value::Object(Default::default()))
        } else {
            serde_json::Value::Object(Default::default())
        };
        Self::write_claude_json_mcp(&mcp)?;
        Ok(())
    }
}

// ── Helper functions ──────────────────────────────────────────────────────────

/// Validate profile name
fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("Profile name cannot be empty");
    }

    if name.starts_with('.') || name.starts_with('-') {
        anyhow::bail!("Profile name '{}' must not start with '.' or '-'", name);
    }

    // Allowed characters: letters, digits, hyphens, underscores
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        anyhow::bail!(
            "Profile name '{}' contains invalid characters. Use only letters, digits, hyphens (-), and underscores (_)",
            name
        );
    }

    Ok(())
}

/// Copy profile files (.git excluded)
///
/// Directories that are excluded from profile copies.
/// These are Claude Code cache/log/temp directories that are not part of the user's settings.
const EXCLUDED_DIRS: &[&str] = &[
    "projects",
    "debug",
    "telemetry",
    "shell-snapshots",
    "backups",
    "file-history",
    "cache",
    "statsig",
    "paste-cache",
    "session-env",
];

/// Files stored globally in ~/.clenv/shared/ and symlinked into every profile.
/// These contain account-level state that should persist across profile switches.
const SHARED_FILES: &[&str] = &[
    "history.jsonl",
    "stats-cache.json",
    ".session-stats.json",
    "mcp-needs-auth-cache.json",
];

/// Copy profile files from source to dest, excluding .git and cache/temp directories.
///
/// On macOS, tries `cp -cR` first (APFS clonefile: copy-on-write, near-instant).
/// Falls back to a walkdir-based copy on failure or other platforms.
fn copy_profile_files(source: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;

    #[cfg(target_os = "macos")]
    if copy_profile_files_clonefile(source, dest) {
        return Ok(());
    }

    copy_profile_files_walkdir(source, dest)
}

/// macOS fast path: clone via `cp -cR` then remove excluded dirs.
/// On APFS, both the clone and the subsequent removes are metadata-only operations.
#[cfg(target_os = "macos")]
fn copy_profile_files_clonefile(source: &std::path::Path, dest: &std::path::Path) -> bool {
    let src_arg = format!("{}/.", source.display());
    let ok = std::process::Command::new("cp")
        .args(["-cR", &src_arg, dest.to_str().unwrap_or("")])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !ok {
        return false;
    }

    // Remove .git, excluded dirs, and shared files (they'll be symlinked separately)
    for name in std::iter::once(".git")
        .chain(EXCLUDED_DIRS.iter().copied())
        .chain(SHARED_FILES.iter().copied())
    {
        let path = dest.join(name);
        if path.is_symlink() || path.is_file() {
            let _ = std::fs::remove_file(&path);
        } else if path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        }
    }

    // Remove any nested .git directories left by cp -cR.
    // Subdirectories like plugins/marketplaces/*/ may themselves be git repos;
    // libgit2 treats them as unregistered gitlinks and fails with "invalid path".
    let nested_git_dirs: Vec<_> = walkdir::WalkDir::new(dest)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name() == ".git" && e.file_type().is_dir())
        .map(|e| e.path().to_owned())
        .collect();
    for git_dir in nested_git_dirs {
        let _ = std::fs::remove_dir_all(&git_dir);
    }

    true
}

/// Portable fallback: walkdir copy with .git and cache dir exclusions.
fn copy_profile_files_walkdir(source: &std::path::Path, dest: &std::path::Path) -> Result<()> {
    for entry in walkdir::WalkDir::new(source).follow_links(true) {
        let entry = entry?;
        let path = entry.path();
        let relative = path.strip_prefix(source)?;

        // Skip the source root itself
        if relative.as_os_str().is_empty() {
            continue;
        }

        // Exclude .git at any depth, cache/temp dirs, and globally shared files.
        // Checking all components (not just the first) catches nested git repos such as
        // plugins/marketplaces/some-plugin/.git which libgit2 rejects as unregistered gitlinks.
        let first = relative
            .components()
            .next()
            .and_then(|c| c.as_os_str().to_str())
            .unwrap_or("");
        let has_git_component = relative.components().any(|c| c.as_os_str() == ".git");
        if has_git_component || EXCLUDED_DIRS.contains(&first) || SHARED_FILES.contains(&first) {
            continue;
        }

        let dest_path = dest.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dest_path)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = dest_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(path, &dest_path)?;
        }
        // Skip broken symlinks, sockets, pipes, etc.
    }

    Ok(())
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use tempfile::TempDir;

    /// Set up isolated test environment (CLENV_HOME, CLAUDE_HOME → TempDir)
    fn setup(temp: &TempDir) {
        std::env::set_var("CLENV_HOME", temp.path().join(".clenv").to_str().unwrap());
        std::env::set_var("CLAUDE_HOME", temp.path().join(".claude").to_str().unwrap());
        dirs::ensure_initialized().unwrap();
    }

    fn teardown() {
        std::env::remove_var("CLENV_HOME");
        std::env::remove_var("CLAUDE_HOME");
    }

    // ── validate_profile_name ────────────────────────────────────────────────

    #[test]
    fn test_validate_name_valid() {
        assert!(validate_profile_name("work").is_ok());
        assert!(validate_profile_name("my-profile").is_ok());
        assert!(validate_profile_name("my_profile").is_ok());
        assert!(validate_profile_name("profile123").is_ok());
        assert!(validate_profile_name("a").is_ok());
        assert!(validate_profile_name("UPPERCASE").is_ok());
        assert!(validate_profile_name("mix-123_test").is_ok());
    }

    #[test]
    fn test_validate_name_empty_fails() {
        assert!(validate_profile_name("").is_err());
    }

    #[test]
    fn test_validate_name_dot_prefix_fails() {
        assert!(validate_profile_name(".hidden").is_err());
    }

    #[test]
    fn test_validate_name_dash_prefix_fails() {
        assert!(validate_profile_name("-start").is_err());
    }

    #[test]
    fn test_validate_name_space_fails() {
        assert!(validate_profile_name("has space").is_err());
    }

    #[test]
    fn test_validate_name_slash_fails() {
        assert!(validate_profile_name("has/slash").is_err());
    }

    #[test]
    fn test_validate_name_dot_inside_ok() {
        // Middle dot is not an allowed character (only letters, digits, -, _ are allowed)
        assert!(validate_profile_name("my.profile").is_err());
    }

    // ── ProfileManager CRUD ──────────────────────────────────────────────────

    #[test]
    #[serial]
    fn test_create_and_list() {
        let temp = TempDir::new().unwrap();
        setup(&temp);

        let manager = ProfileManager::new().unwrap();
        manager.create("alpha", None).unwrap();
        manager.create("beta", None).unwrap();

        let profiles = manager.list().unwrap();
        teardown();

        assert_eq!(profiles.len(), 2);
        assert!(profiles.iter().any(|p| p.name == "alpha"));
        assert!(profiles.iter().any(|p| p.name == "beta"));
    }

    #[test]
    #[serial]
    fn test_create_generates_default_files() {
        let temp = TempDir::new().unwrap();
        setup(&temp);

        let manager = ProfileManager::new().unwrap();
        manager.create("newprofile", None).unwrap();

        let profile_path = dirs::profile_dir("newprofile");
        teardown();

        assert!(profile_path.join("CLAUDE.md").exists());
        assert!(profile_path.join("settings.json").exists());
        assert!(profile_path.join("hooks").is_dir());
        assert!(profile_path.join("agents").is_dir());
        assert!(profile_path.join("skills").is_dir());
        assert!(profile_path.join(".git").is_dir());
    }

    #[test]
    #[serial]
    fn test_create_duplicate_fails() {
        let temp = TempDir::new().unwrap();
        setup(&temp);

        let manager = ProfileManager::new().unwrap();
        manager.create("dup", None).unwrap();
        let result = manager.create("dup", None);
        teardown();

        assert!(result.is_err());
    }

    #[test]
    #[serial]
    fn test_create_from_source_copies_files() {
        let temp = TempDir::new().unwrap();
        setup(&temp);

        let manager = ProfileManager::new().unwrap();
        manager.create("source", None).unwrap();

        // Add a custom file to source
        let source_path = dirs::profile_dir("source");
        std::fs::write(source_path.join("custom.md"), "소스 내용").unwrap();

        manager.create("copy", Some("source")).unwrap();

        let copy_path = dirs::profile_dir("copy");
        teardown();

        assert!(copy_path.join("custom.md").exists());
        let content = std::fs::read_to_string(copy_path.join("custom.md")).unwrap();
        assert!(content.contains("소스 내용"));
    }

    #[test]
    #[serial]
    fn test_create_from_nonexistent_source_fails() {
        let temp = TempDir::new().unwrap();
        setup(&temp);

        let manager = ProfileManager::new().unwrap();
        let result = manager.create("copy", Some("ghost"));
        teardown();

        assert!(result.is_err());
    }

    #[test]
    #[serial]
    fn test_delete_profile() {
        let temp = TempDir::new().unwrap();
        setup(&temp);

        let manager = ProfileManager::new().unwrap();
        manager.create("keeper", None).unwrap();
        manager.create("victim", None).unwrap();
        manager.use_profile("keeper").unwrap();

        manager.delete("victim").unwrap();

        let profiles = manager.list().unwrap();
        teardown();

        assert!(!profiles.iter().any(|p| p.name == "victim"));
        assert!(profiles.iter().any(|p| p.name == "keeper"));
    }

    #[test]
    #[serial]
    fn test_delete_active_profile_fails() {
        let temp = TempDir::new().unwrap();
        setup(&temp);

        let manager = ProfileManager::new().unwrap();
        manager.create("active", None).unwrap();
        manager.use_profile("active").unwrap();

        let result = manager.delete("active");
        teardown();

        assert!(result.is_err());
    }

    #[test]
    #[serial]
    fn test_delete_nonexistent_fails() {
        let temp = TempDir::new().unwrap();
        setup(&temp);

        let manager = ProfileManager::new().unwrap();
        let result = manager.delete("ghost");
        teardown();

        assert!(result.is_err());
    }

    #[test]
    #[serial]
    fn test_clone_profile_copies_files() {
        let temp = TempDir::new().unwrap();
        setup(&temp);

        let manager = ProfileManager::new().unwrap();
        manager.create("original", None).unwrap();

        let orig_path = dirs::profile_dir("original");
        std::fs::write(orig_path.join("important.md"), "중요한 내용").unwrap();

        manager.clone_profile("original", "cloned").unwrap();

        let cloned_path = dirs::profile_dir("cloned");
        teardown();

        assert!(cloned_path.join("important.md").exists());
        let content = std::fs::read_to_string(cloned_path.join("important.md")).unwrap();
        assert!(content.contains("중요한 내용"));
        // A new git repo should be initialized
        assert!(cloned_path.join(".git").is_dir());
    }

    #[test]
    #[serial]
    fn test_clone_nonexistent_fails() {
        let temp = TempDir::new().unwrap();
        setup(&temp);

        let manager = ProfileManager::new().unwrap();
        let result = manager.clone_profile("ghost", "copy");
        teardown();

        assert!(result.is_err());
    }

    #[test]
    #[serial]
    fn test_clone_to_existing_fails() {
        let temp = TempDir::new().unwrap();
        setup(&temp);

        let manager = ProfileManager::new().unwrap();
        manager.create("src", None).unwrap();
        manager.create("existing", None).unwrap();

        let result = manager.clone_profile("src", "existing");
        teardown();

        assert!(result.is_err());
    }

    #[test]
    #[serial]
    fn test_rename_profile() {
        let temp = TempDir::new().unwrap();
        setup(&temp);

        let manager = ProfileManager::new().unwrap();
        manager.create("other", None).unwrap();
        manager.create("oldname", None).unwrap();
        manager.use_profile("other").unwrap();

        manager.rename("oldname", "newname").unwrap();

        let profiles = manager.list().unwrap();
        teardown();

        assert!(profiles.iter().any(|p| p.name == "newname"));
        assert!(!profiles.iter().any(|p| p.name == "oldname"));
    }

    #[test]
    #[serial]
    fn test_rename_nonexistent_fails() {
        let temp = TempDir::new().unwrap();
        setup(&temp);

        let manager = ProfileManager::new().unwrap();
        let result = manager.rename("ghost", "newname");
        teardown();

        assert!(result.is_err());
    }

    #[test]
    #[serial]
    fn test_rename_to_existing_fails() {
        let temp = TempDir::new().unwrap();
        setup(&temp);

        let manager = ProfileManager::new().unwrap();
        manager.create("foo", None).unwrap();
        manager.create("bar", None).unwrap();

        let result = manager.rename("foo", "bar");
        teardown();

        assert!(result.is_err());
    }

    #[test]
    #[serial]
    fn test_use_profile_creates_symlink() {
        let temp = TempDir::new().unwrap();
        setup(&temp);

        let manager = ProfileManager::new().unwrap();
        manager.create("myprofile", None).unwrap();
        manager.use_profile("myprofile").unwrap();

        let claude_home = dirs::claude_home();
        teardown();

        assert!(claude_home.is_symlink(), "~/.claude 는 심링크여야 함");
    }

    #[test]
    #[serial]
    fn test_use_nonexistent_profile_fails() {
        let temp = TempDir::new().unwrap();
        setup(&temp);

        let manager = ProfileManager::new().unwrap();
        let result = manager.use_profile("ghost");
        teardown();

        assert!(result.is_err());
    }

    #[test]
    #[serial]
    fn test_active_profile_name_from_symlink() {
        let temp = TempDir::new().unwrap();
        setup(&temp);

        let manager = ProfileManager::new().unwrap();
        manager.create("myprofile", None).unwrap();
        manager.use_profile("myprofile").unwrap();

        let active = manager.active_profile_name();
        teardown();

        assert_eq!(active, Some("myprofile".to_string()));
    }

    #[test]
    #[serial]
    fn test_doctor_returns_ok() {
        let temp = TempDir::new().unwrap();
        setup(&temp);

        let manager = ProfileManager::new().unwrap();
        let issues = manager.doctor().unwrap();
        teardown();

        // doctor should run without errors in empty environment
        // There may be warnings since ~.claude does not actually exist
        let _ = issues;
    }

    #[test]
    #[serial]
    fn test_doctor_with_active_profile_healthy() {
        let temp = TempDir::new().unwrap();
        setup(&temp);

        let manager = ProfileManager::new().unwrap();
        manager.create("healthy", None).unwrap();
        manager.use_profile("healthy").unwrap();

        let issues = manager.doctor().unwrap();
        teardown();

        // No errors expected when active profile exists and symlink is valid
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == "error").collect();
        assert!(
            errors.is_empty(),
            "정상 상태에서 error 없어야 함: {:?}",
            errors.iter().map(|e| &e.title).collect::<Vec<_>>()
        );
    }
}

/// Ensure ~/.clenv/shared/ exists and each SHARED_FILE has a symlink in the profile directory.
///
/// - If the profile already has a real file (not a symlink), it is migrated to shared first.
/// - If neither the profile nor shared has the file, an empty placeholder is created in shared.
/// - Also updates .gitignore so shared-file symlinks are never committed to the profile repo.
fn ensure_shared_symlinks(profile_path: &std::path::Path) -> Result<()> {
    let shared_dir = dirs::shared_dir();
    std::fs::create_dir_all(&shared_dir)
        .with_context(|| "Failed to create ~/.clenv/shared directory")?;

    // Update .gitignore to exclude shared files from the profile git repo
    let gitignore_path = profile_path.join(".gitignore");
    let mut gitignore = if gitignore_path.exists() {
        std::fs::read_to_string(&gitignore_path).unwrap_or_default()
    } else {
        String::new()
    };
    let mut gitignore_changed = false;
    for &name in SHARED_FILES {
        if !gitignore.contains(name) {
            if !gitignore.is_empty() && !gitignore.ends_with('\n') {
                gitignore.push('\n');
            }
            gitignore.push_str(name);
            gitignore.push('\n');
            gitignore_changed = true;
        }
    }
    if gitignore_changed {
        let _ = std::fs::write(&gitignore_path, &gitignore);
    }

    for &name in SHARED_FILES {
        let shared_path = shared_dir.join(name);
        let profile_file = profile_path.join(name);

        // Migrate a real file in the profile to the shared directory
        if profile_file.exists() && !profile_file.is_symlink() {
            if !shared_path.exists() {
                if std::fs::rename(&profile_file, &shared_path).is_err() {
                    std::fs::copy(&profile_file, &shared_path)
                        .with_context(|| format!("Failed to migrate '{}' to shared", name))?;
                    let _ = std::fs::remove_file(&profile_file);
                }
            } else {
                // Shared already has this file — drop the profile's copy
                let _ = std::fs::remove_file(&profile_file);
            }
        }

        // Remove a broken symlink
        if profile_file.is_symlink() && !profile_file.exists() {
            let _ = std::fs::remove_file(&profile_file);
        }

        // Create an empty placeholder in shared if nothing exists yet
        if !shared_path.exists() {
            let default = if name.ends_with(".jsonl") { "" } else { "{}" };
            std::fs::write(&shared_path, default)
                .with_context(|| format!("Failed to create shared placeholder '{}'", name))?;
        }

        // Create the symlink if not already present and correct
        if !profile_file.is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(&shared_path, &profile_file)
                .with_context(|| format!("Failed to symlink shared file '{}'", name))?;
        }
    }

    Ok(())
}

/// Move SHARED_FILES found as real files in `source_dir` to ~/.clenv/shared/.
/// Used during `clenv init` to migrate files from the original ~/.claude/ backup.
fn migrate_shared_from(source_dir: &std::path::Path) -> Result<()> {
    let shared_dir = dirs::shared_dir();
    std::fs::create_dir_all(&shared_dir)?;

    for &name in SHARED_FILES {
        let src = source_dir.join(name);
        let dst = shared_dir.join(name);
        if src.exists()
            && !src.is_symlink()
            && !dst.exists()
            && std::fs::rename(&src, &dst).is_err()
        {
            let _ = std::fs::copy(&src, &dst);
        }
    }

    Ok(())
}

/// Copy the actual content of SHARED_FILES into `dest_dir` (as real files, not symlinks).
/// Used when exporting a profile to a plain directory (deactivate / restore_original).
fn copy_shared_to(dest_dir: &std::path::Path) -> Result<()> {
    let shared_dir = dirs::shared_dir();

    for &name in SHARED_FILES {
        let src = shared_dir.join(name);
        let dst = dest_dir.join(name);
        if src.exists() {
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&src, &dst)
                .with_context(|| format!("Failed to copy shared file '{}' to dest", name))?;
        }
    }

    Ok(())
}

/// Create default files for a new profile
fn create_default_files(profile_path: &Path, name: &str) -> Result<()> {
    // Default CLAUDE.md
    let claude_md_content = format!(
        "# Claude Code Settings - '{}' Profile\n\n\
        # Write global instructions for Claude Code here.\n\
        # This profile is managed by clenv.\n\
        # Save changes with 'clenv commit -m \"description\"'.\n\n\
        # Examples:\n\
        # - Always respond in English\n\
        # - Always read files before making changes\n",
        name
    );
    std::fs::write(profile_path.join("CLAUDE.md"), claude_md_content)?;

    // Default settings.json (no Korean comment key)
    let settings_content = serde_json::to_string_pretty(&serde_json::json!({
        "permissions": {
            "allow": [],
            "deny": []
        }
    }))?;
    std::fs::write(profile_path.join("settings.json"), settings_content)?;

    // Create hooks, agents, skills directories
    std::fs::create_dir_all(profile_path.join("hooks"))?;
    std::fs::create_dir_all(profile_path.join("agents"))?;
    std::fs::create_dir_all(profile_path.join("skills"))?;

    Ok(())
}
