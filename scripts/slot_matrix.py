#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The slot matrix — every operand position of loft's syntax, with every form of
expression in it (@PLN221, doc/claude/plans/221-slot-walk/README.md).

A construct tested with a plain variable in an operand position is evaluated once by
construction.  The same construct with a call, a lookup or a `?? default` there is a
different lowering, and when that lowering mentions the operand twice the program still
answers right: only a counter inside the operand shows it.  This tool keeps the list of
operand positions ("slots"), the forms of expression that go into them ("fillers"), and
runs the product.

    check      the three registers are well formed, every rule a slot cites is defined,
               and every lexeme the lexer and the parser know is in a slot, a filler, or
               scripts/syntax_outside.tsv with its reason.  The PR gate (doc_hygiene).
    selftest   prove `check` can fail: a slot row removed, a lexeme added, a rule
               misspelled — each must be reported.
    list       the slots, per family, with the count of each count class.
    fillers    run every filler in a neutral slot: it must bump its counter once and
               give its type's plain value.
    run        run cells and say which fail.  Default: every slot with its plain variable,
               which validates the register itself.  `--filler all` is the matrix,
               `--backend both` adds the C backend, `--slot` and `--kind` narrow it.
    cell       one shape tried at once, without touching a table:
                   cell 'for p in <S> { s += p.k; }' --type coll
                   cell 's = <S> + j;' --filler 'at(<C>, 1)' --plain i
               It runs the four checks on both backends and prints the table rows the
               shape would be.  The names a statement may use are the prelude's.
    coverage   what the matrix does not exercise, without reading an issue:
               `coverage rules` lists the evaluation rules of the formal chapters that no
               slot cites; `coverage ops` the IR operations the corpus makes the compiler
               emit and no cell does (the corpus is read once a build: minutes).
    explain    one failing cell laid open for whoever debugs it:
                   explain wr-elem-index.int-fnval.vector
               its program KEPT in a directory (the cell, its bound-first twin, its order
               cell and the plain variable as functions of one file), each statement
               printed, the cell run alone on both backends with loft's own diagnostics on
               (`LOFT_REFREE_FAILS`, `LOFT_LEAK_SITES`), every field that differs with both
               values, and the IR of the cell diffed against the IR of its twin — the
               working form beside the broken one.
    expr       the mirror of `cell`: one EXPRESSION tried wherever it can stand —
                   expr 'at(<C>, 1)' --type int
               in every slot of its type and, at depth two (the default), inside every
               wrapper with a hole of its type; an expression with a hole of its own
               (`h.ones[<I:int> - 1]`) is tried AROUND every filler of that type.
               `--context all --frame all` adds the loop, the branch and the parameters.
               It first checks the expression is usable (one bump, the plain value).
    gate       the whole matrix on both backends against scripts/slot_matrix.baseline: a
               cell that is not ok and not listed fails (BROKEN), and so does a listed cell
               that now passes (FIXED).  `--family F` takes one slot family and its lines,
               which is how tests/slot_matrix.rs splits the PR gate into seven tests;
               The interpreter runs every cell; the C backend, which compiles each one,
               a sample (every slot with its plain variable and its type's first filler)
               unless `--full`.  `--update` rewrites the baseline from a whole, full run.  On GitHub each fold is
               an annotation on the table row it names.
    walk       the matrix on an OLDER loft binary beside this one (`--old <binary>`): the
               cells that failed then and pass now are what the matrix would have caught,
               the ones that passed then and fail now are regressions.

What a cell asserts, on each backend:
    count   the operand ran as often as the slot's `runs` column says;
    order   (an `.order` cell) with every plain variable of the statement replaced by a
            call that writes a letter, the letters and the operand's `S` come in source
            order — made only where the operand runs exactly once and nothing in the
            statement is conditional or repeated;
    value   the cell leaves behind what its bound-first twin does (`t0 = <operand>;` and
            the statement with `t0`), or, where the operand does not run exactly once,
            what the same statement leaves with a plain variable;
    leak    the test runner's `LOFT_LEAK_CHECK` reports nothing, and no store is freed twice.
A cell the compiler refuses is REFUSED, not failed; a slot none of whose cells compile fails
as vacuous.  The cells of one collection kind run as the test functions of one generated
program, so the whole matrix is a handful of processes per backend.

@falsified-at: hand-measured — `LOFT=<the release build of 2026-11-18, which has loft#2573>
slot_matrix.py run --slot it-for --filler all` fails 8 of 36 cells ("the operand ran 5
time(s), the slot says 1"); the same command on a build with the fix fails none.

Any run can go to the background: `slot_matrix.py --bg FILE <command …>` returns at once
with the pid, writes to FILE, and ends FILE with `slot_matrix: done exit=<n>` — the line
`scripts/wait_for.sh FILE 'slot_matrix: done' --pid <pid>` waits for.

Two more axes are off unless asked for: `--context` puts the statement in a loop or a
branch (scripts/syntax_contexts.tsv), `--frame` gives it its values as parameters instead of
locals (the frames of the prelude).  `all` for both multiplies the matrix by eight.

The registers: scripts/syntax_slots.tsv, scripts/syntax_fillers.tsv,
scripts/syntax_outside.tsv, scripts/syntax_contexts.tsv, and the world a cell runs in,
scripts/syntax_slots_prelude.loft.
"""
import argparse
import concurrent.futures
import os
import re
import subprocess
import sys
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
SLOTS_TSV = os.path.join(HERE, "syntax_slots.tsv")
FILLERS_TSV = os.path.join(HERE, "syntax_fillers.tsv")
OUTSIDE_TSV = os.path.join(HERE, "syntax_outside.tsv")
CONTEXTS_TSV = os.path.join(HERE, "syntax_contexts.tsv")
PRELUDE = os.path.join(HERE, "syntax_slots_prelude.loft")
BASELINE = os.path.join(HERE, "slot_matrix.baseline")
EXPECT_TSV = os.path.join(HERE, "syntax_expect.tsv")

COUNTS = ("once", "first", "cond", "step")
ONCE = ("once", "first")
FAMILIES = ("iteration", "write", "read", "call", "operator", "control", "construct")
# The collection kinds a cell can be generated for.  A slot runs in all of them when a
# filler of its type reads one of the prelude's kind-dependent names; otherwise the kind
# changes nothing it reads, and it runs in the first only.
KINDS = (("vector", "vector<P>"), ("sorted", "sorted<P[k]>"),
         ("index", "index<P[k]>"), ("hash", "hash<P[k]>"))
KIND_NAMES = re.compile(r"\b(xs|mk|items|tk)\b")
OUTSIDE_CLASSES = ("none:", "decl:", "unreached:")
DONE = "slot_matrix: done"
SLOT = "<S>"
BIND = "<BIND>"
HOLE = re.compile(r"<I:(\w+)>")
COUNTER = "<C>"



def types_of(fillers):
    """The operand types: whatever the fillers table has a plain variable for."""
    return [f["type"] for f in fillers if f["id"] == f["type"] + "-var"]


def kinded(fillers):
    """The types whose value depends on the collection kind."""
    found = {f["type"] for f in fillers if KIND_NAMES.search(f["expr"])}
    return found


def neutral(slots, fillers):
    """Per type, the slot its fillers are tried in by `fillers`: the first in the register
    that reads the value once in `main`."""
    out = {}
    for t in types_of(fillers):
        for s in slots:
            # Not a write: a filler that answers ANOTHER record than the plain variable
            # would write somewhere else, and that is no fault of the filler.
            if s["type"] == t and s["count"] in ONCE and not s["decl"] and s["family"] != "write":
                out[t] = s["id"]
                break
    return out


def rows(path):
    """The data rows of a register: tab-separated, `#` lines and blank lines skipped."""
    out = []
    with open(path, encoding="utf-8") as f:
        for nr, line in enumerate(f, 1):
            line = line.rstrip("\n")
            if line and not line.startswith("#"):
                out.append((nr, line.split("\t")))
    return out


def load_slots(path=SLOTS_TSV):
    slots, problems = [], []
    for nr, cols in rows(path):
        if len(cols) not in (7, 8):
            problems.append(f"{os.path.basename(path)}:{nr}: {len(cols)} columns, want 7 or 8")
            continue
        slot = dict(zip(("id", "family", "rule", "count", "runs", "type", "stmt"), cols))
        slot["decl"] = cols[7] if len(cols) == 8 else ""
        # `type@kind,kind` keeps the slot to the collection kinds it has a meaning in, and
        # `type!filler,filler` keeps fillers out that are not of the type the statement
        # needs — so that what the compiler still refuses is a refusal of the language.
        m = re.fullmatch(r"([\w-]+?)(?:@([\w,]+))?(?:!([\w,()-]+))?", slot["type"])
        if not m:
            problems.append(f"{os.path.basename(path)}:{nr}: type `{slot['type']}` is not `type[@kinds][!fillers]`")
            continue
        slot["type"] = m.group(1)
        slot["kinds"] = tuple(m.group(2).split(",")) if m.group(2) else ()
        slot["without"] = tuple(m.group(3).split(",")) if m.group(3) else ()
        slot["line"] = nr
        slots.append(slot)
    return slots, problems


def load_fillers(path=FILLERS_TSV):
    fillers, problems = [], []
    for nr, cols in rows(path):
        if len(cols) not in (3, 4):
            problems.append(f"{os.path.basename(path)}:{nr}: {len(cols)} columns, want 3 or 4")
            continue
        only = cols[3] if len(cols) == 4 else ""
        if only not in ("", "first"):
            problems.append(f"{os.path.basename(path)}:{nr}: the fourth column is `first` or absent")
        fillers.append({"id": cols[0], "type": cols[1], "expr": cols[2], "only": only, "line": nr})
    return fillers, problems


def load_outside(path=OUTSIDE_TSV):
    outside, problems = {}, []
    for nr, cols in rows(path):
        if len(cols) != 2 or not cols[1].startswith(OUTSIDE_CLASSES):
            problems.append(f"{os.path.basename(path)}:{nr}: want `lexeme<TAB>class: why`, "
                            f"the class one of {' '.join(OUTSIDE_CLASSES)}")
            continue
        if cols[0] in outside:
            problems.append(f"{os.path.basename(path)}:{nr}: `{cols[0]}` listed twice")
        outside[cols[0]] = cols[1]
    return outside, problems


def lexicon():
    """Every lexeme loft's lexer and parser know: the lexer's TOKENS and KEYWORDS tables,
    and every literal the parser asks the lexer for (`has_token`, `has_keyword` and their
    `peek_` forms), which is where a contextual keyword such as `par` or `index` lives."""
    found = set()
    lexer = open(os.path.join(ROOT, "src", "lexer.rs"), encoding="utf-8").read()
    for table in ("TOKENS", "KEYWORDS"):
        m = re.search(r"static %s: &\[&str\] = &\[(.*?)\];" % table, lexer, re.S)
        if not m:
            raise SystemExit(f"slot_matrix: src/lexer.rs has no `{table}` table any more")
        found.update(re.findall(r'"([^"]+)"', m.group(1)))
    ask = re.compile(r'\b(?:has|peek)_(?:token|keyword)\(\s*"([^"]+)"\s*\)')
    parser_dir = os.path.join(ROOT, "src", "parser")
    for name in sorted(os.listdir(parser_dir)):
        if name.endswith(".rs"):
            found.update(ask.findall(open(os.path.join(parser_dir, name), encoding="utf-8").read()))
    return found


def tokens(text, symbols):
    """The lexemes of a template: words, and the longest symbol at each position."""
    out, at = set(), 0
    text = HOLE.sub(" ", text.replace(SLOT, " ").replace(COUNTER, " c "))
    while at < len(text):
        m = re.match(r"[A-Za-z_][A-Za-z0-9_]*", text[at:])
        if m:
            out.add(m.group(0))
            at += len(m.group(0))
            continue
        for sym in symbols:
            if text.startswith(sym, at):
                out.add(sym)
                at += len(sym)
                break
        else:
            at += 1
    return out


def defined_rules():
    sys.path.insert(0, HERE)
    import rule_tags
    return set(rule_tags.defined_rules())


def check(slots, fillers, outside, lex, rules, problems):
    """Every problem with the registers, as lines.  Pure: `selftest` calls it on edited copies."""
    problems = list(problems)
    types = types_of(fillers)
    seen = set()
    for s in slots:
        where = f"syntax_slots.tsv:{s['line']}: {s['id']}"
        if s["id"] in seen:
            problems.append(f"{where}: the id is used twice")
        seen.add(s["id"])
        if s["family"] not in FAMILIES:
            problems.append(f"{where}: unknown family `{s['family']}`")
        if s["count"] not in COUNTS:
            problems.append(f"{where}: unknown count class `{s['count']}`")
        if s["type"] not in types:
            problems.append(f"{where}: type `{s['type']}` has no `{s['type']}-var` row in syntax_fillers.tsv")
        if not s["runs"].isdigit():
            problems.append(f"{where}: runs `{s['runs']}` is not a number")
        elif s["count"] in ONCE and s["runs"] != "1":
            problems.append(f"{where}: count `once` with runs {s['runs']}")
        elif s["count"] == "cond" and s["runs"] not in ("0", "1"):
            problems.append(f"{where}: count `cond` with runs {s['runs']}")
        if s["rule"] != "-" and s["rule"] not in rules:
            problems.append(f"{where}: rule `{s['rule']}` is not defined in doc/claude/formal/")
        for k in s.get("kinds", ()):
            if k not in [name for name, _ in KINDS]:
                problems.append(f"{where}: unknown collection kind `{k}`")
        for f in s.get("without", ()):
            if f not in {x["id"] for x in fillers}:
                problems.append(f"{where}: `!{f}` names no filler")
        if s["decl"].count(BIND) > 1 or BIND in s["stmt"]:
            problems.append(f"{where}: `{BIND}` stands at most once, and in the declaration")
        if (s["stmt"] + s["decl"]).count(SLOT) != 1:
            problems.append(f"{where}: the statement and declaration hold `{SLOT}` "
                            f"{(s['stmt'] + s['decl']).count(SLOT)} times, want once")
    seen, plain = set(), set()
    for f in fillers:
        where = f"syntax_fillers.tsv:{f['line']}: {f['id']}"
        if f["id"] in seen:
            problems.append(f"{where}: the id is used twice")
        seen.add(f["id"])
        is_var = f["id"] == f["type"] + "-var"
        holes = HOLE.findall(f["expr"])
        if holes:
            if len(holes) != 1 or COUNTER in f["expr"] or holes[0] not in types:
                problems.append(f"{where}: a wrapper has one `<I:type>` hole of a known type and no `{COUNTER}`")
        elif is_var == (COUNTER in f["expr"]):
            problems.append(f"{where}: a `var` filler takes no `{COUNTER}`, every other one does")
        if f["type"] not in types:
            problems.append(f"{where}: type `{f['type']}` has no `{f['type']}-var` row")
    tried = neutral(slots, fillers)
    for t in types:
        if sum(1 for f in fillers if f["type"] == t and not HOLE.search(f["expr"])) < 2:
            problems.append(f"syntax_fillers.tsv: type `{t}` has no filler but its variable")
        if not any(s["type"] == t for s in slots):
            problems.append(f"syntax_fillers.tsv: type `{t}` has fillers and no slot takes it")
        elif t not in tried:
            problems.append(f"syntax_slots.tsv: type `{t}` has no slot that reads it once in `main`, "
                            f"so `fillers` cannot try its fillers")
    symbols = sorted((x for x in lex if not re.match(r"[A-Za-z_]", x)), key=len, reverse=True)
    used = set()
    for s in slots:
        used |= tokens(s["stmt"] + " " + s["decl"], symbols)
    for f in fillers:
        used |= tokens(f["expr"], symbols)
    for x in sorted(lex - used - set(outside)):
        problems.append(f"lexeme `{x}` is in no slot, no filler and not in syntax_outside.tsv: "
                        f"add the slot row for the operand it takes, or the line saying it takes none")
    for x in sorted(set(outside) - lex):
        problems.append(f"syntax_outside.tsv: `{x}` is not a lexeme the lexer or the parser knows any more")
    for x in sorted(set(outside) & used):
        problems.append(f"syntax_outside.tsv: `{x}` is used by a slot or a filler, so it is not outside")
    return problems



def load_contexts(path=CONTEXTS_TSV):
    """Where a cell's statement stands: [(id, passes, template)], the first the default."""
    out = []
    for nr, cols in rows(path):
        if len(cols) != 3 or not cols[1].isdigit() or cols[2].count("<STMT>") != 1:
            raise SystemExit(f"{os.path.basename(path)}:{nr}: want `id<TAB>passes<TAB>template with one <STMT>`")
        out.append((cols[0], int(cols[1]), cols[2]))
    return out


def world_env():
    """`(parameters, arguments)` of the prelude's world: what a function declared by a slot
    takes (`<ENV>`) and is called with (`<ARGS>`), read from the `params` frame so that the
    two cannot drift from the world itself."""
    text = open(PRELUDE, encoding="utf-8").read()
    params = re.search(r"^fn CELLFN_in\((.*)\) \{$", text, re.M)
    args = re.search(r"^\s*CELLFN_in\((.*)\);$", text, re.M)
    if not params or not args:
        raise SystemExit("slot_matrix: the prelude's `params` frame no longer declares `CELLFN_in(…)`")
    return params.group(1), args.group(1)


def world_names(text, word, names):
    """`<ENV>` / `<ARGS>` in a slot's text, as the world's parameters or arguments.
    `<ENV -tk>` leaves the named ones out and `<ENV: c h i>` keeps only the named ones, for
    a declaration that cannot take the whole world."""
    def fill(m):
        only = set((m.group(1) or "").split())
        drop = set((m.group(2) or "").replace("-", " ").split())
        parts = re.split(r",\s*(?=[a-z_0-9]+\b(?::|,|$))", names)
        name = lambda p: p.split(":")[0].strip()
        return ", ".join(p for p in parts if name(p) not in drop and (not only or name(p) in only))
    return re.sub(r"<%s(?:: ([\w ]+))?((?: -\w+)*)>" % word, fill, text)


def load_frames():
    """The prelude's head, and its frames by name: the function a cell's statement stands in."""
    parts = re.split(r"^// === CELL (\w+) ===\n", open(PRELUDE, encoding="utf-8").read(), flags=re.M)
    return parts[0], dict(zip(parts[1::2], parts[2::2]))



def expand(fillers, depth):
    """The fillers of a run: the plain rows, and at depth two every wrapper once per filler
    of its hole's type.  A wrapper never fills a wrapper, and a variable fills no hole: the
    filler in the hole is what counts."""
    plain = [f for f in fillers if not HOLE.search(f["expr"])]
    if depth < 2:
        return plain
    out = list(plain)
    for w in fillers:
        hole = HOLE.search(w["expr"])
        if not hole:
            continue
        for f in plain:
            if f["type"] == hole.group(1) and not f["id"].endswith("-var"):
                out.append({"id": f"{w['id']}({f['id']})", "type": w["type"], "line": w["line"],
                            "only": f.get("only", ""),
                            "expr": w["expr"].replace(hole.group(0), f["expr"])})
    return out



def load_expect(path=EXPECT_TSV):
    """The anchor: {(slot id, kind or `*`): {field: value}} — the fields of the cell's final
    line that the slot's PLAIN statement must leave changed in the prelude's world."""
    out = {}
    if os.path.exists(path):
        for nr, cols in rows(path):
            if len(cols) != 3:
                raise SystemExit(f"{os.path.basename(path)}:{nr}: want `slot<TAB>kind or *<TAB>field=value ; …`")
            out[(cols[0], cols[1])] = fields_of(cols[2])
    return out


def fields_of(dump):
    """A final line's fields: `name=value` parted by ` ; `.  `-` is no field at all."""
    out = {}
    for part in dump.split(" ; "):
        name, eq, value = part.partition("=")
        if eq:
            out[name.strip()] = value          # the value as printed: a width is part of it
    return out


def load_all():
    slots, p1 = load_slots()
    fillers, p2 = load_fillers()
    outside, p3 = load_outside()
    return slots, fillers, outside, p1 + p2 + p3


def cmd_check(_args):
    slots, fillers, outside, problems = load_all()
    problems = check(slots, fillers, outside, lexicon(), defined_rules(), problems)
    for p in problems:
        print(p)
    if problems:
        print(f"slot_matrix check: {len(problems)} problem(s)")
        return 1
    return 0


def cmd_selftest(_args):
    slots, fillers, outside, problems = load_all()
    lex, rules = lexicon(), defined_rules()
    base = check(slots, fillers, outside, lex, rules, problems)
    if base:
        print("slot_matrix selftest: `check` is not clean, so nothing can be proven against it")
        return 1
    failed = []

    def want(name, got, needle):
        if not any(needle in line for line in got):
            failed.append(f"{name}: `check` did not report `{needle}`")

    without = [s for s in slots if s["id"] != "wr-mod-index"]
    want("a slot row removed", check(without, fillers, outside, lex, rules, []), "lexeme `%=`")
    want("a lexeme added", check(slots, fillers, outside, lex | {"unless"}, rules, []),
         "lexeme `unless`")
    wrong = [dict(s, rule="E-Lefty") if s["id"] == "op-add-left" else s for s in slots]
    want("a rule misspelled", check(wrong, fillers, outside, lex, rules, []), "rule `E-Lefty`")
    stale = dict(outside, **{"+": "claimed to take no operand"})
    want("a used lexeme listed outside", check(slots, fillers, stale, lex, rules, []),
         "`+` is used by a slot")
    twice = [dict(s, stmt="s = <S> + <S>;") if s["id"] == "op-add-left" else s for s in slots]
    want("a slot marked twice", check(twice, fillers, outside, lex, rules, []), "2 times")
    for line in failed:
        print(line)
    return 1 if failed else 0


def cmd_list(_args):
    slots, fillers, outside, _ = load_all()
    for fam in FAMILIES:
        mine = [s for s in slots if s["family"] == fam]
        by = {c: sum(1 for s in mine if s["count"] == c) for c in COUNTS}
        print(f"{fam:10} {len(mine):3} slots   once {by['once'] + by['first']:3} (first {by['first']:2})  cond {by['cond']:2}  step {by['step']:2}")
    owed = [s["id"] for s in slots if s["rule"] == "-"]
    print(f"{'total':10} {len(slots):3} slots, {len(fillers)} fillers, {len(outside)} lexemes outside")
    print(f"no rule states the count of {len(owed)}: {' '.join(owed)}")
    unreached = sorted(x for x, why in outside.items() if why.startswith("unreached:"))
    print(f"an operand the matrix cannot reach yet, {len(unreached)}: {' '.join(unreached)}")
    return 0



def statement_of(cell):
    """The loft a cell runs, on one line: its declaration if it has one, then its statement."""
    return (cell["decl"].strip() + "   " if cell["decl"].strip() else "") + cell["stmt"].strip()


_BUILD = {}


def build():
    """The compiler a run used: its path and what it says it is."""
    path = loft_binary()
    if path not in _BUILD:
        try:
            said = subprocess.run([path, "--version"], capture_output=True, text=True, check=False).stdout
            _BUILD[path] = f"{path}, {said.splitlines()[0].strip()}" if said.strip() else path
        except OSError:
            _BUILD[path] = path
    return _BUILD[path]


def loft_binary():
    if os.environ.get("LOFT"):
        return os.environ["LOFT"]
    built = os.path.join(ROOT, "target", "release", "loft")
    return built if os.path.exists(built) else "loft"


# The plain variables of the prelude that an ORDER cell replaces by a call writing a letter.
ORDER_CALLS = {"i": ("oi(c)", "i"), "j": ("oj(c)", "j"), "b": ("ob(c)", "b"),
               "nb": ("onb(c)", "n"), "x": ("ox(c)", "x"), "fl": ("ofl(c)", "f"),
               "en": ("oen(c)", "e")}
ORDER_NAME = re.compile(r"<S>|(?<![.\w])(%s)(?![\w(])" % "|".join(sorted(ORDER_CALLS, key=len, reverse=True)))
# A statement with one of these runs an operand conditionally or more than once, so source
# order is not the order its operands run in; no ORDER cell is made for it.
ORDER_BARRED = re.compile(r"\b(for|while|if|match|and|or|par|parallel|fn|assert)\b|&&|\|\||\?")
CHUNK = 120


def ordered(stmt):
    """The statement with every plain variable replaced by its lettered call, and the log
    source order gives — or None when order says nothing about this statement."""
    if ORDER_BARRED.search(stmt):
        return None
    out, log = [], []
    for n, part in enumerate(re.split(r'("(?:[^"\\]|\\.)*")', stmt)):
        if n % 2:                       # a string: its holes are left as they are
            log.append("S" * part.count(SLOT))
            out.append(part)
            continue

        def swap(m):
            if m.group(0) == SLOT:
                log.append("S")
                return SLOT
            call, letter = ORDER_CALLS[m.group(1)]
            log.append(letter)
            return call
        out.append(ORDER_NAME.sub(swap, part))
    log = "".join(log)
    return ("".join(out), log) if len(log) > 1 else None


def make_cells(slots, fillers, which_filler, which_slot, which_kind, contexts=None, frames=None, depth=1,
               against_plain=False):
    """The cells of a run.  A slot's plain-variable cell is always there: it is what the
    value of every other cell of that slot is compared with.  `contexts` and `frames` are
    the ones to generate for; left out, the first of each."""
    contexts = contexts or load_contexts()[:1]
    frames = frames or list(load_frames()[1])[:1]
    out, seen_worlds, expect = [], set(), load_expect()
    env, env_args = world_env()
    slots = [dict(s, decl=world_names(s["decl"], "ENV", env), stmt=world_names(s["stmt"], "ARGS", env_args))
             for s in slots]
    fillers = expand(fillers, depth)
    kinded_types = kinded(fillers)
    for slot in slots:
        if which_slot and slot["id"] not in which_slot:
            continue
        # Every kind where the operand's type depends on it, and where the statement itself
        # reads a collection of the cell's kind (a keyed lookup by an integer operand).
        kinds = (KINDS if slot["type"] in kinded_types or KIND_NAMES.search(slot["stmt"] + slot["decl"])
                 else KINDS[:1])
        for (kind, kind_type), (ctx, passes, wrap), frame in (
                (k, c, f) for k in kinds for c in contexts for f in frames):
            if (which_kind and kind != which_kind) or (slot.get("kinds") and kind not in slot["kinds"]):
                continue
            where = f"{slot['id']}.%s.{kind}" + ("" if (ctx, frame) == (contexts[0][0], frames[0]) and len(contexts) + len(frames) == 2 else f"@{ctx}/{frame}")
            base = where % f"{slot['type']}-var"
            world = f"_world.{kind}@{ctx}/{frame}"
            if world not in seen_worlds:
                seen_worlds.add(world)
                out.append({"name": world, "slot": "_world", "frame": frame, "kind_type": kind_type,
                            "decl": "", "stmt": wrap.replace("<STMT>", ""), "base": world, "want": 0,
                            "log": None})
            want_fields = expect.get((slot["id"], kind), expect.get((slot["id"], "*")))
            for f in fillers:
                if f["type"] != slot["type"]:
                    continue
                is_var = f["id"].endswith("-var")
                if not is_var and which_filler != "all" and f["id"] != which_filler:
                    continue
                if f.get("only") == "first" and slot["count"] != "first":
                    continue
                if f["id"] in slot.get("without", ()):
                    continue
                expr = f["expr"].replace(COUNTER, "c")
                put = lambda stmt: wrap.replace("<STMT>", stmt)
                cell = {"name": where % f["id"], "slot": slot["id"], "frame": frame,
                        "kind_type": kind_type,
                        "decl": slot["decl"].replace(SLOT, expr).replace(BIND, ""),
                        "stmt": put(slot["stmt"].replace(SLOT, expr)), "base": base,
                        "want": 0 if is_var else int(slot["runs"]) * passes, "log": None}
                if is_var and passes == 1:
                    cell["world"], cell["anchor"] = world, want_fields
                out.append(cell)
                # The bound-first twin: the filler bound to a local, then the statement
                # with that local.  A local is evaluated once by construction, so the twin
                # says what the statement must leave behind.  Only where the operand runs
                # exactly once and sits in the frame; elsewhere the plain variable says it.
                # `against_plain` asks the other question — does the FILLER give its type's
                # plain value — so there the plain variable is what the cell is compared with.
                if (not against_plain and not is_var and slot["count"] in ONCE
                        and (not slot["decl"] or BIND in slot["decl"])):
                    cell["base"] = cell["name"] + ".twin"
                    bind = f"t0 = {expr}; "
                    out.append(dict(
                        cell, name=cell["base"], base=cell["base"], want=passes,
                        decl=slot["decl"].replace(BIND, bind).replace(SLOT, "t0"),
                        stmt=put(("" if slot["decl"] else bind) + slot["stmt"].replace(SLOT, "t0"))))
                silent = ".calls += 1" in f["expr"] and ".log" not in f["expr"]
                turn = (None if is_var or silent or slot["decl"] or slot["count"] not in ONCE
                        else ordered(slot["stmt"]))
                if turn:
                    out.append(dict(cell, name=cell["name"] + ".order",
                                    stmt=put(turn[0].replace(SLOT, expr)), log=turn[1] * passes))
    return out


def program(kind_type, cells):
    """One loft program holding `cells` as test functions; and, per cell, its function name
    and the lines it occupies."""
    head, frames = load_frames()
    text = head.replace("KIND", kind_type)
    spans = []
    for n, cell in enumerate(cells):
        fn = f"test_c{n:04d}"
        # Every function a cell declares is its own: the name gets the cell's number, in
        # the declaration and where the statement calls it.
        declared = re.findall(r"\bfn\s+([A-Za-z_]\w*)\s*\(", cell["decl"])
        rename = lambda t, n=n, d=declared: (
            re.sub(r"\b(%s)\b" % "|".join(map(re.escape, d)), lambda m: f"{m.group(1)}_{n}", t) if d else t)
        body = frames[cell.get("frame") or next(iter(frames))]
        part = (body.replace("CELLDECL", rename(cell["decl"])).replace("CELLFN", fn)
                .replace("CELLSTMT", rename(cell["stmt"])).replace("CELLNAME", cell["name"])
                .replace("KIND", kind_type))
        first = text.count("\n") + 1
        text += part
        spans.append((first, text.count("\n"), fn))
    return text, spans


# loft's own time limit for one generated program, in seconds (`LOFT_TIMEOUT`,
# RUN_BOUNDS.md § Execution timeout).  A cell that LOOPS would otherwise hold its whole
# batch, and the run, for as long as it likes.  loft stops it itself and says where it was:
# the interpreter ends the program naming the line, the C backend fails that one test and
# goes on.  No outside timer: the compiler is the one that can name the cell.
TIME_LIMIT = {"interp": int(os.environ.get("SLOT_MATRIX_INTERP_SECONDS", "40")),
              "c": int(os.environ.get("SLOT_MATRIX_C_SECONDS", "60"))}
# Once a program has had a cell stopped by the limit, what is left of it runs under this
# one: a healthy cell answers in milliseconds, and a statement that loops for one form of
# operand often loops for the next, each costing a whole limit.
TIGHT_LIMIT = {"interp": 6, "c": 20}
TABLE_FAULT = re.compile(r"Unknown (variable|function)|Undefined type")
# What a run cost: per backend, how many programs were run and the seconds they took,
# summed over the threads that ran them (so more than the wall time).
SPENT = {"interp": [0, 0.0], "c": [0, 0.0]}
SPENT_LOCK = threading.Lock()


def invoke(backend, path, limit=None):
    cmd = [loft_binary()] + (["--native-c"] if backend == "c" else []) + ["--no-warnings", "--tests", path]
    env = dict(os.environ, LOFT_LEAK_CHECK="1", LOFT_TIMEOUT=str(limit or TIME_LIMIT[backend]))
    began = time.monotonic()
    done = subprocess.run(cmd, capture_output=True, text=True, env=env, check=False)
    with SPENT_LOCK:
        SPENT[backend][0] += 1
        SPENT[backend][1] += time.monotonic() - began
    return done.stdout + done.stderr


def run_program(backend, cells, tmp, tag):
    """Run `cells` (all of one collection kind) as one program.  Returns, per cell name,
    ("REFUSED", message) | ("LOST", last output) | ("RAN", count, log, dump, test failure)."""
    results, pending, turn, limit = {}, list(cells), 0, TIME_LIMIT[backend]
    while pending:
        turn += 1
        text, spans = program(pending[0]["kind_type"], pending)
        path = os.path.join(tmp, f"{tag}_{turn}.loft")
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)
        out = invoke(backend, path, limit)
        refused = {}
        for message, line in re.findall(r"^\s*Error(?:\[[\w-]+\])?: (.*) at \S+?:(\d+):\d+\s*$", out, re.M):
            for cell, (first, last, _) in zip(pending, spans):
                if first <= int(line) <= last:
                    refused.setdefault(cell["name"], message)
        if refused:
            for name, message in refused.items():
                results[name] = ("REFUSED", message)
            pending = [c for c in pending if c["name"] not in refused]
            continue
        # The interpreter's time limit ends the whole program and names the line it was
        # on: that cell does not finish, and the others are run again without it.
        late = re.search(r"^\[timeout\].*?\bfn=(\S+) file=\S+?:(\d+)", out, re.M)
        if late:
            stuck = [cell for cell, (first, last, fn) in zip(pending, spans)
                     if fn == late.group(1) or first <= int(late.group(2)) <= last]
            if stuck:
                results[stuck[0]["name"]] = ("LOST", f"it does not finish: loft's time limit "
                                             f"({limit} s) ended it — the statement loops")
                pending = [c for c in pending if c["name"] != stuck[0]["name"]]
                limit = TIGHT_LIMIT[backend]
                continue
        failed = dict(re.findall(r"^\s*FAIL\s+\S+::(test_c\d+)\s+—\s+(.*)$", out, re.M))
        for fn, why in failed.items():
            if "exit 124" in why:               # the C backend's form of the same limit
                failed[fn] = (f"it does not finish: loft's time limit ({limit} s) "
                              f"ended it — the statement loops")
        lost = []
        for cell, (_, _, fn) in zip(pending, spans):
            m = re.search(r"^%s c=(\d+) log=<(.*?)> \| (.*)$" % re.escape(cell["name"]), out, re.M)
            if m:
                results[cell["name"]] = ("RAN", int(m.group(1)), m.group(2), m.group(3), failed.get(fn))
            elif fn in failed:
                results[cell["name"]] = ("LOST", failed[fn])
            else:
                lost.append(cell)
        if len(lost) == 1 and len(pending) == 1:
            tail = [ln for ln in out.splitlines() if ln.strip()][-3:]
            why = " / ".join(tail)[:300] or "no output"
            results[lost[0]["name"]] = ("LOST", why)
        elif lost:                      # something took the program down: halve to find it
            half = (len(lost) + 1) // 2
            for n, part in enumerate((lost[:half], lost[half:])):
                if part:
                    results.update(run_program(backend, part, tmp, f"{tag}_{turn}{'ab'[n]}"))
        break
    return results


def run_backend(backend, cells, tmp):
    chunks = []
    for kind_type in sorted({c["kind_type"] for c in cells}):
        mine = [c for c in cells if c["kind_type"] == kind_type]
        chunks += [mine[n:n + CHUNK] for n in range(0, len(mine), CHUNK)]
    results = {}
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(2, (os.cpu_count() or 4) // 2)) as pool:
        futures = [pool.submit(run_program, backend, chunk, tmp, f"{backend}{n}")
                   for n, chunk in enumerate(chunks)]
        for f in futures:
            results.update(f.result())
    return results


def verdict(cell, results):
    """(verdict, reason) of one cell on one backend: ok | FAIL | REFUSED."""
    got = results.get(cell["name"])
    if got is None:
        return "FAIL", "the cell was not run"
    if got[0] == "REFUSED":
        # A name the cell's world does not have is a fault of the TABLES, not a refusal of
        # the language: the cell checks nothing, and listing it would hide that.
        if TABLE_FAULT.search(got[1]):
            return "FAIL", f"the tables name something the cell's world lacks: {got[1]}"
        return "REFUSED", got[1]
    if got[0] == "LOST":
        return "FAIL", f"the cell did not finish: {got[1]}"
    _, count, log, dump, failure = got
    if count != cell["want"]:
        return "FAIL", f"the operand ran {count} time(s), the slot says {cell['want']}"
    if cell["log"] is not None and log != cell["log"]:
        return "FAIL", f"the operands ran in the order {log}, the source order is {cell['log']}"
    base = results.get(cell["base"])
    if base and base[0] == "RAN" and base[3] != dump:
        how = "its operand bound to a local first" if cell["base"].endswith(".twin") else "a plain variable"
        got, want = fields_of(dump), fields_of(base[3])
        off = [k for k in got if got[k] != want.get(k)] or ["the final line"]
        k = off[0]
        return "FAIL", (f"the result differs from the same statement with {how}: it leaves "
                        f"{k}={got.get(k, dump)[:70]}, that one {k}={want.get(k, base[3])[:70]}"
                        + (f" (and {len(off) - 1} more field(s))" if len(off) > 1 else ""))
    if failure:
        return "FAIL", ("a store is left behind: " if "not freed" in failure else "") + failure
    if cell.get("anchor") is not None:
        world = results.get(cell["world"])
        if world and world[0] == "RAN":
            want = dict(fields_of(world[3]), **cell["anchor"])
            got = fields_of(dump)
            # `views` is the views frame's own field: what a view shows after the
            # statement is not part of the statement's expectation.
            off = sorted(k for k in (set(want) | set(got)) - {"views"} if want.get(k) != got.get(k))
            if off:
                k = off[0]
                return "FAIL", (f"the plain statement leaves {k}={got.get(k)}, the register says "
                                f"{k}={want.get(k)}" + (f" (and {len(off) - 1} more field(s))" if len(off) > 1 else ""))
    return "ok", dump


def prelude_problem(cells, backend, tmp):
    """The compiler's message when the prelude itself does not compile for a kind — which
    would otherwise read as every cell of that kind being refused."""
    for kind_type, frame in sorted({(c["kind_type"], c["frame"]) for c in cells}):
        probe = {"name": "prelude", "decl": "", "stmt": "s = 0;", "kind_type": kind_type, "frame": frame}
        got = run_program(backend, [probe], tmp, "prelude").get("prelude", ("LOST", "no output"))
        if got[0] != "RAN":
            return f"the prelude's `{frame}` frame does not compile for {kind_type} on {backend}: {got[1]}"
    return None


def evaluate(cells, backends, on_c=None):
    """Run the cells.  Returns {cell name: (verdict, [(backend, verdict, reason)])}, the
    verdict the worst over the backends: ok | FAIL | REFUSED."""
    with tempfile.TemporaryDirectory(prefix="slot_matrix.") as tmp:
        broken = prelude_problem(cells, "interp", tmp)
        if broken:
            raise SystemExit(f"slot_matrix: {broken}")
        interp = run_backend("interp", cells, tmp)
        per = {"interp": interp}
        if "c" in backends:
            accepted = [c for c in cells if interp.get(c["name"], ("",))[0] != "REFUSED"
                        and (on_c is None or c["name"] in on_c)]
            per["c"] = run_backend("c", accepted, tmp)
    out = {}
    for cell in cells:
        lines, worst = [], "ok"
        for backend in per:
            if backend == "c" and (worst == "REFUSED" or (on_c is not None and cell["name"] not in on_c)):
                continue
            v, why = verdict(cell, per[backend])
            if backend == "c" and v == "REFUSED":
                v, why = "FAIL", f"only the C backend refuses it: {why}"
            if v != "ok":
                lines.append((backend, v, why))
                worst = v if worst == "ok" or v == "FAIL" else worst
        out[cell["name"]] = (worst, lines)
    return out


def channel(why):
    """Which of the four assertions a failure is: count | order | value | leak | crash."""
    for word, name in (("time(s)", "count"), ("in the order", "order"), ("result differs", "value"),
                       ("the register says", "anchor"), ("the cell's world lacks", "table"),
                       ("left behind", "leak"), ("freed twice", "leak")):
        if word in why:
            return name
    return "crash"


def causes(cells, verdicts, universe=None):
    """The failing cells folded toward causes.  A slot that fails with most of its fillers
    is one cause in the SLOT's lowering; what is left is folded by filler, a form of
    expression that fails in several slots being one cause in how that FORM is lowered."""
    by_slot, by_filler, total_slot = {}, {}, {}

    def place(cell):
        slot, filler = cell["name"].split(".")[:2]
        spot = re.search(r"@[\w/]+", cell["name"])
        return slot + (spot.group(0) if spot else ""), filler

    # "Most of its fillers" is counted over the whole run, also when only some of its
    # cells are being folded (the gate's new ones, a walk's group).
    for cell in universe or cells:
        slot, filler = place(cell)
        total_slot.setdefault(slot, set()).add(filler)
    for cell in cells:
        slot, filler = place(cell)
        worst, lines = verdicts[cell["name"]]
        if worst == "FAIL":
            ch = "+".join(sorted({channel(why) for _, v, why in lines if v == "FAIL"}))
            by_slot.setdefault((slot, ch), set()).add(filler)
    spots = sorted({(re.search(r"@[\w/]+", c["name"]) or [""])[0] for c in cells})
    lines, left = {}, {}

    def note(text, slot):
        bare, _, spot = slot.partition("@")
        lines.setdefault(text, set()).add("@" + spot if spot else "")
        return bare

    for (slot, ch), fillers in sorted(by_slot.items()):
        bare = slot.partition("@")[0]
        if len(fillers) * 2 >= len(total_slot[slot]) and len(fillers) > 1:
            note(f"slot {bare}: most fillers ({ch})", slot)
        else:
            for f in fillers:
                left.setdefault((f, ch), set()).add(slot)
    # Depth two: a wrapper around a filler that fails the same way in the same slots is
    # that filler's failure seen again, and a wrapper that fails with most of what stands
    # in its hole is one cause in the WRAPPER's form.
    inners = {}
    for cell in cells:
        m = re.match(r"[^.]+\.([\w-]+)\(([\w-]+)\)", cell["name"])
        if m:
            inners.setdefault(m.group(1), set()).add(m.group(2))
    wrapped, echoes = {}, {}
    for (filler, ch), slots in list(left.items()):
        m = re.fullmatch(r"([\w-]+)\(([\w-]+)\)", filler)
        if not m:
            continue
        if slots <= left.get((m.group(2), ch), set()):
            echoes[(m.group(2), ch)] = echoes.get((m.group(2), ch), 0) + 1
            del left[(filler, ch)]
        else:
            wrapped.setdefault((m.group(1), ch), {})[m.group(2)] = slots
    for (wrapper, ch), per in sorted(wrapped.items()):
        if len(per) * 2 >= len(inners.get(wrapper, per)) and len(per) > 1:
            slots = set().union(*per.values())
            for inner in per:
                del left[(f"{wrapper}({inner})", ch)]
            left[(f"wrapper {wrapper} around most fillers", ch)] = slots
    for (filler, ch), slots in sorted(left.items()):
        bare = sorted({x.partition("@")[0] for x in slots})
        seen = echoes.get((filler, ch))
        label = filler if filler.startswith("wrapper ") else f"filler {filler}"
        text = (f"{label}: {len(bare)} slot(s) ({ch}) — " + ", ".join(bare[:6]) + (" …" if len(bare) > 6 else "")
                + (f"  (and inside {seen} wrapper(s))" if seen else ""))
        for x in slots:
            note(text, x)
    out = []
    for text, where in sorted(lines.items()):
        if len(spots) > 1:
            text += "  [everywhere]" if len(where) == len(spots) else "  [" + " ".join(sorted(w.lstrip("@") for w in where)) + "]"
        out.append(text)
    return out


def report(cells, verdicts, backends, verbose):
    """Print what is not ok.  Returns the number of failing cells and vacuous slots."""
    tally = {"ok": 0, "FAIL": 0, "REFUSED": 0}
    accepted = {}
    cells = [c for c in cells if c["slot"] != "_world"]
    for cell in cells:
        worst, lines = verdicts[cell["name"]]
        tally[worst] += 1
        accepted[cell["slot"]] = accepted.get(cell["slot"], 0) + (worst != "REFUSED")
        for backend, v, why in lines:
            if v == "FAIL" or verbose:
                print(f"{v:8} {cell['name']} [{backend}]: {why[:260]}")
        if worst == "FAIL":
            print(f"         {statement_of(cell)}")
        if worst == "ok" and verbose:
            print(f"ok       {cell['name']}")
    vacuous = sorted(s for s, n in accepted.items() if n == 0)
    for s in vacuous:
        print(f"VACUOUS  {s}: no cell of this slot compiled, so it checks nothing")
    found = causes(cells, verdicts)
    if found:
        print(f"the failing cells fold to {len(found)} group(s):")
        for line in found:
            print("  " + line)
    orders = sum(1 for c in cells if c["log"] is not None)
    print(f"slot_matrix: {len(cells)} cells ({orders} check order) on {' and '.join(backends)}: "
          f"{tally['ok']} ok, {tally['FAIL']} failing, {tally['REFUSED']} refused, "
          f"{len(vacuous)} slot(s) vacuous  [{build()}]")
    print("          cost: " + "; ".join(f"{b} {SPENT[b][0]} program run(s), {SPENT[b][1]:.0f} s" for b in backends))
    if tally["FAIL"]:
        print("for one failing cell — its program kept, loft's diagnostics on, its IR beside its twin's: "
              "python3 scripts/slot_matrix.py explain <cell>")
    return tally["FAIL"] + len(vacuous)


def run_matrix(cells, backends, verbose):
    return report(cells, evaluate(cells, backends), backends, verbose)


BASELINE_HEAD = """# The cells of the slot matrix that do not pass today (@PLN221) — the gate's baseline.
# `python3 scripts/slot_matrix.py gate` runs the matrix (every filler, one context, one
# frame, both backends) and fails on a cell that is not ok and not listed here, and on a
# line here whose cell now passes.  So the file only SHRINKS: a fix removes its lines in the
# same change (`gate --update` rewrites the file; read its diff before committing it), and
# a new line is a defect being accepted, which wants a reason in the commit.
# Columns, tab-separated: cell, backend, channel (count | order | value | leak | crash |
# refused).  A line starting with `~` is UNSTABLE — the cell ends in a crash, which need not
# repeat on another machine — and is allowed to pass.
"""


def baseline_keys(cells, verdicts):
    keys = set()
    for cell in cells:
        if cell["slot"] == "_world":
            continue
        worst, lines = verdicts[cell["name"]]
        if worst == "REFUSED":
            keys.add((cell["name"], "-", "refused"))
        for backend, v, why in lines:
            if v == "FAIL":
                keys.add((cell["name"], backend, channel(why)))
    return keys


def load_baseline(path=BASELINE):
    """(stable keys, unstable keys)."""
    stable, unstable = set(), set()
    if os.path.exists(path):
        for _, cols in rows(path):
            (unstable if cols[0].startswith("~") else stable).add((cols[0].lstrip("~"), *cols[1:3]))
    return stable, unstable


def annotate(folds, slots, fillers):
    """On GitHub, one error annotation per fold, on the table row it names — so a PR shows
    the failing slot or filler where it is declared."""
    if not os.environ.get("GITHUB_ACTIONS"):
        return
    rows_of = {("slot", x["id"]): ("scripts/syntax_slots.tsv", x["line"]) for x in slots}
    rows_of.update({("filler", x["id"]): ("scripts/syntax_fillers.tsv", x["line"]) for x in fillers})
    for fold in folds:
        m = re.match(r"(slot|filler|wrapper) ([\w-]+)", fold)
        hit = m and rows_of.get(("filler" if m.group(1) == "wrapper" else m.group(1), m.group(2)))
        if hit:
            print(f"::error file={hit[0]},line={hit[1]},title=slot matrix::{fold}")


def cmd_gate(args):
    slots, fillers, outside, problems = load_all()
    problems = check(slots, fillers, outside, lexicon(), defined_rules(), problems)
    if problems:
        print("\n".join(problems))
        return 1
    if args.update and args.family:
        print("slot_matrix gate: `--update` rewrites the whole baseline; run it without `--family`")
        return 1
    family = {s["id"]: s["family"] for s in slots}
    mine = {s["id"] for s in slots if not args.family or s["family"] == args.family}
    cells = make_cells(slots, fillers, "all", mine, None)
    full = args.full or args.update
    # The C backend compiles every cell, about a tenth of a second each, which is most of a
    # run.  Unless the whole matrix is asked for, it gets a sample: every slot with its
    # plain variable and with the first filler of its type (and their twins and order cells).
    first_of = {}
    for f in fillers:
        if not f["id"].endswith("-var") and not HOLE.search(f["expr"]):
            first_of.setdefault(f["type"], f["id"])
    sampled = lambda name: name.split(".")[1].endswith("-var") or name.split(".")[1] in first_of.values()
    on_c = None if full else {c["name"] for c in cells if c["slot"] == "_world" or sampled(c["name"])}
    verdicts = evaluate(cells, ("interp", "c"), on_c)
    keys = baseline_keys(cells, verdicts)
    if args.update:
        with open(BASELINE, "w", encoding="utf-8") as f:
            f.write(BASELINE_HEAD)
            for key in sorted(keys):
                f.write(("~" if key[2] == "crash" else "") + "\t".join(key) + "\n")
        print(f"slot_matrix gate: baseline rewritten, {len(keys)} line(s)")
        return 0
    stable, unstable = load_baseline()
    here = lambda key: family.get(key[0].split(".")[0]) == args.family if args.family else True
    # A baseline line of a cell this run did not put on the C backend says nothing here.
    ran = lambda key: key[1] != "c" or on_c is None or key[0] in on_c
    stable = {k for k in stable if here(k) and ran(k)}
    unstable = {k for k in unstable if here(k)}
    # A crash may also show as another channel on another machine; an unstable line
    # excuses its cell on its backend whatever the channel.
    excused = {(n, b) for n, b, _ in unstable}
    new = sorted(k for k in keys - stable if (k[0], k[1]) not in excused)
    gone = sorted(stable - keys)
    what = f"the `{args.family}` slots" if args.family else "the slot matrix"
    if not new and not gone:
        print(f"slot_matrix gate: {what}: {len(cells)} cells, {len(keys)} not ok, all in the baseline  [{loft_binary()}]")
        return 0
    total_new = len(new)
    # The two findings want opposite actions, so each is said on its own, the one that
    # means something broke first.
    refused = [k for k in new if k[2] == "refused"]
    new = [k for k in new if k[2] != "refused"]
    if new:
        only = {n for n, _, _ in new}
        folds = causes([c for c in cells if c["name"] in only], verdicts, cells)
        print(f"BROKEN: {len(new)} cell line(s) of {what} fail and are not in the baseline — a change made "
              f"an operand run more than once, out of order, to another result, or leak.  They fold to:")
        for fold in folds:
            print("  " + fold)
        annotate(folds, slots, fillers)
        print("the cells:")
        for name, backend, ch in new:
            why = next((w for b, v, w in verdicts[name][1] if b == backend and v == "FAIL"), "")
            print(f"  NEW      {name} [{backend}] ({ch}): {why[:170]}")
        print(f"  to see one slot: python3 scripts/slot_matrix.py run --filler all --backend both --slot <id> -v")
    if refused:
        # Not a failure of an evaluation: the compiler now refuses a statement it took, or
        # a new row is one it never takes.  Either is a claim the rules must back, so it
        # is listed with the compiler's words and accepted by hand (`gate --update`).
        print(f"REFUSED: {len(refused)} cell(s) of {what} are refused by the compiler and are not in the "
              f"baseline.  Read each refusal against the rules; if it is right, `gate --update` lists it.")
        for name, _, _ in refused:
            why = next((w for _, v, w in verdicts[name][1] if v == "REFUSED"), "")
            print(f"  REFUSED  {name}: {why[:200]}")
            print(f"           {statement_of(next(c for c in cells if c['name'] == name))}")
    if gone:
        print(f"FIXED: {len(gone)} baseline line(s) of {what} pass now.  Nothing is broken: run "
              f"`python3 scripts/slot_matrix.py gate --update`, read the diff (it must only remove "
              f"lines), and commit it with the fix.")
        for name, backend, ch in gone:
            print(f"  PASSES   {name} [{backend}] ({ch})")
    print(f"slot_matrix gate: {what}: {len(cells)} cells, {len(new)} failing and {total_new - len(new)} refused "
          f"that are new, {len(gone)} to remove  [{loft_binary()}]")
    return 1


def cmd_cell(args):
    """One shape, tried at once: a statement with `<S>`, and the operand forms to put in."""
    slots, fillers, _, problems = load_all()
    if problems:
        print("\n".join(problems))
        return 1
    stmt, decl = args.statement, args.decl or ""
    if (stmt + decl).count(SLOT) != 1:
        print(f"slot_matrix cell: the statement and declaration must hold `{SLOT}` exactly once")
        return 1
    mine = []
    if args.filler:
        if args.type or not args.plain:
            print("slot_matrix cell: give `--type T` (every filler of a type), or `--filler EXPR` with "
                  "`--plain EXPR` (the same value without a counter)")
            return 1
        tp = "adhoc"
        mine = [{"id": "adhoc-var", "type": tp, "expr": args.plain, "only": "", "line": 0},
                {"id": "adhoc-new", "type": tp, "expr": args.filler, "only": "", "line": 0}]
    elif args.type in types_of(fillers):
        tp = args.type
        mine = [f for f in fillers if f["type"] == tp]
    else:
        print(f"slot_matrix cell: `--type` is one of {' '.join(types_of(fillers))}")
        return 1
    slot = {"id": args.id, "family": args.family, "rule": args.rule, "count": args.count,
            "runs": str(args.runs), "type": tp, "stmt": stmt, "decl": decl, "line": 0}
    cells = make_cells([slot], mine, "all", set(), args.kind, *axes_of(args))
    backends = backends_of(args)
    failed = report(cells, evaluate(cells, backends), backends, True)
    print("the rows, should this shape join the tables:")
    print("  syntax_slots.tsv:   " + "\t".join(
        [slot["id"], slot["family"], slot["rule"], slot["count"], slot["runs"], tp, stmt] + ([decl] if decl else [])))
    if args.filler:
        print(f"  syntax_fillers.tsv: <type>-<form>\t<type>\t{args.filler}    (and the slot's type "
              f"in place of `adhoc`; its plain variable is `{args.plain}`)")
    return 1 if failed else 0


def cmd_expect(args):
    """The anchor's rows: `--propose` prints, for every slot with no expectation, the fields
    its plain statement changes on THIS build — a proposal to compute by hand and correct,
    never to paste unread: it is the compiler's answer, not the rules'."""
    slots, fillers, _, problems = load_all()
    if problems:
        print("\n".join(problems))
        return 1
    expect = load_expect()
    owed = [s for s in slots if not any(k[0] == s["id"] for k in expect)]
    if not args.propose:
        print(f"slot_matrix expect: {len(slots) - len(owed)} of {len(slots)} slots have an expectation"
              + (f"; none for: {' '.join(s['id'] for s in owed)}" if owed else ""))
        return 1 if owed else 0
    cells = make_cells(owed, fillers, "var", set(), None)
    with tempfile.TemporaryDirectory(prefix="slot_matrix.") as tmp:
        got = run_backend("interp", cells, tmp)
    by_slot = {}
    for cell in cells:
        if cell["slot"] == "_world" or got.get(cell["name"], ("",))[0] != "RAN":
            continue
        world = fields_of(got[cell["world"]][3])
        mine = fields_of(got[cell["name"]][3])
        changed = " ; ".join(f"{k}={v}" for k, v in mine.items() if world.get(k) != v) or "-"
        by_slot.setdefault(cell["slot"], {})[cell["name"].split(".")[2]] = changed
    for slot in owed:
        kinds = by_slot.get(slot["id"], {})
        if len(set(kinds.values())) == 1:
            print(f"{slot['id']}\t*\t{next(iter(kinds.values()))}")
        else:
            for kind, changed in kinds.items():
                print(f"{slot['id']}\t{kind}\t{changed}")
    return 0


def cmd_expr(args):
    """One EXPRESSION, tried wherever it can stand: in every slot of its type, in the
    contexts and frames asked for, and at depth two both ways round."""
    slots, fillers, _, problems = load_all()
    if problems:
        print("\n".join(problems))
        return 1
    types = types_of(fillers)
    if args.type not in types:
        print(f"slot_matrix expr: `--type` is one of {' '.join(types)}")
        return 1
    plain = [f for f in fillers if not HOLE.search(f["expr"])]
    variables = [f for f in plain if f["id"].endswith("-var")]
    mine = {"id": f"{args.type}-new", "type": args.type, "expr": args.expression, "only": "", "line": 0}
    hole = HOLE.search(args.expression)
    if hole:
        # The expression is a wrapper: it stands AROUND every filler of its hole's type.
        if COUNTER in args.expression or hole.group(1) not in types:
            print(f"slot_matrix expr: an expression with a hole takes no `{COUNTER}`, and the hole's "
                  f"type is one of {' '.join(types)}")
            return 1
        tried = [dict(mine, id=f"{mine['id']}({f['id']})", expr=args.expression.replace(hole.group(0), f["expr"]),
                      only=f.get("only", ""))
                 for f in plain if f["type"] == hole.group(1) and not f["id"].endswith("-var")]
    else:
        if COUNTER not in args.expression:
            print(f"slot_matrix expr: the expression must hold `{COUNTER}` where its counted call takes "
                  f"the counter (`at(<C>, 1)`), or a hole `<I:type>` to stand around the other fillers")
            return 1
        tried = [mine]
        if args.depth == 2:
            # ... and INSIDE every wrapper whose hole is of its type.
            for w in fillers:
                wh = HOLE.search(w["expr"])
                if wh and wh.group(1) == args.type:
                    tried.append({"id": f"{w['id']}({mine['id']})", "type": w["type"], "only": "",
                                  "line": 0, "expr": w["expr"].replace(wh.group(0), args.expression)})
    if args.plain:
        variables = [dict(v, expr=args.plain) if v["type"] == args.type else v for v in variables]
    wanted = {f["type"] for f in tried}
    which = set(args.slot) or {s["id"] for s in slots if s["type"] in wanted}
    backends = backends_of(args)
    # First: is it usable at all — one bump of its counter, its type's plain value?
    first = neutral(slots, fillers)
    probe = make_cells(slots, variables + tried, "all", {first[t] for t in wanted if t in first}, KINDS[0][0],
                       against_plain=True)
    print(f"slot_matrix expr: `{args.expression}` as {len(tried)} filler(s) of type "
          f"{', '.join(sorted(wanted))}; first in the neutral slot of each:")
    if report(probe, evaluate(probe, ("interp",)), ("interp",), False):
        print("slot_matrix expr: the expression is not usable as it is — it must run its counter once and "
              "give the same value as its type's plain variable (`--plain EXPR` names another)")
        return 2
    cells = make_cells(slots, variables + tried, "all", which, args.kind, *axes_of(args))
    failed = report(cells, evaluate(cells, backends), backends, args.verbose)
    print("the row, should it join scripts/syntax_fillers.tsv:")
    print(f"  {args.type}-<form>\t{args.type}\t{args.expression}")
    return 1 if failed else 0


def ir_of(text, functions, names=()):
    """From `loft introspect` output, the IR of the named functions, with what differs
    between any two cells for no reason of theirs taken out — their own names, the source
    line each statement came from — so that two cells can be compared line by line."""
    blocks = {}
    for m in re.finditer(r"^fn n_(\w+)\(.*?(?=^fn n_|^byte-code for |\Z)", text, re.S | re.M):
        blocks.setdefault(m.group(1), m.group(0))
    out = []
    for name in functions:
        body = blocks.get(name)
        if body:
            for other in functions:
                body = body.replace(other, "CELL")
            for name in sorted(names, key=len, reverse=True):
                body = body.replace(name, "NAME")
            body = re.sub(r"^(\s*)\[\d+\] ", r"\1", body, flags=re.M)
            out += [ln.rstrip() for ln in body.splitlines()]
    return out


def cmd_explain(args):
    """One failing cell, laid open for whoever debugs it."""
    import difflib
    slots, fillers, _, problems = load_all()
    if problems:
        print("\n".join(problems))
        return 1
    want = re.sub(r"\.(twin|order)$", "", args.cell)
    slot_id = want.split(".")[0]
    wide = "@" in want
    cells = make_cells(slots, fillers, "all", {slot_id}, None,
                       load_contexts() if wide else None, list(load_frames()[1]) if wide else None,
                       depth=2 if "(" in want else 1)
    by = {c["name"]: c for c in cells}
    if want not in by:
        near = [n for n in by if n.startswith(slot_id + ".") and not n.endswith((".twin", ".order"))][:12]
        print(f"slot_matrix explain: no cell `{want}`" + (f"; this slot has: {' '.join(near)} …" if near
              else f"; `{slot_id}` is no slot of scripts/syntax_slots.tsv"))
        return 2
    cell = by[want]
    plain = by.get(re.sub(r"\.[^.@]+\.(\w+)(@[\w/]+)?$",
                          lambda m: f".{next(s['type'] for s in slots if s['id'] == slot_id)}-var.{m.group(1)}{m.group(2) or ''}",
                          want))
    twin, order = by.get(want + ".twin"), by.get(want + ".order")
    world = next((c for c in cells if c["slot"] == "_world" and c["kind_type"] == cell["kind_type"]
                  and c["frame"] == cell["frame"] and c["name"] == (plain or cell).get("world", c["name"])), None)
    group = [c for c in (cell, twin, order, plain, world) if c]
    roles = {id(cell): "the cell", id(twin): "its twin (the operand bound to a local first)",
             id(order): "its order cell", id(plain): "the plain variable", id(world): "the world untouched"}
    keep = os.path.abspath(args.out or tempfile.mkdtemp(prefix="slot_matrix.explain."))
    os.makedirs(keep, exist_ok=True)
    text, spans = program(cell["kind_type"], group)
    path = os.path.join(keep, "cell.loft")
    with open(path, "w", encoding="utf-8") as f:
        f.write("// " + want + " — written by `slot_matrix.py explain`, with what it is compared with.\n")
        for c, (_, _, fn) in zip(group, spans):
            f.write(f"//   {fn}: {roles[id(c)]}\n")
        f.write(text + "fn main() { " + " ".join(fn + "();" for _, _, fn in spans) + " }\n")
    print(f"cell      {want}")
    print(f"build     {build()}")
    print(f"program   {path}   (kept; `loft {path}` runs it, `loft --tests {path}` each function)")
    for c, (_, _, fn) in zip(group, spans):
        if c is not world:
            print(f"{fn}  {roles[id(c)]}:\n    {statement_of(c)}")
    # Run it alone, with what loft can say about a store or a crash switched on.
    lines = {}
    for backend in backends_of(args):
        cmd = [loft_binary()] + (["--native-c"] if backend == "c" else []) + ["--no-warnings", "--tests", path]
        env = dict(os.environ, LOFT_LEAK_CHECK="1", LOFT_REFREE_FAILS="1", LOFT_LEAK_SITES="1",
                   LOFT_TIMEOUT=str(TIME_LIMIT[backend]))
        done = subprocess.run(cmd, capture_output=True, text=True, env=env, cwd=keep, check=False)
        out = done.stdout + done.stderr
        with open(os.path.join(keep, f"run-{backend}.txt"), "w", encoding="utf-8") as f:
            f.write(out)
        print(f"\n--- {backend}: `LOFT_LEAK_CHECK=1 LOFT_REFREE_FAILS=1 LOFT_LEAK_SITES=1 {' '.join(cmd)}`"
              f"   (whole output in {keep}/run-{backend}.txt)")
        for c in group:
            m = re.search(r"^%s c=(\d+) log=<(.*?)> \| (.*)$" % re.escape(c["name"]), out, re.M)
            lines[(backend, c["name"])] = m
        said = [ln for ln in out.splitlines() if ln.strip() and not re.match(r"\S+ c=\d+ log=<", ln)
                and "test result:" not in ln and not re.match(r"\s*ok\s", ln)]
        for ln in said[:40]:
            print("    " + ln[:700])
        base = twin or plain
        mine, theirs = lines[(backend, cell["name"])], base and lines[(backend, base["name"])]
        if not mine:
            print(f"    the cell printed no final line: it did not finish on {backend}")
            continue
        print(f"    the operand ran {mine.group(1)} time(s), the slot says {cell['want']}; log <{mine.group(2)}>"
              + (f", source order <{order['log']}>" if order else ""))
        if theirs:
            got, ref = fields_of(mine.group(3)), fields_of(theirs.group(3))
            off = [k for k in got if got[k] != ref.get(k)]
            for k in off:
                print(f"    {k}:  the cell {got[k]}\n    {' ' * len(k)}   {roles[id(base)].split(' (')[0]} {ref.get(k)}")
            if not off:
                print(f"    it leaves the same as {roles[id(base)].split(' (')[0]}")
    # The IR of the cell beside the IR of what it is compared with: the working form and
    # the broken one, which is where a compiler fix starts (CODEGEN_METHOD.md).
    base = twin or plain
    if base:
        dump = subprocess.run([loft_binary(), "introspect", path], capture_output=True, text=True,
                              cwd=keep, check=False).stdout.split("std::cell::UnsafeCell")[0]
        with open(os.path.join(keep, "introspect.txt"), "w", encoding="utf-8") as f:
            f.write(dump)
        fn_of = {id(c): fn for c, (_, _, fn) in zip(group, spans)}

        def functions(c):
            n = group.index(c)
            return [f"{d}_{n}" for d in re.findall(r"\bfn\s+([A-Za-z_]\w*)\s*\(", c["decl"])] + [
                fn_of[id(c)] + "_in", fn_of[id(c)]]
        names = [c["name"] for c in group]
        a, b = ir_of(dump, functions(base), names), ir_of(dump, functions(cell), names)
        diff = list(difflib.unified_diff(a, b, roles[id(base)].split(" (")[0], "the cell", lineterm="", n=2))
        print(f"\n--- the IR, {roles[id(base)].split(' (')[0]} against the cell   (`loft introspect`, whole "
              f"output in {keep}/introspect.txt)")
        if not a or not b:
            print("    `loft introspect` printed no IR for these functions")
        elif not diff:
            print("    the two lower to the same IR: the difference is made after it, in a backend or at run time")
        for ln in diff[:args.lines]:
            print("    " + ln[:220])
        if len(diff) > args.lines:
            print(f"    … {len(diff) - args.lines} more line(s); `--lines N` shows more")
    return 0


# The chapters whose rules say how an operand is evaluated: the ones a slot can cite.
EVALUATION_CHAPTERS = ("operational.md", "iteration.md", "calls.md", "collections.md", "matching.md",
                       "formatting.md", "coroutines.md", "concurrency.md", "tuples.md", "closures.md",
                       "operators.md")


def ops_of(path):
    """The IR operations `loft introspect` prints for a program, as a set of names."""
    text = subprocess.run([loft_binary(), "introspect", path], capture_output=True, text=True,
                          check=False).stdout.split("std::cell::UnsafeCell")[0]
    return set(re.findall(r"\bOp[A-Z]\w+", text))


def cmd_coverage(args):
    """What the matrix does not exercise, measured two ways without reading an issue."""
    import json
    slots, fillers, _, problems = load_all()
    if problems:
        print("\n".join(problems))
        return 1
    if args.what == "rules":
        # The evaluation rules no slot cites: each is an operand position the register
        # may lack, or a rule that is not about one — to be read, not assumed.
        sys.path.insert(0, HERE)
        import rule_tags
        cited = {s["rule"] for s in slots}
        defined = rule_tags.defined_rules()
        total = 0
        for chapter in EVALUATION_CHAPTERS:
            mine = sorted(r for r, files in defined.items() if chapter in files)
            missing = [r for r in mine if r not in cited]
            total += len(missing)
            print(f"{chapter:16} {len(mine) - len(missing):3} of {len(mine):3} rules cited by a slot; "
                  f"not cited: {' '.join(missing)}")
        print(f"slot_matrix coverage: {len(cited - {'-'})} rules cited by {len(slots)} slots; "
              f"{total} evaluation rules cited by none")
        return 0
    # IR operations: what the corpus makes the compiler emit and no cell does.
    cells = make_cells(slots, fillers, "all", set(), None, *axes_of(args), depth=args.depth)
    with tempfile.TemporaryDirectory(prefix="slot_matrix.") as tmp:
        verdicts = evaluate(cells, ("interp",))
        accepted = [c for c in cells if verdicts[c["name"]][0] != "REFUSED"]
        by_kind, programs = {}, []
        for c in accepted:
            by_kind.setdefault(c["kind_type"], []).append(c)
        for kind_type, mine in by_kind.items():
            for n in range(0, len(mine), CHUNK):
                text, spans = program(kind_type, mine[n:n + CHUNK])
                # `loft introspect` prints what `main` reaches, so every cell is called.
                text += "fn main() { " + " ".join(fn + "();" for _, _, fn in spans) + " }\n"
                path = os.path.join(tmp, f"ops{len(programs)}.loft")
                with open(path, "w", encoding="utf-8") as f:
                    f.write(text)
                programs.append(path)
        with concurrent.futures.ThreadPoolExecutor(max_workers=os.cpu_count() or 4) as pool:
            got = list(pool.map(ops_of, programs))
    empty = sum(1 for g in got if not g)
    matrix = set().union(*got) if got else set()
    print(f"the matrix: {len(accepted)} accepted cells in {len(programs)} programs emit {len(matrix)} "
          f"distinct IR operations" + (f"  ({empty} program(s) printed no IR — the count is low)" if empty else ""))
    cache = os.path.join(ROOT, "target", "slot_matrix_corpus_ops.json")
    corpus = None
    if os.path.exists(cache) and not args.refresh:
        saved = json.load(open(cache, encoding="utf-8"))
        if saved.get("build") == build():
            corpus = saved["ops"]
    if corpus is None:
        lister = os.path.join(HERE, "corpus_files.sh")
        files = subprocess.run([lister], capture_output=True, text=True, cwd=ROOT, check=False).stdout.split()
        files = [f if os.path.isabs(f) else os.path.join(ROOT, f) for f in files]
        print(f"reading the corpus: {len(files)} files (minutes; kept in {cache} for this build)")
        corpus = {}
        with concurrent.futures.ThreadPoolExecutor(max_workers=os.cpu_count() or 4) as pool:
            for ops in pool.map(ops_of, files):
                for op in ops:
                    corpus[op] = corpus.get(op, 0) + 1
        os.makedirs(os.path.dirname(cache), exist_ok=True)
        json.dump({"build": build(), "ops": corpus}, open(cache, "w", encoding="utf-8"))
    missing = sorted(((n, op) for op, n in corpus.items() if op not in matrix), reverse=True)
    print(f"the corpus: {len(corpus)} distinct IR operations; {len(corpus) - len(missing)} of them emitted "
          f"by a cell, {len(missing)} by none:")
    for n, op in missing[:args.top]:
        print(f"  {op:30} in {n:4} corpus file(s)")
    if len(missing) > args.top:
        print(f"  … {len(missing) - args.top} more; `--top N` shows more")
    return 0


def cmd_walk(args):
    """The matrix on an OLDER build beside this one: what it would have caught."""
    slots, fillers, _, problems = load_all()
    if problems:
        print("\n".join(problems))
        return 1
    cells = make_cells(slots, fillers, "all", set(args.slot), args.kind, *axes_of(args), depth=args.depth)
    backends = backends_of(args)
    new = evaluate(cells, backends)
    here = loft_binary()
    os.environ["LOFT"] = args.old
    try:
        old = evaluate(cells, backends)
    finally:
        os.environ["LOFT"] = here
    groups = {"fixed since": [], "failing on both": [], "new since": [], "refused then": []}
    for cell in cells:
        was, now = old[cell["name"]][0], new[cell["name"]][0]
        if was == "REFUSED" and now != "REFUSED":
            groups["refused then"].append(cell)
        elif was == "FAIL" and now == "ok":
            groups["fixed since"].append(cell)
        elif was == "FAIL" and now == "FAIL":
            groups["failing on both"].append(cell)
        elif was == "ok" and now == "FAIL":
            groups["new since"].append(cell)
    for title, which in (("fixed since", old), ("new since", new), ("failing on both", new)):
        mine = groups[title]
        print(f"{title}: {len(mine)} cell(s)")
        for line in causes(mine, which, cells):
            print("  " + line)
    print(f"refused by the older build and accepted now: {len(groups['refused then'])} cell(s)")
    print(f"slot_matrix walk: {len(cells)} cells, old [{args.old}], new [{here}]")
    return 0


def axes_of(args):
    """The contexts and frames a run was asked for: the first of each, one by id, or `all`."""
    contexts, frames = load_contexts(), list(load_frames()[1])
    pick = lambda have, want, key: have[:1] if not want else have if want == "all" else [x for x in have if key(x) == want]
    chosen = pick(contexts, args.context, lambda c: c[0]), pick(frames, args.frame, lambda f: f)
    if not chosen[0] or not chosen[1]:
        raise SystemExit(f"slot_matrix: contexts are {' '.join(c[0] for c in contexts)}; frames are {' '.join(frames)}")
    return chosen


def backends_of(args):
    return ("interp", "c") if args.backend == "both" else (args.backend,)


def cmd_run(args):
    slots, fillers, _, problems = load_all()
    if problems:
        print("\n".join(problems))
        return 1
    cells = make_cells(slots, fillers, args.filler, set(args.slot), args.kind, *axes_of(args), depth=args.depth)
    return 1 if run_matrix(cells, backends_of(args), args.verbose) else 0


def cmd_fillers(args):
    """Each filler in its type's neutral slot: one bump of its counter, the plain value."""
    slots, fillers, _, problems = load_all()
    if problems:
        print("\n".join(problems))
        return 1
    cells = make_cells(slots, fillers, "all", set(neutral(slots, fillers).values()), KINDS[0][0], depth=args.depth,
                       against_plain=True)
    return 1 if run_matrix(cells, backends_of(args), args.verbose) else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bg", metavar="FILE", help="run the command in the background, its output in FILE, "
                    "ending with a `slot_matrix: done exit=<n>` line (before the subcommand)")
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("check").set_defaults(fn=cmd_check)
    sub.add_parser("selftest").set_defaults(fn=cmd_selftest)
    sub.add_parser("list").set_defaults(fn=cmd_list)
    p = sub.add_parser("fillers")
    p.add_argument("--backend", choices=("interp", "c", "both"), default="interp")
    p.add_argument("-v", "--verbose", action="store_true")
    p.add_argument("--depth", type=int, choices=(1, 2), default=1, help="2 adds every wrapper filler around every other filler")
    p.set_defaults(fn=cmd_fillers)
    p = sub.add_parser("run")
    p.add_argument("--filler", default="var", help="`var` (default), `all`, or one filler id")
    p.add_argument("--backend", choices=("interp", "c", "both"), default="interp")
    p.add_argument("--slot", action="append", default=[], help="one slot id; may repeat")
    p.add_argument("--kind", choices=[k for k, _ in KINDS])
    p.add_argument("-v", "--verbose", action="store_true")
    p.add_argument("--context", help="where the statement stands: an id of syntax_contexts.tsv, or `all`")
    p.add_argument("--frame", help="where its values come from: a frame of the prelude, or `all`")
    p.add_argument("--depth", type=int, choices=(1, 2), default=1, help="2 adds every wrapper filler around every other filler")
    p.set_defaults(fn=cmd_run)
    p = sub.add_parser("coverage")
    p.add_argument("what", choices=("rules", "ops"), help="`rules`: the evaluation rules no slot cites; "
                   "`ops`: the IR operations the corpus emits and no cell does")
    p.add_argument("--depth", type=int, choices=(1, 2), default=1)
    p.add_argument("--context")
    p.add_argument("--frame")
    p.add_argument("--top", type=int, default=40)
    p.add_argument("--refresh", action="store_true", help="read the corpus again")
    p.set_defaults(fn=cmd_coverage)
    p = sub.add_parser("explain")
    p.add_argument("cell", help="a cell's name, as a run prints it")
    p.add_argument("--out", metavar="DIR", help="where to keep its program and outputs (default: a new temp dir)")
    p.add_argument("--backend", choices=("interp", "c", "both"), default="both")
    p.add_argument("--lines", type=int, default=60, help="how much of the IR diff to print")
    p.set_defaults(fn=cmd_explain)
    p = sub.add_parser("expr")
    p.add_argument("expression", help="one expression of the prelude's world; `<C>` its counter, or a hole `<I:type>`")
    p.add_argument("--type", required=True, help="the operand type it gives")
    p.add_argument("--plain", help="the same value with no counter, when the type's variable is not it")
    p.add_argument("--depth", type=int, choices=(1, 2), default=2, help="2 (default) also puts it inside every wrapper")
    p.add_argument("--slot", action="append", default=[], help="only these slots; may repeat")
    p.add_argument("--kind", choices=[k for k, _ in KINDS])
    p.add_argument("--backend", choices=("interp", "c", "both"), default="both")
    p.add_argument("--context", help="an id of syntax_contexts.tsv, or `all`")
    p.add_argument("--frame", help="a frame of the prelude, or `all`")
    p.add_argument("-v", "--verbose", action="store_true")
    p.set_defaults(fn=cmd_expr)
    p = sub.add_parser("expect")
    p.add_argument("--propose", action="store_true", help="print rows for the slots that have none")
    p.set_defaults(fn=cmd_expect)
    p = sub.add_parser("cell")
    p.add_argument("statement", help="one statement of the prelude's world, `<S>` the operand")
    p.add_argument("--type", help="try every filler of this operand type")
    p.add_argument("--filler", help="or: one expression to try, `<C>` its counter")
    p.add_argument("--plain", help="with --filler: the same value with no counter (a variable)")
    p.add_argument("--decl", help="a top-level declaration the statement needs")
    p.add_argument("--count", choices=COUNTS, default="once")
    p.add_argument("--runs", type=int, default=1, help="how often the operand must run")
    p.add_argument("--id", default="adhoc")
    p.add_argument("--family", choices=FAMILIES, default="read")
    p.add_argument("--rule", default="-")
    p.add_argument("--kind", choices=[k for k, _ in KINDS])
    p.add_argument("--backend", choices=("interp", "c", "both"), default="both")
    p.add_argument("--context", help="an id of syntax_contexts.tsv, or `all`")
    p.add_argument("--frame", help="a frame of the prelude, or `all`")
    p.set_defaults(fn=cmd_cell)
    p = sub.add_parser("gate")
    p.add_argument("--update", action="store_true", help="rewrite the baseline from this run")
    p.add_argument("--family", choices=FAMILIES, help="only the slots of one family, and their baseline lines")
    p.add_argument("--full", action="store_true", help="every cell on the C backend too, not the sample")
    p.set_defaults(fn=cmd_gate)
    p = sub.add_parser("walk")
    p.add_argument("--old", required=True, help="the older loft binary")
    p.add_argument("--backend", choices=("interp", "c", "both"), default="interp")
    p.add_argument("--slot", action="append", default=[])
    p.add_argument("--kind", choices=[k for k, _ in KINDS])
    p.add_argument("--context", help="where the statement stands: an id of syntax_contexts.tsv, or `all`")
    p.add_argument("--frame", help="where its values come from: a frame of the prelude, or `all`")
    p.add_argument("--depth", type=int, choices=(1, 2), default=1, help="2 adds every wrapper filler around every other filler")
    p.set_defaults(fn=cmd_walk)
    args = ap.parse_args()
    if args.bg:
        # The same command, detached, its output in a file that ends with one line a wait
        # can look for.  A whole run takes minutes; nothing should have to sit on it.
        out = os.path.abspath(args.bg)
        rest, skip = [], False
        for a in sys.argv[1:]:
            if skip or a.startswith("--bg="):
                skip = False
                continue
            if a == "--bg":
                skip = True
                continue
            rest.append(a)
        with open(out, "w", encoding="utf-8") as f:
            child = subprocess.Popen([sys.executable, os.path.abspath(__file__)] + rest, stdout=f,
                                     stderr=subprocess.STDOUT, start_new_session=True,
                                     env=dict(os.environ, SLOT_MATRIX_SIGNAL="1"))
        print(f"slot_matrix: running in the background, pid {child.pid}, output in {out}\n"
              f"it ends with the line `{DONE} exit=<n>`; wait for it with:\n"
              f"  scripts/wait_for.sh {out} '{DONE}' --pid {child.pid}")
        sys.exit(0)
    code = args.fn(args)
    if os.environ.get("SLOT_MATRIX_SIGNAL"):
        print(f"{DONE} exit={code}", flush=True)
    sys.exit(code)


if __name__ == "__main__":
    main()
