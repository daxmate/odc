#!/usr/bin/env bash
# One-million-entry benchmark: generate -> import -> commit -> edit -> commit
# -> diff -> gc -> verify, reporting wall time and peak RSS for every step.
#
# Everything runs in a scratch directory (default /tmp/odc-bench) on synthetic
# data (gen-entries), so no dictionary content is involved.
#
# Environment overrides: ODC_BENCH_DIR, ODC_BENCH_COUNT, ODC_BENCH_CHUNK.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

WORK="${ODC_BENCH_DIR:-/tmp/odc-bench}"
COUNT="${ODC_BENCH_COUNT:-1000000}"
CHUNK="${ODC_BENCH_CHUNK:-100000}"
DATA="$WORK/entries-$COUNT.jsonl"
PATCH="$WORK/patch-$COUNT.jsonl"
REPO="$WORK/repo"
OUT_LOG="$WORK/out.log"

mkdir -p "$WORK"

echo "== build =="
cargo build --quiet --release --bin odc
cargo build --quiet --release -p odc-testkit --bin gen-entries
ODC="$ROOT/target/release/odc"
GEN="$ROOT/target/release/gen-entries"

if [ ! -f "$DATA" ]; then
  echo "== generate $COUNT entries =="
  "$GEN" --count "$COUNT" --out "$DATA"
fi
PATCH_COUNT=$((COUNT / 100))
if [ ! -f "$PATCH" ]; then
  echo "== generate $PATCH_COUNT patched entries =="
  "$GEN" --count "$PATCH_COUNT" --stride 100 --seed 2 --out "$PATCH"
fi

# measure <label> <command...>: run once, print one table row.
measure() {
  local label="$1"
  shift
  local timefile="$WORK/time-$label.log"
  /usr/bin/time -l "$@" >"$OUT_LOG" 2>"$timefile" || {
    echo "FAILED: $label"
    cat "$timefile"
    exit 1
  }
  local rss real rss_mb
  rss=$(awk '/maximum resident set size/ {print $1; exit}' "$timefile")
  real=$(awk '$2 == "real" {print $1; exit}' "$timefile")
  rss_mb=$(awk -v bytes="${rss:-0}" 'BEGIN {printf "%.1f", bytes/1048576}')
  printf '| %s | %s | %s |\n' "$label" "$real" "$rss_mb"
}

echo "== reset repository =="
rm -rf "$REPO"
"$ODC" --repo "$REPO" init > /dev/null

echo "| step | real (s) | peak RSS (MB) |"
echo "|---|---|---|"

rm -f "$WORK"/chunk-*
split -l "$CHUNK" "$DATA" "$WORK/chunk-"
IMPORT_START=$SECONDS
for part in "$WORK"/chunk-*; do
  "$ODC" --repo "$REPO" import "$part" --format jsonl > /dev/null
done
IMPORT_SECONDS=$((SECONDS - IMPORT_START))
printf '| import (%s entries) | %s | (chunked) |\n' "$COUNT" "$IMPORT_SECONDS"
rm -f "$WORK"/chunk-*

measure "commit (1M)" "$ODC" --repo "$REPO" commit -m "one million entries" --author bench --timestamp 1700000001

"$ODC" --repo "$REPO" import "$PATCH" --format jsonl > /dev/null
measure "commit (10k edits)" "$ODC" --repo "$REPO" commit -m "edit ten thousand entries" --author bench --timestamp 1700000002

measure "diff HEAD~1 HEAD --json" "$ODC" --repo "$REPO" diff HEAD~1 HEAD --json
measure "gc --dry-run" "$ODC" --repo "$REPO" gc --keep-last 1 --dry-run
measure "gc --keep-last 1" "$ODC" --repo "$REPO" gc --keep-last 1
measure "verify" "$ODC" --repo "$REPO" verify

echo
cat "$OUT_LOG"
echo
echo "database: $(du -h "$REPO/odc.db" | awk '{print $1}') at $REPO/odc.db"
