#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# ir_walker_audit.py — two questions about `Value`, the IR tree, and the cross-check
# that settles the second one.
#
# `rule_predicate_audit.py` asks where one rule is spelled as a TYPE LIST more than once.
# This asks the two questions that live one level up, over the IR tree itself:
#
#   walkers   Who hand-rolls `Value`'s tree shape?  `Value::for_each_child` is the
#             keystone (data.rs) — its match is exhaustive on purpose, "so a new `Value`
#             variant forces a decision here and every walker inherits the edge".  A
#             walker that recurses by its own match does NOT inherit it: when a variant
#             is added, or an existing one is reached by a new route, that walker goes
#             silently blind, and no test can see the difference because the arm it is
#             missing is the one nothing constructs yet.
#
#             ⚠ **`walkers` and `reach` measure DESCENT, so they are structurally blind to a
#             walker that omits a LEAF** — and that blindness has a name now.  `Value::FnRef`
#             is a leaf in `for_each_child` (it carries no child expression, correctly), so a
#             walker missing an `FnRef` arm is invisible to both modes.  loft#1477 was exactly
#             that: `scopes::collect_return_sources` classifies which VALUES a return
#             delivers, and a capturing lambda is not a `Var`, so `return fn() { … }`
#             contributed nothing to the delivered set and the frame freed what it had just
#             handed up.  `reach` listed the function and named nine missing variants; `FnRef`
#             was not among them and could not be.
#
#             The distinction to hold: a TRAVERSER must descend into every child-bearing
#             shape, and these modes check that.  A value CLASSIFIER must recognise every
#             value-bearing LEAF, which is the complement, and nothing checks it.  A naive
#             screen for it (walkers naming `Var` but not `FnRef`/`TupleGet`/`Enum`) returns
#             238 functions and is therefore not an instrument — most of them ask a question
#             those leaves cannot answer.  Sharpening it needs a way to say which walkers ask
#             *which values does this deliver or own*, which is not written down anywhere yet.
#
#   producers Which variants can never come into existence?  A variant whose every
#             construction is a REBUILD (inside its own match arm), a DESERIALIZER, or a
#             test is a closed cycle with no source: nothing creates the first instance,
#             so nothing can deserialize one either.  It still costs every walker an arm,
#             and no test can ever reach those arms to check them — which is the argument
#             for deleting it rather than completing it.
#
#   unspan    Who pattern-matches a specific `Value` variant without peeling `Span`?
#             `Value::unspan`'s own doc states the requirement — "every second-pass site
#             that pattern-matches a specific Value variant must call `code.unspan()`
#             first" — because a `Span` wrapper otherwise falls to the catch-all and the
#             shape is simply not seen.  A site is counted as handling it if it calls
#             `unspan()`/`unspan_mut()` or carries a `Value::Span` arm.
#
#             ⚠ Reaching this path is NOT the same as a defect, and the difference is the
#             point: `scopes::find_assigned_vars` dropped Span-wrapped `Set`s and `Block`s
#             on real corpus programs, yet peeling changed no program's IR.  Treat a hit as
#             "measure this one", not "fix this one".
#
#   reach     Of the catch-all walkers `walkers` lists, which ones does a running loft
#             binary actually reach?  The list is long and every entry looks equally
#             suspect; the filter that ranks it is *does production run this*.  A
#             one-level "is it called from outside a test" cannot answer that — the four
#             walkers that turned out to be test-only were called by ordinary-looking
#             functions that were themselves only called from tests — so this walks the
#             call graph transitively from each `[[bin]]`'s `main`.
#
#   spellings One notion, two IR spellings.  A PROJECTION is `OpGetField(Var(b), …)` *or*
#             `Value::TupleGet(b, i)` — the same language-level notion, one spelled as a
#             CALL and one as a `Value` variant carrying its base as a var NUMBER.  A site
#             that resolves the notion by OP NAME cannot see the tuple half, and no test
#             can tell: the shape simply never arrives.  Reports who resolves a projection
#             op by `def_nr` (or via `is_projection_op`) and whether they also carry a
#             `TupleGet` arm.
#
#   former    The same question one TYPE FORMER over, asked of ONE former at a time
#             (`former RefVar`, `former Tuple`, …); `optional` is the founding former's
#             own mode name, and the one the @FR-N-Shape ratchet gates.  Below is that
#             former's statement of the question; every row of `FORMERS` asks it about
#             its own wrapper.
#
#   optional  `Optional(τ)` is `τ` with a
#             nullability bit and the same storage (@FR-L-Null), so a site that resolves a
#             shape by naming `Type` variants answers for `τ` and not for `τ?` — the value
#             takes the catch-all and nothing says so (loft#1106).  Classifies every body
#             that discriminates on a `Type` variant (sees through · descends via the
#             keystone · opaque) and, for each opaque verb in `data.rs`, lists which callers
#             peel the receiver with `.base()` first.  Disagreement between two callers of
#             ONE verb is the shape that bites.
#
#   dead      `producers` INTERSECTED with a census of what the front end actually emits
#             over the 854-program corpus.  Neither half is an oracle and the failures go
#             opposite ways, which is the whole reason this mode exists: `Loop`/`Single`/
#             `Parallel` are screened as producerless (their origin is a helper the screen
#             reads as a rebuild) yet are plainly alive, and `Iter`/`RawExpr` are absent
#             from the census (lowered away before the snapshot, or built after it) yet
#             have real producers.  Only a variant flagged by BOTH is dead.
#
# A REPORT, never a gate.  Every mode is heuristic — `producers` classifies by construction
# SITE, so read the sites it prints before acting.  Verdicts live in
# doc/claude/formal/IMPLEMENTATIONS.md.
#
# Usage:  python3 scripts/ir_walker_audit.py [walkers|producers|unspan|reach|dead|both]
#         [spellings|optional|former <Name>]
#         `reach`, `spellings`, `optional` and `former` need no binary; `dead` needs a built binary (target/debug/loft, or $LOFT_BIN) and takes ~1 min.

import glob
import contextlib
import io
import json
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "src")
DATA_RS = os.path.join(SRC, "data.rs")

FN = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?fn\s+([a-z_][a-z0-9_]*)")
VARIANT = re.compile(r"\bValue::([A-Za-z_][A-Za-z0-9_]*)")
KEYSTONE = ("for_each_child", ".any_node(")
# Files that only ever build a `Value` to put it back on the wire, or to read one off it.
SERIALIZERS = ("ir_read.rs", "ir_schema.rs", "ir_store.rs", "ir_node.rs", "ir_schema_gen.rs")


def rust_files():
    return sorted(glob.glob(os.path.join(SRC, "**", "*.rs"), recursive=True))


def rel(path):
    return os.path.relpath(path, ROOT)


def value_variants():
    """The `Value` variant names, read from the enum itself."""
    out, inside = [], False
    for line in open(DATA_RS, encoding="utf-8"):
        if line.startswith("pub enum Value"):
            inside = True
            continue
        if inside:
            if line.startswith("}"):
                break
            m = re.match(r"\s+([A-Z][A-Za-z0-9]*)\s*[(,{]", line)
            if m:
                out.append(m.group(1))
    return out


def functions(path):
    """Yield (name, start_line, body_text) for each fn in the file."""
    lines = open(path, encoding="utf-8").read().split("\n")
    cur, start, body = None, 0, []
    for i, line in enumerate(lines, 1):
        m = FN.match(line)
        if m:
            if cur:
                yield cur, start, "\n".join(body)
            cur, start, body = m.group(1), i, []
        elif cur is not None:
            body.append(line)
    if cur:
        yield cur, start, "\n".join(body)


# A bare `_ =>` / `other =>` arm.  This is the difference that matters: an EXHAUSTIVE
# match (no catch-all) already gets the keystone's guarantee for free — adding a variant
# breaks the build and forces a decision.  A partial match with a catch-all is the shape
# that goes silently blind, because the new edge just falls into `_`.
CATCHALL = re.compile(r"^\s*(?:_|other|rest)\s*(?:if\b[^=]*)?=>", re.M)


def audit_walkers():
    hand, keyed, exhaustive = [], 0, 0
    for path in rust_files():
        if os.path.basename(path) == "data.rs":
            continue  # the keystone's own home
        for name, start, body in functions(path):
            variants = set(VARIANT.findall(body))
            if len(variants) < 3:
                continue
            if not re.search(rf"\b{re.escape(name)}\s*\(", body):
                continue  # not recursive
            if any(k in body for k in KEYSTONE):
                keyed += 1
            elif not CATCHALL.search(body):
                exhaustive += 1
            else:
                hand.append((f"{rel(path)}:{start}", name, len(variants)))
    total = keyed + exhaustive + len(hand)
    print(f"recursive multi-variant `Value` walkers: {total}")
    print(f"  descend via the keystone         : {keyed:>4}   inherit every edge")
    print(f"  exhaustive match, no catch-all   : {exhaustive:>4}   a new variant breaks the build")
    print(f"  partial match + catch-all        : {len(hand):>4}   a new edge falls into `_`, silently")
    print()
    print("  The last group, fewest variants first — the least covered has the most to miss:")
    for site, name, n in sorted(hand, key=lambda r: r[2])[:30]:
        print(f"  {site:<42} {name:<36} {n:>2} variants")
    if len(hand) > 30:
        print(f"  … and {len(hand) - 30} more")


