# Worktree-navigator

Interactive UI for managing git worktrees on Ubuntu.

`wt` opens full screen, works with keyboard and mouse, and lets you jump between worktrees without typing long git commands.

## Install

### From source

```bash
cargo build --release
./install.sh
source ~/.zshrc
```

### From a release archive

1. Download the right `worktree-navigator-*.tar.gz` from GitHub Releases
2. Extract it
3. Run:

```bash
./install.sh
source ~/.zshrc
```

`install.sh` installs `worktree-navigator` into `~/.local/bin` and adds this wrapper to `~/.zshrc`:

```zsh
wt() {
  local target
  target=$(WT_CWD="$PWD" worktree-navigator "$@")
  local exit_code=$?
  if [[ -n "$target" && -d "$target" ]]; then
    cd "$target"
  fi
  return $exit_code
}
```

## Usage

Run inside a repo or inside a worktree:

```bash
wt
```

Main commands:

- `New Branch [n]` create a new branch worktree and jump into it
- `Sync GH PR [p]` pick an open PR and create/sync its worktree
- `Delete Worktree [d]` inline select in the worktree list, then confirm `[y/n]`
- `Sync Trees [s]` inline select a branch to sync from `origin/<branch>`
- `Refresh List [r]` re-read local worktrees

Navigation:

- `↑↓` or `j/k` move
- `Enter` or click activate
- mouse scroll moves selection
- `Esc` cancel current mode
- `q` quit

## No-repo flow

If you run `wt` in a directory that is not a git repo, it opens a clone flow:

1. Enter repo URL
2. Confirm or edit destination (defaults to `~/Projects/trees/<repo-name>`)
3. Clone as bare repo, create initial worktree, and jump into it

## Requirements

- Linux (Ubuntu tested)
- `git`
- `gh` for PR sync
- `zsh` if you want the `wt` shell wrapper
