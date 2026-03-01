[한국어](README.ko.md) | English

# clenv — Claude Code Environment Manager

<p align="center">
  <img src="public/logo.png" alt="clenv" />
</p>

`clenv` is a CLI tool for managing multiple Claude Code profiles. Each profile is an independent `~/.claude` directory — with its own CLAUDE.md, MCP servers, hooks, agents, and skills — backed by git for version control.

## Why clenv?

Claude Code stores all of its configuration in `~/.claude`. As your workflows grow more complex, a single global configuration becomes a liability:

- Your **work** project enforces strict security policies; your **side project** needs a relaxed, experimental setup.
- You're building an **AI agent** and need one profile for heavy-tooling development and a separate, locked-down profile for production runs.
- Your team has a **standard baseline config** that individuals extend without diverging from the shared foundation.

Modern developers don't have a single identity. Your persona shifts depending on the project you're in, the role you play on a team, and the tools you reach for. A developer building an AI agent needs a completely different Claude Code setup than the same person reviewing a pull request or onboarding a new teammate. `clenv` makes it natural to maintain one profile per persona — and switch between them instantly.

`clenv` solves this by treating each configuration as a first-class, version-controlled profile.

## Who uses clenv?

### The Multi-context Developer

You juggle work and personal projects, or maintain separate environments for different clients. Switching context means switching profiles — one command, instant isolation.

```sh
clenv profile use work      # strict MCP, company CLAUDE.md
clenv profile use personal  # open tools, personal preferences
```

### The AI Agent Developer

As you build and iterate on agents, your Claude Code environment _is_ part of the product. Different agent configurations, MCP server combinations, and hook setups need to be versioned and reproducible — just like your source code.

```sh
clenv profile create agent-dev --from baseline
clenv profile use agent-dev   # heavy tooling: all MCP servers, debug hooks
# ... iterate on your agent ...
clenv commit -m "tune researcher agent prompts"

clenv profile create agent-prod --from agent-dev
clenv profile use agent-prod  # locked-down: only production MCP servers
clenv tag v1.0 -m "production agent config"
```

Profiles store everything Claude Code reads: `CLAUDE.md`, `settings.json`, `.mcp.json`, `hooks/`, `agents/`, `skills/`. Version-control them, tag releases, export for teammates.

### The Team

Maintain a canonical team profile. Teammates import it as their starting point, then layer personal customizations on top — without losing the shared baseline.

```sh
# export the team standard
clenv profile export team-standard -o team-standard.clenvprofile

# import and extend
clenv profile import team-standard.clenvprofile --use
clenv commit -m "add my personal keybindings"
```

---

## Installation

### Homebrew (recommended)

```sh
brew tap chaaaamni/clenv
brew install clenv
```

### cargo install

```sh
cargo install clenv
```

### Build from source

```sh
git clone https://github.com/chaaaamni/clenv.git
cd clenv
cargo build --release
# binary: ./target/release/clenv
```

---

## Quick Start

```sh
clenv init                        # back up ~/.claude/ and create the default profile
clenv profile create work --use   # create and immediately switch to a profile
# ... edit ~/.claude/ as you like ...
clenv commit -m "initial work config"
```

---

## Usage

### Initialization

```sh
# Initialize clenv (run once after install)
# Backs up your existing ~/.claude/ and creates the 'default' profile
clenv init

# Re-initialize without losing the original backup
clenv init --reinit
```

### Profile management

```sh
# Create a new profile
clenv profile create work

# Create and switch immediately
clenv profile create work --use

# Create from an existing profile
clenv profile create agent-prod --from agent-dev

# List all profiles
clenv profile list

# Switch to a profile (updates ~/.claude symlink)
clenv profile use work

# Show the currently active profile name
clenv profile current

# Delete a profile
clenv profile delete old-profile --force

# Clone a profile
clenv profile clone work work-backup

# Rename a profile
clenv profile rename work-backup archived

# Deactivate clenv — restore ~/.claude to a real directory
clenv profile deactivate
clenv profile deactivate --purge   # also delete ~/.clenv/ data
```

### Version control

Each profile is a git repository. Use these commands while a profile is active:

```sh
# Show uncommitted changes
clenv status

# Show a diff
clenv diff
clenv diff HEAD~1..HEAD     # between commits
clenv diff --name-only      # file names only

# Commit current state
clenv commit -m "add GitHub MCP server"

# Commit specific files only
clenv commit -m "update hooks" hooks/

# View commit history
clenv log
clenv log --oneline
clenv log -n 10             # last 10 commits

# Switch to a specific version
clenv checkout v1.0
clenv checkout abc123f

# Revert to the previous commit
clenv revert

# Revert to a specific commit
clenv revert abc123f
```

### Tags

```sh
# Create a tag on the current commit
clenv tag v2.0 -m "Company standard config"

# List all tags
clenv tag --list

# Delete a tag
clenv tag v2.0 --delete
```

### Export / Import

```sh
# Export the active profile
clenv profile export -o my-profile.clenvprofile

# Export a specific profile
clenv profile export work -o work.clenvprofile

# Export without plugins or marketplace items
clenv profile export work --no-plugins --no-marketplace

# Import a profile
clenv profile import my-profile.clenvprofile

# Import, name it, and switch immediately
clenv profile import my-profile.clenvprofile --name imported --use
```

> MCP API keys are automatically redacted during export. You will be prompted to re-enter them after importing.

### Per-directory profile (.clenvrc)

Like `.nvmrc` for Node.js, `.clenvrc` specifies a profile per directory. Priority: `CLENV_PROFILE` env var > `.clenvrc` > global active profile.

```sh
# Pin a profile to the current directory
clenv rc set work

# Show which profile is active and where it comes from
clenv rc show

# Remove the per-directory pin
clenv rc unset

# Print the resolved profile name (useful for shell scripting)
clenv resolve-profile
clenv resolve-profile --quiet   # name only
```

### Diagnostics

```sh
# Detect common issues
clenv doctor

# Detect and auto-fix issues
clenv doctor --fix
```

### Uninstall

```sh
# Remove clenv and restore the original ~/.claude/
clenv uninstall

# Skip confirmation
clenv uninstall -y

# Restore symlink but keep ~/.clenv/ data
clenv uninstall --keep-data
```

---

## Requirements

- macOS or Linux
- No additional dependencies (statically linked binary via Homebrew)

## Shell completions

```sh
# bash
clenv completions bash >> ~/.bash_completion

# zsh
clenv completions zsh > ~/.zsh/completions/_clenv

# fish
clenv completions fish > ~/.config/fish/completions/clenv.fish
```

## Contributing

Issues and pull requests are welcome at [github.com/Imchaemin/clenv](https://github.com/Imchaemin/clenv).

## License

MIT — see [LICENSE](LICENSE).