# A site DISCRIMINATES when it pattern-matches a specific variant (an arm, an `if let`).
# The lookbehind is load-bearing: without it this matches INSIDE another enum's path,
# because `MValue::Scalar` and `VariableValue::Long` both literally contain "Value::".
# Intersecting against the IR variant set does not save you — `Long` and `Single` are
# real IR variant names, so `VariableValue` scored two and `state::static_call` read as
# an unpeeled hazard while discriminating on a debug enum.
DISCRIM = re.compile(r"(?<![A-Za-z0-9_])Value::([A-Za-z]+)\s*(?:\([^)]*\))?\s*(?:=>|\||\)\s*=)")

# A GUARDED arm — `Value::Call(nr, args) if data.def(*nr).name() == "OpGetField" => …`.  The
# pattern above stops at the `if`, so an arm with a guard was invisible to it.
GUARDED_ARM = re.compile(
    r"(?<![A-Za-z0-9_])Value::([A-Za-z]+)\s*(?:\([^)]*\)|\{[^}]*\})?\s*if\b[^\n]*=>"
)

# An EQUALITY test — `if def.code == Value::Null`, `if v != Value::Null`.  Comparing against a
# variant discriminates on it exactly as a pattern does, and a `Span` wrapper defeats it the
# same way, so it belongs in the same census.
#
# It is here because leaving it out did not merely UNDERCOUNT — it made the count depend on
# formatting.  `DISCRIM` ends in `(?:=>|\||\)\s*=)`, and the `|` alternative matched the
# FIRST BAR of a boolean `||`: so `if def.code == Value::Null || f()` counted, and the same
# test written on its own line did not.  Rewrapping one condition in `scopes.rs` moved the
# published total by one with nothing about the code's shape having changed.  A census whose
# answer moves when a line is rewrapped cannot be read as open work.
EQ_TEST = re.compile(r"(?:==|!=)\s*Value::([A-Za-z]+)")

# A binding form — `if let Value::Call(..) = v`, `let Value::Block(bl) = v else`, `while let`.
# `Value::unspan`'s rule is about *pattern-matching a specific variant*, and these do exactly
# that: `pre_eval::create_stack_var` decides on `if let Value::Call(d, args) = v` and falls to
# `None` for anything else, so a wrapper makes it emit no `&mut var_…` at all.
BINDING_PAT = re.compile(r"(?:^|\W)(?:if\s+let|while\s+let|let)\s+(?:Some\()?Value::([A-Za-z]+)")


def ir_value_variants():
    """The variant names of the IR `Value` enum, read from `src/data.rs`.

    `Value` is not one type in this crate: `host::Value` is a separate enum for the
    host-call ABI (`Void` / `Bool` / `Int` / …) and has NO `Span` variant, so a site
    matching on it can never be hidden by one.  Without this set those sites counted
    as unpeeled `Span` hazards — four of them did — and the total is quoted as open
    work, so an over-count is a bill someone pays in review.
    """
    src = open(os.path.join(ROOT, "src", "data.rs"), encoding="utf-8").read()
    start = src.index("pub enum Value {")
    depth, i = 0, src.index("{", start)
    end = i
    for j in range(i, len(src)):
        if src[j] == "{":
            depth += 1
        elif src[j] == "}":
            depth -= 1
            if depth == 0:
                end = j
                break
    names = set(re.findall(r"^\s{4}([A-Z][A-Za-z0-9]*)\s*[({,]", src[i:end], re.M))
    if len(names) < 20:
        raise SystemExit(f"ir_value_variants parsed only {len(names)} variants — the enum shape changed")
    return names


IR_VARIANTS = ir_value_variants()

# `host::Value` shares `Float` / `Int` / `Text` with the IR enum, so intersecting against
# the IR variant set is not enough on its own — a host site matching two of those three
# still looks like an IR site.  `Void` / `Bool` / `Ref` exist ONLY on `host::Value`, so a
# site naming one of them is deciding a host value and cannot be hidden by a `Span`.
HOST_ONLY = {"Void", "Bool", "Ref"}

# `walk` peels a `Span` before calling `f` exactly as `any_node` does
# (`if let Value::Span(b) = self { return b.1.walk(f) }`), so its closure is safe too.
#
# `map_nodes` is the one that does NOT peel, and its closure is safe anyway — for the other
# reason.  Its doc says so outright: "`f` SEES `Span` nodes (it may want to replace them);
# descent still enters the wrapped value."  So a closure whose `if let` misses the wrapper is
# handed the payload one level down, exactly as with `for_each_child`.  The two are worth
# telling apart when reading a closure: moving one from `walk` to `map_nodes` starts feeding
# it `Span` nodes, and only the descent makes that harmless.
TRAVERSAL_OPEN = re.compile(r"\.(any_node|for_each_child|for_each_child_mut|walk|map_nodes)\(")

# A match whose SCRUTINEE is a span-transparent accessor cannot see a `Span` either.
# `Value::tail` peels (`Value::Span(b) => b.1.tail()`), so `match val.tail() { … }` is safe
# however many specific variants it then names.
PEELED_SCRUTINEE = re.compile(r"match\s+[^{;]*\.(tail|unspan|unspan_mut)\(\)")


def test_regions(path):
    """Line ranges covered by `#[cfg(test)]`, as (start, end) pairs.

    Test functions build IR by hand, so a `Span` reaches one only if the test writes it.
    Counting them as unpeeled production sites overstates the backlog — `vectors.rs` alone
    contributed two.

    ⚠ Brace-balanced on purpose, NOT "everything after the first `#[cfg(test)]`".  That
    cheaper rule looks right because test modules sit at the end of a file by convention,
    and it is wrong wherever they do not: `src/trie_db.rs` has one at line 245 of 1355, so
    the shortcut would discard 82 % of a production file and silently shrink the backlog.
    """
    lines = open(path, encoding="utf-8").read().split("\n")
    regions, i = [], 0
    while i < len(lines):
        if "#[cfg(test)]" in lines[i]:
            j, depth, seen = i, 0, False
            while j < len(lines):
                depth += lines[j].count("{") - lines[j].count("}")
                if "{" in lines[j]:
                    seen = True
                if seen and depth <= 0:
                    break
                j += 1
            regions.append((i + 1, j + 1))
            i = j + 1
        else:
            i += 1
    return regions


def strip_traversal_closures(body):
    """Remove `x.any_node(&mut |n| match n { … })` regions from a function body.

    `Value::any_node` unwraps a `Span` BEFORE calling the predicate —
    `if let Value::Span(b) = self { return b.1.any_node(pred) }` — and `for_each_child`
    descends through one the same way.  So a match that IS such a closure's body can
    never be handed a `Span`, and counting it as an unpeeled site is a false positive.
    Measured: this is what made `hoist.rs::writes_store` (the vector-header hoist's
    safety predicate) and `scopes::guard_escapes` look like hazards when neither can be.
    """
    out, i = [], 0
    while True:
        m = TRAVERSAL_OPEN.search(body, i)
        if not m:
            out.append(body[i:])
            return "".join(out)
        out.append(body[i : m.start()])
        depth, j = 0, m.end() - 1  # sitting on the '('
        while j < len(body):
            if body[j] == "(":
                depth += 1
            elif body[j] == ")":
                depth -= 1
                if depth == 0:
                    break
            j += 1
        i = j + 1


def peels_span(body):
    """Does this body see through a `Span` — by unwrapping it, or by giving it an arm?

    Shared by `unspan` and `reach` so the two cannot answer it differently.  `reach` needs
    it to avoid reporting `Span` as a skipped wrapper at a site that peels it without ever
    naming the variant: `collections::holder_type` opens with `base.unspan()`, and reading
    its arms alone says it forgot the case it in fact handles first.
    """
    return (
        ".unspan()" in body
        or ".unspan_mut()" in body
        or "Value::Span" in body
        or bool(PEELED_SCRUTINEE.search(body))
    )


