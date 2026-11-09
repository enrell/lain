#!/usr/bin/env python3
"""Scores V-5 predictions against the gold labels and prints ONLY aggregates.

    score.py <dir> <set> [<set> ...]   # reads <set>.gold.jsonl + <set>.pred.jsonl

Metric (docs/benchmarks/parser-v5.md):
- Values are compared after normalization: titles NFKC + casefold with every
  run of non-alphanumeric characters as one space; groups casefold + trim;
  episodes as a set of integers; resolution lower-case; year and season as
  integers. A missing field equals a missing field.
- exact record (scope "full"): every schema field equal, and a gold
  abstention (no title) is correct only when the parser emits nothing.
- identity: title, season and episodes equal (positive rows).
- per-field accuracy: share of positive rows whose field equals gold
  (both absent counts as equal); a parser that emits nothing gets every
  stated field wrong.
- false emission: share of gold abstentions where the parser emits a title.
- abstention: share of positive rows where the parser emits nothing.
- agreement (any set): share of rows where at least two parsers agree on the
  normalized title, and each parser's agreement with that majority.
Intervals are 95% Wilson score intervals. No row-level output, by design.
"""

import json
import math
import pathlib
import re
import statistics
import sys
import unicodedata

PARSERS = ("tokenizer", "zantetsu", "model", "binding")
FIELDS = ("title", "episode_title", "year", "season", "episodes", "release_group", "resolution")


def norm_text(s):
    if not s:
        return None
    s = unicodedata.normalize("NFKC", str(s)).casefold()
    s = "".join(c if c.isalnum() else " " for c in s)
    s = re.sub(r"\s+", " ", s).strip()
    return s or None


def norm(field, v):
    if v is None or v == [] or v == "":
        return None
    if field in ("title", "episode_title"):
        return norm_text(v)
    if field == "release_group":
        return str(v).strip().casefold() or None
    if field == "episodes":
        try:
            return frozenset(int(x) for x in v)
        except (TypeError, ValueError):
            return None
    if field == "resolution":
        return str(v).lower()
    try:
        return int(v)
    except (TypeError, ValueError):
        return None


def wilson(k, n):
    if n == 0:
        return (0.0, 0.0)
    z, p = 1.96, k / n
    d = 1 + z * z / n
    c = (p + z * z / (2 * n)) / d
    h = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return (max(0.0, c - h), min(1.0, c + h))


def pct(k, n):
    if n == 0:
        return "n/a"
    lo, hi = wilson(k, n)
    return f"{100 * k / n:.1f}% ({k}/{n}; {100 * lo:.1f}–{100 * hi:.1f})"


def load(p):
    return {r["id"]: r for r in (json.loads(x) for x in p.read_text(encoding="utf-8").splitlines() if x.strip())}


