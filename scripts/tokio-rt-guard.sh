#!/usr/bin/env bash
# T10 guard as a function so check-guard-test.sh can drive it with a fake cargo.
# tokio_rt_guard CARGO_CMD...: fails if tokio's `rt` feature is on for op-listen without `detach`.
tokio_rt_guard() {
  local out
  out=$("$@" tree -p op-listen --no-default-features -e normal,features -i tokio) || { echo "guard failed: cargo tree failed" >&2; return 1; }
  if grep -q 'tokio feature "rt"' <<<"$out"; then
    echo "guard failed: tokio feature \"rt\" enabled without op-listen's detach feature" >&2
    return 1
  fi
}
