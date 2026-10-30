<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 strand 1b — reading the ports beside their homes: three fixes, designed

A child document of [1b-interpreter.md](1b-interpreter.md).  Status: **designs, for the owner's
evaluation; nothing built.**

## Why (owner, 2026-10-30)

*"Does your current loft side resemble the rustc side?  Is it clear to read versus the rustc
code?"* — and then *"we are evaluating, so create designs for the fixes."*  The loft ports of
B-1/B-2 (`selfhost/src/own.loft` 1754 lines, `binds.loft` 959, `lower.loft` 2644) follow their
Rust homes function for function: each function keeps its Rust name and cites its home
(`file:line`), the arms of `generate_set` keep their order and labels, and the bind trace checks
the correspondence row by row.  They read worse than their homes in three places, measured
below.  Each design starts from the rule or decision that already speaks, as
[1b-reference.md](1b-reference.md) orders every finding.

## R-1 — the oracle's lattice and the "none" answers as loft types

**Measured.**  `own.loft` encodes `Own` as one integer — `Borrowed{b}` = b, `Join{b}` =
65536 + b, Owned −1, Unknown −2, `NOT_YET` 65534 — and every "no variable" answer of
`own.loft` and `binds.loft` is −1 or 65535.  One `walk(mode)` stands in for the several Rust
traversals `any_node`/`for_each_child` callers write with closures.  Nothing in the language
forced either: loft has payload enums and `integer?`.

**Rule / decision.**  None is touched: the representation is the port's own choice, and goal
constraint 5 asks for the reference's decisions, not its spelling.  The binding constraint is
the comparison: the `ownership` and `binds` layers must stay EQUAL.

**Design.**
- `enum Own { Owned, Unknown, NotYet, Borrowed { base: integer }, Join { base: integer } }` with
  `base_of`, `is_join`, `join` as functions over it — the Rust `Own`'s variants, `NOT_YET` a
  variant rather than a reserved base.  `fmt_own`/`render_own` match on it.
- Every fact answering `Option<u16>` answers `integer?` (null for `None`): `entry_witness`,
  `owner_witness`, `displaced_buffer_witness`, `projection_container_var`,
  `record_copy_source`, `callee_of`, `heap_def_nr`.  Callers test `!= null` where they test
  `>= 0` today.  `u16::MAX` stays a value where the Rust keeps it as one (`base == u16::MAX`,
  a join with no single base).
- `walk(mode)` splits into one function per Rust traversal it serves, each named after it.

**Verification.**  A pure refactor: the `ownership` layer on the kernels, the 60-program
sample and every `tests/scripts` program the front end accepts, the `binds` layer on the
kernels and the sample, and the census — all EQUAL / unchanged before and after.  The
interpreter's speed on the kernels is measured and reported (an enum is a record in loft; the
integer was free), never a gate.

**Cost.**  Mechanical: about 300 changed lines, in one commit per file.

## R-2 — the `?? panic(…)` discharges: mostly one representation choice

**Measured.**  258 discharges across the three files.  By the index they discharge:

| shape | own | binds | lower | what it is |
|---|---:|---:|---:|---|
| a constant index (`inner[0]`, `args[0]`) | 50 | 16 | 110 | almost all the IR schema's BOX-OF-ONE: `ir.loft` holds a single child as `vector<Node>` of one (`NdSet { inner }`, `NdBlock { block }`), because loft has no owned single-child field and `reference<T>` would triple the width (the schema's own comment) |
| a variable index | 25 | 16 | 40 | a definition, variable or type by number — the image's tables, present by construction |
| `len(…) - 1` | 3 | 1 | 5 | the last operator of a block, after a length test |

**Rule / decision.**  `(N-Index)`: `v[i]` types `τ?`, and `(N-Domain)` already elides the `?`
when the index is PROVABLY in range — a constant, a range, a guard.  `@C89` declined Rust's
mitigation layer (`unwrap`, `if let`) — the layer TUPLE variants force; it did not decline sugar
inside the `τ?`-with-`??` family, which already has `x?` (`(N-Default)`, @PLN116): *"`??` = the
default YOU give, `?` = the default the TYPE gives."*  The owner (2026-10-30): *"I am not against
syntactic sugar here, the ? operator already returns the type default instead of constantly
writing ?? 0."*

