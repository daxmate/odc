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

echo
echo "== mdx: synthetic fixture (version 2) =="
FIXTURE="$WORK/fixture.mdx"
cargo run --quiet -p odc-testkit --bin gen-mdx -- 2 "$FIXTURE" | tee "$WORK/mdx-fixture.txt"

MDXREPO="$WORK/mdx-repo"
"$ODC" --repo "$MDXREPO" init
"$ODC" --repo "$MDXREPO" import "$FIXTURE" --format mdx | tee "$WORK/mdx-import.txt"
"$ODC" --repo "$MDXREPO" commit -m "mdx import" --author e2e --timestamp 1700000200 | tee "$WORK/mdx-commit.txt"
"$ODC" --repo "$MDXREPO" lookup bravo --json | tee "$WORK/mdx-lookup.json"
"$ODC" --repo "$MDXREPO" search --prefix bra | tee "$WORK/mdx-search.txt"
"$ODC" --repo "$MDXREPO" export --format jsonl -o "$WORK/mdx-export.jsonl"
"$ODC" --repo "$MDXREPO" verify | tee "$WORK/mdx-verify.txt"

echo "== mdx assertions =="
grep -q "working set = 4" "$WORK/mdx-import.txt" || { echo "FAIL: mdx import count"; exit 1; }
echo "ok: mdx import found 4 entries"
grep -q "4 entries, 1 new blocks" "$WORK/mdx-commit.txt" || { echo "FAIL: mdx commit index"; exit 1; }
echo "ok: mdx commit indexed 4 entries"
grep -q '"word": "bravo"' "$WORK/mdx-lookup.json" || { echo "FAIL: mdx lookup"; exit 1; }
echo "ok: mdx lookup returned the normalised key"
grep -q "^bravo" "$WORK/mdx-search.txt" || { echo "FAIL: mdx prefix search"; exit 1; }
echo "ok: mdx prefix search"
[ "$(wc -l < "$WORK/mdx-export.jsonl" | tr -d ' ')" -eq 4 ] || { echo "FAIL: mdx export line count"; exit 1; }
echo "ok: mdx export wrote 4 jsonl lines"
grep -q "^verify: OK" "$WORK/mdx-verify.txt" || { echo "FAIL: mdx verify"; exit 1; }
echo "ok: mdx verify passed"

set +e
"$ODC" --repo "$MDXREPO" lookup missing >/dev/null 2>&1
STATUS=$?
set -e
if [ "$STATUS" -ne 4 ]; then
  echo "FAIL: lookup of a missing word exited $STATUS (expected 4)"
  exit 1
fi
echo "ok: lookup of a missing word exits 4"

echo "== patch roundtrip =="
hash_of() {
  "$ODC" --repo "$1" log --json | awk -v msg="$2" '/"hash":/ { h = $2 } /"message":/ { if (index($0, "\"" msg "\"") > 0) { gsub(/[\",]/, "", h); print h; exit } }'
}
PATCH_BASE=$(hash_of "$REPO" "initial 10k entries")
PATCH_TARGET=$(hash_of "$REPO" "edit one, add one, remove one")
if [ -z "$PATCH_BASE" ] || [ -z "$PATCH_TARGET" ]; then
  echo "FAIL: could not resolve the patch revisions"
  exit 1
fi
"$ODC" --repo "$REPO" patch create "$PATCH_BASE" "$PATCH_TARGET" -o "$WORK/fix.odp" | tee "$WORK/patch-create.txt"
grep -q "target: $PATCH_TARGET" "$WORK/patch-create.txt" || { echo "FAIL: patch create target"; exit 1; }
MIRROR="$WORK/mirror"
"$ODC" --repo "$MIRROR" init >/dev/null
"$ODC" --repo "$MIRROR" import "$DATA/initial.jsonl" --format jsonl >/dev/null
"$ODC" --repo "$MIRROR" commit -m "initial 10k entries" --author e2e --timestamp 1700000000 >/dev/null
MIRROR_BASE=$(hash_of "$MIRROR" "initial 10k entries")
if [ "$MIRROR_BASE" != "$PATCH_BASE" ]; then
  echo "FAIL: mirror base $MIRROR_BASE != $PATCH_BASE"
  exit 1
fi
"$ODC" --repo "$MIRROR" patch verify "$WORK/fix.odp" | tee "$WORK/patch-verify.txt"
grep -q "ok: base $PATCH_BASE" "$WORK/patch-verify.txt" || { echo "FAIL: patch verify"; exit 1; }
"$ODC" --repo "$MIRROR" patch apply "$WORK/fix.odp" | tee "$WORK/patch-apply.txt"
MIRROR_HEAD=$("$ODC" --repo "$MIRROR" log --json | awk '/"hash":/ { h = $2; gsub(/[\",]/, "", h); print h; exit }')
if [ "$MIRROR_HEAD" != "$PATCH_TARGET" ]; then
  echo "FAIL: mirror HEAD $MIRROR_HEAD != $PATCH_TARGET"
  exit 1
