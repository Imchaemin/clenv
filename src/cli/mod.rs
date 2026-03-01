// src/cli/mod.rs
// CLI module entry point
//
// Each subcommand group is split into a child module.
// This file re-exports the child modules publicly.

pub mod profile;

/// Print a success message to the terminal (green ✓)
pub fn print_success(msg: &str) {
    use colored::Colorize;
    println!("{} {}", "✓".green().bold(), msg);
}

/// Print an error message to the terminal (red ✗)
pub fn print_error(msg: &str) {
    use colored::Colorize;
    eprintln!("{} {}", "✗".red().bold(), msg);
}

/// Print a warning message to the terminal (yellow ⚠)
pub fn print_warning(msg: &str) {
    use colored::Colorize;
    println!("{} {}", "⚠".yellow().bold(), msg);
}

/// Print an info message to the terminal (blue ℹ)
pub fn print_info(msg: &str) {
    use colored::Colorize;
    println!("{} {}", "ℹ".blue(), msg);
}

/// Print a step message to the terminal (dimmed →)
pub fn print_step(msg: &str) {
    use colored::Colorize;
    println!("  {} {}", "→".dimmed(), msg);
}
