// src/profile/export.rs
// Profile export/import (v2)
//
// New features:
// - Resolve symlinks to real files for a complete archive
// - Replace MCP server API keys in settings.json with ${ENV_VAR} placeholders
// - Automatically exclude .git, .omc, and sensitive files
// - Support including plugins/ and marketplace/
// - manifest.json v2 metadata

use anyhow::{Context, Result};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tar::{Archive, Builder};

use crate::dirs;
use crate::error::ClenvError;

// ── Option structs ────────────────────────────────────────────────────────────

/// Options for export
pub struct ExportOptions {
    /// Output file path
    pub output_path: PathBuf,
    /// Whether to include plugins/ directory
    pub include_plugins: bool,
    /// Whether to include marketplace/ contents
    pub include_marketplace: bool,
}

/// Options for import
pub struct ImportOptions {
    /// Override profile name (reads from manifest if absent)
    pub name_override: Option<String>,
    /// Whether to overwrite an existing profile
    pub force: bool,
}

// ── Result structs ────────────────────────────────────────────────────────────

/// Summary of export results
pub struct ExportSummary {
    pub output_path: PathBuf,
    pub files_exported: usize,
    pub symlinks_resolved: usize,
    pub redacted_servers: Vec<String>,
    pub warnings: Vec<String>,
}

/// Summary of import results
pub struct ImportSummary {
    pub profile_name: String,
    pub files_imported: usize,
    /// List of MCP servers that need API key reconfiguration
    pub redacted_servers: Vec<String>,
    pub import_notes: Vec<String>,
}

// ── Archive metadata ──────────────────────────────────────────────────────────

/// manifest.json v2 structure
#[derive(Debug, Serialize, Deserialize)]
struct ExportManifest {
    /// Format version ("2")
    version: String,
    /// Original profile name
    profile_name: String,
    /// clenv version
    clenv_version: String,
    /// Export timestamp (RFC3339)
    exported_at: String,
    /// Information about included content
    contents: ContentsMeta,
    /// Names of MCP servers with redacted API keys
    redacted_mcp_servers: Vec<String>,
    /// Guidance messages shown to the user upon import
    import_notes: Vec<String>,
    /// Warning messages for broken symlinks, etc.
    warnings: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ContentsMeta {
    has_skills: bool,
    has_plugins: bool,
    has_marketplace: bool,
    symlinks_resolved: bool,
    sensitive_fields_redacted: bool,
}

/// Legacy (v1) metadata (backward compatibility)
#[derive(Debug, Serialize, Deserialize)]
struct LegacyExportMeta {
    profile_name: String,
    #[serde(default)]
    clenv_version: String,
    #[serde(default)]
    exported_at: String,
    #[serde(default)]
    secret_refs: Vec<String>,
}

// ── Exclusion list ────────────────────────────────────────────────────────────

/// Determine whether a path should be excluded from export
fn is_excluded(path: &Path) -> bool {
    // Exclude by directory component
    let excluded_dirs = [".git", ".omc"];
    if path
        .components()
        .any(|c| excluded_dirs.contains(&c.as_os_str().to_str().unwrap_or("")))
    {
        return true;
    }

    // Exclude by filename
    let excluded_files = [
        "auth.json",
        "user-mcp.json",
        ".claude.json",
        ".snapshot-meta.json",
    ];
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if excluded_files.contains(&name) {
            return true;
        }
    }

    false
}

// ── MCP sensitive data handling ───────────────────────────────────────────────

