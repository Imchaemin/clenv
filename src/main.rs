// src/main.rs
// claude-env (clenv) - Claude Code environment manager

use anyhow::Result;
use clap::{Parser, Subcommand};

mod cli;
mod config;
mod dirs;
mod error;
mod profile;
mod rc;

use cli::profile as cli_profile;

/// clenv - Claude Code environment manager
///
/// Manages Claude Code configurations (CLAUDE.md, MCP servers, hooks, agents, skills, etc.)
/// as profiles with version control.
///
/// Quick start:
///   clenv init               # initialize clenv (run once after install)
///   clenv profile create work    # create a profile
///   clenv profile use work       # switch profile
///   clenv commit -m "changes"    # save changes
///   clenv profile export work    # export profile
#[derive(Parser)]
#[command(
    name = "clenv",
    about = "Claude Code environment manager",
    long_about = None,
    version,
    arg_required_else_help = true,
)]
struct Cli {
    /// Verbosity level (-v: info, -vv: debug, -vvv: trace)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Disable colored output
    #[arg(long, global = true)]
    no_color: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    // ── Initialization ────────────────────────────────────────────────────────
    /// Initialize clenv: back up ~/.claude/ and set up the default profile
    ///
    /// Run this once after installing clenv.
    ///
    /// Examples:
    ///   clenv init
    ///   clenv init --reinit   # reinitialize (preserves original backup)
    Init(cli_profile::InitArgs),

    // ── Profile management ────────────────────────────────────────────────────
    /// Manage profiles (create, switch, list, delete, export, import, etc.)
    ///
    /// Examples:
    ///   clenv profile create work
    ///   clenv profile use work
    ///   clenv profile export work
    Profile(cli_profile::ProfileArgs),

    // ── Version control (git-based) ───────────────────────────────────────────
    /// Show current profile changes
    Status(cli_profile::StatusArgs),

    /// Show diff between versions
    ///
    /// Examples:
    ///   clenv diff
    ///   clenv diff v1.0..v2.0
    Diff(cli_profile::DiffArgs),

    /// Save changes as a commit
    ///
    /// Examples:
    ///   clenv commit -m "Add GitHub MCP server"
    Commit(cli_profile::CommitArgs),

    /// Show commit history
    Log(cli_profile::LogArgs),

    /// Switch to a specific version
    ///
    /// Examples:
    ///   clenv checkout v1.0
    ///   clenv checkout abc123f
    Checkout(cli_profile::CheckoutArgs),

    /// Revert changes
    Revert(cli_profile::RevertArgs),

    /// Manage version tags
    ///
    /// Examples:
    ///   clenv tag v2.0 -m "Company standard config"
    ///   clenv tag --list
    Tag(cli_profile::TagArgs),

    // ── Diagnostics ───────────────────────────────────────────────────────────
    /// Diagnose configuration (auto-detect issues)
    Doctor(cli_profile::DoctorArgs),

    // ── .clenvrc (per-directory profile) ─────────────────────────────────────
    /// Set per-directory profile (.clenvrc management)
    ///
    /// Like nvm's .nvmrc, specifies a profile per directory (repo).
    ///
    /// Priority: CLENV_PROFILE env var > .clenvrc > ~/.clenvrc > global config
    ///
    /// Examples:
    ///   clenv rc set work      # create .clenvrc in current directory
    ///   clenv rc show          # show current active profile and source
    ///   clenv rc unset         # delete .clenvrc
    Rc(cli_profile::RcArgs),

    /// Print the resolved profile name based on current priority
    ///
    /// Used for shell integration and scripting.
    ///
    /// Examples:
    ///   clenv resolve-profile
    ///   clenv resolve-profile --quiet  # print name only
    #[command(name = "resolve-profile")]
    ResolveProfile(cli_profile::ResolveProfileArgs),

    // ── Uninstall ─────────────────────────────────────────────────────────────
    /// Completely uninstall clenv and restore initial state
    ///
    /// Restores ~/.claude symlink to a real directory,
    /// restores ~/.claude.json MCP config, and deletes clenv data.
    ///
    /// Examples:
    ///   clenv uninstall          # confirm then uninstall
    ///   clenv uninstall -y       # uninstall without confirmation
    ///   clenv uninstall --keep-data  # keep data, restore symlink only
    Uninstall(cli_profile::UninstallArgs),
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let cli = Cli::parse();

    if cli.no_color {
        colored::control::set_override(false);
    }

    let log_level = match cli.verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    std::env::set_var("RUST_LOG", format!("clenv={}", log_level));
    env_logger::init();

    // Initialize clenv directory structure
    dirs::ensure_initialized()?;

    // Enforce initialization: all commands except 'init', 'doctor', and 'uninstall'
    // require clenv to be initialized first.
    let bypass_init_check = matches!(
        cli.command,
        Commands::Init(_) | Commands::Doctor(_) | Commands::Uninstall(_)
    );

    if !bypass_init_check && !dirs::is_clenv_initialized() {
        use colored::Colorize;
        eprintln!("{}: clenv is not initialized.", "error".red().bold());
        eprintln!("Run {} to set up clenv.", "clenv init".bold());
        std::process::exit(1);
    }

    let result = match cli.command {
        // Init
        Commands::Init(args) => cli_profile::run_init(args).await,

        // Profile management
        Commands::Profile(args) => cli_profile::run(args).await,

        // Version control
        Commands::Status(args) => cli_profile::run_status(args).await,
        Commands::Diff(args) => cli_profile::run_diff(args).await,
        Commands::Commit(args) => cli_profile::run_commit(args).await,
        Commands::Log(args) => cli_profile::run_log(args).await,
        Commands::Checkout(args) => cli_profile::run_checkout(args).await,
        Commands::Revert(args) => cli_profile::run_revert(args).await,
        Commands::Tag(args) => cli_profile::run_tag(args).await,

        // Diagnostics
        Commands::Doctor(args) => cli_profile::run_doctor(args).await,

        // .clenvrc
        Commands::Rc(args) => cli_profile::run_rc(args).await,
        Commands::ResolveProfile(args) => cli_profile::run_resolve_profile(args).await,

        // Uninstall
        Commands::Uninstall(args) => cli_profile::run_uninstall(args).await,
    };

    if let Err(e) = result {
        use colored::Colorize;
        eprintln!("{}: {}", "error".red(), e);

        if let Some(clenv_err) = e.downcast_ref::<crate::error::ClenvError>() {
            std::process::exit(clenv_err.exit_code());
        }
        std::process::exit(1);
    }

    Ok(())
}
