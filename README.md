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


### Update existing install

```bash
wt --update
```

## Usage

Run inside a repo or inside a worktree:

```bash
wt
```

Mark existing worktree repo:

```bash
wt --mark-tree
```

Main commands:

- `New Branch [b]` create a new branch worktree and jump into it
- `Sync with PR [p]` enter a PR number (`#123` or `123`) and create/sync its worktree
- `Delete Worktree [d]` inline select in the worktree list, then confirm with `Enter` or `y` (`n`/`Esc` cancels)
- `Sync Worktree [s]` inline select a worktree to fast-forward from `origin/<branch>`
- `Copy Secrets [c]` copy secret files into the selected worktree
- `Checkout Remote [r]` fetch a remote branch and create a worktree for it

Navigation:

- `↑↓` or `j/k` move
- `Enter` or click activate
- mouse scroll moves selection
- `Esc` cancel current mode
- `q` quit

## Requirements

- Linux
- `git`
- `gh` for PR sync
- `zsh` or `bash` if you want the `wt` shell wrapper
