#!/bin/sh
# Try the Acme example in a throwaway sandbox: nothing touches your real
# home directory or AI tools.
#
#   ./examples/try.sh            # a Payments developer
#   ACME_ROLE=product-manager ACME_ORG= ACME_TEAM= ./examples/try.sh
#
# Needs `git` and a built `lo` on PATH (or LOADOUT=/path/to/lo).
set -eu

here=$(cd "$(dirname "$0")" && pwd)
loadout=${LOADOUT:-lo}
sandbox=$(mktemp -d "${TMPDIR:-/tmp}/loadout-example.XXXXXX")
echo "Sandbox: $sandbox"

# Each example directory becomes a local Git repo.
for dir in "$here"/*/; do
  name=$(basename "$dir")
  mkdir -p "$sandbox/remotes/$name"
  cp -R "$dir". "$sandbox/remotes/$name/"
done
# Point the company config's example.com URLs at the local repos.
sed -i.orig "s#https://git.example.com/acme/#$sandbox/remotes/#g" \
  "$sandbox/remotes/acme-config/LOADOUT.md"
rm "$sandbox/remotes/acme-config/LOADOUT.md.orig"
for repo in "$sandbox"/remotes/*/; do
  git -C "$repo" init -q -b main
  git -C "$repo" add -A
  git -C "$repo" -c user.name=Example -c user.email=example@example.com \
    -c commit.gpgsign=false commit -q -m example
done

# A fake home with Claude Code "installed".
mkdir -p "$sandbox/home/.claude"
export LOADOUT_HOME="$sandbox/home"
export LOADOUT_CONFIG_DIR="$sandbox/config"
export LOADOUT_DATA_DIR="$sandbox/data"
export ACME_ORG="${ACME_ORG-eng}"
export ACME_TEAM="${ACME_TEAM-payments-dev}"
export ACME_ROLE="${ACME_ROLE-developer}"

"$loadout" init "$sandbox/remotes/acme-config" --non-interactive
echo
"$loadout" list
echo
"$loadout" why skill/write-spec
echo
echo "Installed skills:"
ls "$sandbox/home/.claude/skills"
echo
echo "Try more (same environment):"
echo "  export LOADOUT_HOME=$LOADOUT_HOME LOADOUT_CONFIG_DIR=$LOADOUT_CONFIG_DIR LOADOUT_DATA_DIR=$LOADOUT_DATA_DIR"
echo "  $lo join product:billing"
echo "  $lo enable eng-skills:skill/incident-runbook"
echo "  $lo secrets check"
echo "  cat $sandbox/home/.claude.json"
