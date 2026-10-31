# Three-way merge (M4)

Two branches that grew from the same base can be combined without replaying
edits: ODC compares the entry trees of both commits against their merge base
and writes a merge commit only when the result is unambiguous. When it is not,
the divergent words are recorded as conflicts, nothing is committed, and the
resolver decides how to proceed. The wire format and commit formula do not
change: merge commits use the existing `parent2` field.

## Usage

```console
# Merge their branch into HEAD (HEAD must be the first revision).
odc merge HEAD <theirs> [--strategy auto|ours|theirs|union] [--json]

# Inspect and resolve the conflicts of an unfinished merge.
odc conflicts [--json]
odc resolve <word> --ours|--theirs|--union|--file payload.cbor [--json]

# Finish the merge once every conflict is resolved.
odc merge --continue [--json]
```

## Semantics

### Merge base

Distances are computed with a BFS over the `parent` and `parent2` DAG.
Candidates are ranked by increasing `max(distance_ours, distance_theirs)`,
ties are broken by the smaller distance sum and then by the lexicographically
smallest `commit_hash`. The rule is deterministic for every caller, so two
repositories that contain the same commits always pick the same base.

Unrelated histories have no base; they are merged as if the base were an empty
tree, which turns every word that exists on both sides into an add/add
conflict.

### Fast-forward

When their commit already descends from ours, merging is a no-op
(`up-to-date`). When ours descends from theirs, HEAD simply moves to their
commit and the entry index is rebuilt from its snapshot; no merge commit is
written.

### Automatic merge

For every word the merge compares the content hashes of base, ours and theirs:

| base | ours | theirs | result |
| --- | --- | --- | --- |
| b | b | t | theirs |
| b | o | b | ours |
| b | o | t (o = t) | the shared payload |
| b | o | t (all differ) | field-level merge or conflict |
| b | b | none | removed |
| none | o | none | ours (added) |

Fields are the top-level keys of the canonical CBOR payload (`display`,
`phonetic`, `defs`, `raw_html`, `extra`). Each one is compared
through its field hash (BLAKE3 of the field's canonical CBOR). When the sets of
changed fields on both sides are disjoint, the merge takes the changed side per
field, re-encodes the payload and recomputes `content_hash`. Otherwise the
word is a conflict and the differing field names are reported.

### Conflicts

Conflicts are stored in the `conflicts` table instead of a commit. HEAD, the
entry index and the working set stay untouched, and the command exits with
code **5**. The pending merge (base, ours, theirs, strategy) is kept in the
`meta` table under the `merge` key.

While a merge is unfinished, starting another one fails with `ODC-E028`;
`merge --continue` fails with `ODC-E029` until every conflict is resolved;
`resolve` and `merge --continue` without a pending merge fail with `ODC-E030`.

### Resolutions

| Flag | Result |
| --- | --- |
| `--ours` | keep our payload for the word |
| `--theirs` | keep their payload |
| `--union` | keep ours at the key, add theirs under `<key>~N` |
| `--file PATH` | use the canonical CBOR payload in PATH |

### Strategies

Strategies decide the both-changed case before any conflict is recorded:

* `auto` (default) - field-level merge, conflicts otherwise.
* `ours` - keep our payload (deletions on our side win).
* `theirs` - keep their payload.
* `union` - keep both. Our copy keeps the key; their copy is stored under
  the first free `<key>~N` suffix, starting at `~2`. The suffix is applied
  to the already normalised key, is ASCII-only and is therefore stable under
  `normalize_key`. When only one side still has the word (delete/modify),
  `union` keeps the surviving copy.

## Conflict JSON

Both `merge --json` and `conflicts --json` use the same conflict object:

```json
{
  "word": "alpha",
  "fields": ["defs"],
  "base": "<hex>",
  "ours": "<hex>",
  "theirs": "<hex>",
  "resolution": null
}
```

* `fields` lists the payload fields that disagree. An empty list marks a
  whole-entry conflict (add/add or delete/modify).
* `base` is `null` when the word did not exist in the merge base;
  `ours` and `theirs` are `null` when that side deleted the word.
* `resolution` is one of `ours`, `theirs`, `union`, `file` or
  `null`.

The merge report adds the operation outcome:

```json
{
  "kind": "conflicts",
  "strategy": "auto",
  "base": "<hex>",
  "ours": "<hex>",
  "theirs": "<hex>",
  "commit": null,
  "auto_merged": 0,
  "new_blocks": 0,
  "reused_blocks": 0,
  "conflicts": []
}
```

`kind` is one of `up-to-date`, `fast-forward`, `merged` or `conflicts`.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | merged, fast-forward, up to date, or a resolution was recorded |
| 1 | any other error (bad revision, dirty working set, `ODC-E028`-`E030`, ...) |
| 5 | the merge recorded conflicts and did not write a commit |

## Conflict storage

The frozen schema keeps `conflicts(id, word, detail)`. `detail` is canonical CBOR:

```text
{
  "base": bytes|null, "ours": bytes|null, "theirs": bytes|null,
  "fields": [text],
  "base_fields": {text: bytes}, "ours_fields": {text: bytes},
  "theirs_fields": {text: bytes},
  "resolution": text|null, "payload": bytes|null, "union_key": text|null,
  "ts": int
}
```

The field maps carry the per-field hashes that produced the conflict, so an
external tool can audit the decision without re-reading the payloads.

## Patch integration

`odc patch apply` installs the patch's target commit but does not move HEAD.
When HEAD is exactly the base the patch applies as before. When HEAD descends
from the base, the target and HEAD are merged three-way (base = patch base,
ours = HEAD, theirs = target) with the `auto` strategy. A conflicted merge keeps
HEAD, the entry index and the working set untouched, stores the target objects
so `resolve` / `merge --continue` can finish the job, and exits with 5.

## Limits

* A merge keeps an in-memory index of `word -> (content_hash, block location)` for
  the three trees (roughly 100 bytes per word). The written commit itself is
  built from the database, not from that index.
* Blocks belonging only to old revisions can be collected by `odc gc`. Merging
  such a revision fails with `ODC-E009` when a payload can no longer be found;
  run the merge before collecting history.
* Union suffixes are appended to normalised keys and are not re-normalised.
  A word that literally ends in `~N` is treated as an ordinary key.

