<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN124 — a library-implementable interpolation

## Status

**SHIPPED 2026-08-03 — H1–H7, both backends.** A format string whose TARGET TYPE
implements the interpolation contract hands over its literal and hole parts
instead of appending them into a text buffer. `text` is unchanged, proven as a
byte-identical IR diff. Its first consumer is @PLN23 S4, built on it.

**The contract is [INTERFACES.md § Interpolation targets](../../INTERFACES.md)** —
the methods, the named-type hole and how its name is derived, the two refusals,
and why the parser has to carry this. The user-facing entry is
[`@F94`](https://github.com/loft-lang/features/issues/94). The design reasoning —
why neither the type system nor `const` can carry the distinction — is
[@PLN23's INTERPOLATION_HOOK.md](../23-db-clients/INTERPOLATION_HOOK.md).

This file is the closure record: what the build COST, and what it corrected.

## Where the target comes from

`Parser::interpolation_target` is a fifth SHAPE read off the one `⇐` expected-type
channel, beside `lambda_hint` / `enum_hint` / `vector_hint` / `read_target_type` —
not a sixth side-channel. `var_tp` carries the target for a typed-local
declaration, a typed reassignment and a struct-field init; `expected` carries it
for a call argument (free function and method) and a return body. That is the
same two-source rule the bare-enum-variant branch already used.

## What it cost, and what the build corrected

**The mint has to be pass-stable, and that is the sharp edge.** Taking the branch
mints an accumulator variable, and the variable tables persist across passes BY
NAME (loft#662), so a decision that differed between passes would shift every
later work variable underneath itself. Method defs are collected on both passes —
the property the `to_text` hook already relies on — so the branch is stable; and
the accumulator draws from its own `__fmt_N` counter (`Function::work_format`)
rather than sharing `__work_N` or `__ref_N`, for the same reason every other work
namespace has one of its own.

**The expected-type channel leaked across a nested call.** The hints that read it
each SET it when they applied and none CLEARED it when they did not, so in
`take(build_one("arg"))` the string literal — `build_one`'s `text` parameter — was
checked against `take`'s parameter type. Latent before this arc (the enum,
collection and function shapes rarely nest that way) and immediate once a `text`
parameter could be shadowed by an outer struct target. The channel is now cleared
per argument, at both the free-function and the method site.

**A format spec on a hole is REFUSED.** `"{x:>8}"` into a built type has nothing
to format — the value is handed over, not rendered — so it is an error rather than
a silently ignored spec.

**The expected-type channel leaked a second time — into the HOLE.** A hole is not
the destination, so a string literal inside one must not inherit the
destination's type; without that, `q: SqlText = "{"seed"}"` checked the inner
literal against `SqlText` and it took the BUILD path, so a string that was plainly
the author's value came back as a second accumulator. The same leak the arc had
already closed per call argument, one level in, and found only once H7 wrote a
text hole inside a built statement.

**The fix had to be narrower than the first one written.** Clearing `expected`
for the whole hole — the obvious mirror of the call-argument fix — broke
`store_load_layout_gate` on `--native`: the hole
`"{(h[42] ?? Tile { … }).name}"` is a KEYED LOOKUP, and a keyed lookup resolves
its record type through that same channel, so blanking it silently changed the
schema the generated `init()` replays. The suite caught it and a pristine-`HEAD`
binary in a `git worktree` proved whose it was. What is gated now is only the
TARGET derivation, and only its `expected` source (`var_tp` still applies, since
a declaration written inside a hole does name a destination). The channel itself
is untouched. Re-proved inert on the corpus, and the gate passes again.

The narrow gate costs one shape: a format string in ARGUMENT position inside a
hole (`"{ build("p{n}q") }"`) is plain text rather than a built value. That is a
visible type error at the call, not a silent difference, and it was worth more
than reaching into a channel that carries other facts.

**Nullability is not a kind.** A `text?` hole is a text hole whose value may be
absent; `format_hole` peels `Optional` and lets the target's own `hole_text`
parameter type decide whether it accepts one. That is what lets `SqlText` make SQL
NULL a distinct bound value rather than the text `"null"`.

## Proof

- **H1, inertness.** `bytecode-comparisons/format-corpus.loft` is one function per
  format-string path the dispatch can reach — literals, bare holes, alternation,
  the numeric spec grammar, a `text?` hole, a fault-prone hole (`OpTagFault`), an
  inner fault that must NOT tag, struct/JSON/pretty specs, a custom `to_text` spec,
  expression holes, three `for`-comprehension forms, backtick multiline, escaped
  braces, `+=` accumulation, and argument position. 104 format sites.
  `loft introspect` before/after is **byte-identical**; an empty diff is the whole
  proof, and it is re-checked after each change to the arc.
- **H2/H3/H4/H6.** `tests/scripts/interpolation-hook.loft` asserts the call
  SEQUENCE rather than the result — a target that only checked the final string
  could not tell the hook from ordinary formatting. Every scalar kind routes to
  its own method, and so do a struct and an enum; both backends.
  `tests/scripts/pln124-hole-kind-refused.loft` pins the two REFUSALS (an
  unhandled kind, a spec on a hole), which are the rules that would be cheapest
  to soften and the ones that put a value back on the text path if softened.
- **H5/H6/H7.** `tests/native.rs::one_sql_interface_drives_four_different_c_libraries`
  runs the whole contract against sqlite, postgres and mariadb, on both backends,
  and compares the lines WHOLE. Each field was proven to move: replacing sqlite's
  `db_call` with `return true` gives `guard=true rows=0`, and a `procedural` that
  never refuses gives `ctl=true`.
- **The target shape was captured BEFORE the parser was touched.**
  `bytecode-comparisons/target-shape.loft` is the hand-written program whose IR
  the branch had to reproduce, proven on both backends first. It also settled a
  design question by measurement: a default-constructed `T { }` is equivalent to a
  named constructor, so the contract needs only methods and `Interpolated` stays a
  pure interface.

## H6 and H7 — what the consumer proved

**H6, the identifier.** `SqlIdent` in `tests/fixtures/sqldb/sql`: `ident(text)`
admits ASCII `[A-Za-z_][A-Za-z0-9_]*` up to 64 bytes and answers `null` for
anything else, at CONSTRUCTION rather than at execution. A refused one poisons the
statement, and `statement` therefore answers `text?` — **saying it in the type is
what makes each backend handle it**, where a boolean beside the text would be a
flag every backend has to remember to read.

The identifier is recorded structurally and quoted at ASSEMBLY time, not when the
hole was filled, because the quote is per-dialect: mariadb reads `"loft_p"` as a
string literal and wants a backtick. Measured — handing mariadb the ANSI quote the
other three use answers *`error in your SQL syntax … near '"loft_p" WHERE`*, so
the split is not a preference.

**H7, procedures.** A procedure is a named, parameterised statement, and the hook
already separates the parts: the table name is a `SqlIdent`, and a hole in a body
is a PARAMETER typed by the value it will receive. **A procedure is declared by
writing its statement, not by declaring its signature a second time.** postgres
and mariadb `CREATE OR REPLACE PROCEDURE` and `CALL`; sqlite and duckdb keep the
definition in the process through one shared registry. The uniform line is
byte-identical on all of them — where a procedure lives is not something a caller
can see.

**The design's one correction.** It expected procedural control flow to be the
sqlite-versus-servers line: "a body using them must be rejected by the sqlite
backend". Measuring moved the line. mariadb writes procedural bodies in SQL/PSM
and postgres in plpgsql or a `BEGIN ATOMIC` body, and neither reads the other's —
a `BEGIN … END` mariadb accepts is a syntax error to postgres. So there is no
procedural body a uniform API could carry across even the two backends that HAVE
one, and the contract is **one statement per procedure, refused at deploy
everywhere**. One rule, one function (`procedural`), four backends that cannot
drift apart on what it means.

## What is NOT built, and why none of it is a tail

Each is a decision rather than an unfinished step, which is why this plan closes
with them standing:

- **A boxed value type** collapsing the per-kind `hole_*` methods into one needs
  a GENERIC METHOD (`fn hole<T>(self: Self, v: T)`) — not @PLN125 arc A as first
  recorded; arc A shipped associated types and they do not reach this. It was
  filed as @PLN137 and is now **DECLINED** (@C110): a generic method accepts every
  type by construction, which deletes the per-kind opt-in that makes this plan's
  own refusal auditable. The per-kind form was chosen first because it could be
  collapsed later; the evaluation found that collapsing it would cost the safety
  property, so it stays.
- **Specs on holes** are refused rather than delivered; if a target ever wants
  them, they have to reach the hole as data, not as a rendering.
- **A procedural body** on the two backends that could carry one — refused
  instead, because there is no such body a uniform API could carry across even
  those two (measured above).

## Bugs this surfaced, filed rather than fixed here

Both were store-lifetime / marshalling faults with clean workarounds, and both
were their own investigation rather than a patch alongside a language feature.
**Both are now closed:**

- **loft#771** — a value consumed where it is PRODUCED keeps no owner, in two
  carriers: a text field returned from a nullable struct, and a `T?`-returning
  call passed inline as an argument. Binding to a local first frees it.
- **loft#773** — a library function returning `text` answers `""` when used in
  place (a call argument, a format hole) through the prebuilt-cdylib bridge on
  `--interpret`; `--native` and `LOFT_NO_NATIVE_LIBS=1` are correct. Silent, and
  the default mode for every published library had it.
