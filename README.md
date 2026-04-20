# Worktree-navigator

Interactive UI for managing git worktrees on Ubuntu.

`wt` opens full screen, works with keyboard and mouse, and lets you jump between worktrees without typing long git commands.

## Install

### From a GitHub release

```bash
curl -fsSL https://github.com/VmMad/worktree-navigator/releases/latest/download/worktree-navigator-x86_64-linux-gnu \
  -o ~/.local/bin/wt && chmod +x ~/.local/bin/wt
```

Then add the `wt()` shell wrapper so navigating to a worktree changes your shell's directory. Pick the installer that matches your shell:

**zsh**

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/VmMad/worktree-navigator/main/scripts/zsh-install.sh) \
  && source ~/.zshrc
```

**bash**

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/VmMad/worktree-navigator/main/scripts/bash-install.sh) \
  && source ~/.bashrc
```

### From source

```bash
cargo build --release && cp target/release/worktree-navigator ~/.local/bin/wt
```

Then add the `wt()` shell wrapper so navigating to a worktree changes your shell's directory. Pick the installer that matches your shell:

**zsh**

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/VmMad/worktree-navigator/main/scripts/zsh-install.sh) \
  && source ~/.zshrc
```

**bash**

```bash
bash <(curl -fsSL https://raw.githubusercontent.com/VmMad/worktree-navigator/main/scripts/bash-install.sh) \
  && source ~/.bashrc
```

### Update existing install

If you already have the `wt` binary installed and the `wt()` wrapper in `~/.zshrc`, you only need to replace the binary.

From the latest GitHub release:

```bash
curl -fsSL https://github.com/VmMad/worktree-navigator/releases/latest/download/worktree-navigator-x86_64-linux-gnu \
  -o ~/.local/bin/wt && chmod +x ~/.local/bin/wt
```

From source:

```bash
cargo build --release && cp target/release/worktree-navigator ~/.local/bin/wt
```

No `~/.zshrc` changes are needed if `wt()` is already present.

You can also run:

```bash
wt --update
```

When a newer release is available, `wt` now checks on startup and prompts:

- `y` updates immediately
- `n` disables future startup prompts and shows a reminder to use `wt --update`

`wt --update` selects the release asset matching your current binary target (for example `x86_64-linux-gnu` or `aarch64-linux-musl`) and remembers that asset for future updates.

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
3. Clone repo, show a loading indicator while it runs, and jump into the default branch folder (for example `<repo>/main`)

For `owner/repo`, `wt` uses `gh repo clone` when available, and falls back to `git clone` using your preferred GitHub protocol (SSH/HTTPS)

## Requirements

- Linux (Ubuntu tested)
- `git`
- `gh` for PR sync
- `zsh` or `bash` if you want the `wt` shell wrapper
