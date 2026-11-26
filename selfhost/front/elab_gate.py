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
import argparse, threading, collections, concurrent.futures as cf, glob, os, re, shutil, subprocess, sys, tempfile

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
    # `LOFT_C_SPLIT_SHIP=1`: the optimised build writes the elaborator's giant functions in
    # parts too (`@FR-R-CSplit`, gcc and clang above 2000 call sites), so cc1's memory stays
    # bounded: one 165 000-site function took it past 8.8 GB, out of a GitHub runner's 15.
    ship = [] if os.environ.get("LOFT_ELAB_TOOL_BUILD") == "dev" else ["--ship"]
    b = subprocess.run([loft, "--native-c", *ship, "--out", out, src],
                       env=dict(env, LOFT_TIMEOUT=str(BUILD_SECONDS), LOFT_C_SPLIT_SHIP="1"),
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
# path -> [(function, subject, item, side)] of the not-yet-agreeing definitions of a refused
# program (`STATE_PENDING`, unscored).
STATE_PENDING = {}
ROUTE = re.compile(r"^state (\d) (\d+):(\d+) (route|refuse) (.*)$")

# The `route` lines of both sides and the port's `refuse` lines by program
# (elaborator-instruments.md § 2), filled by `one` when the state trace is on:
# path -> [(side, pass, line, col, text)], side `rust` / `port` / `refuse`.
ROUTES = {}
# The function each route site belongs to, `{path: {(side, pass, line): name}}`: the name of the
# next `fn-end` of that pass in the same trace (a site prints before its function's end).  What
# the route join reads to say whose a disagreement is.
ROUTE_FN = {}
FN_END_ANY = re.compile(r"^state (\d) \d+:\d+ fn-end (\S+)")


def route_owners(side, stderr):
    """`{(side, pass, line): function}` for every route line of one trace."""
    out, pending = {}, {}
    for l in stderr.splitlines():
        m = ROUTE.match(l)
        if m:
            pending.setdefault(m.group(1), []).append((side, m.group(1), m.group(2)))
            continue
        e = FN_END_ANY.match(l)
        if e:
            for k in pending.pop(e.group(1), []):
                out.setdefault(k, e.group(2))
    return out
# The reimplementation contract (`LOFT_REPORT_CONTRACT`, src/parser/contract.rs and
# selfhost/src/el_contract.loft): per program that agrees on its image, the facts its functions
# carry differently — {fact: count} — and the facts this port does not derive yet (`?`).
CONTRACT = {}
# The state kinds the PORT reports anywhere in the run (`one` adds each program's): a kind it
# reports nowhere is a fact it does not derive yet, counted apart like the contract's `?`.
PORT_STATE_KINDS = set()
# A refused program's agreeing definitions: path -> {definition: (contract, state, reasons)
# differences}, decided after the run (a kind not derived yet is known only then).
REFUSED_FN_LAYERS = {}
REFUSED_LAYERS = {}  # refused program -> (the port reported a layer, Rust did)
AGREE_SILENT = set()  # agreeing programs one side reported no contract and no state for
ONE_SIDED = {}  # refused program -> (definitions only Rust registers, only the port does)
CONTRACT_LINE = re.compile(r"^\[contract\] fn=(\S+) (.*)$")


def contract_facts(text):
    """{function: {fact: value}} of a `[contract]` report."""
    out = {}
    for l in text.splitlines():
        m = CONTRACT_LINE.match(l)
        if m:
            out[m.group(1)] = dict(kv.split("=", 1) for kv in m.group(2).split(" ") if "=" in kv)
    return out


# The image's function state (`LOFT_REPORT_STATE`, `report::state_report`): per program that
# agrees on its image, the state items its functions carry differently, by kind.
STATE = {}
STATE_LOCK = threading.Lock()
STATE_LINE = re.compile(r"^\[state\] fn=(\S+) (var=\S+|param=\S+|def|inline_refs)(.*)$")


def state_table(text):
    """{(function, subject): set of items} of a `[state]` report."""
    out = {}
    for l in text.splitlines():
        m = STATE_LINE.match(l)
        if m:
            out[(m.group(1), m.group(2))] = set(m.group(3).split())
    return out


# Every compared fact's TIER: who could observe it differing (the owner's question, and C122's
# line between the language and the compiler).  Strict equality holds for every tier; the tier
# says how to read a difference.
#   T1 semantic — values, faults, effect order, accept or refuse: a defect in one side.  The IR
#      itself is this tier's channel, but an IR difference can also be representation, so the IR
#      is reported as `IR` and judged by a reader.
#   T2 correctness-bearing implementation — ownership and delivery (what is freed, copied,
#      adopted): invisible when right, a fault or a wrong value when wrong.  Probe which side is
#      right before either moves.
#   T3 pure implementation — no program can observe it even when "wrong" (the rule that set a
#      flag, a temporary's name or number, a slot, a reservation's capacity): transcription; the
#      cleaner routine wins and Rust may move.
#   T4 diagnostics — read only by a lint: which warnings appear.
# `user_named` decides "compiler temporary" (`Function::is_compiler_generated`) only for a name
# starting with `_`, which codegen reads; on any other name it only feeds the never-read lint.
TIERS = {
    "state": {"user_named(_)": "T2", "user_named": "T4", "user_appended": "T4", "copy_bound": "T4",
              # What a program can observe of a definition: a field's runtime check and its
              # message, a key, a write-once or public field, visibility, which generic instance,
              # the C binding.
              "check": "T1", "check_message": "T1", "primary": "T1", "const_field": "T1",
              "pub_field": "T1", "pub": "T1", "instance_of": "T1", "instance_args": "T1",
              "c_symbol": "T1", "c_sig": "T1"},
    "contract": {},
    "reasons": {},
}
TIER_DEFAULT = {"state": "T2", "contract": "T2", "reasons": "T3"}


def tier(layer, kind):
    """The tier of a `kind` the `layer` compares (see TIERS)."""
    return TIERS[layer].get(kind, TIER_DEFAULT[layer])


def tagged(layer, kinds):
    """A program's differing kinds (a set or a counter's keys), each with its tier."""
    return ", ".join(f"{k} [{tier(layer, k)}]" for k in sorted(kinds))


def state_kinds(table):
    """The kinds of the items a state table carries, as `state_compare` names them."""
    out = set()
    for (fn, subject), items in table.items():
        for item in items:
            kind = "inline_refs" if subject == "inline_refs" else item.split("=")[0]
            out.add(kind)
            if kind == "user_named":
                out.add("user_named(_)")
    return out


def state_compare(rust, port, own, path=None):
    """{kind: count} of the state items the two images carry differently over the program's
    own functions — a flag by its name, a dep list as `deps`, the inline refs as
    `inline_refs`.  `LOFT_ELAB_STATE_FILE=<file>`: each differing item, one
    `program\tfunction\tsubject\titem\tside` line (`rust` = only Rust carries it)."""
    kinds = collections.Counter()
    lines = []
    for key in set(rust) | set(port):
        if key[0] not in own:
            continue
        r, p = rust.get(key, set()), port.get(key, set())
        for item in r ^ p:
            kind = "inline_refs" if key[1] == "inline_refs" else item.split("=")[0]
            if kind == "user_named" and key[1].startswith("var=_"):
                kind = "user_named(_)"
            kinds[kind] += 1
            lines.append(f"{path}\t{key[0]}\t{key[1]}\t{item}\t{'rust' if item in r else 'loft'}\n")
    if lines and path and os.environ.get("LOFT_ELAB_STATE_FILE"):
        with STATE_LOCK, open(os.environ["LOFT_ELAB_STATE_FILE"], "a", encoding="utf-8") as fh:
            fh.write("".join(lines))
    return kinds


def contract_compare(rust, port, own):
    """({fact: functions that differ}, {fact: functions this port does not derive}) over the
    program's own functions.  Every fact either side reports is compared, so a fact only one
    side has differs; a fact the port prints as `?` (not derived) differs too, and is also
    counted apart for the line.  A function only one side reports differs on `report`."""
    differ, missing = collections.Counter(), collections.Counter()
    for f in own:
        if (f in rust) != (f in port):
            differ["report"] += 1
            continue
        r, p = rust.get(f, {}), port.get(f, {})
        for k in sorted(set(r) | set(p)):
            if p.get(k) == "?":
                missing[k] += 1
                differ[k] += 1
            elif p.get(k) != r.get(k):
                differ[k] += 1
    return differ, missing

# The rule that set each image flag (`LOFT_TRACE_VARFLAGS=*`, src/variables/mod.rs
# `trace_flag`, mirrored by `el_vars::trace_flag`): part of the contract — the same flag on the
# same variable is set by the same Rust rule.  Rust prints the line of the setter's call, which
# `rust_rule` resolves to the function enclosing it; the port prints that function's name.
REASONS = {}
VARFLAG_LINE = re.compile(r"^\[varflag\] (\S+) (\S+) \(var=\d+\) in (\S+) @ (\S+)")
RUST_FN = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+|unsafe\s+)*fn\s+(\w+)")
RUST_FNS = {}
RUST_FNS_LOCK = threading.Lock()
RUST_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def rust_rule(loc):
    """The Rust function enclosing `file:line:col` — a flag's reason."""
    f, line = loc.split(":")[:2]
    with RUST_FNS_LOCK:
        if f not in RUST_FNS:
            fns = []
            try:
                with open(os.path.join(RUST_ROOT, f), encoding="utf-8") as fh:
                    for n, l in enumerate(fh, 1):
                        m = RUST_FN.match(l)
                        if m:
                            fns.append((n, m.group(1)))
            except OSError:
                pass
            RUST_FNS[f] = fns
        fns = RUST_FNS[f]
    name = loc
    for n, fn in fns:
        if n > int(line):
            break
        name = fn
    return name


