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
    gate       the whole matrix on both backends against scripts/slot_matrix.baseline: a
               cell that is not ok and not listed fails, and so does a listed cell that now
               passes.  `--update` rewrites the baseline.  The PR gate (tests/slot_matrix.rs).
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

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
SLOTS_TSV = os.path.join(HERE, "syntax_slots.tsv")
FILLERS_TSV = os.path.join(HERE, "syntax_fillers.tsv")
OUTSIDE_TSV = os.path.join(HERE, "syntax_outside.tsv")
CONTEXTS_TSV = os.path.join(HERE, "syntax_contexts.tsv")
PRELUDE = os.path.join(HERE, "syntax_slots_prelude.loft")
BASELINE = os.path.join(HERE, "slot_matrix.baseline")

COUNTS = ("once", "cond", "step")
FAMILIES = ("iteration", "write", "read", "call", "operator", "control", "construct")
# The collection kinds a cell can be generated for.  A slot runs in all of them when a
# filler of its type reads one of the prelude's kind-dependent names; otherwise the kind
# changes nothing it reads, and it runs in the first only.
KINDS = (("vector", "vector<P>"), ("sorted", "sorted<P[k]>"),
         ("index", "index<P[k]>"), ("hash", "hash<P[k]>"))
KIND_NAMES = re.compile(r"\b(xs|mk|items|tk)\b")
OUTSIDE_CLASSES = ("none:", "decl:", "unreached:")
SLOT = "<S>"
BIND = "<BIND>"
COUNTER = "<C>"



def types_of(fillers):
    """The operand types: whatever the fillers table has a plain variable for."""
    return [f["type"] for f in fillers if f["id"] == f["type"] + "-var"]


def kinded(fillers):
    """The types whose value depends on the collection kind."""
    return {f["type"] for f in fillers if KIND_NAMES.search(f["expr"])}


def neutral(slots, fillers):
    """Per type, the slot its fillers are tried in by `fillers`: the first in the register
    that reads the value once in `main`."""
    out = {}
    for t in types_of(fillers):
        for s in slots:
            if s["type"] == t and s["count"] == "once" and not s["decl"] and s["family"] != "write":
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
        slot["line"] = nr
        slots.append(slot)
    return slots, problems


def load_fillers(path=FILLERS_TSV):
    fillers, problems = [], []
    for nr, cols in rows(path):
        if len(cols) != 3:
            problems.append(f"{os.path.basename(path)}:{nr}: {len(cols)} columns, want 3")
            continue
        fillers.append({"id": cols[0], "type": cols[1], "expr": cols[2], "line": nr})
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
    text = text.replace(SLOT, " ").replace(COUNTER, " c ")
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
        elif s["count"] == "once" and s["runs"] != "1":
            problems.append(f"{where}: count `once` with runs {s['runs']}")
        elif s["count"] == "cond" and s["runs"] not in ("0", "1"):
            problems.append(f"{where}: count `cond` with runs {s['runs']}")
        if s["rule"] != "-" and s["rule"] not in rules:
            problems.append(f"{where}: rule `{s['rule']}` is not defined in doc/claude/formal/")
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
        if is_var == (COUNTER in f["expr"]):
            problems.append(f"{where}: a `var` filler takes no `{COUNTER}`, every other one does")
        if f["type"] not in types:
            problems.append(f"{where}: type `{f['type']}` has no `{f['type']}-var` row")
    tried = neutral(slots, fillers)
    for t in types:
        if sum(1 for f in fillers if f["type"] == t) < 2:
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