def audit_unspan():
    """Sites that read a specific `Value` shape without seeing through `Span`."""
    total, handled, rows = 0, 0, []
    # The serializers walk a node they were handed by kind, not by pattern-matching a
    # shape they expect, so `Span` is a variant to them like any other.
    skip = ("data.rs", "ir_schema.rs", "ir_store.rs", "ir_read.rs", "ir_node.rs")
    for path in rust_files():
        if os.path.basename(path).endswith(skip):
            continue
        regions = test_regions(path)
        for name, start, body in functions(path):
            if any(lo <= start <= hi for lo, hi in regions):
                continue  # a test fn: it constructs its own IR, spans included
            code = strip_traversal_closures(body)
            named = set(DISCRIM.findall(code))
            named |= set(GUARDED_ARM.findall(code))
            named |= set(BINDING_PAT.findall(code))
            named |= set(EQ_TEST.findall(code))
            named -= {"Span"}
            if named & HOST_ONLY:
                continue  # a `host::Value` site — that enum has no `Span`
            specific = named & IR_VARIANTS
            if len(specific) < 2:
                continue  # not discriminating between shapes of the IR `Value`
            total += 1
            if peels_span(body):
                handled += 1
            else:
                rows.append((f"{rel(path)}:{start}", name, len(specific)))
    print(f"sites discriminating on 2+ specific `Value` variants : {total}")
    print(f"  peel `Span` (unspan, or a `Span` arm)              : {handled}")
    print(f"  neither — a `Span` hides the shape from them       : {len(rows)}")
    print()
    print("  Most variants first: the more shapes a site tells apart, the more a missed")
    print("  wrapper costs it.  A hit is a MEASUREMENT to make, not a defect found.")
    for site, name, n in sorted(rows, key=lambda r: -r[2]):
        print(f"  {site:<44} {name:<34} {n} variants")


def constructor_sites(variant):
    """Lines that plausibly CONSTRUCT `Value::<variant>`, with a classification.

    Three things are NOT a producer, and the point of this mode is to tell them apart:
      * `serializer` — ir_read/ir_schema/ir_store/ir_node build a node to put it on the
        wire or read one off it.  A deserializer can only ever produce what a producer
        once wrote, so it cannot be the origin.
      * `test`       — inside a `#[cfg(test)]` module.
      * `rebuild`    — the construction sits in a function that also MATCHES this
        variant, so it rewrites an instance that already existed.  This is the one the
        first cut of this script missed, and it is the one that hides a dead variant:
        `Scopes::scan` rebuilds every variant it walks.
    """
    # Unit variants (`Value::Null`) take no parens; the rest do.
    pat = re.compile(rf"\bValue::{variant}\s*\(")
    unit = re.compile(rf"\bValue::{variant}\b(?!\s*\()")
    arm = re.compile(rf"\bValue::{variant}\b[^=]*=>")
    out = []
    for path in rust_files():
        text = open(path, encoding="utf-8").read()
        lines = text.split("\n")
        test_line = next((i for i, l in enumerate(lines, 1) if "#[cfg(test)]" in l), None)
        # Enclosing-function body for each line, so a rebuild can be recognised.
        fn_of = {}
        for name, start, body in functions(path):
            end = start + body.count("\n") + 1
            for i in range(start, end + 1):
                fn_of[i] = (name, body)
        for i, line in enumerate(lines, 1):
            if not (pat.search(line) or unit.search(line)):
                continue
            stripped = line.strip()
            # Comments and string literals mention a variant without building one.
            if stripped.startswith(("//", "*", "#")) or f'"Value::{variant}' in line:
                continue
            if "matches!" in line or "=>" in line:
                continue  # pattern position
            # `Value::X(_` / `Value::X(_,` binds nothing: a pattern whose `=>` is on a
            # later line (`Value::Return(_) | Value::Break(_) | …` wraps in control.rs).
            if re.search(rf"\bValue::{variant}\s*\(\s*_", line):
                continue
            if stripped.startswith("|") or re.search(r"\b(?:if|while)\s+let\b", line):
                continue
            if re.search(rf"let\s+Value::{variant}\b", line):
                continue
            if os.path.basename(path) in SERIALIZERS:
                kind = "serializer"
            elif test_line is not None and i > test_line:
                kind = "test"
            elif i in fn_of and arm.search(fn_of[i][1]):
                kind = "rebuild"
            else:
                kind = "code"
            out.append((f"{rel(path)}:{i}", kind, stripped[:88]))
    return out


def audit_producers():
    dead, live = [], []
    for v in value_variants():
        sites = constructor_sites(v)
        if any(s[1] == "code" for s in sites):
            live.append(v)
        else:
            dead.append((v, sites))
    print(f"`Value` variants: {len(live) + len(dead)}   with a producer: {len(live)}   without: {len(dead)}")
    print()
    for v, sites in dead:
        seen = sorted({s[1] for s in sites}) or ["none at all"]
        print(f"  {v} — every construction is: {', '.join(seen)}")
        for site, kind, txt in sites:
            if kind != "test":
                print(f"      {kind:<11} {site:<32} {txt}")
        print()


def census(limit=None):
    """How many nodes of each variant does the front end actually PRODUCE?

    `LOFT_DUMP_SNAPSHOT=<path>` writes the parsed `Data` as JSON and exits, so this is
    front-end only — the corpus is never run.  Two caveats that make this a screen and
    not an oracle, both measured:
      * the snapshot is written POST-`scopes::check`, so a variant the parser builds and
        the lowering removes reads as 0 (`Iter`);
      * a variant generated LATER still reads as 0 (`RawExpr`, built in generation/emit).
    So corpus-absent alone proves nothing.  Absent AND with no producer (see `producers`)
    is what makes a variant dead.
    """
    import subprocess, tempfile, collections
    binary = os.environ.get("LOFT_BIN", os.path.join(ROOT, "target", "debug", "loft"))
    if not os.path.exists(binary):
        print(f"  no binary at {binary} — build it, or set LOFT_BIN")
        return None
    progs = sorted(glob.glob(os.path.join(ROOT, "tests", "scripts", "*.loft")))
    if limit:
        progs = progs[:limit]
    counts = collections.Counter()
    tag = re.compile(r'"k":"([A-Za-z]+)"')
    with tempfile.TemporaryDirectory() as td:
        snap = os.path.join(td, "snap.json")
        env = dict(os.environ, LOFT_DUMP_SNAPSHOT=snap, LOFT_TIMEOUT="60")
        for prog in progs:
            subprocess.run([binary, prog], env=env, capture_output=True)
            try:
                counts.update(tag.findall(open(snap, encoding="utf-8").read()))
            except OSError:
                pass
    print(f"  {len(progs)} programs introspected (the stdlib is parsed into every one)")
    return counts


def audit_dead():
    """The intersection: no producer AND never present in the corpus."""
    print("  screening constructions…")
    screened = {v for v in value_variants()
                if not any(k == "code" for _, k, _ in constructor_sites(v))}
    counts = census()
    if counts is None:
        return
    print()
    print(f"  {'variant':<12} {'corpus nodes':>13}   producer   verdict")
    for v in value_variants():
        n = counts.get(v, 0)
        flagged = v in screened
        if not flagged and n:
            continue  # ordinary live variant, nothing to say
        if flagged and n == 0:
            verdict = "DEAD — no origin, and never present"
        elif flagged:
            verdict = "live (built through a helper the screen reads as a rebuild)"
        else:
            verdict = "live — absent only because of the snapshot's STAGE"
        print(f"  {v:<12} {n:>13}   {'none' if flagged else 'yes':<9}  {verdict}")



# ---------------------------------------------------------------- reach ------
# `for_each_child`'s leaf arm names every variant with no child expression, so the
# CHILD-BEARING set is its complement — derived from the keystone rather than listed
# here, because a new variant must not silently join the wrong side of it.
def child_bearing_variants():
    src = open(DATA_RS, encoding="utf-8").read()
    i = src.index("pub fn for_each_child(&self, f: &mut impl FnMut(&Value))")
    j = src.index("// Leaves — no child expressions.", i)
    end = src.index("\n        }\n", j)
    leaves = set(re.findall(r"Value::([A-Za-z0-9]+)", src[j:end]))
    named = set(re.findall(r"Value::([A-Za-z0-9]+)", src[i:j]))
    if not leaves or not named:
        raise SystemExit("for_each_child's shape changed — `reach` cannot derive its child set")
    return named - leaves


