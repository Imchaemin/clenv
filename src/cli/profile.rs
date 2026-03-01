// src/cli/profile.rs
// CLI command handlers for profile-related subcommands

use anyhow::Result;
use clap::{Args, Subcommand};
use colored::Colorize;

use crate::cli::{print_error, print_info, print_step, print_success, print_warning};
use crate::profile::manager::ProfileManager;
use crate::profile::vcs::ProfileVcs;
use crate::rc::RcResolver;

// ── profile subcommands ───────────────────────────────────────────────────────

#[derive(Args)]
pub struct ProfileArgs {
    #[command(subcommand)]
    pub command: ProfileCommands,
}

#[derive(Subcommand)]
pub enum ProfileCommands {
    /// Create a new profile
    Create {
        name: String,
        /// Switch to this profile immediately after creation
        #[arg(long = "use", short = 'u')]
        use_now: bool,
        /// Copy from an existing profile
        #[arg(long)]
        from: Option<String>,
    },

    /// List all profiles
    List,

    /// Switch to a profile (changes ~/.claude/ symlink)
    Use { name: String },

    /// Delete a profile
    Delete {
        name: String,
        #[arg(long, short)]
        force: bool,
    },

    /// Copy a profile
    Clone { source: String, destination: String },

    /// Rename a profile
    Rename { old_name: String, new_name: String },

    /// Export a profile to a file (.clenvprofile)
    ///
    /// Examples:
    ///   clenv profile export work
    ///   clenv profile export work -o my-config.clenvprofile
    Export {
        /// Profile name to export (default: currently active profile)
        name: Option<String>,
        /// Output file path
        #[arg(long, short)]
        output: Option<String>,
        /// Exclude plugins/ directory
        #[arg(long)]
        no_plugins: bool,
        /// Exclude marketplace/ items
        #[arg(long)]
        no_marketplace: bool,
    },

    /// Import a profile from a file
    ///
    /// Examples:
    ///   clenv profile import my-config.clenvprofile
    ///   clenv profile import my-config.clenvprofile -n my-profile --use
    Import {
        file: String,
        /// Override profile name (default: read from file)
        #[arg(long, short)]
        name: Option<String>,
        /// Overwrite if a profile with the same name already exists
        #[arg(long)]
        force: bool,
        /// Switch to this profile immediately after import
        #[arg(long, short = 'u')]
        use_now: bool,
    },

    /// Print the currently active profile name (for scripting)
    Current,

