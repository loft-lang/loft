#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# o_proxy_check.py — enforce `formal/ownership.md`'s two @FR-O-Proxy obligations:
#
#     a site that FREES on the empty-`deps` proxy MUST also consult @FR-O-Override, and
#     every site that reads the proxy MUST DECLARE which of the four facts it is reading.
#
# The second exists because the first is only decidable where a free is lexically reachable
# from the condition, and for most of these sites it is not.
#
# WHY there is an obligation at all.  `tp.depend().is_empty()` is how a site asks "does
# this binding own its store?", and it is a PROXY, not the oracle: a borrow whose dep list
# was never populated also reads empty, so the proxy answers "owner" for a borrower.
# `Function::is_skip_free` is the veto that makes it safe at a free site.  Consulting the
# veto only at the scope-exit sweep is what left an unconditional pre-Set free reachable
# inside a loop body, where it landed on the NEXT iteration's store — stale bytes without
# `LOFT_POISON`, SIGSEGV with it (loft#723).
#
# WHAT IS AND IS NOT A VIOLATION — the eight discriminations this check makes: 1-7 here, and
# the eighth, the declaration obligation, beside `DECL` below.  Each of 1-4 was a false
# positive before it was made; 5-8 came from the measurement that found this check green over
# its own violations, and each is falsified at the end rather than argued.
#
#   1. `!tp.depend().is_empty()` is USUALLY a DIFFERENT QUESTION — "is this a borrow?" —
#      and needs no veto, because a borrow is not freed either way.  But the SYNTAX does
#      not settle it: when the negated test guards an early exit (`continue` / `return`),
#      the free is on the FALL-THROUGH and the site concludes ownership exactly as a
#      positive test would.  Reading the `!` alone is what hid `scopes.rs`'s
#      `tuple_owned_elem_frees`, which frees a tuple element on empty element deps and
#      consulted no override — the coroutine loop variable it fired on was a borrow of the
#      generator's frame.  So: negated AND early-exit is a positive site.
#   2. The free must be in the region the condition GATES — the block an `if` opens, or the
#      uses of a `let` it binds — not merely nearby.  A 20-line window bled across function
#      boundaries and accused `dispatch::materialises_element`, a classifier that frees
#      nothing.
#   3. Comments are not code.  Matching `OpFreeRef` in prose accused
#      `codegen.rs`'s element-materialise arm, whose comment DISCUSSES a pre-Set free.
#   4. An early-exit guard INVERTS the sense of its test (discrimination 1), so its region is
#      what it FALLS THROUGH to, not the block it opens — and only as far as the keyword
#      actually reaches: `continue` leaves the enclosing loop body, `return` the function.
#      Taking the rest of the function for both accused `scopes.rs`'s
#      `null_arm_record_sources` loop, whose body only pushes to a list while the frees sit
#      far below in the same very long function.
#   5. @FR-O-Override is a per-BINDING flag (`Function::is_skip_free(v)`), so a site reading
#      `depend()` off a bare Type — a call's result type, a block result — has no variable to
#      consult it on and cannot discharge the obligation.  Those sites are counted and reported
#      separately (`no-binding`); their fact question is @FR-O-Oracle's, not this one's.
#      Reading them as positives accuses `null_test` and `own_joined_call_arms`, which are
#      handed a `&Type` and never see a variable number.
#      ⚠ ONLY when the region emits no free.  A site that EMITS one frees a store whatever the
#      proxy was spelled off, so it stays measured — `tuple_owned_elem_frees` reads
#      `elems[idx].depend()` (a tuple element type, no `tp(v)`) and is this check's original
#      catch; skipping it on the spelling would retire the one regression the check exists for.
#   6. A free is REACHED, not only emitted.  `get_free_vars` is what actually emits `OpFreeRef`,
#      and a site reaches it by WRITING the fact that sweep reads — so the free vocabulary has
#      to name the ownership-fact writers, not just the emitters.  Matching emitters alone is
#      what made 25 of the 29 positive verdicts vacuous: the site concludes ownership here and
#      the free lands in another function, so nothing in the gated region matched and every one
#      of them passed without proving anything.  The writer API is small and enumerable
#      (`variables/mod.rs`), and only the writes that can cause a free of THE PROXIED BINDING
#      count — which is two shapes, not the whole API:
#        `make_independent` / `without_deps`  strip the deps, so the scope-exit sweep frees it
#                                             (`scan_set`, scopes.rs:4925)
#        `set_skip_free`                      on the proxied binding, this is the spelling of a
#                                             MOVE: the source is vetoed because the TARGET has
#                                             taken its store and will free it — an interior
#                                             pointer if the proxy was wrong (loft#823 SIGSEGV,
#                                             `gen_set_first_ref_var_copy`, codegen.rs:3207)
#      Adding a dep (`depend`) is the restrictive direction and suppresses a free, so it is
#      absent — and so are `mark_inline_ref` and minting (`work_refs`, `create_unique`), which
#      make a NEW binding or suppress an init: neither frees the binding under test.
#   7. A writer counts only when it NAMES the binding the proxy was read on.  Without that,
#      `mark_inline_ref(db)` in `build_vector_list` (a write to the BACKING var, three lines
#      below a proxy read on `vec`) and a `set_skip_free` fifty lines away in the same
#      `parse_assign_op_inner` block both read as frees, and both are unrelated to the binding
#      the condition concluded about.  Two identifiers in one region is exactly the trap
#      IMPLEMENTATIONS.md names: a test that consults only one is consistent with itself and
#      proves nothing.
#
# FALSIFIED 2026-09-03 — each discrimination path was proved to fire by deleting the veto at
# one site and confirming the check goes red, one at a time:
#   scopes.rs `tuple_owned_elem_frees`      direct emitter, read off a tuple element (no `tp(v)`)
#   scopes.rs `scan_set` displaced strip    negated read + `make_independent` on the same binding
#   codegen.rs `gen_set_first_ref_var_copy` `set_skip_free` on the proxied binding (a move)
# and the declaration obligation the same way:
#   deleting `vector_needs_db`'s declaration           -> reported undeclared
#   re-declaring `scan_set`'s transition free as `copy` -> reported as a contradiction
#   two synthetic sites five lines apart, first declared -> the second reported undeclared
#     (and accepted, wrongly, with the decl_floor clamp removed)
# A check whose green is never contrasted with a red is the state this one shipped in.
#
# A REPORT that exits 1 on a violation, so it can gate.  Verdicts and the rule map live in
# doc/claude/formal/IMPLEMENTATIONS.md § The variable-lifetime map.
#
# Usage:  python3 scripts/o_proxy_check.py [-v]

