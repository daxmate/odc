#!/usr/bin/env bash
# 兼容别名：统一入口已迁到 scripts/build-app.sh。
# 旧调用方式 ./build-app.sh 仍然可用，参数原样透传。
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "${ROOT}/scripts/build-app.sh" "$@"