    /// Deactivate clenv management - restore ~/.claude symlink to a real directory
    Deactivate {
        #[arg(long)]
        purge: bool,
        /// Run without confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },
}

/// Run the `clenv profile` command
pub async fn run(args: ProfileArgs) -> Result<()> {
    let manager = ProfileManager::new()?;

    match args.command {
        ProfileCommands::Create {
            name,
            use_now,
            from,
        } => cmd_profile_create(&manager, &name, use_now, from.as_deref()).await,
        ProfileCommands::List => cmd_profile_list(&manager).await,
        ProfileCommands::Use { name } => cmd_profile_use(&manager, &name).await,
        ProfileCommands::Delete { name, force } => cmd_profile_delete(&manager, &name, force).await,
        ProfileCommands::Clone {
            source,
            destination,
        } => cmd_profile_clone(&manager, &source, &destination).await,
        ProfileCommands::Rename { old_name, new_name } => {
            cmd_profile_rename(&manager, &old_name, &new_name).await
        }
        ProfileCommands::Export {
            name,
            output,
            no_plugins,
            no_marketplace,
        } => {
            cmd_profile_export(
                &manager,
                name.as_deref(),
                output.as_deref(),
                !no_plugins,
                !no_marketplace,
            )
            .await
        }
        ProfileCommands::Import {
            file,
            name,
            force,
            use_now,
        } => cmd_profile_import(&manager, &file, name.as_deref(), force, use_now).await,
        ProfileCommands::Current => {
            if let Some(name) = crate::dirs::active_profile_name() {
                println!("{}", name);
            }
            Ok(())
        }
        ProfileCommands::Deactivate { purge, yes } => {
            cmd_profile_deactivate(&manager, purge, yes).await
        }
    }
}

async fn cmd_profile_create(
    manager: &ProfileManager,
    name: &str,
    use_now: bool,
    from: Option<&str>,
) -> Result<()> {
    print_step(&format!("Creating profile '{}' ...", name));
    manager.create(name, from)?;
    print_success(&format!("Profile '{}' created", name.bold()));

    if use_now {
        print_step(&format!("Switching to profile '{}' ...", name));
        manager.use_profile(name)?;
        print_success(&format!("Active profile: {}", name.green().bold()));
    } else {
        print_info(&format!(
            "To switch to this profile: {}",
            format!("clenv profile use {}", name).bold()
        ));
    }
    Ok(())
}

async fn cmd_profile_list(manager: &ProfileManager) -> Result<()> {
    let profiles = manager.list()?;
    let active = manager.active_profile_name();

    if profiles.is_empty() {
        print_info("No profiles found.");
        println!();
        print_info(&format!(
            "Create your first profile: {}",
            "clenv profile create <name>".bold()
        ));
        return Ok(());
    }

    println!("{}", "Profiles:".bold());
    println!();

    for profile in &profiles {
        let is_active = active.as_deref() == Some(profile.name.as_str());
        let indicator = if is_active {
            "* ".green().bold()
        } else {
            "  ".normal()
        };

        let name_display = if is_active {
            profile.name.green().bold()
        } else {
            profile.name.normal().bold()
        };

        let commit_info = if let Some(last) = &profile.last_commit {
            format!("  {} {}", last.hash[..7].dimmed(), last.message.dimmed())
        } else {
            format!("  {}", "(no commits)".dimmed())
        };

        println!("{}{}{}", indicator, name_display, commit_info);
    }

    println!();
    println!("{} profile(s)", profiles.len().to_string().bold());

    Ok(())
}

async fn cmd_profile_use(manager: &ProfileManager, name: &str) -> Result<()> {
    let current = manager.active_profile_name();

    if current.as_deref() == Some(name) {
        print_info(&format!("Already using profile '{}'", name));
        return Ok(());
    }

    print_step(&format!("Switching to profile '{}' ...", name));
    manager.use_profile(name)?;
    print_success(&format!("Active profile: {}", name.green().bold()));

    Ok(())
}

async fn cmd_profile_delete(manager: &ProfileManager, name: &str, force: bool) -> Result<()> {
    if !force {
        let confirm = dialoguer::Confirm::new()
            .with_prompt(format!("Delete profile '{}'? This cannot be undone.", name))
            .default(false)
            .interact()?;

        if !confirm {
            print_info("Delete cancelled");
            return Ok(());
        }
    }

    print_step(&format!("Deleting profile '{}' ...", name));
    manager.delete(name)?;
    print_success(&format!("Profile '{}' deleted", name));

    Ok(())
}

async fn cmd_profile_clone(
    manager: &ProfileManager,
    source: &str,
    destination: &str,
) -> Result<()> {
    print_step(&format!("Cloning '{}' → '{}' ...", source, destination));
    manager.clone_profile(source, destination)?;
    print_success(&format!(
        "Profile '{}' cloned to '{}'",
        source.bold(),
        destination.bold()
    ));
    Ok(())
}

async fn cmd_profile_rename(
    manager: &ProfileManager,
    old_name: &str,
    new_name: &str,
) -> Result<()> {
    print_step(&format!("Renaming '{}' → '{}' ...", old_name, new_name));
    manager.rename(old_name, new_name)?;
    print_success(&format!(
        "Profile renamed: '{}' → '{}'",
        old_name,
        new_name.bold()
    ));
    Ok(())
}

async fn cmd_profile_export(
    manager: &ProfileManager,
    name: Option<&str>,
    output: Option<&str>,
    include_plugins: bool,
    include_marketplace: bool,
) -> Result<()> {
    use crate::profile::export::{ExportOptions, ProfileExporter};

    let profile_name = name
        .map(|n| n.to_string())
        .or_else(|| manager.active_profile_name())
        .ok_or_else(|| anyhow::anyhow!("No active profile. Specify a profile name."))?;

    let output_path = output.map(std::path::PathBuf::from).unwrap_or_else(|| {
        std::path::PathBuf::from(format!(
            "{}-{}.clenvprofile",
            profile_name,
            chrono::Utc::now().format("%Y%m%d")
        ))
    });

    print_step(&format!("Exporting profile '{}' ...", profile_name));

    let exporter = ProfileExporter::new();
    let summary = exporter.export(
        &profile_name,
        ExportOptions {
            output_path: output_path.clone(),
            include_plugins,
            include_marketplace,
        },
    )?;

    print_success(&format!(
        "Exported: {}",
        summary.output_path.display().to_string().bold()
    ));
    print_info(&format!(
        "{} file(s) exported, {} symlink(s) resolved",
        summary.files_exported, summary.symlinks_resolved
    ));

    if !summary.redacted_servers.is_empty() {
        println!();
        print_warning(
            "The following MCP servers had API keys redacted. Re-enter them after importing:",
        );
        for server in &summary.redacted_servers {
            println!("  - {}", server.yellow());
        }
    }

    if !summary.warnings.is_empty() {
        println!();
        for w in &summary.warnings {
            print_warning(w);
        }
    }

    Ok(())
}

async fn cmd_profile_import(
    manager: &ProfileManager,
    file: &str,
    name: Option<&str>,
    force: bool,
    use_now: bool,
) -> Result<()> {
    use crate::profile::export::{ImportOptions, ProfileExporter};

    let path = std::path::PathBuf::from(file);
    print_step(&format!("Importing from '{}' ...", file));

    let exporter = ProfileExporter::new();
    let summary = exporter.import(
        &path,
        ImportOptions {
            name_override: name.map(|s| s.to_string()),
            force,
        },
    )?;

    print_success(&format!(
        "Profile '{}' imported",
        summary.profile_name.bold()
    ));
    print_info(&format!("{} file(s) imported", summary.files_imported));

    if !summary.redacted_servers.is_empty() {
        println!();
        print_warning("Re-enter API keys for the following MCP servers:");
        for server in &summary.redacted_servers {
            println!("  - {}", server.yellow());
        }
    }

    for note in &summary.import_notes {
        print_info(note);
    }

    if use_now {
        print_step(&format!(
            "Switching to profile '{}' ...",
            summary.profile_name
        ));
        manager.use_profile(&summary.profile_name)?;
        print_success(&format!(
            "Active profile: {}",
            summary.profile_name.green().bold()
        ));
    } else {
        print_info(&format!(
            "To switch to this profile: {}",
            format!("clenv profile use {}", summary.profile_name).bold()
        ));
    }
    Ok(())
}

// ── uninstall command ─────────────────────────────────────────────────────────

#[derive(Args)]
pub struct UninstallArgs {
    /// Skip confirmation prompt
    #[arg(long, short)]
    pub yes: bool,

