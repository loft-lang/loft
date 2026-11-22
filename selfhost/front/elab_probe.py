#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""One program against the elaboration gate's kept tools (@PLN91 stage 1, gate E).

The gate (`elab_gate.py`) answers for the whole corpus in minutes; this answers for one
program in a second, with tools built before:

  elab_probe.py one FILE              the two reports diffed (`AGREE`, or the diff)
  elab_probe.py where FILE            each refusal with the source line it was first met on
  elab_probe.py fam TEXT VERDICTS     the source lines behind refusal TEXT over the programs
                                      a `LOFT_ELAB_VERDICTS` file lists with it

The tools: `--tools DIR` is a gate run's kept directory (`LOFT_ELAB_KEEP=1`: `defs/defs`,
`std.store`); `--elab EXE` is the elaborator to ask — a development build
(`loft --native-c --out DIR selfhost/front/elab.loft`) by default `DIR/elab/elab`.  Both
default to `$LOFT_ELAB_TOOLS` and `$LOFT_ELAB_BIN`.
"""
import argparse, collections, difflib, os, re, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def reports(o, path):
    """(Rust's report, loft's report, the refusals' `where` lines) for one program."""
    path = os.path.realpath(path)
    env = dict(os.environ, LOFT_NO_CACHE="1")
    with tempfile.TemporaryDirectory(prefix="loft-probe-") as tmp:
        img = os.path.join(tmp, "p.store")
        made = subprocess.run([o.loft, path], env=dict(env, LOFT_EMIT_PARSED=img, LOFT_TRACE_RETPROMO="1", LOFT_REPORT_CONTRACT="1"),
                              capture_output=True, text=True, errors="replace", timeout=180)
        if not os.path.exists(img):
            return None, None, []
        std = os.path.join(o.tools, "std.store")
        dr = subprocess.run([os.path.join(o.tools, "defs", "defs"), img, "program", std],
                            env=dict(env, LOFT_REPORT_STATE="1"), capture_output=True, text=True,
                            errors="replace", timeout=300)
        want = dr.stdout.splitlines()
        m = subprocess.run([o.elab, img, path, std], env=dict(env, LOFT_ELAB_WHERE="1", LOFT_TRACE_RETPROMO="1", LOFT_REPORT_STATE="1", LOFT_REPORT_CONTRACT="1"),
                           capture_output=True, text=True, errors="replace", timeout=300)
    o.decisions = (made.stderr.splitlines(), m.stderr.splitlines())
    o.state = ([l for l in dr.stderr.splitlines() if l.startswith("[state] ")],
               [l for l in m.stderr.splitlines() if l.startswith("[state] ")])
    return want, m.stdout.splitlines(), [l for l in m.stderr.splitlines() if l.startswith("where: ")]


RUST_RETPROMO = re.compile(r"^\[retpromo\] fn=(\S+) v=\d+ name=(\S+) pass=(\d) .*=> (\w+)")
LOFT_RETPROMO = re.compile(r"^\[retpromo\] fn=(\S+) name=(\S+) pass=(\d) => (\w+)")


def decisions(o, want):
    """The return-buffer decisions (`LOFT_TRACE_RETPROMO`) the two front ends take differently:
    ([(fn, name, pass, rust, loft)] that conflict, [(fn, name, pass, rust)] Rust takes and this
    port does not report).  Only `Rename` and `Bind` are compared — the verdicts the port names —
    and only for the program's own functions (those its report lists)."""
    rust_lines, loft_lines = getattr(o, "decisions", ([], []))
    own = {m.group(1) for m in (re.match(r"^#\d+ (\S+) type=1 ", l) for l in want or []) if m}
    rust, loft = {}, {}
    for l in rust_lines:
        m = RUST_RETPROMO.match(l)
        if m and m.group(1) in own and m.group(4) in ("Rename", "Bind"):
            rust[m.group(1, 2, 3)] = m.group(4)
    for l in loft_lines:
        m = LOFT_RETPROMO.match(l)
        if m:
            loft[m.group(1, 2, 3)] = m.group(4)
    conflicts = [k + (rust.get(k, "-"), v) for k, v in sorted(loft.items()) if rust.get(k) != v]
    missing = [k + (v,) for k, v in sorted(rust.items()) if k not in loft]
    return conflicts, missing


STATE = re.compile(r"^\[state\] fn=(\S+) (var=\S+|param=\S+|def|inline_refs)(.*)$")


def state_differences(o):
    """The function state (`LOFT_REPORT_STATE`: variable flags, `inline_refs`) the two images
    carry differently: [(fn, var, rust flags, loft flags)].  A function only one side reports
    state for is compared as holding none on the other."""
    rust_lines, loft_lines = getattr(o, "state", ([], []))
    def table(lines):
        t = {}
        for l in lines:
            m = STATE.match(l)
            if m:
                t[(m.group(1), m.group(2))] = m.group(3).strip()
        return t
    rust, loft = table(rust_lines), table(loft_lines)
    return [k + (rust.get(k, "-"), loft.get(k, "-")) for k in sorted(set(rust) | set(loft))
            if rust.get(k, "") != loft.get(k, "")]


CONTRACT = re.compile(r"^\[contract\] fn=(\S+) (.*)$")


def contract_differences(o, want):
    """The contract facts (`LOFT_REPORT_CONTRACT`) the two front ends answer differently for the
    program's own functions: [(fn, fact, rust, loft)], a `?` being a fact this port does not
    derive yet."""
    rust_lines, loft_lines = getattr(o, "decisions", ([], []))
    own = {m.group(1) for m in (re.match(r"^#\d+ (\S+) type=1 ", l) for l in want or []) if m}
    def table(lines):
        t = {}
        for l in lines:
            m = CONTRACT.match(l)
            if m and m.group(1) in own:
                t[m.group(1)] = dict(kv.split("=", 1) for kv in m.group(2).split(" ") if "=" in kv)
        return t
    rust, loft = table(rust_lines), table(loft_lines)
    out = []
    for f in sorted(rust):
        for k, v in rust[f].items():
            lv = loft.get(f, {}).get(k, "-")
            if lv != v:
                out.append((f, k, v, lv))
    return out


def show_contract(diffs, limit):
    real = [d for d in diffs if d[3] != "?"]
    gaps = collections.Counter(d[1] for d in diffs if d[3] == "?")
    if real:
        print(f"contract facts that differ (LOFT_REPORT_CONTRACT: {len(real)}; fn fact: rust | loft):")
        for f, k, r, l in real[:limit]:
            print(f"  {f} {k}: {r} | {l}")
    if gaps:
        print("contract facts this port does not derive yet: " + ", ".join(f"{k} ({v})" for k, v in gaps.most_common()))


def show_state(diffs, limit):
    if diffs:
        print(f"function state that differs (LOFT_REPORT_STATE: {len(diffs)}; fn var: rust | loft):")
        for f, v, r, l in diffs[:limit]:
            print(f"  {f} {v}: {r} | {l}")


def show_decisions(conflicts, missing, all_missing):
    if conflicts:
        print("return decisions that differ (LOFT_TRACE_RETPROMO: fn name pass rust loft):")
        for f, n, p, r, l in conflicts:
            print(f"  {f} {n} pass {p}: rust {r}, loft {l}")
    if missing and all_missing:
        print("return decisions Rust takes that this port does not report:")
        for f, n, p, r in missing:
            print(f"  {f} {n} pass {p}: rust {r}")


def where(o, path):
    """[(line number, refusal, source line)] of a program's refusals."""
    _, _, lines = reports(o, path)
    with open(path, encoding="utf-8", errors="replace") as fh:
        src = fh.read().splitlines()
    out = []
    for l in lines:
        head, _, what = l[len("where: line "):].partition(": ")
        n = int(head)
        out.append((n, what, src[n - 1].strip() if 0 < n <= len(src) else ""))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("what", choices=["one", "where", "fam"])
    ap.add_argument("arg", help="a program, or the refusal text for `fam`")
    ap.add_argument("verdicts", nargs="?", help="`fam`: a LOFT_ELAB_VERDICTS file")
    ap.add_argument("--tools", default=os.environ.get("LOFT_ELAB_TOOLS", ""))
    ap.add_argument("--elab", default=os.environ.get("LOFT_ELAB_BIN", ""))
    ap.add_argument("--loft", default=os.path.join(ROOT, "target", "release", "loft"))
    ap.add_argument("--max", type=int, default=40, help="`fam`: programs read; `one`: diff lines")
    o = ap.parse_args()
    if not o.tools:
        sys.exit("name the kept tools: --tools DIR or LOFT_ELAB_TOOLS (a LOFT_ELAB_KEEP=1 gate run)")
    o.elab = o.elab or os.path.join(o.tools, "elab", "elab")
    if o.what == "one":
        want, mine, _ = reports(o, o.arg)
        if want is None:
            sys.exit("the Rust front end wrote no image")
        conflicts, missing = decisions(o, want)
        show_decisions(conflicts, missing, want != mine)
        show_contract(contract_differences(o, want), o.max)
        show_state(state_differences(o), o.max)
        if want == mine:
            print("AGREE")
            return
        for l in list(difflib.unified_diff(want, mine, "rust", "loft", lineterm="", n=0))[:o.max]:
            print(l)
        sys.exit(1)
    if o.what == "where":
        for n, what, src in where(o, o.arg):
            print(f"line {n}: {what}\n    | {src}")
        return
    if not o.verdicts:
        sys.exit("fam needs the verdicts file of a gate run (LOFT_ELAB_VERDICTS)")
    seen = collections.Counter()
    base = os.path.dirname(ROOT)
    with open(o.verdicts, encoding="utf-8") as fh:
        rels = [l.split("\t")[1] for l in fh if o.arg in l and l.count("\t") >= 2]
    for rel in rels[:o.max]:
        for _, what, src in where(o, os.path.join(base, rel)):
            if o.arg in what:
                seen[src[:170]] += 1
    for src, k in seen.most_common():
        print(f"{k:4}  {src}")


if __name__ == "__main__":
    main()