def pass_through_variants():
    """Variants that forward exactly ONE child and carry nothing else, per `for_each_child`.

    These are the sharp signal.  Omitting `If` or `Call` from a walker's arms is usually a
    decision — the walker does not care about that shape.  Omitting a pass-through is never
    a decision about the shape, because the shape carries no information of its own: it
    means the subtree underneath is not entered and a verdict about it is issued anyway.
    All four walkers that turned out to be wrong the same way were missing one of these.
    """
    src = open(DATA_RS, encoding="utf-8").read()
    i = src.index("pub fn for_each_child(&self, f: &mut impl FnMut(&Value))")
    j = src.index("// Leaves \u2014 no child expressions.", i)
    out = set()
    # `item (| item)*` rather than `(item |?)+`: the repeated form lets the leading and
    # trailing `\s*` split the same whitespace two ways and makes the separator optional, so a
    # segment that does NOT end in `=> f(` backtracks exponentially (CodeQL py/redos).  This
    # form has one parse per input and extracts the identical set.
    arm_re = r"(Value::[A-Za-z0-9]+(?:\([^)]*\))?(?:\s*\|\s*Value::[A-Za-z0-9]+(?:\([^)]*\))?)*)\s*=>\s*f\("
    for arm in re.findall(arm_re, src[i:j]):
        out |= set(re.findall(r"Value::([A-Za-z0-9]+)", arm))
    if not out:
        raise SystemExit("for_each_child's shape changed \u2014 `reach` cannot derive its wrappers")
    return out


def bin_entry_files():
    """The `[[bin]] path =` entries from Cargo.toml — the roots production actually starts at.

    Read from the manifest rather than listed here so a new binary joins the root set on
    its own; a missed root reports its whole subtree as unreached, which is the direction
    that manufactures findings.
    """
    txt = open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8").read()
    paths = re.findall(r"^\[\[bin\]\]$(.*?)(?=^\[|\Z)", txt, re.M | re.S)
    out = []
    for block in paths:
        m = re.search(r'^path\s*=\s*"([^"]+)"', block, re.M)
        if m:
            out.append(os.path.normpath(m.group(1)))
    if not out:
        raise SystemExit("no [[bin]] path in Cargo.toml — `reach` has no roots")
    return out


RUST_FN = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?"
    r'(?:extern\s+"[^"]*"\s+)?fn\s+([a-z_][A-Za-z0-9_]*)'
)
# A call, and a function handed over as a VALUE (`.and_then(Self::arg_root_var)`), which
# has no parentheses after the name.  `parser::control::arg_root_var` has that as its only
# use, so a call-only matcher reports it as code production never runs.
CALL = re.compile(r"\b([a-z_][A-Za-z0-9_]*)\s*\(")
PATH_REF = re.compile(r"::([a-z_][A-Za-z0-9_]*)\b(?!\s*\()")

# An identifier merely PASSED as an argument (`.map(helper)`) is deliberately not matched:
# it cannot be told apart from an ordinary variable whose name collides with a function's,
# and adding it invented an edge to `ir_schema::value_from_parsed`, whose decode half has
# no production caller at all.
_STRING = re.compile(r'"(?:\\.|[^"\\\n])*"')
_LINE_COMMENT = re.compile(r"//[^\n]*")
_BLOCK_COMMENT = re.compile(r"/\*.*?\*/", re.S)


def code_only(body):
    """`body` with string literals and comments blanked, so prose cannot forge a call edge.

    Load-bearing: `parallel.rs`'s module doc names `scopes::is_par_safe` in a sentence
    explaining that it is wired to nothing, and counting that mention as a reference makes
    the function read as production-reached — inverting the one answer this mode exists to
    give.

    Strings go FIRST and do not cross a newline.  Stripping comments first lets a `//`
    inside a string truncate real code, and an unterminated quote inside a comment then
    swallows to the next quote anywhere in the file.  Block comments go LAST for the same
    kind of reason: `/*.loft` appears inside a line comment in `main.rs`, and treated as an
    opener it ate 97 KB — 1219 functions dropped out of the reachable set at a stroke.
    """
    return _BLOCK_COMMENT.sub(" ", _LINE_COMMENT.sub(" ", _STRING.sub('""', body)))


def code_only_positioned(text):
    """[`code_only`](#code_only) that keeps every newline, so a match offset still names
    its line.  The shared one collapses a multi-line block comment to one space, which
    slid `is_dbref`'s reported call sites four lines up the file — a line number that is
    close but wrong is worse than none, because it reads as checked."""
    keep = lambda m: "\n" * m.group(0).count("\n")  # noqa: E731
    return _BLOCK_COMMENT.sub(keep, _LINE_COMMENT.sub(keep, _STRING.sub(keep, text)))


def fn_bodies(path):
    """Yield (name, line, indent, body) for each `fn` in a file, nested ones INCLUDED.

    The body runs to the first `}` at the fn's own indentation, which is exact for
    rustfmt'd source and — unlike brace counting — cannot be thrown by a `'{'` char
    literal.  An outer fn's body therefore CONTAINS its nested helpers, which is the
    whole point: `parser::operators`'s two `try_swap`s are declared inside the functions
    that call them, and a splitter that ends the outer body at the nested `fn` line drops
    the only call edge either one has.  That reads as "production never reaches this",
    which is precisely the verdict this mode exists to make trustworthy.
    """
    lines = open(path, encoding="utf-8").read().split("\n")
    for i, line in enumerate(lines):
        m = RUST_FN.match(line)
        if not m:
            continue
        indent = len(line) - len(line.lstrip())
        sig = line
        j = i
        while "{" not in sig and ";" not in sig and j + 1 < len(lines):
            j += 1
            sig += lines[j]
        if "{" not in sig:
            continue  # a bodiless declaration (trait item, `extern` block)
        close = " " * indent + "}"
        end = len(lines)
        for k in range(j + 1, len(lines)):
            if lines[k].rstrip() == close:
                end = k
                break
        yield m.group(1), i + 1, indent, "\n".join(lines[j + 1 : end])


def call_graph():
    """(defs, bodies, edges, reached, called, root count) over `src/`, keyed by NAME.

    A name-keyed graph MERGES same-named functions, and every merge can only ADD
    reachability — so *not reached* is the strong verdict and *reached* is the weak one.
    Read the report that way round: this mode exists to find analyses production never
    runs, and over-reporting one would manufacture a finding.
    """
    defs = {}       # name -> [(path, line, is_test)]
    bodies = {}     # (path, line) -> body text
    for path in rust_files():
        regions = test_regions(path)
        for name, line, _indent, body in fn_bodies(path):
            is_test = any(lo <= line <= hi for lo, hi in regions)
            defs.setdefault(name, []).append((path, line, is_test))
            bodies[(path, line)] = body

    edges = {}
    for k, raw in bodies.items():
        b = code_only(raw)
        names = set(CALL.findall(b)) | set(PATH_REF.findall(b))
        edges[k] = {c for c in names if c in defs}
    roots = []
    for entry in bin_entry_files():
        full = os.path.join(ROOT, entry)
        roots += [(p, ln) for p, ln, t in defs.get("main", []) if os.path.samefile(p, full)]
    seen, stack = set(), list(roots)
    while stack:
        k = stack.pop()
        if k in seen:
            continue
        seen.add(k)
        for callee in edges.get(k, ()):
            for p, ln, is_test in defs[callee]:
                if not is_test and (p, ln) not in seen:
                    stack.append((p, ln))
    reached = {n for n, sites in defs.items() if any((p, ln) in seen for p, ln, _ in sites)}
    called = set()
    for cs in edges.values():
        called |= cs
    return defs, bodies, edges, reached, called, len(roots)


# A fallback that answers "no" — `_ => false`, `_ => None`.  This is the shape where a missed
# wrapper COSTS something: the walker reports the absence of a property it never looked for, and
# a caller that guards on it stops guarding.  A fallback answering `true` fails safe by
# comparison, and one that RETURNS A VALUE is usually a resolver over a narrower grammar.
NEGATIVE_FALLBACK = re.compile(r"^\s*(?:_|other|rest)\s*(?:if\b[^=]*)?=>\s*(?:false|None)\b", re.M)


