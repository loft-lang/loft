#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""How many pieces of code decide *is a free needed here*, and do any two decide it alike?

@PLN155's question, asked the way its owner states it: the frees the compiler emits are in the
right places; what needs bounding is the amount of code that DERIVES whether one is needed.
This counts that code and puts the derivations side by side, so "the same question in two
places" is visible rather than argued.

Two tables, because there are two ways to be a derivation:

  A. NAMED PREDICATES — a function whose body reads the ownership facts and whose answer gates
     a free.  These are the ones a fold can remove; `Function::proxy_says_owned` was six of
     them until @PLN155 phase 1.
  B. FREE SITES — every construction of a free op, with the FACTS the conditions guarding it
     read.  A site whose fact-set matches another's is asking the same question inline; a site
     whose fact-set is a superset is asking that question plus something of its own.

The fact VOCABULARY is the load-bearing part and is listed in `FACTS` below — one entry per
thing a licence can rest on, named after the rule it implements where there is one.  A site
reading facts outside it reads as `-`, which is a hole in the vocabulary rather than a site
with no licence, so the report says how many of those there are.

⚠ A REPORT, never a gate, and the grouping is a CLAIM.  Two sites reading the same facts may
still ask different questions — @PLN155 phase 1 found three that do, and recorded them in
`formal/IMPLEMENTATIONS.md` so the fold is not "finished" by merging them.  Read the sites
before folding; this says where to look.

Usage:
    scripts/free_licence_audit.py            # both tables
    scripts/free_licence_audit.py --sites    # + every free site with its facts
