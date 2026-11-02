#!/usr/bin/env bash
# Build from this checkout; use an available compiler cache unless the caller
# explicitly selected a wrapper (an empty RUSTC_WRAPPER also opts out).
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ -z "${RUSTC_WRAPPER+x}" ]] && command -v sccache >/dev/null 2>&1; then
  export RUSTC_WRAPPER=sccache
  export SCCACHE_CLIENT_SIDE="${SCCACHE_CLIENT_SIDE-1}"
  export SCCACHE_IGNORE_SERVER_IO_ERROR="${SCCACHE_IGNORE_SERVER_IO_ERROR-1}"
fi

exec cargo build "$@"
