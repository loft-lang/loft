#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The slot matrix's SEQUENCE axis: a few statements over one local, every order of them
(doc/claude/SLOT_MATRIX.md § The sequence axis).  Run through `slot_matrix.py seq …`.

A single statement with every form of operand misses the defects that need two statements
to meet: a first bind and a rebind of one local, an early return ahead of the bind a later
one renames, a loop pass that keeps a record and a return before the next mint, a local
answered on one arm of a join.  A cell here is a function

    fn sq_N(p: T, k: integer, h: HoldT) -> T     (or the type of a projection of T)

whose body is drawn from the statement shapes of scripts/syntax_sequence.tsv, over the four
types and the callees of scripts/syntax_sequence_prelude.loft.  The driver calls it for two
parameters (one a callee hands back, one it replaces) and five values of `k` (each branch
taken, and none), and prints what came back, the parameter and the holder.

The answer is not compared with a twin: the generator evaluates the same statements in its
own model of the values (records numbered n, every bind a copy: binding.md `(B-Copy)`,
calls.md `(F-Ret)`), so a value that is wrong in every spelling is still wrong here.  Every
program runs with `LOFT_LEAK_CHECK`, `LOFT_NATIVE_LEAK_CHECK`, `LOFT_STRICT_STORES=1` and
`LOFT_POISON=1`, on the interpreter and the C backend.

    seq list    the shapes and the size of the product, per type and length.
    seq run     the product, sampled and seeded when it is larger than `--sample`.
    seq cell    one sequence, its statements as loft or as shape ids:
                    seq cell 'x = lw_<t>(p);' 'if k == 0 { return x; }' 'y = w_<t>(x);' 'y'
                the last is the tail; `<t>` stands for every type.
    seq walk    the product on an older build beside this one (`--old <binary>`).
    seq gate    a fixed sample against scripts/slot_sequence.baseline; `--update` rewrites it.

