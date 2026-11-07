#!/usr/bin/env python3
"""Build the OALD9 dialect sample as a raw jsonl (key + source fragment).

The survey drop under /tmp/odc-oald9-samples is one manifest.tsv plus one
HTML file per record.  This script is the reproducible bridge from that drop
to the input format `profile_compare --source` expects, so the OALD9 profile
can be re-measured without re-scraping the dictionary.

    scripts/oald9_raw.py [<sample dir>] [<out jsonl>]
"""

from __future__ import annotations

import csv
import json
import pathlib
import sys


def main(argv: list[str]) -> int:
    src = pathlib.Path(argv[1] if len(argv) > 1 else "/tmp/odc-oald9-samples")
    out = pathlib.Path(argv[2] if len(argv) > 2 else "/tmp/odc-oald9/raw.jsonl")
    manifest = src / "manifest.tsv"
    if not manifest.is_file():
        print(f"missing manifest: {manifest}", file=sys.stderr)
        return 2
    rows = []
    with manifest.open(encoding="utf-8", newline="") as handle:
        for row in csv.DictReader(handle, delimiter="\t"):
            body = (src / row["filename"]).read_text(encoding="utf-8")
            rows.append({"key": row["key"], "text": body})
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, ensure_ascii=False) + "\n")
    aliases = sum(1 for row in rows if row["text"].startswith("@@@LINK="))
    print(f"{len(rows)} records ({aliases} alias) -> {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
