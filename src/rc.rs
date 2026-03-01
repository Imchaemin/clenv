// src/rc.rs
// Profile auto-detection based on .clenvrc
//
// Like nvm's .nvmrc, specifies a profile per directory (repo).
//
// Priority (highest first):
//   1. CLENV_PROFILE environment variable
//   2. .clenvrc found in current or parent directories
//   3. ~/.clenvrc (global home directory)
//   4. active_profile in config.toml (global default)

use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::dirs;

/// Source that determined the active profile
#[derive(Debug, Clone, PartialEq)]
pub enum ProfileSource {
    /// CLENV_PROFILE environment variable
    EnvVar,
    /// .clenvrc file (includes the path)
    RcFile(PathBuf),
    /// Global ~/.clenvrc
    GlobalRcFile,
    /// Global default from config.toml
    GlobalConfig,
}

impl ProfileSource {
    /// Human-readable description of the source
    pub fn display(&self) -> String {
        match self {
            ProfileSource::EnvVar => "environment variable CLENV_PROFILE".to_string(),
            ProfileSource::RcFile(path) => format!("{}", path.display()),
            ProfileSource::GlobalRcFile => "~/.clenvrc".to_string(),
            ProfileSource::GlobalConfig => "~/.clenv/config.toml".to_string(),
        }
    }
}

/// Resolved profile information
#[derive(Debug, Clone)]
pub struct ResolvedProfile {
    pub name: String,
    pub source: ProfileSource,
}

/// Profile resolver based on .clenvrc
pub struct RcResolver;

impl RcResolver {
    /// Check the CLENV_PROFILE environment variable
    pub fn env_profile() -> Option<String> {
        std::env::var("CLENV_PROFILE")
            .ok()
            .filter(|s| !s.is_empty())
    }

    /// Walk up from start directory and read profile name from the first .clenvrc found
    pub fn find_rc_profile(start: &Path) -> Option<(String, PathBuf)> {
        let rc_path = dirs::find_clenvrc(start)?;
        let name = read_clenvrc(&rc_path)?;
        Some((name, rc_path))
    }

    /// Read the profile name from the global ~/.clenvrc
    pub fn global_rc_profile() -> Option<String> {
        let path = dirs::global_clenvrc();
        if path.exists() {
            read_clenvrc(&path)
        } else {
            None
        }
    }

    /// Resolve the active profile according to full priority order
    ///
    /// start_dir: directory to start the search from (usually current working directory)
    pub fn resolve(start_dir: &Path) -> Result<Option<ResolvedProfile>> {
        // 1. Environment variable
        if let Some(name) = Self::env_profile() {
            return Ok(Some(ResolvedProfile {
                name,
                source: ProfileSource::EnvVar,
            }));
        }

        // 2. Walk up from current directory
        if let Some((name, rc_path)) = Self::find_rc_profile(start_dir) {
            return Ok(Some(ResolvedProfile {
                name,
                source: ProfileSource::RcFile(rc_path),
            }));
        }

        // 3. Global ~/.clenvrc
        if let Some(name) = Self::global_rc_profile() {
            return Ok(Some(ResolvedProfile {
                name,
                source: ProfileSource::GlobalRcFile,
            }));
        }

        // 4. Global default from config.toml
        let config = crate::config::ClenvConfig::load()?;
        if let Some(name) = config.active_profile {
            return Ok(Some(ResolvedProfile {
                name,
                source: ProfileSource::GlobalConfig,
            }));
        }

        Ok(None)
    }

    /// Write a profile name to .clenvrc in the given directory
    pub fn set_rc(dir: &Path, profile_name: &str) -> Result<()> {
        let rc_path = dir.join(".clenvrc");
        std::fs::write(&rc_path, format!("{}\n", profile_name))
            .with_context(|| format!("Failed to write .clenvrc: {}", rc_path.display()))?;
        Ok(())
    }

    /// Delete .clenvrc from the given directory
    pub fn unset_rc(dir: &Path) -> Result<bool> {
        let rc_path = dir.join(".clenvrc");
        if rc_path.exists() {
            std::fs::remove_file(&rc_path)
                .with_context(|| format!("Failed to delete .clenvrc: {}", rc_path.display()))?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Read a .clenvrc file and return the profile name.
/// Ignores comment lines (#) and blank lines.
fn read_clenvrc(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() && !trimmed.starts_with('#') {
            return Some(trimmed.to_string());
        }
    }
    None
}

use anyhow::Context;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_read_clenvrc() {
        let dir = TempDir::new().unwrap();
        let rc_path = dir.path().join(".clenvrc");

        // comments and blank lines should be ignored
        fs::write(&rc_path, "# 이것은 주석\n\nwork\n").unwrap();
        assert_eq!(read_clenvrc(&rc_path), Some("work".to_string()));
    }

    #[test]
    fn test_set_unset_rc() {
        let dir = TempDir::new().unwrap();

        RcResolver::set_rc(dir.path(), "personal").unwrap();
        assert!(dir.path().join(".clenvrc").exists());

        let content = fs::read_to_string(dir.path().join(".clenvrc")).unwrap();
        assert!(content.contains("personal"));

        let removed = RcResolver::unset_rc(dir.path()).unwrap();
        assert!(removed);
        assert!(!dir.path().join(".clenvrc").exists());
    }

    #[test]
    fn test_env_profile() {
        std::env::set_var("CLENV_PROFILE", "test-profile");
        assert_eq!(RcResolver::env_profile(), Some("test-profile".to_string()));
        std::env::remove_var("CLENV_PROFILE");
        assert_eq!(RcResolver::env_profile(), None);
    }
}