def audit_reach():
    """Which catch-all `Value` walkers does a running loft binary actually reach?

    The catch-all list (see `walkers`) is long and every entry looks equally suspect.  The
    filter that ranks it is *does production run this*, and a one-level "is it called by
    anything outside a test" cannot answer it: the walkers that turned out to be test-only
    were called by ordinary-looking functions that were themselves only called from tests.
    Transitivity is the whole question, so this walks the call graph from each binary's
    `main`.

    A walker production never reaches costs nothing today and cannot be checked by running
    the suite either — nothing exercises it, so nothing reports that it drifted.  A walker
    production DOES reach is where a missing arm is a live wrong answer.
    """
    wrappers = child_bearing_variants()
    passthrough = pass_through_variants()
    defs, bodies, edges, reached, called, nroots = call_graph()
    rows = []
    for path in rust_files():
        if os.path.basename(path) == "data.rs":
            continue  # the keystone's own home
        regions = test_regions(path)
        for name, start, body in functions(path):
            if any(lo <= start <= hi for lo, hi in regions):
                continue
            named = set(VARIANT.findall(body))
            if len(named) < 3 or not re.search(rf"\b{re.escape(name)}\s*\(", body):
                continue
            if any(k in body for k in KEYSTONE) or not CATCHALL.search(body):
                continue
            omitted = wrappers - named
            if peels_span(body):
                omitted -= {"Span"}
            negative = bool(NEGATIVE_FALLBACK.search(body))
            rows.append(
                (
                    f"{rel(path)}:{start}",
                    name,
                    sorted(omitted & passthrough),
                    len(omitted - passthrough),
                    name in reached,
                    name in called,
                    negative,
                )
            )

    live = [r for r in rows if r[4]]
    print(f"catch-all `Value` walkers                      : {len(rows)}")
    print(f"  reached transitively from a binary's `main`  : {len(live)}")
    print(f"  production never reaches                     : {len(rows) - len(live)}")
    print(f"  (one-level 'called from outside a test'      : {sum(1 for r in rows if r[5])} — "
          "the check that cannot rank them)")
    print(f"  roots: {nroots} `main` fns from Cargo.toml's [[bin]] entries")
    print()
    print("  ⚠ The graph is keyed by function NAME, so same-named functions merge and every")
    print("     merge can only ADD reachability.  `production never reaches` is therefore the")
    print("     trustworthy verdict; a `reached` is weak.  The library's public API as called")
    print("     by another crate is not a root — this asks what a loft binary runs.")
    print()
    guards = [r for r in rows if r[4] and r[6]]
    print(f"  of the reached, fallback answers false/None : {len(guards)}   <- where a miss COSTS")
    print()
    print("  Production-reached first, then by omitted PASS-THROUGH wrappers — the shapes that")
    print("  carry no information, so skipping one is never a decision about the shape.  The")
    print("  trailing count is other child-bearing variants omitted, which is usually a choice.")
    print("  A hit is a MEASUREMENT to make, not a defect found.")
    for site, name, miss, others, is_live, _, neg in sorted(
        rows, key=lambda r: (not r[4], not r[6], -len(r[2]), -r[3])
    ):
        tag = "reached" if is_live else "  \u2014    "
        shown = ",".join(miss) if miss else "-"
        guard = "no!" if neg else "   "
        print(f"  {tag} {guard} {site:<40} {name:<30} skips {shown:<26} (+{others} other)")


PROJ_OPS = (
    "OpGetField",
    "OpGetVector",
    "OpVectorRef",
    "OpGetRecord",
    "OpGetText",
    "OpGetChar",
)
# Three ways to resolve a projection op, and a screen that sees only some of them
# under-reports the very family it exists to rank (the B4g lesson, one mode later):
#   * by def-number      — `d == data.def_nr("OpGetField")`
#   * through the shared predicate — `is_projection_op(data, d)`
#   * by NAME against a literal    — `data.def(*d).name() == "OpGetField"`, and the
#     `matches!(…name(), "OpGetVector" | "OpVectorRef" | …)` form, which is how every
#     hand-spelled list in the tree is written.
# The name form is anchored on `name()` so a `cl("OpGetRecord", …)` that CONSTRUCTS the
# op does not read as one that resolves it.
PROJ_LOOKUP = re.compile(
    r'def_nr\("(?:' + "|".join(PROJ_OPS) + r')"\)'
    r'|\bis_projection_op\s*\('
    r'|name\(\)[^;{}]{0,120}?"(?:' + "|".join(PROJ_OPS) + r')"',
    re.S,
)
LINE_COMMENT = re.compile(r"//.*")


def audit_spellings():
    """One notion, two IR spellings — who sees only the call-shaped one?

    A PROJECTION is `OpGetField(Var(b), …)` *or* `Value::TupleGet(b, i)`: the same
    language-level notion, one spelled as a CALL and one as a `Value` variant carrying its
    base as a var NUMBER.  A site that resolves the notion by op name therefore cannot see
    the tuple half, and no test can tell — the shape simply does not arrive.

    Reports every function that resolves a projection op by `def_nr` (or calls
    `is_projection_op`), and whether it also carries a `TupleGet` arm.  Comments are
    stripped first, so a doc line NAMING the variant does not read as handling it.

    ⚠ Per FUNCTION, so a pair that splits the question — one function matching the call and
    a caller carrying the tuple arm — reads as a miss.  Every hit is a site to READ, and the
    verdict is whether the fallback encodes a semantic boundary (QUALITY.md B6d), not
    whether the arm is present.
    """
    rows = []
    for path in rust_files():
        for name, start, body in functions(path):
            code = LINE_COMMENT.sub("", body)
            if not PROJ_LOOKUP.search(code):
                continue
            rows.append((f"{rel(path)}:{start}", name, "Value::TupleGet" in code))
    seen = len(rows)
    handled = sum(1 for r in rows if r[2])
    print(f"functions resolving a projection by OP NAME : {seen}")
    print(f"  ALSO handling the `TupleGet` spelling     : {handled}")
    print(f"  seeing only the call spelling             : {seen - handled}")
    print()
    print("  A hit is a site to READ: ask whether its fallback encodes a semantic boundary")
    print("  (a tuple element cannot reach here) or is just a shape nobody listed.")
    for site, name, ok in sorted(rows, key=lambda r: (r[2], r[0])):
        print(f"  {'TupleGet' if ok else '  --    '}  {site:<40} {name}")


# ── the opacity screen, one type FORMER at a time ────────────────────
# `Optional(τ)` is `τ` with a nullability BIT — compile-time only, same runtime layout
# (`Type::Optional`'s own doc).  So a site that resolves a shape by naming its variant
# (`Type::Vector(..) => …`) does not see the wrapped spelling of the same shape, and the
# value falls to whatever the catch-all does.  Same question as `spellings`, one type
# former over: one notion, two spellings, and only one of them matched.
#
# The question is asked of ONE former at a time — see `FORMERS` below.  `optional` is the
# founding former's mode name and the one the @FR-N-Shape ratchet gates; `former <Name>`
# points the same screen at the other wrappers.
TYPE_ARM = re.compile(
    r"(?<![A-Za-z0-9_])Type::([A-Za-z][A-Za-z0-9_]*)\s*(?:\([^)]*\)|\{[^}]*\})?"
    r"\s*(?:if\b[^\n]*)?(?:=>|\|)"
)
TYPE_LET = re.compile(
    r"(?:^|\W)(?:if\s+let|while\s+let|let)\s+(?:Some\()?(?<![A-Za-z0-9_])Type::([A-Za-z][A-Za-z0-9_]*)"
)
TYPE_MATCHES = re.compile(r"matches!\s*\([^;]{0,600}?(?<![A-Za-z0-9_])Type::([A-Za-z][A-Za-z0-9_]*)", re.S)
# `Type::` as a WHOLE path head — the lookbehind is what keeps `DefType::` out.
TYPE_BARE = re.compile(r"(?<![A-Za-z0-9_])Type::")
TYPE_DESCEND = re.compile(r"\.(?:any_node|for_each_child|contains_def)\s*\(")

# ── the type FORMERS this screen can be pointed at ────────────────────────────
# `Optional` is the founding case, not the only one.  The blindness it names — a site
# resolves a shape by listing `Type` variants, the WRAPPED spelling of that shape is not in
# the list, and the value takes the catch-all with nothing said — is a property of the
# WRAPPER, and the type language has four.  Two of the other three carry the same finding in
# their own doc: `Type::unrewritten` says "leaving it on makes every `matches!` over the type
# constructor miss (loft#943)", and `Type::peel_link` says a bare `matches!` "silently answers
# no for every `&` spelling", which cost the whole `File` surface through a `&File`
# (loft#753).  Asking one former at a time is what turns "does this class have a chokepoint?"
# from an opinion into a number — @PLN155 arc A.
#
# A row is (the peel VERBS that strip this former, the verbs that only prove AWARENESS of
# it, prose).  The split matters at one place: a verb that hands back a PEELED TYPE can be
# bound to a local and that local is then a peeled scrutinee, while a verb that answers
# *does this peel?* returns a `bool` and binding it proves nothing about the value under
# test.  `ret_promo_peels` is the one such verb today; keeping it out of the peel list is
# what stops `let peels = t.ret_promo_peels()` from clearing a bare test on `t`.  `Tuple` has no peel because it is
# not a wrapper: a tuple is its own shape, so the only way to see the tuple spelling of a
# question is to carry an arm for it — which is exactly what the screen then counts.  The
# peel lists are deliberately NOT shared: `base()` strips an `Optional` and leaves a `&`
# exactly where it was, so crediting it to `RefVar` would clear 700 sites that cannot see a
# `&τ` at all.
#
# `Optional`'s row carries the agnostic peel plus the two return-side peels that answer
# "which shapes peel" for their own callers.  `peel_link` belongs in it because it IS
# `base()` and then some: its first line is `let mut tp = self.base()`, and it goes on to
# strip every `&` link as well.  A site that upgrades from `base()` to `peel_link()`
# therefore sees through the `τ?` wrapper at least as well as it did before — but while
# that list named only `base`, the upgrade read as a REGRESSION and moved the site from
# peeling to opaque.  Measured on loft#1445, which moved three: `is_keyed`, `is_collection`
# and `keyed_known_type` all peel MORE than they used to and all three scored worse for it.
# An instrument that penalises the stronger peel argues against the fix it exists to find.
FORMERS = {
    "Optional": (
        ("base", "peel_optional", "peel_link", "ret_dep_shape", "ret_promo_base"),
        ("ret_promo_peels",),
        "`\u03c4?` \u2014 a nullability bit over \u03c4's own layout, same runtime shape (@FR-N-Shape)",
    ),
    "RefVar": (
        ("peel_link",),
        (),
        "`&\u03c4` \u2014 a link to a variable, which reads THROUGH to its referent (@FR-C-Ref)",
    ),
    "Rewritten": (
        ("unrewritten",),
        (),
        "`Rewritten(\u03c4)` \u2014 a built-in-place marker no slot can hold (loft#943)",
    ),
    "Tuple": (
        (),
        (),
        "`(\u03c4, \u2026)` \u2014 a compound whose MEMBERS are shapes the site never reaches",
    ),
}


