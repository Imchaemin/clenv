// tests/e2e_test.rs
//
// clenv full-feature E2E integration tests
//
// All tests isolate CLENV_HOME and CLAUDE_HOME to temporary directories
// so that real user settings are not affected.
//
// Run with:
//   cargo test --test e2e_test

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use predicates::prelude::*;
use std::collections::HashMap;
use std::fs;
use tempfile::TempDir;
use walkdir::WalkDir;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Test helpers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Set up an isolated test environment
/// Set CLENV_HOME and CLAUDE_HOME to temp dirs to protect real settings
fn setup_test_env(temp: &TempDir) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert(
        "CLENV_HOME".to_string(),
        temp.path().join(".clenv").to_str().unwrap().to_string(),
    );
    env.insert(
        "CLAUDE_HOME".to_string(),
        temp.path().join(".claude").to_str().unwrap().to_string(),
    );
    // Disable ANSI color codes for easier output parsing
    env.insert("NO_COLOR".to_string(), "1".to_string());
    env
}

/// clenv command builder
fn clenv(env: &HashMap<String, String>) -> Command {
    let mut cmd = cargo_bin_cmd!("clenv");
    cmd.envs(env);
    cmd
}

/// Initialize clenv in the test environment (required before any command except doctor/uninstall)
fn init_clenv(env: &HashMap<String, String>) {
    clenv(env).arg("init").assert().success();
}

/// Set up an initialized test environment
/// Most tests should use this instead of setup_test_env directly
fn setup_initialized_env(temp: &TempDir) -> HashMap<String, String> {
    let env = setup_test_env(temp);
    init_clenv(&env);
    env
}

/// Helper: create a profile and activate it immediately
fn create_and_use(env: &HashMap<String, String>, name: &str) {
    clenv(env)
        .args(["profile", "create", name, "--use"])
        .assert()
        .success();
}

/// Write a file to a profile directory
fn write_profile_file(temp: &TempDir, profile: &str, filename: &str, content: &str) {
    let dir = temp.path().join(".clenv").join("profiles").join(profile);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(filename), content).unwrap();
}

/// Read a file from a profile directory
fn read_profile_file(temp: &TempDir, profile: &str, filename: &str) -> String {
    fs::read_to_string(
        temp.path()
            .join(".clenv")
            .join("profiles")
            .join(profile)
            .join(filename),
    )
    .unwrap_or_default()
}

/// Extract stdout string from an assert output
fn stdout_of(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

/// Create a nested git repository inside the fake ~/.claude/ to simulate
/// plugin directories like plugins/marketplaces/claude-plugins-official/
fn create_nested_git_repo(temp: &TempDir, rel_path: &str) {
    let dir = temp.path().join(".claude").join(rel_path);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("README.md"), "plugin").unwrap();
    // Initialise a real git repo so libgit2 detects it as a nested repo
    std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&dir)
        .status()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "--allow-empty", "-m", "init", "-q"])
        .current_dir(&dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .status()
        .unwrap();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 1. CLI basics
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn test_version_flag() {
    cargo_bin_cmd!("clenv")
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("clenv"));
}

#[test]
fn test_help_flag() {
    cargo_bin_cmd!("clenv")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("profile"))
        .stdout(predicate::str::contains("commit"))
        .stdout(predicate::str::contains("status"));
}

#[test]
fn test_no_args_fails() {
    // arg_required_else_help → fails when no args provided
    cargo_bin_cmd!("clenv").assert().failure();
}

#[test]
fn test_unknown_command_fails() {
    cargo_bin_cmd!("clenv")
        .arg("nonexistent-command")
        .assert()
        .failure();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 2. Profile CRUD
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn test_profile_list_shows_default_after_init() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    clenv(&env)
        .args(["profile", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("default"));
}

#[test]
fn test_profile_create() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    clenv(&env)
        .args(["profile", "create", "myprofile"])
        .assert()
        .success()
        .stdout(predicate::str::contains("created"));

    // Verify directory was created
    assert!(temp.path().join(".clenv/profiles/myprofile").exists());
}

#[test]
fn test_profile_create_with_use_flag() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    clenv(&env)
        .args(["profile", "create", "myprofile", "--use"])
        .assert()
        .success()
        .stdout(predicate::str::contains("created"))
        .stdout(predicate::str::contains("Active profile"));

    // Verify with current command
    clenv(&env)
        .args(["profile", "current"])
        .assert()
        .success()
        .stdout(predicate::str::contains("myprofile"));
}

#[test]
fn test_profile_create_duplicate_fails() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    clenv(&env)
        .args(["profile", "create", "dup"])
        .assert()
        .success();

    clenv(&env)
        .args(["profile", "create", "dup"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn test_profile_create_from_existing() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "source");
    write_profile_file(&temp, "source", "custom.txt", "소스 파일 내용");
    clenv(&env)
        .args(["commit", "-m", "소스 커밋"])
        .assert()
        .success();

    clenv(&env)
        .args(["profile", "create", "copy", "--from", "source"])
        .assert()
        .success()
        .stdout(predicate::str::contains("created"));

    // Verify file was copied
    let content = read_profile_file(&temp, "copy", "custom.txt");
    assert!(content.contains("소스 파일 내용"));
}

#[test]
fn test_profile_list_shows_all_profiles() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    clenv(&env)
        .args(["profile", "create", "alpha"])
        .assert()
        .success();
    clenv(&env)
        .args(["profile", "create", "beta"])
        .assert()
        .success();
    clenv(&env)
        .args(["profile", "create", "gamma"])
        .assert()
        .success();

    let out = clenv(&env).args(["profile", "list"]).assert().success();
    let stdout = stdout_of(&out);
    assert!(stdout.contains("alpha"));
    assert!(stdout.contains("beta"));
    assert!(stdout.contains("gamma"));
    // default profile from init + alpha + beta + gamma = 4
    assert!(stdout.contains("4"));
}

