#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
REPO="${1:-jonathanjie/hwp-workbench}"
[[ "$REPO" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || { echo 'Expected owner/repository.' >&2; exit 1; }
command -v gh >/dev/null || { echo 'GitHub CLI (gh) is required.' >&2; exit 1; }
gh auth status
if gh repo view "$REPO" --json name >/dev/null 2>&1; then
  echo "Repository already exists: $REPO. This script will not modify or overwrite an existing repository." >&2
  exit 1
fi
[[ -d .git ]] || git init -b main
# Use the developer's configured Git identity; do not invent one.
git var GIT_AUTHOR_IDENT >/dev/null
# .gitignore excludes third-party samples, all user documents, drafts, node_modules and builds.
git add .
git diff --cached --stat
printf "\nReview the staged files for secrets and document data.\n"
read -r -p "Create and push new PRIVATE repository $REPO? Type yes: " CONFIRM
[[ "$CONFIRM" == yes ]] || { echo "Publication cancelled; no remote repository was created."; exit 1; }
git diff --cached --quiet || git commit -m 'Initial preservation-first HWP Workbench alpha'
gh repo create "$REPO" --private --source=. --remote=origin --push --description 'Internal HWP/HWPX editor with shared CLI and MCP operations'
