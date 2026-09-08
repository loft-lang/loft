#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# optional_rank.py — the OPAQUE functions of `ir_walker_audit.py optional`, ranked by whether
# an undischarged `τ?` can reach them (@PLN153 phase 4).
#
# The screen says WHICH functions resolve a shape by naming a `Type` variant without seeing
# through `Optional` (353 at the time of writing).  It cannot say which of them a nullable
# value ever reaches, and the plan's phase 4 wants them in that order: a body that reads a
# DECLARED type — a field's (`attr_type`, `.typedef`), a local's or parameter's (`vars.tp`,
# `function.tp`), a return's (`.returned`) — is where a `τ?` arrives with its wrapper ON, so
# it ranks first (tier 0); a body that decides an LVALUE place ranks next (tier 1); a body on
# the use path, where the value was usually peeled upstream, ranks last (tier 2).
#
# ⚠ A proxy, not a proof.  The tiers are regex evidence over the body's text: tier 0 also
# holds emitters that match `Void` or `Text` on a type that can never be nullable, and a
# function whose declared read sits in a helper it calls is in tier 2.  The list is the
# ORDER to walk in, and each function in the top tier is closed by reading it: either it
# peels through `base()`, or a probe cell shows a `τ?` cannot arrive there.  A function moved
# with neither is the B6u failure and does not count.
#
# usage:  scripts/optional_rank.py [<loft-root>]        (default: the repo this script is in)
#         prints one line per opaque function: tier  file:line  name  evidence  variants
import os
import re
import sys

root = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
os.chdir(root)


sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__))))
import ir_walker_audit as A  # noqa: E402 — after the chdir + path insert above

DECL = re.compile(r"attr_type\(|\.typedef\b|\.returned\(|vars\.tp\(|function\.tp\(|func\.tp\(|\.type_def\b")
LVALUE = re.compile(r"Value::Set\(|towards_set|set_type\(|change_var|assign|OpSet[A-Z]|lvalue|place\b")
USE = re.compile(r"call_ownership|args\b|argument")

rows = []
for path in A.rust_files():
    for name, start, body in A.functions(path):
        code = A.code_only(body)
        if not A.type_discriminated(code) or A.classify_optional(code) != "opaque":
            continue
        tags = []
        if DECL.search(code):
            tags.append("decl")
        if LVALUE.search(code):
            tags.append("lvalue")
        if USE.search(code):
            tags.append("use")
        tier = 0 if "decl" in tags else (1 if "lvalue" in tags else 2)
        variants = ",".join(sorted(A.type_discriminated(code)))
        rows.append((tier, A.rel(path), start, name, "+".join(tags) or "-", variants))
rows.sort()
tiers = [sum(1 for r in rows if r[0] == t) for t in (0, 1, 2)]
try:
    print(f"opaque functions: {len(rows)}  tier0(decl)={tiers[0]}  tier1(lvalue)={tiers[1]}  tier2(use/other)={tiers[2]}")
    for tier, path, start, name, tags, variants in rows:
        print(f"{tier}  {path}:{start:<6} {name:<44} {tags:<16} {variants}")
except BrokenPipeError:  # `| head` closed the pipe; the lines it wanted are out
    pass