def score(d, name):
    gold = load(d / f"{name}.gold.jsonl")
    pred = load(d / f"{name}.pred.jsonl")
    ids = [i for i in gold if i in pred]
    parsers = [p for p in PARSERS if any(p in pred[i] for i in ids)]
    scope = gold[ids[0]]["scope"]
    fields = FIELDS if scope == "full" else ("title", "episodes")
    unlabeled = scope == "unlabeled"
    pos = [] if unlabeled else [i for i in ids if gold[i]["gold"] is not None]
    neg = [] if unlabeled else [i for i in ids if gold[i]["gold"] is None]
    if unlabeled:
        out = [f"### {name}", "", f"{len(ids)} names, no labels: agreement only.", ""]
    else:
        out = [f"### {name}", "", f"{len(ids)} names: {len(pos)} with a title, {len(neg)} gold abstentions; "
               f"scored fields: {', '.join(fields)}.", ""]
    head = "| metric | " + " | ".join(parsers) + " |"
    out += [head, "|---" * (len(parsers) + 1) + "|"]

    def row(label, fn):
        out.append(f"| {label} | " + " | ".join(fn(p) for p in parsers) + " |")

    def eq(p, i, f):
        g = gold[i]["gold"]
        r = pred[i].get(p)
        gv = norm(f, g.get(f)) if g else None
        rv = norm(f, r.get(f)) if r else None
        return gv == rv

    def emits(p, i):
        r = pred[i].get(p)
        return bool(r) and norm("title", r.get("title")) is not None

    if unlabeled:
        row("emits a title", lambda p: pct(sum(1 for i in ids if emits(p, i)), len(ids)))
        fields = ()
    elif scope == "full":
        def exact(p):
            k = sum(1 for i in pos if emits(p, i) and all(eq(p, i, f) for f in fields))
            k += sum(1 for i in neg if not emits(p, i))
            return pct(k, len(ids))
        row("**exact record**", exact)
        row("identity (title+season+episodes)",
            lambda p: pct(sum(1 for i in pos if emits(p, i) and all(eq(p, i, f) for f in ("title", "season", "episodes"))), len(pos)))
    else:
        row("**exact (title+episodes)**",
            lambda p: pct(sum(1 for i in pos if emits(p, i) and all(eq(p, i, f) for f in fields)), len(pos)))
    for f in fields:
        if scope == "full" or f in fields:
            stated = [i for i in pos if norm(f, gold[i]["gold"].get(f)) is not None]
            row(f"{f} (all / where stated)", lambda p, f=f, stated=stated: (
                f"{100 * sum(1 for i in pos if emits(p, i) and eq(p, i, f)) / max(1, len(pos)):.1f}% / "
                f"{pct(sum(1 for i in stated if emits(p, i) and eq(p, i, f)), len(stated))}"))
    if scope == "title+episodes":
        def lenient(p):
            k = 0
            for i in pos:
                g = norm("title", gold[i]["gold"]["title"])
                r = norm("title", (pred[i].get(p) or {}).get("title"))
                if g and r and (g in r or r in g):
                    k += 1
            return pct(k, len(pos))
        row("title lenient (one contains the other)", lenient)
    if pos:
        row("abstention on positive rows", lambda p: pct(sum(1 for i in pos if not emits(p, i)), len(pos)))
    if neg:
        row("false emission on gold abstentions", lambda p: pct(sum(1 for i in neg if emits(p, i)), len(neg)))

    voters = [p for p in ("tokenizer", "zantetsu", "binding") if p in parsers]
    maj = {}
    for i in ids:
        titles = [norm("title", (pred[i].get(p) or {}).get("title")) for p in voters]
        best = max(set(titles), key=titles.count)
        if titles.count(best) >= 2:
            maj[i] = best
    row("agrees with the 2-of-3 title majority",
        lambda p: pct(sum(1 for i, t in maj.items() if norm("title", (pred[i].get(p) or {}).get("title")) == t), len(maj)))
    if maj and pos:
        k = sum(1 for i, t in maj.items() if gold[i]["gold"] and norm("title", gold[i]["gold"]["title"]) == t)
        out += ["", f"Majority vote (tokenizer, zantetsu, binding) exists on {pct(len(maj), len(ids))} of names; "
                f"where it exists it matches the gold title on {pct(k, len(maj))}."]

    lat = []
    for p in parsers:
        key = "model" if p == "binding" else p
        xs = sorted(pred[i]["latency_ms"].get(key, 0) for i in ids if key in pred[i].get("latency_ms", {}))
        if xs:
            p95 = xs[min(len(xs) - 1, int(0.95 * len(xs)))]
            lat.append(f"{p} p50 {statistics.median(xs):.2f} ms / p95 {p95:.2f} ms")
    status = {}
    for i in ids:
        s = pred[i].get("model_status")
        if s:
            status[s] = status.get(s, 0) + 1
    out += ["", "Latency per name: " + "; ".join(lat) + ".", ""]
    if status:
        out += ["Model answers: " + ", ".join(f"{k} {v}" for k, v in sorted(status.items())) + ".", ""]
    return "\n".join(out)


def main():
    d = pathlib.Path(sys.argv[1])
    for s in sys.argv[2:]:
        print(score(d, s))


if __name__ == "__main__":
    main()
