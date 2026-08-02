#!/bin/bash
# Build a installable WhimprFlow.app (+ optional .dmg) for personal use on macOS.
set -euo pipefail
cd "$(dirname "$0")"

if [ ! -x "ui/node_modules/.bin/tauri" ]; then
  echo "Installing UI dependencies…"
  npm install --prefix ui
fi

# Native deps for Whisper + optional GGUF worker.
if ! command -v cmake >/dev/null 2>&1; then
  echo "cmake is required — install with: brew install cmake"
  exit 1
fi

if [ -f "$HOME/.cargo/env" ]; then
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

echo "Building release app bundle…"
ui/node_modules/.bin/tauri build --bundles app,dmg

APP="src-tauri/target/release/bundle/macos/WhimprFlow.app"
DMG="src-tauri/target/release/bundle/dmg/WhimprFlow_"*.dmg

echo ""
echo "✓ Built: $APP"
if compgen -G "$DMG" >/dev/null; then
  echo "✓ DMG:   $(ls -1 $DMG | head -1)"
fi
echo ""
echo "Install (unsigned, personal use):"
echo "  cp -R \"$APP\" /Applications/"
echo "  xattr -cr /Applications/WhimprFlow.app   # clear quarantine if macOS blocks it"
echo ""
echo "Models are NOT bundled. Place Whisper + optional GGUF under:"
echo "  ~/Library/Application Support/WhimprFlow/models/"