    /// Keep ~/.clenv/ data and only restore the symlink (default: delete everything)
    #[arg(long)]
    pub keep_data: bool,
}

pub async fn run_uninstall(args: UninstallArgs) -> Result<()> {
    let manager = ProfileManager::new()?;

    // Check whether clenv is managing ~/.claude
    if !crate::dirs::is_managed_by_clenv() {
        print_warning("~/.claude is not managed by clenv");
        if crate::dirs::clenv_home().exists() && !args.keep_data {
            let purge = if args.yes {
                true
            } else {
                dialoguer::Confirm::new()
                    .with_prompt("Delete ~/.clenv/ data?")
                    .default(false)
                    .interact()?
            };
            if purge {
                std::fs::remove_dir_all(crate::dirs::clenv_home())
                    .map_err(|e| anyhow::anyhow!("Failed to delete ~/.clenv: {}", e))?;
                print_success("~/.clenv/ deleted");
            }
        }
        return Ok(());
    }

    let active = manager
        .active_profile_name()
        .unwrap_or_else(|| "(unknown)".to_string());

    if !args.yes {
        println!();
        println!("{}", "clenv uninstall plan:".bold());
        println!(
            "  {} Restore ~/.claude/ from original backup (current profile: '{}')",
            "1.".dimmed(),
            active.green()
        );
        println!("  {} Keep MCP settings from current profile", "2.".dimmed());
        if args.keep_data {
            println!("  {} Preserve ~/.clenv/ data (--keep-data)", "3.".dimmed());
        } else {
            println!(
                "  {} Delete ~/.clenv/ entirely (all profiles and history)",
                "3.".dimmed().red()
            );
        }
        println!();

        let confirm = dialoguer::Confirm::new()
            .with_prompt("Continue? This cannot be undone")
            .default(false)
            .interact()?;

        if !confirm {
            print_info("Cancelled");
            return Ok(());
        }
    }

    // 1. Restore original ~/.claude from backup
    print_step("Restoring original ~/.claude/ from backup...");
    manager.restore_original()?;
    print_success("~/.claude/ restored from original backup");

    // 2. Delete ~/.clenv/ unless --keep-data
    if !args.keep_data {
        print_step("Removing ~/.clenv/ ...");
        let clenv_home = crate::dirs::clenv_home();
        if clenv_home.exists() {
            std::fs::remove_dir_all(&clenv_home)
                .map_err(|e| anyhow::anyhow!("Failed to delete ~/.clenv: {}", e))?;
        }
        print_success("~/.clenv/ removed");
    }

    // 3. Clean up shell rc files (remove any legacy shell-init lines)
    let home = crate::dirs::home_dir();
    let rc_files = [
        home.join(".zshrc"),
        home.join(".bashrc"),
        home.join(".bash_profile"),
    ];
    let mut cleaned = Vec::new();
    for rc_path in &rc_files {
        if rc_path.exists() {
            if let Ok(content) = std::fs::read_to_string(rc_path) {
                let new_content: Vec<&str> = content
                    .lines()
                    .filter(|line| !line.contains("clenv shell-init"))
                    .collect();
                let new_str = new_content.join("\n");
                if new_str != content.trim_end_matches('\n') {
                    let _ = std::fs::write(rc_path, new_str + "\n");
                    cleaned.push(rc_path.display().to_string());
                }
            }
        }
    }
    if cleaned.is_empty() {
        print_info("No shell integration to remove");
    } else {
        for path in &cleaned {
            print_success(&format!("Shell integration removed: {}", path));
        }
        print_warning("Open a new terminal or run 'source ~/.zshrc' to apply changes");
    }

    // 4. Remove the clenv binary
    print_step("Removing clenv binary...");
    match std::env::current_exe() {
        Ok(bin_path) => {
            if let Err(e) = std::fs::remove_file(&bin_path) {
                print_warning(&format!("Failed to auto-remove binary: {}", e));
                print_info(&format!("Remove it manually: rm '{}'", bin_path.display()));
            } else {
                print_success(&format!("Binary removed: {}", bin_path.display()));
            }
        }
        Err(e) => {
            print_warning(&format!("Could not find binary path: {}", e));
            print_info("Remove the clenv binary manually (e.g. which clenv && rm <path>)");
        }
    }

    println!();
    print_success("clenv has been completely removed");
    print_info("Your original ~/.claude/ settings have been restored from backup.");
    if args.keep_data {
        print_info("Profile data is preserved in ~/.clenv/");
    }

    Ok(())
}

async fn cmd_profile_deactivate(manager: &ProfileManager, purge: bool, yes: bool) -> Result<()> {
    let active = manager
        .active_profile_name()
        .unwrap_or_else(|| "(unknown)".to_string());

    if !yes {
        let prompt = if purge {
            format!(
                "Restore active profile '{}' to ~/.claude/ and delete ~/.clenv/ entirely. Continue?",
                active
            )
        } else {
            format!(
                "Restore active profile '{}' to a real ~/.claude/ directory. Continue?",
                active
            )
        };

        let confirm = dialoguer::Confirm::new()
            .with_prompt(prompt)
            .default(false)
            .interact()?;

        if !confirm {
            print_info("Cancelled");
            return Ok(());
        }
    }

    print_step("Restoring ~/.claude symlink to a real directory...");
    manager.deactivate(purge)?;

    print_success("~/.claude restored — clenv management deactivated");
    if purge {
        print_success("~/.clenv/ deleted");
    } else {
        print_info("Profile data remains in ~/.clenv/");
        print_info(&format!(
            "To remove data too: {}",
            "clenv profile deactivate --purge".bold()
        ));
    }
    Ok(())
}

// ── Version control commands ──────────────────────────────────────────────────

#[derive(Args)]
pub struct StatusArgs {
    #[arg(long, short)]
    pub short: bool,
}

#[derive(Args)]
pub struct DiffArgs {
    pub range: Option<String>,
    #[arg(long)]
    pub name_only: bool,
}

#[derive(Args)]
pub struct CommitArgs {
    #[arg(long, short)]
    pub message: String,
    pub files: Vec<String>,
}

#[derive(Args)]
pub struct LogArgs {
    #[arg(long, short = 'n', default_value = "20")]
    pub limit: usize,
    #[arg(long)]
    pub oneline: bool,
    pub file: Option<String>,
}

#[derive(Args)]
pub struct CheckoutArgs {
    pub reference: String,
}

#[derive(Args)]
pub struct RevertArgs {
    pub commit: Option<String>,
}

#[derive(Args)]
pub struct TagArgs {
    pub name: Option<String>,
    #[arg(long, short)]
    pub message: Option<String>,
    #[arg(long, short)]
    pub delete: bool,
    #[arg(long, short)]
    pub list: bool,
}

#[derive(Args)]
pub struct DoctorArgs {
    #[arg(long)]
    pub fix: bool,
}

pub async fn run_status(args: StatusArgs) -> Result<()> {
    let manager = ProfileManager::new()?;
    let profile_name = manager
        .active_profile_name()
        .ok_or_else(|| anyhow::anyhow!("No active profile"))?;

    let vcs = ProfileVcs::new(manager.profile_path(&profile_name))?;
    let changes = vcs.status()?;

    if changes.is_empty() {
        print_info("Nothing to commit (working tree clean)");
        return Ok(());
    }

    println!(
        "{}",
        format!("Changes in profile '{}':", profile_name).bold()
    );
    println!();

    for change in &changes {
        let (symbol, color_fn): (&str, fn(&str) -> colored::ColoredString) =
            match change.status.as_str() {
                "added" => ("A ", |s| s.green()),
                "modified" => ("M ", |s| s.yellow()),
                "deleted" => ("D ", |s| s.red()),
                _ => ("? ", |s| s.normal()),
            };

        if args.short {
            println!("{} {}", symbol.to_string().bold(), change.path);
        } else {
            println!("  {} {}", color_fn(symbol).bold(), change.path);
        }
    }

    println!();
    println!(
        "{} file(s) changed. To save: {}",
        changes.len().to_string().bold(),
        "clenv commit -m \"message\"".bold()
    );

    Ok(())
}

pub async fn run_diff(args: DiffArgs) -> Result<()> {
    let manager = ProfileManager::new()?;
    let profile_name = manager
        .active_profile_name()
        .ok_or_else(|| anyhow::anyhow!("No active profile"))?;

    let vcs = ProfileVcs::new(manager.profile_path(&profile_name))?;
    let diff = vcs.diff(args.range.as_deref(), args.name_only)?;

    if diff.is_empty() {
        print_info("No changes");
        return Ok(());
    }

    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            println!("{}", line.bold());
        } else if line.starts_with('+') {
            println!("{}", line.green());
        } else if line.starts_with('-') {
            println!("{}", line.red());
        } else if line.starts_with("@@") {
            println!("{}", line.cyan());
        } else {
            println!("{}", line);
        }
    }

    Ok(())
}

