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

The registers: scripts/syntax_slots.tsv, scripts/syntax_fillers.tsv,
scripts/syntax_outside.tsv, and the world a cell runs in,
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
PRELUDE = os.path.join(HERE, "syntax_slots_prelude.loft")

COUNTS = ("once", "cond", "step")
TYPES = ("int", "bool", "text", "float", "rec", "coll", "vint", "enum", "fn")
FAMILIES = ("iteration", "write", "read", "call", "operator", "control", "construct")
# The collection kinds a `coll` slot is generated for.  A slot of another type runs in the
# first one only: the kind changes nothing it reads.
KINDS = (("vector", "vector<P>"), ("sorted", "sorted<P[k]>"),
         ("index", "index<P[k]>"), ("hash", "hash<P[k]>"))
# The slot each type's fillers are tried in by `fillers`: one that reads the value and
# writes nothing else.
NEUTRAL = {"int": "wr-var", "bool": "ct-if", "text": "op-concat", "float": "op-as",
           "rec": "rd-field", "coll": "rd-len", "vint": "it-for-vint",
           "enum": "ct-match-enum", "fn": "ca-callee"}
OUTSIDE_CLASSES = ("none:", "decl:", "unreached:")
SLOT = "<S>"
COUNTER = "<C>"


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
        if s["type"] not in TYPES:
            problems.append(f"{where}: unknown type `{s['type']}`")
        if not s["runs"].isdigit():
            problems.append(f"{where}: runs `{s['runs']}` is not a number")
        elif s["count"] == "once" and s["runs"] != "1":
            problems.append(f"{where}: count `once` with runs {s['runs']}")
        elif s["count"] == "cond" and s["runs"] not in ("0", "1"):
            problems.append(f"{where}: count `cond` with runs {s['runs']}")
        if s["rule"] != "-" and s["rule"] not in rules:
            problems.append(f"{where}: rule `{s['rule']}` is not defined in doc/claude/formal/")
        if (s["stmt"] + s["decl"]).count(SLOT) != 1:
            problems.append(f"{where}: the statement and declaration hold `{SLOT}` "
                            f"{(s['stmt'] + s['decl']).count(SLOT)} times, want once")
    seen, plain = set(), set()
    for f in fillers:
        where = f"syntax_fillers.tsv:{f['line']}: {f['id']}"
        if f["id"] in seen:
            problems.append(f"{where}: the id is used twice")
        seen.add(f["id"])
        if f["type"] not in TYPES:
            problems.append(f"{where}: unknown type `{f['type']}`")
        is_var = f["id"] == f["type"] + "-var"
        if is_var:
            plain.add(f["type"])
        if is_var == (COUNTER in f["expr"]):
            problems.append(f"{where}: a `var` filler takes no `{COUNTER}`, every other one does")
    for t in TYPES:
        if t not in plain:
            problems.append(f"syntax_fillers.tsv: type `{t}` has no `{t}-var` row")
        if t not in NEUTRAL or NEUTRAL[t] not in {s["id"] for s in slots}:
            problems.append(f"slot_matrix.py: type `{t}` has no neutral slot `{NEUTRAL.get(t)}`")
    for t in sorted({s["type"] for s in slots if s["type"] in TYPES}):
        if sum(1 for f in fillers if f["type"] == t) < 2:
            problems.append(f"syntax_fillers.tsv: type `{t}` has a slot and no filler but its variable")
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
DECL_NAMES = re.compile(r"\b(give|tail|gen)\b")
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


def make_cells(slots, fillers, which_filler, which_slot, which_kind):
    """The cells of a run.  A slot's plain-variable cell is always there: it is what the
    value of every other cell of that slot is compared with."""
    out = []
    for slot in slots:
        if which_slot and slot["id"] not in which_slot:
            continue
        kinds = KINDS if slot["type"] == "coll" else KINDS[:1]
        for kind, kind_type in kinds:
            if which_kind and kind != which_kind:
                continue
            base = f"{slot['id']}.{slot['type']}-var.{kind}"
            for f in fillers:
                if f["type"] != slot["type"]:
                    continue
                is_var = f["id"].endswith("-var")
                if not is_var and which_filler != "all" and f["id"] != which_filler:
                    continue
                expr = f["expr"].replace(COUNTER, "c")
                cell = {"name": f"{slot['id']}.{f['id']}.{kind}", "slot": slot["id"],
                        "kind_type": kind_type, "decl": slot["decl"].replace(SLOT, expr),
                        "stmt": slot["stmt"].replace(SLOT, expr), "base": base,
                        "want": 0 if is_var else int(slot["runs"]), "log": None}
                out.append(cell)
                # The bound-first twin: the filler bound to a local, then the statement
                # with that local.  A local is evaluated once by construction, so the twin
                # says what the statement must leave behind.  Only where the operand runs
                # exactly once and sits in `main`; elsewhere the plain variable says it.
                if not is_var and slot["count"] == "once" and not slot["decl"]:
                    cell["base"] = cell["name"] + ".twin"
                    out.append(dict(cell, name=cell["base"], base=cell["base"], want=1,
                                    stmt=f"t0 = {expr}; " + slot["stmt"].replace(SLOT, "t0")))
                silent = ".calls += 1" in f["expr"] and ".log" not in f["expr"]
                turn = (None if is_var or silent or slot["decl"] or slot["count"] != "once"
                        else ordered(slot["stmt"]))
                if turn:
                    out.append(dict(cell, name=cell["name"] + ".order",
                                    stmt=turn[0].replace(SLOT, expr), log=turn[1]))
    return out