import glob
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# The proxy read, in its two spellings: written out, and asked through the one home that asks
# it WITH its veto (@PLN155 phase 1).  Both are counted, because folding sites onto a predicate
# must not remove them from this check's population — a site whose obligation is discharged by
# construction is still a site that frees on the proxy, and a check that stops seeing it stops
# saying anything about it.  Measured: the fold took `9 of 29 reach a free` to `3 of 23` before
# this line was added, which reads like an improvement and is a blinding.
PROXY = re.compile(r"depend\(\)\.is_empty\(\)|proxy_says_owned\s*\(")
# The same read, one statement apart: `let deps = <…>.depend();` and a later `deps.is_empty()`.
# The one-expression spelling is the common one, and reading only it left this check blind to a
# form the tree already uses twice (`ownership_cfg.rs`'s Check B, `control.rs`'s arm-return
# free) — both compliant, so the hole cost nothing yet and was a claim about the classifier
# rather than about the code.  A gate that cannot see a spelling reports it as clean.
DEP_ALIAS = re.compile(
    r"\blet\s+(?:mut\s+)?([a-z_][a-z0-9_]*)\s*(?::[^=]+)?=\s*[^;]*\.depend\(\)\s*;"
)
# Emitting a free, not merely naming one.
FREE_EMIT = re.compile(r"OpFree|free_ref|emit_free")
# Discrimination 6 — WRITING the ownership fact `get_free_vars` reads reaches a free just as
# surely as emitting one, and is how most of these sites do it.
FREE_MANUF = re.compile(r"\b(?:make_independent|without_deps|set_skip_free)\s*\(\s*([^,)]*)")
# Discrimination 5 — the proxy read has to hang off a variable for the veto to be consultable,
# and discrimination 7 needs to know WHICH variable, so capture it.
BINDING = re.compile(
    r"\.tp\(\s*\*?\s*([A-Za-z_][A-Za-z0-9_]*)\s*\)\s*(?:\.base\(\))?\.depend\(\)"
    r"|proxy_says_owned\(\s*\*?\s*([A-Za-z_][A-Za-z0-9_]*)\s*\)"
)
# The obligation is discharged by the veto (`Function::is_skip_free`) or by one of the two
# homes that ask the proxy WITH it: `Function::proxy_says_owned` is the pair itself, and
# `Scopes::owns_freeable_store` is that pair plus the scope-exit sweep's parameter carve-out.
# A site calling either has discharged @FR-O-Override by construction rather than by
# remembering to.
DISCHARGE = re.compile(r"skip_free|proxy_says_owned|owns_freeable_store")
# Discrimination 8 — every positive site DECLARES which of the four facts it reads.
#
# The obligation the rest of this check enforces is decidable only where a free is lexically
# reachable from the condition.  For the rest it is not, and no amount of widening fixes that:
# a site can conclude ownership in the parser and have its free emitted by `get_free_vars`.
# What `ownership.md` § The facts that answer it says about those is that the CHOICE is
# invisible — "some legitimately want the proxy, some memo the oracle, and some free.  Nothing
# in the source distinguishes them, and both compile."  So the site says which:
#
#   @FR-O-Proxy asks copy    chooses copy-vs-alias / materialise-vs-view.  Authorises no free;
#                            a wrong answer costs a copy, never a release.
#   @FR-O-Proxy asks alloc   decides whether to ALLOCATE or null-init a store.  The opposite
#                            direction from a free.
#   @FR-O-Proxy asks oracle  an independent derivation that drives no emission — @PLN94's
#                            flow-sensitive oracle, or witness accounting that consults
#                            @FR-O-Oracle for the real answer.
#   @FR-O-Proxy asks free    concludes ownership and a free follows, wherever it is emitted.
#                            This one ALSO requires @FR-O-Override, exactly as a lexically
#                            visible free does.
#
# A declaration is a claim, so the check contradicts it where it can: a site declaring
# anything but `free` while a free IS visible in the region it gates is reported, not trusted.
DECL = re.compile(r"@FR-O-Proxy\s+asks\s+(copy|alloc|oracle|free)\b")
FN = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?fn\s")
LET = re.compile(r"let\s+(?:mut\s+)?([a-z_][a-z0-9_]*)\s*=")


