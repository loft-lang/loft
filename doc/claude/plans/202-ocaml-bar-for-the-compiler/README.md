# 202 — The OCaml bar, for the compiler: what a loft-written front end needs, measured, and the steps that close it

## Status

**Open — filed 2026-10-24 (owner, @PLN202), measured, no implementation.**  Child of @PLN91 strand
1a (the compiler in loft).  Every row below was measured on main @ b36d0449f and re-measured
2026-10-25 on `157-native-4x` @ f32f908a2, on BOTH backends from a scratch directory; the probes
are in the plan directory's `probes/` (one per row, named by bar entry) and become cells under P0.
The re-measurement closed two steps without code — **P1** (`>>` already works on main) and **P6**
(not needed by 1a's AST rule) — and added one, **P8** (two diagnostics that name a cure which does
not compile).  The implementable arc is P0 → P2 → P3 → P4 → P8 → P7, with P5 in two clauses.
One defect fell out of the measurement and is filed: loft#2163 (open).

## Goal

Bring loft to the entries of [OCAML_BAR.md](../../bars/OCAML_BAR.md) that a compiler front end
written in loft uses — a lexer over a character cursor, a recursive-descent parser building a
recursive AST, rewrites that copy a node with one field changed, matches over tokens and node
kinds that must be total — and no further.  Each step is small enough to be validated on its
own, names its code position, and ends with the bar row re-measured.

## Effort + design

- **Effort:** S overall; P5 is M, its scalar clause (P5a) S.
- **Design:** ✓ for P2–P4 and P8 (each is a parser rule with a known site); ~ for P5 (an `opt`
  rewrite, conditions stated below).
- **Last touched:** 2026-10-25.

## What a compiler asks of the bar — and where loft is (measured 2026-10-24, re-measured 2026-10-25)

