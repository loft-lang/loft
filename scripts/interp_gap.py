#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Where the INTERPRETER still moves data the native backend no longer moves — per bench routine.

    scripts/interp_gap.py                        # every bench lane: time both backends, census both
    scripts/interp_gap.py --only 14,16           # some lanes
    scripts/interp_gap.py --package <clone>/drawing=drawing   # a library's own bench as well
    scripts/interp_gap.py --timing old.tsv       # reuse a `stats.py --lanes native,interp --tsv` run
    scripts/interp_gap.py --no-timing            # the censuses only (seconds, no ratio column)

A REPORT, run by hand, never a gate (`make interp-gap`).

What it answers
---------------
The native generator applies rewrites the interpreter never sees (`formal/rewrites.md`: "the
interpreter applies no GENERATOR rewrite").  Where such a rewrite removes a record, a copy or a
header walk, the interpreter still pays for it.  Moving the rule into the IR phase — the scope
pass, where R-Place, R-VecCopy, R-ByteCopy already live — gives it to both backends.  This tool
ranks the candidates by evidence:

1. TIME — `bench/stats.py --lanes native,interp`: the interp/native ratio per routine.
2. WHAT THE INTERPRETER DOES — `LOFT_OP_CENSUS` (src/op_census.rs): every operator the
   interpreter executes, keyed by the routine (the line of `main` it ran under) and the
   function, with the bytes it moved through the store's block routes (copy / relocate / text).
   Operators are grouped into families: `frame` (a value onto the stack and back — native keeps
   these in registers), `store` (field and element reads and writes), `records` (records,
   vectors and texts built, copied, appended, freed), `control`, `compute`.
3. WHAT NATIVE DOES INSTEAD — `LOFT_REWRITE_CENSUS_FN` (src/rewrite_census.rs): each rewrite
   admission with the function and the PHASE it was decided in.  `native` is generator-only;
   `ir` and `parse` already reach both backends and are listed apart.

A routine's native-only rewrites are those admitted in the functions its interpreter run
executed.  The per-rule table weighs each rule by the interpreter time spent in the functions it
fires in — the time the rule's function carries, NOT a promise of what porting it saves.

The routine of an op
--------------------
Every bench prints one row per routine from `main`.  The k-th row-printing line of `main`
(`row(…)`, `print_row(…)`, or `println("<name>\\t{…")`) is paired with the k-th row the program
printed; the routine's REGION is the lines of `main` from the last `… = ticks()` before that line
(else the line after the previous row) up to it, exclusive.  Ops the census keyed to those lines
are the routine's, divided by the row's `iters`.  A program whose row lines and printed rows do
not pair up is reported as such, not guessed.  A `par` worker's ops run on another thread and are
not counted (11_par, 19_stdlib_par read low).
"""

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BENCH = ROOT / "bench"

FAMILIES = [
    # (family, regex over the operator name) — first match wins.
    ("control", r"^Op(Goto|Call|StaticCall|Return|Iterate|Step|Parallel|Coroutine)"),
    ("records", r"^Op(CopyRecord|AppendCopy|AppendVector|BindOrCopy|MoveRecord|PlaceRecord|NewRecord|"
                r"FinishRecord|Database|Free(Ref|Text|RecordIn|Scratch)|Clear|InsertVector|ReserveVector|"
                r"PreAllocVector|AdoptVector|Append(Text|Character|TextBytes|Stack)|Format|ClaimChildRec|"
                r"LinkRecord|Delete|Remove|ReplaceVector|KeepVectorRange|SliceVector|Release|Deliver|"
                r"CopyRefOrNull|ConstStoreText|ConstLongText|DistinctStore|InitText|ClearText)"),
    ("frame", r"^Op(Var|Put|Const|Push|FreeStack|ReserveFrame|CreateStack|InitCreateStack|GetStack|"
              r"SetStack|ArgText|RefAlias|InitRef|NullRefSentinel)"),
    ("store", r"^Op(Get|Set|VectorRef|Length|Size|HashFind|HashAdd|HashRemove|RefFromChildRec|Expose|"
              r"SliceView|FillKeyed|ReplaceKeyed|ClearKeyed|IndexGroup|OpVectorIsNull|RefIsNull|VectorIsNull)"),
]
FAMILY_RE = [(f, re.compile(r)) for f, r in FAMILIES]
FAMILY_ORDER = ["frame", "store", "records", "control", "compute"]


def family(op):
    for f, r in FAMILY_RE:
        if r.search(op):
            return f
    return "compute"


# The entry function runs every routine's timing loop, so its own admissions belong to no
# routine; its ops still count.
ENTRY = "n_main"


def short(fn):
    """`n_v_push` → `v_push`; a method `t_4text_split` stays as written."""
    return fn[2:] if fn.startswith("n_") else fn


# ── the programs ──────────────────────────────────────────────────────────────────────────


def programs(only, packages, no_suite):
    """(name, cwd, source path, argv after `loft`) for each program to read."""
    out = []
    if not no_suite:
        for d in sorted(BENCH.glob("[0-9][0-9]_*")):
            src = d / "bench.loft"
            if not src.is_file():
                continue
            if only and not any(d.name == w or d.name.split("_")[0].lstrip("0") == w.lstrip("0") for w in only):
                continue
            out.append((d.name, ROOT, src, ["--path", f"{ROOT}/", str(src)]))
    for spec in packages:
        pkg, _, name = spec.partition("=")
        pkg = Path(pkg).resolve()
        out.append((name or pkg.name, pkg, pkg / "bench" / "bench.loft", ["bench/bench.loft"]))
    return out


def run(cmd, cwd, env, timeout):
    return subprocess.run(cmd, cwd=cwd, env={**os.environ, **env}, capture_output=True, text=True,
                          timeout=timeout)


def op_census(loft, prog, n, timeout):
    """The interpreter's op census, the rows the run printed, and its store work per routine."""
    name, cwd, _src, argv = prog
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / "ops.tsv"
        store = Path(tmp) / "store.tsv"
        p = run([loft, "--interpret", *argv, "--n", str(n)], cwd,
                {"LOFT_OP_CENSUS": str(out), "LOFT_STORE_CENSUS": str(store)}, timeout)
        if p.returncode != 0 or not out.exists():
            return None, None, None, (p.stderr or p.stdout).strip().splitlines()[-1:] or ["no census written"]
        store_work = store_by_routine(store.read_text(), p.stdout) if store.exists() else None
        rows = []
        for line in p.stdout.splitlines():
            parts = line.split("\t")
            if len(parts) == 7 and parts[0] != "routine":
                rows.append((parts[0], int(parts[1])))
        census = []
        for line in out.read_text().splitlines():
            if line.startswith("#"):
                if "not counted" in line:
                    return None, None, None, ["the census binary counts no bytes (build it with --features op-census)"]
                continue
            ln, fn, op, cnt, copy, reloc, text = line.split("\t")
            census.append((int(ln), fn, op, int(cnt), int(copy), int(reloc), int(text)))
        return census, rows, store_work, None


STORE_COLUMNS = ["claims", "grows", "relocations", "relocated_bytes", "copied_bytes", "text_bytes",
                 "stores_new", "deletes", "stack_bytes"]
# The work that moves data out of the cache: `stack_bytes` (copies into the interpreter's own
# hot stack frame, a return value sliding down) is reported apart and never ranks a row.
HEAP_COLUMNS = [c for c in STORE_COLUMNS if c != "stack_bytes"]


def store_by_routine(tsv, stdout):
    """routine -> {column: work per op} from a LOFT_STORE_CENSUS file and the rows the run
    printed.  Each row claims the NEXT pair of consecutive ticks() lines whose clock difference
    is the time it printed, in print order, so two routines that took the same time cannot
    trade intervals."""
    lines = [l for l in tsv.splitlines() if l and not l.startswith("#")]
    if len(lines) < 2:
        return {}
    head = lines[0].split("\t")[1:]
    snaps = [list(map(int, l.split("\t"))) for l in lines[1:]]
    printed = []
    for line in stdout.splitlines():
        f = line.split("\t")
        if len(f) == 7 and f[0] != "routine":
            printed.append((f[0], int(f[1]), int(f[2])))
    out, k = {}, 0
    for name, iters, us in printed:
        for i in range(k, len(snaps) - 1):
            if snaps[i + 1][0] - snaps[i][0] == us:
                work = dict(zip(head, ((b - a) / max(iters, 1) for a, b in zip(snaps[i][1:], snaps[i + 1][1:]))))
                out[name] = {c: work.get(c, 0.0) for c in STORE_COLUMNS}
                k = i + 1
                break
    return out


def native_store(census_loft, prog, n, timeout):
    """The same program built with --native against the census rlib, run with
    LOFT_STORE_CENSUS: its store work per routine, or an error line."""
    name, cwd, _src, argv = prog
    lib = Path(census_loft).parent
    if cwd != ROOT:
        return None, "a library package's native build needs its dependencies — interpreter column only"
    with tempfile.TemporaryDirectory() as tmp:
        rs, exe, store = Path(tmp) / "p.rs", Path(tmp) / "p", Path(tmp) / "store.tsv"
        p = run([census_loft, "--native-emit", str(rs), "--lean", *argv], cwd, {}, timeout)
        if p.returncode != 0 or not rs.exists():
            return None, "native emit failed: " + ((p.stderr or p.stdout).strip().splitlines()[-1:] or ["?"])[0]
        p = run(["rustc", "-C", "opt-level=3", "-C", "codegen-units=1", "--edition=2024",
                 "--extern", f"loft={lib}/libloft.rlib", "-L", f"{lib}/deps", "-o", str(exe), str(rs)],
                cwd, {}, timeout)
        if p.returncode != 0:
            return None, "native build failed: " + ((p.stderr or p.stdout).strip().splitlines()[-1:] or ["?"])[0]
        p = run([str(exe), "--n", str(n)], cwd, {"LOFT_STORE_CENSUS": str(store)}, timeout)
        if p.returncode != 0 or not store.exists():
            return None, "native run failed: " + ((p.stderr or p.stdout).strip().splitlines()[-1:] or ["?"])[0]
        return store_by_routine(store.read_text(), p.stdout), None


def rewrite_census(loft, prog, timeout):
    """(phase, function, rule) → admissions, from one native emission."""
    name, cwd, _src, argv = prog
    with tempfile.TemporaryDirectory() as tmp:
        tsv = Path(tmp) / "rw.tsv"
        p = run([loft, "--native-emit", str(Path(tmp) / "out.rs"), "--lean", *argv], cwd,
                {"LOFT_REWRITE_CENSUS_FN": str(tsv)}, timeout)
        if p.returncode != 0 or not tsv.exists():
            return None, (p.stderr or p.stdout).strip().splitlines()[-1:] or ["no rewrite census written"]
        out = {}
        for line in tsv.read_text().splitlines():
            phase, fn, rule, cnt = line.split("\t")
            out[(phase, fn, rule)] = out.get((phase, fn, rule), 0) + int(cnt)
        return out, None


# ── routine regions ───────────────────────────────────────────────────────────────────────

ROW_LINE = re.compile(r'(\brow\s*\(|\bprint_row\s*\(|println\s*\(\s*"[A-Za-z_][A-Za-z_0-9]*\\t\{)')
TICKS_ASSIGN = re.compile(r"^\s*[A-Za-z_][A-Za-z_0-9]*\s*=\s*ticks\(\)\s*;")


def regions(src, printed):
    """routine → set of `main` lines, or an error string when rows and lines do not pair."""
    lines = src.read_text().splitlines()
    start = next((i for i, l in enumerate(lines) if re.match(r"\s*(pub\s+)?fn\s+main\s*\(", l)), None)
    if start is None:
        return "no fn main"
    end = next((i for i in range(start + 1, len(lines)) if lines[i].startswith("}")), len(lines))
    body = range(start + 1, end)
    row_lines = [i for i in body if ROW_LINE.search(lines[i]) and not lines[i].lstrip().startswith("//")]
    if len(row_lines) != len(printed):
        return f"{len(printed)} rows printed, {len(row_lines)} row lines in main"
    out = {}
    prev = start
    for (routine, _iters), at in zip(printed, row_lines):
        first = next((i for i in range(at - 1, prev, -1) if TICKS_ASSIGN.match(lines[i])), prev + 1)
        out[routine] = set(range(first + 1, at + 1))  # 1-based lines first+1 .. at (exclusive of the row line)
        out[routine].discard(at + 1)
        prev = at
    return out


# ── timing ────────────────────────────────────────────────────────────────────────────────


def timing(a, progs):
    """(bench, routine) → (interp ns/op, native ns/op), from stats.py or a saved TSV."""
    path = a.timing
    if not path:
        if a.no_timing:
            return {}
        path = str(Path(a.work) / "timing.tsv")
        cmd = [sys.executable, str(BENCH / "stats.py"), "--lanes", "native,interp", "--samples", str(a.samples),
               "--tsv", path, "--loft", a.loft]
        suite = [p[0] for p in progs if p[1] == ROOT]
        if suite:
            cmd += ["--only", ",".join(suite)]
        else:
            cmd += ["--no-suite"]
        for spec in a.package:
            cmd += ["--package", spec]
        print(f"# timing both backends: {' '.join(cmd[1:])}", flush=True)
        if subprocess.run(cmd).returncode != 0:
            print("# stats.py reported a failure; the rows it measured are used", flush=True)
    out = {}
    if not os.path.exists(path):
        return out
    head = None
    for line in open(path):
        if line.startswith("#"):
            continue
        parts = line.rstrip("\n").split("\t")
        if head is None:
            head = parts
            continue
        r = dict(zip(head, parts))
        try:
            out[(r["bench"], r["routine"])] = (float(r["interp_ns"]), float(r["native_ns"]))
        except (KeyError, ValueError):
            pass
    return out


# ── the report ────────────────────────────────────────────────────────────────────────────


def fmt_bytes(b):
    for unit in ("B", "KB", "MB", "GB"):
        if b < 1024 or unit == "GB":
            return f"{b:,.0f} {unit}" if unit == "B" else f"{b:,.1f} {unit}"
        b /= 1024


def analyse(prog, census, printed, rw):
    """One program → a list of routine records."""
    name, _cwd, src, _argv = prog
    reg = regions(src, printed)
    if isinstance(reg, str):
        return None, reg
    native = defaultdict(dict)   # fn → rule → count (generator-only)
    shared = defaultdict(dict)   # fn → rule → count (ir / parse)
    for (phase, fn, rule), n in (rw or {}).items():
        (native if phase == "native" else shared)[fn][rule] = n
    iters = dict(printed)
    out = []
    for routine, lines in reg.items():
        n = max(iters[routine], 1)
        fns = defaultdict(lambda: dict(ops=0, fam=defaultdict(int), bytes=[0, 0, 0], opc=defaultdict(int)))
        for ln, fn, op, cnt, copy, reloc, text in census:
            if ln not in lines:
                continue
            f = fns[fn]
            f["ops"] += cnt
            f["fam"][family(op)] += cnt
            f["opc"][op] += cnt
            f["bytes"][0] += copy
            f["bytes"][1] += reloc
            f["bytes"][2] += text
        total = sum(f["ops"] for f in fns.values())
        fam = defaultdict(int)
        moved = [0, 0, 0]
        for f in fns.values():
            for k, v in f["fam"].items():
                fam[k] += v
            for i in range(3):
                moved[i] += f["bytes"][i]
        # A rule's share: the fraction of the routine's interpreter ops spent in the functions
        # it is admitted in.
        rules = defaultdict(float)
        shared_rules = defaultdict(float)
        for fn, f in fns.items():
            if fn == ENTRY:
                continue
            for rule in native.get(fn, {}):
                rules[rule] += f["ops"] / (total or 1)
            for rule in shared.get(fn, {}):
                shared_rules[rule] += f["ops"] / (total or 1)
        out.append(dict(bench=name, routine=routine, iters=n, ops=total / n, fam={k: v / n for k, v in fam.items()},
                        moved=[m / n for m in moved], fns=fns, rules=dict(rules), shared=dict(shared_rules),
                        native_by_fn=native))
    return out, None


def render(recs, times, failures, a):
    for r in recs:
        t = times.get((r["bench"], r["routine"]))
        r["interp_ns"], r["native_ns"] = t if t else (None, None)
        r["ratio"] = (t[0] / t[1]) if t and t[1] > 0 else None
    recs.sort(key=lambda r: (r["ratio"] is None, -(r["ratio"] or 0), -r["ops"]))
    md = ["# Interpreter against native — where the interpreter still moves data", ""]
    md.append("Generated by `scripts/interp_gap.py` (`make interp-gap`); a report, never a gate.  Ratios are "
              "interp ns/op over native ns/op from `bench/stats.py`; ops and bytes are the interpreter's, per "
              "op of the routine.  Families: frame = values onto the stack and back, store = field/element "
              "reads and writes, records = records/vectors/texts built, copied, appended, freed.  "
              "Native-only rewrites are the generator's admissions in the functions the routine ran.")
    md.append("")
    md += store_section(recs)
    md.append("## Routines, by interp/native")
    md.append("")
    md.append("| bench | routine | interp ns/op | native ns/op | interp/native | interp ops/op | frame | store | records | "
              "bytes moved/op | native-only rewrites (share of ops in their functions) |")
    md.append("|---|---|--:|--:|--:|--:|--:|--:|--:|--:|---|")
    for r in recs:
        ops = r["ops"] or 1
        pct = lambda k: f"{100 * r['fam'].get(k, 0) / ops:.0f} %"
        rules = ", ".join(f"{k} {100 * v:.0f} %" for k, v in sorted(r["rules"].items(), key=lambda kv: -kv[1])[:5]) or "—"
        interp = "" if r["interp_ns"] is None else f"{r['interp_ns']:,.0f}"
        native = "" if r["native_ns"] is None else f"{r['native_ns']:,.0f}"
        ratio = "" if r["ratio"] is None else f"{r['ratio']:.1f}×"
        md.append(f"| {r['bench']} | {r['routine']} | {interp} | {native} | {ratio} | "
                  f"{r['ops']:,.0f} | {pct('frame')} | {pct('store')} | {pct('records')} | "
                  f"{fmt_bytes(sum(r['moved']))} | {rules} |")
    md.append("")
    md += port_candidates(recs)
    md += details(recs, a)
    if failures:
        md.append("## Not read")
        md.append("")
        for f in failures:
            md.append(f"- {f}")
        md.append("")
    return "\n".join(md) + "\n"


def store_extra(r):
    """The bytes the interpreter moves out of place that native does not, per op: relocated,
    copied and written as text, interpreter minus native."""
    i, n = r.get("store_i"), r.get("store_n")
    if not i or not n:
        return None
    moved = ("relocated_bytes", "copied_bytes", "text_bytes")
    return sum(i[c] for c in moved) - sum(n[c] for c in moved)


def store_section(recs):
    """Per routine: the store work of one op on each backend — the question whether the
    interpreter does more work on stores than the compiled code."""
    rows = [r for r in recs if r.get("store_i") and r.get("store_n")]
    if not rows:
        return []
    num = lambda v: f"{v:,.0f}" if v >= 10 or v == 0 else f"{v:.1f}"
    def pair(r, c):
        return f"{num(r['store_i'][c])} / {num(r['store_n'][c])}"
    more = [r for r in rows if any(r["store_i"][c] > r["store_n"][c] + 0.5 for c in HEAP_COLUMNS)]
    md = ["## Store work: interpreter against native", "",
          "One op of each routine, counted by `LOFT_STORE_CENSUS` at the store chokepoints both backends "
          "share (interpreter / native).  A row where the interpreter's figure is higher is store work a "
          "native rule avoids — an object created, a record grown and moved, a block copied — and the data "
          "that work moves leaves the cache.  Copies into the interpreter's own stack frame (a return "
          "value sliding down) stay in cache and are shown apart, as `stack bytes`.  Ranked by the extra "
          "bytes moved; "
          f"{len(more)} of {len(rows)} routines do more store work on the interpreter.", "",
          "| routine | extra bytes | claims | grows | relocations | relocated bytes | copied bytes | text bytes | stores created | stack bytes |",
          "|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|"]
    for r in sorted(rows, key=lambda r: -(store_extra(r) or 0)):
        md.append(f"| {r['bench']}/{r['routine']} | {fmt_bytes(max(store_extra(r), 0))} | "
                  + " | ".join(pair(r, c) for c in ("claims", "grows", "relocations", "relocated_bytes",
                                                     "copied_bytes", "text_bytes", "stores_new",
                                                     "stack_bytes")) + " |")
    md.append("")
    return md


def port_candidates(recs):
    """Per generator rule: the routines and functions it fires in, weighed by the interpreter
    time those functions carry."""
    by_rule = defaultdict(lambda: dict(weight=0.0, share=0.0, routines=set(), fns=set(), sites=0))
    for r in recs:
        ops = sum(f["ops"] for f in r["fns"].values()) or 1
        for fn, f in r["fns"].items():
            if fn == ENTRY:
                continue
            for rule, c in r["native_by_fn"].get(fn, {}).items():
                e = by_rule[rule]
                share = f["ops"] / ops
                e["weight"] += share * (r["interp_ns"] or 0)
                e["share"] += share
                e["routines"].add(f"{r['bench']}/{r['routine']}")
                e["fns"].add(short(fn))
                e["sites"] += c
    if not by_rule:
        return []
    md = ["## Native-only rewrites — the candidates for the IR phase", "",
          "Each generator rule and the interpreter work in the functions it is admitted in, two ways: "
          "`routines' worth` sums, over the routines, the share of each routine's interpreter ops those functions "
          "carry (scale-free: every routine counts once), and `interp ns` sums the time they carry (dominated by "
          "the slowest lanes).  Neither is what porting the rule saves: read the routine's detail below for what "
          "the interpreter does there.", "",
          "| rule | routines' worth | interp ns in its functions | routines | functions | sites |",
          "|---|--:|--:|--:|---|--:|"]
    for rule, e in sorted(by_rule.items(), key=lambda kv: -kv[1]["share"]):
        fns = ", ".join(sorted(e["fns"])[:8]) + (" …" if len(e["fns"]) > 8 else "")
        md.append(f"| {rule} | {e['share']:.1f} | {e['weight']:,.0f} | {len(e['routines'])} | {fns} | {e['sites']} |")
    md.append("")
    return md


def details(recs, a):
    md = ["## Per routine", ""]
    for r in recs:
        head = f"### {r['bench']} / {r['routine']}"
        if r["ratio"] is not None:
            head += f" — {r['ratio']:.1f}× native"
        md.append(head)
        md.append("")
        c, rl, t = r["moved"]
        md.append(f"{r['ops']:,.0f} interpreter ops per op; bytes moved per op: copy {fmt_bytes(c)}, "
                  f"relocate {fmt_bytes(rl)}, text {fmt_bytes(t)}.")
        if r["shared"]:
            md.append("Rewrites both backends already run here: "
                      + ", ".join(f"{k} ({100 * v:.0f} %)" for k, v in sorted(r["shared"].items(), key=lambda kv: -kv[1]))
                      + ".")
        md.append("")
        md.append("| function | ops/op | frame | store | records | control | compute | bytes/op | top operators "
                  "| native-only rewrites |")
        md.append("|---|--:|--:|--:|--:|--:|--:|--:|---|---|")
        n = r["iters"]
        for fn, f in sorted(r["fns"].items(), key=lambda kv: -kv[1]["ops"])[:a.functions]:
            ops = f["ops"] or 1
            fam = " | ".join(f"{100 * f['fam'].get(k, 0) / ops:.0f} %" for k in FAMILY_ORDER)
            top = ", ".join(f"{op[2:]} {c / n:,.0f}" for op, c in sorted(f["opc"].items(), key=lambda kv: -kv[1])[:5])
            rules = "" if fn == ENTRY else ", ".join(
                f"{k} {v}" for k, v in sorted(r["native_by_fn"].get(fn, {}).items(), key=lambda kv: -kv[1]))
            rules = rules or "—"
            md.append(f"| {short(fn)} | {f['ops'] / n:,.0f} | {fam} | {fmt_bytes(sum(f['bytes']) / n)} | {top} | {rules} |")
        md.append("")
    return md


def as_json(recs, failures, a):
    """The report's numbers, for a page to render (render() has sorted `recs`)."""
    routines = []
    for r in recs:
        n = r["iters"]
        fns = []
        for fn, f in sorted(r["fns"].items(), key=lambda kv: -kv[1]["ops"])[:a.functions]:
            ops = f["ops"] or 1
            fns.append(dict(
                name=short(fn), ops=f["ops"] / n,
                fam={k: f["fam"].get(k, 0) / ops for k in FAMILY_ORDER},
                bytes=sum(f["bytes"]) / n,
                top=[[op[2:], c / n] for op, c in sorted(f["opc"].items(), key=lambda kv: -kv[1])[:5]],
                rules={} if fn == ENTRY else r["native_by_fn"].get(fn, {})))
        ops = r["ops"] or 1
        routines.append(dict(
            bench=r["bench"], routine=r["routine"], interp_ns=r["interp_ns"], native_ns=r["native_ns"],
            ratio=r["ratio"], ops=r["ops"], fam={k: r["fam"].get(k, 0) / ops for k in FAMILY_ORDER},
            moved=r["moved"], rules=r["rules"], shared=r["shared"], functions=fns,
            store={"interp": r.get("store_i"), "native": r.get("store_n")}))
    rules = defaultdict(lambda: dict(share=0.0, ns=0.0, routines=set(), fns=set(), sites=0))
    for r in recs:
        total = sum(f["ops"] for f in r["fns"].values()) or 1
        for fn, f in r["fns"].items():
            if fn == ENTRY:
                continue
            for rule, c in r["native_by_fn"].get(fn, {}).items():
                e = rules[rule]
                e["share"] += f["ops"] / total
                e["ns"] += f["ops"] / total * (r["interp_ns"] or 0)
                e["routines"].add(f"{r['bench']}/{r['routine']}")
                e["fns"].add(short(fn))
                e["sites"] += c
    return dict(
        routines=routines,
        rules=[dict(rule=k, share=v["share"], ns=v["ns"], routines=sorted(v["routines"]),
                    functions=sorted(v["fns"]), sites=v["sites"])
               for k, v in sorted(rules.items(), key=lambda kv: -kv[1]["share"])],
        not_read=failures)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--only", default="", help="comma-separated bench numbers or names")
    ap.add_argument("--package", action="append", default=[], metavar="DIR[=NAME]",
                    help="a library's bench/bench.loft (a SCRATCH clone: builds write beside it)")
    ap.add_argument("--no-suite", action="store_true", help="only the --package benches")
    ap.add_argument("--n", type=int, default=2, help="--n for the census run (iterations per routine)")
    ap.add_argument("--timing", default="", help="reuse a stats.py TSV with interp and native lanes")
    ap.add_argument("--no-timing", action="store_true", help="censuses only; no ratio column")
    ap.add_argument("--samples", type=int, default=3, help="stats.py samples per lane")
    ap.add_argument("--functions", type=int, default=8, help="functions listed per routine")
    ap.add_argument("--out", default=str(ROOT / "target" / "interp-gap" / "report.md"))
    ap.add_argument("--timeout", type=int, default=900)
    ap.add_argument("--loft", default=os.environ.get("LOFT_BIN", str(ROOT / "target" / "release" / "loft")),
                    help="the binary the TIMING runs (an ordinary release build)")
    ap.add_argument("--census-loft", default=str(ROOT / "target" / "op-census" / "release" / "loft"),
                    help="the binary the censuses run: built with --features op-census, so it counts "
                         "bytes; its store carries counters, so it never times anything")
    a = ap.parse_args()
    if not os.access(a.loft, os.X_OK):
        sys.exit(f"interp_gap: no loft binary at {a.loft} (cargo build --release --bin loft)")
    if not os.access(a.census_loft, os.X_OK):
        sys.exit(f"interp_gap: no census binary at {a.census_loft} — `cargo build --release --bin loft "
                 "--features op-census --target-dir target/op-census` (or run `make interp-gap`)")
    rlib = Path(a.loft).parent / "libloft.rlib"
    if not (a.timing or a.no_timing):
        # The native lane links this rlib, and building the binary never rebuilds it: a stale
        # one measures an older runtime, a missing one fails every native build.  Current
        # means what `make check-rlib` means: nothing under src/ or Cargo.toml is newer.
        built = rlib.stat().st_mtime if rlib.exists() else 0
        newer = [f for f in [ROOT / "Cargo.toml", *(ROOT / "src").rglob("*")] if f.is_file() and f.stat().st_mtime > built]
        if newer:
            sys.exit(f"interp_gap: {rlib} is {'older than ' + str(newer[0].relative_to(ROOT)) if built else 'missing'}"
                     " — `cargo build --release --lib` (or run `make interp-gap`)")
    only = [w.strip() for w in a.only.split(",") if w.strip()]
    progs = programs(only, a.package, a.no_suite)
    if not progs:
        sys.exit("interp_gap: no program matches")
    a.work = str(Path(a.out).parent)
    os.makedirs(a.work, exist_ok=True)

    print(f"# census of {len(progs)} program(s): interpreter ops (--n {a.n}) and native rewrites", flush=True)

    def both(prog):
        census, printed, store_i, err = op_census(a.census_loft, prog, a.n, a.timeout)
        rw, rerr = rewrite_census(a.census_loft, prog, a.timeout)
        store_n, serr = native_store(a.census_loft, prog, a.n, a.timeout)
        return prog, census, printed, err, rw, rerr, (store_i or {}, store_n or {}, serr)

    with ThreadPoolExecutor(max_workers=2) as pool:
        done = list(pool.map(both, progs))
    recs, failures = [], []
    for prog, census, printed, err, rw, rerr, store in done:
        if census is None:
            failures.append(f"{prog[0]}: the interpreter census failed — {err[0]}")
            continue
        if rw is None:
            failures.append(f"{prog[0]}: no native rewrite census — {rerr[0]}")
        got, why = analyse(prog, census, printed, rw)
        if got is None:
            failures.append(f"{prog[0]}: routines not attributed — {why}")
            continue
        store_i, store_n, serr = store
        if serr:
            failures.append(f"{prog[0]}: no native store census — {serr}")
        for r in got:
            r["store_i"] = store_i.get(r["routine"])
            r["store_n"] = store_n.get(r["routine"])
        recs += got
    times = timing(a, progs)
    report = render(recs, times, failures, a)
    Path(a.out).write_text(report)
    Path(a.out).with_suffix(".json").write_text(json.dumps(as_json(recs, failures, a), indent=1))
    # The terminal gets the ranking; the file has the per-routine detail.
    store_md = report.split("## Store work: interpreter against native")
    if len(store_md) > 1:
        print("## Store work: interpreter against native" + store_md[1].split("## Routines, by")[0].rstrip())
    table = report.split("## Native-only rewrites")[0]
    print(table.split("## Routines, by interp/native")[1].strip() if "## Routines" in table else table)
    for f in failures:
        print(f"  not read: {f}")
    print(f"wrote {os.path.relpath(a.out, ROOT)}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