"""
import collections
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "src")

# The five spellings of a free (`use_analysis::OpSets::frees`) plus the scratch release, as
# they appear where one is CONSTRUCTED.  Read from the op names rather than from a hand list of
# call shapes, so a new construction helper is still found.
FREE_OP = re.compile(r'"(OpFree(?:Ref|RefTag|Text|RefIfDistinct|RefOrHandUp))"')
# ⚠ A free op NAMED is not a free CONSTRUCTED, and the difference is most of the population:
# `generation/ops/mod.rs`'s emitter registry names all five, `hoist.rs`'s `RECORD_FREE_OPS` is a
# matcher list, and `use_analysis`'s `OpSets::build` is the one home of the notion.  A
# construction resolves the name to a definition number or calls a build helper on the same
# line.  Measured: without this, 79 "sites" of which 58 read as guarded by nothing — a number
# about this script, not about the compiler.
CONSTRUCTS = re.compile(r"\b(?:def_nr|call|cl|op_call|make_call)\s*\(")
# ⚠ And a `def_nr` in a COMPARISON is a matcher, not a construction — `d == def_nr("OpFreeRef")`
# asks whether a node IS a free.  A construction builds a node, so a `Value::Call` or one of the
# build helpers is within a line or two of it.
MATCHER = re.compile(r"==|!=|\.contains\(|\bnrs\s*\(")
BUILDS = re.compile(r"Value::Call|\bcall\s*\(|\bcl\s*\(|\badd_op\s*\(")

# What a licence can rest on.  Keyed by the token that appears in the source; valued by the
# short name printed, so several spellings of one fact collapse to one column.
FACTS = {
    r"\bproxy_says_owned\b": "proxy+veto",         # @FR-O-Proxy + @FR-O-Override, folded
    r"\bowns_freeable_store\b": "sweep-licence",   # the pair + the parameter carve-out
    r"\bowns_displaced_store\b": "displaced",      # the pair, widened by borrows_one_argument
    r"\bdepend\(\)\.is_empty\(\)": "proxy",        # @FR-O-Proxy, written out
    r"\bdep\.is_empty\(\)|\bdeps\.is_empty\(\)": "proxy",
    r"\bis_skip_free\b": "veto",                   # @FR-O-Override
    r"\bis_argument\b|\bis_promoted_ret_buffer\b": "param",
    r"\bowned_refs\b": "latest",                   # @FR-O-Latest
    r"\bownership_of\b|\bOwn::|\breturn_ownership\b": "oracle",  # @FR-O-Oracle
    r"\bis_captured\b|\bcapture_adoption_owns_free\b": "capture",
    r"\blift_join_witness\b|\brbuf_witness\b|\bowner_witness\b|\blocal_owns\b": "witness",
    r"\bis_work_ref\b": "work-ref",
    r"\bfree_transferred\b|\bdrop_transferred\b|\bin_ret\b": "transferred",
    r"\brebind_params\b|\brebind_orig\b": "rebound-param",
    r"\bis_staged_text_temp\b|\barm_consumed\b": "staged",
}
FACT_RE = [(re.compile(k), v) for k, v in FACTS.items()]
FN = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?fn\s+([a-z_][a-z0-9_]*)")
# Functions that BUILD a free node and decide nothing — their licence is at every caller.
HELPERS = {"release_witness", "call", "cl"}
STRING_SPAN = re.compile(r'"(?:[^"\\]|\\.)*"', re.S)


def strip_prose(text):
    """Blank string CONTENTS that contain whitespace, keeping line count — a free named in a
    diagnostic is not a free, while `def_nr("OpFreeRef")` is how one is built (the
    discrimination `o_proxy_check.py` had to learn the hard way)."""

    def blank(m):
        body = m.group(0)
        if not re.search(r"\s", body[1:-1]):
            return body
        return '"' + "".join("\n" if c == "\n" else " " for c in body[1:-1]) + '"'

    return STRING_SPAN.sub(blank, text)


def code_lines(path):
    text = strip_prose(open(path, encoding="utf-8").read())
    return [l.split("//")[0] for l in text.split("\n")]


def rust_files():
    out = []
    for base, _d, files in os.walk(SRC):
        out += [os.path.join(base, f) for f in files if f.endswith(".rs")]
    return sorted(out)


def facts_in(text):
    return sorted({name for rx, name in FACT_RE if rx.search(text)})


COND = re.compile(r"\b(?:if|while|match)\b|&&|\|\|")


def enclosing_conditions(lines, fn_start, n):
    """Every condition line still OPEN at line `n`, plus a window above it.

    A free is licensed by the tests it sits INSIDE as much as by the one on its own line — the
    arm-return free in `parser/control.rs` reads its veto from a block two levels up — so the
    licence is the union of them.

    Brace depth is tracked per line and a condition line is kept while the site is still
    deeper than it was.  The first version filtered its own open-list on the running depth and
    kept almost nothing, which is why `free_vars` — where most of the compiler's frees are
    built, under conditions forty lines up — read as guarded by no fact at all.
    """
    depth, kept, fallthrough = 0, [], []
    for i in range(fn_start, n + 1):
        opens, closes = lines[i].count("{"), lines[i].count("}")
        if COND.search(lines[i]):
            kept.append((depth, i))
        depth += opens - closes
        closed = [(d, j) for d, j in kept if not (d < depth or j >= i - 1)]
        # An EARLY-EXIT guard licenses what it FALLS THROUGH to, so it does not stop applying
        # when its block closes — `tuple_owned_elem_frees` reads
        # `if is_skip_free(v) || !depend().is_empty() { continue }` and then frees below it.
        # `o_proxy_check.py` learned the same discrimination (its #1 and #4); reading only
        # still-open blocks is what made those sites report as guarded by nothing.
        for d, j in closed:
            body = "\n".join(lines[j : i + 1])
            if re.search(r"\b(?:continue|return|break)\b", body):
                fallthrough.append(j)
        kept = [(d, j) for d, j in kept if d < depth or j >= i - 1]
    rows = sorted({j for _d, j in kept} | set(fallthrough))
    return "\n".join(lines[j] for j in rows)


IDENT = re.compile(r"\b([a-z_][a-z0-9_]*)\b")
LET_OF = r"\blet\s+(?:mut\s+)?%s\s*(?::[^=;]{0,60})?="


def resolve_locals(lines, fn_start, end, conds, depth=2):
    """Pull in the DEFINITION of every local the guarding conditions name, one level at a time.

    The licence is usually not written at the free.  `get_free_vars` computes `owns` from four
    disjuncts and then `emit` from `owns` and four more, and the construction forty lines below
    says only `if emit {`.  A purely lexical scan at the construction therefore reads "guarded
    by nothing", which is how 33 of 49 sites first reported — a statement about the scan.

    Two levels, because that is what `emit` -> `owns` -> the facts costs; deeper would start
    dragging in unrelated locals and the fact-sets would all converge to everything.
    """
    text = conds
    for _ in range(depth):
        names = set(IDENT.findall(text))
        add = []
        for i in range(fn_start, end):
            m = re.search(r"\blet\s+(?:mut\s+)?([a-z_][a-z0-9_]*)\s*(?::[^=;]{0,60})?=", lines[i])
            if m and m.group(1) in names:
                # the binding's whole right-hand side, which may run over several lines
                j, chunk = i, []
                while j < end and len(chunk) < 14:
                    chunk.append(lines[j])
                    if lines[j].rstrip().endswith(";"):
                        break
                    j += 1
                add.append("\n".join(chunk))
        if not add:
            break
        text += "\n" + "\n".join(add)
    return text


def main():
    show_sites = "--sites" in sys.argv
    predicates, sites, unknown = {}, [], 0

    for path in rust_files():
        lines = code_lines(path)
        rel = os.path.relpath(path, ROOT)
        starts = [i for i, l in enumerate(lines) if FN.match(l)]

        # --- table A: named predicates whose BODY reads the facts and returns a bool
        for k, i in enumerate(starts):
            end = starts[k + 1] if k + 1 < len(starts) else len(lines)
            head, body = lines[i], "\n".join(lines[i:end])
            # Eight lines of signature, not three: `Scopes::owns_freeable_store` takes four
            # parameters one per line and its `-> bool` is on the sixth — the licence home
            # @PLN155 phase 1 built was missing from this table entirely for that reason.
            if "-> bool" not in "\n".join(lines[i : i + 8]):
                continue
            f = facts_in(body)
            # A licence predicate reads the PROXY or the ORACLE and at least one other fact;
            # one lone `is_argument` is a shape question, not a licence.
            if len(f) < 2 or not ({"proxy", "proxy+veto", "oracle"} & set(f)):
                continue
            # WHICH of the four questions the proxy answers (o_proxy_check.py's taxonomy): a
            # predicate deciding whether to ALLOCATE reads the same facts as one deciding
            # whether to FREE and is not the same question.  Taken from the name and the body,
            # so an `alloc` decider is not offered as a fold candidate for a `free` one.
            asks = "free" if re.search(r"free|Free", body) else (
                "alloc" if re.search(r"needs_db|_db\b|alloc", head + body) else "?")
            predicates[f"{rel}:{i + 1} {FN.match(head).group(1)}"] = (f, asks)

        # --- table B: every free construction, with the facts guarding it
        for n, l in enumerate(lines):
            m = FREE_OP.search(l)
            if not m or not CONSTRUCTS.search(l) or MATCHER.search(l):
                continue
            if not BUILDS.search("\n".join(lines[max(0, n - 2) : n + 3])):
                continue
            fn_start = max([s for s in starts if s <= n], default=0)
            fn_end = min([s for s in starts if s > n], default=len(lines))
            conds = enclosing_conditions(lines, fn_start, n)
            f = facts_in(resolve_locals(lines, fn_start, fn_end, conds))
            name = FN.match(lines[fn_start]).group(1) if FN.match(lines[fn_start]) else "?"
            if not f:
                unknown += 1
            sites.append((f"{rel}:{n + 1}", name, m.group(1), tuple(f)))

    print("\n=== A. named predicates that answer *is a free needed* ===\n")
    print(f"  {len(predicates)} predicate(s)\n")
    print(f"  {'site':<62} {'asks':<6} facts read")
    for site, (f, asks) in sorted(predicates.items(), key=lambda kv: (kv[1][1], kv[0])):
        print(f"  {site:<62} {asks:<6} {' + '.join(f)}")
    same = collections.defaultdict(list)
    for site, (f, asks) in predicates.items():
        same[(asks, tuple(f))].append(site)
    dup = {k: v for k, v in same.items() if len(v) > 1}
    if dup:
        print("\n  predicates asking ONE question off ONE fact-set — the fold candidates:")
        for (asks, f), members in sorted(dup.items()):
            print(f"    {asks}: {' + '.join(f)}")
            for m in members:
                print(f"      {m}")

    # A construction HELPER — a short function whose whole body builds the node and asks
    # nothing — is not a decision site; its licence is at its callers.  Counting it as
    # "guarded by nothing" reads as a missing licence when it is a missing CALLER.
    helpers = {n for _s, n, _o, f in sites if not f and n in HELPERS}
    sites = [(s_, n, o, f) for s_, n, o, f in sites if n not in HELPERS]

    print("\n\n=== B. free CONSTRUCTION sites, grouped by the facts guarding them ===\n")
    if helpers:
        print(f"  ({len(helpers)} construction HELPER(s) excluded — they build a free and decide"
              f" nothing: {', '.join(sorted(helpers))})\n")
    groups = collections.defaultdict(list)
    for site, name, op, f in sites:
        groups[f].append((site, name, op))
    print(f"  {len(sites)} construction site(s) in {len(groups)} distinct fact-sets"
          f"; {unknown} guarded by no fact this vocabulary names\n")
    for f, members in sorted(groups.items(), key=lambda kv: (-len(kv[1]), kv[0])):
        label = " + ".join(f) if f else "- (no named fact in the guarding conditions)"
        print(f"  [{len(members):2d}x] {label}")
        if show_sites:
            for site, name, op in members:
                print(f"         {site:<28} {name:<38} {op}")

    print("\n\n=== the number to drive down ===\n")
    shared = {f: m for f, m in groups.items() if len(m) > 1 and f}
    print(f"  named predicates                                  : {len(predicates)}")
    print(f"  distinct fact-sets across free sites              : {len(groups)}")
    print(f"  fact-sets used at MORE THAN ONE site (fold candidates): {len(shared)}"
          f", covering {sum(len(m) for m in shared.values())} sites")
    print("\n  A shared fact-set is a CLAIM that two sites ask one question, not proof —")
    print("  @PLN155 phase 1 found three that read alike and ask differently.  Read them.")
    print()


if __name__ == "__main__":
    main()