@falsified-at: hand-measured — `LOFT=<origin/main ebff34f71> slot_matrix.py seq run` fails cells
of loft#2671, #2673, #2674, #2677, #2681 and #2684; on the build with their fixes they pass.
"""
import itertools
import os
import random
import re
import sys
import tempfile

import slot_matrix as sm

HERE = os.path.dirname(os.path.abspath(__file__))
SHAPES_TSV = os.path.join(HERE, "syntax_sequence.tsv")
PRELUDE = os.path.join(HERE, "syntax_sequence_prelude.loft")
BASELINE = os.path.join(HERE, "slot_sequence.baseline")
CLASSES = ("src", "isrc", "ret", "stmt", "end")
NUMBERED = re.compile(r"<(LIT|LITO|N|K|RLIT)>")
INNER = ("<SRC>", "<ISRC>", "<RET>")
PARAMS = (4, 7)                  # one a callee hands back (even), one it replaces (odd)
KS = range(5)                    # every branch of a cell taken, and none
CHECKS = {"LOFT_STRICT_STORES": "1", "LOFT_POISON": "1", "LOFT_NATIVE_LEAK_CHECK": "1"}
# The gate's sample: fixed, so that its baseline names the same cells on every machine.  The
# interpreter runs all of it; the C backend, which compiles every cell (a quarter of a second
# each on a loaded machine), every `c_every`-th unless `--full`.
GATE = {"length": 3, "sample": 1500, "seed": 1, "c_every": 6}


# --- the values -------------------------------------------------------------------------
# A value is the record numbered n, held as a tuple so that every bind is a copy.

def _vec(xs):
    return "[" + ",".join(str(x) for x in xs) + "]"


class Type:
    def __init__(self, letter, name, lit, ilit, make, num, bump, show, projections):
        self.letter, self.name, self.lit, self.ilit = letter, name, lit, ilit
        self.make, self.num, self.bump, self.show = make, num, bump, show
        # projection suffix -> (loft type, show function, model, show of the model, literal, its value)
        self.projections = projections


TYPES = {
    "p": Type("p", "SqP", lambda n: f"SqP {{ v: {n} }}", "SqP { v: i + 20 }",
              lambda n: (n,), lambda v: v[0], lambda v: (v[0] + 1,), lambda v: str(v[0]),
              {".v": ("integer", "show_i", lambda v: v[0], str, "5", 5)}),
    "r": Type("r", "SqR", lambda n: f"SqR {{ v: {n}, at: SqAt {{ a: {n + 1} }}, xs: [{n}, {n + 2}] }}",
              "SqR { v: i + 20, at: SqAt { a: i + 21 }, xs: [i + 20, i + 22] }",
              lambda n: (n, n + 1, (n, n + 2)), lambda v: v[0], lambda v: (v[0] + 1, v[1], v[2]),
              lambda v: f"{v[0]}/{v[1]}/{_vec(v[2])}",
              {".at": ("SqAt", "show_at", lambda v: v[1], str, "SqAt { a: 5 }", 5),
               ".xs": ("vector<integer>", "show_v", lambda v: v[2], _vec, "[5, 7]", (5, 7))}),
    "e": Type("e", "SqE", lambda n: f"S {{ v: {n}, inner: A {{ x: {n + 1} }} }}",
              "S { v: i + 20, inner: A { x: i + 21 } }",
              lambda n: ("S", n, ("A", n + 1)), lambda v: v[1] if v[0] == "S" else None, None,
              lambda v: f"S{v[1]}/A{v[2][1]}" if v[0] == "S" else f"A{v[1]}", {}),
    "l": Type("l", "vector<integer>", lambda n: f"[{n}, {n + 2}]", "[i + 20, i + 22]",
              lambda n: (n, n + 2), lambda v: v[0], lambda v: (v[0] + 1, v[1]), _vec,
              {"[0]": ("integer", "show_i", lambda v: v[0], str, "5", 5)}),
}


# --- the table ----------------------------------------------------------------------------

def load_shapes(path=SHAPES_TSV):
    """The rows of the shape table, and what is wrong with it."""
    shapes, problems = [], []
    for nr, cols in sm.rows(path):
        where = f"{os.path.basename(path)}:{nr}"
        if len(cols) != 5:
            problems.append(f"{where}: want 5 tab-separated columns, found {len(cols)}")
            continue
        cls, sid, types, loft, model = cols
        if cls not in CLASSES:
            problems.append(f"{where}: class `{cls}` is none of {' '.join(CLASSES)}")
        if types != "*" and set(types) - set(TYPES):
            problems.append(f"{where}: types `{types}`: want `*` or letters of {''.join(TYPES)}")
        if NUMBERED.findall(loft) != NUMBERED.findall(model):
            problems.append(f"{where}: the numbered holes differ between the loft {NUMBERED.findall(loft)} "
                            f"and the model {NUMBERED.findall(model)}")
        for hole in INNER:
            if (hole in loft) != (hole in model):
                problems.append(f"{where}: `{hole}` is in one of the loft and the model only")
        try:
            parse(model)
        except ValueError as e:
            problems.append(f"{where}: the model does not read: {e}")
        shapes.append({"cls": cls, "id": sid, "types": TYPES if types == "*" else types,
                       "loft": loft, "model": model, "line": nr})
    for cls in CLASSES:
        if not any(s["cls"] == cls for s in shapes):
            problems.append(f"{os.path.basename(path)}: no row of class `{cls}`")
    return shapes, problems


def parse(text):
    """A model s-expression as nested lists of atoms."""
    tokens = re.findall(r"\(|\)|[^\s()]+", text)
    pos = 0

    def one():
        nonlocal pos
        if pos >= len(tokens):
            raise ValueError("it ends early")
        tok = tokens[pos]
        pos += 1
        if tok == ")":
            raise ValueError("a `)` with no `(`")
        if tok != "(":
            return tok
        out = []
        while pos < len(tokens) and tokens[pos] != ")":
            out.append(one())
        if pos >= len(tokens):
            raise ValueError("a `(` is not closed")
        pos += 1
        return out
    got = one()
    if pos != len(tokens):
        raise ValueError("more than one expression")
    return got


# --- the variants: a row with its inner holes filled ---------------------------------------

def variants(shapes, letter, pj):
    """{class: [(id, loft, model)]} for one type and projection: every src, isrc and ret,
    and every stmt and end with its `<SRC>`, `<ISRC>` and `<RET>` filled."""
    ty = TYPES[letter]
    proj = ty.projections.get(pj)
    mine = [s for s in shapes if letter in s["types"]]

    def put(text, model=False):
        text = text.replace("<PJ>", pj).replace("<t>", letter)
        if model:
            return text.replace("<RLIT>", "(plit)" if proj else "<LIT>")
        return text.replace("<RLIT>", proj[4] if proj else "<LIT>").replace("<ILIT>", ty.ilit)

    plain = {cls: [(s["id"], put(s["loft"]), put(s["model"], True)) for s in mine if s["cls"] == cls]
             for cls in ("src", "isrc", "ret")}
    out = {"src": plain["src"], "isrc": plain["isrc"], "ret": plain["ret"], "stmt": [], "end": []}
    for s in mine:
        if s["cls"] not in ("stmt", "end"):
            continue
        if s["cls"] == "end" and "<PJ>" not in s["loft"] and pj:
            continue
        holes = [h for h in INNER if h in s["loft"]]
        pools = [plain[{"<SRC>": "src", "<ISRC>": "isrc", "<RET>": "ret"}[h]] for h in holes]
        for combo in itertools.product(*pools):
            loft, model, ids = put(s["loft"]), put(s["model"], True), [s["id"]]
            for hole, (iid, il, im) in zip(holes, combo):
                loft, model = loft.replace(hole, il, 1), model.replace(hole, im, 1)
                ids.append(iid)
            vid = ":".join(ids) if s["cls"] == "stmt" else ids[0]
            # A projection returns the projection, so a statement that returns is made per one.
            out[s["cls"]].append((vid, loft, model))
    return out


def names_in(expr):
    """The locals an expression of the model reads."""
    if isinstance(expr, str):
        return {expr} & {"x", "y"}
    return set().union(*(names_in(e) for e in expr)) if expr else set()


def scoped(stmts, end):
    """Whether every statement reads only locals bound before it, as loft's scopes say: a
    local first bound in a loop's body is gone after the loop.  The first must bind `x`."""
    bound = set()
    first = parse(stmts[0][2])
    if first[0] == "set" and first[1] != "x":
        return False
    if first[0] not in ("set", "loop"):
        return False
    for _, _, model in stmts:
        m = parse(model)
        op = m[0]
        if op == "set":
            if not names_in(m[2]) <= bound:
                return False
            bound.add(m[1])
        elif op == "loop":
            kind, src = m[1], m[2]
            ret = m[3] if len(m) > 3 else None
            if not names_in(src) <= bound:
                return False
            if ret is not None and not names_in(ret) <= (bound | ({"x"} if kind == "post" else set())):
                return False
        elif not names_in(m[1:]) <= bound:
            return False
    return names_in(parse(end[2])) <= bound


# --- a cell -------------------------------------------------------------------------------

def number(loft, model, ty):
    """The holes of a cell's text numbered left to right, the same in the loft and the model."""
    counts = {"n": 30, "k": -1}

    def take(kind):
        if kind == "K":
            counts["k"] += 1
            return counts["k"]
        counts["n"] += 2
        return counts["n"] + (1 if kind == "LITO" else 0)
    loft_vals = []

    def in_loft(m):
        val = take(m.group(1))
        loft_vals.append(val)
        return ty.lit(val) if m.group(1) in ("LIT", "LITO") else str(val)
    loft = NUMBERED.sub(in_loft, loft)
    vals = iter(loft_vals)

    def in_model(m):
        val = next(vals)
        return f"#{val}" if m.group(1) in ("LIT", "LITO") else str(val)
    return loft, NUMBERED.sub(in_model, model)