#[test]
fn test_profile_use_switches_active() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "first");
    clenv(&env)
        .args(["profile", "create", "second"])
        .assert()
        .success();

    clenv(&env)
        .args(["profile", "use", "second"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Active profile"));

    clenv(&env)
        .args(["profile", "current"])
        .assert()
        .success()
        .stdout(predicate::str::contains("second"));
}

#[test]
fn test_profile_use_already_active_shows_info() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "myprofile");

    clenv(&env)
        .args(["profile", "use", "myprofile"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Already"));
}

#[test]
fn test_profile_use_nonexistent_fails() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    clenv(&env)
        .args(["profile", "use", "ghost"])
        .assert()
        .failure();
}

#[test]
fn test_profile_delete() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "keeper");
    clenv(&env)
        .args(["profile", "create", "victim"])
        .assert()
        .success();

    clenv(&env)
        .args(["profile", "delete", "victim", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains("deleted"));

    assert!(!temp.path().join(".clenv/profiles/victim").exists());
}

#[test]
fn test_profile_delete_active_fails() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "active");

    clenv(&env)
        .args(["profile", "delete", "active", "--force"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Cannot delete"));
}

#[test]
fn test_profile_delete_nonexistent_fails() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    clenv(&env)
        .args(["profile", "delete", "ghost", "--force"])
        .assert()
        .failure();
}

#[test]
fn test_profile_rename() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "other");
    clenv(&env)
        .args(["profile", "create", "oldname"])
        .assert()
        .success();

    clenv(&env)
        .args(["profile", "rename", "oldname", "newname"])
        .assert()
        .success()
        .stdout(predicate::str::contains("renamed"));

    let out = clenv(&env).args(["profile", "list"]).assert().success();
    let stdout = stdout_of(&out);
    assert!(stdout.contains("newname"));
    // Verify oldname directory is removed (commit messages may still reference it)
    assert!(!temp.path().join(".clenv/profiles/oldname").exists());
}

#[test]
fn test_profile_clone() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "original");
    write_profile_file(&temp, "original", "important.md", "중요한 설정");
    clenv(&env)
        .args(["commit", "-m", "설정 추가"])
        .assert()
        .success();

    clenv(&env)
        .args(["profile", "clone", "original", "cloned"])
        .assert()
        .success()
        .stdout(predicate::str::contains("cloned"));

    let content = read_profile_file(&temp, "cloned", "important.md");
    assert!(content.contains("중요한 설정"));
}

#[test]
fn test_profile_deactivate() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "myprofile");

    // Verify symlink
    let claude_home = temp.path().join(".claude");
    assert!(claude_home.is_symlink());

    // deactivate: restore symlink → real directory (-y to skip prompt)
    clenv(&env)
        .args(["profile", "deactivate", "-y"])
        .assert()
        .success();

    // Afterward, should be a real directory, not a symlink
    assert!(!claude_home.is_symlink());
    assert!(claude_home.is_dir());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 3. Version control (VCS)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn test_vcs_status_clean() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "vcs-test");

    clenv(&env)
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("clean").or(predicate::str::contains("Nothing")));
}

#[test]
fn test_vcs_status_shows_changes() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "vcs-test");
    write_profile_file(&temp, "vcs-test", "new.md", "새 파일");

    clenv(&env)
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("new.md"));
}

#[test]
fn test_vcs_status_nonexistent_profile_fails() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    // Directly remove the active profile dir to simulate a broken state
    std::fs::remove_dir_all(temp.path().join(".clenv/profiles/default")).unwrap();

    clenv(&env).args(["status"]).assert().failure();
}

#[test]
fn test_vcs_commit() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "vcs-test");
    write_profile_file(&temp, "vcs-test", "file.md", "내용");

    clenv(&env)
        .args(["commit", "-m", "테스트 커밋"])
        .assert()
        .success()
        .stdout(predicate::str::contains("테스트 커밋"));

    // Clean state after commit
    clenv(&env)
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("clean").or(predicate::str::contains("Nothing")));
}

#[test]
fn test_vcs_commit_no_git_repo_fails() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    // Directly remove the active profile dir to simulate a broken state
    std::fs::remove_dir_all(temp.path().join(".clenv/profiles/default")).unwrap();

    clenv(&env)
        .args(["commit", "-m", "test"])
        .assert()
        .failure();
}

#[test]
fn test_vcs_diff() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "diff-test");
    write_profile_file(&temp, "diff-test", "CLAUDE.md", "원본 내용\n");
    clenv(&env)
        .args(["commit", "-m", "초기"])
        .assert()
        .success();

    write_profile_file(&temp, "diff-test", "CLAUDE.md", "수정된 내용\n새 줄\n");

    clenv(&env)
        .args(["diff"])
        .assert()
        .success()
        .stdout(predicate::str::contains("+").or(predicate::str::contains("수정")));
}

#[test]
fn test_vcs_diff_name_only() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "diff-test");
    write_profile_file(&temp, "diff-test", "changed.md", "원본");
    clenv(&env).args(["commit", "-m", "v1"]).assert().success();

    write_profile_file(&temp, "diff-test", "changed.md", "수정");

    clenv(&env)
        .args(["diff", "--name-only"])
        .assert()
        .success()
        .stdout(predicate::str::contains("changed.md"));
}

#[test]
fn test_vcs_log() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "log-test");

    for i in 1..=3 {
        write_profile_file(&temp, "log-test", "file.md", &format!("v{}", i));
        clenv(&env)
            .args(["commit", "-m", &format!("커밋 {}", i)])
            .assert()
            .success();
    }

    let out = clenv(&env).args(["log"]).assert().success();
    let stdout = stdout_of(&out);
    assert!(stdout.contains("커밋 1"));
    assert!(stdout.contains("커밋 2"));
    assert!(stdout.contains("커밋 3"));
}

#[test]
fn test_vcs_log_oneline() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "log-test");
    write_profile_file(&temp, "log-test", "f.md", "v1");
    clenv(&env)
        .args(["commit", "-m", "한줄 커밋"])
        .assert()
        .success();

    clenv(&env)
        .args(["log", "--oneline"])
        .assert()
        .success()
        .stdout(predicate::str::contains("한줄 커밋"));
}

