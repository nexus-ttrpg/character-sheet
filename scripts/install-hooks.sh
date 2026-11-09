#!/usr/bin/env bash
#
# install-hooks.sh — point git at the repo's tracked hooks directory.
#
# Uses core.hooksPath rather than copying files into .git/hooks, so the hooks
# stay version-controlled and every clone gets the same ones with one command.
#
#   scripts/install-hooks.sh            # install
#   scripts/install-hooks.sh --uninstall

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

if [[ "${1:-}" == "--uninstall" ]]; then
    git config --unset core.hooksPath 2>/dev/null || true
    echo "Hooks uninstalled (core.hooksPath cleared)."
    exit 0
fi

chmod +x .githooks/* scripts/ci-local.sh 2>/dev/null || true
git config core.hooksPath .githooks

cat <<'EOF'
Hooks installed: core.hooksPath -> .githooks

  pre-commit  runs the GitHub Actions checks for the suites your staged files
              touch (scripts/ci-local.sh --changed)

Useful escapes:
  NEXUS_PRECOMMIT_SCOPE=all git commit ...   run every check, not just affected
  NEXUS_PRECOMMIT_FAST=1 git commit ...      skip the audit jobs
  SKIP_CHECKS=clippy git commit ...          skip checks by id
  git commit --no-verify                     bypass the hook
  NEXUS_PRECOMMIT=0 git commit ...           disable without uninstalling

Run the full suite by hand any time:
  scripts/ci-local.sh            # everything
  scripts/ci-local.sh --list     # available check ids
EOF