def make_cell(letter, pj, stmts, end):
    """A cell from its type, its projection, its statements and its tail (each (id, loft, model))."""
    ty = TYPES[letter]
    sep = "\x00"
    loft, model = number(sep.join([s[1] for s in stmts] + [end[1]]), sep.join([s[2] for s in stmts] + [end[2]]), ty)
    lines, models = loft.split(sep), [parse(m) for m in model.split(sep)]
    name = f"{letter}{pj}|" + "|".join(s[0] for s in stmts) + f"|={end[0]}"
    cell = {"name": name, "letter": letter, "pj": pj, "lines": lines, "models": models,
            "ids": [s[0] for s in stmts] + ["=" + end[0]], "kind_type": letter,
            "want": 0, "log": None, "base": None, "slot": "seq", "decl": "",
            "stmt": " ".join(lines), "parts": (letter, pj, list(stmts), end)}
    cell["expect"] = expect(cell)
    return cell


class Return(Exception):
    def __init__(self, value):
        super().__init__()
        self.value = value


UNBOUND = object()


def evaluate_model(cell, pv, k):
    """(what the call returns, the parameter after it, the holder after it), shown as the
    driver shows them."""
    ty = TYPES[cell["letter"]]
    proj = ty.projections.get(cell["pj"])
    env = {"p": ty.make(pv), "x": UNBOUND, "y": UNBOUND, "i": None}
    hold = [ty.make(0), []]

    def w(v):
        n = ty.num(v)
        return ty.make(n + 100) if n is not None and n % 2 else v

    def ev(e):
        if isinstance(e, str):
            if e in ("p", "x", "y"):
                if env[e] is UNBOUND:
                    raise AssertionError(f"`{e}` read before a bind in {cell['name']}")
                return env[e]
            if e.startswith("#"):
                return ty.make(int(e[1:]))
            return int(e)
        op, args = e[0], e[1:]
        if op == "mk":
            return ty.make(ev(args[0]))
        if op == "id":
            return ev(args[0])
        if op == "w":
            return w(ev(args[0]))
        if op == "alt":
            v = ev(args[0])
            return w(v) if w(v) is not v else None
        if op == "??":
            v = ev(args[0])
            return ev(args[1]) if v is None else v
        if op == "if":
            return ev(args[1]) if k == ev(args[0]) else ev(args[2])
        if op == "pj":
            v = ev(args[0])
            return proj[2](v) if proj else v
        if op == "plit":
            return proj[5]
        if op == "ilit":
            return ty.make(env["i"] + 20)
        raise AssertionError(f"no model for `{op}`")

    def run(m):
        op = m[0]
        if op == "set":
            env[m[1]] = ev(m[2])
        elif op == "field":
            hold[0] = ev(m[1])
        elif op == "elem":
            hold[1].append(ev(m[1]))
        elif op == "bump":
            env["x"] = ty.bump(env["x"])
        elif op == "drop":
            for a in m[1:]:
                ev(a)
        elif op == "ret":
            if k == ev(m[1]):
                raise Return(ev(m[2]))
        elif op == "loop":
            kind, src = m[1], m[2]
            outer = env["x"]
            for i in range(3):
                env["i"] = i
                if kind == "pre" and i == k:
                    raise Return(ev(m[3]))
                env["x"] = ev(src)
                if kind == "post" and i == k:
                    raise Return(ev(m[3]))
            if outer is UNBOUND:            # a local first bound in the body ends with it
                env["x"] = UNBOUND
        else:
            raise AssertionError(f"no model for the statement `{op}`")

    try:
        for m in cell["models"][:-1]:
            run(m)
        got = ev(cell["models"][-1])
    except Return as r:
        got = r.value
    shown = (proj[3] if proj else ty.show)(got)
    held = ty.show(hold[0]) + "".join("+" + ty.show(e) for e in hold[1])
    return f"{shown} {ty.show(env['p'])} {held}"


def expect(cell):
    return ";".join(evaluate_model(cell, pv, k) for pv in PARAMS for k in KS) + ";"


# --- the program ----------------------------------------------------------------------------

_PRELUDE = {}


def prelude():
    if not _PRELUDE:
        text = open(PRELUDE, encoding="utf-8").read()
        head, _, below = text.partition("// === BELOW ===\n")
        _PRELUDE.update(head=head, below=below)
    return _PRELUDE["head"], _PRELUDE["below"]


def cell_text(cell, n):
    """The cell's function and its test function."""
    ty = TYPES[cell["letter"]]
    proj = ty.projections.get(cell["pj"])
    result, show = (proj[0], proj[1]) if proj else (ty.name, f"show_{ty.letter}")
    t = ty.letter
    body = "".join(f"  {line}\n" for line in cell["lines"])
    return (f"fn sq_{n}(p: {ty.name}, k: integer, h: Hold{t.upper()}) -> {result} {{\n{body}}}\n"
            f"fn test_c{n:04d}() {{\n"
            f"  out = \"\";\n"
            f"  for pv in [{', '.join(map(str, PARAMS))}] {{\n"
            f"    for k in 0..{len(KS)} {{\n"
            f"      h = hold_{t}();\n"
            f"      p = mk_{t}(pv);\n"
            f"      r = sq_{n}(p, k, h);\n"
            f"      out += \"{{{show}(r)}} {{show_{t}(p)}} {{show_hold_{t}(h)}};\";\n"
            f"    }}\n"
            f"  }}\n"
            f"  println(\"{cell['name']} c=0 log=<> | {{out}}\");\n"
            f"}}\n")


def program(_kind, cells):
    """One loft program holding `cells` as test functions; per cell, the lines it occupies."""
    head, below = prelude()
    text, spans = head, []
    for n, cell in enumerate(cells):
        first = text.count("\n") + 1
        text += cell_text(cell, n)
        spans.append((first, text.count("\n"), f"test_c{n:04d}"))
    return text + below, spans


