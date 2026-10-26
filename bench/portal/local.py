#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""local.py — every measurement this checkout took, kept per routine and tagged by subject.

The committed history (`portal/results/<host>.tsv`) holds full runs only.  The runs between
them — `stats.py --only 13`, `--routine split`, a hand-priced instruction count — used to live
in scrollback.  Here each one is a row in `bench/portal/local/<bench>/<routine>.tsv`
(git-ignored, kept by the disk janitor), one row per lane per measurement:

    date time commit dirty host subject lane value unit spread ratio source note

`ratio` is against the Rust twin when the run measured it.  The SUBJECT is the arc the
measurement belongs to (`--subject c-store-port` on `stats.py`, or `LOFT_RESULT_SUBJECT`);
`-` when none was given.

    python3 bench/portal/local.py show [FILTER…] [--subject S] [--lane L] [--last N]
    python3 bench/portal/local.py subjects
    python3 bench/portal/local.py import RUN.tsv [--subject S]   # a saved `stats.py --tsv` run
    python3 bench/portal/local.py add BENCH/ROUTINE --lane c-hand --value 4.69e9 \\
        --unit instructions [--subject S] [--note "adopted split, hand-priced"]

A FILTER is a bench (`13_stdlib_text`, `13`), a routine (`split`) or both (`13/split`).
"""
import argparse
import datetime
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
STORE = os.path.join(HERE, "local")
COLS = ["date", "time", "commit", "dirty", "host", "subject", "lane", "value", "unit",
        "spread", "ratio", "source", "note"]


def subject_default():
    return os.environ.get("LOFT_RESULT_SUBJECT", "") or "-"


def clean(v):
    """A field as one TSV cell: no tabs or newlines."""
    return str(v).replace("\t", " ").replace("\n", " ") if v not in (None, "") else "-"


def path_of(bench, routine):
    safe = routine.replace("/", "_")
    return os.path.join(STORE, bench or "_packages", f"{safe}.tsv")


def append(bench, routine, rows):
    """Append `rows` (dicts over COLS) to the routine's file, writing its header first."""
    p = path_of(bench, routine)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    new = not os.path.exists(p)
    with open(p, "a", encoding="utf-8") as f:
        if new:
            f.write("\t".join(COLS) + "\n")
        for r in rows:
            f.write("\t".join(clean(r.get(c)) for c in COLS) + "\n")


def git(*args):
    p = subprocess.run(["git", "-C", ROOT, *args], capture_output=True, text=True)
    return p.stdout.strip()


def stamp_now():
    now = datetime.datetime.now()
    return {
        "date": now.date().isoformat(),
        "time": now.strftime("%H:%M"),
        "commit": git("rev-parse", "--short", "HEAD"),
        "dirty": "yes" if git("status", "--porcelain", "--untracked-files=no") else "no",
    }


def record_stats(out_rows, lanes, meta, subject, source):
    """`stats.py`'s records: one row per routine per measured lane."""
    now = stamp_now()
    if "date" in meta and meta["date"] != now["date"]:
        now = {**now, "date": meta["date"], "time": "--:--"}
    for rec in out_rows:
        rows = []
        for lane in lanes:
            ns = rec.get(f"{lane}_ns")
            if ns in (None, ""):
                continue
            ratio = 1.0 if lane == "rust" else rec.get("ratio" if lane == "native" else f"{lane}_ratio")
            rows.append({
                "date": now["date"], "time": now["time"], "commit": meta.get("commit"),
                "dirty": meta.get("dirty"), "host": meta.get("host"), "subject": subject,
                "lane": lane, "value": f"{float(ns):.0f}", "unit": "ns/op",
                "spread": f"{float(rec[f'{lane}_spread']):.1f}" if rec.get(f"{lane}_spread") not in (None, "") else "",
                "ratio": f"{float(ratio):.2f}" if ratio not in (None, "") else "",
                "source": source, "note": "",
            })
        if rows:
            append(rec.get("bench", ""), rec["routine"], rows)


def read_all():
    """Every row: (bench, routine, row dict)."""
    out = []
    if not os.path.isdir(STORE):
        return out
    for bench in sorted(os.listdir(STORE)):
        d = os.path.join(STORE, bench)
        if not os.path.isdir(d):
            continue
        for name in sorted(os.listdir(d)):
            if not name.endswith(".tsv"):
                continue
            with open(os.path.join(d, name), encoding="utf-8") as f:
                lines = f.read().splitlines()
            if not lines:
                continue
            head = lines[0].split("\t")
            for line in lines[1:]:
                out.append((bench, name[:-4], dict(zip(head, line.split("\t")))))
    return out


