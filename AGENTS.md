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

- `install.sh` installs the binary to `~/.local/bin` and sets a `wt()` zsh wrapper.
- Running `wt` outside a repo starts clone flow and creates an initial worktree.
- PR sync accepts `123` or `#123` and checks out `pr-<number>`.
- Delete/Sync Trees actions are inline select modes in the worktree list.

## Key files

- `src/main.rs`: event loop, input handlers, action transitions.
- `src/app.rs`: app state and command list.
- `src/ui.rs`: ratatui rendering and overlays.
- `src/git.rs`: git/gh integration.
- `src/types.rs`: shared domain types.
- `install.sh`: install + shell wrapper wiring.