# --- the product --------------------------------------------------------------------------

def pjs_of(letter, projections):
    return [""] + (list(TYPES[letter].projections) if projections else [])


def space(shapes, letters, length, projections=True):
    """Per (type, projection, length): the first statements, the others, the tails."""
    out = []
    for letter in letters:
        for pj in pjs_of(letter, projections):
            v = variants(shapes, letter, pj)
            firsts = [s for s in v["stmt"] if parse(s[2])[0] == "loop" or
                      (parse(s[2])[0] == "set" and parse(s[2])[1] == "x" and not names_in(parse(s[2])[2]))]
            for n in range(1, length + 1):
                out.append((letter, pj, n, firsts, v["stmt"], v["end"]))
    return out


def product(shapes, letters, length, sample, seed, projections=True):
    """The cells of a run: each (type, projection, length) whole when the budget holds it,
    else a seeded sample of it; the budget is shared equally, the small parts first, and what
    a part leaves goes to the rest.  Returns (cells, the size of the whole product)."""
    parts = space(shapes, letters, length, projections)
    sizes = [len(f) * len(s) ** (n - 1) * len(e) for (_, _, n, f, s, e) in parts]
    total = sum(sizes)
    rng = random.Random(seed)
    cells, left = [], sample
    order = sorted(range(len(parts)), key=lambda i: sizes[i])
    for done, i in enumerate(order):
        letter, pj, n, firsts, stmts, ends = parts[i]
        share = left // (len(parts) - done) if sample else None
        picked = []
        if share is None or sizes[i] <= share:
            for combo in itertools.product(firsts, *([stmts] * (n - 1)), ends):
                if scoped(combo[:-1], combo[-1]):
                    picked.append(combo)
        else:
            seen, tries = set(), 0
            while len(picked) < share and tries < share * 40:
                tries += 1
                combo = (rng.choice(firsts),) + tuple(rng.choice(stmts) for _ in range(n - 1)) + (rng.choice(ends),)
                key = tuple(c[0] for c in combo)
                if key in seen or not scoped(combo[:-1], combo[-1]):
                    continue
                seen.add(key)
                picked.append(combo)
        if share is not None:
            left -= min(len(picked), left)
        cells += [make_cell(letter, pj, list(c[:-1]), c[-1]) for c in picked]
    return cells, total


# --- running -------------------------------------------------------------------------------

def evaluate(cells, backends, on_c=None):
    """{cell name: (verdict, [(backend, verdict, reason)])}, as the slot matrix's own.  With
    `on_c`, only the cells it names go to the C backend."""
    sm.EXTRA_ENV.update(CHECKS)
    per = {}
    with tempfile.TemporaryDirectory(prefix="slot_sequence.") as tmp:
        per["interp"] = sm.run_backend("interp", cells, tmp, program)
        if "c" in backends:
            accepted = [c for c in cells if per["interp"].get(c["name"], ("",))[0] != "REFUSED"
                        and (on_c is None or c["name"] in on_c)]
            per["c"] = sm.run_backend("c", accepted, tmp, program)
    out = {}
    for cell in cells:
        lines, worst = [], "ok"
        for backend in backends:
            if backend == "c" and (worst == "REFUSED" or (on_c is not None and cell["name"] not in on_c)):
                continue
            v, why = verdict(cell, per[backend])
            if backend == "c" and v == "REFUSED":
                v, why = "FAIL", f"only the c backend refuses it: {why}"
            if v != "ok":
                lines.append((backend, v, why))
                worst = v if worst == "ok" or v == "FAIL" else worst
        out[cell["name"]] = (worst, lines)
    return out


def verdict(cell, results):
    got = results.get(cell["name"])
    if got and got[0] == "RAN" and got[3] != cell["expect"]:
        calls = [(pv, k) for pv in PARAMS for k in KS]
        have, want = got[3].split(";"), cell["expect"].split(";")
        off = [n for n in range(len(want)) if n >= len(have) or have[n] != want[n]]
        n = off[0] if off else 0
        pv, k = calls[n] if n < len(calls) else ("?", "?")
        tail = f"; and a store is left behind or freed twice: {got[4]}" if got[4] else ""
        return "FAIL", (f"the result differs from the model: with p={pv} k={k} it gives "
                        f"`{have[n] if n < len(have) else '(nothing)'}`, the model `{want[n]}` "
                        f"(result, parameter, holder){' and ' + str(len(off) - 1) + ' more call(s)' if len(off) > 1 else ''}"
                        + tail)
    v, why = sm.verdict(cell, results)
    if v == "FAIL" and got and got[0] == "RAN" and got[4] and "freed twice" not in why and "not freed" not in why:
        why = "a store-lifetime check fails: " + why
    return v, why


def channel(why):
    for word, name in (("differs from the model", "value"), ("freed twice", "double-free"),
                       ("not freed", "leak"), ("NEVER FREED", "leak"), ("left behind", "leak"),
                       ("use after free", "use-after-free"), ("poison", "use-after-free"),
                       ("does not finish", "loop"), ("refuses", "refused")):
        if word in why:
            return name
    return "crash"


def features(cell):
    """What a cell is made of, for the fold: its type, its projection, each statement shape
    with and without what fills its holes, its tail, and each two shapes that follow one
    another (the defects this axis is for are two statements meeting)."""
    out = {f"type {cell['letter']}"}
    if cell["pj"]:
        out.add(f"projection {cell['pj']}")
    ids = cell["ids"]
    for sid in ids:
        row = sid.split(":")[0]
        out.add(f"shape {sid}")
        out.add(f"shape {row}")
        for part in sid.split(":")[1:]:
            out.add(f"{row} of {part}")
    for a, b in zip(ids, ids[1:]):
        out.add(f"{a} then {b}")
        out.add(f"{a.split(':')[0]} then {b.split(':')[0]}")
    return out


