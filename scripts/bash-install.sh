#!/usr/bin/env bash
set -e

BASHRC="$HOME/.bashrc"
MARKER="# worktree-navigator wt()"

if grep -qF "$MARKER" "$BASHRC" 2>/dev/null; then
  echo "✓ wt() already present in $BASHRC — nothing to do."
  exit 0
fi

cat >> "$BASHRC" << 'EOT'

# worktree-navigator wt()
wt() {
  local target
  target=$(WT_CWD="$PWD" command wt "$@")
  local exit_code=$?
  if [[ -n "$target" && -d "$target" ]]; then
    cd "$target"
  fi
  return $exit_code
}
EOT

echo "✓ Added wt() to $BASHRC"
echo "  Run: source ~/.bashrc  (or open a new terminal)"