def code_only(text):
    """Strip line comments — discrimination 3."""
    return "\n".join(l.split("//")[0] for l in text.split("\n"))


def enclosing_guards(lines, fn_start, n):
    """The lines that OPEN a block still open at line `n` — the guards this site sits inside.

    The obligation is that a site freeing on the proxy consults @FR-O-Override, and a site can
    do that from an enclosing `if !…skip_free(x) { … }` as legitimately as from its own
    condition.  Reading only the statement is what made `control.rs`'s arm-return free look
    unguarded when its whole block is inside exactly that test — a false positive, and one
    only visible once the aliased spelling was in scope at all.
    """
    depth, opens = 0, []
    for i in range(fn_start, n):
        code = code_only(lines[i])
        opened = code.count("{") - code.count("}")
        if opened > 0:
            opens.append((depth, lines[i]))
        depth += opened
        opens = [(d, l) for (d, l) in opens if d < depth]
    return "\n".join(l for _, l in opens)


def frees(region, binding):
    """Does the region this condition gates reach a free of `binding`? (discriminations 6-7)"""
    code = code_only(region)
    if FREE_EMIT.search(code):
        return True
    if binding is None:
        return False
    return any(m.group(1).strip().lstrip("*&") == binding for m in FREE_MANUF.finditer(code))