def fold(cells, verdicts, universe):
    """The failing cells gathered toward causes: repeatedly the shape whose cells fail most
    often (smoothed, so that two of two does not beat thirty of forty), with its smallest
    failing cell.  `universe` is every cell of the run, which says how many have a shape."""
    failing = [c for c in cells if verdicts[c["name"]][0] == "FAIL"]
    have = {}
    for c in universe:
        for f in features(c):
            have[f] = have.get(f, 0) + 1
    out, left = [], list(failing)
    while left:
        hits = {}
        for c in left:
            for f in features(c):
                hits.setdefault(f, []).append(c)
        best = max(sorted(hits), key=lambda f: (len(hits[f]) >= 2 or len(left) == 1,
                                                len(hits[f]) / (have.get(f, len(hits[f])) + 2),
                                                len(hits[f])))
        mine = hits[best]
        chans = sorted({channel(why) for c in mine for _, v, why in verdicts[c["name"]][1] if v == "FAIL"})
        backs = sorted({b for c in mine for b, v, _ in verdicts[c["name"]][1] if v == "FAIL"})
        small = min(mine, key=lambda c: (len(c["ids"]), len(" ".join(c["lines"]))))
        out.append((best, len(mine), have.get(best, len(mine)), chans, backs, small))
        left = [c for c in left if c not in mine]
    return out


_SHAPES = []


def shapes_cached():
    if not _SHAPES:
        _SHAPES.append(loaded())
    return _SHAPES[0]


def positions():
    """Each shape's place in its class, the order of the table: earlier is simpler."""
    pos = {}
    for s in shapes_cached():
        pos.setdefault(s["cls"], {}).setdefault(s["id"], len(pos[s["cls"]]))
    return pos


def part_classes(row_id):
    """The classes of what fills a statement row's holes, in the order its id names them."""
    row = next(s for s in shapes_cached() if s["cls"] == "stmt" and s["id"] == row_id)
    return [{"<SRC>": "src", "<ISRC>": "isrc", "<RET>": "ret"}[h] for h in INNER if h in row["loft"]]


def complexity(cell):
    """(statements, the table places of every shape in it, a projection): smaller is simpler."""
    pos = positions()
    score = 0
    for sid in cell["ids"][:-1]:
        row, *parts = sid.split(":")
        score += pos["stmt"][row] + sum(pos[c][p] for c, p in zip(part_classes(row), parts))
    score += pos["end"][cell["ids"][-1][1:]]
    return (len(cell["ids"]), score, bool(cell["pj"]))


def smaller(cell):
    """The simpler cells next to this one: a statement left out, the projection left out, a
    simpler tail, a simpler shape in one hole.  Only those that are still cells."""
    shapes, pos = shapes_cached(), positions()
    head = cell["letter"] + cell["pj"]
    ids = cell["ids"][:-1]
    end = cell["ids"][-1]
    names = set()
    for n in range(len(ids)):
        names.add("|".join([head] + ids[:n] + ids[n + 1:] + [end]))
    if cell["pj"]:
        names.add("|".join([cell["letter"]] + ids + [end]))
    for e, at in pos["end"].items():
        if at < pos["end"][end[1:]]:
            names.add("|".join([head] + ids + ["=" + e]))
    for n, sid in enumerate(ids):
        row, *parts = sid.split(":")
        for j, (cls, part) in enumerate(zip(part_classes(row), parts)):
            for other, at in pos[cls].items():
                if at < pos[cls][part]:
                    new = ":".join([row] + parts[:j] + [other] + parts[j + 1:])
                    names.add("|".join([head] + ids[:n] + [new] + ids[n + 1:] + [end]))
    out = []
    for name in sorted(names):
        small = cell_of_name(shapes, name)
        if small:
            out.append(small)
    return out


def failing_as(verdict_lines):
    """What a failure is, for asking whether a smaller cell fails the same way."""
    return {(b, channel(why)) for b, v, why in verdict_lines if v == "FAIL"}


def shrink(cells, verdicts, backends):
    """Each failing cell shrunk to a simplest cell that fails the same way (on a backend it
    failed on, through a channel it failed through): the simpler cells next to it are run,
    the simplest failing one taken, until none of its neighbours fails.  Returns {cell
    name: (simplest cell, its verdict lines)}."""
    cur = {c["name"]: (c, verdicts[c["name"]][1]) for c in cells if verdicts[c["name"]][0] == "FAIL"}
    known = {}
    active = set(cur)
    while active:
        tries = {}
        for name in active:
            for small in smaller(cur[name][0]):
                if small["name"] not in known:
                    tries.setdefault(small["name"], small)
        if tries:
            known.update(evaluate(list(tries.values()), backends))
        still = set()
        for name in active:
            cell, lines = cur[name]
            want = failing_as(lines)
            hit = [t for t in smaller(cell) if failing_as(known[t["name"]][1]) & want
                   and complexity(t) < complexity(cell)]
            if hit:
                best = min(hit, key=complexity)
                cur[name] = (best, known[best["name"]][1])
                still.add(name)
        active = still
    return cur


