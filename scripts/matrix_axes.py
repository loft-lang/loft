#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Matrix axes — which composition axis does a guard file hold FIXED?

A boundary matrix is only as good as the axes it MOVES, and the failure mode is
always the same: the author varies the axis the bug report named and pins four
others.  QUALITY.md § B6m counted five such matrices in a single day and named the
gap exactly — *"which axis did I hold fixed" has no instrument; it is entirely a
matter of the author remembering*.

Remembering is what fails.  `formal/ownership.md` D-own-6 wrote its pinned axis into
its own closing paragraph and the next four defects still came from moving it:

    An axis named in a closure is not an axis measured by it.

So this tool does not ask the author to declare anything.  It carries a fixed
vocabulary of composition axes, each with the DOMAIN of values the language offers,
and reports which of those values a guard file actually reaches.  That is what lets
it name an axis the author never considered — the domain comes from the language,
not from the author's list.

The vocabulary is derived from axes that have actually bitten, one citation each;
it is not a taxonomy invented up front.

    A1 container kind       loft#1104 -> B6i: `pick(h[k], ...)` leaked at every KEYED
                            kind, and the matrix carried only vector and tuple
    A2 container provenance loft#1105: every cell built its container INSIDE the
                            calling function, which is what hid the over-free
    A3 argument spelling    D-own-6: the closure enumerated the spellings it had
                            thought of; four more were found by moving this
    A4 statement context    loft#1118: the lift fired only in a `for` body, so
                            eleven other statement contexts leaked
    A5 nullability          loft#1106 / B6p: `Optional(t)` is a second spelling of
                            `t`, and a matrix in one spelling answers for one
    A6 default shape        this branch's nullable matrix pinned the `??` right-hand
                            side to a literal
    A7 element type         formal/tuples.md read OPEN: 0 while loft#1004/#1005 were
                            live, because its oracle is all-(integer, integer)
    A9 evaluation count     loft#1118 again: one record per EVALUATION is invisible
                            in a matrix whose cells each run once
    A10 closure capture     loft#1178's guard: every one of its eight cells put a
                            lambda's own PARAMETER inside the comprehension and none
                            put a CAPTURE there, so a regression that made a captured
                            scalar read 0 sat in the file the fix shipped with

WHAT THE VOCABULARY DOES NOT CARRY, measured twice on 2026-09-07 by two walks that did
not share a subject: the axis a defect turns on may not be a COMPOSITION axis at all,
and then an all-green report says nothing about it.

    ROUTE       @PLN153-adjacent, loft#1437: which decoder reads a narrow field --
                a field op, a schema read, a keyed lookup, a binary file.  Three of
                the five decoders were wrong and the tool named none of them; the
                author found them by sweeping the routes by hand.
    ESCAPE      @PLN153 batch 10, loft#1439: does the closure LEAVE the frame that
                built it.  The report listed container kind, element type and
                nullability for that guard and was silent on the one axis its two
                halves differ by -- and the difference is a use-after-free.

Neither is a value of a domain the language offers; each is a QUESTION about where a
value goes.  So read an all-green axis report as "the compositions I can see are
covered", never as "the guard sweeps".

Neither is DERIVABLE from the guard either, and that is why they are documented here
rather than implemented.  `{v[0]}` and `v[0].a` differ by one character and are two
routes; `h[k]` and `for e in h` are two more.  Telling them apart needs the knowledge
that an interpolation of a record goes through `Parts` while a field access goes through
the op — a fact about the IMPLEMENTATION, not about the language surface this vocabulary
is built from.  ESCAPE is the same shape one question over: whether a closure leaves the
frame is a fact about the callee's storage, not about the call's spelling.

THE RANKING CLAIM WAS FALSIFIED BY ITS OWN ORACLE, and that is worth knowing before
reading any output.  The first design ranked files by how many values of an axis they
reach -- the theory being that a file reaching several and stopping short is an author
who was demonstrably enumerating and ran out of ideas, while a file reaching one never
claimed to sweep.  Measured against the cases whose answer is known, that ranking puts
the evidence in the wrong order: loft#1105's killer axis (container provenance -- every
cell built its container inside the calling function) sits at ONE of four, which the
depth ranking does not even list.  Reaching one value is not a point test; it is exactly
what a pinned axis looks like.