def load_frames():
    """The prelude's head, and its frames by name: the function a cell's statement stands in."""
    parts = re.split(r"^// === CELL (\w+) ===\n", open(PRELUDE, encoding="utf-8").read(), flags=re.M)
    return parts[0], dict(zip(parts[1::2], parts[2::2]))


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
        print(f"{fam:10} {len(mine):3} slots   once {by['once']:3}  cond {by['cond']:2}  step {by['step']:2}")
    owed = [s["id"] for s in slots if s["rule"] == "-"]
    print(f"{'total':10} {len(slots):3} slots, {len(fillers)} fillers, {len(outside)} lexemes outside")
    print(f"no rule states the count of {len(owed)}: {' '.join(owed)}")
    unreached = sorted(x for x, why in outside.items() if why.startswith("unreached:"))
    print(f"an operand the matrix cannot reach yet, {len(unreached)}: {' '.join(unreached)}")
    return 0


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


def make_cells(slots, fillers, which_filler, which_slot, which_kind, contexts=None, frames=None):
    """The cells of a run.  A slot's plain-variable cell is always there: it is what the
    value of every other cell of that slot is compared with.  `contexts` and `frames` are
    the ones to generate for; left out, the first of each."""
    contexts = contexts or load_contexts()[:1]
    frames = frames or list(load_frames()[1])[:1]
    out = []
    kinded_types = kinded(fillers)
    for slot in slots:
        if which_slot and slot["id"] not in which_slot:
            continue
        kinds = KINDS if slot["type"] in kinded_types else KINDS[:1]
        for (kind, kind_type), (ctx, passes, wrap), frame in (
                (k, c, f) for k in kinds for c in contexts for f in frames):
            if which_kind and kind != which_kind:
                continue
            where = f"{slot['id']}.%s.{kind}" + ("" if (ctx, frame) == (contexts[0][0], frames[0]) and len(contexts) + len(frames) == 2 else f"@{ctx}/{frame}")
            base = where % f"{slot['type']}-var"
            for f in fillers:
                if f["type"] != slot["type"]:
                    continue
                is_var = f["id"].endswith("-var")
                if not is_var and which_filler != "all" and f["id"] != which_filler:
                    continue
                expr = f["expr"].replace(COUNTER, "c")
                put = lambda stmt: wrap.replace("<STMT>", stmt)
                cell = {"name": where % f["id"], "slot": slot["id"], "frame": frame,
                        "kind_type": kind_type,
                        "decl": slot["decl"].replace(SLOT, expr).replace(BIND, ""),
                        "stmt": put(slot["stmt"].replace(SLOT, expr)), "base": base,
                        "want": 0 if is_var else int(slot["runs"]) * passes, "log": None}
                out.append(cell)
                # The bound-first twin: the filler bound to a local, then the statement
                # with that local.  A local is evaluated once by construction, so the twin
                # says what the statement must leave behind.  Only where the operand runs
                # exactly once and sits in the frame; elsewhere the plain variable says it.
                if not is_var and slot["count"] == "once" and (not slot["decl"] or BIND in slot["decl"]):
                    cell["base"] = cell["name"] + ".twin"
                    bind = f"t0 = {expr}; "
                    out.append(dict(
                        cell, name=cell["base"], base=cell["base"], want=passes,
                        decl=slot["decl"].replace(BIND, bind).replace(SLOT, "t0"),
                        stmt=put(("" if slot["decl"] else bind) + slot["stmt"].replace(SLOT, "t0"))))
                silent = ".calls += 1" in f["expr"] and ".log" not in f["expr"]
                turn = (None if is_var or silent or slot["decl"] or slot["count"] != "once"
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


def invoke(backend, path):
    cmd = [loft_binary()] + (["--native-c"] if backend == "c" else []) + ["--no-warnings", "--tests", path]
    env = dict(os.environ, LOFT_LEAK_CHECK="1", LOFT_TIMEOUT="900")
    try:
        done = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=1200, check=False)
    except subprocess.TimeoutExpired:
        return "no answer in 1200 s"
    return done.stdout + done.stderr


