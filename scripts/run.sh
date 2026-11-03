#!/usr/bin/env bash
# Supply benchmark defaults before the server captures its environment.
set -euo pipefail

usage() {
    echo 'Usage: scripts/run.sh <claviron-other|claviron-axum|claviron-warp|claviron-routable> [SERVER_ARGS...]'
    echo 'BENCH_HTTP_EXECUTION: shared (default) or current_thread'
}

case "${1-}" in
    -h|--help) usage; exit 0 ;;
    claviron-other|claviron-axum|claviron-warp|claviron-routable) binary="$1"; shift ;;
    *) usage >&2; exit 2 ;;
esac

export BENCH_HTTP_EXECUTION="${BENCH_HTTP_EXECUTION-shared}"
case "$BENCH_HTTP_EXECUTION" in
    shared|current_thread) ;;
    *) echo 'BENCH_HTTP_EXECUTION must be shared or current_thread' >&2; exit 2 ;;
esac

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd -- "$project_dir"
target_dir="${CARGO_TARGET_DIR:-target}"
exec "$target_dir/release/$binary" "$@"