/// Replace MCP server API keys in settings.json with placeholders
///
/// Substitutes mcpServers.*.env.* values with "${ENV_VAR_NAME}".
/// Also handles mcpServers.*.headers.* (Authorization, etc.) the same way.
///
/// Returns: (processed JSON string, list of server names with redacted credentials)
fn redact_mcp_api_keys(content: &str) -> (String, Vec<String>) {
    let mut json: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => return (content.to_string(), Vec::new()),
    };

    let mut redacted_servers = Vec::new();

    if let Some(mcp_servers) = json.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        for (server_name, server_config) in mcp_servers.iter_mut() {
            let mut server_redacted = false;

            // Process env fields
            if let Some(env) = server_config.get_mut("env").and_then(|v| v.as_object_mut()) {
                for (key, value) in env.iter_mut() {
                    // Skip if already in placeholder format
                    let is_already_placeholder = value
                        .as_str()
                        .map(|s| s.starts_with("${") && s.ends_with('}'))
                        .unwrap_or(false);

                    if !is_already_placeholder && !value.is_null() {
                        *value = serde_json::Value::String(format!("${{{}}}", key));
                        server_redacted = true;
                    }
                }
            }

            // Process headers fields (Authorization, etc.)
            if let Some(headers) = server_config
                .get_mut("headers")
                .and_then(|v| v.as_object_mut())
            {
                for (key, value) in headers.iter_mut() {
                    let is_already_placeholder = value
                        .as_str()
                        .map(|s| s.starts_with("${") && s.ends_with('}'))
                        .unwrap_or(false);

                    if !is_already_placeholder && !value.is_null() {
                        *value = serde_json::Value::String(format!("${{{}}}", key.to_uppercase()));
                        server_redacted = true;
                    }
                }
            }

            // Process direct token field
            if let Some(token) = server_config.get_mut("token") {
                let is_already_placeholder = token
                    .as_str()
                    .map(|s| s.starts_with("${") && s.ends_with('}'))
                    .unwrap_or(false);

                if !is_already_placeholder && !token.is_null() {
                    *token = serde_json::Value::String(format!(
                        "${{{}_TOKEN}}",
                        server_name.to_uppercase().replace('-', "_")
                    ));
                    server_redacted = true;
                }
            }

            if server_redacted {
                redacted_servers.push(server_name.clone());
            }
        }
    }

    let output = serde_json::to_string_pretty(&json).unwrap_or_else(|_| content.to_string());
    (output, redacted_servers)
}

// ── ProfileExporter ───────────────────────────────────────────────────────────

/// Handles profile export and import
pub struct ProfileExporter;

impl ProfileExporter {
    pub fn new() -> Self {
        Self
    }

    /// Export a profile to a .clenvprofile file
    ///
    /// Archive internal structure:
    ///   manifest.json          ← v2 metadata
    ///   contents/              ← profile files (symlinks resolved)
    ///     CLAUDE.md
    ///     settings.json        ← MCP API keys redacted
    ///     keybindings.json
    ///     hooks/
    ///     agents/
    ///     skills/              ← symlinks → real files
    ///     plugins/             ← OMC plugins (optional)
    ///     marketplace/         ← marketplace items (optional)
    pub fn export(&self, name: &str, opts: ExportOptions) -> Result<ExportSummary> {
        let profile_path = dirs::profile_dir(name);

        if !profile_path.exists() {
            return Err(ClenvError::ProfileNotFound(name.to_string()).into());
        }

        let output_file = std::fs::File::create(&opts.output_path).with_context(|| {
            format!(
                "Failed to create output file: {}",
                opts.output_path.display()
            )
        })?;

        let encoder = GzEncoder::new(output_file, Compression::best());
        let mut tar = Builder::new(encoder);

        let mut files_exported: usize = 0;
        let mut symlinks_resolved: usize = 0;
        let mut all_redacted_servers: Vec<String> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();

        // follow_links(true): follow symlinks to read actual content
        for entry in walkdir::WalkDir::new(&profile_path).follow_links(true) {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    // Failed to access entry (broken symlink, etc.) — warn and continue
                    let warn = format!("Failed to access entry (skipping): {}", e);
                    log::warn!("{}", warn);
                    warnings.push(warn);
                    continue;
                }
            };

            let path = entry.path();

            // Apply exclusion list filter
            if is_excluded(path) {
                continue;
            }

            let relative = match path.strip_prefix(&profile_path) {
                Ok(r) => r,
                Err(_) => continue,
            };

            // Skip the root directory itself
            if relative.as_os_str().is_empty() {
                continue;
            }

            // Filter plugins/ and marketplace/ based on options
            if let Some(first) = relative.components().next() {
                let first_str = first.as_os_str().to_str().unwrap_or("");
                if first_str == "plugins" && !opts.include_plugins {
                    continue;
                }
                if first_str == "marketplace" && !opts.include_marketplace {
                    continue;
                }
            }

            let archive_path = PathBuf::from("contents").join(relative);

            // Check if the original path was a symlink (for resolved count)
            let original_path = profile_path.join(relative);
            let was_symlink = original_path.is_symlink();

