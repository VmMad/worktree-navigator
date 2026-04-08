#!/usr/bin/env bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# Prefer a pre-built binary from a release tarball (named worktree-navigator-*)
# then fall back to a local cargo build.
RELEASE_BINARIES=()
for f in "$SCRIPT_DIR"/worktree-navigator-*; do
  if [[ -f "$f" && -x "$f" ]]; then
    RELEASE_BINARIES+=("$f")
  fi
done

BINARY_SRC=""
if [[ ${#RELEASE_BINARIES[@]} -eq 1 ]]; then
  BINARY_SRC="${RELEASE_BINARIES[0]}"
elif [[ ${#RELEASE_BINARIES[@]} -gt 1 ]]; then
  echo "Multiple release binaries found in $SCRIPT_DIR; refusing to guess which one to install:"
  for f in "${RELEASE_BINARIES[@]}"; do
    echo "  - $f"
  done
  echo "Please keep only the correct release binary for this system, or remove the extras and rerun."
  exit 1
fi

if [[ -z "$BINARY_SRC" ]]; then
  BINARY_SRC="$SCRIPT_DIR/target/release/worktree-navigator"
fi

if [[ ! -f "$BINARY_SRC" ]]; then
  echo "Binary not found. Either:"
  echo "  - Run: cargo build --release"
  echo "  - Or download a release tarball from GitHub Releases"
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