pub async fn run_commit(args: CommitArgs) -> Result<()> {
    let manager = ProfileManager::new()?;
    let profile_name = manager
        .active_profile_name()
        .ok_or_else(|| anyhow::anyhow!("No active profile"))?;

    let vcs = ProfileVcs::new(manager.profile_path(&profile_name))?;

    print_step("Staging changes...");
    let hash = vcs.commit(&args.message, &args.files)?;

    print_success(&format!("[{}] {}", hash[..7].bold(), args.message));

    Ok(())
}

pub async fn run_log(args: LogArgs) -> Result<()> {
    let manager = ProfileManager::new()?;
    let profile_name = manager
        .active_profile_name()
        .ok_or_else(|| anyhow::anyhow!("No active profile"))?;

    let vcs = ProfileVcs::new(manager.profile_path(&profile_name))?;
    let commits = vcs.log(args.limit, args.file.as_deref())?;

    if commits.is_empty() {
        print_info("No commit history");
        return Ok(());
    }

    for commit in &commits {
        if args.oneline {
            println!("{} {}", commit.hash[..7].yellow(), commit.message);
        } else {
            println!("{} {}", "commit".yellow(), commit.hash.yellow());
            println!("Author: {}", commit.author);
            println!("Date:   {}", commit.date);
            println!();
            println!("    {}", commit.message);
            println!();
        }
    }

    Ok(())
}