def report(cells, verdicts, backends, verbose, total=None, shrunk=None, listing=None):
    """Print what is not ok: every failing cell with -v; else the smallest cells the failing
    ones shrink to, each once, and the fold of those.  Returns the number of failing cells."""
    tally = {"ok": 0, "FAIL": 0, "REFUSED": 0}
    for cell in cells:
        worst, lines = verdicts[cell["name"]]
        tally[worst] += 1
        if verbose or shrunk is None:
            for backend, v, why in lines:
                if v == "FAIL" or verbose:
                    print(f"{v:8} {cell['name']} [{backend}]: {why[:300]}")
            if worst == "FAIL" or (verbose and worst == "REFUSED"):
                print("         " + " ".join(cell["lines"]))
    smallest = {}
    if shrunk:
        for name, (small, lines) in shrunk.items():
            smallest.setdefault(small["name"], (small, lines, []))[2].append(name)
        groups = {}
        for key, (small, lines, of) in smallest.items():
            groups.setdefault("|".join(small["ids"]), []).append((small, lines, of))
        print(f"the {tally['FAIL']} failing cells shrink to {len(smallest)} simplest cell(s), "
              f"{len(groups)} sequence(s) over the types:")
        for sig, members in sorted(groups.items(), key=lambda kv: (-sum(len(m[2]) for m in kv[1]), kv[0])):
            small, lines, _ = min(members, key=lambda m: complexity(m[0]))
            kinds = " ".join(sorted(m[0]["letter"] + m[0]["pj"] for m in members))
            chans = "+".join(sorted({f"{b} {ch}" for m in members for b, ch in failing_as(m[1])}))
            print(f"  {sig}   [{kinds}] ({chans}; {sum(len(m[2]) for m in members)} cell(s) shrink to it)")
            why = next((w for b, v, w in lines if v == "FAIL"), "")
            print(f"      {small['name']}: {why[:240]}")
            print(f"        {' '.join(small['lines'])}")
        universe = cells + [s for s, _, _ in smallest.values()]
        small_verdicts = dict(verdicts, **{k: ("FAIL", v[1]) for k, v in smallest.items()})
        folds = fold([s for s, _, _ in smallest.values()], small_verdicts, universe)
    else:
        folds = fold(cells, verdicts, cells)
    if folds:
        print(f"the failing cells fold to {len(folds)} group(s):")
        for f, n, of, chans, backs, small in folds:
            print(f"  {f}: {n} failing of {of} cell(s) ({'+'.join(chans)}; {' '.join(backs)})")
            print(f"      smallest: {small['name']}")
            print(f"        {' '.join(small['lines'])}")
    if listing:
        with open(listing, "w", encoding="utf-8") as f:
            for name, (small, lines, of) in sorted(smallest.items()):
                f.write(name + "\n")
            if not shrunk:
                for cell in cells:
                    if verdicts[cell["name"]][0] == "FAIL":
                        f.write(cell["name"] + "\n")
        print(f"the failing cells' names are in {listing} (`seq run --cells {listing}` runs them again)")
    sized = f" of a product of {total}" if total and total > len(cells) else ""
    print(f"slot_matrix seq: {len(cells)} cells{sized} on {' and '.join(backends)}: {tally['ok']} ok, "
          f"{tally['FAIL']} failing, {tally['REFUSED']} refused  [{sm.build()}]")
    print("          cost: " + "; ".join(f"{b} {sm.SPENT[b][0]} program run(s), {sm.SPENT[b][1]:.0f} s"
                                         for b in backends))
    if tally["FAIL"]:
        print("one alone, its program kept: python3 scripts/slot_matrix.py seq cell <its name> --keep DIR")
    return tally["FAIL"]


def cell_of_name(shapes, name):
    """The cell a name names (`p.v|loop.pre:w:lit|bind:jn.mk.p|=x`), or None."""
    head, *ids = name.split("|")
    letter, pj = head[:1], head[1:]
    if letter not in TYPES or not ids or not ids[-1].startswith("="):
        return None
    v = variants(shapes, letter, pj)
    stmts = [next((s for s in v["stmt"] if s[0] == i), None) for i in ids[:-1]]
    end = next((s for s in v["end"] if s[0] == ids[-1][1:]), None)
    if end is None or None in stmts or not stmts or not scoped(stmts, end):
        return None
    return make_cell(letter, pj, stmts, end)


def letters_of(args):
    return list(TYPES) if args.type in (None, "all") else [args.type]


def backends_of(args):
    return {"both": ("interp", "c")}.get(args.backend, (args.backend,))


def loaded():
    shapes, problems = load_shapes()
    if problems:
        raise SystemExit("\n".join(problems))
    return shapes


def cmd_list(args):
    shapes = loaded()
    for cls in CLASSES:
        print(f"{cls:5} " + " ".join(s["id"] + ("" if s["types"] is TYPES else f"[{s['types']}]")
                                     for s in shapes if s["cls"] == cls))
    for letter, pj, n, firsts, stmts, ends in space(shapes, letters_of(args), args.length):
        size = len(firsts) * len(stmts) ** (n - 1) * len(ends)
        print(f"type {letter}{pj or '':4} length {n}: {len(firsts)} first × {len(stmts)} statements^{n - 1} "
              f"× {len(ends)} tails = {size}")
    return 0


def cmd_run(args):
    shapes = loaded()
    if args.cells:
        names = [ln.strip() for ln in open(args.cells, encoding="utf-8") if ln.strip() and not ln.startswith("#")]
        cells = [cell_of_name(shapes, n) for n in names]
        if None in cells:
            raise SystemExit(f"slot_matrix seq: `{names[cells.index(None)]}` names no cell of the table")
        total = len(cells)
    else:
        cells, total = product(shapes, letters_of(args), args.length, args.sample, args.seed, not args.no_projections)
    if args.shape:
        cells = [c for c in cells if any(s in c["name"] for s in args.shape)]
    print(f"slot_matrix seq: {len(cells)} cells of {total} (length up to {args.length}, seed {args.seed})", flush=True)
    backends = backends_of(args)
    verdicts = evaluate(cells, backends)
    shrunk = None if args.no_shrink else shrink(cells, verdicts, backends)
    return 1 if report(cells, verdicts, backends, args.verbose, total, shrunk, args.list) else 0


