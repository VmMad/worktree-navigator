# AI Instructions

Repository-specific guidance for AI coding agents working in `worktree-navigator`.

## Scope and constraints

- Keep changes focused and avoid unrelated refactors.
- Do not alter runtime behavior unless explicitly requested.
- Prefer docs/config updates when the request is documentation-only.
- Keep README updates selective: document important user-facing workflows or behavior changes, not every individual feature or implementation detail.

## Build, lint, test

```bash
cargo build
cargo fmt
cargo clippy --all-targets --all-features
cargo test
```

## Local run and install

```bash
WT_CWD="$PWD" cargo run
cargo build --release
./install.sh
source ~/.zshrc
wt
```

## Expected workflow

- Running `wt` outside a repo starts clone flow and creates an initial worktree.
- PR sync accepts `123` or `#123` and checks out `pr-<number>`.
- Delete/Sync Trees actions are inline select modes in the worktree list.

## Functionality notes

### No-repo / clone flow

Running `wt` outside a git repo opens a clone overlay:

1. Enter a repo source (`owner/repo`, SSH URL, or HTTPS URL).
2. Confirm or edit the destination (defaults to `<cwd>/<repo-name>`).
3. `wt` clones the repo (using `gh repo clone` when available, falling back to `git clone`) and jumps into the default branch folder (e.g. `<repo>/main`).

### `wt --mark-tree`

Marks an existing directory as a worktree repo root so `wt` treats it as part of a worktree workspace.

### Update mechanism (`wt --update`)

- `wt --update` fetches the release asset matching the current binary target (e.g. `x86_64-linux-gnu`) and replaces the binary in place.
- In the background, `wt` checks for a newer release while the TUI is open and prints a notice to stderr at most once per day after exit.
- Release builds carry the tagged version. Local (`-dev`) builds skip update notices.

### Build from source

```bash
cargo build --release && cp target/release/worktree-navigator ~/.local/bin/wt
```

Then run the shell wrapper installer for your shell (see README Install section).

## Key files

- `src/main.rs`: event loop, input handlers, action transitions.
- `src/app.rs`: app state and command list.
- `src/ui.rs`: ratatui rendering and overlays.
- `src/git.rs`: git/gh integration.
- `src/types.rs`: shared domain types.
- `install.sh`: install + shell wrapper wiring.
