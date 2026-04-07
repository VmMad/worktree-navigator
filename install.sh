#!/usr/bin/env bash
set -e

BINARY_SRC="$(cd "$(dirname "$0")" && pwd)/target/release/worktree-navigator"

if [[ ! -f "$BINARY_SRC" ]]; then
  echo "Binary not found. Run: cargo build --release"
  exit 1
fi

# Install binary to ~/.local/bin
INSTALL_DIR="$HOME/.local/bin"
mkdir -p "$INSTALL_DIR"
cp "$BINARY_SRC" "$INSTALL_DIR/worktree-navigator"
chmod +x "$INSTALL_DIR/worktree-navigator"
echo "✓ Installed to $INSTALL_DIR/worktree-navigator"

# Ensure ~/.local/bin is in PATH
if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
  echo "  Note: add $INSTALL_DIR to your PATH if not already set."
fi

# Append wt() shell function to ~/.zshrc (idempotent)
ZSHRC="$HOME/.zshrc"
MARKER="# worktree-navigator wt()"

if grep -qF "$MARKER" "$ZSHRC" 2>/dev/null; then
  echo "✓ wt() already present in $ZSHRC — skipping."
else
  cat >> "$ZSHRC" << 'EOF'

# worktree-navigator wt()
wt() {
  local target
  target=$(WT_CWD="$PWD" worktree-navigator "$@")
  local exit_code=$?
  if [[ -n "$target" && -d "$target" ]]; then
    cd "$target"
  fi
  return $exit_code
}
EOF
  echo "✓ Added wt() function to $ZSHRC"
  echo "  Run: source ~/.zshrc  (or open a new terminal)"
fi
