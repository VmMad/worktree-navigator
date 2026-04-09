#!/usr/bin/env bash
set -e

ZSHRC="$HOME/.zshrc"
MARKER="# worktree-navigator wt()"

if grep -qF "$MARKER" "$ZSHRC" 2>/dev/null; then
  echo "✓ wt() already present in $ZSHRC — nothing to do."
  exit 0
fi

cat >> "$ZSHRC" << 'EOF'

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
EOF

echo "✓ Added wt() to $ZSHRC"
echo "  Run: source ~/.zshrc  (or open a new terminal)"
