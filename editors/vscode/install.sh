#!/bin/bash
# Build and install the Conch Plugin Support extension for VS Code.
set -e

cd "$(dirname "$0")"
npm install --silent
npx tsc -p ./
npx --yes @vscode/vsce package
code --install-extension conch-lua-*.vsix
echo "Conch Plugin Support extension installed. Restart VS Code to activate."