#[test]
fn test_vcs_log_limit() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "log-test");

    for i in 1..=5 {
        write_profile_file(&temp, "log-test", "f.md", &format!("{}", i));
        clenv(&env)
            .args(["commit", "-m", &format!("커밋 {}", i)])
            .assert()
            .success();
    }

    let out = clenv(&env).args(["log", "-n", "2"]).assert().success();
    let stdout = stdout_of(&out);
    // Timestamps may match in fast tests so order is undefined — only validate count
    // Only the 2 most recent commits from HEAD should be returned
    assert!(
        stdout.contains("커밋 5"),
        "최신 커밋은 항상 포함: {}",
        stdout
    );
    // Verify no more than 2 commits are returned
    let commit_count = ["커밋 1", "커밋 2", "커밋 3", "커밋 4", "커밋 5"]
        .iter()
        .filter(|&&s| stdout.contains(s))
        .count();
    assert!(commit_count <= 2, "limit=2이면 최대 2개: stdout={}", stdout);
}

#[test]
fn test_vcs_tag_create_list() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "tag-test");
    write_profile_file(&temp, "tag-test", "f.md", "v1");
    clenv(&env)
        .args(["commit", "-m", "릴리즈"])
        .assert()
        .success();

    clenv(&env)
        .args(["tag", "v1.0.0", "-m", "첫 버전"])
        .assert()
        .success()
        .stdout(predicate::str::contains("created"));

    clenv(&env)
        .args(["tag", "--list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("v1.0.0"));
}

#[test]
fn test_vcs_tag_delete() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "tag-test");
    write_profile_file(&temp, "tag-test", "f.md", "content");
    clenv(&env)
        .args(["commit", "-m", "init"])
        .assert()
        .success();
    clenv(&env).args(["tag", "v1.0.0"]).assert().success();

    clenv(&env)
        .args(["tag", "v1.0.0", "--delete"])
        .assert()
        .success()
        .stdout(predicate::str::contains("deleted"));

    clenv(&env)
        .args(["tag", "--list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("v1.0.0").not());
}

#[test]
fn test_vcs_tag_duplicate_fails() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "tag-test");
    write_profile_file(&temp, "tag-test", "f.md", "content");
    clenv(&env)
        .args(["commit", "-m", "init"])
        .assert()
        .success();
    clenv(&env).args(["tag", "v1.0"]).assert().success();

    clenv(&env)
        .args(["tag", "v1.0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn test_vcs_checkout_by_tag() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "checkout-test");
    write_profile_file(&temp, "checkout-test", "CLAUDE.md", "v1 설정\n");
    clenv(&env).args(["commit", "-m", "v1"]).assert().success();
    clenv(&env).args(["tag", "v1.0"]).assert().success();

    write_profile_file(&temp, "checkout-test", "CLAUDE.md", "v2 설정\n");
    clenv(&env).args(["commit", "-m", "v2"]).assert().success();

    clenv(&env)
        .args(["checkout", "v1.0"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Checked out"));

    let content = read_profile_file(&temp, "checkout-test", "CLAUDE.md");
    assert!(
        content.contains("v1 설정"),
        "체크아웃 후 v1 내용이어야 함: {}",
        content
    );
}

#[test]
fn test_vcs_checkout_by_hash() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "hash-test");
    write_profile_file(&temp, "hash-test", "f.md", "initial");
    clenv(&env)
        .args(["commit", "-m", "init"])
        .assert()
        .success();

    // Extract hash from log
    let out = clenv(&env).args(["log", "--oneline"]).assert().success();
    let stdout = stdout_of(&out);
    let short_hash = stdout.split_whitespace().next().unwrap_or("").to_string();

    if !short_hash.is_empty() {
        clenv(&env)
            .args(["checkout", &short_hash])
            .assert()
            .success();
    }
}

#[test]
fn test_vcs_revert() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "revert-test");
    write_profile_file(&temp, "revert-test", "CLAUDE.md", "초기 내용\n");
    clenv(&env)
        .args(["commit", "-m", "초기"])
        .assert()
        .success();

    write_profile_file(&temp, "revert-test", "CLAUDE.md", "잘못된 변경\n");
    clenv(&env)
        .args(["commit", "-m", "잘못된 커밋"])
        .assert()
        .success();

    clenv(&env)
        .args(["revert"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Reverted"));

    // Revert commit should be added to log
    let out = clenv(&env).args(["log", "--oneline"]).assert().success();
    assert!(stdout_of(&out).contains("Revert"));
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 4. .clenvrc (automatic profile detection)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn test_rc_set_creates_file() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "rc-profile");

    let project_dir = temp.path().join("project");
    fs::create_dir_all(&project_dir).unwrap();

    clenv(&env)
        .args(["rc", "set", "rc-profile"])
        .current_dir(&project_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains(".clenvrc"));

    let rc_path = project_dir.join(".clenvrc");
    assert!(rc_path.exists());
    let content = fs::read_to_string(&rc_path).unwrap();
    assert!(content.contains("rc-profile"));
}

#[test]
fn test_rc_show() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "rc-profile");

    let project_dir = temp.path().join("project");
    fs::create_dir_all(&project_dir).unwrap();
    fs::write(project_dir.join(".clenvrc"), "rc-profile\n").unwrap();

    clenv(&env)
        .args(["rc", "show"])
        .current_dir(&project_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("rc-profile"));
}

#[test]
fn test_rc_unset_removes_file() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    let project_dir = temp.path().join("project");
    fs::create_dir_all(&project_dir).unwrap();
    fs::write(project_dir.join(".clenvrc"), "someprofile\n").unwrap();

    clenv(&env)
        .args(["rc", "unset"])
        .current_dir(&project_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("removed"));

    assert!(!project_dir.join(".clenvrc").exists());
}

#[test]
fn test_rc_unset_no_file_shows_info() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    let project_dir = temp.path().join("project");
    fs::create_dir_all(&project_dir).unwrap();

    clenv(&env)
        .args(["rc", "unset"])
        .current_dir(&project_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("No .clenvrc"));
}

