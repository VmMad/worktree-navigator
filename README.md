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

If `wt` detects a zsh or bash shell, it also refreshes the `wt()` shell wrapper and tells you to restart the console.

## Usage

Open the interactive UI inside a repo or inside a worktree:

```bash
wt
```

Fast CLI commands:

```bash
wt pr 123
wt pr #123

wt gco
wt gco feature/login

wt b feature/login
wt b feature/login --from-default
wt b feature/login --base release/1

wt d
wt d feature/login
wt d feature/login --yes
```

Mark existing worktree repo:

```bash
wt --mark-tree
```

Main commands:

- `New Branch [b]` create a new branch worktree and jump into it
- `Rename Worktree [m]` rename the selected non-default branch and move its worktree folder to match
- `Sync with PR [p]` enter a PR number (`#123` or `123`) and create/sync its worktree
- `Delete Worktree [d]` inline select in the worktree list, then confirm with `Enter` or `y` (`n`/`Esc` cancels)
- `Sync Worktree [s]` inline select a worktree to fast-forward from `origin/<branch>`
- `Copy Secrets [c]` copy secret files into the selected worktree
- `Checkout Remote [r]` fetch a remote branch and create a worktree for it

CLI commands:

- `wt pr <number>` or `wt checkout-pr <number>` fetch the PR head branch, create/select its worktree, and print the destination path
- `wt gco [branch]` or `wt checkout [branch]` jump to an existing worktree and print its path
- `wt b <branch>` or `wt branch <branch>` create a new branch worktree and print the destination path
- `wt b <branch> --from-default` branch from the repo default branch instead of the current branch
- `wt b <branch> --from-current` force the current-branch base explicitly
- `wt b <branch> --base <branch>` branch from an explicit base branch
- `wt d [branch]` or `wt delete [branch]` delete a worktree by branch, or the current worktree when no branch is passed
- `wt d ...` requires typing the branch name to confirm unless `--yes` is passed

Notes:

- `wt b <branch>` defaults to branching from the current branch when run inside a worktree
- `wt gco` defaults to the default-branch worktree
- if the current directory is not itself a git worktree, `wt b <branch>` falls back to the repo default branch
- when `wt d` deletes the current worktree, the shell wrapper moves you back to the repo root

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

## Testing

Run the CLI end-to-end suite with:

```bash
cargo test --test cli_e2e
```