So there is no corpus-wide queue here, because nothing measured supports one.  EVERY file
in the corpus leaves some axis short -- 892 of 892 -- which is a thermometer nobody will
read (QUALITY.md § B4).
Two products replace it, and both are measurements rather than rankings:

  * `file <path>` -- the census for ONE guard, to run while writing it.  This is the
    instrument the gap was about, and the domain being external is what lets it name an
    axis the author never considered.
  * `cross <A> <B>` -- which PAIRS of values no file in the corpus reaches.  Sharper than
    either axis alone, because every failure B6m counted was a matrix that moved one axis
    and pinned another: a pair, not a value.  Its thin row is `spatial`, which 20 of the
    892 guards reach at all.

A REPORT, never a gate.  It reads syntax, so it under-reports on anything it cannot
spell (see `caveats`).

    python3 scripts/matrix_axes.py                # the ranked queue
    python3 scripts/matrix_axes.py -c             # the thermometer
    python3 scripts/matrix_axes.py file <path>    # full census for one file
    python3 scripts/matrix_axes.py axis A1        # who reaches what, corpus-wide
    python3 scripts/matrix_axes.py caveats        # what it cannot see
"""
import glob
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# ---------------------------------------------------------------- the vocabulary
# Each axis: (id, title, ordered domain).  The ORDER is the reading order of the
# report, not a ranking -- a missing value is a missing value wherever it sits.
AXES = [
    ("A1", "container kind",
     ["vector", "hash", "sorted", "index", "spatial", "tuple"]),
    ("A2", "container provenance",
     ["local-literal", "parameter", "global", "callee-return"]),
    ("A3", "argument spelling",
     ["literal", "local", "field", "element", "tuple-element",
      "coalesce-result", "call-result", "chain"]),
    ("A4", "statement context",
     ["top-level", "if-arm", "block", "loop-body", "call-argument",
      "interpolation", "if-condition", "coalesce-subject", "coalesce-rhs",
      "discarded"]),
    ("A5", "nullability", ["non-null", "nullable"]),
    ("A6", "default shape", ["literal", "variable", "call"]),
    ("A7", "element type",
     ["integer", "narrow-int", "float", "text", "boolean", "struct",
      "enum", "nested-container"]),
    ("A9", "evaluation count", ["single", "loop"]),
    ("A10", "closure capture",
     ["none", "scalar", "collection", "struct", "through-comprehension"]),
]
AXIS_TITLE = {a: t for a, t, _ in AXES}
AXIS_DOMAIN = {a: d for a, _, d in AXES}

KEYED = ("hash", "sorted", "index", "spatial")
NARROW = ("u8", "u16", "i8", "i16", "i32", "u32", "byte", "short")

# ---------------------------------------------------------------- lexical pass


def strip(src):
    """Blank comments and string bodies, keeping offsets and interpolation code.

    Every later detector runs on this, so a `//` inside a text literal and a
    `hash<` inside a comment both stop being findable -- which is the point: the
    census must count what the program DOES, not what its header talks about.

    An interpolated `{...}` keeps its CONTENTS, not just its braces.  A call
    written there is a real call in a real statement context, and blanking it
    made an eight-context sweep read as six.
    """
    out = list(src)
    i, n = 0, len(src)
    while i < n:
        c = src[i]
        if c == '"':
            j = i + 1
            while j < n and src[j] != '"':
                if src[j] == "\\":
                    out[j] = " "
                    j += 1
                    if j < n:
                        out[j] = " "
                    j += 1
                    continue
                if src[j] == "{":          # interpolation: keep the code in it
                    depth = 0
                    while j < n:
                        if src[j] == "{":
                            depth += 1
                        elif src[j] == "}":
                            depth -= 1
                            if depth == 0:
                                j += 1
                                break
                        j += 1
                    continue
                out[j] = " "
                j += 1
            i = j + 1
            continue
        if c == "/" and i + 1 < n and src[i + 1] == "/":
            while i < n and src[i] != "\n":
                out[i] = " "
                i += 1
            continue
        i += 1
    return "".join(out)


def functions(s):
    """(name, params, body_start, body_end) for every `fn` in stripped source."""
    res = []
    for m in re.finditer(r"\bfn\s+(\w+)\s*\(([^)]*)\)", s):
        b = s.find("{", m.end())
        if b < 0:
            continue
        depth, i = 0, b
        while i < len(s):
            if s[i] == "{":
                depth += 1
            elif s[i] == "}":
                depth -= 1
                if depth == 0:
                    break
            i += 1
        res.append((m.group(1), m.group(2), b + 1, i))
    return res


def split_args(a):
    """Top-level comma split, honouring (), [], {} and <>-free type text."""
    out, depth, cur = [], 0, ""
    for ch in a:
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
        if ch == "," and depth == 0:
            out.append(cur.strip())
            cur = ""
        else:
            cur += ch
    if cur.strip():
        out.append(cur.strip())
    return out


# ---------------------------------------------------------------- detectors

def a1_container_kind(s):
    seen = set()
    if re.search(r"\bvector\s*<", s):
        seen.add("vector")
    for k in KEYED:
        if re.search(r"\b" + k + r"\s*<", s):
            seen.add(k)
    # A tuple shows as a projection (`t.0`) or a parenthesised multi-value bind.
    if re.search(r"\w\.\d+\b", s) or re.search(r"=\s*\([^()]*,[^()]*\)", s):
        seen.add("tuple")
    return seen


def a2_container_provenance(s):
    """Where does a container-typed value in a function BODY come from?

    The over-free loft#1105 hid behind this axis: a container built in the calling
    function dies with the frame, so a cell that never receives one from outside
    cannot witness a free that outlives it.
    """
    seen = set()
    ctype = r"(?:vector|hash|sorted|index|spatial)\s*<"
    for _, params, b, e in functions(s):
        if re.search(ctype, params):
            seen.add("parameter")
        body = s[b:e]
        for m in re.finditer(r":\s*" + ctype + r"[^=;]*=\s*([^;]+)", body):
            rhs = m.group(1).strip()
            if rhs.startswith("["):
                seen.add("local-literal")
            elif re.match(r"\w+\s*\(", rhs):
                seen.add("callee-return")
        # an untyped bind from a literal is still a locally built container
        if re.search(r"=\s*\[", body):
            seen.add("local-literal")
        if re.search(r"->\s*" + ctype, s):
            # some function answers one; a caller that binds it has a callee-return
            for m in re.finditer(r"=\s*(\w+)\s*\(", body):
                if re.search(r"\bfn\s+" + re.escape(m.group(1)) + r"\s*\([^)]*\)\s*->\s*"
                             + ctype, s):
                    seen.add("callee-return")
    # a container declared outside every function body is a global
    outside, last = "", 0
    for _, _, b, e in functions(s):
        outside += s[last:b]
        last = e
    outside += s[last:]
    if re.search(r"^\s*\w+\s*:\s*" + ctype, outside, re.M):
        seen.add("global")
    return seen


def _classify_arg(a):
    """Every spelling an argument CONTAINS, not the first one that matches.

    `v[3, 6].t.0` is an element access AND a tuple projection AND a chain, and a
    one-label answer picks whichever test ran first — which made the corpus look as if
    no guard ever reached a tuple element through a container.  A coverage question
    asks what is PRESENT, so the answer is a set.
    """
    a = a.strip()
    if not a:
        return set()
    out = set()
    if "??" in a:
        out.add("coalesce-result")
    if re.match(r'^(-?\d|true\b|false\b|\[|\{|null\b)', a) or a.startswith('"'):
        out.add("literal")
    if re.match(r"^\w+\s*\(", a):
        out.add("call-result")
    if re.search(r"\w\s*\[", a):
        out.add("element")
    if re.search(r"\.\d+\b", a):
        out.add("tuple-element")
    if re.match(r"^\w+\s*$", a):
        out.add("local")
    steps = re.findall(r"\.\w+", a)
    if len(steps) == 1 and "tuple-element" not in out:
        out.add("field")
    if len(steps) >= 2 or re.search(r"[\)\]]\s*\.\w+", a):
        out.add("chain")
    return out


def a3_argument_spelling(s):
    """How is an argument SPELLED at a call site?

    D-own-6's register entry is the reason this axis exists: a predicate that
    enumerates spellings keeps being one spelling short, so the domain here comes
    from the language's places-a-value-can-live, not from any one fix's list.
    """
    seen = set()
    for m in re.finditer(r"\b(\w+)\s*\(", s):
        if m.group(1) in ("fn", "if", "for", "while", "match", "return", "assert"):
            continue
        # the call's own argument list
        i, depth = m.end() - 1, 0
        while i < len(s):
            if s[i] == "(":
                depth += 1
            elif s[i] == ")":
                depth -= 1
                if depth == 0:
                    break
            i += 1
        for a in split_args(s[m.end():i]):
            seen |= _classify_arg(a)
    return seen


def a4_statement_context(s):
    """Where does a CALL sit?  loft#1118's lift fired only in a `for` body.

    Two things have to be right or the sweep reads as narrower than it is.  A
    block's kind comes from the text since the last statement break, so a bare
    `{` is a block and not its enclosing function; and a `(` only means
    "call argument" when an identifier opens it -- `t += (f(x))` is a grouping
    paren, and reading it as an argument list hides seven of loft#1118's cells.
    """
    seen = set()
    stack = []
    i, n, brk, in_str = 0, len(s), 0, False
    while i < n:
        c = s[i]
        if c == '"':
            in_str = not in_str
        elif c == "{":
            head = "" if in_str else s[brk:i]
            if in_str:
                stack.append("interpolation")
                brk = i + 1
                i += 1
                continue
            if re.search(r"\b(for|while)\b", head):
                kind = "loop-body"
            elif re.search(r"\b(if|else|match)\b|=>", head):
                kind = "if-arm"
            elif re.search(r"\bfn\b", head):
                kind = "top-level"
            else:
                kind = "block"
            stack.append(kind)
            brk = i + 1
        elif c == "}":
            if stack:
                stack.pop()
            brk = i + 1
        elif c == ";":
            brk = i + 1
        elif c == "(" and i and re.match(r"\w", s[i - 1]):
            name = re.search(r"(\w+)\s*$", s[:i])
            if name and name.group(1) not in _NOT_A_CALL:
                inner = stack[-1] if stack else "top-level"
                seen.add(_place(s[brk:name.start()], inner, s, i))
        i += 1
    return seen


_NOT_A_CALL = ("fn", "if", "for", "while", "match", "return", "assert", "print")


def _last_unclosed(t, o, c):
    depth = 0
    for j in range(len(t) - 1, -1, -1):
        if t[j] == c:
            depth += 1
        elif t[j] == o:
            if depth == 0:
                return j
            depth -= 1
    return None


def _place(lead, inner, s, i):
    """Place a call within its own statement; the innermost context wins."""
    # peel grouping parens; only an identifier-opened `(` is an argument list
    while True:
        j = _last_unclosed(lead, "(", ")")
        if j is None:
            break
        if j and re.match(r"\w", lead[j - 1]):
            return "call-argument"
        lead = lead[:j]
    if re.search(r"\?\?\s*$", lead):
        return "coalesce-rhs"
    if re.match(r"\s*(\?\.\w+)?\s*\)?\s*\?\?", s[_skip_call(s, i):]):
        return "coalesce-subject"
    if re.search(r"\bif\b", lead):
        return "if-condition"
    if lead.strip() == "":
        j = _skip_call(s, i)
        if re.match(r"\s*;", s[j:]):
            return "discarded"
    return inner


def _skip_call(s, i):
    """Offset just past the `)` closing the call whose `(` is at i."""
    depth = 0
    while i < len(s):
        if s[i] == "(":
            depth += 1
        elif s[i] == ")":
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    return i


def a5_nullability(s):
    seen = set()
    if re.search(r":\s*\w+\?", s) or "??" in s or "?." in s or re.search(r"<\w+\?>", s):
        seen.add("nullable")
    if re.search(r":\s*\w+\s*[,)=;]", s):
        seen.add("non-null")
    return seen


def a6_default_shape(s):
    seen = set()
    for m in re.finditer(r"\?\?\s*([^;,)\]}\n]+)", s):
        rhs = m.group(1).strip()
        if re.match(r"^\w+\s*\(", rhs):
            seen.add("call")
        elif re.match(r'^(-?\d|true\b|false\b|\[|\{)', rhs) or rhs.startswith('"'):
            seen.add("literal")
        elif re.match(r"^\w+", rhs):
            seen.add("variable")
    return seen


def a7_element_type(s):
    """What TYPE is carried?  formal/tuples.md read OPEN: 0 on an all-integer oracle."""
    seen = set()
    inner = re.findall(r"(?:vector|hash|sorted|index|spatial)\s*<([^<>]*)>", s)
    fields = re.findall(r"\w+\s*:\s*([\w?<>\[\]]+)", s)
    # A TUPLE's element types sit inside parens, which the field pattern above cannot
    # reach -- and a tuple is this axis's own citation, so missing them would make the
    # instrument blind exactly where the defect it names lived.
    for grp in re.findall(r":\s*\(([^()]*(?:\([^()]*\)[^()]*)*)\)", s):
        fields += [t.strip() for t in split_args(grp)]
    nested = re.findall(r"(?:vector|hash|sorted|index|spatial)\s*<\s*"
                        r"(?:vector|hash|sorted|index|spatial)\s*<", s)
    structs = set(re.findall(r"\bstruct\s+(\w+)", s))
    enums = set(re.findall(r"\benum\s+(\w+)", s))
    for t in inner + fields:
        base = t.split("[")[0].rstrip("?").strip()
        if base == "integer":
            seen.add("integer")
        elif base in NARROW:
            seen.add("narrow-int")
        elif base == "float":
            seen.add("float")
        elif base == "text":
            seen.add("text")
        elif base == "boolean":
            seen.add("boolean")
        elif base in structs:
            seen.add("struct")
        elif base in enums:
            seen.add("enum")
    if nested:
        seen.add("nested-container")
    return seen


def a9_evaluation_count(s):
    seen = set()
    for _, _, b, e in functions(s):
        body = s[b:e]
        for m in re.finditer(r"\b(for|while)\b[^{]*\{", body):
            j, depth = m.end() - 1, 0
            while j < len(body):
                if body[j] == "{":
                    depth += 1
                elif body[j] == "}":
                    depth -= 1
                    if depth == 0:
                        break
                j += 1
            if re.search(r"\w\s*\(", body[m.end():j]):
                seen.add("loop")
        if re.search(r"\w\s*\(", body):
            seen.add("single")
    return seen


def _lambda_bodies(s):
    """(body_start, body_end) for every lambda literal — both spellings.

    A lambda is where the capture question lives, and it is the one construct the
    `functions()` scan above cannot see: it has no name, and `fn(` with no identifier
    after it is exactly what distinguishes it from a declaration.
    """
    res = []
    for m in re.finditer(r"\bfn\s*\(|\|[\w\s,]*\|\s*\{", s):
        b = s.find("{", m.end() - 1)
        if b < 0:
            continue
        depth, i = 0, b
        while i < len(s):
            if s[i] == "{":
                depth += 1
            elif s[i] == "}":
                depth -= 1
                if depth == 0:
                    res.append((b + 1, i))
                    break
            i += 1
    return res


COMPREHENSION = re.compile(r"\.(map|filter|reduce|any|all|sort_by)\s*\(|\[\s*for\b")


def a10_closure_capture(s):
    """Which OUTER binding a lambda body reads, and whether it reads it through a
    comprehension.

    Approximate by construction, and in the safe direction: it reads bindings written
    `name = <rhs>` at the start of a line, classifies them by the SHAPE of that rhs, and
    asks which of those names a lambda body mentions.  A capture it cannot spell is a
    value this axis reports as unreached, which is what a census is for.
    """
    seen = set()
    kind = {}
    for m in re.finditer(r"^\s*(\w+)\s*(?::[^=\n]+)?=\s*(.*)$", s, re.M):
        name, rhs = m.group(1), m.group(2).lstrip()
        if name.startswith("__") or rhs.startswith("fn") or rhs.startswith("|"):
            continue
        # A SET per name, not one verdict: two cells in one file legitimately spell the
        # same name at different types, and keeping only the last one silently drops a
        # value the file does reach.
        if rhs.startswith("["):
            kind.setdefault(name, set()).add("collection")
        elif re.match(r"[A-Z]\w*\s*\{", rhs):
            kind.setdefault(name, set()).add("struct")
        else:
            kind.setdefault(name, set()).add("scalar")
    for b, e in _lambda_bodies(s):
        body = s[b:e]
        # A lambda's own parameters and locals are not captures.
        local = set(re.findall(r"^\s*(\w+)\s*(?::[^=\n]+)?=", body, re.M))
        hit = False
        for name, kinds in kind.items():
            if name in local:
                continue
            if not re.search(r"\b" + re.escape(name) + r"\b", body):
                continue
            hit = True
            seen |= kinds
            for c in COMPREHENSION.finditer(body):
                tail = body[c.end():]
                if re.search(r"\b" + re.escape(name) + r"\b", tail):
                    seen.add("through-comprehension")
                    break
        if not hit:
            seen.add("none")
    return seen


DETECT = {
    "A1": a1_container_kind, "A2": a2_container_provenance,
    "A3": a3_argument_spelling, "A4": a4_statement_context,
    "A5": a5_nullability, "A6": a6_default_shape,
    "A7": a7_element_type, "A9": a9_evaluation_count,
    "A10": a10_closure_capture,
}


def census(path):
    s = strip(open(path, encoding="utf-8", errors="replace").read())
    return {a: DETECT[a](s) & set(AXIS_DOMAIN[a]) for a, _, _ in AXES}


# ---------------------------------------------------------------- report

def corpus():
    return sorted(glob.glob(os.path.join(ROOT, "tests/scripts/*.loft")))


def rows():
    """Every (file, axis) where a file reaches some values of an axis and not all.

    Kept for the thermometer only.  It is NOT a ranked queue: see the docstring -- the
    depth ordering this used to impose was falsified against loft#1105, whose pinned axis
    sits at one of four.
    """
    out = []
    for f in corpus():
        c = census(f)
        for a, _, dom in AXES:
            hit = c[a]
            if hit and len(hit) < len(dom):
                out.append((len(hit), len(dom), f, a, hit,
                            [v for v in dom if v not in hit]))
    return out


def cross(x, y):
    """Which (value, value) pairs of two axes does NO corpus file reach?

    File-level co-occurrence, so a hit only means one file contains both somewhere -- an
    UPPER bound on real cross-coverage.  A zero is therefore solid and a small number is
    not: the true figure can only be lower.
    """
    import itertools
    dx, dy = AXIS_DOMAIN[x], AXIS_DOMAIN[y]
    n = {}
    for f in corpus():
        c = census(f)
        for a, b in itertools.product(c[x], c[y]):
            n[(a, b)] = n.get((a, b), 0) + 1
    print(f"== {x} {AXIS_TITLE[x]}  x  {y} {AXIS_TITLE[y]} ==")
    print("   files reaching BOTH values; an upper bound (co-occurrence, not interaction)\n")
    w = max(18, max(len(v) for v in dx) + 2)
    print(f"{'':{w}s}" + "".join(f"{v[:9]:>10s}" for v in dy))
    for a in dx:
        print(f"{a:{w}s}" + "".join(f"{n.get((a, b), 0):10d}" for b in dy))
    zeros = [(a, b) for a in dx for b in dy if not n.get((a, b))]
    if zeros:
        print(f"\n  never crossed ({len(zeros)}): "
              + ", ".join(f"{a}x{b}" for a, b in zeros))


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else ""
    if mode == "caveats":
        print(__doc__.split("A REPORT, never a gate.")[0].strip())
        print("""