def matches(bench, routine, filters):
    if not filters:
        return True
    for f in filters:
        b, _, r = f.partition("/") if "/" in f else (f, "", "")
        bench_hit = bench == b or bench.split("_")[0].lstrip("0") == b.lstrip("0")
        if "/" in f and bench_hit and routine == r:
            return True
        if "/" not in f and (bench_hit or routine == f):
            return True
    return False


def show(a):
    rows = [(b, r, x) for b, r, x in read_all()
            if matches(b, r, a.filter)
            and (not a.subject or x.get("subject") == a.subject)
            and (not a.lane or x.get("lane") == a.lane)]
    if not rows:
        print("no local results match" + (f" (store: {STORE})" if not os.path.isdir(STORE) else ""))
        return
    by = {}
    for b, r, x in rows:
        by.setdefault((b, r), []).append(x)
    for (b, r), xs in by.items():
        print(f"{b}/{r}")
        for x in xs[-a.last:] if a.last else xs:
            val = x.get("value", "-")
            try:
                val = f"{float(val):,.0f}"
            except ValueError:
                pass
            spread = f"±{x['spread']}%" if x.get("spread", "-") != "-" else ""
            ratio = f"{x['ratio']}x" if x.get("ratio", "-") != "-" else ""
            note = x.get("note", "-") if x.get("note", "-") != "-" else ""
            print(f"  {x.get('date')} {x.get('time')}  {x.get('commit')}{'*' if x.get('dirty') == 'yes' else ' '}"
                  f" {x.get('subject', '-'):16} {x.get('lane', '-'):8} {val:>16} {x.get('unit', ''):12}"
                  f" {spread:>7} {ratio:>7}  {note}".rstrip())


def subjects(_a):
    seen = {}
    for b, r, x in read_all():
        s = seen.setdefault(x.get("subject", "-"), {"rows": 0, "routines": set(), "last": ""})
        s["rows"] += 1
        s["routines"].add(f"{b}/{r}")
        s["last"] = max(s["last"], f"{x.get('date')} {x.get('time')}")
    if not seen:
        print("no local results yet")
    for name, s in sorted(seen.items(), key=lambda kv: kv[1]["last"], reverse=True):
        print(f"{name:24} {s['rows']:5} rows  {len(s['routines']):4} routines  last {s['last']}")


def add(a):
    bench, _, routine = a.routine.partition("/")
    if not routine:
        sys.exit("add: name the routine as BENCH/ROUTINE")
    now = stamp_now()
    append(bench, routine, [{
        **now, "host": os.uname().nodename.split(".")[0], "subject": a.subject or subject_default(),
        "lane": a.lane, "value": a.value, "unit": a.unit, "spread": "", "ratio": a.ratio or "",
        "source": "by hand", "note": a.note,
    }])
    print(f"recorded {a.routine} {a.lane} {a.value} {a.unit}")


def import_tsv(a):
    meta, head, recs = {}, None, []
    with open(a.file, encoding="utf-8") as f:
        for line in f.read().splitlines():
            if line.startswith("# ") and "=" in line:
                k, _, v = line[2:].partition("=")
                meta[k] = v
            elif head is None:
                head = line.split("\t")
            elif line:
                recs.append(dict(zip(head, line.split("\t"))))
    lanes = sorted({k[:-3] for r in recs for k in r if k.endswith("_ns")})
    record_stats(recs, lanes, meta, a.subject or subject_default(), f"import {os.path.basename(a.file)}")
    print(f"filed {len(recs)} routine(s) from {a.file}, lanes {','.join(lanes)}")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd")
    s = sub.add_parser("show")
    s.add_argument("filter", nargs="*")
    s.add_argument("--subject", default="")
    s.add_argument("--lane", default="")
    s.add_argument("--last", type=int, default=0)
    sub.add_parser("subjects")
    i = sub.add_parser("import")
    i.add_argument("file")
    i.add_argument("--subject", default="")
    d = sub.add_parser("add")
    d.add_argument("routine")
    d.add_argument("--lane", required=True)
    d.add_argument("--value", required=True)
    d.add_argument("--unit", default="ns/op")
    d.add_argument("--ratio", default="")
    d.add_argument("--subject", default="")
    d.add_argument("--note", default="")
    argv = sys.argv[1:] or ["subjects"]
    if argv[0] not in ("show", "subjects", "add", "import", "-h", "--help"):
        argv = ["show", *argv]
    a = ap.parse_args(argv)
    {"show": show, "subjects": subjects, "add": add, "import": import_tsv}[a.cmd](a)


if __name__ == "__main__":
    main()