def run_program(backend, cells, tmp, tag):
    """Run `cells` (all of one collection kind) as one program.  Returns, per cell name,
    ("REFUSED", message) | ("LOST", last output) | ("RAN", count, log, dump, test failure)."""
    results, pending, turn = {}, list(cells), 0
    while pending:
        turn += 1
        text, spans = program(pending[0]["kind_type"], pending)
        path = os.path.join(tmp, f"{tag}_{turn}.loft")
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)
        out = invoke(backend, path)
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
        failed = dict(re.findall(r"^\s*FAIL\s+\S+::(test_c\d+)\s+—\s+(.*)$", out, re.M))
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
            results[lost[0]["name"]] = ("LOST", " / ".join(tail)[:300] or "no output")
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
        return "FAIL", f"the result differs from the same statement with {how}"
    if failure:
        return "FAIL", ("a store is left behind: " if "not freed" in failure else "") + failure
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


def evaluate(cells, backends):
    """Run the cells.  Returns {cell name: (verdict, [(backend, verdict, reason)])}, the
    verdict the worst over the backends: ok | FAIL | REFUSED."""
    with tempfile.TemporaryDirectory(prefix="slot_matrix.") as tmp:
        broken = prelude_problem(cells, "interp", tmp)
        if broken:
            raise SystemExit(f"slot_matrix: {broken}")
        interp = run_backend("interp", cells, tmp)
        per = {"interp": interp}
        if "c" in backends:
            accepted = [c for c in cells if interp.get(c["name"], ("",))[0] != "REFUSED"]
            per["c"] = run_backend("c", accepted, tmp)
    out = {}
    for cell in cells:
        lines, worst = [], "ok"
        for backend in per:
            if backend == "c" and worst == "REFUSED":
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
                       ("left behind", "leak"), ("freed twice", "leak")):
        if word in why:
            return name
    return "crash"


def causes(cells, verdicts):
    """The failing cells folded toward causes.  A slot that fails with most of its fillers
    is one cause in the SLOT's lowering; what is left is folded by filler, a form of
    expression that fails in several slots being one cause in how that FORM is lowered."""
    by_slot, by_filler, total_slot = {}, {}, {}
    for cell in cells:
        slot, filler = cell["name"].split(".")[:2]
        spot = re.search(r"@[\w/]+", cell["name"])
        slot += spot.group(0) if spot else ""
        total_slot.setdefault(slot, set()).add(filler)
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
    for (filler, ch), slots in sorted(left.items()):
        bare = sorted({x.partition("@")[0] for x in slots})
        text = f"filler {filler}: {len(bare)} slot(s) ({ch}) — " + ", ".join(bare[:6]) + (" …" if len(bare) > 6 else "")
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
    for cell in cells:
        worst, lines = verdicts[cell["name"]]
        tally[worst] += 1
        accepted[cell["slot"]] = accepted.get(cell["slot"], 0) + (worst != "REFUSED")
        for backend, v, why in lines:
            if v == "FAIL" or verbose:
                print(f"{v:8} {cell['name']} [{backend}]: {why[:200]}")
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
          f"{len(vacuous)} slot(s) vacuous  [{loft_binary()}]")
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