def resolve(shapes, letter, items):
    """The statements a fixer wrote, as table variants of one type: by shape id, or by their
    loft with every number left out.  None and the first that matches no row, when one does not."""
    unnum = lambda t: re.sub(r"\s+", " ", re.sub(r"-?\d+", "#", t.replace("<t>", letter))).strip().rstrip(";").strip()
    for pj in pjs_of(letter, True):
        v = variants(shapes, letter, pj)
        stmts, missing = [], None
        for n, item in enumerate(items):
            pool = v["end"] if n == len(items) - 1 else v["stmt"]
            want = item.lstrip("=") if n == len(items) - 1 else item
            hit = next((s for s in pool if s[0] == want), None)
            if hit is None:
                key = unnum(item)
                hit = next((s for s in pool if unnum(number(s[1], s[2], TYPES[letter])[0]) == key), None)
            if hit is None:
                missing = item
                break
            stmts.append(hit)
        if missing is None:
            return pj, stmts, None
    return None, None, missing


def cmd_cell(args):
    shapes = loaded()
    cells = []
    if len(args.statements) == 1 and "|" in args.statements[0]:
        named = cell_of_name(shapes, args.statements[0])
        if named is None:
            print(f"slot_matrix seq cell: `{args.statements[0]}` names no cell of the table")
            return 2
        cells, args.type = [named], named["letter"]
    for letter in ([] if cells else letters_of(args)):
        if any(re.search(r"\b[a-z]+_([a-z])\(", item) and re.search(r"\b[a-z]+_([a-z])\(", item).group(1) != letter
               for item in args.statements):
            continue
        pj, stmts, missing = resolve(shapes, letter, args.statements)
        if stmts is None:
            if args.type not in (None, "all"):
                print(f"slot_matrix seq cell: `{missing}` is no shape of scripts/syntax_sequence.tsv for type "
                      f"{letter}; a row for it (class stmt, or end for the tail) is the change: its loft with "
                      f"`<t>` and holes, and its model")
                return 2
            continue
        cell = make_cell(letter, pj, stmts[:-1], stmts[-1])
        if not scoped([(s[0], s[1], s[2]) for s in stmts[:-1]], stmts[-1]):
            print(f"slot_matrix seq cell: type {letter}: a statement reads a local not bound before it")
            continue
        cells.append(cell)
    if not cells:
        print("slot_matrix seq cell: no type has every statement as a shape of scripts/syntax_sequence.tsv "
              "(`seq list` prints them); name one type with --type to see the statement that is missing")
        return 2
    for cell in cells:
        print(f"cell      {cell['name']}")
        for line in cell["lines"]:
            print(f"            {line}")
    if args.keep:
        os.makedirs(args.keep, exist_ok=True)
        text, spans = program(None, cells)
        path = os.path.join(args.keep, "cells.loft")
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)
        print(f"kept      {path} (`loft --tests` runs it; the checks: "
              + " ".join(f"{k}={v}" for k, v in sorted(CHECKS.items())) + " LOFT_LEAK_CHECK=1)")
    backends = backends_of(args)
    verdicts = evaluate(cells, backends)
    for cell in cells:
        worst, lines = verdicts[cell["name"]]
        print(f"{worst:8} {cell['name']}")
        for backend, v, why in lines:
            print(f"   {backend}: {why[:400]}")
        if worst != "ok" or args.verbose:
            print("   the model: " + " ".join(f"p={pv},k={k}: {r}" for (pv, k), r in
                                              zip([(pv, k) for pv in PARAMS for k in KS], cell["expect"].split(";"))))
    return 1 if any(verdicts[c["name"]][0] == "FAIL" for c in cells) else 0


def cmd_walk(args):
    shapes = loaded()
    cells, total = product(shapes, letters_of(args), args.length, args.sample, args.seed)
    backends = backends_of(args)
    new = evaluate(cells, backends)
    here = sm.loft_binary()
    os.environ["LOFT"] = args.old
    try:
        old = evaluate(cells, backends)
    finally:
        os.environ["LOFT"] = here
    groups = {"fixed since": [], "new since": [], "failing on both": []}
    for cell in cells:
        was, now = old[cell["name"]][0], new[cell["name"]][0]
        if was == "FAIL" and now == "ok":
            groups["fixed since"].append(cell)
        elif was == "ok" and now == "FAIL":
            groups["new since"].append(cell)
        elif was == "FAIL" and now == "FAIL":
            groups["failing on both"].append(cell)
    for title, which in (("fixed since", old), ("new since", new), ("failing on both", new)):
        mine = groups[title]
        print(f"{title}: {len(mine)} cell(s)")
        for f, n, of, chans, backs, small in fold(mine, which, cells):
            print(f"  {f}: {n} of {of} ({'+'.join(chans)}; {' '.join(backs)}) — {small['name']}")
            print(f"      {' '.join(small['lines'])}")
    print(f"slot_matrix seq walk: {len(cells)} cells of {total}, old [{args.old}], new [{here}]")
    return 0


BASELINE_HEAD = """# The cells of the slot matrix's sequence axis that do not pass today — the gate's baseline
# (`python3 scripts/slot_matrix.py seq gate`, doc/claude/SLOT_MATRIX.md § The sequence axis).
# The gate runs a fixed seeded sample on both backends and fails on a cell that is not ok and
# not listed here, and on a line here whose cell passes; a fix removes its lines in the same
# change (`seq gate --update`, then read the diff).  Columns, tab-separated: cell, backend,
# channel.  A line starting with `~` ends in a crash and may pass.
"""


