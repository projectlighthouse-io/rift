#!/usr/bin/env bash
# Fails if any *-driver package is reachable from `rift` (the app) other than through `registry`,
# or if a library crate breaks the layering rules in docs/STYLE.md.
set -euo pipefail

fail=0

# app -> driver edges are only allowed below registry. --prune registry hides that subtree.
direct=$(cargo tree -p rift -e normal --prefix none --prune registry | grep -E '^[a-z]+-driver ' || true)
if [[ -n "$direct" ]]; then
  echo "error: app reaches driver crates outside registry:" >&2
  echo "$direct" >&2
  fail=1
fi

# Workspace crates each package may depend on (normal deps only).
check() {
  local pkg=$1 allowed=$2
  local deps
  deps=$(cargo tree -p "$pkg" -e normal --depth 1 --prefix none \
    | tail -n +2 | grep ' (/' | awk '{print $1}' | sort -u || true)
  for d in $deps; do
    if [[ " $allowed " != *" $d "* ]]; then
      echo "error: $pkg must not depend on $d" >&2
      fail=1
    fi
  done
}

check types ""
check traits "types"
check dialect "types"
check fdb-ffi ""
for d in postgres mysql sqlite clickhouse redis; do
  check "$d-driver" "types traits dialect"
done
# The FoundationDB client bindings are the driver's private FFI layer.
check foundationdb-driver "types traits dialect fdb-ffi"

exit $fail
