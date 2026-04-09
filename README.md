# Worktree-navigator

Interactive UI for managing git worktrees on Ubuntu.

`wt` opens full screen, works with keyboard and mouse, and lets you jump between worktrees without typing long git commands.

For a maintainer quick reference (setup, dev commands, workflow, and file map), see [CONTRIBUTING.md](./CONTRIBUTING.md).

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
- `Sync GH PR [p]` enter a PR number (`#123` or `123`) and create/sync its worktree
- `Delete Worktree [d]` inline select in the worktree list, then confirm with `Enter` or `y` (`n`/`Esc` cancels)
- `Sync Trees [s]` inline select a branch to sync from `origin/<branch>`

Navigation:

- `↑↓` or `j/k` move
- `Enter` or click activate
- mouse scroll moves selection
- `Esc` cancel current mode
- `q` quit

## No-repo flow

If you run `wt` in a directory that is not a git repo, it opens a clone flow:

1. Enter repo source (`owner/repo`, SSH URL or HTTPS URL)
2. Confirm or edit destination (defaults to `<current-working-directory>/<repo-name>`)
3. Clone repo and jump into the default branch folder (for example `<repo>/main`)

For `owner/repo`, `wt` uses `gh repo clone` when available, and falls back to `git clone` using your preferred GitHub protocol (SSH/HTTPS)

## Requirements

- Linux (Ubuntu tested)
- `git`
- `gh` for PR sync
- `zsh` if you want the `wt` shell wrapper
