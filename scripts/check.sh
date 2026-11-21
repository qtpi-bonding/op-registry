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

pin='rev = "de21d22e31baff78eae7d4ca817b87430bff1f81"'
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
. scripts/tokio-rt-guard.sh
bash scripts/check-guard-test.sh || exit 1
tokio_rt_guard "${cargo_cmd[@]}" || exit 1

"${cargo_cmd[@]}" fmt --check
"${cargo_cmd[@]}" clippy --workspace --all-targets --all-features -- -D warnings
"${cargo_cmd[@]}" test --workspace --all-features
"${cargo_cmd[@]}" check -p op-listen --no-default-features
"${cargo_cmd[@]}" test -p op-listen --no-default-features
"${cargo_cmd[@]}" test -p op-listen --features testkit
"${cargo_cmd[@]}" check -p op-status
"${cargo_cmd[@]}" check -p op-status --features registry
"${cargo_cmd[@]}" check -p op-status --features listen
"${cargo_cmd[@]}" test -p op-status --all-features
echo "check: ok"
