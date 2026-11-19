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
    run        run cells on the interpreter and compare each operand's run count with
               the slot's `runs` column.  Default: every slot with its plain variable,
               which validates the register itself.  `--filler all` is the matrix.

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


def cell_source(prelude, slot, filler, kind_type, name):
    expr = filler["expr"].replace(COUNTER, "c")
    src = prelude.replace("KIND", kind_type).replace("CELL", name)
    src = src.replace("// DECL", slot["decl"].replace(SLOT, expr))
    return src.replace("// STMT", "  " + slot["stmt"].replace(SLOT, expr))


def run_cell(job):
    """Run one cell.  Returns (name, verdict, detail): ok | FAIL | REFUSED."""
    name, source, want, tmp = job
    path = os.path.join(tmp, name + ".loft")
    with open(path, "w", encoding="utf-8") as f:
        f.write(source)
    env = dict(os.environ, LOFT_TIMEOUT="120")
    try:
        done = subprocess.run([loft_binary(), path], capture_output=True, text=True,
                              env=env, timeout=180, check=False)
    except subprocess.TimeoutExpired:
        return name, "FAIL", "no answer in 180 s"
    out = done.stdout + done.stderr
    m = re.search(r"^%s c=(\d+) d=\d+ \| (.*)$" % re.escape(name), out, re.M)
    if not m:
        first = next((ln for ln in out.splitlines() if ln.strip()), "no output")
        return name, "REFUSED", first.replace(tmp + os.sep, "")[:200]
    got = int(m.group(1))
    if got != want:
        return name, "FAIL", f"the operand ran {got} time(s), the slot says {want}"
    return name, "ok", m.group(2)


def cells(slots, fillers, which_filler, which_slot, which_kind):
    prelude = open(PRELUDE, encoding="utf-8").read()
    for slot in slots:
        if which_slot and slot["id"] not in which_slot:
            continue
        kinds = KINDS if slot["type"] == "coll" else KINDS[:1]
        for kind, kind_type in kinds:
            if which_kind and kind != which_kind:
                continue
            for f in fillers:
                if f["type"] != slot["type"]:
                    continue
                is_var = f["id"].endswith("-var")
                if which_filler == "var" and not is_var:
                    continue
                if which_filler not in ("var", "all") and f["id"] != which_filler:
                    continue
                name = f"{slot['id']}.{f['id']}.{kind}"
                want = 0 if is_var else int(slot["runs"])
                yield name, cell_source(prelude, slot, f, kind_type, name), want, slot["id"]


def run_jobs(jobs):
    with tempfile.TemporaryDirectory(prefix="slot_matrix.") as tmp:
        with concurrent.futures.ThreadPoolExecutor(max_workers=os.cpu_count() or 4) as pool:
            return list(pool.map(run_cell, [(n, s, w, tmp) for n, s, w, _ in jobs]))


def cmd_run(args):
    slots, fillers, _, problems = load_all()
    if problems:
        print("\n".join(problems))
        return 1
    jobs = list(cells(slots, fillers, args.filler, set(args.slot), args.kind))
    results = run_jobs(jobs)
    accepted = {}
    tally = {"ok": 0, "FAIL": 0, "REFUSED": 0}
    for (name, _, _, slot_id), (_, verdict, detail) in zip(jobs, results):
        tally[verdict] += 1
        accepted.setdefault(slot_id, 0)
        if verdict != "REFUSED":
            accepted[slot_id] += 1
        if verdict != "ok" or args.verbose:
            print(f"{verdict:8} {name}: {detail}")
    vacuous = sorted(s for s, n in accepted.items() if n == 0)
    for s in vacuous:
        print(f"VACUOUS  {s}: no cell of this slot compiled, so it checks nothing")
    print(f"slot_matrix run: {len(jobs)} cells, {tally['ok']} ok, {tally['FAIL']} failing, "
          f"{tally['REFUSED']} refused, {len(vacuous)} slot(s) vacuous  [{loft_binary()}]")
    return 1 if tally["FAIL"] or vacuous else 0


def cmd_fillers(args):
    """Each filler in its type's neutral slot: one bump of its counter, the plain value."""
    slots, fillers, _, problems = load_all()
    if problems:
        print("\n".join(problems))
        return 1
    by_id = {s["id"]: s for s in slots}
    prelude = open(PRELUDE, encoding="utf-8").read()
    jobs = []
    for f in fillers:
        slot = by_id[NEUTRAL[f["type"]]]
        name = f"filler.{f['id']}"
        want = 0 if f["id"].endswith("-var") else 1
        jobs.append((name, cell_source(prelude, slot, f, KINDS[0][1], name), want, f["type"]))
    results = dict((n, (v, d)) for n, v, d in run_jobs(jobs))
    bad = 0
    for f in fillers:
        verdict, detail = results[f"filler.{f['id']}"]
        plain = results[f"filler.{f['type']}-var"]
        if verdict == "ok" and detail != plain[1]:
            verdict, detail = "FAIL", "it gives another value than its type's variable"
        if verdict != "ok":
            bad += 1
        if verdict != "ok" or args.verbose:
            print(f"{verdict:8} {f['id']}: {detail[:160]}")
    print(f"slot_matrix fillers: {len(fillers)} fillers, {bad} not usable  [{loft_binary()}]")
    return 1 if bad else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("check").set_defaults(fn=cmd_check)
    sub.add_parser("selftest").set_defaults(fn=cmd_selftest)
    sub.add_parser("list").set_defaults(fn=cmd_list)
    p = sub.add_parser("fillers")
    p.add_argument("-v", "--verbose", action="store_true")
    p.set_defaults(fn=cmd_fillers)
    p = sub.add_parser("run")
    p.add_argument("--filler", default="var", help="`var` (default), `all`, or one filler id")
    p.add_argument("--slot", action="append", default=[], help="one slot id; may repeat")
    p.add_argument("--kind", choices=[k for k, _ in KINDS])
    p.add_argument("-v", "--verbose", action="store_true")
    p.set_defaults(fn=cmd_run)
    args = ap.parse_args()
    sys.exit(args.fn(args))


if __name__ == "__main__":
    main()