#[test]
fn test_resolve_profile_from_rc_file() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "project-profile");

    let project_dir = temp.path().join("myproject");
    fs::create_dir_all(&project_dir).unwrap();
    fs::write(project_dir.join(".clenvrc"), "project-profile\n").unwrap();

    clenv(&env)
        .args(["resolve-profile"])
        .current_dir(&project_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("project-profile"));
}

#[test]
fn test_resolve_profile_quiet() {
    let temp = TempDir::new().unwrap();
    let mut env = setup_initialized_env(&temp);

    create_and_use(&env, "quiet-profile");
    env.insert("CLENV_PROFILE".to_string(), "quiet-profile".to_string());

    let out = clenv(&env)
        .args(["resolve-profile", "--quiet"])
        .assert()
        .success();

    assert_eq!(stdout_of(&out).trim(), "quiet-profile");
}

#[test]
fn test_resolve_profile_env_var_overrides_rc() {
    let temp = TempDir::new().unwrap();
    let mut env = setup_initialized_env(&temp);

    create_and_use(&env, "env-profile");
    clenv(&env)
        .args(["profile", "create", "rc-profile"])
        .assert()
        .success();

    let project_dir = temp.path().join("project");
    fs::create_dir_all(&project_dir).unwrap();
    fs::write(project_dir.join(".clenvrc"), "rc-profile\n").unwrap();

    // CLENV_PROFILE env var takes priority over .clenvrc
    env.insert("CLENV_PROFILE".to_string(), "env-profile".to_string());

    let out = clenv(&env)
        .args(["resolve-profile", "--quiet"])
        .current_dir(&project_dir)
        .assert()
        .success();

    assert_eq!(stdout_of(&out).trim(), "env-profile");
}