def negated(line, pos):
    """Discrimination 1: is this the `!…is_empty()` is-it-a-borrow form?"""
    i = pos
    while i > 0 and (line[i - 1].isalnum() or line[i - 1] in "_.()[]*&:"):
        i -= 1
    return i > 0 and line[i - 1] == "!"


EARLY_EXIT = re.compile(r"^\s*(continue|return\b)")


def early_exit_guard(region):
    """Discrimination 4: does this gated block do nothing but leave?

    A `if !…is_empty() { continue; }` concludes ownership on the FALL-THROUGH, so the
    negated spelling is a positive site and its region is what comes after, not the block.
    Returns the keyword, which decides HOW FAR the fall-through reaches, or None.
    """
    body = [l for l in code_only(region).split("\n")[1:] if l.strip() not in ("", "}", "{")]
    if not body:
        return None
    kinds = {m.group(1) for l in body for m in [EARLY_EXIT.match(l)] if m}
    return kinds.pop() if len(kinds) == 1 and len(kinds) == len(body) else None


def fallthrough_region(lines, n, fn_end, kind):
    """What an early-exit guard at line `n` gates by falling through.

    A `continue` leaves the enclosing LOOP BODY, so the free it would authorise has to be
    in that body; a `return` leaves the function.  Taking the rest of the function for both
    accused `scopes.rs`'s `null_arm_record_sources` loop, whose body only pushes to a list
    while the frees sit far below in the same (very long) function.
    """
    if kind == "return":
        return "\n".join(lines[n:fn_end])
    depth, j = 0, n
    while j < fn_end:
        depth += lines[j].count("{") - lines[j].count("}")
        j += 1
        if depth < 0:
            break
    return "\n".join(lines[n:j])


def _comment_only(line):
    """A line that carries no code — it must not consume the statement window's budget."""
    return not code_only(line).strip()


def gated_region(lines, n, fn_end, decl_floor=0):
    """Discrimination 2: the statement, and the region its result actually gates.

    The 14-line budget counts CODE lines.  Counting raw lines instead spends the window on
    prose: `output_set_body` explains each conjunct of one boolean in a comment, so its
    `is_skip_free` sits fifteen lines under the proxy read and fell outside a statement it
    is part of — the check then asked a site for a declaration it had already discharged.
    """
    a, budget = n, 14
    while a > 0 and budget:
        if _comment_only(lines[a - 1]):
            a -= 1  # prose is stepped over, and is neither a boundary nor a cost
            continue
        if re.search(r"[;{}]\s*$", code_only(lines[a - 1]).rstrip()):
            break
        a -= 1
        budget -= 1
    b, budget = n, 14
    while b + 1 < fn_end and budget:
        code = code_only(lines[b]).rstrip()
        if re.search(r"[;{]\s*$", code):
            if not code.endswith("{") or code.count("(") == code.count(")"):
                break
            # An inline block INSIDE the expression — `… || {` with a paren still open —
            # does not end the statement, and reading it as the gated block truncates one:
            # `output_set_body`'s `is_skip_free` sits past such a block, so the check asked
            # a site to declare what it had already discharged.  Step over to the matching
            # `}` and keep walking the same condition.
            depth, k = 0, b
            while k < fn_end:
                depth += lines[k].count("{") - lines[k].count("}")
                k += 1
                if depth <= 0:
                    break
            b = k - 1
            budget -= 1
            continue
        b += 1
        if not _comment_only(lines[b]):
            budget -= 1
    stmt = "\n".join(lines[a : b + 1])
    # The declaration window reaches back over the comment block above the statement — but
    # never past the PREVIOUS proxy site in this file, or two sites closer than ten lines
    # would share one declaration and the second would be accepted undeclared.  No pair is
    # that close today (measured); the clamp is what keeps that from being load-bearing.
    decl = "\n".join(lines[max(decl_floor, a - 10) : b + 1])
    if lines[b].rstrip().endswith("{"):
        depth, j = 0, b
        while j < fn_end:
            depth += lines[j].count("{") - lines[j].count("}")
            j += 1
            if depth <= 0:
                break
        return stmt, decl, "\n".join(lines[b:j])
    m = LET.search(code_only(stmt))
    if m:
        # A `let NAME = <proxy cond>;` gates whatever the `if NAME …` blocks contain — the
        # free is inside the block, not on the line naming NAME.  Collecting only the
        # mentioning LINES is what made this check vacuous on the very regression it exists
        # for, so take each use-line's block too.
        nm = m.group(1)
        out = []
        j = b
        while j < fn_end:
            if re.search(rf"\b{nm}\b", code_only(lines[j])):
                out.append(lines[j])
                if lines[j].rstrip().endswith("{"):
                    depth, k = 0, j
                    while k < fn_end:
                        depth += lines[k].count("{") - lines[k].count("}")
                        k += 1
                        if depth <= 0:
                            break
                    out.extend(lines[j + 1 : k])
                    j = k
                    continue
            j += 1
        return stmt, decl, "\n".join(out)
    return stmt, decl, ""