            if entry.file_type().is_dir() {
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Directory);
                header.set_mode(0o755);
                header.set_size(0);
                header.set_cksum();
                tar.append_data(&mut header, &archive_path, std::io::empty())?;
            } else if entry.file_type().is_file() {
                // Insert settings.json after redacting sensitive data
                let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if file_name == "settings.json" {
                    let content = std::fs::read_to_string(path).with_context(|| {
                        format!("Failed to read settings.json: {}", path.display())
                    })?;

                    let (redacted_content, redacted) = redact_mcp_api_keys(&content);

                    for server in redacted {
                        if !all_redacted_servers.contains(&server) {
                            all_redacted_servers.push(server);
                        }
                    }

                    let bytes = redacted_content.as_bytes();
                    let mut header = tar::Header::new_gnu();
                    header.set_size(bytes.len() as u64);
                    header.set_mode(0o644);
                    header.set_cksum();
                    tar.append_data(&mut header, &archive_path, bytes)?;
                } else {
                    tar.append_path_with_name(path, &archive_path)?;
                }

                files_exported += 1;
                if was_symlink {
                    symlinks_resolved += 1;
                }
            }
        }

        // Check existence of skills/plugins/marketplace
        let has_skills = profile_path.join("skills").exists();
        let has_plugins = opts.include_plugins && profile_path.join("plugins").exists();
        let has_marketplace = opts.include_marketplace && profile_path.join("marketplace").exists();

        // Build import guidance messages
        let mut import_notes = Vec::new();
        if !all_redacted_servers.is_empty() {
            import_notes.push(format!(
                "MCP server API keys were redacted from settings.json. Re-enter them after importing: {}",
                all_redacted_servers.join(", ")
            ));
        }

        // Create and insert manifest.json
        let manifest = ExportManifest {
            version: "2".to_string(),
            profile_name: name.to_string(),
            clenv_version: env!("CARGO_PKG_VERSION").to_string(),
            exported_at: chrono::Utc::now().to_rfc3339(),
            contents: ContentsMeta {
                has_skills,
                has_plugins,
                has_marketplace,
                symlinks_resolved: symlinks_resolved > 0,
                sensitive_fields_redacted: !all_redacted_servers.is_empty(),
            },
            redacted_mcp_servers: all_redacted_servers.clone(),
            import_notes: import_notes.clone(),
            warnings: warnings.clone(),
        };

        let manifest_json = serde_json::to_string_pretty(&manifest)?;
        let manifest_bytes = manifest_json.as_bytes();
        let mut header = tar::Header::new_gnu();
        header.set_size(manifest_bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "manifest.json", manifest_bytes)?;

        tar.finish()?;

        Ok(ExportSummary {
            output_path: opts.output_path,
            files_exported,
            symlinks_resolved,
            redacted_servers: all_redacted_servers,
            warnings,
        })
    }

    /// Import a profile from a .clenvprofile file
    pub fn import(&self, file: &PathBuf, opts: ImportOptions) -> Result<ImportSummary> {
        if !file.exists() {
            anyhow::bail!("File not found: {}", file.display());
        }

        let input_file = std::fs::File::open(file)
            .with_context(|| format!("Failed to open file: {}", file.display()))?;
        let decoder = GzDecoder::new(input_file);
        let mut archive = Archive::new(decoder);

        let temp_dir = tempfile::TempDir::new()?;
        archive.unpack(temp_dir.path())?;

        // Read manifest.json (v2) or meta.json (v1)
        let (profile_name_from_meta, redacted_servers, import_notes) =
            read_manifest(temp_dir.path(), file)?;

        // Determine profile name
        let profile_name = opts.name_override.unwrap_or(profile_name_from_meta);

        let profile_path = dirs::profile_dir(&profile_name);

        // Handle existing profile
        if profile_path.exists() {
            if !opts.force {
                return Err(ClenvError::ProfileAlreadyExists(profile_name).into());
            }
            std::fs::remove_dir_all(&profile_path)?;
        }

        // Copy from contents/ (v2), files/ (v1), or root
        let contents_dir = temp_dir.path().join("contents");
        let files_dir = temp_dir.path().join("files");

        let source_dir = if contents_dir.exists() {
            contents_dir
        } else if files_dir.exists() {
            files_dir
        } else {
            temp_dir.path().to_path_buf()
        };

        let files_imported = copy_dir(&source_dir, &profile_path)?;

        // Initialize git and create initial commit
        let mut vcs = crate::profile::vcs::ProfileVcs::new(profile_path)?;
        vcs.init()?;
        vcs.commit(
            &format!(
                "Imported from '{}'",
                file.file_name().unwrap_or_default().to_string_lossy()
            ),
            &[],
        )?;

        Ok(ImportSummary {
            profile_name,
            files_imported,
            redacted_servers,
            import_notes,
        })
    }
}