**Design, in four parts — the sugar first, then the shapes that remain:**
- **`e!` — the discharge with no default: present, or a halt that names it.**  The family's
  third member: `??` the default you give, `?` the type's default, `!` none — it must be there.
  - *Rule* (types.md, beside `(N-Default)`): `(N-Present)  Γ ⊢ e ⇒ τ? ⟹ Γ ⊢ e! ⇒ τ`, with
    `e!  ≡  e ?? panic("<e> is null")`, the message the parser writes from the source text and
    the panic carrying the position, as every panic does.  No new runtime: it IS the idiom the
    ports spell by hand, so a production run that logs a panic and continues continues as `??
    panic` does today.  No `has_default` side condition, so it reaches what `?` cannot: a
    reference (`(D-NoRef)`) and a bare enum (`(D-NoEnumF)`) — the ports' `Node`, `Block` and
    `Definition`, which is why they spell `?? panic` and not `?`.
  - *Where `?` stays right*: absence with a sensible answer (a missing count is 0).  Where it
    is wrong: an image invariant, a table read by its own index — a silent default there turns
    a corrupt image into a plausible wrong answer.  `e!` is for exactly that.
  - *Spelling*: postfix `!`, parsed where `?` is (`operators.rs`, the postfix loop at
    `@PLN116`'s `x?`), so `v[i]!.f` chains as `x?.f` does.  The lexer's greedy `!=` keeps
    `x != y` a comparison (`x!=y` too, as today); a discharge is never an assignment target, so
    `e! = …` is no form.  Kotlin spells it `!!`, Swift `!`; one character matches `?`.
  - *Diagnostics*: on a statically non-null operand, the advice that `!x` on one already gets
    (DESIGN_DECISIONS_VALUES: *"`!x` on a statically non-null operand warns"*) — `e!` there is
    a no-op that says the author expected a null.
  - *Decision*: a new `@C` entry in DESIGN_DECISIONS_VALUES.md beside @PLN116, the owner's to
    ratify; the formal rule, LOFT.md, STDLIB.md's discharge table, the formatter and the LSP
    take it in one change, both backends and the C, with suite cells under the new rule.
  - *Effect on the ports*: 258 discharges become `!` (or an accessor's one `!`); each message
    the ports wrote by hand that says more than "is null" stays as `?? panic(…)`.
- **What `!` leaves** — the three shapes below still apply, now as structure rather than noise:
- **The box-of-one (176 sites): one accessor per schema field, in `ir.loft`** (with `!`, a
  one-liner each, or simply `inner[0]!` at the site).  `fn one(ns:
  vector<Node>) -> Node` (and `block_of`, `type_of` for the other boxes), the one place the
  image's invariant "a box holds exactly one" is discharged, with one message naming the box.
  The ports read `one(inner)` where they read `inner[0] ?? panic("a set has its value")`.  The
  representation itself is a language question — an owned single-child field — recorded in the
  friction register (X24) with this count, not decided here.
- **The table reads (81): the accessors the ports already half-have.**  `binds::def`,
  `fvar`, `fn_of` exist; every port reads definitions, variables and types through them, so the
  discharge is written once per table.
- **`len(…) - 1` after a length test (9): an `(N-Domain)` elision the compiler may be missing.**
  A cell per shape (`if len(v) > 0 { v[len(v) - 1] }`, the same with the length in a local) on
  both backends: if the rule's guard elision does not reach it, that is a front-end gap against
  `(N-Domain)`, filed and fixed as one, and the discharges go when it is.

**Verification.**  For `e!`: suite cells under `(N-Present)` — present (the value), absent
(the halt, its message naming the expression and its line), chained (`v[i]!.f`), a reference
and a bare enum (what `?` refuses), a statically non-null operand (the advice) — each on the
interpreter, `--native` and the C.  For the ports: the same receipt as R-1 (every comparison
layer unchanged); the count of `?? panic` in the three files is reported before and after.

## R-3 — a write into a copy: the bug the port met first

**Measured.**  The first bind-layer difference was a lost write: `v = fvar(b, d, var)` (a call
returning a projection of its argument) and then `v.stack_allocated = true`, which reached the
copy and not the image — every later set of the variable read as a first one.  The
`lost-write` lint (`@C86`, loft#1840; DIAGNOSTICS.md) was quiet, because it judges the copy
WHOLE: other fields of `v` are read after the write, so the copy reads as used.

**Rule / decision.**  `@C86`: a whole-value bind copies; a struct-typed PROJECTION (`a = vv[0]`)
is a view.  First step: read what `(B-Copy)` / the return rules say for a call that RETURNS a
projection of its parameter (`fn f(b) -> R { b.xs[i] ?? panic(…) }`, then `v = f(b); v.x = 1`):
is the bind a copy (the write is lost) or a view (the write lands)?  The two backends and the C
are measured on that cell.  If the rule says view and the run copies, that is a deviation, fixed
at its home and filed if `main` has it (silent-wrong).  If the rule says copy, the lint is the
cure.

**Design (the rule saying copy).**  `lost-write` judges each FIELD of a copy, not the copy as a
whole: a field written in a local that holds a copy of a place (a projection, or a call's
returned projection) and never read afterwards — neither the field, nor the whole value passed
on, written back or returned — is reported, with the cure the lint already names (`&` for a
write-through, or read the copy back).  It gates: ignoring it loses a write (DIAGNOSTICS.md §
Diagnostic tiers).

**Verification.**  The cells: the port's own shape (reported), a field written and read back
(quiet), a copy written then passed whole to a call (quiet), a write through `&` (quiet), each
on both backends; the lint's false positives measured over `tests/scripts`, the suite and the
libraries before it gates, as every new warning is.

## Order

R-1 and R-2's accessors are refactors of the ports and can go together, after B-1 and E-3 (the
receipt is the comparison).  R-2's elision cells and R-3's rule cell come first in their
designs, because each can turn into a defect on `main` — then they follow the bug policy.  The
two language questions (an owned single-child field; the per-field lint) are the owner's.
