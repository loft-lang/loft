#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The elaboration gate (@PLN91 stage 1, gate E): does the loft front end elaborate each program
as loft's own parser does?

For each corpus program the Rust front end writes the image as its parser left it
(`LOFT_EMIT_PARSED`, before the scope pass); `compare/defs.loft` reports that image's program
definitions and the program's part of its schema (the types after the standard library's), and
`front/elab.loft` elaborates the program over the image's standard library and reports its own.  A program AGREES when the two reports are equal and the elaboration refused
nothing.  Both runners are built through the C backend once per run.  The refusals are counted
by reason: the constructs the port carries next.

  selfhost/front/elab_gate.py [--part NAME]… [--jobs N] [--show N] [FILE…]
  selfhost/front/elab_gate.py --check BASELINE | --write-baseline BASELINE

The position is gated by a committed baseline (`elab_gate.baseline.tsv` beside this file, in
`selfhost/bench/baseline.py`'s format), layer `elab`: the agree count and every program that
does not agree yet, with its verdict.  `--check` fails on a drop and on any `differ` (an image
that is not Rust's), listed or not; both need the whole corpus and the suite at the pin.
"""
import argparse, collections, concurrent.futures as cf, glob, os, re, shutil, subprocess, sys, tempfile

sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "bench"))
import baseline  # noqa: E402  one baseline format for every survey

PARTS = ["scripts", "docs", "suite", "lib", "cells"]


def corpus(root, suite):
    return {
        "scripts": sorted(glob.glob(os.path.join(root, "tests", "scripts", "*.loft"))),
        "docs": sorted(glob.glob(os.path.join(root, "tests", "docs", "*.loft"))),
        "suite": sorted(glob.glob(os.path.join(suite, "rules", "**", "*.loft"), recursive=True)),
        "lib": sorted(glob.glob(os.path.join(root, "lib", "**", "*.loft"), recursive=True)),
        # The gate's own cells: one shape each that the corpus above does not hold alone — a
        # difference found behind a refusal, kept where the next run compares it.
        "cells": sorted(glob.glob(os.path.join(root, "selfhost", "front", "cells", "*.loft"))),
    }


BUILD_SECONDS = 1800


def build(loft, src, out, env):
    os.makedirs(out, exist_ok=True)
    # The tools' own build is bounded by the build's limit, not by the per-program one: an
    # optimised build of the elaborator takes minutes on a loaded box.
    # `LOFT_ELAB_TOOL_BUILD=dev`: a development build of the tools.  The verdicts are the same;
    # the optimised (`--ship`) compile of the elaborator needs more than 5.5 GB in cc1, which a
    # 7 GB CI runner no longer holds (runs 44227446673, 44231429943, 44233220732).
    ship = [] if os.environ.get("LOFT_ELAB_TOOL_BUILD") == "dev" else ["--ship"]
    b = subprocess.run([loft, "--native-c", *ship, "--out", out, src],
                       env=dict(env, LOFT_TIMEOUT=str(BUILD_SECONDS)),
                       capture_output=True, text=True, timeout=BUILD_SECONDS + 60)
    exe = os.path.join(out, os.path.splitext(os.path.basename(src))[0])
    if not os.path.exists(exe):
        sys.exit(f"{src} did not build:\n{b.stderr[-3000:]}")
    return exe


def lib_dirs(text):
    """The `--lib` directories of a program's `// @ARGS:` line, absolute."""
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    out = []
    for line in text.splitlines():
        if line.startswith("// @ARGS:"):
            args = line[len("// @ARGS:"):].split()
            for k, a in enumerate(args[:-1]):
                if a == "--lib":
                    out.append(os.path.normpath(os.path.join(root, args[k + 1])))
    return out


# A generic instance's name holds spaces (`i_21integer(-128, 127)s1?_n_set_from`): the name runs
# to the first ` type=`, so no instance is folded into the definition before it.
HEAD = re.compile(r"^#\d+ (.+?) type=\d")


def sections(lines):
    """A report by definition: {name: its lines} (a definition's header and body), the schema's
    rows (`NNN:…` and their fields) as one `(schema)` section, anything before the first
    definition as `(head)`.  A name met twice keeps both, the second as `name#2`."""
    out, key, seen = {}, "(head)", {}
    for l in lines:
        m = HEAD.match(l)
        if m:
            name = m.group(1)
            seen[name] = seen.get(name, 0) + 1
            key = name if seen[name] == 1 else f"{name}#{seen[name]}"
        elif re.match(r"^\d+:", l):
            key = "(schema)"
        out.setdefault(key, []).append(l)
    return out


HANDED = re.compile(r"^state \d \d+:\d+ (stmt|arg)( |$)")
# `var` lines say which STATEMENT changed a function's variable table (port-debug-output.md
# § 1): they locate a difference `fn-end` reports and are not part of the verdict.
VAR = re.compile(r"^state \d \d+:\d+ var ")
ROUTE = re.compile(r"^state (\d) (\d+):(\d+) (route|refuse) (.*)$")

# The `route` lines of both sides and the port's `refuse` lines by program
# (elaborator-instruments.md § 2), filled by `one` when the state trace is on:
# path -> [(side, pass, line, col, text)], side `rust` / `port` / `refuse`.
ROUTES = {}

FN_END = re.compile(r"^state 1 \d+:\d+ fn-end (.+?) (?:ret=|var \d+ )")

# `--pass1` (elaborator-instruments.md § 1): each program's first-pass verdict by function,
# filled by `one` when the state trace is on: path -> {function: (verdict, first differing
# line of each side)}.
PASS1 = {}


def pass1_sections(stderr):
    """The FIRST pass's state lines by function: {name: its lines}.  A function's lines are the
    ones up to and including its `fn-end` group; a lambda's `fn-end` therefore also closes the
    lines its enclosing function printed before it — the comparison is of both sides cut alike,
    so a difference is still found, under the lambda's name.  Lines after the last `fn-end`
    are `(top)`."""
    out, cur, name = {}, [], None
    for l in stderr.splitlines():
        # `route` and `refuse` say which code ran, not what a pass left: they are joined by
        # line (`ROUTES`), never compared line for line.
        if not l.startswith("state 1 ") or ROUTE.match(l):
            continue
        # `stmt` and `arg` say what a point was HANDED (`expected=`, the assign target).  Rust
        # keeps `expected` as one field that lingers between sites and the port carries the
        # hint per node, so those lines differ by modelling; the layer compares what a pass
        # LEFT (`stmt-end`, `fn-end`).  `state_diff.py` still shows them.
        if HANDED.match(l) or VAR.match(l):
            continue
        m = FN_END.match(l)
        if name is not None and (m is None or m.group(1) != name):
            out.setdefault(name, []).extend(cur)
            cur, name = [], None
        cur.append(l)
        if m:
            name = m.group(1)
    if cur:
        out.setdefault(name if name is not None else "(top)", []).extend(cur)
    return out


# The first pass of the functions a refusal is met in: path -> {function: (verdict, rust line,
# port line, parted rust, parted port)}.  Reported apart (`LOFT_ELAB_PASS1_REFUSED`), never
# counted or pinned.
PASS1_REFUSED = {}

# Where a function's two variable tables PARTED on the first pass (port-debug-output.md § 1):
# path -> {function: (the first differing `var` line of the Rust side, of the port)}.
PASS1_AT = {}


def pass1_parted(rust, port, names):
    """For each function of `names`, the first `state 1 … var <function> …` line the two sides
    do not share, as (rust line, port line); a function whose `var` lines agree is left out."""
    def lines(stderr):
        return [l for l in stderr.splitlines() if l.startswith("state 1 ") and VAR.match(l)]
    a, b = lines(rust), lines(port)
    out = {}
    for name in names:
        mark = f" var {name} "
        x, y = [l for l in a if mark in l], [l for l in b if mark in l]
        if x == y:
            continue
        k = next((i for i, (p, q) in enumerate(zip(x, y)) if p != q), min(len(x), len(y)))
        out[name] = (x[k] if k < len(x) else "(no line)", y[k] if k < len(y) else "(no line)")
    return out


def pass1_verdicts(rust, port, compared):
    """`pass1-agree` or `pass1-differ` for each function of `compared` (None: every function
    either side printed), with the first differing line of each side."""
    a, b = pass1_sections(rust), pass1_sections(port)
    out = {}
    for name in sorted(set(a) | set(b)):
        if compared is not None and name not in compared:
            continue
        x, y = a.get(name, []), b.get(name, [])
        if x == y:
            out[name] = ("pass1-agree", "", "")
            continue
        k = next((i for i, (p, q) in enumerate(zip(x, y)) if p != q), min(len(x), len(y)))
        out[name] = ("pass1-differ", x[k] if k < len(x) else "(no line)", y[k] if k < len(y) else "(no line)")
    return out


def by_function(want, mine, refused_in, program_refused):
    """Each definition's verdict (front-end-instruments.md item 2): `agree` (its section is
    Rust's), `refused` (a refusal was met in it), `beside` (it differs, holds no refusal of its
    own, in a program that holds one), `differ` (it differs in a program that holds none)."""
    w, m = sections(want), sections(mine)
    out = {}
    for name in sorted(set(w) | set(m)):
        if name == "(head)":
            continue
        if name.split("#")[0] in refused_in:
            out[name] = "refused"
        elif w.get(name) == m.get(name):
            out[name] = "agree"
        else:
            out[name] = "beside" if program_refused else "differ"
    return out, w, m


def one(loft, defs, elab, std, path, tmp, env, i):
    """(verdict, detail): `agree`, `noimage` (the Rust front end refuses it, or the program
    says it expects a refusal), `differ` (the
    first differing line) or `refused` (the reasons)."""
    img = os.path.join(tmp, f"p{i}.store")
    with open(path, encoding="utf-8", errors="replace") as fh:
        text = fh.read()
    if "@EXPECT_ERROR" in text:
        return "noimage", "", {}
    # The `--lib` directories the program's `@ARGS` line names, as the script runner passes them
    # (relative to the repository root): to the Rust front end on its command line, to the
    # elaborator as `LOFT_ELAB_LIB`.
    libs = lib_dirs(text)
    lib_args = [a for d in libs for a in ("--lib", d)]
    env = dict(env, LOFT_ELAB_LIB=":".join(libs))
    try:
        r = subprocess.run([loft] + lib_args + [path], env=dict(env, LOFT_EMIT_PARSED=img),
                           capture_output=True, timeout=180)
        if r.returncode != 0 or not os.path.exists(img):
            return "noimage", "", {}
        o = subprocess.run([defs, img, "program", std], env=env, capture_output=True, text=True,
                           errors="replace", timeout=300)
        m = subprocess.run([elab, img, path, std], env=env, capture_output=True, text=True,
                           errors="replace", timeout=300)
    except subprocess.TimeoutExpired:
        return "differ", "timed out", {}
    finally:
        for f in (img, img + ".dschema"):
            if os.path.exists(f):
                os.remove(f)
    lines = m.stdout.splitlines()
    rust_err = r.stderr.decode("utf-8", "replace") if env.get("LOFT_STATE_TRACE") else ""
    refused = [l[len("refused: "):] for l in lines if l.startswith("refused: ")]
    # `refused-in: <function>\t<line>\t<text>`: each refusal with the function it was met in.
    refused_in = {l[len("refused-in: "):].split("\t")[0] for l in lines if l.startswith("refused-in: ")}
    # `LOFT_ELAB_WHERE_FILE=<file>`: every refusal with the function and source line it was met
    # at, `program\tfunction\tline\ttext` — where a family fires, for porting from the corpus.
    if os.environ.get("LOFT_ELAB_WHERE_FILE"):
        with open(os.environ["LOFT_ELAB_WHERE_FILE"], "a", encoding="utf-8") as fh:
            fh.write("".join(f"{path}\t{l[len('refused-in: '):]}\n" for l in lines if l.startswith("refused-in: ")))
    mine = [l for l in lines if not l.startswith("refused: ") and not l.startswith("refused-in: ")]
    if m.returncode != 0 and not refused:
        return "differ", f"elab exited {m.returncode}: {m.stderr.strip()[-300:]}", {}
    want = o.stdout.splitlines()
    fns, w, mn = by_function(want, mine, refused_in, bool(refused))
    # A `beside` function's shape, for the worklist: whether it differs in numbers only, and
    # whether the port's section is (nearly) empty — a function a refusal elsewhere emptied.
    shapes = {}
    for name, v in fns.items():
        if v == "beside":
            a, b = w.get(name, []), mn.get(name, [])
            digits = lambda xs: [re.sub(r"\d+", "N", x) for x in xs]
            shapes[name] = ("numbers" if digits(a) == digits(b) else
                            "emptied" if len(b) <= 3 < len(a) else
                            f"{abs(len(a) - len(b))} lines")
    fns = {k: (v, shapes.get(k, "")) for k, v in fns.items()}
    if env.get("LOFT_STATE_TRACE"):
        # (side, pass, line, col, text): Rust's routes, the port's routes and its refusals.
        ROUTES[path] = ([("rust",) + m2.groups()[:3] + (m2.group(5),)
                         for m2 in map(ROUTE.match, rust_err.splitlines()) if m2 and m2.group(4) == "route"]
                        + [("port" if m2.group(4) == "route" else "refuse",) + m2.groups()[:3] + (m2.group(5),)
                           for m2 in map(ROUTE.match, m.stderr.splitlines()) if m2])
        # The first pass is compared where the image is: every function of a program that
        # agrees, the agreeing functions of a refused one; a differing program's is not.
        if not refused and mine == want:
            PASS1[path] = pass1_verdicts(rust_err, m.stderr, None)
        elif refused:
            PASS1[path] = pass1_verdicts(rust_err, m.stderr, {k for k, (v, _) in fns.items() if v == "agree"})
        if path in PASS1:
            PASS1_AT[path] = pass1_parted(rust_err, m.stderr,
                                          [k for k, (v, _, _) in PASS1[path].items() if v == "pass1-differ"])
        # The functions a refusal is met in are outside the verdict (their image is not
        # compared), but their first pass can be read all the same: a refused function whose
        # first pass agrees needs a second-pass lowering only.
        if refused:
            rf = pass1_verdicts(rust_err, m.stderr, {k for k, (v, _) in fns.items() if v == "refused"})
            at = pass1_parted(rust_err, m.stderr, [k for k, (v, _, _) in rf.items() if v == "pass1-differ"])
            PASS1_REFUSED[path] = {k: v + at.get(k, ("", "")) for k, v in rf.items()}
    if refused:
        return "refused", refused, fns
    if mine == want:
        return "agree", "", fns
    for k, (a, b) in enumerate(zip(want, mine)):
        if a != b:
            head = next((x for x in reversed(want[:k + 1]) if x.startswith("#")), "")
            return "differ", f"{head}\n    oracle: {a}\n    loft:   {b}", fns
    return "differ", f"oracle {len(want)} lines, loft {len(mine)}", fns


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("files", nargs="*", help="programs to compare (default: the corpus)")
    ap.add_argument("--part", action="append", choices=PARTS, help="corpus parts (default: all)")
    ap.add_argument("--loft", default="target/release/loft")
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 4)
    ap.add_argument("--show", type=int, default=20, help="differing programs to list")
    ap.add_argument("--plan", action="store_true",
                    help="rank refusal families by the programs clearing them in turn would admit")
    ap.add_argument("--which", help="list the programs a refusal starting with this text blocks alone")
    ap.add_argument("--details", type=int, default=3, help="details listed per refusal family")
    ap.add_argument("--check", metavar="BASELINE", help="exit 1 on a drop against BASELINE or any differ")
    ap.add_argument("--functions-against", metavar="BASELINE",
                    help="list the definitions BASELINE's `elab-functions` block pins that no longer agree, "
                         "with any tools and at any pin (a development cycle's early warning); exit 1 if any")
    ap.add_argument("--write-baseline", metavar="BASELINE", help="write the `elab` block of BASELINE")
    ap.add_argument("--pass1", action="store_true",
                    help="also compare the FIRST pass (the state trace's `state 1` lines) of every "
                         "function whose image agrees: `pass1-agree` / `pass1-differ`, ranked by the "
                         "kind of the first differing line; with --write-baseline / --check the "
                         "`elab-pass1` block pins the functions that agree")
    o = ap.parse_args()
    if (o.check or o.write_baseline) and (o.files or o.part):
        sys.exit("--check and --write-baseline read the whole corpus: drop the files and --part")
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    loft = o.loft if os.path.isabs(o.loft) else os.path.join(root, o.loft)
    suite = os.path.realpath(os.environ.get("LOFT_SUITE_DIR", os.path.join(os.path.dirname(root), "loft-suite-91")))
    pin = baseline.require_pin(root, suite, o.check) if (o.check or o.write_baseline) else None
    env = dict(os.environ, LOFT_NO_CACHE="1", LOFT_TIMEOUT="300")
    if o.files:
        # Canonical: the Rust front end names a program by its canonical path (an `assert`'s
        # file literal is it), so both sides are handed that one — following a symbolic link
        # is the driver's, not the elaboration's.
        todo = [("args", os.path.realpath(f)) for f in o.files]
        parts = ["args"]
    else:
        parts = o.part or PARTS
        files = corpus(root, suite)
        todo = [(part, os.path.realpath(f)) for part in parts for f in files[part]]
    tmp = tempfile.mkdtemp(prefix="loft-elab-", dir=os.environ.get("TMPDIR"))
    sh = os.path.join(root, "selfhost")
    # The standard library's store types: what an empty program's image holds.
    empty = os.path.join(tmp, "empty.loft")
    with open(empty, "w", encoding="utf-8") as fh:
        fh.write("fn main() {}\n")
    std = os.path.join(tmp, "std.store")
    subprocess.run([loft, empty], env=dict(env, LOFT_EMIT_PARSED=std), capture_output=True,
                   timeout=180)
    if not os.path.exists(std):
        sys.exit("the empty program wrote no image")
    # `LOFT_ELAB_DEFS=<exe>` / `LOFT_ELAB_BIN=<exe>`: tools built before (a development build of
    # the elaborator is half the optimised one's time), for a run between two edits.  A
    # baseline is written and checked with the tools this run builds.
    prebuilt = os.environ.get("LOFT_ELAB_DEFS"), os.environ.get("LOFT_ELAB_BIN")
    if any(prebuilt) and (o.check or o.write_baseline):
        sys.exit("--check and --write-baseline build their own tools: unset LOFT_ELAB_DEFS / LOFT_ELAB_BIN")
    # The two builds are one C compile each: side by side, the gate waits for the longer only.
    # `LOFT_ELAB_BUILD_JOBS=1`: build the two tools one after the other.  Side by side they halve
    # the wait, and double the peak memory: on a runner of 7 GB the elaborator's optimised C
    # compile (above 5 GB in cc1) ran out of memory beside the other one.
    with cf.ThreadPoolExecutor(int(os.environ.get("LOFT_ELAB_BUILD_JOBS", "2"))) as ex:
        fd = None if prebuilt[0] else ex.submit(build, loft, os.path.join(sh, "compare", "defs.loft"), os.path.join(tmp, "defs"), env)
        fe = None if prebuilt[1] else ex.submit(build, loft, os.path.join(sh, "front", "elab.loft"), os.path.join(tmp, "elab"), env)
        defs = prebuilt[0] or fd.result()
        elab = prebuilt[1] or fe.result()
    per = collections.defaultdict(collections.Counter)
    # Verdicts by FUNCTION (front-end-instruments.md item 2): `program::function` -> verdict,
    # and the `beside` worklist (shape, program, function).
    fn_results = {}
    beside = []
    reasons = collections.Counter()
    families = collections.Counter()
    # The families that alone stand between a program and its comparison: what to port next.
    alone = collections.Counter()
    which = []
    differs = []
    blocked = []
    # `LOFT_ELAB_VERDICTS=<file>`: each program's verdict, one per line — what two runs diff —
    # a refused one followed by its refusals, so a grep names the programs a family blocks.
    verdicts = []
    results = {}          # machine-free program name -> verdict, the Rust-refused left out
    # The trace is asked of the programs' runs only: the tools' own builds stay silent.
    penv = dict(env, LOFT_STATE_TRACE="1") if o.pass1 else env
    with cf.ThreadPoolExecutor(o.jobs) as ex:
        futs = {ex.submit(one, loft, defs, elab, std, f, tmp, penv, i): (part, f)
                for i, (part, f) in enumerate(todo)}
        for fut in cf.as_completed(futs):
            part, f = futs[fut]
            verdict, detail, fns = fut.result()
            # By function only where it adds to the program's verdict: a refused program (a program
            # that agrees pins all its definitions through the `elab` block already).
            if verdict == "refused":
                for fname, (fv, shape) in fns.items():
                    fn_results[baseline.name_of(f, root, suite) + "::" + fname] = fv
                    if fv == "beside":
                        beside.append((shape, baseline.name_of(f, root, suite), fname))
            per[part][verdict] += 1
            if verdict != "noimage":
                results[baseline.name_of(f, root, suite)] = verdict
            rel = os.path.relpath(f, os.path.dirname(root))
            refusals = " | ".join(sorted(set(detail))) if verdict == "refused" else ""
            verdicts.append(f"{verdict}\t{rel}" + (f"\t{refusals}" if refusals else ""))
            if verdict == "refused":
                reasons.update(set(detail))
                fams = {d.split(": ", 1)[0] for d in detail}
                families.update(fams)
                blocked.append(fams)
                if o.which and any(d.startswith(o.which) for d in detail):
                    which.append((len(fams), f))
                if len(fams) == 1:
                    alone.update(set(detail) if len(set(detail)) == 1 else fams)
            elif verdict == "differ":
                differs.append((f, detail))
    # `LOFT_ELAB_FN_VERDICTS=<file>`: each definition of a refused program and its verdict, one
    # `program::definition\tverdict` per line — what a skip sweep compares function by function.
    if os.environ.get("LOFT_ELAB_FN_VERDICTS"):
        with open(os.environ["LOFT_ELAB_FN_VERDICTS"], "w", encoding="utf-8") as fh:
            shape_of = {f"{prog}::{fname}": shape for shape, prog, fname in beside}
            fh.write("".join(f"{k}\t{v}\t{shape_of.get(k, '')}\n" for k, v in sorted(fn_results.items())))
    if os.environ.get("LOFT_ELAB_VERDICTS"):
        with open(os.environ["LOFT_ELAB_VERDICTS"], "w", encoding="utf-8") as fh:
            fh.write("\n".join(sorted(verdicts)) + "\n")
    tot = collections.Counter()
    for c in per.values():
        tot.update(c)
    n = total = sum(tot.values()) - tot["noimage"]
    print(f"{tot['agree']} of {n} programs agree; {tot['refused']} refused, {tot['differ']} differ"
          + (f"; {tot['noimage']} the Rust front end refuses" if tot["noimage"] else ""))
    fv = collections.Counter(fn_results.values())
    print(f"definitions of refused programs: {fv['agree']} of {len(fn_results)} agree; {fv['refused']} "
          f"refused, {fv['beside']} beside a refusal")
    if beside:
        # The shape of a `beside` definition: numbers only and emptied are what a refusal
        # elsewhere does to it (noise); the rest differ by lines, bucketed by how many.
        def bucket(s):
            if not s[0].isdigit():
                return s
            n = int(s.split(" ")[0])
            return "1-2 lines" if n <= 2 else "3-10 lines" if n <= 10 else "more lines"
        sh = collections.Counter(bucket(s) for s, _, _ in beside)
        print("  beside, by shape: " + ", ".join(f"{n} {k}" for k, n in sh.most_common()))
    for part in parts:
        c = per[part]
        print(f"  {part:8} {c['agree']} of {sum(c.values()) - c['noimage']}")
    # A refusal may name its detail after `: ` (the operator, the type, the function): ranked by
    # its family first, then the details of the families that lead.
    print("refusals, by the programs that meet them:")
    for f, k in families.most_common(25):
        print(f"  {k:5}  {f}")
        details = [(r, n) for r, n in reasons.most_common() if r.startswith(f + ": ")]
        for r, n in details[:o.details]:
            print(f"         {n:5}  {r[len(f) + 2:]}")
    if alone:
        print("refusals that alone block a program:")
        for f, k in alone.most_common(15):
            print(f"  {k:5}  {f}")
    if o.plan:
        # Greedy cover: the family whose clearing admits the most programs next, counting a
        # program once every family it meets is cleared.
        print("families to clear, in the order that admits the most programs:")
        left = [set(b) for b in blocked]
        admitted = 0
        for _ in range(25):
            gain = collections.Counter()
            for b in left:
                gain.update(b)
            if not gain:
                break
            def admits(f):
                return sum(1 for b in left if b == {f})
            best = max(gain, key=lambda f: (admits(f), gain[f]))
            admitted += admits(best)
            left = [b - {best} for b in left if b - {best}]
            print(f"  {admitted:5}  {best}  (met by {gain[best]})")
    for k, f in sorted(which)[:o.show]:
        print(f"WHICH {k} {os.path.relpath(f, os.path.dirname(root))}")
    for f, d in sorted(differs)[:o.show]:
        print(f"DIFFER {os.path.relpath(f, os.path.dirname(root))}\n  {d}")
    pass1 = {}            # machine-free `program::function` -> pass1 verdict
    if o.pass1:
        kinds, example = collections.Counter(), {}
        programs = collections.Counter()
        for path, fns1 in PASS1.items():
            prog = baseline.name_of(path, root, suite)
            programs["pass1-differ" if any(v == "pass1-differ" for v, _, _ in fns1.values()) else "pass1-agree"] += 1
            for fname, (v, rust_line, port_line) in fns1.items():
                pass1[f"{prog}::{fname}"] = v
                if v == "pass1-differ":
                    # The kind of the first differing line: its point, and for `fn-end` which fact.
                    shown = rust_line if rust_line != "(no line)" else port_line
                    w = shown.split(" ")
                    kind = w[3] if len(w) > 3 else "(no line)"
                    if kind == "fn-end":
                        kind += " ret" if " ret=" in shown else " var"
                    kinds[kind] += 1
                    example.setdefault(kind, (prog, fname, rust_line, port_line))
        pv = collections.Counter(pass1.values())
        print(f"pass 1: {pv['pass1-agree']} of {len(pass1)} functions agree, {pv['pass1-differ']} differ; "
              f"by program {programs['pass1-agree']} of {sum(programs.values())} agree")
        for kind, n in kinds.most_common():
            prog, fname, rust_line, port_line = example[kind]
            print(f"  {n:5}  first differing at `{kind}`   e.g. {prog}::{fname}\n"
                  f"             rust: {rust_line[:150]}\n             port: {port_line[:150]}")
        # `LOFT_ELAB_PASS1=<file>`: each compared function's first-pass verdict and, where it
        # differs, the first differing line of each side, then the first differing `var` line of
        # each side (the statement where the variable tables parted; empty where they agree).
        if os.environ.get("LOFT_ELAB_PASS1"):
            with open(os.environ["LOFT_ELAB_PASS1"], "w", encoding="utf-8") as fh:
                for path, fns1 in sorted(PASS1.items()):
                    prog = baseline.name_of(path, root, suite)
                    for fname, (v, rust_line, port_line) in sorted(fns1.items()):
                        # and the statement where the two variable tables parted, if they did
                        at = PASS1_AT.get(path, {}).get(fname, ("", ""))
                        fh.write(f"{v}\t{prog}::{fname}\t{rust_line}\t{port_line}\t{at[0]}\t{at[1]}\n")
        # `LOFT_ELAB_PASS1_REFUSED=<file>`: the same six columns for the functions a refusal is
        # met in — which of them agree on the first pass already.
        if os.environ.get("LOFT_ELAB_PASS1_REFUSED"):
            with open(os.environ["LOFT_ELAB_PASS1_REFUSED"], "w", encoding="utf-8") as fh:
                for path, fns1 in sorted(PASS1_REFUSED.items()):
                    prog = baseline.name_of(path, root, suite)
                    for fname, row in sorted(fns1.items()):
                        fh.write(f"{row[0]}\t{prog}::{fname}\t" + "\t".join(row[1:]) + "\n")
            rv = collections.Counter(r[0] for f in PASS1_REFUSED.values() for r in f.values())
            print(f"pass 1 of the functions a refusal is met in: {rv['pass1-agree']} of "
                  f"{sum(rv.values())} agree (not counted above)")
        # `LOFT_ELAB_ROUTES=<file>`: every `route` line of both sides and every `refuse` line of
        # the port, as `program<TAB>side<TAB>pass<TAB>line<TAB>col<TAB>text` — what the
        # where-list joins a refusal's line to, and what route coverage counts.
        if os.environ.get("LOFT_ELAB_ROUTES"):
            with open(os.environ["LOFT_ELAB_ROUTES"], "w", encoding="utf-8") as fh:
                for path, rs in sorted(ROUTES.items()):
                    for side, ps, ln, col, text in rs:
                        fh.write(f"{path}\t{side}\t{ps}\t{ln}\t{col}\t{text}\n")
        # Route coverage (elaborator-instruments.md § 3): per Rust handler, the sites and the
        # programs where Rust takes it, and what the port does there — takes its mirror on that
        # line (`both`), refuses on that line (`refused`), or neither.  A verdict handler
        # (`materialize_return_into=whole`) counts under its function's name.  A program
        # CARRIES a handler when the port takes it at every site Rust does.
        cover = {}
        for path, rs in ROUTES.items():
            rust_at, port_at, refuse_at = collections.defaultdict(set), collections.defaultdict(set), set()
            for side, ps, ln, _, text in rs:
                if side == "refuse":
                    refuse_at.add((ps, ln))
                else:
                    (rust_at if side == "rust" else port_at)[text.split("=")[0]].add((ps, ln))
            for h in set(rust_at) | set(port_at):
                c = cover.setdefault(h, collections.Counter())
                a_, b_ = rust_at[h], port_at[h]
                c["sites"] += len(a_)
                c["both"] += len(a_ & b_)
                c["refused"] += len((a_ - b_) & refuse_at)
                c["neither"] += len(a_ - b_ - refuse_at)
                c["port only"] += len(b_ - a_)
                if a_:
                    c["programs"] += 1
                    c["carried" if a_ <= b_ else "refusing" if (a_ - b_) & refuse_at else "missing"] += 1
        taken = {h: c for h, c in cover.items() if c["programs"]}
        whole = sum(1 for c in taken.values() if c["carried"] == c["programs"])
        print(f"routes: {whole} of {len(taken)} routes the corpus exercises are carried in every program that takes them")
        ranked = sorted(taken, key=lambda h: (-taken[h]["refusing"], -taken[h]["missing"], h))
        print("  what to port next, by the programs the port refuses a route in "
              "(programs: Rust takes it / carried / refusing / missing):")
        for h in ranked[:12]:
            c = taken[h]
            print(f"    {h:30} {c['programs']:5} / {c['carried']:5} / {c['refusing']:5} / {c['missing']:5}")
        # `LOFT_ELAB_ROUTE_COVERAGE=<file>`: the whole table, one row per handler.
        if os.environ.get("LOFT_ELAB_ROUTE_COVERAGE"):
            cols = ["programs", "carried", "refusing", "missing", "sites", "both", "refused", "neither", "port only"]
            with open(os.environ["LOFT_ELAB_ROUTE_COVERAGE"], "w", encoding="utf-8") as fh:
                fh.write("handler\t" + "\t".join(cols) + "\n")
                for h in sorted(cover, key=lambda h: (-cover[h]["refusing"], -cover[h]["missing"], h)):
                    fh.write(h + "\t" + "\t".join(str(cover[h][k]) for k in cols) + "\n")
        # The positive control: the layer must be able to fail.  One function that agrees is
        # read again with the port's trace told to answer a wrong result on the first pass
        # (`LOFT_STATE_TRACE_CONTROL`, `el_trace::fn_end`); it must read `pass1-differ`.
        control = next((p for p, f in sorted(PASS1.items())
                        if f and all(v == "pass1-agree" for v, _, _ in f.values())), None)
        if control is None:
            print("pass 1: no program agrees whole, so the positive control did not run")
        else:
            before = dict(PASS1[control])
            one(loft, defs, elab, std, control, tmp, dict(penv, LOFT_STATE_TRACE_CONTROL="1"), 10 ** 6)
            caught = any(v == "pass1-differ" for v, _, _ in PASS1[control].values())
            PASS1[control] = before
            if not caught:
                sys.exit(f"pass 1: the positive control read pass1-agree on {control}: the layer cannot fail")
            print(f"pass 1: positive control caught ({os.path.relpath(control, os.path.dirname(root))})")
    # `LOFT_ELAB_KEEP=1` keeps the built elaborator and the standard image for single runs.
    if not os.environ.get("LOFT_ELAB_KEEP"):
        shutil.rmtree(tmp, ignore_errors=True)
    # The function layer pins only what AGREES: its rows are the agreeing functions, and a check
    # fails when one of them stops agreeing — a refused or `beside` function never fails.
    fn_agree = {k: "agree" for k, v in fn_results.items() if v == "agree"}
    if o.write_baseline:
        baseline.write(o.write_baseline, pin, "elab-functions", len(fn_agree), len(fn_results), fn_agree)
    if o.check or o.functions_against:
        fb = baseline.read(o.check or o.functions_against).get("elab-functions")
        if fb is None:
            print("the baseline has no `elab-functions` block: write it with --write-baseline")
        else:
            lost = sorted(k for k in fb[2] if k in fn_results and fn_results[k] != "agree")
            for k in lost[:40]:
                print(f"FUNCTION DROP\t{k}\t{fn_results[k]}")
            if lost:
                print(f"elab-functions: {len(lost)} function(s) that agreed no longer agree")
                sys.exit(1)
    # The first-pass layer is a RATCHET: it pins the functions whose first pass agrees, names
    # each one that stopped (`PASS1 DROP`), and fails only when fewer agree than were pinned —
    # a `pass1-differ` that was never pinned is the worklist, not a failure.
    p1_agree = {k: "agree" for k, v in pass1.items() if v == "pass1-agree"}
    if o.pass1 and o.write_baseline:
        baseline.write(o.write_baseline, pin, "elab-pass1", len(p1_agree), len(pass1), p1_agree)
    if o.pass1 and (o.check or o.functions_against):
        pb = baseline.read(o.check or o.functions_against).get("elab-pass1")
        if pb is None:
            print("the baseline has no `elab-pass1` block: write it with --pass1 --write-baseline")
        else:
            lost = sorted(k for k in pb[2] if pass1.get(k) == "pass1-differ")
            for k in lost[:40]:
                print(f"PASS1 DROP\t{k}")
            if len(p1_agree) < pb[0]:
                print(f"elab-pass1: {len(p1_agree)} functions agree on the first pass, {pb[0]} were pinned")
                sys.exit(1)
    if o.write_baseline:
        baseline.write(o.write_baseline, pin, "elab", tot["agree"], total,
                       {k: v for k, v in results.items() if v != "agree"})
    if o.check:
        fails, repins = baseline.check("elab", baseline.read(o.check), tot["agree"], total, results)
        baseline.report("elab", o.check, fails, repins)
        sys.exit(0)
    if o.write_baseline or o.functions_against:
        sys.exit(0)
    sys.exit(0 if tot["agree"] == total else 1)


if __name__ == "__main__":
    main()