def flag_reasons(text, rust):
    """{(function, variable, flag): set of reasons} of a `[varflag]` trace."""
    out = collections.defaultdict(set)
    for l in text.splitlines():
        m = VARFLAG_LINE.match(l)
        if m:
            flag, var, fn, why = m.groups()
            key = fn[2:] if fn.startswith("n_") else fn
            # A CLEARED flag (`-user_named`, `retire_capture_placeholders`) is no longer in the
            # image: the rules that set it are dropped, as the image has none.
            if flag.startswith("-"):
                out.pop((key, var, flag[1:]), None)
                continue
            out[(key, var, flag)].add(rust_rule(why) if rust else why)
    return out


def reasons_compare(rust, port, own, path=None):
    """{flag: count} of the flags both front ends set on a variable of the program's own
    functions but by different rules.  `LOFT_ELAB_REASON_FILE=<file>`: each, one
    `program\tfunction\tvariable\tflag\trust rules\tport rules` line."""
    own = {f[2:] if f.startswith("n_") else f for f in own}
    kinds = collections.Counter()
    lines = []
    for key in set(rust) & set(port):
        if key[0] in own and rust[key] != port[key]:
            kinds[key[2]] += 1
            lines.append(f"{path}\t{key[0]}\t{key[1]}\t{key[2]}\t{','.join(sorted(rust[key]))}\t{','.join(sorted(port[key]))}\n")
    if lines and path and os.environ.get("LOFT_ELAB_REASON_FILE"):
        with STATE_LOCK, open(os.environ["LOFT_ELAB_REASON_FILE"], "a", encoding="utf-8") as fh:
            fh.write("".join(lines))
    return kinds

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