| capability | bar entry | compiler use | measured, both backends |
|---|---|---|---|
| generics over vectors, tuples, structs, user `Result` | A1–A4, A6, A7 | symbol tables, pass results | **PASS** as written |
| recursive enum through `reference<T>`, generic | A5 | a typed AST | **PASS** through `reference<Tree<T>>` — written `>>` or `> >`, both on main @ b36d0449f — with nodes bound to typed locals (`n1: Tree<integer> = Node {…}`).  The bar's BY-VALUE spelling (`left: Tree<T>`) is refused by design (*"Enum 'Tree<integer>' contains itself — use reference<Tree<integer>>"*), so the bar's FAIL row measures that spelling, not the capability; the refusal of the bare `Leaf` inside the nested literal names a cure that does not parse (P8) |
| recursive enum with `vector<Node>` children, built inline | (new, `a5e`) | the AST shape a parser builds in one expression | **PASS** — `Call { name: "f", args: [Leaf {…}, Call {…}] }` |
| a nested constructor through a `reference<T>` field, inline | A5b note | `Add { l: &Lit {…}, r: … }` | **refused**: `&` needs an addressable operand (`src/parser/operators.rs:1996`); the shape 1a's AST rule below excludes (P6 closed) |
| dispatch table of plain fn-refs in a vector or keyed collection | B1 (its `won't` half) | a Pratt parser's prefix/infix table | **PASS** — `hash<Rule[name]>` of `Rule { name, apply: fn(…) }`, as a literal or `+= […]` |
| capturing closures in a collection | B1, B2 | — | refused by design, @C116 |
| a local recursive fn | B4 | — | refused by design, @C63; a file-scope `fn` says it |
| nested patterns, literals in fields, rename, or-patterns, tuple subjects, guards | C1–C5, H7 | every pass over the IR | **PASS** |
| an enum hole named | (new, `c9`) | adding a Value variant must break every match | **PASS** — *"missing: Minus, RParen"* |
| a hole in a match over a SCALAR | C7 | `match c { 'a' => …, }` in a lexer | **silent** when the match is a statement (`match c { 'a' => …, 'b' => … }` skips `'z'`); at a non-null sink the hole already warns (*"a nullable `text?` is stored into the return value of the non-null type `text`"*) because the result type is `τ?` under `(M-Total)` |
| `[]` + `[x, ..rest]` read as total | C8 | token-list matchers | **refused**: *"a slice pattern can fail"* (`src/parser/control.rs:12214`) |
| copy a record with one field changed, one expression | H1 | every rewrite | **refused** (`Expect token ;`); `d = u; d.hp = 0;` works |
| a self-call in tail position runs to any depth | H6 | loops written as recursion | **stops at 10000 frames** (`src/limits.rs:152`); the same function written as a `while` over its reassigned parameters runs 100 000 deep on both backends (`probes/h6_tail_recursion.loft`'s loop twin) |
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
| **P0** | — | **done 2026-10-25:** ten cells in loft-suite @ bbac66998, under H-RefHome, H-NewRec, F-Rec, H-LongRef, L-Escape, L-FnRef, M-Exhaust (two), Slice-Value and B-Ref-Intro; the probes no rule names (an empty stub's default, partial application, a process exit code, `contains`, a local `fn`) stay probes, and the loft#2163 shape waits for its fix.  The scalar hole, the head/tail slice, the record update and the tail call are P3, P4, P2 and P5a's cells.  The AST rule above is in @PLN91 1a | `tests/loft-suite.pin` | `make suite ARGS="@FR-… --mode both"` green |
| **P1** | — | **closed 2026-10-25, already true:** `>>` closing two type-argument lists in a variant field passes on main @ b36d0449f on both backends (`probes/a5_typed.loft` with `>>`); the row that asked for it was mis-measured.  A5's bar row is corrected by P7 | — | — |
| **P2** | — | **done 2026-10-25:** `Unit { hp: 0, ..base }` — `..base` last, lowered to `dest = base;` then `dest.f = e;` per named field THROUGH THE ASSIGNMENT PATH (`Parser::parse_object_update`, `src/parser/objects.rs`), so the IR and bytecode are the two statements' own (measured: identical modulo the file-name constant), a text or collection field releases what the copy brought (`OpSetTextReplace`, `OpClearVector`), a `const` field is refused, and a `&` into a `reference<T>` field stores the pointer (loft#2166, fixed first: the two-statement spelling was broken there).  Pass 1 records the update literals, pass 2 takes the road before any work reference is minted; a base that is the destination binds nothing (as `d = d;` emits nothing).  Refused: a variant literal (its tag would need checking against the base's — the design question left for P2b), an absent base, a base of another type, `..base` not last.  Cells: loft-suite `binding/B-Copy` (two).  Docs: LOFT_DATA.md § Structs | `src/parser/objects.rs` | `make suite ARGS="@FR-B-Copy --mode both"`; the matrix on both backends |
| **P3** | S | a match over a scalar subject (integer, text, character) with no catch-all arm reports the hole at the match, with a DIAGNOSTICS.md code and its `LOFT_NO_*` opt-out; tier per open question 3 | `Parser::parse_scalar_match`, `src/parser/control.rs:11430`, right after `bool_exhaustive` — NOT the enum site at `:7160`, which never sees a scalar | an `@EXPECT_WARNING` (or advice) cell; the corpus moves by exactly the sites a grep names today — 14: `tests/scripts/10-match.loft` ×3, `tests/docs/29-match.loft` ×1 (a reference page: its text moves too), `tests/…` ×2, `../loft-suite` ×8 (five in `attic/`) — and none in `default/` or any `loft-libs-*` library, so no library CI changes with either tier |
| **P4** | S | `[]` + `[x, ..rest]` is total.  `parse_slice_pattern` already knows each arm's shape (`head.len()`, `tail_total`, `has_rest`, `rest_min`, `elem_conds`, `multi_alt`): an exact arm covers one length, a rest arm every length from its minimum, and an arm counts only with total element patterns (bare names, `_`, a type capture) and no guard.  When the arms cover every length the LAST arm becomes the fallback, as `bool_exhaustive` does, so the join is not nullable | `src/parser/control.rs:12246` returns the shape; the gate at `:12214` reads it; `(M-Total)` gains the length-coverage clause (Contract: strained) | C8 as written, both backends; `make falsify` with the clause reverted brings the refusal back; a cell per non-total neighbour (a guard, a variant element, `[x, ..rest]` alone, `[]` + `[x, y, ..rest]`) still refused |
| **P5a** | S | a SELF-call in tail position becomes a jump to the function entry — an `opt` pass, so the interpreter, the Rust writer and the C writer get it at once (all three already read `Value::Loop` / `Value::Continue`: `src/state/codegen.rs`, `src/generation/emit.rs`, `src/c/body.rs`).  First clause: SCALAR parameters, a scalar or void result, no store-owning local — so nothing is released before the jump and the drop gate is vacuous; same arity; arguments evaluated into temporaries before the rebind; every decline reported (`rewrite_census::declined`); `LOFT_NO_TAIL_SELF` switches it off | `src/compile.rs::close_program`, beside `leaf_inline::rewrite_program`, which is the pattern (swap the body out, rewrite, `scopes::compute_function_intervals` + `assign_function_slots`, `rewrite_census::fired`) | H6 at 100000 both backends; the switch A/B byte-identical elsewhere (`scripts/introspect_diff.sh`); `make rewrite-census` row; `make interp-gap` not slower |
| **P5b** | M | the HEAP clause, only when 1a's parser recurses on a record or vector parameter: a call hands a reference over ([formal/calls.md](../../formal/calls.md)) while an assignment copies `(B-Copy)`, and a heap-parameter rebind carries its entry witness (`@FR-O-Detach`), so the rebind must use the CALL's binding, not the assignment's; the frame's stores are released before the jump; no `&` parameter rebound through itself | the same pass | the drop gate (`LOFT_STRICT_STORES=1`, the leak scan) over a record, a vector and a `&` parameter; the P5a gates again |
| **P6** | — | **closed 2026-10-25, not needed:** 1a's AST rule (above) keeps children in a `vector<Node>`, a keyed collection or an arena, never behind `reference<>` into a growable vector, and `a5e` (inline `vector<Node>` children) passes.  Binding `&` of a temporary would serve the shape the rule excludes | `Parser::is_amp_place`, `src/parser/operators.rs:1996` stays as it is | — |
| **P7** | XS | re-measure A5, C7, C8, H1, H6 and write their dispositions into OCAML_BAR.md (`shipped`, this plan); correct A5's row — the capability passes through `reference<>`, the by-value spelling is refused by design | `doc/claude/bars/OCAML_BAR.md` | the bar's table and the cells agree |
| **P8** | XS | the bare-variant refusal names a cure that compiles.  Today `src/parser/objects.rs:1649` prints the INSTANCE name — *"qualify it as 'Tree<T#4>.Leaf'"* — an internal name whose `<` / `>` read as chained comparisons in expression position, where the DECLARED name is meant (`Tree.Leaf`, the spelling that works for a plain enum: `C.A`).  And `Tree.Leaf` on a GENERIC enum answers *"Unknown type null — did you mean 'JNull'?"*: it takes its arguments from the target, or is refused naming the typed-target cure (`l: Tree<integer> = Leaf`) | `src/parser/objects.rs:1649` (the `declared` name is already computed there for the import cure); the resolution of `Enum.Variant` for a generic enum (@PLN22's enum-scoped variants) | an `@EXPECT_ERROR` cell per message, and the cure each message names compiles on both backends |

## Phase ordering

P0 → P2 → P3 → P4 → P8 → P7, each a push of its own with its gate green on both backends.
P5a after P4 if 1a's parser wants it (a recursive-descent parser's depth is bounded by the
program it reads; the loop-as-recursion habit is what P5 serves); P5b only when a recursion over
a heap parameter asks for it.

## Open questions for the owner

1. **P5: the rewrite or the limit.**  Raising `MAX_CALL_DEPTH` is not the one-line alternative
   it reads as: the interpreter's call stack is a `Vec` with no structural bound, but the native
   binary sizes its 512 MiB virtual stack for 10 000 frames at a ~50 KiB budget
   (`src/codegen_runtime.rs:6698`), so a 10× raise re-derives that budget — and still only moves
   the cliff.  Recommendation: the rewrite, P5a first.
2. **P2's spelling.**  `..base` last in the literal, mirroring `[x, ..rest]` in patterns.
3. **P3's tier.**  Proposed as `warning` (gates a library's CI under `LOFT_DENY_WARNINGS`), on
   the ground that a silent null is a wrong result.  Measured: the match's result type is already
   `τ?` under `(M-Total)`, and a non-null sink warns today on both backends — so the type DOES
   advertise the null, and DIAGNOSTICS.md's rule (a diagnostic gates iff ignoring it can produce a
   wrong result the type does not advertise) reads `advice`.  The one silent case is a match used
   as a statement — the lexer's — which is the same silence as an `if` without `else`.
   Recommendation: `advice`; `warning` with the opt-out if the statement case must gate, in which
   case DIAGNOSTICS.md writes the exception down.

## Cross-arc dependencies

- **@PLN91** — the parent; strand 1a consumes every row above.
- **@PLN179** — findings 002 (tokenising), 015 (no set of scalars), 017 (closed: `contains`).
- **@PLN186** (patterns) and **@PLN165** (generics) — the arcs that made C1–C5 and A1–A6 pass.
- **@PLN22** (namespaces) — the enum-scoped `Enum.Variant` spelling P8 repairs for a generic enum.
- **loft#2163** — the reference-into-a-grown-vector disagreement.

## See also

- [OCAML_BAR.md](../../bars/OCAML_BAR.md) — the ruler; its § Evaluation is what P7 rewrites.
- [formal/matching.md](../../formal/matching.md) `(M-Total)` — the rule P3 reports against and P4 extends.
- [formal/heap.md](../../formal/heap.md) (H-Removed) — why `&v[i]` goes stale.
- [DIAGNOSTICS.md](../../DIAGNOSTICS.md) — the two tiers and the opt-out convention P3 follows.
- [NATIVE_SWITCHES.md](../../NATIVE_SWITCHES.md) / [BOTH_BACKEND_SWITCHES.md](../../BOTH_BACKEND_SWITCHES.md) — where P5's switch is documented.
