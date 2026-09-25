#!/bin/sh
# Install Loadout's `lo` from a GitHub release, verified against the
# release's SHA256SUMS, then (optionally) run `lo init` with your
# company config so you pick your groups and AI tools.
#
#   curl -fsSL https://raw.githubusercontent.com/Drewdl3/loadout/main/install.sh | sh
#   curl -fsSL …/install.sh | sh -s -- --company-config https://git.example.com/acme/agent-config
#
# Options: --company-config <url>  run `lo init <url>` afterwards
#          --version <tag>  install this release (default: latest)
#          --dir <dir>      install directory (default: ~/.local/bin)
# Environment: LOADOUT_RELEASES_API (default https://api.github.com/repos/Drewdl3/loadout),
#              GH_TOKEN (optional, for private repos / rate limits).
set -eu

api="${LOADOUT_RELEASES_API:-https://api.github.com/repos/Drewdl3/loadout}"
dir="${HOME}/.local/bin"
version=""
company_config=""
while [ $# -gt 0 ]; do
  case "$1" in
    --company-config) company_config="$2"; shift 2 ;;
    --version) version="$2"; shift 2 ;;
    --dir) dir="$2"; shift 2 ;;
    -h|--help) sed -n '2,16p' "$0"; exit 0 ;;
    *) echo "install.sh: unknown option $1" >&2; exit 2 ;;
  esac
done

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target=x86_64-unknown-linux-musl ;;
  Linux-aarch64|Linux-arm64) target=aarch64-unknown-linux-musl ;;
  Darwin-x86_64) target=x86_64-apple-darwin ;;
  Darwin-arm64) target=aarch64-apple-darwin ;;
  *) echo "install.sh: no lo build for $(uname -s) $(uname -m); try \`cargo install --git https://github.com/Drewdl3/loadout loadout-cli\`" >&2; exit 1 ;;
esac

# GitHub answers 415 to `Accept: application/octet-stream` on the release JSON,
# so the JSON and the assets each get their own media type.
fetch() { # url accept -> stdout
  if [ -n "${GH_TOKEN:-}" ]; then
    curl -fsSL -H "Authorization: Bearer ${GH_TOKEN}" -H "Accept: $2" "$1"
  else
    curl -fsSL -H "Accept: $2" "$1"
  fi
}
octet=application/octet-stream

if [ -n "$version" ]; then rel="$api/releases/tags/$version"; else rel="$api/releases/latest"; fi
json="$(fetch "$rel" application/vnd.github+json)"
# The release JSON lists assets as "name": … and "browser_download_url": ….
urls="$(printf '%s' "$json" | tr ',' '\n' | sed -n 's/.*"browser_download_url": *"\([^"]*\)".*/\1/p')"
asset_url="$(printf '%s\n' "$urls" | grep "/lo-[^/]*-${target}\.tar\.gz\$" | head -n1 || true)"
sums_url="$(printf '%s\n' "$urls" | grep '/SHA256SUMS$' | head -n1 || true)"
[ -n "$asset_url" ] || { echo "install.sh: the release has no build for $target" >&2; exit 1; }
[ -n "$sums_url" ] || { echo "install.sh: the release has no SHA256SUMS" >&2; exit 1; }

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
name="$(basename "$asset_url")"
fetch "$asset_url" "$octet" > "$tmp/$name"
fetch "$sums_url" "$octet" > "$tmp/SHA256SUMS"
expected="$(awk -v n="$name" '{ f=$2; sub(/^\*/, "", f); if (f == n) print $1 }' "$tmp/SHA256SUMS")"
if command -v sha256sum >/dev/null 2>&1; then actual="$(sha256sum "$tmp/$name" | awk '{print $1}')"
else actual="$(shasum -a 256 "$tmp/$name" | awk '{print $1}')"; fi
if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
  echo "install.sh: checksum mismatch for $name; not installing" >&2; exit 1
fi
tar -xzf "$tmp/$name" -C "$tmp"
bin="$(find "$tmp" -type f -name lo | head -n1)"
mkdir -p "$dir"
cp "$bin" "$dir/lo.new" && chmod 755 "$dir/lo.new" && mv "$dir/lo.new" "$dir/lo"
echo "Installed $("$dir/lo" --version) to $dir/lo"
case ":$PATH:" in *":$dir:"*) ;; *) echo "Add $dir to your PATH (e.g. in ~/.profile): export PATH=\"$dir:\$PATH\"" ;; esac

if [ -n "$company_config" ]; then
  # Interactive when a terminal is available: pick groups and detected tools.
  if [ -t 1 ] && [ -r /dev/tty ]; then "$dir/lo" init "$company_config" < /dev/tty
  else "$dir/lo" init "$company_config" --non-interactive; fi
else
  echo "Next: lo init <your company config URL>"
fi