# The points each side printed for a program (port-debug-output.md § 4): path -> (the Rust
# front end's set, the elaborator's), read by the self-check on the positive control's program.
POINT = re.compile(r"^state \d \d+:\d+ (\S+)")
SEEN_POINTS = {}
# What both sides print for any function with a statement in it.
ALWAYS = ("stmt", "stmt-end", "var", "fn-end")

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
        r = subprocess.run([loft] + lib_args + [path], env=dict(env, LOFT_EMIT_PARSED=img, LOFT_REPORT_CONTRACT="1", LOFT_TRACE_VARFLAGS="*"),
                           capture_output=True, timeout=180)
        if r.returncode != 0 or not os.path.exists(img):
            return "noimage", "", {}
        o = subprocess.run([defs, img, "program", std], env=dict(env, LOFT_REPORT_STATE="1"), capture_output=True, text=True,
                           errors="replace", timeout=300)
        m = subprocess.run([elab, img, path, std], env=dict(env, LOFT_REPORT_CONTRACT="1", LOFT_REPORT_STATE="1", LOFT_TRACE_VARFLAGS="*"), capture_output=True, text=True,
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
    # The definitions only one side registers (`ONE_SIDED`): a refusal must not drop a definition
    # Rust registers — one that does shifts the number of every definition after it, so they all
    # differ in numbers only.  Counted per program, the schema and the head left out.
    if refused:
        only_r = sorted(n for n in w if n not in mn and n not in ("(head)", "(schema)"))
        only_m = sorted(n for n in mn if n not in w and n not in ("(head)", "(schema)"))
        if only_r or only_m:
            with STATE_LOCK:
                ONE_SIDED[path] = (only_r, only_m)
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
        ROUTE_FN[path] = {**route_owners("rust", rust_err), **route_owners("port", m.stderr)}
        # The first pass is compared where the image is: every function of a program that
        # agrees, the agreeing functions of a refused one.  A program whose image differs has
        # every function compared too: its first pass shows where it parted, and the functions
        # that agree there are pinned like any other.
        if not refused:
            PASS1[path] = pass1_verdicts(rust_err, m.stderr, None)
        else:
            PASS1[path] = pass1_verdicts(rust_err, m.stderr, {k for k, (v, _) in fns.items() if v == "agree"})
        if path in PASS1:
            PASS1_AT[path] = pass1_parted(rust_err, m.stderr,
                                          [k for k, (v, _, _) in PASS1[path].items() if v == "pass1-differ"])
        SEEN_POINTS[path] = tuple({m2.group(1) for m2 in map(POINT.match, t.splitlines()) if m2}
                                  for t in (rust_err, m.stderr))
        # The functions a refusal is met in are outside the verdict (their image is not
        # compared), but their first pass can be read all the same: a refused function whose
        # first pass agrees needs a second-pass lowering only.
        if refused:
            rf = pass1_verdicts(rust_err, m.stderr, {k for k, (v, _) in fns.items() if v == "refused"})
            at = pass1_parted(rust_err, m.stderr, [k for k, (v, _, _) in rf.items() if v == "pass1-differ"])
            PASS1_REFUSED[path] = {k: v + at.get(k, ("", "")) for k, v in rf.items()}
    if refused:
        # The agreeing definitions of a refused program are held to every layer too, as a
        # program that agrees is: one whose IR agrees while its contract facts, state or flag
        # reasons do not is `unaligned`.  Its items go to the state and reasons files like an
        # agreeing program's, so the refused programs' definitions are checked before a lift
        # brings them in whole.
        ok = {k for k, (v, _) in fns.items() if v == "agree"}
        if ok:
            r_err = r.stderr.decode("utf-8", "replace")
            rc, mc = contract_facts(r_err), contract_facts(m.stderr)
            rs, ms = state_table(o.stderr), state_table(m.stderr)
            rr, mr = flag_reasons(r_err, True), flag_reasons(m.stderr, False)
            # A port that reports no layer for the program compares nothing: counted apart, so
            # silence never reads as alignment.
            REFUSED_LAYERS[path] = (bool(mc) or bool(ms), bool(rc) or bool(rs))
            with STATE_LOCK:
                PORT_STATE_KINDS.update(state_kinds(ms))
            state_compare(rs, ms, ok, path)
            reasons_compare(rr, mr, ok, path)
            layers = {}
            for k in ok:
                cd, _ = contract_compare(rc, mc, {k})
                sd = state_compare(rs, ms, {k})
                rd = reasons_compare(rr, mr, {k})
                if cd or sd or rd:
                    layers[k] = (cd, sd, rd)
            REFUSED_FN_LAYERS[path] = layers
        # UNSCORED: the state of the definitions whose IR does not agree yet (`beside`).  A rule
        # that over-marks shows here a cycle before the function's IR agrees and the row is
        # scored; it moves no verdict.
        pending = {k for k, (v, _) in fns.items() if v == "beside"}
        if pending:
            ps_r, ps_m = state_table(o.stderr), state_table(m.stderr)
            rows = []
            for key in set(ps_r) | set(ps_m):
                if key[0] not in pending:
                    continue
                a, b2 = ps_r.get(key, set()), ps_m.get(key, set())
                for item in sorted(a ^ b2):
                    rows.append((key[0], key[1], item, "rust" if item in a else "loft"))
            with STATE_LOCK:
                STATE_PENDING[path] = rows
        return "refused", refused, fns
    if mine == want:
        # The program's functions, and its records and field-carrying variants (their layout and
        # field facts are state lines too).
        own = {mm.group(1) for mm in (re.match(r"^#\d+ (\S+) type=[145] ", l) for l in want) if mm}
        rc_all, mc_all = contract_facts(r.stderr.decode("utf-8", "replace")), contract_facts(m.stderr)
        CONTRACT[path] = contract_compare(rc_all, mc_all, own)
        port_state = state_table(m.stderr)
        if own and (not (rc_all or state_table(o.stderr)) or not (mc_all or port_state)):
            AGREE_SILENT.add(path)
        with STATE_LOCK:
            PORT_STATE_KINDS.update(state_kinds(port_state))
        STATE[path] = state_compare(state_table(o.stderr), port_state, own, path)
        REASONS[path] = reasons_compare(flag_reasons(r.stderr.decode("utf-8", "replace"), True),
                                        flag_reasons(m.stderr, False), own, path)
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
    # The HIDDEN differs: definitions of a refused program that differ (or are unaligned) with no
    # refusal of their own — wrong already, and seen only once the refusal elsewhere is lifted.
    # `(families, program, definition, verdict, shape)`.
    hidden = []
    fn_layers = {}     # program::definition -> the layers an unaligned definition differs in
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
        done = [(futs[fut][0], futs[fut][1]) + tuple(fut.result()) for fut in cf.as_completed(futs)]
        # A state kind the port reports in no program of this run is not derived yet.  It is
        # listed apart on the state line AND holds every program and definition that carries it
        # back: an agree holds on every layer, with no exception for a fact not yet ported.
        rust_kinds = collections.Counter()
        for sd in STATE.values():
            rust_kinds.update({k: 1 for k in sd})
        for layers in REFUSED_FN_LAYERS.values():
            for _, sd, _ in layers.values():
                rust_kinds.update({k: 1 for k in sd})
        not_derived = {k for k in rust_kinds if k not in PORT_STATE_KINDS}
        state_not_derived = collections.Counter()
        for f in list(STATE):
            sd = STATE[f]
            state_not_derived.update({k: 1 for k in sd if k in not_derived})
        for part, f, verdict, detail, fns in done:
            if verdict == "refused":
                for k, (cd, sd, rd) in REFUSED_FN_LAYERS.get(f, {}).items():
                    if cd or sd or rd:
                        fns[k] = ("unaligned", "; ".join(
                            f"{layer} differ: {tagged(layer, kinds)}"
                            for layer, kinds in (("contract", cd), ("state", sd), ("reasons", rd)) if kinds))
            # An agree is valid only when everything aligns (the owner's definition): a program
            # whose IR agrees but whose contract facts, state or flag reasons do not is
            # `unaligned`, counted apart from `differ` (which is the IR's).  A fact not derived
            # yet differs like any other, and a program either side reported no layer for
            # (`AGREE_SILENT`) is unaligned too: silence never reads as alignment.
            if verdict == "agree":
                if f not in CONTRACT or CONTRACT[f][0] or STATE.get(f) or REASONS.get(f) \
                        or f in AGREE_SILENT:
                    verdict = "unaligned"
            # By function only where it adds to the program's verdict: a refused program (a program
            # that agrees pins all its definitions through the `elab` block already).
            if verdict == "refused":
                for fname, (fv, shape) in fns.items():
                    fn_results[baseline.name_of(f, root, suite) + "::" + fname] = fv
                    if fv == "beside":
                        beside.append((shape, baseline.name_of(f, root, suite), fname))
                    elif fv == "unaligned":
                        fn_layers[baseline.name_of(f, root, suite) + "::" + fname] = shape
            per[part][verdict] += 1
            if verdict != "noimage":
                results[baseline.name_of(f, root, suite)] = verdict
            rel = os.path.relpath(f, os.path.dirname(root))
            refusals = " | ".join(sorted(set(detail))) if verdict == "refused" else ""
            # An agreeing or unaligned program's layers: the facts its functions carry differently.
            if verdict in ("agree", "unaligned") and f in CONTRACT:
                cd, _ = CONTRACT[f]
                refusals = "contract " + ("agree" if not cd else "differ: " + tagged("contract", cd))
                sd = STATE.get(f)
                if sd is not None:
                    refusals += "; state " + ("agree" if not sd else "differ: " + tagged("state", sd))
                rd = REASONS.get(f)
                if rd is not None:
                    refusals += "; reasons " + ("agree" if not rd else "differ: " + tagged("reasons", rd))
                if f in AGREE_SILENT:
                    refusals += "; a side reported no layer"
            verdicts.append(f"{verdict}\t{rel}" + (f"\t{refusals}" if refusals else ""))
            if verdict == "refused":
                reasons.update(set(detail))
                fams = {d.split(": ", 1)[0] for d in detail}
                families.update(fams)
                # (`numbers` is a renumbering the refusal itself causes, `emptied` a section it
                # emptied: neither is a difference of the definition's own.)
                for fname, (fv, shape) in fns.items():
                    if fname.startswith("(") or not (fv == "unaligned" or (fv == "beside" and shape not in ("numbers", "emptied"))):
                        continue
                    hidden.append((fams, baseline.name_of(f, root, suite), fname, fv, shape))
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
            shape_of = {f"{prog}::{fname}": shape for shape, prog, fname in beside} | fn_layers
            fh.write("".join(f"{k}\t{v}\t{shape_of.get(k, '')}\n" for k, v in sorted(fn_results.items())))
    if os.environ.get("LOFT_ELAB_VERDICTS"):
        with open(os.environ["LOFT_ELAB_VERDICTS"], "w", encoding="utf-8") as fh:
            fh.write("\n".join(sorted(verdicts)) + "\n")
    tot = collections.Counter()
    for c in per.values():
        tot.update(c)
    n = total = sum(tot.values()) - tot["noimage"]
    print(f"{tot['agree']} of {n} programs agree; {tot['refused']} refused, {tot['differ']} differ"
          + (f", {tot['unaligned']} unaligned" if tot["unaligned"] else "")
          + (f"; {tot['noimage']} the Rust front end refuses" if tot["noimage"] else ""))
    if CONTRACT:
        # The reimplementation contract over the programs that agree on their image: a fact a
        # function carries differently is a behaviour the port gets wrong whatever its code
        # says; a fact the port does not derive yet (`?`) is counted apart.
        cagree = sum(1 for cd, _ in CONTRACT.values() if not cd)
        cdiff, cmiss = collections.Counter(), collections.Counter()
        for cd, cm in CONTRACT.values():
            cdiff.update({k: 1 for k in cd})
            cmiss.update({k: 1 for k in cm})
        print(f"contract: {cagree} of {len(CONTRACT)} agreeing programs carry every derived fact alike"
              + ("; differ on " + ", ".join(f"{k} {v} [{tier('contract', k)}]" for k, v in cdiff.most_common()) if cdiff else "")
              + ("; not derived yet: " + ", ".join(f"{k} {v}" for k, v in cmiss.most_common()) if cmiss else ""))
    if STATE:
        # The image's function state over the programs that agree on their code: a flag the
        # later passes read (frees, copies, the lints) that the two images carry differently.
        sagree = sum(1 for sd in STATE.values() if not sd)
        skinds = collections.Counter()
        for sd in STATE.values():
            skinds.update({k: 1 for k in sd})
        print(f"state: {sagree} of {len(STATE)} agreeing programs carry the same function state"
              + ("; differ on " + ", ".join(f"{k} {v} [{tier('state', k)}]" for k, v in skinds.most_common(12)) if skinds else "")
              + ("; not derived yet: " + ", ".join(f"{k} {v}" for k, v in state_not_derived.most_common()) if state_not_derived else ""))
    if REASONS:
        # The rule behind each flag both front ends set: the port follows Rust's reasoning, so a
        # flag set by another rule is a difference even where the state agrees.
        ragree = sum(1 for rd in REASONS.values() if not rd)
        rkinds = collections.Counter()
        for rd in REASONS.values():
            rkinds.update(rd)
        print(f"reasons: {ragree} of {len(REASONS)} agreeing programs set each shared flag by the same rule"
              + ("; differ on " + ", ".join(f"{k} {v} [{tier('reasons', k)}]" for k, v in rkinds.most_common(8)) if rkinds else ""))
    if CONTRACT:
        # Each unaligned program by the most severe tier among its differences: what reading
        # the gate's verdicts asks first (a T2 needs a probe, a T3 or T4 only a transcription).
        worst = collections.Counter()
        for f, (cd, _) in CONTRACT.items():
            tiers = {tier("contract", k) for k in cd} | {tier("state", k) for k in STATE.get(f) or ()} \
                | {tier("reasons", k) for k in REASONS.get(f) or ()}
            if tiers:
                worst[min(tiers)] += 1
        if worst:
            print("unaligned by tier: " + ", ".join(f"{t} {worst[t]}" for t in sorted(worst))
                  + "  (T2 ownership: probe which side is right; T3 implementation: transcribe; "
                  "T4 lints only)")
    fv = collections.Counter(fn_results.values())
    if REFUSED_LAYERS:
        silent = sum(1 for port, rust in REFUSED_LAYERS.values() if rust and not port)
        print(f"layers of refused programs: compared in {len(REFUSED_LAYERS) - silent} of "
              f"{len(REFUSED_LAYERS)} with agreeing definitions"
              + (f"; the port reports none for {silent}" if silent else ""))
    if ONE_SIDED:
        # A refusal that drops a definition Rust registers shifts every later definition's
        # number (`beside numbers`): the rule is register-then-refuse.  A rise here is a new one.
        nr = sum(len(a) for a, _ in ONE_SIDED.values()); np_ = sum(len(b) for _, b in ONE_SIDED.values())
        print(f"definitions one side registers: {nr} Rust only, {np_} port only, in {len(ONE_SIDED)} refused programs")
        if os.environ.get("LOFT_ELAB_ONE_SIDED_FILE"):
            with open(os.environ["LOFT_ELAB_ONE_SIDED_FILE"], "w", encoding="utf-8") as fh:
                for f, (a, b) in sorted(ONE_SIDED.items()):
                    for n in a:
                        fh.write(f"{baseline.name_of(f, root, suite)}\t{n}\trust\n")
                    for n in b:
                        fh.write(f"{baseline.name_of(f, root, suite)}\t{n}\tport\n")
    if fv["unaligned"]:
        # Which layers and tiers hold the refused programs' unaligned definitions back.
        print(f"definitions of refused programs unaligned: {fv['unaligned']} (their IR agrees; "
              "a layer does not: see the verdict file's definitions)")
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
    # The hidden differs by the refusal family that hides them: `alone` counts those in programs
    # the family alone blocks — what lifting that family exposes at once.
    if hidden:
        by_fam = collections.defaultdict(lambda: [0, set(), 0, set()])
        for fams_, prog, fname, fv, shape in hidden:
            for fam in fams_:
                b = by_fam[fam]
                b[0] += 1
                b[1].add(prog)
                if len(fams_) == 1:
                    b[2] += 1
                    b[3].add(prog)
        print(f"hidden differs (definitions of refused programs that differ with no refusal of their own): "
              f"{len(hidden)} in {len({h[1] for h in hidden})} program(s); by the family hiding them "
              f"(definitions/programs, of which where it alone blocks):")
        for fam, (n, ps, na, pa) in sorted(by_fam.items(), key=lambda x: (-x[1][2], -x[1][0], x[0]))[:12]:
            print(f"  {n:5}/{len(ps):<4} alone {na:4}/{len(pa):<4} {fam}")
        if os.environ.get("LOFT_ELAB_HIDDEN_FILE"):
            with open(os.environ["LOFT_ELAB_HIDDEN_FILE"], "w", encoding="utf-8") as fh:
                for fams_, prog, fname, fv, shape in sorted(hidden, key=lambda h: (sorted(h[0]), h[1], h[2])):
                    fh.write(f"{' | '.join(sorted(fams_))}\t{'alone' if len(fams_) == 1 else len(fams_)}\t{prog}\t{fname}\t{fv}\t{shape}\n")
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
        # The unscored state of not-yet-agreeing definitions (`STATE_PENDING`): one line, and each
        # row to `LOFT_ELAB_STATE_PENDING_FILE`.
        if STATE_PENDING:
            n_rows = sum(len(v) for v in STATE_PENDING.values())
            n_progs = sum(1 for v in STATE_PENDING.values() if v)
            by = collections.Counter(item.split("=")[0] + " " + side for rows_ in STATE_PENDING.values() for _, _, item, side in rows_)
            print(f"state of not-yet-agreeing definitions (unscored): {n_rows} row(s) in {n_progs} program(s); "
                  + ", ".join(f"{k} {n}" for k, n in by.most_common(8)))
            if os.environ.get("LOFT_ELAB_STATE_PENDING_FILE"):
                with open(os.environ["LOFT_ELAB_STATE_PENDING_FILE"], "w", encoding="utf-8") as fh:
                    for path_, rows_ in sorted(STATE_PENDING.items()):
                        for fn_, subj, item, side in rows_:
                            fh.write(f"{path_}\t{fn_}\t{subj}\t{item}\t{side}\n")
        # The route GATE: for the return-delivery classifiers Rust logs as `handler=verdict`, the
        # verdict the port reports at the same site — not only whether it took the handler.  A site
        # is a pass and a line (the n-th verdict of a handler on that line pairs with the n-th);
        # `agree` = the same verdict, `kind` = the same variant with other fields, `differ`,
        # `missing` (the port names no verdict there) and `port only`.
        # (`literal_into_place`: where a literal written into a destination is built, and why;
        # `match_subject`: what a match is over and what that borrows.)
        VALUE_HANDLERS = ("classify_ret_promotion", "classify_reference_delivery", "materialize_return_into",
                          "literal_into_place", "match_subject")
        vals = {h: collections.Counter() for h in VALUE_HANDLERS}
        rows = []
        kind_of = lambda v: re.split(r"[ {(]", v, maxsplit=1)[0]
        # Whose a disagreement is (`owner`, the last column): a site in a definition that agrees on
        # every layer is the CLASSIFIER's — the lowering is Rust's there, the verdict is not; one in
        # a definition that differs or is refused follows from the port's own state (`port-state`);
        # a port-only verdict whose twin is missing elsewhere in the same function is a
        # `position` slip, not a disagreement.
        owners = collections.Counter()
        def fn_of(path, side, site):
            fns_ = ROUTE_FN.get(path, {})
            return fns_.get((side, site[0], site[1])) or fns_.get(("rust" if side == "port" else "port", site[0], site[1])) or "?"
        def fn_verdict(path, fname):
            name = baseline.name_of(path, root, suite)
            pv_ = results.get(name, "?")
            return fn_results.get(name + "::" + fname, pv_) if pv_ == "refused" else pv_
        for path, rs in ROUTES.items():
            for h in VALUE_HANDLERS:
                at = {"rust": collections.defaultdict(list), "port": collections.defaultdict(list)}
                for side, ps, ln, col, text in rs:
                    if side in at and text.startswith(h + "="):
                        at[side][(ps, ln)].append((col, text[len(h) + 1:]))
                prow = []
                for site in sorted(set(at["rust"]) | set(at["port"])):
                    ra, pa = at["rust"].get(site, []), at["port"].get(site, [])
                    for i in range(max(len(ra), len(pa))):
                        rv = ra[i][1] if i < len(ra) else None
                        pv = pa[i][1] if i < len(pa) else None
                        verdict = ("agree" if rv == pv else "port only" if rv is None else "missing" if pv is None
                                   else "kind" if kind_of(rv) == kind_of(pv) else "differ")
                        vals[h][verdict] += 1
                        if verdict != "agree":
                            fname = fn_of(path, "port" if rv is None else "rust", site)
                            prow.append([path, h, site[0], site[1], rv or "", pv or "", verdict, fname])
                # A port-only verdict and a missing one of the same value in the same function and
                # pass are one site printed at two positions.
                for a in prow:
                    if a[6] != "port only":
                        continue
                    for b in prow:
                        if b[6] == "missing" and b[2] == a[2] and b[7] == a[7] and b[4] == a[5] and len(b) == 8:
                            a.append("position")
                            b.append("position")
                            break
                for a in prow:
                    fv = fn_verdict(path, a[7])
                    owner = a[8] if len(a) > 8 else ("classifier" if fv == "agree" else "port-state")
                    owners[(h, owner)] += 1
                    rows.append(tuple(a[:7]) + (f"{a[7]}:{fv}", owner))
        for h in VALUE_HANDLERS:
            c = vals[h]
            sites = sum(c[k] for k in ("agree", "kind", "differ", "missing"))
            print(f"route values: {h} {c['agree']} of {sites} sites agree; {c['kind']} same kind, "
                  f"{c['differ']} differ, {c['missing']} missing, {c['port only']} port only"
                  + "; not agreeing by owner: " + ", ".join(f"{o} {owners[(h, o)]}" for o in ("classifier", "port-state", "position")))
        # `LOFT_ELAB_ROUTE_PROGRAMS=<file>`: the REACH index — every route Rust's trace prints
        # (`parse_match`, `match_subject=literal/record`, `classify_ret_promotion=Bind {…}`), each
        # program that reaches it, how many sites, and the program's verdict: the agreeing
        # programs a change to that road can lose, listed before the change is tried.
        if os.environ.get("LOFT_ELAB_ROUTE_PROGRAMS"):
            reach = collections.defaultdict(collections.Counter)
            for path, rs in ROUTES.items():
                for side, ps, ln, col, text in rs:
                    if side == "rust":
                        reach[text][path] += 1
            with open(os.environ["LOFT_ELAB_ROUTE_PROGRAMS"], "w", encoding="utf-8") as fh:
                for text in sorted(reach):
                    for path, n in sorted(reach[text].items()):
                        name = baseline.name_of(path, root, suite)
                        fh.write(f"{text}\t{name}\t{n}\t{results.get(name, '?')}\n")
        # `LOFT_ELAB_ROUTE_VALUES=<file>`: each site that does not agree, with both verdicts.
        if os.environ.get("LOFT_ELAB_ROUTE_VALUES"):
            with open(os.environ["LOFT_ELAB_ROUTE_VALUES"], "w", encoding="utf-8") as fh:
                for r in rows:
                    fh.write("\t".join(str(x) for x in r) + "\n")
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
            # The self-check: neither side's instruments went silent on that program.
            for side, seen in zip(("the Rust front end", "the elaborator"), SEEN_POINTS.get(control, (set(), set()))):
                gone = [p for p in ALWAYS if p not in seen]
                if gone:
                    sys.exit(f"state trace: {side} printed no `{'`, `'.join(gone)}` line for {control}: "
                             "an instrument went silent")
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
