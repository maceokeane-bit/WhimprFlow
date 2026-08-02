#!/bin/bash
# Run WhimprFlow in development: starts the Vite UI server + the app with hot reload.
set -e
cd "$(dirname "$0")"

if [ ! -x "ui/node_modules/.bin/tauri" ]; then
  echo "Installing UI dependencies (first run)..."
  npm install --prefix ui
fi

exec ui/node_modules/.bin/tauri dev "$@"
