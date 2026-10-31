#!/usr/bin/env bash
# Set the version of both plugins (Cargo.toml, plugin.toml and Cargo.lock)
# to JJJJ.W.N: the number given, or this ISO week with the next free N
# after this week's tags. The release workflow refuses a tag that differs.
#
#   scripts/version.sh            # e.g. 2026.44.4
#   scripts/version.sh 2026.45.0
set -euo pipefail
cd "$(dirname "$0")/.."

v="${1:-}"
if [ -z "$v" ]; then
  week="$(date +%G).$(date +%V | sed 's/^0//')"
  # v2026.44 counts as N = 0, v2026.44.3 as 3.
  last=$(git tag -l "v$week" "v$week.*" | sed -E "s/^v$week\.?//; s/^$/0/" | sort -n | tail -1)
  if [ -z "$last" ]; then n=0; else n=$((last + 1)); fi
  v="$week.$n"
fi
[[ "$v" =~ ^[0-9]{4}\.[1-9][0-9]?\.[0-9]+$ ]] || { echo "geen JJJJ.W.N (zonder voorloopnul in de week): $v" >&2; exit 1; }

for f in Cargo.toml civiel/Cargo.toml plugin.toml civiel/plugin.toml; do
  sed -i -E "0,/^version = \"[^\"]+\"/s//version = \"$v\"/" "$f"
done
# The two workspace crates in Cargo.lock.
awk -v v="$v" '/^name = "open-infra(-civiel)?"$/ { print; getline; sub(/"[^"]+"/, "\"" v "\""); print; next } { print }' Cargo.lock > Cargo.lock.tmp
mv Cargo.lock.tmp Cargo.lock
echo "$v"
