#!/usr/bin/env bash
# End-to-end demonstration: init -> import 10k -> commit -> edit -> commit ->
# log -> diff -> verify. Prints every command output and asserts the results.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

WORK="$(mktemp -d "/tmp/odc-e2e-XXXXXX")"
REPO="$WORK/repo"
DATA="$WORK/data"
mkdir -p "$DATA"

echo "== build =="
cargo build --quiet --bin odc
ODC="$ROOT/target/debug/odc"

gen_initial() {
  awk 'BEGIN {
    for (i = 0; i < 10000; i++) {
      printf "{\"word\":\"word-%05d\",\"display\":\"Word %05d\",\"defs\":[{\"pos\":\"noun\",\"sense\":\"definition %05d\"}]}\n", i, i, i
    }
  }'
}

gen_edited() {
  # word-00000 removed, word-00001 modified, zzz-new added.
  awk 'BEGIN {
    for (i = 1; i < 10000; i++) {
      sense = sprintf("definition %05d", i)
      if (i == 1) sense = "UPDATED definition 00001"
      printf "{\"word\":\"word-%05d\",\"display\":\"Word %05d\",\"defs\":[{\"pos\":\"noun\",\"sense\":\"%s\"}]}\n", i, i, sense
    }
    printf "{\"word\":\"zzz-new\",\"display\":\"zzz-new\",\"defs\":[{\"pos\":\"noun\",\"sense\":\"brand new entry\"}]}\n"
  }'
}

gen_initial > "$DATA/initial.jsonl"
gen_edited > "$DATA/edited.jsonl"

echo "== init =="
"$ODC" --repo "$REPO" init | tee "$WORK/init.txt"

echo "== import 10k =="
"$ODC" --repo "$REPO" import "$DATA/initial.jsonl" --format jsonl | tee "$WORK/import1.txt"

echo "== commit #1 =="
"$ODC" --repo "$REPO" commit -m "initial 10k entries" --author e2e --timestamp 1700000000 | tee "$WORK/commit1.txt"

echo "== status after commit #1 =="
"$ODC" --repo "$REPO" status | tee "$WORK/status1.txt"

echo "== import edit (modify 1, add 1, remove 1) =="
"$ODC" --repo "$REPO" import "$DATA/edited.jsonl" --format jsonl --prune | tee "$WORK/import2.txt"

echo "== status before commit #2 =="
"$ODC" --repo "$REPO" status | tee "$WORK/status2.txt"

echo "== commit #2 =="
"$ODC" --repo "$REPO" commit -m "edit one, add one, remove one" --author e2e --timestamp 1700000100 | tee "$WORK/commit2.txt"

echo "== log =="
"$ODC" --repo "$REPO" log | tee "$WORK/log.txt"

echo "== diff HEAD~1 HEAD =="
"$ODC" --repo "$REPO" diff "HEAD~1" "HEAD" | tee "$WORK/diff.txt"

echo "== verify =="
"$ODC" --repo "$REPO" verify | tee "$WORK/verify.txt"

echo "== assertions =="

COMMITS=$("$ODC" --repo "$REPO" log --json | grep -c '"hash"')
GENESIS=$("$ODC" --repo "$REPO" log --json | grep -c '"message": "genesis"')
if [ "$GENESIS" -ne 1 ]; then
  echo "FAIL: expected exactly one genesis commit, found $GENESIS"
  exit 1
fi
if [ $((COMMITS - GENESIS)) -ne 2 ]; then
  echo "FAIL: expected 2 user commits, found $((COMMITS - GENESIS))"
  exit 1
fi
echo "ok: log shows 2 commits (plus the genesis row)"

grep -qx "added: 1" "$WORK/diff.txt" || { echo "FAIL: diff added count"; exit 1; }
grep -qx "removed: 1" "$WORK/diff.txt" || { echo "FAIL: diff removed count"; exit 1; }
grep -qx "modified: 1" "$WORK/diff.txt" || { echo "FAIL: diff modified count"; exit 1; }
echo "ok: diff shows 1 added / 1 removed / 1 modified"

REUSED=$(grep -o '[0-9]* reused blocks' "$WORK/commit2.txt" | grep -o '^[0-9]*')
if [ "${REUSED:-0}" -lt 1 ]; then
  echo "FAIL: expected the second commit to reuse blocks, got ${REUSED:-0}"
  exit 1
fi
echo "ok: second commit reused $REUSED blocks (content-defined chunking works)"

grep -q "^verify: OK" "$WORK/verify.txt" || { echo "FAIL: verify"; exit 1; }
echo "ok: verify passed"

grep -q "10000 entries" "$WORK/commit1.txt" || { echo "FAIL: first commit entry count"; exit 1; }
echo "ok: first commit contains 10000 entries"

echo "e2e: PASS (workdir $WORK)"
