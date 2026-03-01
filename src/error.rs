// src/error.rs
// Custom error type definitions for claude-env

use thiserror::Error;

/// All error types for clenv, unified in one enum
#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum ClenvError {
    // ── Profile errors ────────────────────────────────────────────────────────
    /// Profile with the given name does not exist
    #[error("Profile not found: '{0}'")]
    ProfileNotFound(String),

    /// A profile with the same name already exists
    #[error("Profile already exists: '{0}'")]
    ProfileAlreadyExists(String),

    /// Cannot delete the currently active profile
    #[error("Cannot delete active profile '{0}'. Switch to another profile first.")]
    CannotDeleteActiveProfile(String),

    /// No profiles have been created yet
    #[error("No profiles exist. Create one with 'clenv profile create <name>'.")]
    NoProfilesExist,

    // ── Version control errors ─────────────────────────────────────────────────
    /// No changes to commit
    #[error("Nothing to commit")]
    NothingToCommit,

    /// The specified version/tag/hash does not exist
    #[error("Version not found: '{0}'")]
    VersionNotFound(String),

    /// Tag already exists
    #[error("Tag already exists: '{0}'")]
    TagAlreadyExists(String),

    // ── Export/Import errors ──────────────────────────────────────────────────
    /// Profile export failed
    #[error("Export failed: {0}")]
    ExportError(String),

    /// Profile import failed
    #[error("Import failed: {0}")]
    ImportError(String),

    /// Unsupported archive format
    #[error("Unsupported archive format: {0}")]
    UnsupportedArchiveFormat(String),

    // ── .clenvrc errors ───────────────────────────────────────────────────────
    /// Profile specified in .clenvrc does not exist
    #[error("Profile '{0}' specified in .clenvrc does not exist")]
    RcProfileNotFound(String),

    // ── File system errors ────────────────────────────────────────────────────
    /// Failed to initialize ~/.clenv directory
    #[error("Failed to initialize clenv directory: {0}")]
    InitializationError(String),

    /// ~/.claude directory not found
    #[error("~/.claude directory not found. Is Claude Code installed?")]
    ClaudeDirectoryNotFound,

    /// Failed to create/change symlink
    #[error("Failed to create symlink: {0}")]
    SymlinkError(String),

    // ── Configuration errors ──────────────────────────────────────────────────
    /// Failed to parse config file
    #[error("Failed to parse config file ({path}): {message}")]
    ConfigParseError { path: String, message: String },
}

/// Map error variant to exit code
impl ClenvError {
    pub fn exit_code(&self) -> i32 {
        match self {
            ClenvError::ExportError(_) | ClenvError::ImportError(_) => 2,
            _ => 1,
        }
    }
}
