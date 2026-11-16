#!/usr/bin/env bash
# Workspace gate: fmt, clippy, tests, and dependency-pin guards.
# The caller must have CARGO_TARGET_DIR set (e.g. via targets/op-registry.env).
set -euo pipefail
cd "$(dirname "$0")/.."

if command -v cargo-slot >/dev/null 2>&1; then
  cargo_cmd=(cargo-slot cargo)
else
  cargo_cmd=(cargo)
fi

pin='rev = "ffeaf3949230a8883b56c75e7031346a4f28ed4d"'
n=$(grep -c "$pin" Cargo.toml || true)
if [ "$n" != "1" ]; then
  echo "guard failed: expected exactly 1 coded-error pin in Cargo.toml, found $n" >&2
  exit 1
fi
n=$(grep -c 'name = "coded-error"' Cargo.lock || true)
if [ "$n" != "1" ]; then
  echo "guard failed: expected exactly 1 coded-error entry in Cargo.lock, found $n" >&2
  exit 1
fi

# T10 guard: tokio's `rt` feature must only come in through op-listen's `detach`.
if "${cargo_cmd[@]}" tree -p op-listen --no-default-features -e normal,features -i tokio | grep -q 'tokio feature "rt"'; then
  echo "guard failed: tokio feature \"rt\" enabled without op-listen's detach feature" >&2
  exit 1
fi

"${cargo_cmd[@]}" fmt --check
"${cargo_cmd[@]}" clippy --workspace --all-targets --all-features -- -D warnings
"${cargo_cmd[@]}" test --workspace --all-features
"${cargo_cmd[@]}" check -p op-listen --no-default-features
"${cargo_cmd[@]}" test -p op-listen --no-default-features
echo "check: ok"
