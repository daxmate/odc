#!/usr/bin/env bash
# Backwards-compatible alias for ../build-app.sh.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec "$ROOT/build-app.sh" "$@"
