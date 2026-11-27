# @PLN91 — the refusal worklist: history

Past measurements of [refusal-worklist.md](refusal-worklist.md), newest first.

## 2026-11-24: the ranking at 635b29b75

730 programs refused, 3083 refused definitions and 6103 beside a refusal.

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

## 2026-11-24 → 11-26: R1 and R2, cleared by 075d704de

At try dc356f49d R1 has 2 programs and 11 definitions left (`a method call`), R2 one
definition.  Their sections as written at 635b29b75:

### R1 — names across a used library

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

### R2 — a constant of a heap type

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


## The ranking at try dc356f49d (porter tip 075d704de, 2026-11-26)

**Measured at** try dc356f49d (porter tip 075d704de, 2026-11-26): 1634 of 2269 agree, 634
refused, 0 differ; 2396 refused definitions and 4538 beside a refusal (1453 of them numbers
only).  Data: `kr-tools/try-dc356f49d/{verdicts,where}.tsv`; re-rank a later try with
`kr-tools/family.py TRY PATTERN…` (a topic's programs admitted, and those one refusal short).

**The landscape is flat.**  184 refused programs meet one family, 185 two, 265 three or more;
361 families, 106 of them met by one program.  Clearing families greedily admits 4–5 programs
per family at first, 51 in the first 15, 69 in the first 20.  So the order below is by TOPIC
(the families one port clears together), each measured as "admitted once the topic is ported /
one refusal short of it":

| # | topic | admitted / one short | programs meeting | the families in it (programs) | port site |
|---|---|---|---|---|---|
| T1 | a record answered from something other than a literal (was R7) | 10 / 5 | 29 | a function returning a record from something other than a literal (21: `if` 11 defs, `call OpGetRecord` 7, `block match` 5); a `return` of a record that is not a literal (`other` 7, `if` 4, `block EnumUnitLit` 4); a value block answering a record | el_expressions.loft:827, el_ex_null.loft:3261 |
| T2 | `par` workers and loops of a non-scalar | 7 / 1 | 11 | a `par` worker answering TyOptional (10 defs), TyVector, TyEnum; a `par` loop over TyTuple, TyOptional | el_par.loft:99, :273 |
| T3 | tuples | 28 / 30 | 119 | 93 refusal kinds: a member of a boxed tuple that is no plain scalar, text or record (18); a tuple member of this subject: TyTuple (18); a vector of tuples with a member of this kind (16); a tuple with a text member that is no literal or variable (15); a boxed tuple answered by something other than a tuple literal (24 programs over 7 tails) | el_ex_tuple.loft:138, :526, el_expressions.loft:341 |
| T4 | links: `&` and `reference<…>` (was R5) | 19 / 19 | 81 | a `&` reference to something other than a scalar or a vector place (25); a `&` argument that is not a local (12); a link re-pointed at a place of another type | el_ex_vector.loft:1599 |
| T5 | lambdas and captures | 18 / 12 | 85 | a lambda that captures a local of its function (17); a lambda answering a record or a collection beside a shared capture (9); a short lambda with no `fn(…)` its position expects | — |
| T6 | generics (was R4) | 21 / 13 | 70 | a type with arguments, keys, a limit or a member (18: `Tree`, `Bag`, `B` 9 defs each, `Wrap` 8); a generic instance at text whose body holds an answer other than a parameter (12 hidden); an operator on a type variable its bounds do not declare | el_types.loft:685 |
| T7 | a vector answered from a tail (was R3) | 7 / 6 | 43 | a function returning a vector from this tail (25: `null` 18 defs, `block Append Vector` 8, `an owned local` 8, `block block` 6); a `return` with a value of this type (22); a function returning a keyed collection (8) | el_ex_deliver.loft:606, :657, :712 |
| T8 | MEASURED: no family of its own, a second refusal another one causes (§ T8) | — | 57 | a call that is not of a function by its bare name: a local of type TyUnknown (80 defs); a call returning a heap value: TyUnknown (49 defs) | el_ex_object.loft:1848, el_ex_call.loft:1990 |
| T9 | record literals (was R8) | 3 / 16 | 83 | a literal of something other than a declared struct (25 programs, 63 defs); a struct field of this type (26); a field the struct does not declare (18); a value struct or a struct template (16) | el_ex_into.loft:101, el_ex_object.loft:432 |
| — | a vector match over this element type (was R6) | — | 7 | 43 defs: a narrow integer element, TyTuple, TyOptional | el_ex_match.loft:1412 |
| R9 | the definitions a refused construct never registers (tester) | — | 157 | 1689 Rust-only and 108 port-only definitions; 1453 beside by numbers only (2339 at 635b29b75) | per site, below |
| R10 | the 32-bit regime's lowering | — | 0 | built (d067d87a8) | el_definitions.loft:92 |

**Recommended order:** T1 and T2 first: the most programs admitted per refusal kind, and T1 is
the porter's current topic.  Then T3, tuples, as ONE topic: by far the most programs met (119)
and the most admitted (28, and 30 more one refusal short), but 93 kinds, so it is cut into its
families and each try measured.  T8 is not a lever: its refusals follow from others (§ T8), so it leaves the ranking.
