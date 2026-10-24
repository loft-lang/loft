# 202 — The OCaml bar, for the compiler: what a loft-written front end needs, measured, and the steps that close it

## Status

**Open — filed 2026-10-24 (owner, @PLN202), measured, no implementation.**  Child of @PLN91 strand
1a (the compiler in loft).  Every row below was re-measured on main @ b36d0449f on BOTH
backends from a scratch directory; the probes are in `probes/` beside this file (one per row, named by bar entry) and become cells under P0.
One defect fell out of the measurement and is filed: loft#2163.

## Goal

Bring loft to the entries of [OCAML_BAR.md](../../bars/OCAML_BAR.md) that a compiler front end
written in loft uses — a lexer over a character cursor, a recursive-descent parser building a
recursive AST, rewrites that copy a node with one field changed, matches over tokens and node
kinds that must be total — and no further.  Each step is small enough to be validated on its
own, names its code position, and ends with the bar row re-measured.

## Effort + design

- **Effort:** S overall; the one M step (P5) is optional for 1a.
- **Design:** ✓ for P1–P4 (each is a parser rule with a known site); ~ for P5 (an `opt` rewrite,
  conditions to state); P6 conditional on 1a's AST shape.
- **Last touched:** 2026-10-24.

## What a compiler asks of the bar — and where loft is (measured 2026-10-24)

| capability | bar entry | compiler use | measured, both backends |
|---|---|---|---|
| generics over vectors, tuples, structs, user `Result` | A1–A4, A6, A7 | symbol tables, pass results | **PASS** as written |
| recursive enum through `reference<T>`, generic | A5 | a typed AST | **PASS** written `> >`, nodes bound to typed locals (`n1: Tree<integer> = Node {…}`); the bar's row says FAIL and is stale |
| recursive enum with `vector<Node>` children, built inline | (new, `a5e`) | the AST shape a parser builds in one expression | **PASS** — `Call { name: "f", args: [Leaf {…}, Call {…}] }` |
| a nested constructor through a `reference<T>` field, inline | A5b note | `Add { l: &Lit {…}, r: … }` | **refused**: `&` needs an addressable operand (`src/parser/operators.rs:1996`) |
| dispatch table of plain fn-refs in a vector or keyed collection | B1 (its `won't` half) | a Pratt parser's prefix/infix table | **PASS** — `hash<Rule[name]>` of `Rule { name, apply: fn(…) }`, as a literal or `+= […]` |
| capturing closures in a collection | B1, B2 | — | refused by design, @C116 |
| a local recursive fn | B4 | — | refused by design, @C63; a file-scope `fn` says it |
| nested patterns, literals in fields, rename, or-patterns, tuple subjects, guards | C1–C5, H7 | every pass over the IR | **PASS** |
| an enum hole named | (new, `c9`) | adding a Value variant must break every match | **PASS** — *"missing: Minus, RParen"* |
| a hole in a match over a SCALAR | C7 | `match c { 'a' => …, }` in a lexer | **silent null**, no warning, both backends |
| `[]` + `[x, ..rest]` read as total | C8 | token-list matchers | **refused**: *"a slice pattern can fail"* (`src/parser/control.rs:12214`) |
| copy a record with one field changed, one expression | H1 | every rewrite | **refused** (`Expect token ;`); `d = u; d.hp = 0;` works |
| a self-call in tail position runs to any depth | H6 | loops written as recursion | **stops at 10000 frames** (`src/limits.rs:152`) |
| a character-cursor lexer: `src[i]`, char compares, `src[a..b]` | (new, `l2`) | the lexer | **PASS** |
| membership: `v.contains(x)` | @PLN179 017 | keyword sets | **PASS**; there is no `in` and no set of scalars (015) |
| `x ?? exit(n)`, `h = half(n)?` | @PLN179 019, H8 | expect-or-fail in a parser | **PASS** |
| an empty-body stub answers the type's default | D2 | — | by design (@F19 stubs); a compiler writes no stubs |
| associated type in a generic's own signature, partial application, sharing measured | A8, H10, E | — | not needed by 1a |