class Former:
    """One type former, with the three regexes the screen asks about it.

    Built per former rather than written out per former, so a peel added to one row
    reaches the arm test, the scrutinee test and the bound-local test together.  Two
    homes for "what strips this wrapper" is the exact defect this instrument looks for.
    """

    def __init__(self, name):
        peels, aware, self.prose = FORMERS[name]
        self.name = name
        self.arm = f"Type::{name}"
        alt = "|".join(peels)
        # The FUNCTION unit's question is the weaker one — does this body know the wrapper
        # exists at all — so it reads the peels and the awareness verbs together.
        body_alt = "|".join(peels + aware)
        self.aware_call = re.compile(rf"\.(?:{body_alt})\s*\(" if body_alt else r"(?!)")
        # No peel verb: a regex that cannot match, rather than `(?:)` — which matches
        # the empty string and would score every site as seeing through.
        self.peel_call = re.compile(rf"\.(?:{alt})\s*\(" if peels else r"(?!)")
        # A local bound FROM a peel: the scrutinee is then a peeled value under another
        # name.  Both spellings occur — `let base = t.base()` and the same-name rebinding
        # `let tp = tp.base().clone()`.
        self.peel_bind = re.compile(
            r"(?<![A-Za-z0-9_])let\s+(?:mut\s+)?(?:\(([^)]{0,120})\)|([A-Za-z_][A-Za-z0-9_]*))"
            rf"\s*(?::[^=;]{{0,80}})?=\s*[^;]{{0,240}}?\.(?:{alt})\s*\(\s*\)"
            if peels
            else r"(?!)"
        )
        # The caller table's prefix: a receiver already peeled at the CALL site.
        self.peel_prefix = (
            "|".join(rf"\.{v}\(\)" for v in peels) if peels else r"(?!)"
        )


# The founding former, and the default every entry point falls back to.
OPTIONAL = Former("Optional")


def type_discriminated(code):
    """The `Type` variants this body pattern-matches (not the ones it CONSTRUCTS)."""
    return (
        set(TYPE_ARM.findall(code))
        | set(TYPE_LET.findall(code))
        | set(TYPE_MATCHES.findall(code))
    )


def classify_optional(code, former=None):
    """`sees` / `descends` / `opaque` for one function body, against one former."""
    former = former or OPTIONAL
    if former.arm in code or former.aware_call.search(code):
        return "sees"
    if TYPE_DESCEND.search(code):
        return "descends"
    return "opaque"


def type_verbs():
    """The `Type` VERBS in `data.rs`: methods in an `impl … Type` block, and free
    functions taking a `&Type`.

    The filter is what keeps the caller table about the shape question.  Without it the
    table listed `three_way_swap_exchanges_two_indices` — a test helper that happens to
    name two variants — beside `heap_dep`, and a reader has no way to tell which rows are
    the subject.
    """
    path = os.path.join(SRC, "data.rs")
    lines = open(path, encoding="utf-8").read().split("\n")
    impl_for_type, out = False, {}
    for i, line in enumerate(lines, 1):
        if line.startswith("impl"):
            impl_for_type = bool(re.match(r"impl(?:<[^>]*>)?\s+(?:[\w:]+(?:<[^>]*>)?\s+for\s+)?Type\b", line))
        elif line.startswith("}"):
            # A column-0 close ENDS the block.  Without this every free function
            # between `impl Type` and the next `impl` inherited the flag, and two
            # unit tests in the `mod tests` between them read as `Type` verbs.
            impl_for_type = False
        m = RUST_FN.match(line)
        if not m:
            continue
        if impl_for_type and not line.startswith(" "):
            continue  # a free fn textually inside the impl's range is not a method
        sig = line
        j = i - 1
        while "{" not in sig and j + 1 < len(lines):
            j += 1
            sig += lines[j]
        takes_type = re.search(r":\s*&(?:mut\s+)?\[?Type\b", sig) is not None
        takes_self = re.search(r"\(\s*&(?:mut\s+)?self\b", sig) is not None
        if (impl_for_type and takes_self) or takes_type:
            out[m.group(1)] = f"{rel(path)}:{i}"
    return out


# ── the per-TEST unit ─────────────────────────────────────────────────────────
# A FUNCTION is not the unit of this question; a shape TEST is.  `handle_field` peels
# `td` and then matches `exp_tp` bare, so a screen that absolves a body for peeling
# ANYWHERE reports the site that carries the defect as clean — measured, twice, on
# sites later found by hand (loft#1106, loft#1198).  Each `match` / `let` / `matches!`
# that names a `Type` variant is scored on ITS OWN scrutinee.
LET_START = re.compile(r"(?<![A-Za-z0-9_])(?:if\s+let|while\s+let|let)(?![A-Za-z0-9_])")
MATCH_START = re.compile(r"(?<![A-Za-z0-9_])match(?![A-Za-z0-9_])")
MATCHES_START = re.compile(r"(?<![A-Za-z0-9_])matches!\s*\(")
IDENT_ONLY = re.compile(r"^[&*\s(]*([A-Za-z_][A-Za-z0-9_]*)(?:\.clone\(\))?[\s)]*$")


def _balanced(code, i, opener="(", closer=")"):
    """Offset just past the `closer` matching the `opener` at `code[i]`, or len(code)."""
    depth = 0
    while i < len(code):
        if code[i] == opener:
            depth += 1
        elif code[i] == closer:
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    return len(code)


def _scan_to(code, i, stops):
    """Offset of the first char in `stops` at paren/bracket depth 0, from `i`.

    `<` and `>` are deliberately NOT tracked: they are ambiguous between a generic and a
    comparison, and a wrong guess ends a scrutinee in the middle of itself.  Every stop
    this is asked for (`{`, `;`, `&&`) is unreachable inside a type argument anyway.
    """
    depth = 0
    while i < len(code):
        c = code[i]
        if c in "([":
            depth += 1
        elif c in ")]":
            if depth == 0:
                return i
            depth -= 1
        elif depth == 0:
            if c in stops:
                return i
            if c == "&" and "&&" in stops and code[i : i + 2] == "&&":
                return i
        i += 1
    return len(code)


def _top_level_eq(code, i, end):
    """Offset of the binding `=` of a `let` pattern — not `==`, `=>`, `<=`, `>=`, `!=`."""
    depth = 0
    while i < end:
        c = code[i]
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        elif c == "=" and depth == 0:
            if code[i + 1 : i + 2] in ("=", ">") or code[i - 1 : i] in ("=", "<", ">", "!"):
                i += 2
                continue
            return i
        i += 1
    return -1


# Over PATTERN text every `Type::X` is a discrimination, so no trailing context is
# needed — and requiring one loses the LAST alternative of a `|`-chain, which
# `type_discriminated` does: `| Type::Trie(d, _, dep) = &in_type` ends in the binding
# `=`, not in `=>` or `|`.  That dropped `Trie` from `for_type` and `index_type` and
# split the keyed family into a five-variant list and a four-variant one — manufacturing
# a "these homes are short by Trie" finding out of the detector's own short list.
PATTERN_VARIANT = re.compile(r"(?<![A-Za-z0-9_])Type::([A-Za-z][A-Za-z0-9_]*)")


def pattern_variants(pats):
    """The `Type` variants a shape test's PATTERNS name."""
    return set(PATTERN_VARIANT.findall(pats))


