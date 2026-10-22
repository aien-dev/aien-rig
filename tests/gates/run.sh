#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
step="${1:?usage: tests/gates/run.sh <step>}"
obs() { printf 'AIEN-OBS %s %s\n' "$1" "$2"; }
case "$step" in

  zero-disk-secrets)
    found=false
    if find . -type f \( -name ".env" -o -name ".env.*" \) \
         ! -name "*.example" ! -name "*.sample" ! -path "./target/*" | head -1 | grep -q .; then
      found=true
      find . -type f \( -name ".env" -o -name ".env.*" \) \
        ! -name "*.example" ! -name "*.sample" ! -path "./target/*" | head -5 >&2
    fi
    obs env_files_found "$found"
    [ "$found" = false ]
    ;;

  structure)
    bad=0
    [ -s README.md ] || { echo "README.md missing or empty" >&2; bad=1; }
    for d in app creator docs engine speech tuner; do
      [ -d "$d" ] || { echo "missing directory: $d" >&2; bad=1; }
    done
    # every directory in the tree must carry at least one tracked file
    empties=0
    for d in $(find . -type d -not -path './.git*' -not -path './tests/gates' -not -path './.aien' -not -path './.github*' | sort); do
      [ -n "$(ls -A "$d" 2>/dev/null)" ] || { echo "empty directory: $d" >&2; empties=$((empties+1)); }
    done
    obs dirs_checked "$(find . -type d -not -path './.git*' | wc -l)"
    obs empty_dirs "$empties"
    [ "$bad" = 0 ]
    ;;
  *) echo "run.sh: unknown step $step" >&2; exit 2 ;;
esac