pub async fn run_checkout(args: CheckoutArgs) -> Result<()> {
    let manager = ProfileManager::new()?;
    let profile_name = manager
        .active_profile_name()
        .ok_or_else(|| anyhow::anyhow!("No active profile"))?;

    let vcs = ProfileVcs::new(manager.profile_path(&profile_name))?;

    print_step(&format!("Checking out '{}' ...", args.reference));
    vcs.checkout(&args.reference)?;
    print_success(&format!("Checked out '{}'", args.reference.bold()));
    print_warning(
        "HEAD is now in detached state. Create a tag or branch before making new commits.",
    );

    Ok(())
}

pub async fn run_revert(args: RevertArgs) -> Result<()> {
    let manager = ProfileManager::new()?;
    let profile_name = manager
        .active_profile_name()
        .ok_or_else(|| anyhow::anyhow!("No active profile"))?;

    let vcs = ProfileVcs::new(manager.profile_path(&profile_name))?;
    let target = args.commit.as_deref().unwrap_or("HEAD");

    print_step(&format!("Reverting '{}' ...", target));
    let hash = vcs.revert(target)?;
    print_success(&format!("Reverted: [{}]", hash[..7].bold()));

    Ok(())
}

pub async fn run_tag(args: TagArgs) -> Result<()> {
    let manager = ProfileManager::new()?;
    let profile_name = manager
        .active_profile_name()
        .ok_or_else(|| anyhow::anyhow!("No active profile"))?;

    let vcs = ProfileVcs::new(manager.profile_path(&profile_name))?;

    if args.list || args.name.is_none() {
        let tags = vcs.list_tags()?;
        if tags.is_empty() {
            print_info("No tags");
        } else {
            for tag in &tags {
                println!("{}", tag);
            }
        }
    } else if let Some(name) = &args.name {
        if args.delete {
            vcs.delete_tag(name)?;
            print_success(&format!("Tag '{}' deleted", name));
        } else {
            vcs.create_tag(name, args.message.as_deref())?;
            print_success(&format!("Tag '{}' created", name.bold()));
        }
    }

    Ok(())
}

