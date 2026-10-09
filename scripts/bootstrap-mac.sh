#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
[[ "$(uname -s)" == Darwin ]] || { echo 'Run this installer on macOS.' >&2; exit 1; }
[[ "$(uname -m)" == arm64 ]] || { echo 'This initial build targets Apple Silicon.' >&2; exit 1; }
command -v node >/dev/null || { echo 'Install Node.js 22 or newer, then rerun.' >&2; exit 1; }
node -e 'if(Number(process.versions.node.split(".")[0])<22)process.exit(1)' || { echo 'Node.js 22+ required.' >&2; exit 1; }
PYTHON="${HWP_WORKBENCH_PYTHON:-python3}"
"$PYTHON" -c 'import sys; assert sys.version_info >= (3,11), "Python 3.11+ required"'
export HWP_WORKBENCH_PYTHON="$(command -v "$PYTHON")"
"$PYTHON" scripts/generate-fixtures.py
"$PYTHON" scripts/run-tests.py
npm install
npm run check
printf '\nStarting the developer application. No document is uploaded by this app.\n'
npm start