def arm_patterns(block):
    """The PATTERN halves of a match's arms — no bodies, no guards.

    Both exclusions are load-bearing, and each was a false positive before it was made.
    An arm BODY that constructs a `Type::` is not a discrimination, and a nested `match`
    or `matches!` inside one belongs to itself.  A GUARD is a test in its own right and is
    scored as one: crediting `borrow_root`'s `matches!(… Type::Reference | …)` guard to the
    `match val.unspan()` it hangs off made a `Value` match read as a bare `Type` test, and
    put the pair at the head of the disagreement queue — one function appearing to peel in
    one place and not the other, when the two are not the same test at all.
    """
    out, i, depth, arm = [], 0, 0, 0
    while i < len(block):
        c = block[i]
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        elif depth == 0 and block[i : i + 2] == "=>":
            pat = block[arm:i]
            guard = re.search(r"(?<![A-Za-z0-9_])if(?![A-Za-z0-9_])", pat)
            out.append(pat[: guard.start()] if guard else pat)
            j = i + 2
            while j < len(block) and block[j].isspace():
                j += 1
            if j < len(block) and block[j] == "{":
                j = _balanced(block, j, "{", "}")
            else:
                j = _scan_to(block, j, ",")
            i, arm, depth = j + 1, j + 1, 0
            continue
        i += 1
    return "\n".join(out)


def shape_tests(code):
    """Yield (offset, kind, scrutinee, patterns) for each site that discriminates a `Type`.

    Three forms, because the language offers three: a `match`, a `let`/`if let`/`while let`
    pattern (including the `&&`-chained let-chain, which is how the parser writes most of
    them), and `matches!`.  A test is only yielded when its PATTERN half names a `Type`
    variant — a `match` over something else is not a shape test.
    """
    out, match_spans = [], []
    for m in MATCH_START.finditer(code):
        brace = _scan_to(code, m.end(), "{")
        if brace >= len(code):
            continue
        end = _balanced(code, brace, "{", "}")
        match_spans.append((brace, end))
        out.append([m.start(), "match", code[m.end() : brace], (brace + 1, end - 1)])
    for m in MATCHES_START.finditer(code):
        end = _balanced(code, m.end() - 1)
        inner = code[m.end() : end - 1]
        comma = _scan_to(inner, 0, ",")
        out.append([m.start(), "matches!", inner[:comma], inner[comma + 1 :]])
    for m in LET_START.finditer(code):
        stop = _scan_to(code, m.end(), "{;")
        eq = _top_level_eq(code, m.end(), stop)
        if eq < 0:
            continue
        pat = code[m.end() : eq]
        if "Type::" not in pat:
            continue
        rhs_end = _scan_to(code, eq + 1, "{;&")
        out.append([m.start(), "let", code[eq + 1 : rhs_end], pat])
    for row in out:
        if row[1] == "match":
            lo, hi = row[3]
            row[3] = arm_patterns(code[lo:hi])
    for off, kind, scrut, pats in out:
        # The catch-all needs the same lookbehind the two regexes beside it carry.  Written as
        # a bare `"Type::" in pats` it also matched `DefType::` — a different enum, naming
        # definition KINDS (`DefType::Struct`, `DefType::Enum`) and not type shapes — so every
        # `matches!(self.def_type(d), DefType::Struct | DefType::Enum)` counted as a shape test
        # that cannot see through a `τ?`, which it has no business seeing through.  There are
        # 548 `DefType::` mentions in `src/`, so this silently inflated the OPAQUE side of the
        # census the @FR-N-Shape ratchet gates on.
        if TYPE_ARM.search(pats) or TYPE_LET.search(pats) or TYPE_BARE.search(pats):
            yield off, kind, scrut, pats


def peel_bound(code, upto, former=None):
    """Locals bound from a peel before `upto` — `let base = t.base()`, `let tp = tp.base()`."""
    former = former or OPTIONAL
    names = set()
    for m in former.peel_bind.finditer(code, 0, upto):
        if m.group(2):
            names.add(m.group(2))
        else:
            names.update(re.findall(r"[A-Za-z_][A-Za-z0-9_]*", m.group(1)))
    return names


def test_sees(scrut, pats, peeled, former=None):
    """Does THIS test see the wrapped spelling, on its own scrutinee and its own patterns?"""
    former = former or OPTIONAL
    if former.arm in pats:
        return True
    if former.peel_call.search(scrut):
        return True
    m = IDENT_ONLY.match(scrut)
    return bool(m) and m.group(1) in peeled


def audit_optional(former=None):
    """Who can see through one type former's wrapper, and who resolves a shape without it?

    Written for `Optional` and parameterised by @PLN155 arc A; the prose below is the
    founding former's, and every other row of `FORMERS` asks the identical question about
    its own wrapper.

    `Optional(τ)` shares `τ`'s runtime layout and adds a compile-time bit, so the wrapper
    is a SPELLING of the same shape rather than a shape of its own.  A function that
    resolves the shape by naming variants therefore answers for `τ` and not for `τ?`, and
    nothing says so: the wrapped value simply takes the catch-all.  That is what loft#1106
    was — `deps_mut` did not peel while `depend` and `with_deps` did, so a nullable heap
    local's dep could be read and set but never cleared.

    Two halves.  The FUNCTION half classifies every body that discriminates on a `Type`
    variant: sees through (peels, or carries an `Optional` arm) · descends via the `Type`
    keystone · opaque.  The CALLER half is the list the function half cannot give — for
    each opaque verb defined in `data.rs`, who peels the receiver before asking and who
    does not.  Disagreement between two callers of ONE opaque verb is the shape that bites.

    The UNIT of the second half is the shape TEST, not the function.  A body that peels
    ANYWHERE used to read as seeing, which cleared every site loft#1198 was found at by hand:
    `handle_field` peels `td` and then matches `exp_tp` bare.  Each `match` / `let` /
    `matches!` is now scored on ITS OWN scrutinee, with a peel counted when it is in the
    scrutinee, in an `Optional` arm, or in the binding of a local the scrutinee names
    (`let base = t.base()` and the same-name `let tp = tp.base()` are both common).

    ⚠ The bias flips with the unit, so read the two halves differently.  The FUNCTION count
    is a floor: it under-reports.  The per-TEST count over-reports instead — a parameter its
    caller already peeled, or a scrutinee returned by a function that peels, cannot be seen
    from here.  That is why the ranking exists: a bare test is weak evidence on its own, and a
    LIST spelled bare in one home and peeled in another is a claim about two homes.
    """
    former = former or OPTIONAL
    rows, verbs, on_type, tests = [], {}, type_verbs(), []
    for path in rust_files():
        for name, start, body in functions(path):
            code = code_only(body)
            variants = type_discriminated(code)
            verdict = classify_optional(code, former)
            if variants:
                rows.append((f"{rel(path)}:{start}", name, verdict, len(variants)))
                if verdict == "opaque" and os.path.basename(path) == "data.rs" and name in on_type:
                    verbs[name] = f"{rel(path)}:{start}"
            # NOT gated on `variants`: the function unit's three regexes want a `Type::X`
            # followed by `=>`, `|` or a `let`, and a tuple pattern
            # (`let (Type::Enum(..), Type::Reference(..)) = …`) is none of those.
            # `wrap_dense_default_as_some` — one of the five writers @FR-L-Null-Tag names —
            # is invisible to the function unit for exactly that reason, so gating the
            # sharper pass on the blunter one would inherit its blind spot.
            for off, kind, scrut, pats in shape_tests(code):
                sees = test_sees(scrut, pats, peel_bound(code, off, former), former)
                line = start + code[:off].count("\n") + 1
                spelled = tuple(sorted(pattern_variants(pats)))
                tests.append((f"{rel(path)}:{line}", name, kind, sees, verdict, spelled))

    seen = len(rows)
    sees = sum(1 for r in rows if r[2] == "sees")
    desc = sum(1 for r in rows if r[2] == "descends")
    print(f"former: Type::{former.name} — {former.prose}")
    print(f"functions discriminating on a `Type` variant : {seen}")
    print(f"  see through the wrapper (peel or arm)      : {sees}")
    print(f"  descend via the `Type` keystone            : {desc}")
    opaque_functions = seen - sees - desc
    print(f"  opaque to a wrapped shape                  : {opaque_functions}")
    print()
    print("  callers of an OPAQUE `data.rs` verb — does the receiver peel first?")
    print(f"  {'verb':<22}{'peeled':>7}{'bare':>6}   bare call sites")
    src = {p: code_only_positioned(open(p, encoding="utf-8").read()) for p in rust_files()}
    for verb, where in sorted(verbs.items()):
        peeled, bare = 0, []
        call = re.compile(r"(?:(%s)(?:\.0)?\s*)?\.%s\s*\(" % (former.peel_prefix, verb))
        own = re.compile(r"fn\s+%s\s*\(" % verb)
        free = re.compile(r"(?<!fn )(?<![A-Za-z0-9_.])%s\s*\(([^()]{0,80})\)" % verb)
        for p, code in src.items():
            for m in call.finditer(code):
                if m.group(1):
                    peeled += 1
                else:
                    bare.append(f"{rel(p)}:{code[:m.start()].count(chr(10)) + 1}")
            for m in free.finditer(code):
                if own.search(code, max(0, m.start() - 4), m.end()):
                    continue  # the definition, not a call
                if re.search(former.peel_prefix, m.group(1)):
                    peeled += 1
                else:
                    bare.append(f"{rel(p)}:{code[:m.start()].count(chr(10)) + 1}")
        if not peeled and not bare:
            continue
        shown = " ".join(bare[:3]) + (f" +{len(bare) - 3}" if len(bare) > 3 else "")
        print(f"  {verb:<22}{peeled:>7}{len(bare):>6}   {shown}")
    print()
    blind = [t for t in tests if not t[3] and t[4] != "opaque"]
    # The disagreement ranking: group by the variant LIST a test spells and keep the ones
    # where some homes peel and some do not.  Two homes answering one question differently is
    # a claim, where a whole group spelled bare is only a convention.  Lists shorter than three
    # are dropped — a single-variant `Type::Reference(..)` test is a generic shape question,
    # not a shared notion, and including them buried the signal under 276 rows.
    lists = {}
    for site, name, kind, sees, _v, spelled in tests:
        if len(spelled) < 3:
            continue
        lists.setdefault(spelled, {True: [], False: []})[sees].append((site, name, kind))
    dis = {k: v for k, v in lists.items() if v[True] and v[False]}
    print(f"  shape TESTS naming a `Type` variant                : {len(tests)}")
    print(f"    the test itself sees through the wrapper        : {sum(1 for t in tests if t[3])}")
    print(f"    opaque on its OWN scrutinee                     : {sum(1 for t in tests if not t[3])}")
    print(f"    ← of those, inside a body the function unit clears: {len(blind)}")
    counts = {
        "opaque_functions": opaque_functions,
        "opaque_tests": sum(1 for t in tests if not t[3]),
    }
    print()
    print("  the queue the FUNCTION unit cannot produce — an opaque test in a body that")
    print("  peels somewhere else.  A hit is a site to READ: ask whether a `τ?` can arrive")
    print("  there, and whether what it falls to is the answer the rules give for `τ`.")
    for site, name, kind, _s, _v, _sp in sorted(blind):
        print(f"  {site:<44} {name:<40} {kind}")
    print()
    return counts
    print(f"  hand-spelled LISTS (3+ variants) whose homes DISAGREE : {len(dis)}")
    print(f"    bare tests inside them                             : {sum(len(v[False]) for v in dis.values())}")
    print("  Read a disagreement as a claim that two homes answer one question differently.")
    print("  A `data.rs` VERB in this list is not a hit on its own: `is_dbref` / `is_scalar`")
    print("  are layout predicates over a bare `Type` by design, with the peel at the caller")
    print("  (`ref_tuple_element_ok` is `is_scalar(tp.base())`) — read those through the")
    print("  caller table above instead.")
    for spelled, v in sorted(dis.items(), key=lambda kv: (-len(kv[1][False]), kv[0])):
        print(f"  == {'+'.join(spelled)}   [peel {len(v[True])} / bare {len(v[False])}]")
        for site, name, kind in v[True]:
            print(f"     peel  {site:<40} {name} ({kind})")
        for site, name, kind in v[False]:
            print(f"     BARE  {site:<40} {name} ({kind})")
    print()
    print("  functions with NO peel at all (the old unit's list, unchanged):")
    for site, name, verdict, n in sorted(rows, key=lambda r: (r[2] != "opaque", -r[3], r[0])):
        if verdict != "opaque":
            continue
        print(f"  {site:<44} {name:<40} {n:>2} variants")