// ── Helper functions ──────────────────────────────────────────────────────────

/// Read manifest.json (v2) or meta.json (v1) and return profile name, redacted servers, and import notes
fn read_manifest(
    temp_path: &Path,
    source_file: &Path,
) -> Result<(String, Vec<String>, Vec<String>)> {
    // v2: manifest.json
    let manifest_path = temp_path.join("manifest.json");
    if manifest_path.exists() {
        let content = std::fs::read_to_string(&manifest_path)?;
        if let Ok(manifest) = serde_json::from_str::<ExportManifest>(&content) {
            return Ok((
                manifest.profile_name,
                manifest.redacted_mcp_servers,
                manifest.import_notes,
            ));
        }
    }

    // v1: meta.json (legacy compatibility)
    let meta_path = temp_path.join("meta.json");
    if meta_path.exists() {
        let content = std::fs::read_to_string(&meta_path)?;
        if let Ok(meta) = serde_json::from_str::<LegacyExportMeta>(&content) {
            let notes = if !meta.secret_refs.is_empty() {
                vec![format!(
                    "This profile references the following secrets: {}",
                    meta.secret_refs.join(", ")
                )]
            } else {
                Vec::new()
            };
            return Ok((meta.profile_name, Vec::new(), notes));
        }
    }

    // No metadata: extract profile name from filename
    let profile_name = source_file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("imported")
        .to_string();

    Ok((profile_name, Vec::new(), Vec::new()))
}

/// Recursively copy a directory and return the number of files copied
fn copy_dir(source: &Path, dest: &PathBuf) -> Result<usize> {
    std::fs::create_dir_all(dest)?;
    let mut count = 0;

    for entry in walkdir::WalkDir::new(source) {
        let entry = entry?;
        let path = entry.path();
        let relative = path.strip_prefix(source)?;
        let dest_path = dest.join(relative);

        if relative.as_os_str().is_empty() {
            continue;
        }

        if path.is_dir() {
            std::fs::create_dir_all(&dest_path)?;
        } else if path.is_file() {
            if let Some(parent) = dest_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(path, &dest_path)?;
            count += 1;
        }
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_mcp_api_keys_basic() {
        let settings = r#"{
            "mcpServers": {
                "github": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-github"],
                    "env": {
                        "GITHUB_TOKEN": "ghp_realtoken123"
                    }
                }
            }
        }"#;

        let (result, redacted) = redact_mcp_api_keys(settings);
        assert!(redacted.contains(&"github".to_string()));

        let json: serde_json::Value = serde_json::from_str(&result).unwrap();
        let token = &json["mcpServers"]["github"]["env"]["GITHUB_TOKEN"];
        assert_eq!(token.as_str().unwrap(), "${GITHUB_TOKEN}");
    }

    #[test]
    fn test_redact_mcp_already_placeholder() {
        let settings = r#"{
            "mcpServers": {
                "slack": {
                    "env": {
                        "SLACK_TOKEN": "${SLACK_TOKEN}"
                    }
                }
            }
        }"#;

        let (_, redacted) = redact_mcp_api_keys(settings);
        // Already a placeholder, should not be included in redacted
        assert!(!redacted.contains(&"slack".to_string()));
    }

    #[test]
    fn test_is_excluded() {
        assert!(is_excluded(Path::new("/home/user/profile/.git/config")));
        assert!(is_excluded(Path::new("/home/user/profile/.omc/state.json")));
        assert!(is_excluded(Path::new("/home/user/profile/auth.json")));
        assert!(is_excluded(Path::new("/home/user/profile/user-mcp.json")));
        assert!(!is_excluded(Path::new("/home/user/profile/CLAUDE.md")));
        assert!(!is_excluded(Path::new("/home/user/profile/settings.json")));
    }
}
