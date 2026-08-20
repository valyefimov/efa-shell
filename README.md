# efa

A lightweight interactive shell for macOS that learns the commands you run
in each directory and suggests them back to you, Fish-style.

```
~/projects/payslick ❯ pnpm dev
```

Later, in the same directory:

```
~/projects/payslick ❯ pn
```

efa shows an inline autosuggestion for `pnpm dev`, with the already-typed
`pn` rendered normally and the remaining `pm dev` dimmed. Press `Tab` or
`Right Arrow` (at the end of the line) to accept it, then `Enter` to run it.

## v0.1 scope

The only feature in this version is: **remember commands per directory and
suggest them when you start typing.**

Explicitly out of scope for now: AI/LLM suggestions, GitHub integration,
git-aware or filesystem-aware completion, project-root detection, plugins,
cloud sync, accounts, telemetry, and a config UI. See "Development roadmap"
below for what might come later.

## Requirements

- macOS, Apple Silicon (`aarch64-apple-darwin`).
- A normal macOS terminal emulator: Terminal.app, iTerm2, Warp, etc.
- To build from source: a stable Rust toolchain (install via
  [rustup](https://rustup.rs)).

efa does not replace your login shell. You still use zsh/bash normally; efa
is a separate interactive program you start on demand, and it delegates
actual command execution to your `$SHELL` (falling back to `/bin/zsh`).

## Build

```
cargo build
```

## Run (development)

```
cargo run
```

## Build for production

```
cargo build --release
```

## Install

Copy or symlink the release binary onto your `$PATH`, for example:

```
ln -s "$(pwd)/target/release/efa" /usr/local/bin/efa
```

(Homebrew packaging is not set up yet.)

## Example interaction

```
$ efa
~/projects/payslick ❯ pnpm dev
...
~/projects/payslick ❯ exit
$ efa
~/projects/payslick ❯ pn
~/projects/payslick ❯ pnpm dev
                        ^^^^^^ dimmed suggestion, accepted with Tab
```

## Database location

History is stored in a SQLite database at `~/.efa/efa.db`, in a
`command_history` table (`command`, `cwd`, `project_root`, `exit_code`,
`executed_at`). The directory is created automatically on first run.

Every non-empty command is recorded, including built-in `cd` (kept because
directory-navigation history is likely useful for future navigation
features).

Reedline's own `Up`/`Down` history (a separate, plain-text file at
`~/.efa/line_history.txt`) is used only to drive normal up/down recall in
the line editor; SQLite remains the single source of truth for
directory-aware suggestions and ranking.

## Current limitations

- Suggestions only match the exact current directory (`cwd`), not a
  project root — running the same command from a subdirectory of a project
  won't yet surface history from the parent directory.
- The `cd` builtin handles the common forms (`cd`, `cd ~`, `cd ..`, `cd
  ./foo`, `cd /abs/path`) but is not a full zsh-compatible implementation
  (no `cd -`, no `CDPATH`, etc.).
- No AI-based, git-aware, or filesystem-aware suggestions yet — only exact
  history prefix matches.
- No Homebrew formula; install by copying/symlinking the binary.

## Development roadmap

Planned, but intentionally deferred past v0.1:

- Project-root-aware suggestions (distinct from raw `cwd`).
- Additional completion providers beyond history (filesystem, git,
  project-specific commands).
- AI-assisted suggestions.
- GitHub integration.
- Plugin system.
- Homebrew packaging.
