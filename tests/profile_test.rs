// tests/profile_test.rs
// Profile management integration tests
//
// Tests that use the real filesystem.
// Each test creates a temporary directory and runs independently.

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// Build a clenv Command (avoids the deprecated cargo_bin API)
fn clenv_cmd() -> Command {
    cargo_bin_cmd!("clenv")
}

/// Test environment setup
/// Sets CLENV_HOME to a temporary directory to avoid touching real settings
fn setup_test_env(temp: &TempDir) -> std::collections::HashMap<String, String> {
    let mut env = std::collections::HashMap::new();
    env.insert(
        "CLENV_HOME".to_string(),
        temp.path().join(".clenv").to_str().unwrap().to_string(),
    );
    env.insert(
        "CLAUDE_HOME".to_string(),
        temp.path().join(".claude").to_str().unwrap().to_string(),
    );
    env
}

/// Initialize clenv in the test environment (required before any command)
fn init_clenv(env: &std::collections::HashMap<String, String>) {
    clenv_cmd().envs(env).arg("init").assert().success();
}

/// Set up an initialized test environment
fn setup_initialized_env(temp: &TempDir) -> std::collections::HashMap<String, String> {
    let env = setup_test_env(temp);
    init_clenv(&env);
    env
}

#[test]
fn test_profile_create() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    clenv_cmd()
        .envs(&env)
        .args(["profile", "create", "test-profile"])
        .assert()
        .success()
        .stdout(predicate::str::contains("created"));
}

#[test]
fn test_profile_list_shows_default() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    // After init, default profile always exists
    clenv_cmd()
        .envs(&env)
        .args(["profile", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("default"));
}

#[test]
fn test_profile_create_and_list() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    // Create profile
    clenv_cmd()
        .envs(&env)
        .args(["profile", "create", "work"])
        .assert()
        .success();

    // Check list
    clenv_cmd()
        .envs(&env)
        .args(["profile", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("work"));
}

#[test]
fn test_profile_use() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    // Create profile
    clenv_cmd()
        .envs(&env)
        .args(["profile", "create", "work"])
        .assert()
        .success();

    // Switch profile
    clenv_cmd()
        .envs(&env)
        .args(["profile", "use", "work"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Active profile"));
}

#[test]
fn test_profile_delete() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    // Create profile
    clenv_cmd()
        .envs(&env)
        .args(["profile", "create", "to-delete"])
        .assert()
        .success();

    // Delete profile (skip confirmation with --force)
    clenv_cmd()
        .envs(&env)
        .args(["profile", "delete", "to-delete", "--force"])
        .assert()
        .success()
        .stdout(predicate::str::contains("deleted"));
}

#[test]
fn test_profile_duplicate_name_fails() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    // Create profile
    clenv_cmd()
        .envs(&env)
        .args(["profile", "create", "duplicate"])
        .assert()
        .success();

    // Create with same name again → should fail
    clenv_cmd()
        .envs(&env)
        .args(["profile", "create", "duplicate"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn test_commit_and_log() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    // Create and activate profile
    clenv_cmd()
        .envs(&env)
        .args(["profile", "create", "work"])
        .assert()
        .success();

    clenv_cmd()
        .envs(&env)
        .args(["profile", "use", "work"])
        .assert()
        .success();

    // Commit changes
    clenv_cmd()
        .envs(&env)
        .args(["commit", "-m", "첫 번째 커밋"])
        .assert()
        .success();

    // Check log
    clenv_cmd()
        .envs(&env)
        .args(["log"])
        .assert()
        .success()
        .stdout(predicate::str::contains("첫 번째 커밋"));
}

#[test]
fn test_profile_clone() {
    let temp = TempDir::new().unwrap();
    let env = setup_initialized_env(&temp);

    // Create original profile
    clenv_cmd()
        .envs(&env)
        .args(["profile", "create", "original"])
        .assert()
        .success();

    // Clone
    clenv_cmd()
        .envs(&env)
        .args(["profile", "clone", "original", "copy"])
        .assert()
        .success()
        .stdout(predicate::str::contains("cloned"));

    // 'copy' should appear in the list
    clenv_cmd()
        .envs(&env)
        .args(["profile", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("copy"));
}