def cmd_gate(args):
    shapes = loaded()
    cells, total = product(shapes, list(TYPES), GATE["length"], GATE["sample"], GATE["seed"])
    full = args.full or args.update
    on_c = None if full else {c["name"] for n, c in enumerate(cells) if n % GATE["c_every"] == 0}
    verdicts = evaluate(cells, ("interp", "c"), on_c)
    keys = set()
    for cell in cells:
        worst, lines = verdicts[cell["name"]]
        if worst == "REFUSED":
            keys.add((cell["name"], "-", "refused"))
        for backend, v, why in lines:
            if v == "FAIL":
                keys.add((cell["name"], backend, channel(why)))
    if args.update:
        with open(BASELINE, "w", encoding="utf-8") as f:
            f.write(BASELINE_HEAD)
            for key in sorted(keys):
                f.write(("~" if key[2] == "crash" else "") + "\t".join(key) + "\n")
        print(f"slot_matrix seq gate: baseline rewritten, {len(keys)} line(s)")
        return 0
    stable, unstable = sm.load_baseline(BASELINE)
    # A baseline line of a cell this run did not compile says nothing here.
    stable = {k for k in stable if k[1] != "c" or on_c is None or k[0] in on_c}
    excused = {(n, b) for n, b, _ in unstable}
    new = sorted(k for k in keys - stable if (k[0], k[1]) not in excused)
    gone = sorted(stable - keys)
    if not new and not gone:
        print(f"slot_matrix seq gate: {len(cells)} cells, {len(keys)} not ok, all in the baseline  [{sm.loft_binary()}]")
        return 0
    if new:
        only = {n for n, _, _ in new}
        print(f"BROKEN: {len(new)} cell line(s) of the sequence axis fail and are not in the baseline — a "
              f"change made a sequence of statements answer other than the model, leak, or free twice.  They fold to:")
        for f, n, of, chans, backs, small in fold([c for c in cells if c["name"] in only], verdicts, cells):
            print(f"  {f}: {n} of {of} ({'+'.join(chans)}; {' '.join(backs)}) — {small['name']}")
            print(f"      {' '.join(small['lines'])}")
        for name, backend, ch in new:
            why = next((w for b, v, w in verdicts[name][1] if b == backend), "")
            print(f"  NEW      {name} [{backend}] ({ch}): {why[:200]}")
        print("  one of them: python3 scripts/slot_matrix.py seq cell '<shape id>' … (the ids are the name's parts)")
    if gone:
        print(f"FIXED: {len(gone)} baseline line(s) pass now.  Nothing is broken: run "
              f"`python3 scripts/slot_matrix.py seq gate --update`, read the diff (it must only remove lines), "
              f"and commit it with the fix.")
        for name, backend, ch in gone:
            print(f"  PASSES   {name} [{backend}] ({ch})")
    print(f"slot_matrix seq gate: {len(cells)} cells, {len(new)} new, {len(gone)} to remove  [{sm.loft_binary()}]")
    return 1


def table_problems():
    """The shape table is well formed, and every cell of one statement it makes has a model
    that runs (`slot_matrix.py check`, the doc_hygiene gate, reads this too)."""
    shapes, problems = load_shapes()
    if problems:
        return problems
    for letter in TYPES:
        for pj in pjs_of(letter, True):
            v = variants(shapes, letter, pj)
            for st in v["stmt"]:
                for e in v["end"]:
                    if parse(st[2])[0] in ("set", "loop") and scoped([st], e):
                        try:
                            make_cell(letter, pj, [st], e)
                        except (AssertionError, KeyError, TypeError, ValueError, IndexError) as err:
                            problems.append(f"syntax_sequence.tsv: type {letter}{pj}: `{st[1]} {e[1]}`: "
                                            f"the model fails: {err}")
    return problems


def cmd_check(_args):
    problems = table_problems()
    print("\n".join(problems[:40]) if problems else "slot_matrix seq check: ok")
    return 1 if problems else 0


def add_parser(sub):
    p = sub.add_parser("seq", help="the sequence axis: a few statements over one local (slot_sequence.py)")
    seq = p.add_subparsers(dest="seq_cmd", required=True)

    def common(q, sample=2000):
        q.add_argument("--type", choices=list(TYPES) + ["all"], help="one type of the local (default all)")
        q.add_argument("--length", type=int, default=3, help="statements before the tail, at most (default 3)")
        q.add_argument("--sample", type=int, default=sample, help="cells at most; a larger product is sampled")
        q.add_argument("--seed", type=int, default=1)
        q.add_argument("--backend", choices=("interp", "c", "both"), default="both")
    q = seq.add_parser("list")
    q.add_argument("--type", choices=list(TYPES) + ["all"])
    q.add_argument("--length", type=int, default=3)
    q.set_defaults(fn=cmd_list)
    q = seq.add_parser("check")
    q.set_defaults(fn=cmd_check)
    q = seq.add_parser("run")
    common(q)
    q.add_argument("--shape", action="append", default=[], help="only cells whose name holds this; may repeat")
    q.add_argument("--no-projections", action="store_true")
    q.add_argument("--cells", metavar="FILE", help="run the cells named in FILE, one a line, instead of the product")
    q.add_argument("--no-shrink", action="store_true", help="do not shrink the failing cells")
    q.add_argument("--list", metavar="FILE", help="write the names of the smallest failing cells to FILE")
    q.add_argument("-v", "--verbose", action="store_true", help="every failing cell, not only the smallest")
    q.set_defaults(fn=cmd_run)
    q = seq.add_parser("cell")
    q.add_argument("statements", nargs="+", help="the statements, the last the tail: loft (`<t>` the type's "
                   "letter) or shape ids as `seq list` and a cell's name show them")
    q.add_argument("--type", choices=list(TYPES) + ["all"])
    q.add_argument("--backend", choices=("interp", "c", "both"), default="both")
    q.add_argument("--keep", metavar="DIR", help="keep the program in DIR")
    q.add_argument("-v", "--verbose", action="store_true")
    q.set_defaults(fn=cmd_cell)
    q = seq.add_parser("walk")
    common(q)
    q.add_argument("--old", required=True, help="the older loft binary")
    q.set_defaults(fn=cmd_walk)
    q = seq.add_parser("gate")
    q.add_argument("--update", action="store_true", help="rewrite the baseline from a full run")
    q.add_argument("--full", action="store_true", help="every cell on the C backend too, not the sample")
    q.set_defaults(fn=cmd_gate)