def program(kind_type, cells):
    """One loft program holding `cells` as test functions; and, per cell, its function name
    and the lines it occupies."""
    head, _, body = open(PRELUDE, encoding="utf-8").read().partition("// === CELL ===\n")
    text = head.replace("KIND", kind_type)
    spans = []
    for n, cell in enumerate(cells):
        fn = f"test_c{n:04d}"
        rename = lambda t, n=n: DECL_NAMES.sub(lambda m: f"{m.group(1)}_{n}", t)
        part = (body.replace("CELLDECL", rename(cell["decl"])).replace("CELLFN", fn)
                .replace("CELLSTMT", rename(cell["stmt"])).replace("CELLNAME", cell["name"]))
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


def run_matrix(cells, backends, verbose):
    """Run the cells; print what is not ok.  Returns the number of failing cells and slots."""
    with tempfile.TemporaryDirectory(prefix="slot_matrix.") as tmp:
        interp = run_backend("interp", cells, tmp)
        per = {"interp": interp}
        if "c" in backends:
            accepted = [c for c in cells if interp.get(c["name"], ("",))[0] != "REFUSED"]
            per["c"] = run_backend("c", accepted, tmp)
    tally = {"ok": 0, "FAIL": 0, "REFUSED": 0}
    accepted_per_slot, causes = {}, {}
    for cell in cells:
        accepted_per_slot.setdefault(cell["slot"], 0)
        lines, worst = [], "ok"
        for backend in ("interp", "c"):
            if backend not in per:
                continue
            if backend == "c" and worst == "REFUSED":
                continue
            v, why = verdict(cell, per[backend])
            if backend == "c" and v == "REFUSED":
                v, why = "FAIL", f"only the C backend refuses it: {why}"
            if v != "ok":
                lines.append((backend, v, why))
                worst = v if worst == "ok" or v == "FAIL" else worst
        tally[worst] += 1
        if worst != "REFUSED":
            accepted_per_slot[cell["slot"]] += 1
        for backend, v, why in lines:
            if v == "FAIL":
                causes.setdefault((cell["slot"], re.sub(r"\d+ time", "N time", why)[:90]), []).append(cell["name"])
            if v == "FAIL" or verbose:
                print(f"{v:8} {cell['name']} [{backend}]: {why[:200]}")
        if worst == "ok" and verbose:
            print(f"ok       {cell['name']}")
    vacuous = sorted(s for s, n in accepted_per_slot.items() if n == 0)
    for s in vacuous:
        print(f"VACUOUS  {s}: no cell of this slot compiled, so it checks nothing")
    if causes:
        print("failing cells by slot and reason:")
        for (slot, why), names in sorted(causes.items()):
            print(f"  {len(names):4}  {slot}: {why}")
    orders = sum(1 for c in cells if c["log"] is not None)
    print(f"slot_matrix: {len(cells)} cells ({orders} check order) on {' and '.join(per)}: "
          f"{tally['ok']} ok, {tally['FAIL']} failing, {tally['REFUSED']} refused, "
          f"{len(vacuous)} slot(s) vacuous  [{loft_binary()}]")
    return tally["FAIL"] + len(vacuous)


def backends_of(args):
    return ("interp", "c") if args.backend == "both" else (args.backend,)


def cmd_run(args):
    slots, fillers, _, problems = load_all()
    if problems:
        print("\n".join(problems))
        return 1
    cells = make_cells(slots, fillers, args.filler, set(args.slot), args.kind)
    return 1 if run_matrix(cells, backends_of(args), args.verbose) else 0


def cmd_fillers(args):
    """Each filler in its type's neutral slot: one bump of its counter, the plain value."""
    slots, fillers, _, problems = load_all()
    if problems:
        print("\n".join(problems))
        return 1
    cells = make_cells(slots, fillers, "all", set(NEUTRAL.values()), KINDS[0][0])
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
    p.set_defaults(fn=cmd_run)
    args = ap.parse_args()
    sys.exit(args.fn(args))


if __name__ == "__main__":
    main()
