// src/dirs.rs
// Path management module
//
// All directory and file paths used by clenv are managed here in one place.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

// ── clenv internal directories ────────────────────────────────────────────────

/// clenv root directory: ~/.clenv/
/// Uses CLENV_HOME env var if set (useful for testing and custom setups)
pub fn clenv_home() -> PathBuf {
    if let Ok(custom) = std::env::var("CLENV_HOME") {
        return PathBuf::from(custom);
    }
    dirs::home_dir()
        .expect("Home directory not found")
        .join(".clenv")
}

/// Directory for all profiles: ~/.clenv/profiles/
pub fn profiles_dir() -> PathBuf {
    clenv_home().join("profiles")
}

/// Directory for a specific profile: ~/.clenv/profiles/<name>/
pub fn profile_dir(name: &str) -> PathBuf {
    profiles_dir().join(name)
}

/// clenv global config file: ~/.clenv/config.toml
pub fn config_file() -> PathBuf {
    clenv_home().join("config.toml")
}

// ── Claude Code directories ───────────────────────────────────────────────────

/// Claude Code global config file: ~/.claude.json
pub fn claude_json() -> PathBuf {
    dirs::home_dir()
        .expect("Home directory not found")
        .join(".claude.json")
}

/// Per-profile user-level MCP storage: ~/.clenv/profiles/<name>/user-mcp.json
pub fn user_mcp_file(name: &str) -> PathBuf {
    profile_dir(name).join("user-mcp.json")
}

/// Shared directory for account-level state that persists across all profiles: ~/.clenv/shared/
pub fn shared_dir() -> PathBuf {
    clenv_home().join("shared")
}

/// Path to a specific shared file: ~/.clenv/shared/<name>
#[allow(dead_code)]
pub fn shared_file(name: &str) -> PathBuf {
    shared_dir().join(name)
}

/// Claude Code config directory: ~/.claude/
/// Uses CLAUDE_HOME env var if set
pub fn claude_home() -> PathBuf {
    if let Ok(custom) = std::env::var("CLAUDE_HOME") {
        return PathBuf::from(custom);
    }
    dirs::home_dir()
        .expect("Home directory not found")
        .join(".claude")
}

/// Path to CLAUDE.md
#[allow(dead_code)]
pub fn claude_md() -> PathBuf {
    claude_home().join("CLAUDE.md")
}

/// Path to Claude settings file
#[allow(dead_code)]
pub fn claude_settings() -> PathBuf {
    claude_home().join("settings.json")
}

/// Path to Claude keybindings file
#[allow(dead_code)]
pub fn claude_keybindings() -> PathBuf {
    claude_home().join("keybindings.json")
}

/// Claude hooks directory
#[allow(dead_code)]
pub fn claude_hooks_dir() -> PathBuf {
    claude_home().join("hooks")
}

/// Claude agents directory
#[allow(dead_code)]
pub fn claude_agents_dir() -> PathBuf {
    claude_home().join("agents")
}

/// Claude skills directory
#[allow(dead_code)]
pub fn claude_skills_dir() -> PathBuf {
    claude_home().join("skills")
}

// ── .clenvrc lookup ───────────────────────────────────────────────────────────

/// Walk up from the given directory looking for a .clenvrc file.
/// Returns the path if found, None otherwise.
pub fn find_clenvrc(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    // Only search within the home directory (do not walk all the way to root)
    let home = dirs::home_dir()?;

    loop {
        let candidate = current.join(".clenvrc");
        if candidate.exists() {
            return Some(candidate);
        }

        // Stop if we've reached the home directory without finding one
        if current == home {
            break;
        }

        // Move up one level
        if !current.pop() {
            break;
        }
    }

    None
}

/// Global .clenvrc path: ~/.clenvrc
pub fn global_clenvrc() -> PathBuf {
    dirs::home_dir()
        .expect("Home directory not found")
        .join(".clenvrc")
}

// ── Initialization ────────────────────────────────────────────────────────────

/// Initialize the clenv directory structure
pub fn ensure_initialized() -> Result<()> {
    let dirs_to_create = [clenv_home(), profiles_dir()];

    for dir in &dirs_to_create {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("Failed to create directory: {}", dir.display()))?;
    }

    Ok(())
}

/// Return the user's home directory (wrapper around the dirs crate)
pub fn home_dir() -> PathBuf {
    dirs::home_dir().expect("Home directory not found")
}

/// Check whether ~/.claude/ exists
#[allow(dead_code)]
pub fn claude_home_exists() -> bool {
    claude_home().exists()
}

/// Check whether ~/.claude/ is currently a symlink managed by clenv
pub fn is_managed_by_clenv() -> bool {
    let claude = claude_home();
    if let Ok(meta) = std::fs::symlink_metadata(&claude) {
        meta.file_type().is_symlink()
    } else {
        false
    }
}

/// Returns the backup directory: ~/.clenv/backup/
pub fn backup_dir() -> PathBuf {
    clenv_home().join("backup")
}

/// Returns the original backup directory: ~/.clenv/backup/original/
pub fn backup_original_dir() -> PathBuf {
    backup_dir().join("original")
}

/// Returns the backup manifest path: ~/.clenv/backup/manifest.json
pub fn backup_manifest() -> PathBuf {
    backup_dir().join("manifest.json")
}

/// Returns the init marker path: ~/.clenv/.initialized
pub fn init_marker() -> PathBuf {
    clenv_home().join(".initialized")
}

/// Check whether clenv has been initialized
pub fn is_clenv_initialized() -> bool {
    init_marker().exists()
}

/// Return the real path that the symlink points to
pub fn active_profile_path() -> Option<PathBuf> {
    if is_managed_by_clenv() {
        std::fs::read_link(claude_home()).ok()
    } else {
        None
    }
}

/// Extract the active profile name from the symlink target
pub fn active_profile_name() -> Option<String> {
    active_profile_path().and_then(|path| {
        path.file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
    })
}

// ── Utilities ─────────────────────────────────────────────────────────────────

/// Display a path relative to the home directory (~/... format)
#[allow(dead_code)]
pub fn display_path(path: &std::path::Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rel) = path.strip_prefix(&home) {
            return format!("~/{}", rel.display());
        }
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn test_profile_dir() {
        let dir = profile_dir("work");
        assert!(dir.to_str().unwrap().contains("profiles"));
        assert!(dir.to_str().unwrap().contains("work"));
    }

    #[test]
    #[serial]
    fn test_clenv_home_env() {
        std::env::set_var("CLENV_HOME", "/tmp/test-clenv");
        assert_eq!(clenv_home(), PathBuf::from("/tmp/test-clenv"));
        std::env::remove_var("CLENV_HOME");
    }

    #[test]
    fn test_find_clenvrc_not_found() {
        // /tmp is unlikely to have a .clenvrc
        let result = find_clenvrc(Path::new("/tmp"));
        // search is limited to below home dir, so /tmp returns None
        assert!(result.is_none());
    }
}
