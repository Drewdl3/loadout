#!/bin/sh
# The onboarding story from docs/onboarding.md, end to end, in a throwaway
# sandbox: nothing touches your real home directory or AI tools.
#
#   cargo build && LOADOUT=$PWD/target/debug/lo ./examples/onboard.sh
#
# 1. Sam (Checkout squad lead) creates the squad's catalog on top of the
#    Payments team's, turns Engineering's PR template into the squad's own
#    skill, and registers the squad in the Acme company config.
# 2. Alex (a new developer) runs `lo init`, joins the squad, and adds a
#    personal layer.
# 3. Sam updates the squad's skill; Alex reviews and approves it.
#
# Needs `git`. Local repos stand in for your Git host; each "commit" below
# would be a push and a reviewed pull request in real life.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
loadout=${LOADOUT:-lo}
sandbox=$(mktemp -d "${TMPDIR:-/tmp}/loadout-onboarding.XXXXXX")
remotes="$sandbox/remotes"
step() { printf '\n\033[1;36m== %s\033[0m\n' "$*"; }
run() { printf '\033[2m$ loadout %s\033[0m\n' "$*"; "$loadout" "$@"; }
commit() { git -C "$1" add -A && git -C "$1" -c commit.gpgsign=false commit -q -m "$2"; }

export GIT_AUTHOR_NAME=Example GIT_AUTHOR_EMAIL=example@example.com
export GIT_COMMITTER_NAME=Example GIT_COMMITTER_EMAIL=example@example.com
export ACME_ORG=eng ACME_TEAM=payments-dev ACME_ROLE=developer

# Acme's existing repos (examples/*), as local Git repos.
for dir in "$here"/*/; do
  name=$(basename "$dir")
  mkdir -p "$remotes/$name"
  cp -R "$dir". "$remotes/$name/"
done
sed -i.orig "s#https://git.example.com/acme/#$remotes/#g" "$remotes/acme-config/LOADOUT.md"
rm "$remotes/acme-config/LOADOUT.md.orig"
for repo in "$remotes"/*/; do
  git -C "$repo" init -q -b main
  commit "$repo" "Acme example"
done

# Each person gets their own home, config and data directory.
as() {
  export LOADOUT_HOME="$sandbox/$1/home" LOADOUT_CONFIG_DIR="$sandbox/$1/config"
  export LOADOUT_DATA_DIR="$sandbox/$1/data" USER="$1"
  mkdir -p "$LOADOUT_HOME/.claude"
}

step "Sam, the Checkout squad lead, is already set up as a Payments developer"
as sam
run init "$remotes/acme-config" --non-interactive --quiet
run template list

step "Sam creates the squad's catalog, building on the Payments team's"
run init --new-source "$remotes/checkout-squad" --layer squad --group checkout \
  --upstream ../payments-skills --git-init
run template use pr-workflow --dir "$remotes/checkout-squad" --name checkout-prs \
  --set team=Checkout --set reviewers=1
rm -r "$remotes/checkout-squad/skills/example"   # the scaffold's placeholder
commit "$remotes/checkout-squad" "Add our PR workflow from the Engineering template"
run audit --path "$remotes/checkout-squad"

step "Sam registers the squad in the company config (a reviewed PR to the company config repo)"
awk -v src="$remotes/checkout-squad" '
  /^  policy:/ && !done {
    print "    - layer: squad"; print "      name: checkout"
    print "      sources: [\"" src "\"]"; print "      membership: { manual: true }"
    done = 1
  }
  { print }' "$remotes/acme-config/LOADOUT.md" > "$sandbox/LOADOUT.md"
mv "$sandbox/LOADOUT.md" "$remotes/acme-config/LOADOUT.md"
commit "$remotes/acme-config" "Register the Checkout squad"

step "Alex joins Acme: one command discovers org, team and role"
as alex
run init "$remotes/acme-config" --non-interactive
run join squad:checkout       # joining installs the squad's items right away
run list
run why skill/checkout-prs

step "Alex adds a personal layer on top of the squad"
run init --new-source "$sandbox/alex-skills" --layer user \
  --upstream "$remotes/checkout-squad" --subscribe --quiet
mkdir -p "$sandbox/alex-skills/skills/write-spec"
printf -- "---\nname: write-spec\ndescription: Alex's spec format.\n---\nMy way.\n" \
  > "$sandbox/alex-skills/skills/write-spec/SKILL.md"
commit "$sandbox/alex-skills" "My spec format"
run sync --yes --quiet
run why skill/write-spec

step "Sam tightens the squad's rule; Alex reviews it before it lands"
sed -i.orig 's/merging: 1 /merging: 2 /' "$remotes/checkout-squad/skills/checkout-prs/SKILL.md"
rm "$remotes/checkout-squad/skills/checkout-prs/SKILL.md.orig"
commit "$remotes/checkout-squad" "Two approvals from now on"
run sync || true   # exits 2: changes wait for review
run diff checkout-squad || true
run approve checkout-squad

step "Done"
echo "Sandbox: $sandbox"
echo "Explore as Alex:"
echo "  export LOADOUT_HOME=$sandbox/alex/home LOADOUT_CONFIG_DIR=$sandbox/alex/config LOADOUT_DATA_DIR=$sandbox/alex/data"
echo "  $loadout tui      # or: $loadout ui"