It reads SYNTAX, so every count is a floor:

  * a value reached through a `use`d library is invisible -- the census sees the
    call, not the container the library builds;
  * A2 reads a declaration's right-hand side, so a container handed on through
    two binds reads as whatever the FIRST bind was;
  * A4 places a call by the text before it, so a macro-ish spelling this file
    does not know reads as its enclosing block rather than as its own context;
  * a file that reaches a value only in a COMMENTED-OUT cell reads as not
    reaching it, which is correct, and as never having considered it, which is
    not;
  * A10 reads a capture as any outer `name = <rhs>` a lambda body MENTIONS, and
    types it from the shape of that rhs -- so a capture reached through a call, a
    field of a captured struct, or a binding whose rhs it cannot classify reads as
    `scalar` or as nothing at all.  Its `none` is therefore the softest value here:
    it means "no capture this file can spell", not "no capture".

The DEPTH ranking this used to impose is gone, and the way it went is the useful
part: it was scored against loft#1105, whose pinned axis -- every cell building
its container inside the calling function -- sits at ONE of four values.  Ranking
by how much of an axis a file covers therefore buries the evidence, because a
pinned axis and a point test look identical from the outside.  What remains is
measurement without ranking: the per-file census and the pair cross.""")
        return
    if mode == "file":
        f = sys.argv[2]
        c = census(f)
        print(os.path.relpath(f, ROOT))
        for a, t, dom in AXES:
            hit = c[a]
            if not hit:
                continue
            miss = [v for v in dom if v not in hit]
            print(f"  {a} {t:22s} {len(hit)}/{len(dom)}  "
                  f"reaches {', '.join(sorted(hit))}")
            if miss:
                print(f"     {'':25s}   MISSING {', '.join(miss)}")
        return
    if mode == "axis":
        a = sys.argv[2]
        dom = AXIS_DOMAIN[a]
        tot = {v: 0 for v in dom}
        touch = 0
        for f in corpus():
            hit = census(f)[a]
            if hit:
                touch += 1
            for v in hit:
                tot[v] += 1
        print(f"== {a} {AXIS_TITLE[a]} ==   {touch} of {len(corpus())} files reach it")
        for v in dom:
            print(f"  {v:18s} {tot[v]:4d}")
        return
    if mode == "cross":
        cross(sys.argv[2], sys.argv[3])
        return
    r = rows()
    if mode == "-c":
        print(f"axes left short: {len(r)} rows over "
              f"{len({x[2] for x in r})} of {len(corpus())} files "
              f"(a thermometer, not a queue -- see the docstring)")
        for a, t, _ in AXES:
            k = [x for x in r if x[3] == a]
            if k:
                print(f"  {a} {t:22s} {len(k):4d}")
        return
    # default: what the CORPUS reaches, per axis.  A value the whole suite barely
    # reaches is a blind spot with a name, which a per-file list cannot show.
    print(f"== corpus coverage over {len(corpus())} guards ==")
    print("   run `file <path>` for one guard, `cross <A> <B>` for the pairs\n")
    for a, t, dom in AXES:
        tot = {v: 0 for v in dom}
        touch = 0
        for f in corpus():
            hit = census(f)[a]
            touch += 1 if hit else 0
            for v in hit:
                tot[v] += 1
        print(f"{a} {t:22s} {touch:4d} files reach it")
        print("     " + "  ".join(f"{v}={tot[v]}" for v in dom))


if __name__ == "__main__":
    main()