# ── the @FR-N-Shape ratchet ───────────────────────────────────────────────────
# The rule says a SHAPE question answers alike for `τ` and `τ?`, and the `Optional`
# VARIANT was chosen so an omission is a COMPILE ERROR — but that fires for an
# exhaustive `match Type` only, and 1520 of the 2268 shape tests are a `matches!`,
# an `if let` or a catch-all arm, where it does not.  The population is far too
# large to walk, so this gates the DERIVATIVE: the count may fall and may not rise.
#
# A COUNT rather than a site allowlist, for `asan_leak_ratchet.sh`'s reason: the
# opaque tests are indistinguishable from one another by any pattern a suppression
# could name — they are ordinary `matches!` on a `Type` — so only the aggregate can
# tell a known one from a new one.
#
# Falling is not a failure: it prints the command and exits 0, so a PR that fixes
# sites is never blocked by its own improvement.  Re-pin in the same commit, so the
# baseline in the diff is the receipt for the walk that earned it.
#
# ⚠ The baseline is a DERIVED row, so it must never be CARRIED across a join or a
# rebase — re-run this and re-pin on the joined tree.  Two branches that each lowered
# the count hold two true numbers and neither is the merged tree's, and the same trap
# has cost QUALITY.md's audit row a false figure on eight consecutive joins.
RATCHET = os.path.join(ROOT, "index", "optional_ratchet.json")


def ratchet(counts, write):
    import json

    if write:
        with open(RATCHET, "w", encoding="utf-8") as fh:
            json.dump(counts, fh, indent=2, sort_keys=True)
            fh.write("\n")
        print(f"optional ratchet: pinned {counts} into {rel(RATCHET)}")
        return 0
    try:
        with open(RATCHET, encoding="utf-8") as fh:
            base = json.load(fh)
    except FileNotFoundError:
        print(f"optional ratchet: no baseline at {rel(RATCHET)} — run with --write")
        return 2
    grew = {k: (base.get(k), v) for k, v in counts.items() if v > base.get(k, v)}
    fell = {k: (base.get(k), v) for k, v in counts.items() if v < base.get(k, v)}
    for k, (was, now) in sorted(grew.items()):
        print(f"  GREW  {k}: {was} -> {now}")
    for k, (was, now) in sorted(fell.items()):
        print(f"  fell  {k}: {was} -> {now}")
    if grew:
        print(
            "\nERROR: a new shape test cannot see through `τ?` (@FR-N-Shape).\n"
            "  Peel the scrutinee — `.base()`, or `.data_shape()` where a `&` may also\n"
            "  reach it — or, if the site really is asking a NULLABILITY question, spell\n"
            "  that test rather than leaving it to a missing match arm.\n"
            "  The rule and its one open exception (`is_dbref`) are in formal/types.md."
        )
        return 1
    if fell:
        print("\nThe count fell — re-pin it in this commit:")
        print("  python3 scripts/ir_walker_audit.py optional --write-ratchet")
    else:
        print(f"optional ratchet: at baseline {base}.")
    return 0


def main():
    """Dispatch one audit MODE.  A `main` rather than a module-level run so the audit's
    definitions can be imported — `optional_rank.py` and `campaign_review.py` both read
    them, and loading a module should not run a minute of analysis."""
    mode = sys.argv[1] if len(sys.argv) > 1 else "both"
    if mode in ("walkers", "both"):
        print("== walkers ==")
        audit_walkers()
        print()
    if mode in ("producers", "both"):
        print("== producers ==")
        audit_producers()
    if mode in ("unspan", "both"):
        print("== unspan ==")
        audit_unspan()
        print()
    if mode == "reach":
        print("== reach (which catch-all walkers does production actually run?) ==")
        audit_reach()
    if mode == "dead":
        print("== dead variants (no producer AND absent from the corpus) ==")
        audit_dead()
    if mode == "spellings":
        print("== spellings (who sees only the CALL half of a projection?) ==")
        audit_spellings()
    if mode == "optional":
        check = "--check-ratchet" in sys.argv
        write = "--write-ratchet" in sys.argv
        if check or write:
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                counts = audit_optional()
            print("== @FR-N-Shape ratchet (opaque shape tests must not GROW) ==")
            sys.exit(ratchet(counts, write))
        print("== optional (who can see through the `τ?` wrapper?) ==")
        audit_optional()
    if mode == "former":
        # The same screen, pointed at another wrapper.  Reported only — the ratchet gates
        # `Optional` alone, because that is the former whose count a walk has been driven down
        # and whose baseline therefore means something; a fresh former's first number is a
        # measurement, not a bar to hold.
        name = sys.argv[2] if len(sys.argv) > 2 else ""
        if name not in FORMERS:
            sys.exit(f"former: name one of {', '.join(FORMERS)} (got {name!r})")
        print(f"== former {name} (who can see through a `Type::{name}` wrapper?) ==")
        audit_optional(Former(name))


if __name__ == "__main__":
    main()