pub async fn run_doctor(args: DoctorArgs) -> Result<()> {
    println!("{}", "Running clenv diagnostics...".bold());
    println!();

    let manager = ProfileManager::new()?;
    let issues = manager.doctor()?;

    if issues.is_empty() {
        print_success("All checks passed");
        return Ok(());
    }

    let mut has_error = false;

    for issue in &issues {
        match issue.severity.as_str() {
            "error" => {
                print_error(&format!("{}: {}", issue.title, issue.description));
                has_error = true;
            }
            "warning" => {
                print_warning(&format!("{}: {}", issue.title, issue.description));
            }
            _ => {
                print_info(&format!("{}: {}", issue.title, issue.description));
            }
        }

        if args.fix {
            if let Some(fix_fn) = &issue.auto_fix {
                print_step("Applying auto-fix...");
                match manager.apply_fix(fix_fn) {
                    Ok(()) => print_success("Fixed"),
                    Err(e) => print_error(&format!("Auto-fix failed: {}", e)),
                }
            } else {
                print_info(&format!("Manual fix required: {}", issue.fix_hint));
            }
        } else if let Some(hint) = &issue.fix_hint_opt {
            print_step(&format!("Fix: {}", hint.bold()));
        }
        println!();
    }

    if !args.fix && has_error {
        print_info(&format!(
            "To auto-fix issues: {}",
            "clenv doctor --fix".bold()
        ));
    }

    Ok(())
}

// ── .clenvrc commands ─────────────────────────────────────────────────────────