fi
"$ODC" --repo "$MIRROR" verify >/dev/null
"$ODC" --repo "$MIRROR" status | grep -q "working set clean" || { echo "FAIL: patch apply left a dirty working set"; exit 1; }
echo "ok: patch roundtrip produced the target commit with a clean working set"
set +e
"$ODC" --repo "$MIRROR" patch apply "$WORK/fix.odp" >/dev/null 2>&1
REPLAY=$?
set -e
if [ "$REPLAY" -ne 4 ]; then
  echo "FAIL: replaying a patch exited $REPLAY (expected 4)"
  exit 1
fi
echo "ok: replaying a patch exits 4"

echo "== merge: fork, conflict, resolve, continue =="
MERGE="$WORK/merge"
FORK="$WORK/merge-fork"
"$ODC" --repo "$MERGE" init >/dev/null
cat > "$WORK/m-base.jsonl" <<'JSONL'
{"word":"alpha","display":"Alpha","defs":[{"pos":"noun","sense":"base alpha"}]}
{"word":"beta","display":"Beta","defs":[{"pos":"noun","sense":"base beta"}]}
JSONL
"$ODC" --repo "$MERGE" import "$WORK/m-base.jsonl" >/dev/null
"$ODC" --repo "$MERGE" commit -m base --timestamp 1700000300 >/dev/null
python3 - "$MERGE/odc.db" "$FORK/odc.db" <<'PY'
import os, sqlite3, sys
source, target = sys.argv[1], sys.argv[2]
os.makedirs(os.path.dirname(target), exist_ok=True)
connection = sqlite3.connect(source)
connection.execute("VACUUM INTO ?", (target,))
connection.close()
PY
cat > "$WORK/m-ours.jsonl" <<'JSONL'
{"word":"alpha","display":"Alpha","defs":[{"pos":"noun","sense":"ours alpha"}]}
{"word":"beta","display":"Beta","defs":[{"pos":"noun","sense":"base beta"}]}
JSONL
"$ODC" --repo "$MERGE" import "$WORK/m-ours.jsonl" >/dev/null
"$ODC" --repo "$MERGE" commit -m ours --timestamp 1700000310 >/dev/null
cat > "$WORK/m-theirs.jsonl" <<'JSONL'
{"word":"alpha","display":"Alpha","defs":[{"pos":"noun","sense":"theirs alpha"}]}
{"word":"beta","display":"Beta","defs":[{"pos":"noun","sense":"base beta"}]}
JSONL
"$ODC" --repo "$FORK" import "$WORK/m-theirs.jsonl" >/dev/null
"$ODC" --repo "$FORK" commit -m theirs --timestamp 1700000320 >/dev/null
THEIRS=$("$ODC" --repo "$FORK" log --json | awk '/"hash":/ { print $2; exit }' | tr -d '",')
python3 - "$MERGE/odc.db" "$FORK/odc.db" <<'PY'
import sqlite3, sys
target, source = sys.argv[1], sys.argv[2]
connection = sqlite3.connect(target)
connection.execute("ATTACH DATABASE ? AS fork", (source,))
for table in ("commits", "commit_index", "blocks"):
    connection.execute(
        "INSERT OR IGNORE INTO main." + table + " SELECT * FROM fork." + table
    )
connection.commit()
connection.execute("DETACH DATABASE fork")
connection.execute("PRAGMA wal_checkpoint(TRUNCATE)")
connection.close()
PY
HEAD_BEFORE=$("$ODC" --repo "$MERGE" log --json | awk '/"hash":/ { print $2; exit }' | tr -d '",')
set +e
"$ODC" --repo "$MERGE" merge HEAD "$THEIRS" --json > "$WORK/merge-conflict.json"
CONFLICT=$?
set -e
if [ "$CONFLICT" -ne 5 ]; then
  echo "FAIL: merge with conflicts exited $CONFLICT (expected 5)"
  exit 1
fi
grep -q '"word": "alpha"' "$WORK/merge-conflict.json" || { echo "FAIL: conflict word"; exit 1; }
grep -q '"defs"' "$WORK/merge-conflict.json" || { echo "FAIL: conflict fields"; exit 1; }
HEAD_AFTER=$("$ODC" --repo "$MERGE" log --json | awk '/"hash":/ { print $2; exit }' | tr -d '",')
if [ "$HEAD_BEFORE" != "$HEAD_AFTER" ]; then
  echo "FAIL: conflicted merge moved HEAD"
  exit 1
fi
echo "ok: conflicted merge exits 5 and leaves HEAD unchanged"
"$ODC" --repo "$MERGE" conflicts --json | tee "$WORK/merge-conflicts.json" >/dev/null
grep -q '"resolution": null' "$WORK/merge-conflicts.json" || { echo "FAIL: conflict listing"; exit 1; }
"$ODC" --repo "$MERGE" resolve alpha --ours
"$ODC" --repo "$MERGE" merge --continue --json | tee "$WORK/merge-done.json"
grep -q '"kind": "merged"' "$WORK/merge-done.json" || { echo "FAIL: merge --continue"; exit 1; }
"$ODC" --repo "$MERGE" lookup alpha | grep -q "ours alpha" || { echo "FAIL: resolved payload"; exit 1; }
"$ODC" --repo "$MERGE" status | grep -q "working set clean" || { echo "FAIL: merge status"; exit 1; }
"$ODC" --repo "$MERGE" verify | grep -q "verify: OK" || { echo "FAIL: merge verify"; exit 1; }
echo "ok: merge conflict resolved, merge commit verified"

echo "e2e: PASS (workdir $WORK)"