**The AST representation this fixes for 1a** (from loft#2163): a node holds its children
in a `vector<Node>` or in a keyed collection whose records do not move (`hash<Node[id]>` with
`reference<Node>?` children builds a 2000-deep chain in 0.05 s), or as an index into an arena.
Never `&v[i]` into a vector that later grows: the interpreter reads null and `--native` reads
the moved record's old bytes.

## Sub-arcs

| step | size | what | code position | gate (what goes red on its own) |
|---|---|---|---|---|
| **P0** | XS | the 22 probes graduate to cells under their rules (`../loft-suite`; `tests/scripts` for what this repo names); the AST rule above goes into @PLN91 1a | — | `make suite ARGS="@FR-… --mode both"` green; a cell that cannot fail is dropped |
| **P1** | XS | `>>` closes two type-argument lists in a variant field; `> >` keeps working | `src/lexer.rs:574` (the token table) and the type-argument reader that already peeks `>>` at `src/parser/definitions.rs:5533` | A5 as the bar writes it, both backends; every existing `> >` cell unchanged |
| **P2** | S | `Unit { hp: 0, ..base }` — `..base` as the last entry of a struct literal, lowered to the copy-then-set IR | `Parser::parse_object_inner`, `src/parser/objects.rs:6114`; cite `(B-Copy)` | `loft introspect` byte-identical to the two-statement spelling over a corpus of shapes (nested fields, a nullable field, a variant); both backends; H1 cell |
| **P3** | S | a match over a scalar subject with no catch-all arm warns, naming the hole (`warning` tier: a silent null is a wrong result) with its `LOFT_NO_*` opt-out and a DIAGNOSTICS.md row | the exhaustiveness site `src/parser/control.rs:7160`, extended from enum subjects to integer, text and character subjects | an `@EXPECT_WARNING` cell; the corpus warning count moves only by the sites the new warning names |
| **P4** | S | `[]` + `[x, ..rest]` is total: an arm covering length 0 and an arm with one fixed element and a rest cover every length | `src/parser/control.rs:12214` | C8 as written, both backends; `make falsify` with the totality rule reverted brings the refusal back |
| **P5** | M | a SELF-call in tail position becomes a jump to the function entry — an `opt` pass (`src/opt/`), so the interpreter, the Rust writer and the C writer get it at once; conditions: same arity, arguments evaluated into temporaries before the rebind, no `&` parameter rebound through itself; `LOFT_NO_TAIL_SELF` switches it off | `src/opt/` (new pass beside `leaf_inline`), the call lowering it replaces | H6 at 100000 both backends; the switch A/B byte-identical elsewhere (`scripts/introspect_diff.sh`); `make rewrite-census` row; the drop gate (locals released before the jump); `make interp-gap` not slower |
| **P6** | M | `&` of a temporary inside a constructor binds a hidden local with the constructed value's lifetime — ONLY if 1a's AST keeps `reference<>` children; else closed as not needed | `Parser::is_amp_place`, `src/parser/operators.rs:1996` | A5c probe both backends; the ownership drop gate |
| **P7** | XS | re-measure A5, C7, C8, H1, H6 and write their dispositions into OCAML_BAR.md (`shipped`, this plan); correct A5's stale row | `doc/claude/bars/OCAML_BAR.md` | the bar's table and the cells agree |

## Phase ordering

P0 → P1 → P2 → P3 → P4 → P7, each a push of its own with its gate green on both backends.
P5 after P4 if 1a's parser wants it (a recursive-descent parser's depth is bounded by the
program it reads; the loop-as-recursion habit is what P5 serves).  P6 only when 1a's AST
shape asks for it.

## Open questions for the owner

1. **P5: the rewrite or the limit.**  Raising `MAX_CALL_DEPTH` is a one-line change that
   moves the cliff; the rewrite removes it for the one shape that matters and serves the C
   backend too.  Recommendation: the rewrite, gated as above.
2. **P2's spelling.**  `..base` last in the literal, mirroring `[x, ..rest]` in patterns.
3. **P3's tier.**  `warning` (gates a library's CI under `LOFT_DENY_WARNINGS`), because a
   silent null is a wrong result; the opt-out keeps a deliberate hole quiet.

## Cross-arc dependencies

- **@PLN91** — the parent; strand 1a consumes every row above.
- **@PLN179** — findings 002 (tokenising), 015 (no set of scalars), 017 (closed: `contains`).
- **@PLN186** (patterns) and **@PLN165** (generics) — the arcs that made C1–C5 and A1–A6 pass.
- **loft#2163** — the reference-into-a-grown-vector disagreement.

## See also

- [OCAML_BAR.md](../../bars/OCAML_BAR.md) — the ruler; its § Evaluation is what P7 rewrites.
- [formal/heap.md](../../formal/heap.md) (H-Removed) — why `&v[i]` goes stale.
- [DIAGNOSTICS.md](../../DIAGNOSTICS.md) — the two tiers and the opt-out convention P3 follows.
- [NATIVE_SWITCHES.md](../../NATIVE_SWITCHES.md) / [BOTH_BACKEND_SWITCHES.md](../../BOTH_BACKEND_SWITCHES.md) — where P5's switch is documented.
