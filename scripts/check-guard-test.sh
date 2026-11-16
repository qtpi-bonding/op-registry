#!/usr/bin/env bash
# Self-test for tokio_rt_guard: a fake cargo stands in for the real one.
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/tokio-rt-guard.sh

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fake() { # fake NAME BODY
  printf '#!/usr/bin/env bash\n%s\n' "$2" >"$tmp/$1"
  chmod +x "$tmp/$1"
}
# Enough output that grep -q can exit early and a writer can take SIGPIPE.
fake leaky 'echo "tokio v1 "; echo "|-- tokio feature \"rt\""; yes "padding" | head -n 200000'
fake broken 'echo "error: failed to resolve" >&2; exit 1'
fake clean 'echo "tokio v1"; echo "|-- tokio feature \"net\""'

fail=0
expect() { # expect WANT_RC NAME
  local rc=0
  (set -euo pipefail; tokio_rt_guard "$tmp/$2") 2>/dev/null || rc=$?
  if { [ "$1" = ok ] && [ "$rc" != 0 ]; } || { [ "$1" = fail ] && [ "$rc" = 0 ]; }; then
    echo "check-guard-test: FAIL: $2 expected $1, guard rc=$rc" >&2
    fail=1
  else
    echo "check-guard-test: ok: $2 -> $1"
  fi
}
expect fail leaky   # (a) rt present
expect fail broken  # (b) cargo tree itself fails
expect ok clean     # (c) clean tree
exit "$fail"
