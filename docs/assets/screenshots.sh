#!/bin/sh
# Regenerates the README screenshots from the Acme example in a sandbox.
# Nothing touches your real home directory or AI tools.
#
#   cargo build && LOADOUT=$PWD/target/debug/lo CHROME=/path/to/chrome-headless-shell \
#     ./docs/assets/screenshots.sh
#
# Needs git, tmux, aha and ImageMagick (`convert`). CHROME is any headless
# Chromium (e.g. Playwright's chrome-headless-shell).
set -eu

here=$(cd "$(dirname "$0")" && pwd)
examples="$here/../../examples"
loadout=${LOADOUT:-lo}
chrome=${CHROME:?set CHROME to a headless Chromium binary}
root=$(mktemp -d "${TMPDIR:-/tmp}/loadout-shots.XXXXXX")
trap 'tmux -L loadout-shots kill-server 2>/dev/null || true; kill "$ui" 2>/dev/null || true' EXIT

# The example repos, reachable as https://git.example.com/acme/<name> so the
# screenshots show realistic URLs.
for dir in "$examples"/*/; do
  name=$(basename "$dir")
  mkdir -p "$root/remotes/$name"
  cp -R "$dir". "$root/remotes/$name/"
  git -C "$root/remotes/$name" init -q -b main
  git -C "$root/remotes/$name" add -A
  git -C "$root/remotes/$name" -c user.name=Example -c user.email=example@example.com \
    -c commit.gpgsign=false commit -q -m example
done
printf '[url "file://%s/remotes/"]\n\tinsteadOf = https://git.example.com/acme/\n' "$root" > "$root/gitconfig"
mkdir -p "$root/home/.claude" "$root/home/.cursor"
export HOME="$root/home" LOADOUT_HOME="$root/home" LOADOUT_CONFIG_DIR="$root/config" \
  LOADOUT_DATA_DIR="$root/data" GIT_CONFIG_GLOBAL="$root/gitconfig" GIT_CONFIG_NOSYSTEM=1 \
  ACME_ORG=eng ACME_TEAM=payments-dev ACME_ROLE=developer

"$loadout" init https://git.example.com/acme/acme-config --non-interactive >/dev/null
"$loadout" join product:billing >/dev/null
"$loadout" sync >/dev/null

# Web UI.
"$loadout" ui --no-open --port 0 >"$root/ui.log" 2>&1 &
ui=$!
sleep 1
url=$(grep -o 'http://[^ ]*' "$root/ui.log")
shot() { "$chrome" --headless --hide-scrollbars --window-size="$2" --virtual-time-budget=10000 \
  --screenshot="$root/$1.png" "$3" 2>/dev/null; }
shot catalog 1400,900 "$url#Catalog"
shot start 1400,900 "$url#Start"
shot layers 1400,1000 "$url#Layers"
convert "$root/catalog.png" -strip "$here/ui-catalog.png"
convert "$root/start.png" -strip "$here/ui-start.png"
convert "$root/layers.png" -strip "$here/ui-layers.png"

# Terminal UI: the Payments write-spec skill with `why` open.
tmux -L loadout-shots new-session -d -x 120 -y 22 -e TERM=xterm-256color "$loadout tui"
sleep 2
for _ in 1 2 3 4 5 6 7 8 9 10 11; do tmux -L loadout-shots send-keys Down; done
tmux -L loadout-shots send-keys w
sleep 1
tmux -L loadout-shots capture-pane -e -p | aha --black --no-header >"$root/tui.body"
{
  printf '<!doctype html><meta charset=utf-8><style>html,body{margin:0;background:#16181d}'
  printf 'pre{margin:0;padding:14px 16px;font:15px/1.0 "DejaVu Sans Mono",monospace;color:#d8dee9}</style><pre>'
  cat "$root/tui.body"
  printf '</pre>'
} >"$root/tui.html"
shot tui 1130,395 "file://$root/tui.html"
convert "$root/tui.png" -crop 1130x355+0+0 +repage -strip "$here/tui.png"
echo "Updated $here/{ui-catalog,ui-start,ui-layers,tui}.png"
