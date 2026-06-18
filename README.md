# Worktree Navigator

> Interactive UI for managing git worktrees on Ubuntu.

`wt` opens full-screen, works with keyboard and mouse, and lets you jump between worktrees without typing long git commands.

---

## Install

### From a GitHub release

```bash
curl -fsSL https://github.com/VmMad/worktree-navigator/releases/latest/download/worktree-navigator-x86_64-linux-gnu \
  -o ~/.local/bin/wt && chmod +x ~/.local/bin/wt
```

Then add the `wt()` shell wrapper so navigating to a worktree changes your shell's directory:

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

If `wt` detects a zsh or bash shell, it also refreshes the `wt()` shell wrapper and prompts you to restart the console.

---

## Usage

### Interactive UI

Open the UI inside a repo or a worktree:

```bash
wt
```

#### UI commands

| Command | Key | Description |
|---|---|---|
| New Branch | `b` | Create a new branch worktree and jump into it |
| Rename Worktree | `m` | Rename the selected non-default branch and move its folder |
| Sync with PR | `p` | Enter a PR number and create/sync its worktree |
| Delete Worktree | `d` | Select inline, then confirm with `Enter` or `y` |
| Sync Worktree | `s` | Fast-forward a worktree from `origin/<branch>` |
| Copy Secrets | `c` | Copy secret files into the selected worktree |
| Options | `o` | Configure repo-local post-create commands for new worktrees |
| Checkout Remote | `r` | Fetch a remote branch and create a worktree for it |

#### Navigation

| Key | Action |
|---|---|
| `↑` / `↓` or `j` / `k` | Move selection |
| `Enter` or click | Activate |
| Mouse scroll | Move selection |
| `Esc` | Cancel current mode |
| `q` | Quit |

---

### CLI commands

Clone a repo into a worktree workspace:

```bash
wt clone owner/repo
wt clone git@github.com:owner/repo.git
wt clone https://github.com/owner/repo.git ~/src/repo
```

Check out a PR:

```bash
wt pr 123
wt pr #123
```

Jump to an existing worktree:

```bash
wt gco
wt gco feature/login
```

Create a new branch worktree:

```bash
wt b feature/login
wt b feature/login --from-default
wt b feature/login --base release/1
```

Delete a worktree:

```bash
wt d
wt d feature/login
wt d feature/login --yes
```

Mark an existing repo as a worktree workspace:

```bash
wt --mark-tree
```

#### CLI reference

| Command | Aliases | Description |
|---|---|---|
| `wt clone <repo> [dest]` | | Clone into a worktree workspace, print the default-branch path |
| `wt pr <number>` | `checkout-pr` | Fetch the PR head branch, create/select its worktree |
| `wt gco [branch]` | `checkout` | Jump to an existing worktree; defaults to the default branch |
| `wt b <branch>` | `branch` | Create a new branch worktree |
| `wt b <branch> --from-default` | | Branch from the repo default branch |
| `wt b <branch> --from-current` | | Force the current branch as base |
| `wt b <branch> --base <branch>` | | Branch from an explicit base |
| `wt d [branch]` | `delete` | Delete a worktree by branch, or the current one if omitted |
| `wt d ... --yes` | | Skip the branch-name confirmation prompt |

#### Notes

- `wt b <branch>` defaults to the current branch when run inside a worktree; falls back to the repo default branch otherwise.
- `o` opens repo-local options where you can add shell commands to run automatically after creating a new worktree.
- Post-create commands run inside the new worktree and receive `WT_REPO_ROOT`, `WT_WORKTREE_PATH`, `WT_WORKTREE_BRANCH`, `WT_WORKTREE_BASE_BRANCH`, and `WT_DEFAULT_WORKTREE_PATH`.
- `wt gco` with no argument goes to the default-branch worktree.
- When `wt d` deletes the current worktree, the shell wrapper moves you back to the repo root.

---

## Requirements

- Linux
- `git`
- `gh` (for PR sync)
- `zsh` or `bash` (for the `wt` shell wrapper)

---

## Testing

```bash
cargo test --test cli_e2e
```

---

## License

MIT
