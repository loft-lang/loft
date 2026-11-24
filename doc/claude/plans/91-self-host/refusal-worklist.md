# @PLN91 — the refusals, ranked: the worklist after parity

**Where this starts:** parity holds (parity-worklist.md): every program that agrees on its IR
agrees on its contract, state and reasons too, and since the agree merge an `agree` requires all
four.  What is left is the refusals: 730 programs the port refuses, and in them 3083 refused
definitions and 6103 beside a refusal.

**Measured at** loft30-gate 7201cfb57, try 635b29b75's verdicts (`verdicts.tsv`, `where.tsv`).
Two rankings, because the landscape is flat:
- **programs freed** — most refused programs meet several refusals, so no family frees more than
  five programs alone; clearing families in turn, 25 of them free 96 programs;
- **definitions freed** — the refused definitions a family alone blocks.  This is what moves the
  pinned `elab-functions` count, and where a few families dominate.

Each item names the port's refusal site, what Rust does there (the routine to port, under the
corresponding-routine ruling), the sub-kinds that make it up, and the cells to probe first.

| # | refusal family | programs met | definitions it alone blocks / meets | port site | owner |
|---|---|---|---|---|---|
| R1 | names across a used library: `a method call: <name>`, and a variant (`Fatal`) | 17 + 3 | 217 / 255, + 18 | el_ex_object.loft:882, :2974; el_ex_name.loft:463 | porter |
| R2 | a constant of a heap type, and every use of it (`a name that is not a variable: NUMS`) | 46 | 55 / 118 | el_definitions.loft:970 (`parse_constant`) | porter |
| R3 | a function returning a vector from this tail | 37 | 48 / 94 | el_ex_deliver.loft:443, :492 | porter |
| R4 | a type with arguments, keys, a limit or a member (a generic struct or enum) | 9 | 46 / 67 | el_types.loft:626 | porter |
| R5 | a `&` reference to something other than a scalar or a vector place | 25 | 41 / 70 | el_ex_vector.loft:1570 | porter |
| R6 | a vector match over this element type | 7 | 38 / 45 | el_ex_match.loft:3275 | porter |
| R7 | a function returning a record from something other than a literal | 35 | 27 / 52 | el_expressions.loft:784 | porter |
| R8 | a literal of something other than a declared struct | 32 | 32 / 84 | el_ex_into.loft:99, el_ex_object.loft:430 | porter |
| R9 | the definitions a refused construct never registers (the `beside numbers` pool) | — | 2339 beside | per site, below | tester |
| R10 | the 32-bit regime's lowering (`a \`#width 32\` file`) | 0 | — | el_definitions.loft:92 | tester — built |

## R1 — names across a used library

**What fails:** a name a used library declares, read from another source.  `lx.matches("a")` in
the lexer tests calls `fn matches(self: Lexer, …)` from `lib/lexer.loft`; `lib/parser.loft` reads
`Fatal`, a variant of `lib/logger.loft`'s enum.  The port's lookups key a name by source
(`el::def_nr`, `key(el.source, name)`), and the method lookup (`find_fn`) and the variant lookup
look in the reading source and the standard library only.
**Sub-kinds** (method names): `matches` 194, `identifier` 102, `test` 84, `offset` 78.
**Rust:** a method is the global `t_<len><Type>_<name>` and a variant its enum's; read where
`Data::def_nr` and the variant resolution find them across sources — confirm on the first probe.
**Probe:** tests/scripts/1092-lexer-revert-restores-the-cursor.loft (143 definitions),
1339-the-parser-is-not-charged-for-a-prelude-that-never-existed.loft (139), lib/parser.loft
(136), 1259-the-parser-library-accepts-the-language-it-documents.loft.
**Why first:** the largest definition count of any family, in three programs.

## R2 — a constant of a heap type

**What fails:** `NUMS: vector<integer> = [7, 8, 9];` at top level.  `parse_constant` refuses
it (`a constant of this kind: TyVector`), so every later use is `a name that is not a variable:
\`NUMS\` is not declared` (NUMS 12, NAMES 8, …).
**Rust:** the constant is a definition of type 8, its value built in the constant store and read
through `OpConstRef`.
**Also numbering:** a Rust-only constant was the second-largest first divergence in the beside-
numbers pool (434 shifted definitions in 16 programs: `GV`, `SEED`, `SQUARES`), so this frees
both the refusals and the definitions numbered after the constant.
**Probe:** a-placed-call-copies-an-answer-from-elsewhere.loft (NAMES, NUMS),
a-literal-table-is-built-from-its-constant.loft (SQUARES),
an-appended-call-result-arrives-from-every-exit.loft, 2240-a-constant-holds-its-declared-type.

## R3 — a vector returned from a tail

**Sub-kinds:** `null` 36, `if` 15, `block Append Vector` 8, `an owned local` 7.
**Rust:** `ref_return` delivers each kind of vector tail into the caller's buffer
(`@FR-O-Buffer`): a `null` tail as the empty vector, an `if` arm by arm.
**Probe:** Col-Concat a-concatenation-is-a-new-vector-and-neither-operand-changes-wherever-it-
stands (13), 1493-a-branch-arm-yielding-a-locals-field-delivers-a-copy (13),
2233-a-coalesce-tail-delivers-into-the-callers-buffer-a-lambdas-too (7).

