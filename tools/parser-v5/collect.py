#!/usr/bin/env python3
"""Builds the V-5 benchmark sets on this machine (docs/benchmarks/parser-v5.md).

Writes `<out>/<set>.names.jsonl` ({id, name}) and `<out>/<set>.gold.jsonl`
({id, gold, scope}) OUTSIDE the repository (default ~/.cache/lain-bench/
parser-v5). The files hold real names: never commit, paste or log them.

Sets:
- `guessit-holdout`: lain-parser's public GuessIt-fixture holdout
  (filename-parser data/public-guessit-holdout*.jsonl), labelled with the
  full parse_filename schema; an empty answer is a gold abstention.
- `library`: the owner's media library. Every video that has an Anything
  `meta.toml` sidecar is labelled from the scraped page metadata, with no
  manual step: title = `series` when present, else `title`; episodes =
  [`episode`] when `series` is present and `episode` is a number. Only
  those two fields are scored there (scope "title+episodes").
- `tracker-unlabeled`: lain-parser's claim-v1 candidate pool of real tracker
  names, unlabelled; scored by parser agreement only (scope "unlabeled").
"""

import argparse
import hashlib
import json
import os
import pathlib
import re
import sys
import tomllib

VIDEO = {".mkv", ".mp4", ".avi", ".webm", ".m4v", ".wmv", ".flv", ".mov", ".ts", ".rmvb"}
FIELDS = ("title", "episode_title", "year", "season", "episodes", "release_group", "resolution")


def rid(name):
    return hashlib.sha256(name.encode()).hexdigest()[:16]


def guessit(fp_repo):
    rows = []
    for f in ("public-guessit-holdout.jsonl", "public-guessit-holdout-part2.jsonl"):
        p = fp_repo / "data" / f
        for line in p.read_text(encoding="utf-8").splitlines():
            if not line.strip():
                continue
            d = json.loads(line)
            args = d["answers"][0]["arguments"] if d["answers"] else None
            gold = None if args is None else {k: args.get(k) for k in FIELDS}
            rows.append((d["query"], gold, "full"))
    return rows


def unlabeled(fp_repo):
    """Real release names from lain-parser's claim-v1 candidate pool (a
    public anime tracker sample, layout-disjoint from its training data),
    without labels: scored by agreement only."""
    p = fp_repo / "data" / "claim-v1" / "candidates-v1.jsonl"
    rows = []
    for line in p.read_text(encoding="utf-8").splitlines():
        if line.strip():
            rows.append((json.loads(line)["query"], None, "unlabeled"))
    return rows


def local_path(win_or_posix, media_root):
    """`V:\\.media\\x\\y.mp4` (the Windows app's dialect) as a path here."""
    p = win_or_posix.replace("\\", "/")
    m = re.match(r"^[A-Za-z]:/\.media/(.*)$", p)
    if m:
        return media_root / m.group(1)
    return pathlib.Path(p)


def library(media_root):
    rows, seen = [], set()
    for root, _dirs, files in os.walk(media_root):
        if "/previews" in root or "/thumbnails" in root or "/transmux" in root:
            continue
        for f in files:
            if not f.endswith(".toml"):
                continue
            try:
                meta = tomllib.loads(pathlib.Path(root, f).read_text(encoding="utf-8"))
            except (OSError, tomllib.TOMLDecodeError, UnicodeDecodeError):
                continue
            video = meta.get("local", {}).get("video_path") or ""
            if not video:
                continue
            path = local_path(video, media_root)
            if path.suffix.lower() not in VIDEO or not path.exists():
                continue
            name = path.name
            if name in seen:
                continue
            seen.add(name)
            series = (meta.get("series") or "").strip()
            title = series or (meta.get("title") or "").strip()
            if not title:
                continue
            ep = str(meta.get("episode") or "").strip()
            episodes = [int(ep)] if series and ep.isdigit() else None
            rows.append((name, {"title": title, "episodes": episodes}, "title+episodes"))
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=str(pathlib.Path.home() / ".cache/lain-bench/parser-v5"))
    ap.add_argument("--filename-parser", default=str(pathlib.Path.home() / "code/media/filename-parser"))
    ap.add_argument("--media-root", default="/mnt/v/.media",
                    help="the library root (V:\\.media); /mnt/d when V: is not attached")
    args = ap.parse_args()
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    root = pathlib.Path(args.media_root)
    fp = pathlib.Path(args.filename_parser)
    sets = {"guessit-holdout": guessit(fp), "tracker-unlabeled": unlabeled(fp)}
    if root.is_dir():
        sets["library"] = library(root)
    else:
        print(f"{root} is not mounted; skipping the library set", file=sys.stderr)
    for name, rows in sets.items():
        with open(out / f"{name}.names.jsonl", "w", encoding="utf-8") as n, \
             open(out / f"{name}.gold.jsonl", "w", encoding="utf-8") as g:
            written = set()
            for query, gold, scope in rows:
                i = rid(query)
                # The two holdout files share some names: keep the first label.
                if i in written:
                    continue
                written.add(i)
                n.write(json.dumps({"id": i, "name": query}, ensure_ascii=False) + "\n")
                g.write(json.dumps({"id": i, "gold": gold, "scope": scope}, ensure_ascii=False) + "\n")
        print(f"{name}: {len(written)} unique names", file=sys.stderr)


if __name__ == "__main__":
    main()
