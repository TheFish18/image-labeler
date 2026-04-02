#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${TARGET:-x86_64-pc-windows-gnu}"
BIN_NAME="${BIN_NAME:-image-labeler}"

cd "$ROOT_DIR"

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo is not installed or not on PATH" >&2
  exit 1
fi

if ! rustup target list --installed | grep -qx "$TARGET"; then
  echo "Installing Rust target $TARGET"
  rustup target add "$TARGET"
fi

echo "Building $BIN_NAME for $TARGET in release mode"
cargo build --release --target "$TARGET"

ARTIFACT="target/$TARGET/release/${BIN_NAME}.exe"
if [[ -f "$ARTIFACT" ]]; then
  echo "Build complete: $ARTIFACT"
else
  echo "warning: build finished but expected artifact was not found at $ARTIFACT" >&2
fi