## R4 — a generic struct or enum

**Sub-kinds:** `Tree` 9, `Bag` 9, `B` 9, `Wrap` 8 (a type written with arguments, `Tree<T>`).
**Rust:** the type template and its instances (`@FR-G-Mono` for types).
**Probe:** a-template-that-names-itself-at-its-own-variables-compiles-on-its-own (18),
2547-a-generic-enum-named-above-its-declaration-is-laid-out (9),
a-library-carries-generic-types-across-the-boundary (8).

## R5 — a `&` to a place that is no scalar or vector

**Sub-kinds:** `TyUnknown` 28 (a place pass 1 has not typed), `TyFunction` 19, `TyHash` 14,
`TyTuple` 13.
**Rust:** `@FR-B-Ref-Uniform`: a `&τ` of any type is a link to the place.
**Probe:** a-link-to-a-fn-value-is-a-link-like-any-other (36),
1836-a-link-to-a-nullable-place-writes-through (10),
a-whole-write-through-a-link-to-a-plain-parameter-rebinds-the-callees-name-only (9).

## R6 — a vector match over this element type

**Sub-kinds:** `TyInteger` 24 (a narrow integer element), `TyTuple` 10, `TyOptional` 5.
**Rust:** `parse_vector_match` over any element, its reads at the element's own width.
**Probe:** a-narrow-vector-element-keeps-its-width-in-every-operation (23),
a-match-streams-an-iterator-of-any-element (8),
a-pattern-capture-takes-the-type-its-pattern-matched (6).

## R7 — a record returned from a tail that is no literal

**Sub-kinds:** `if` 24, `call OpGetRecord` 7, `block match` 6, `block scalar_match` 3.
**Rust:** `ref_return` / `MaterializeView`: each exit answering a view copies into the buffer.
**Programs:** first in the freed-in-turn ranking — alone it frees five.
**Probe:** 2561-a-record-copied-out-of-a-view-is-absent-where-its-source-is (8),
a-tail-match-answer-materialised-into-a-work-ref-is-freed-once (4),
each-pass-frees-the-answer-it-copied (3).

## R8 — a literal of something other than a declared struct

**What fails:** a struct literal whose name resolves to no struct the port has declared — a
record declared below its use, a library's, an alias.
**Probe:** 947-feature-worked-examples (9), 03-crystal-incr (7),
2497-a-return-naming-a-record-declared-below-is-that-record (5).

## The rule: register, then refuse

**A refusal never drops a definition Rust registers.**  Wherever the port refuses a construct
that registers definitions in Rust — a lambda and its closure record, a vector type, a function
whose default it cannot lower, a constant — it registers them first, as Rust does (same name,
same kind, same place), and then refuses; nothing of the construct is elaborated.  A definition
dropped instead shifts the number of every definition after it, so they all differ in numbers
only and no correct port of them can show.  The rule binds every new refusal site.

**Measured:** the gate prints `definitions one side registers: N Rust only, M port only` and
writes them per program (`LOFT_ELAB_ONE_SIDED_FILE`); a try's result line names any Rust-only
definition new against its base (`NEW RUST-ONLY DEFINITION(S)`, `one_sided.tsv`).

**Order of work:** before an R-item starts, the tester aligns the numbering in the programs it
touches (R9), except where the R-item itself is the cause (R2's constants, R4's templates): then
the item's port shows as definitions won, not hidden behind shifted numbers.

## R9 — what a refused construct never registers (tester)

A construct the port refuses before it registers its definitions shifts the number of every
definition after it, so those definitions differ in numbers only (`beside numbers`).  The fix at
each site is to register what Rust registers, then refuse.  Done: the refused lambda
(7201cfb57: 277 definitions); an unnamed vector element type, a refused parameter default and an
expected empty literal (6711b88de: 84).  Next, by the shifts each causes:
- a vector of an element type the port refuses, which never registers its `vector<…>` (102);
- a named argument (86, one program);
- a vector of tuples with a member of an unsupported kind (52);
- the remaining one-side definitions: a user function (292), a generic template (280), a
  struct (121), a drop helper (90), a tuple record (84).
`np/shift.py` and `np/verify.py` (the tester's scratch tools) rank them from the saved IR.

## R10 — the 32-bit regime

Built (d067d87a8): a `#width 32` file is lowered as the Rust front end lowers it — W-Name,
W-Lit, W-Count, W-Op, W-Call (both sides), W-Store, `@FR-I-Narrow-U32`'s guard, and the
`@FR-I-Join` integer union in `if_expr`.  Every regime program with a Rust image agrees; the two
`@EXPECT_ERROR` cells have none.  Not carried: the regime under `LOFT_WIDTH=32` for the standard
library, and the `match` arms' join (`join_arm_into`'s union and `arm_fits_named_width`), which
no corpus program reaches yet.
