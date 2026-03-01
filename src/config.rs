// src/config.rs
// Global configuration management for clenv
//
// Reads and writes ~/.clenv/config.toml.
// Manages clenv-internal settings such as active profile and color output.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::dirs;

/// Global configuration struct for clenv.
/// Saved and loaded from ~/.clenv/config.toml.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ClenvConfig {
    /// Currently active profile name (None if not set)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_profile: Option<String>,

    /// Whether to use colored output (default: true)
    #[serde(default = "default_true")]
    pub color: bool,

    /// Telemetry (anonymous usage stats) consent (default: false)
    #[serde(default)]
    pub telemetry: bool,
}

fn default_true() -> bool {
    true
}

impl ClenvConfig {
    /// Load from config file.
    /// Returns default value if file does not exist (first run).
    pub fn load() -> Result<Self> {
        let path = dirs::config_file();

        if !path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))
    }

    /// Save to config file
    pub fn save(&self) -> Result<()> {
        let path = dirs::config_file();

        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;

        std::fs::write(&path, content)
            .with_context(|| format!("Failed to save config file: {}", path.display()))?;

        Ok(())
    }

    /// Set the active profile
    pub fn set_active_profile(&mut self, name: &str) -> Result<()> {
        self.active_profile = Some(name.to_string());
        self.save()
    }

    /// Get the active profile
    #[allow(dead_code)]
    pub fn get_active_profile(&self) -> Option<&str> {
        self.active_profile.as_deref()
    }

    /// Clear the active profile
    #[allow(dead_code)]
    pub fn clear_active_profile(&mut self) -> Result<()> {
        self.active_profile = None;
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use tempfile::TempDir;

    // ── Pure serialization/deserialization tests (no env var needed) ──────────

    #[test]
    fn test_default_values() {
        let config = ClenvConfig::default();
        assert!(config.active_profile.is_none());
        // Rust Default::default() uses false for bool;
        // default_true() only applies during serde deserialization
        assert!(!config.color, "Rust Default: color = false");
        assert!(!config.telemetry, "default: telemetry = false");
    }

    #[test]
    fn test_serde_round_trip() {
        let mut config = ClenvConfig::default();
        config.active_profile = Some("work".to_string());
        config.color = false;
        config.telemetry = true;

        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: ClenvConfig = toml::from_str(&serialized).unwrap();

        assert_eq!(deserialized.active_profile, Some("work".to_string()));
        assert!(!deserialized.color);
        assert!(deserialized.telemetry);
    }

    #[test]
    fn test_active_profile_none_not_serialized() {
        let config = ClenvConfig::default();
        let serialized = toml::to_string_pretty(&config).unwrap();
        // verify skip_serializing_if = "Option::is_none" is applied
        assert!(!serialized.contains("active_profile"));
    }

    #[test]
    fn test_active_profile_some_is_serialized() {
        let mut config = ClenvConfig::default();
        config.active_profile = Some("myprofile".to_string());
        let serialized = toml::to_string_pretty(&config).unwrap();
        assert!(serialized.contains("active_profile"));
        assert!(serialized.contains("myprofile"));
    }

    #[test]
    fn test_get_active_profile() {
        let mut config = ClenvConfig::default();
        assert!(config.get_active_profile().is_none());

        config.active_profile = Some("test".to_string());
        assert_eq!(config.get_active_profile(), Some("test"));
    }

    #[test]
    fn test_default_color_preserved_when_missing_from_toml() {
        // TOML without color key → default_true() should apply
        let toml_str = r#"active_profile = "work""#;
        let config: ClenvConfig = toml::from_str(toml_str).unwrap();
        assert!(config.color, "missing color key defaults to true");
    }

    // ── File I/O tests (need env var, run serially) ────────────────────────

    #[test]
    #[serial]
    fn test_load_missing_file_returns_default() {
        let temp = TempDir::new().unwrap();
        std::env::set_var("CLENV_HOME", temp.path().to_str().unwrap());

        let result = ClenvConfig::load();
        std::env::remove_var("CLENV_HOME");

        let config = result.unwrap();
        assert!(config.active_profile.is_none());
        // No file → Default::default() → color=false (serde default_true only during deserialization)
        assert!(!config.color);
        assert!(!config.telemetry);
    }

    #[test]
    #[serial]
    fn test_save_and_load_round_trip() {
        let temp = TempDir::new().unwrap();
        std::env::set_var("CLENV_HOME", temp.path().to_str().unwrap());

        let mut config = ClenvConfig::default();
        config.active_profile = Some("myprofile".to_string());
        config.color = false;
        config.save().unwrap();

        let loaded = ClenvConfig::load().unwrap();
        std::env::remove_var("CLENV_HOME");

        assert_eq!(loaded.active_profile, Some("myprofile".to_string()));
        assert!(!loaded.color);
    }

    #[test]
    #[serial]
    fn test_set_active_profile_persists() {
        let temp = TempDir::new().unwrap();
        std::env::set_var("CLENV_HOME", temp.path().to_str().unwrap());

        let mut config = ClenvConfig::default();
        config.set_active_profile("newprofile").unwrap();

        let loaded = ClenvConfig::load().unwrap();
        std::env::remove_var("CLENV_HOME");

        assert_eq!(loaded.active_profile, Some("newprofile".to_string()));
    }

    #[test]
    #[serial]
    fn test_clear_active_profile_persists() {
        let temp = TempDir::new().unwrap();
        std::env::set_var("CLENV_HOME", temp.path().to_str().unwrap());

        let mut config = ClenvConfig::default();
        config.set_active_profile("some-profile").unwrap();
        config.clear_active_profile().unwrap();

        let loaded = ClenvConfig::load().unwrap();
        std::env::remove_var("CLENV_HOME");

        assert!(loaded.active_profile.is_none());
    }
}
