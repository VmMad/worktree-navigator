#!/usr/bin/env bash
set -e

if ! command -v wt >/dev/null 2>&1; then
  echo "✗ wt was not found on PATH. Install the wt binary first, then re-run this script." >&2
  exit 1
fi

wt --install-shell bash

echo "  Run: source ~/.bashrc  (or open a new terminal)"
