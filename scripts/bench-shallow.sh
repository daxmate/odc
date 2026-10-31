#!/usr/bin/env bash
# Distribution-size baseline for `odc pack`.
#
# Builds a 100,000-entry repository with several commits, then compares:
#   * the source repository directory,
#   * a fresh single-commit repository with the same HEAD,
#   * a full single-file pack,
#   * a shallow single-file pack (HEAD only).
#
# Everything runs on synthetic data in a scratch directory. The exit status is
# non-zero when the acceptance thresholds are not met, and the measured numbers
# are always printed.
#
# Environment overrides: ODC_BENCH_SHALLOW_DIR, ODC_BENCH_SHALLOW_COUNT,
# ODC_BENCH_SHALLOW_BATCHES.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

WORK="${ODC_BENCH_SHALLOW_DIR:-/tmp/odc-bench-shallow}"
COUNT="${ODC_BENCH_SHALLOW_COUNT:-100000}"
BATCHES="${ODC_BENCH_SHALLOW_BATCHES:-10}"
HISTORY="$WORK/history"
FRESH="$WORK/fresh"
FULL_PACK="$WORK/full.odc"
SHALLOW_PACK="$WORK/shallow.odc"
ODC="$ROOT/target/release/odc"
GEN="$ROOT/target/release/gen-entries"

mkdir -p "$WORK"

echo "== build =="
cargo build --quiet --release --bin odc
cargo build --quiet --release -p odc-testkit --bin gen-entries

path_bytes() {
  if stat -f%z "$1" >/dev/null 2>&1; then
    stat -f%z "$1"
  else
    stat -c%s "$1"
  fi
}

dir_bytes() {
  local total=0 file
  while IFS= read -r -d '' file; do
    total=$((total + $(path_bytes "$file")))
  done < <(find "$1" -type f -print0)
  printf '%s' "$total"
}

human() {
  awk -v bytes="$1" 'BEGIN {
    split("B KiB MiB GiB", units, " ");
    value = bytes; idx = 1;
    while (value >= 1024 && idx < 4) { value /= 1024; idx++ }
    printf "%.1f %s", value, units[idx]
  }'
}

BATCH_COUNT=$((COUNT / BATCHES))
if [ "$BATCH_COUNT" -lt 1 ]; then
  echo "count must be at least the number of batches" >&2
  exit 2
fi

rm -rf "$HISTORY" "$FRESH"
mkdir -p "$HISTORY" "$FRESH"

echo "== import $COUNT entries over $BATCHES commits =="
"$ODC" --repo "$HISTORY" init >/dev/null
BATCH_FILES=()
for ((index = 0; index < BATCHES; index++)); do
  start=$((index * BATCH_COUNT))
  batch="$WORK/batch-$index.jsonl"
  "$GEN" --count "$BATCH_COUNT" --start "$start" --out "$batch"
  BATCH_FILES+=("$batch")
  "$ODC" --repo "$HISTORY" import "$batch" >/dev/null
  "$ODC" --repo "$HISTORY" commit -m "batch $((index + 1))" >/dev/null
done

echo "== fresh repository with the same HEAD =="
"$ODC" --repo "$FRESH" init >/dev/null
for batch in "${BATCH_FILES[@]}"; do
  "$ODC" --repo "$FRESH" import "$batch" >/dev/null
done
"$ODC" --repo "$FRESH" commit -m "single commit" >/dev/null

echo "== pack =="
rm -f "$FULL_PACK" "$SHALLOW_PACK" "$FULL_PACK-wal" "$FULL_PACK-shm" \
      "$SHALLOW_PACK-wal" "$SHALLOW_PACK-shm"
"$ODC" --repo "$HISTORY" pack -o "$FULL_PACK"
"$ODC" --repo "$HISTORY" pack --shallow -o "$SHALLOW_PACK"

HISTORY_BYTES=$(dir_bytes "$HISTORY")
FRESH_BYTES=$(dir_bytes "$FRESH")
FULL_BYTES=$(path_bytes "$FULL_PACK")
SHALLOW_BYTES=$(path_bytes "$SHALLOW_PACK")

SHALLOW_OVER_FULL=$(awk -v shallow="$SHALLOW_BYTES" -v full="$FULL_BYTES" \
  'BEGIN { if (full == 0) print "0"; else printf "%.3f", shallow / full }')
FULL_OVER_SHALLOW=$(awk -v shallow="$SHALLOW_BYTES" -v full="$FULL_BYTES" \
  'BEGIN { if (shallow == 0) print "0"; else printf "%.3f", full / shallow }')
SHALLOW_OVER_FRESH=$(awk -v shallow="$SHALLOW_BYTES" -v fresh="$FRESH_BYTES" \
  'BEGIN { if (fresh == 0) print "0"; else printf "%.3f", shallow / fresh }')

echo
printf '%-28s %14s %12s\n' "artifact" "bytes" "human"
printf '%-28s %14s %12s\n' "----------------------------" "--------------" "------------"
printf '%-28s %14s %12s\n' "history repo directory" "$HISTORY_BYTES" "$(human "$HISTORY_BYTES")"
printf '%-28s %14s %12s\n' "fresh HEAD repo directory" "$FRESH_BYTES" "$(human "$FRESH_BYTES")"
printf '%-28s %14s %12s\n' "full pack" "$FULL_BYTES" "$(human "$FULL_BYTES")"
printf '%-28s %14s %12s\n' "shallow pack" "$SHALLOW_BYTES" "$(human "$SHALLOW_BYTES")"
echo
printf 'shallow / full       %s\n' "$SHALLOW_OVER_FULL"
printf 'full / shallow       %s\n' "$FULL_OVER_SHALLOW"
printf 'shallow / fresh HEAD %s\n' "$SHALLOW_OVER_FRESH"

status=0
if awk -v ratio="$SHALLOW_OVER_FRESH" 'BEGIN { exit !(ratio <= 1.10) }'; then
  echo "PASS shallow <= 1.10 x fresh HEAD repository"
else
  echo "FAIL shallow > 1.10 x fresh HEAD repository"
  status=1
fi
if awk -v ratio="$FULL_OVER_SHALLOW" 'BEGIN { exit !(ratio >= 1.50) }'; then
  echo "PASS full >= 1.50 x shallow"
else
  echo "FAIL full < 1.50 x shallow"
  status=1
fi

exit "$status"