/// Arguments for `clenv rc`
#[derive(Args)]
pub struct RcArgs {
    #[command(subcommand)]
    pub command: RcCommands,
}

#[derive(Subcommand)]
pub enum RcCommands {
    /// Create .clenvrc in the current directory (per-directory profile)
    ///
    /// Example: clenv rc set work
    Set {
        /// Profile name to assign
        profile: String,
    },

    /// Delete .clenvrc from the current directory
    Unset,

    /// Show the currently active profile and its source
    Show,
}

/// Run the `clenv rc` command
pub async fn run_rc(args: RcArgs) -> Result<()> {
    match args.command {
        RcCommands::Set { profile } => {
            let cwd = std::env::current_dir()?;
            RcResolver::set_rc(&cwd, &profile)?;
            print_success(&format!(
                ".clenvrc created in current directory: {}",
                profile.bold()
            ));
            print_info(
                "This profile will be auto-selected when running clenv from this directory.",
            );
        }
        RcCommands::Unset => {
            let cwd = std::env::current_dir()?;
            if RcResolver::unset_rc(&cwd)? {
                print_success(".clenvrc removed");
            } else {
                print_info("No .clenvrc in current directory");
            }
        }
        RcCommands::Show => {
            let cwd = std::env::current_dir()?;
            match RcResolver::resolve(&cwd)? {
                Some(resolved) => {
                    println!(
                        "{} {} {}",
                        "Profile:".bold(),
                        resolved.name.green().bold(),
                        format!("({})", resolved.source.display()).dimmed()
                    );
                }
                None => {
                    print_info("No active profile");
                }
            }
        }
    }
    Ok(())
}

/// Arguments for `clenv resolve-profile`
#[derive(Args)]
pub struct ResolveProfileArgs {
    /// Print only the profile name (for shell scripting)
    #[arg(long)]
    pub quiet: bool,
}

/// Run `clenv resolve-profile`
/// Prints the resolved profile name according to current priority rules
pub async fn run_resolve_profile(args: ResolveProfileArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    match RcResolver::resolve(&cwd)? {
        Some(resolved) => {
            if args.quiet {
                println!("{}", resolved.name);
            } else {
                println!(
                    "{} ({})",
                    resolved.name.green().bold(),
                    resolved.source.display().dimmed()
                );
            }
        }
        None => {
            if !args.quiet {
                print_info("No active profile");
            }
        }
    }
    Ok(())
}

// ── init command ──────────────────────────────────────────────────────────────

#[derive(Args)]
pub struct InitArgs {
    /// Reinitialize even if already initialized (backup is preserved)
    #[arg(long)]
    pub reinit: bool,
}

pub async fn run_init(args: InitArgs) -> Result<()> {
    println!("{}", "Initializing clenv...".bold());
    println!();

    let claude_home = crate::dirs::claude_home();

    // Check and report current ~/.claude state
    if claude_home.exists() && !claude_home.is_symlink() {
        print_step("Checking for existing ~/.claude/ ...");
        print_success("Found existing ~/.claude/ (will be backed up)");
        print_warning("Close Claude Code before running init to avoid file access errors.");
        print_step("Backing up ~/.claude/ to ~/.clenv/backup/original/ ...");
    } else if !claude_home.exists() {
        print_step("No existing ~/.claude/ found — starting fresh");
    }

    let manager = crate::profile::manager::ProfileManager::new()?;

    match manager.initialize(args.reinit) {
        Ok(()) => {}
        Err(e) => {
            print_error(&e.to_string());
            return Ok(());
        }
    }

    let backup_exists = crate::dirs::backup_original_dir().exists();
    if backup_exists && claude_home.exists() {
        print_success("Backup saved to ~/.clenv/backup/original/");
    }

    print_step("Creating 'default' profile ...");
    print_success("Profile 'default' created");
    print_step("Activating 'default' profile ...");
    print_success(&format!(
        "~/.claude → {}",
        crate::dirs::profile_dir("default").display()
    ));
    println!();
    print_success("clenv is ready!");
    print_info(&format!(
        "Run {} to see your profiles.",
        "clenv profile list".bold()
    ));

    Ok(())
}