verbose = "-v" in sys.argv
pos = neg = nobind = reaching = 0
viol = []
undecl = []
contra = []
census = {}
for path in sorted(glob.glob(os.path.join(ROOT, "src", "**", "*.rs"), recursive=True)):
    lines = open(path, encoding="utf-8").read().split("\n")
    starts = [i for i, l in enumerate(lines) if FN.match(l)] + [len(lines)]
    rel = os.path.relpath(path, ROOT)
    last_proxy = -1
    # Locals holding a dep list, so `<name>.is_empty()` below reads as the proxy it is.
    # Function-scoped: the name is re-seeded at each `let`, and a stale one only ever adds a
    # site to inspect, never removes one.
    dep_aliases: set[str] = set()
    fn_starts = set(starts)
    for n, line in enumerate(lines):
        # A name means nothing outside the function that bound it, and carrying one across
        # would read an unrelated `xs.is_empty()` as an ownership question.
        if n in fn_starts:
            dep_aliases.clear()
        if line.lstrip().startswith(("//", "///")):
            continue
        am = DEP_ALIAS.search(code_only(line))
        if am:
            dep_aliases.add(am.group(1))
        alias_hits = [
            am2
            for am2 in re.finditer(r"\b([a-z_][a-z0-9_]*)\.is_empty\(\)", code_only(line))
            if am2.group(1) in dep_aliases
        ]
        for m in list(PROXY.finditer(code_only(line))) + alias_hits:
            fn_end = next((s for s in starts if s > n), len(lines))
            stmt, decl, region = gated_region(lines, n, fn_end, decl_floor=last_proxy + 1)
            last_proxy = n
            bm = BINDING.search(code_only(line), max(0, m.start() - 90))
            # Either alternative of BINDING: the written-out `tp(v).…depend()` or the folded
            # `proxy_says_owned(v)`.  Both name the variable the veto must be consulted on.
            binding = (
                (bm.group(1) or bm.group(2))
                if bm is not None and bm.end() >= m.start()
                else None
            )
            if negated(line, m.start()):
                # Discrimination 4: an early-exit guard inverts the sense, so the site
                # concludes ownership on what it FALLS THROUGH to.  Resolved BEFORE
                # discrimination 5, whose "does this site free at all?" escape hatch has to
                # read the region the site really gates: `tuple_owned_elem_frees` gates a bare
                # `continue` and does its freeing on the fall-through.
                # Discrimination 1, amended: `!…is_empty()` is usually the is-it-a-borrow
                # question, but a region that WRITES the ownership fact concludes on the proxy
                # in the permissive direction whichever way the test is spelled — `scan_set`
                # reads "this still looks like a borrow" and strips the deps so the sweep frees
                # it anyway.  Reading the `!` alone is what hid that site, so such a region
                # falls through to the obligation instead of being dismissed here.
                kind = early_exit_guard(region)
                if kind is not None:
                    region = fallthrough_region(lines, n, fn_end, kind)
                elif not frees(region, binding):
                    neg += 1
                    continue
            if binding is None and not FREE_EMIT.search(code_only(region)):
                # Discrimination 5: no variable, so no `is_skip_free` to consult — and no
                # free emitted here either, so nothing to hold this site to.
                nobind += 1
                if verbose:
                    print(f"  no-binding {rel}:{n + 1}")
                continue
            pos += 1
            reaches = frees(region, binding)
            if reaches:
                reaching += 1
            dm = DECL.search(decl)
            verdict = dm.group(1) if dm else None
            census[verdict] = census.get(verdict, 0) + 1
            if dm is None:
                undecl.append((f"{rel}:{n + 1}", line.strip()[:74]))
            elif reaches and verdict != "free":
                contra.append((f"{rel}:{n + 1}", verdict))
            fn_start = max([sx for sx in starts if sx <= n], default=0)
            discharged = DISCHARGE.search(code_only(stmt)) or DISCHARGE.search(
                enclosing_guards(lines, fn_start, n)
            )
            if (reaches or verdict == "free") and not discharged:
                viol.append((f"{rel}:{n + 1}", line.strip()[:74]))
            elif verbose:
                # Say WHICH green this is.  "discharged" is a proof — the site reaches a free
                # and consults the veto.  "no-free-in-region" is not: nothing in the region
                # this condition gates reaches a free, so the site is either asking a
                # different question or deciding one further away than the region can see.
                why = "discharged" if reaches else "no-free-in-region"
                print(f"  ok   {rel}:{n + 1}  ({why})")