def cmd_gate(args):
    slots, fillers, outside, problems = load_all()
    problems = check(slots, fillers, outside, lexicon(), defined_rules(), problems)
    if problems:
        print("\n".join(problems))
        return 1
    cells = make_cells(slots, fillers, "all", set(), None)
    verdicts = evaluate(cells, ("interp", "c"))
    keys = baseline_keys(cells, verdicts)
    if args.update:
        with open(BASELINE, "w", encoding="utf-8") as f:
            f.write(BASELINE_HEAD)
            for key in sorted(keys):
                f.write(("~" if key[2] == "crash" else "") + "\t".join(key) + "\n")
        print(f"slot_matrix gate: baseline rewritten, {len(keys)} line(s)")
        return 0
    stable, unstable = load_baseline()
    # A crash may also show as another channel on another machine; an unstable line
    # excuses its cell on its backend whatever the channel.
    excused = {(n, b) for n, b, _ in unstable}
    new = sorted(k for k in keys - stable if (k[0], k[1]) not in excused)
    gone = sorted(stable - keys)
    for name, backend, ch in new:
        why = next((w for b, v, w in verdicts[name][1] if b == backend and v == "FAIL"), "refused by the compiler")
        print(f"NEW      {name} [{backend}] ({ch}): {why[:180]}")
    for name, backend, ch in gone:
        print(f"PASSES   {name} [{backend}] ({ch}): remove its line from scripts/slot_matrix.baseline")
    if new:
        print("the new cells fold to:")
        only = {n for n, _, _ in new}
        for line in causes([c for c in cells if c["name"] in only], verdicts):
            print("  " + line)
    print(f"slot_matrix gate: {len(cells)} cells, {len(keys)} not ok ({len(stable) + len(unstable)} in the "
          f"baseline), {len(new)} new, {len(gone)} to remove  [{loft_binary()}]")
    return 1 if new or gone else 0


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
        mine = [{"id": "adhoc-var", "type": tp, "expr": args.plain, "line": 0},
                {"id": "adhoc-new", "type": tp, "expr": args.filler, "line": 0}]
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


def cmd_walk(args):
    """The matrix on an OLDER build beside this one: what it would have caught."""
    slots, fillers, _, problems = load_all()
    if problems:
        print("\n".join(problems))
        return 1
    cells = make_cells(slots, fillers, "all", set(args.slot), args.kind, *axes_of(args))
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
        for line in causes(mine, which):
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
    cells = make_cells(slots, fillers, args.filler, set(args.slot), args.kind, *axes_of(args))
    return 1 if run_matrix(cells, backends_of(args), args.verbose) else 0


def cmd_fillers(args):
    """Each filler in its type's neutral slot: one bump of its counter, the plain value."""
    slots, fillers, _, problems = load_all()
    if problems:
        print("\n".join(problems))
        return 1
    cells = make_cells(slots, fillers, "all", set(neutral(slots, fillers).values()), KINDS[0][0])
    return 1 if run_matrix(cells, backends_of(args), args.verbose) else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("check").set_defaults(fn=cmd_check)
    sub.add_parser("selftest").set_defaults(fn=cmd_selftest)
    sub.add_parser("list").set_defaults(fn=cmd_list)
    p = sub.add_parser("fillers")
    p.add_argument("--backend", choices=("interp", "c", "both"), default="interp")
    p.add_argument("-v", "--verbose", action="store_true")
    p.set_defaults(fn=cmd_fillers)
    p = sub.add_parser("run")
    p.add_argument("--filler", default="var", help="`var` (default), `all`, or one filler id")
    p.add_argument("--backend", choices=("interp", "c", "both"), default="interp")
    p.add_argument("--slot", action="append", default=[], help="one slot id; may repeat")
    p.add_argument("--kind", choices=[k for k, _ in KINDS])
    p.add_argument("-v", "--verbose", action="store_true")
    p.add_argument("--context", help="where the statement stands: an id of syntax_contexts.tsv, or `all`")
    p.add_argument("--frame", help="where its values come from: a frame of the prelude, or `all`")
    p.set_defaults(fn=cmd_run)
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
    p.set_defaults(fn=cmd_gate)
    p = sub.add_parser("walk")
    p.add_argument("--old", required=True, help="the older loft binary")
    p.add_argument("--backend", choices=("interp", "c", "both"), default="interp")
    p.add_argument("--slot", action="append", default=[])
    p.add_argument("--kind", choices=[k for k, _ in KINDS])
    p.add_argument("--context", help="where the statement stands: an id of syntax_contexts.tsv, or `all`")
    p.add_argument("--frame", help="where its values come from: a frame of the prelude, or `all`")
    p.set_defaults(fn=cmd_walk)
    args = ap.parse_args()
    sys.exit(args.fn(args))


if __name__ == "__main__":
    main()
