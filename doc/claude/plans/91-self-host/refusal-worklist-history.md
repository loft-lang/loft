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