print(
    f"@FR-O-Proxy — empty-deps ownership sites: {pos} positive, "
    f"{neg} negated (is-a-borrow), {nobind} no-binding (@FR-O-Oracle's question)"
)
# The check's own control.  A positive site whose region reaches no free is a verdict that
# proves nothing, and a run where NONE of them reach one is a green with no content — the
# state this check shipped in.  Print the split so a reader can never mistake the second
# kind of green for the first.
print(f"  {reaching} of {pos} reach a free (emitted or written); {pos - reaching} do not")
# The census is the point of the declarations: which fact each site reads, countable.
print(
    "  declared: "
    + ", ".join(f"{v or 'UNDECLARED'} {n}" for v, n in sorted(census.items(), key=lambda kv: (kv[0] or "~")))
)
if not (viol or undecl or contra):
    print("  ok — every site that frees on the proxy also consults @FR-O-Override,")
    print("       and every site declares which of the four facts it reads")
    sys.exit(0)

if viol:
    print(f"\n  {len(viol)} site(s) FREE on the empty-deps proxy without consulting the override:\n")
    for site, text in viol:
        print(f"    {site}\n      {text}")
    print("""
  An empty dep list does not mean "owner" — it means "nothing recorded a dep here", which
  is also true of a borrow nobody populated.  Freeing on it releases a store someone else
  owns.  Add `&& !<vars>.is_skip_free(v)` to the condition, or read @FR-O-Oracle
  (`use_analysis::ownership_of`) instead of the proxy.""")

if undecl:
    print(f"\n  {len(undecl)} site(s) read the proxy without declaring which fact they read:\n")
    for site, text in undecl:
        print(f"    {site}\n      {text}")
    print("""
  Say so in a comment on or above the condition, with the question it asks:

      // @FR-O-Proxy asks copy   — chooses copy-vs-alias; authorises no free
      // @FR-O-Proxy asks alloc  — decides whether to ALLOCATE, not whether to release
      // @FR-O-Proxy asks oracle — an independent derivation that drives no emission
      // @FR-O-Proxy asks free   — a free follows; then @FR-O-Override is required too

  The empty dep list answers all four questions and means something different in each.
  Without the declaration a reader cannot tell a site that legitimately wants the proxy
  from one that reached for the wrong fact, and both compile — which is how the count of
  these sites grew from 24 to 38 unnoticed.""")

if contra:
    print(f"\n  {len(contra)} site(s) DECLARE a non-free question while a free is visible:\n")
    for site, verdict in contra:
        print(f"    {site}  declares `{verdict}`, but the region it gates reaches a free")
    print("""
  A declaration is a claim about the site, not a way past this check.  Either the question
  is `free` — and @FR-O-Override is required with it — or the free in that region belongs
  to some other binding and the condition should not be gating it.""")

print("\n  formal/ownership.md § The facts that answer it.")
sys.exit(1)