#[test]
fn test_resolve_profile_rc_in_parent_dir() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "parent-profile");

    let parent_dir = temp.path().join("parent");
    let child_dir = parent_dir.join("child").join("grandchild");
    fs::create_dir_all(&child_dir).unwrap();
    fs::write(parent_dir.join(".clenvrc"), "parent-profile\n").unwrap();

    // Running from a subdirectory should still detect the parent's .clenvrc
    clenv(&env)
        .args(["resolve-profile", "--quiet"])
        .current_dir(&child_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("parent-profile"));
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 5. Export / Import
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn test_profile_export_import_roundtrip() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "src");
    write_profile_file(
        &temp,
        "src",
        "CLAUDE.md",
        "# 내보내기 테스트\n\n중요한 설정\n",
    );
    clenv(&env)
        .args(["commit", "-m", "준비"])
        .assert()
        .success();

    let export_path = temp.path().join("export.clenvprofile");

    clenv(&env)
        .args([
            "profile",
            "export",
            "src",
            "--output",
            export_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Exported"));

    assert!(export_path.exists());

    clenv(&env)
        .args([
            "profile",
            "import",
            export_path.to_str().unwrap(),
            "--name",
            "imported",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("imported"));

    clenv(&env)
        .args(["profile", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("imported"));

    let content = read_profile_file(&temp, "imported", "CLAUDE.md");
    assert!(
        content.contains("내보내기 테스트"),
        "CLAUDE.md 내용 불일치: {}",
        content
    );
}

#[test]
fn test_profile_import_force_overwrites() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "base");
    write_profile_file(&temp, "base", "CLAUDE.md", "기본 내용\n");
    clenv(&env)
        .args(["commit", "-m", "init"])
        .assert()
        .success();

    let export_path = temp.path().join("base.clenvprofile");
    clenv(&env)
        .args([
            "profile",
            "export",
            "base",
            "--output",
            export_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    // First import
    clenv(&env)
        .args([
            "profile",
            "import",
            export_path.to_str().unwrap(),
            "--name",
            "target",
        ])
        .assert()
        .success();

    // Duplicate import → should fail
    clenv(&env)
        .args([
            "profile",
            "import",
            export_path.to_str().unwrap(),
            "--name",
            "target",
        ])
        .assert()
        .failure();

    // Overwrite with --force → should succeed
    clenv(&env)
        .args([
            "profile",
            "import",
            export_path.to_str().unwrap(),
            "--name",
            "target",
            "--force",
        ])
        .assert()
        .success();
}

#[test]
fn test_profile_export_mcp_keys_redacted() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "mcp-test");

    let settings = r#"{
  "mcpServers": {
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": {
        "GITHUB_TOKEN": "ghp_실제토큰값1234567890"
      }
    }
  }
}"#;
    write_profile_file(&temp, "mcp-test", "settings.json", settings);
    clenv(&env)
        .args(["commit", "-m", "mcp 설정"])
        .assert()
        .success();

    let export_path = temp.path().join("mcp.clenvprofile");
    clenv(&env)
        .args([
            "profile",
            "export",
            "mcp-test",
            "--output",
            export_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    // Verify API key is removed from settings.json after import
    clenv(&env)
        .args([
            "profile",
            "import",
            export_path.to_str().unwrap(),
            "--name",
            "mcp-imported",
        ])
        .assert()
        .success();

    let imported_settings = read_profile_file(&temp, "mcp-imported", "settings.json");
    assert!(
        !imported_settings.contains("ghp_실제토큰값1234567890"),
        "실제 API 키가 포함됨: {}",
        imported_settings
    );
    assert!(
        imported_settings.contains("${GITHUB_TOKEN}"),
        "Placeholder가 없음: {}",
        imported_settings
    );
}

#[test]
fn test_profile_export_active_profile_by_default() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "active-export");
    write_profile_file(&temp, "active-export", "CLAUDE.md", "활성 프로필 내용\n");
    clenv(&env)
        .args(["commit", "-m", "init"])
        .assert()
        .success();

    // Omit profile name → export the currently active profile
    let export_path = temp.path().join("active.clenvprofile");
    clenv(&env)
        .args([
            "profile",
            "export",
            "--output",
            export_path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Exported"));

    assert!(export_path.exists());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 7. MCP settings swap
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn test_mcp_swap_on_profile_switch() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    // Create ~/.claude.json
    let claude_json = temp.path().join(".claude.json");
    fs::write(&claude_json, r#"{"mcpServers": {}}"#).unwrap();

    create_and_use(&env, "profile-a");

    // MCP settings for profile-a
    write_profile_file(
        &temp,
        "profile-a",
        "user-mcp.json",
        r#"{"server-a": {"command": "cmd-a"}}"#,
    );

    clenv(&env)
        .args(["profile", "create", "profile-b"])
        .assert()
        .success();

    // Switch to profile-b → back up profile-a MCP, apply profile-b MCP
    clenv(&env)
        .args(["profile", "use", "profile-b"])
        .assert()
        .success();

    // Switch back to profile-a → restore profile-a MCP
    clenv(&env)
        .args(["profile", "use", "profile-a"])
        .assert()
        .success();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 8. Doctor
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// init: edge cases
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Build a realistic full ~/.claude/ structure covering all user-settable areas.
/// Used by comprehensive init tests.
fn setup_full_claude_dir(temp: &TempDir) {
    let claude = temp.path().join(".claude");
    fs::create_dir_all(&claude).unwrap();

    // ── Core config ──────────────────────────────────────────────────────────
    fs::write(
        claude.join("CLAUDE.md"),
        "# My Claude Config\n\nAlways respond in Korean.\n",
    )
    .unwrap();
    fs::write(
        claude.join("settings.json"),
        r#"{"permissions":{"allow":["bash"],"deny":[]}}"#,
    )
    .unwrap();

    // ── hooks/ ───────────────────────────────────────────────────────────────
    let hooks = claude.join("hooks");
    fs::create_dir_all(&hooks).unwrap();
    fs::write(hooks.join("pre-tool-use.sh"), "#!/bin/bash\necho pre").unwrap();
    fs::write(hooks.join("post-tool-use.sh"), "#!/bin/bash\necho post").unwrap();
    fs::write(hooks.join("session-start.sh"), "#!/bin/bash\necho start").unwrap();
    fs::write(hooks.join("session-end.sh"), "#!/bin/bash\necho end").unwrap();

    // ── plugins/ — local (regular dir) + marketplace (nested git repos) ──────
    let plugins = claude.join("plugins");
    let local_plugin = plugins.join("my-local-plugin");
    fs::create_dir_all(&local_plugin).unwrap();
    fs::write(
        local_plugin.join("plugin.json"),
        r#"{"name":"my-plugin","version":"1.0"}"#,
    )
    .unwrap();
    fs::write(local_plugin.join("README.md"), "# My Local Plugin").unwrap();
    fs::write(local_plugin.join("main.js"), "module.exports = {};").unwrap();

    // Marketplace plugins are nested git repos (the real-world scenario)
    create_nested_git_repo(temp, "plugins/marketplaces/claude-plugins-official");
    create_nested_git_repo(temp, "plugins/marketplaces/omc");
    create_nested_git_repo(temp, "plugins/marketplaces/superpowers-marketplace");
    create_nested_git_repo(temp, "plugins/marketplaces/plannotator");
    // Add files inside a marketplace repo to verify content survives stripping
    fs::write(
        temp.path()
            .join(".claude/plugins/marketplaces/omc/README.md"),
        "# OMC marketplace plugins",
    )
    .unwrap();
    fs::create_dir_all(temp.path().join(".claude/plugins/marketplaces/omc/plugins")).unwrap();
    fs::write(
        temp.path()
            .join(".claude/plugins/marketplaces/omc/plugins/my-omc-plugin.json"),
        r#"{"id":"my-omc-plugin"}"#,
    )
    .unwrap();

    // ── skills/ ──────────────────────────────────────────────────────────────
    let skills = claude.join("skills");
    fs::create_dir_all(&skills).unwrap();
    fs::write(skills.join("daily.md"), "# Daily Skill\n\nRun daily tasks.").unwrap();
    fs::write(skills.join("review.md"), "# Review Skill\n\nReview PRs.").unwrap();
    fs::write(skills.join("git-commit.md"), "# Git Commit Skill").unwrap();

    // ── agents/ ──────────────────────────────────────────────────────────────
    let agents = claude.join("agents");
    fs::create_dir_all(&agents).unwrap();
    fs::write(
        agents.join("researcher.md"),
        "# Researcher\n\nYou are a researcher.",
    )
    .unwrap();
    fs::write(agents.join("coder.md"), "# Coder\n\nYou write code.").unwrap();

    // ── Shared files (must move to ~/.clenv/shared/, not stay in profile) ────
    fs::write(claude.join("history.jsonl"), "{\"cmd\":\"echo hello\"}\n").unwrap();
    fs::write(claude.join("stats-cache.json"), r#"{"version":1}"#).unwrap();
    fs::write(claude.join(".session-stats.json"), r#"{"sessions":5}"#).unwrap();
    fs::write(claude.join("mcp-needs-auth-cache.json"), r#"{}"#).unwrap();

    // ── Excluded dirs (cache/temp — must NOT appear in profile) ──────────────
    for (dir, file, content) in &[
        ("projects/my-project", "CLAUDE.md", "project data"),
        ("debug", "log.txt", "debug output"),
        ("telemetry", "events.json", "{}"),
        ("shell-snapshots", "snap.json", "{}"),
        ("backups", "bak.tar.gz", "backup"),
        ("file-history", "file.json", "{}"),
        ("cache", "cache.bin", "cached"),
        ("statsig", "config.json", "{}"),
        ("paste-cache", "paste.txt", "pasted"),
        ("session-env", "env.json", "{}"),
    ] {
        let d = claude.join(dir);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(file), content).unwrap();
    }
}

// ── Individual init copy tests ────────────────────────────────────────────────

/// Core config files (CLAUDE.md, settings.json) are copied to the default profile.
#[test]
fn test_init_copies_core_config_files() {
    let temp = TempDir::new().unwrap();
    let env = setup_test_env(&temp);

    let claude = temp.path().join(".claude");
    fs::create_dir_all(&claude).unwrap();
    fs::write(claude.join("CLAUDE.md"), "# My Claude Config\n").unwrap();
    fs::write(
        claude.join("settings.json"),
        r#"{"permissions":{"allow":["bash"],"deny":[]}}"#,
    )
    .unwrap();

    clenv(&env).arg("init").assert().success();

    let profile = temp.path().join(".clenv/profiles/default");
    assert!(profile.join("CLAUDE.md").exists(), "CLAUDE.md missing");
    assert!(
        profile.join("settings.json").exists(),
        "settings.json missing"
    );

    let md = fs::read_to_string(profile.join("CLAUDE.md")).unwrap();
    assert!(
        md.contains("My Claude Config"),
        "CLAUDE.md content mismatch"
    );
    let st = fs::read_to_string(profile.join("settings.json")).unwrap();
    assert!(st.contains("bash"), "settings.json content mismatch");
}

/// hooks/ directory with all hook scripts is fully copied to the profile.
#[test]
fn test_init_copies_hooks_directory() {
    let temp = TempDir::new().unwrap();
    let env = setup_test_env(&temp);

    let hooks = temp.path().join(".claude/hooks");
    fs::create_dir_all(&hooks).unwrap();
    fs::write(hooks.join("pre-tool-use.sh"), "#!/bin/bash\necho pre").unwrap();
    fs::write(hooks.join("post-tool-use.sh"), "#!/bin/bash\necho post").unwrap();
    fs::write(hooks.join("session-start.sh"), "#!/bin/bash\necho start").unwrap();
    fs::write(hooks.join("session-end.sh"), "#!/bin/bash\necho end").unwrap();

    clenv(&env).arg("init").assert().success();

    let ph = temp.path().join(".clenv/profiles/default/hooks");
    assert!(ph.is_dir(), "hooks/ missing from profile");
    for hook in &[
        "pre-tool-use.sh",
        "post-tool-use.sh",
        "session-start.sh",
        "session-end.sh",
    ] {
        assert!(ph.join(hook).exists(), "{hook} missing");
    }
    let content = fs::read_to_string(ph.join("pre-tool-use.sh")).unwrap();
    assert!(content.contains("echo pre"), "hook content mismatch");
}

/// skills/ and agents/ directories with content files are fully copied.
#[test]
fn test_init_copies_skills_and_agents() {
    let temp = TempDir::new().unwrap();
    let env = setup_test_env(&temp);

    let skills = temp.path().join(".claude/skills");
    fs::create_dir_all(&skills).unwrap();
    fs::write(skills.join("daily.md"), "# Daily Skill\n\nRun daily tasks.").unwrap();
    fs::write(skills.join("review.md"), "# Review Skill").unwrap();

    let agents = temp.path().join(".claude/agents");
    fs::create_dir_all(&agents).unwrap();
    fs::write(agents.join("researcher.md"), "# Researcher Agent").unwrap();
    fs::write(agents.join("coder.md"), "# Coder Agent").unwrap();

    clenv(&env).arg("init").assert().success();

    let profile = temp.path().join(".clenv/profiles/default");
    assert!(
        profile.join("skills/daily.md").exists(),
        "skills/daily.md missing"
    );
    assert!(
        profile.join("skills/review.md").exists(),
        "skills/review.md missing"
    );
    let skill = fs::read_to_string(profile.join("skills/daily.md")).unwrap();
    assert!(skill.contains("Daily Skill"), "skill content mismatch");

    assert!(
        profile.join("agents/researcher.md").exists(),
        "agents/researcher.md missing"
    );
    assert!(
        profile.join("agents/coder.md").exists(),
        "agents/coder.md missing"
    );
}

/// Local plugins (plain directory, not a git repo) are copied to the profile.
#[test]
fn test_init_copies_local_plugins() {
    let temp = TempDir::new().unwrap();
    let env = setup_test_env(&temp);

    let plugin = temp.path().join(".claude/plugins/my-local-plugin");
    fs::create_dir_all(&plugin).unwrap();
    fs::write(
        plugin.join("plugin.json"),
        r#"{"name":"my-plugin","version":"1.0"}"#,
    )
    .unwrap();
    fs::write(plugin.join("README.md"), "# My Local Plugin").unwrap();
    fs::write(plugin.join("main.js"), "module.exports = {};").unwrap();

    clenv(&env).arg("init").assert().success();

    let pp = temp
        .path()
        .join(".clenv/profiles/default/plugins/my-local-plugin");
    assert!(pp.is_dir(), "local plugin dir missing from profile");
    assert!(pp.join("plugin.json").exists(), "plugin.json missing");
    assert!(pp.join("README.md").exists(), "README.md missing");
    assert!(pp.join("main.js").exists(), "main.js missing");
    let content = fs::read_to_string(pp.join("plugin.json")).unwrap();
    assert!(
        content.contains("my-plugin"),
        "plugin.json content mismatch"
    );
}

/// Marketplace plugins (nested git repos) are copied but their .git dirs are stripped.
/// Files inside the marketplace repo must survive the copy.
#[test]
fn test_init_marketplace_plugins_content_preserved_without_git() {
    let temp = TempDir::new().unwrap();
    let env = setup_test_env(&temp);

    create_nested_git_repo(&temp, "plugins/marketplaces/omc");
    // Add real files inside the marketplace repo
    fs::create_dir_all(temp.path().join(".claude/plugins/marketplaces/omc/plugins")).unwrap();
    fs::write(
        temp.path()
            .join(".claude/plugins/marketplaces/omc/README.md"),
        "# OMC marketplace",
    )
    .unwrap();
    fs::write(
        temp.path()
            .join(".claude/plugins/marketplaces/omc/plugins/my-plugin.json"),
        r#"{"id":"my-omc-plugin"}"#,
    )
    .unwrap();

    clenv(&env).arg("init").assert().success();

    let market = temp
        .path()
        .join(".clenv/profiles/default/plugins/marketplaces/omc");
    // Files present
    assert!(
        market.join("README.md").exists(),
        "marketplace README.md missing"
    );
    assert!(
        market.join("plugins/my-plugin.json").exists(),
        "marketplace plugin file missing"
    );
    let content = fs::read_to_string(market.join("plugins/my-plugin.json")).unwrap();
    assert!(content.contains("my-omc-plugin"), "plugin content mismatch");
    // .git stripped
    assert!(
        !market.join(".git").exists(),
        ".git must be stripped from marketplace plugin"
    );
}

/// All EXCLUDED_DIRS (Claude Code cache/temp) must not appear in the default profile.
#[test]
fn test_init_excludes_cache_and_temp_dirs() {
    let temp = TempDir::new().unwrap();
    let env = setup_test_env(&temp);

    let claude = temp.path().join(".claude");
    fs::create_dir_all(&claude).unwrap();
    fs::write(claude.join("CLAUDE.md"), "# config").unwrap();

    // Create every EXCLUDED_DIR with a sentinel file
    let excluded = [
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
    for dir in &excluded {
        let d = claude.join(dir);
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("sentinel.txt"), "should-not-copy").unwrap();
    }

    clenv(&env).arg("init").assert().success();

    let profile = temp.path().join(".clenv/profiles/default");
    for dir in &excluded {
        assert!(
            !profile.join(dir).exists(),
            "EXCLUDED_DIR '{dir}' must not appear in profile"
        );
    }
}

/// SHARED_FILES (history.jsonl, stats-cache.json, etc.) must be migrated to
/// ~/.clenv/shared/ and must not remain as plain files in the profile.
#[test]
fn test_init_moves_shared_files_to_shared_dir() {
    let temp = TempDir::new().unwrap();
    let env = setup_test_env(&temp);

    let claude = temp.path().join(".claude");
    fs::create_dir_all(&claude).unwrap();
    fs::write(claude.join("CLAUDE.md"), "# config").unwrap();
    fs::write(claude.join("history.jsonl"), "{\"cmd\":\"echo hello\"}\n").unwrap();
    fs::write(claude.join("stats-cache.json"), r#"{"version":1}"#).unwrap();
    fs::write(claude.join(".session-stats.json"), r#"{"sessions":5}"#).unwrap();
    fs::write(claude.join("mcp-needs-auth-cache.json"), r#"{}"#).unwrap();

    clenv(&env).arg("init").assert().success();

    let shared = temp.path().join(".clenv/shared");

    assert!(
        shared.join("history.jsonl").exists(),
        "history.jsonl missing from shared"
    );
    let hist = fs::read_to_string(shared.join("history.jsonl")).unwrap();
    assert!(
        hist.contains("echo hello"),
        "history.jsonl content not preserved"
    );

    assert!(
        shared.join("stats-cache.json").exists(),
        "stats-cache.json missing from shared"
    );
    let stats = fs::read_to_string(shared.join("stats-cache.json")).unwrap();
    assert!(
        stats.contains("version"),
        "stats-cache.json content not preserved"
    );
}

/// After init, the default profile must contain zero nested .git directories
/// regardless of how many marketplace plugins (nested git repos) were present.
#[test]
fn test_init_all_nested_git_dirs_stripped_from_profile() {
    let temp = TempDir::new().unwrap();
    let env = setup_test_env(&temp);

    // Plant nested git repos in ALL marketplace slots (mirrors real ~/.claude/)
    for slug in &[
        "plugins/marketplaces/claude-plugins-official",
        "plugins/marketplaces/omc",
        "plugins/marketplaces/superpowers-marketplace",
        "plugins/marketplaces/plannotator",
    ] {
        create_nested_git_repo(&temp, slug);
    }
    // A nested git repo even deeper (edge case)
    create_nested_git_repo(&temp, "plugins/custom-git-plugin");

    fs::write(temp.path().join(".claude/CLAUDE.md"), "# config").unwrap();

    clenv(&env).arg("init").assert().success();

    let profile = temp.path().join(".clenv/profiles/default");

    // min_depth(2) skips the profile's own top-level .git (depth 1, created by vcs.init()).
    // Only .git dirs inside subdirectories (depth ≥ 2) are considered nested.
    let nested_git_count = WalkDir::new(&profile)
        .min_depth(2)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name() == ".git" && e.file_type().is_dir())
        .count();

    assert_eq!(
        nested_git_count, 0,
        "Profile must have no nested .git directories, found {nested_git_count}"
    );
}

/// Regression: ~/.claude/ containing nested git repos (e.g. plugin marketplaces)
/// must not cause `clenv init` to fail with "invalid path: class=Index".
#[test]
fn test_init_with_nested_git_repo_in_claude_dir() {
    let temp = TempDir::new().unwrap();
    let env = setup_test_env(&temp);

    // Simulate plugins/marketplaces/some-plugin/ being its own git repo
    create_nested_git_repo(&temp, "plugins/marketplaces/claude-plugins-official");
    create_nested_git_repo(&temp, "plugins/marketplaces/omc");
    fs::write(temp.path().join(".claude/CLAUDE.md"), "# existing config").unwrap();

    clenv(&env).arg("init").assert().success();
}

/// Comprehensive: full realistic ~/.claude/ structure — all areas must copy correctly.
#[test]
fn test_init_full_claude_dir_structure() {
    let temp = TempDir::new().unwrap();
    let env = setup_test_env(&temp);

    setup_full_claude_dir(&temp);

    clenv(&env).arg("init").assert().success();

    let profile = temp.path().join(".clenv/profiles/default");
    let shared = temp.path().join(".clenv/shared");

    // ── Core config ──────────────────────────────────────────────────────────
    assert!(profile.join("CLAUDE.md").exists(), "CLAUDE.md missing");
    assert!(
        profile.join("settings.json").exists(),
        "settings.json missing"
    );
    let md = fs::read_to_string(profile.join("CLAUDE.md")).unwrap();
    assert!(
        md.contains("My Claude Config"),
        "CLAUDE.md content mismatch"
    );

    // ── hooks/ ───────────────────────────────────────────────────────────────
    for hook in &[
        "pre-tool-use.sh",
        "post-tool-use.sh",
        "session-start.sh",
        "session-end.sh",
    ] {
        assert!(
            profile.join("hooks").join(hook).exists(),
            "hooks/{hook} missing"
        );
    }

    // ── skills/ ──────────────────────────────────────────────────────────────
    assert!(
        profile.join("skills/daily.md").exists(),
        "skills/daily.md missing"
    );
    assert!(
        profile.join("skills/review.md").exists(),
        "skills/review.md missing"
    );
    assert!(
        profile.join("skills/git-commit.md").exists(),
        "skills/git-commit.md missing"
    );
    let skill = fs::read_to_string(profile.join("skills/daily.md")).unwrap();
    assert!(skill.contains("Daily Skill"), "skill content mismatch");

    // ── agents/ ──────────────────────────────────────────────────────────────
    assert!(
        profile.join("agents/researcher.md").exists(),
        "agents/researcher.md missing"
    );
    assert!(
        profile.join("agents/coder.md").exists(),
        "agents/coder.md missing"
    );

    // ── plugins/ — local ─────────────────────────────────────────────────────
    assert!(
        profile.join("plugins/my-local-plugin/plugin.json").exists(),
        "local plugin missing"
    );
    assert!(
        profile.join("plugins/my-local-plugin/main.js").exists(),
        "plugin main.js missing"
    );

    // ── plugins/ — marketplace content present (files survive .git strip) ───
    assert!(
        profile.join("plugins/marketplaces/omc/README.md").exists(),
        "marketplace README.md missing"
    );
    assert!(
        profile
            .join("plugins/marketplaces/omc/plugins/my-omc-plugin.json")
            .exists(),
        "marketplace plugin file missing"
    );

    // ── No nested .git inside subdirectories ─────────────────────────────────
    // min_depth(2): skip the profile's own top-level .git (depth 1).
    let git_count = WalkDir::new(&profile)
        .min_depth(2)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name() == ".git" && e.file_type().is_dir())
        .count();
    assert_eq!(git_count, 0, "Profile must have no nested .git dirs");

    // ── Shared files migrated ─────────────────────────────────────────────────
    assert!(
        shared.join("history.jsonl").exists(),
        "history.jsonl missing from shared"
    );
    assert!(
        shared.join("stats-cache.json").exists(),
        "stats-cache.json missing from shared"
    );

    // ── EXCLUDED_DIRS absent from profile ────────────────────────────────────
    for dir in &["projects", "debug", "telemetry", "cache", "statsig"] {
        assert!(
            !profile.join(dir).exists(),
            "'{dir}' must not be in profile"
        );
    }

    // ── Profile is a valid git repo with at least one commit ─────────────────
    assert!(profile.join(".git").is_dir(), "Profile must be a git repo");
    clenv(&env).args(["status"]).assert().success();
}

#[test]
fn test_doctor_no_profiles() {
    let temp = TempDir::new().unwrap();
    // Doctor bypasses the init check, so setup_test_env is sufficient
    let env = setup_test_env(&temp);

    clenv(&env).args(["doctor"]).assert().success();
}

#[test]
fn test_doctor_healthy() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    create_and_use(&env, "healthy");

    clenv(&env)
        .args(["doctor"])
        .assert()
        .success()
        .stdout(predicate::str::contains("All").or(predicate::str::contains("passed")));
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 9. Verify default file generation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn test_profile_create_generates_default_files() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    clenv(&env)
        .args(["profile", "create", "newprofile"])
        .assert()
        .success();

    let profile_dir = temp.path().join(".clenv/profiles/newprofile");

    // Verify default files and directories exist
    assert!(profile_dir.join("CLAUDE.md").exists(), "CLAUDE.md 없음");
    assert!(
        profile_dir.join("settings.json").exists(),
        "settings.json 없음"
    );
    assert!(profile_dir.join("hooks").is_dir(), "hooks 디렉토리 없음");
    assert!(profile_dir.join("agents").is_dir(), "agents 디렉토리 없음");
    assert!(profile_dir.join("skills").is_dir(), "skills 디렉토리 없음");

    // Verify git repo initialization
    assert!(profile_dir.join(".git").is_dir(), ".git 없음");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 10. Full lifecycle integration
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn test_full_lifecycle() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    // 1. Create and activate profile
    create_and_use(&env, "main");

    // 2. Add file → commit
    write_profile_file(&temp, "main", "CLAUDE.md", "# Main 프로필\n");
    clenv(&env)
        .args(["commit", "-m", "초기 설정"])
        .assert()
        .success();

    // 3. Tag
    clenv(&env)
        .args(["tag", "v1.0", "-m", "v1 릴리즈"])
        .assert()
        .success();

    // 4. Modify → commit
    write_profile_file(&temp, "main", "CLAUDE.md", "# Main 프로필\n\n추가 설정\n");
    clenv(&env)
        .args(["commit", "-m", "설정 추가"])
        .assert()
        .success();

    // 5. Export
    let export_path = temp.path().join("main.clenvprofile");
    clenv(&env)
        .args([
            "profile",
            "export",
            "main",
            "--output",
            export_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    // 6. Clone
    clenv(&env)
        .args(["profile", "clone", "main", "main-copy"])
        .assert()
        .success();

    // 7. Import
    clenv(&env)
        .args([
            "profile",
            "import",
            export_path.to_str().unwrap(),
            "--name",
            "restored",
        ])
        .assert()
        .success();

    // 8. Verify list (default + main + main-copy + restored = 4)
    let out = clenv(&env).args(["profile", "list"]).assert().success();
    let stdout = stdout_of(&out);
    assert!(stdout.contains("main"));
    assert!(stdout.contains("main-copy"));
    assert!(stdout.contains("restored"));
    assert!(stdout.contains("4"));

    // 9. Set per-project profile with rc
    let project_dir = temp.path().join("project");
    fs::create_dir_all(&project_dir).unwrap();
    clenv(&env)
        .args(["rc", "set", "main"])
        .current_dir(&project_dir)
        .assert()
        .success();

    // 10. Doctor
    clenv(&env).args(["doctor"]).assert().success();
}
