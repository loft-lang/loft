# 202 — The OCaml bar, for the compiler: what a loft-written front end needs, measured, and the steps that close it

## Status

**Open — filed 2026-10-24 (owner, @PLN202), measured, no implementation.**  Child of @PLN91 strand
1a (the compiler in loft).  Every row below was measured on main @ b36d0449f and re-measured
2026-10-25 on `157-native-4x` @ f32f908a2, on BOTH backends from a scratch directory; the probes
are in the plan directory's `probes/` (one per row, named by bar entry) and become cells under P0.
The re-measurement closed two steps without code — **P1** (`>>` already works on main) and **P6**
(not needed by 1a's AST rule) — and added two, **P8** (two diagnostics that name a cure which does
not compile) and **P9** (the documentation and the skills).  **P0, P2, P3, P4, P5a, P7, P8 and P9 are done**
(2026-10-25, with loft#2166, #2168 and #2170 fixed on the way and loft#2171 found); P5b stays
conditional on the parser's need (below), and P2b (the variant update) is the other open clause.
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
| **P3** | — | **done 2026-10-25:** the `match-scalar-hole` ADVICE at `Parser::parse_scalar_match` (`src/parser/control.rs`), on an integer, text or character subject with no `_` and no bare-binding arm, naming how many values the arms name and what the hole answers (null and `τ?` as an expression; nothing as a statement); `LOFT_NO_SCALAR_HOLE` opts out; DIAGNOSTICS.md row, `@F29` door, pinned in `tests/e1_code_set.rs`.  Tier: `advice`, as open question 3 argued — the null is `(M-Total)`'s answer and the type carries it.  Corpus: the two deliberate holes in `tests/scripts/10-match.loft` claim it; `tests/docs/29-match.loft` no longer says the compiler will not remind you; none in the stdlib or a library.  Cell: `matching/M-Total/a-scalar-match-with-no-catch-all-is-advised-at-the-match` | `src/parser/control.rs`, `src/keys.rs` | `make suite ARGS="@FR-M-Total --mode both"`; the corpus moved by the named sites only |
| **P4** | — | **done 2026-10-25:** `parse_slice_pattern` answers each arm's `SliceShape` (its minimum length, exact or open, element patterns total), and `slice_arms_cover_every_length` asks the set: an exact arm covers its length, a rest arm every length from its minimum, each with total element patterns and no guard, the last arm among them; then the last arm is the fallback, as a `_`'s is, so the join is not nullable.  `[]` with `[x, ..rest]` compiles; a rest arm alone, a length left out, a guarded rest arm and a variant element pattern stay refused, the message now naming the covering spelling.  `(M-Total)` gained its vector clause (Contract: strained); LOFT_CONTROL.md § Match rewritten.  Cells: `matching/M-Total` (two) | `src/parser/control.rs` | C8 as written, both backends; the matching chapter green |
| **P5a** | — | **done 2026-10-25** — `src/opt/tail_self.rs`, `(R-TailSelf)` in formal/rewrites.md: a self-call in tail position of a function whose parameters and locals are scalars (a scalar or void result) is a jump to the entry — the arguments into temporaries, the parameters from them, `continue` by the user loops between; `LOFT_NO_TAIL_SELF` / `LOFT_TRACE_TAIL_SELF`; every decline reported.  It runs on EVERY lane, `open_world` and coverage included (unlike `(R-InlineLeaf)`): a depth a program reaches is a property of the language, so `--interpret`, the `--native` semantics run (which ships the live tier and is therefore `open_world`), the shipped binary and `loft test` must agree on it — a live flip reaches the function at its next outside entry, as it reaches a `while` loop | `close_program`, after `leaf_inline` | two cells in loft-suite `rules/rewrites/R-TailSelf/` (every tail spelling and scalar type, 100 000 deep on two shapes; the limits), both backends; `scripts/introspect_diff.sh` on/off reaches four corpus files (the two cells, `08-functions.loft`'s `is_even`, an R-RangedCall cell), each green on both backends; `make rewrite-census` prints the new rule; `make falsify` receipt in the cell |
| **P5b** | M | **the LOCAL and the PARAMETER clauses built, the REFUSAL ruled 2026-10-25** — a local held in a store is released before the jump (the pass reads the scope pass's lowered `__ret_N = E; releases; return __ret_N` and the void body's `call; releases`, and copies the releases into the jump after the arguments are read); a parameter held in a store or a `&` parameter is passed through as the pointer the call shares — itself, a sibling parameter, a projection rooted at one — and rebound as a pointer.  **Owner's ruling (@C155): a tail self-call the frame cannot reuse is a compile error, never a call that stops at 10 000 frames** — the parser refuses it (pass two, `admit_frame`, the pass's own predicate) naming the variable and the cure: an argument that makes a value or names a local, a parameter the body rebinds (its store is the frame's, and the epilogue would free it under the temporary — found by the F-Rec cell), a `&` local, a heap result.  `r = f(…); r` keeps a call on purpose.  Measured with `loft --check`: of the corpus's 2876 files 16 were caught by the scalar clause's refusal and 6 remain under the parameter clause — all heap results, each cell's recursion re-spelled as a call (through a partner, or `z: T = f(…); return z;`); of the registry's 1465 files 22 (hex_terrain, script) were caught and none remain.  **Found on the way:** a recursive generic that hands back a record through `r = mx(…); r` or through a partner function answers the wrong parameter on one backend or both (the G-Mono 1881 cell's `mx`; the typed-return spelling is right) — a silent-wrong defect of the return delivery, measured on main and filed as loft#2179.  The census records three fewer rewrite admissions in hex_terrain (`th_subdiv` now loops: R-Range 218→214, R-RangedCall 16→15, R-GuardFree 14→13); its bench is flat on/off (hydrology 40.20 vs 40.15–40.18 ms per op), so the drop is blessed as the loop's static shape.  **Still open — the heap RESULT clause:** the result is delivered into the caller's hidden buffer (`___acc = f(…)` for text, `__retbuf` for a record), so the jump must deliver the final arm's value into that same buffer; the `(R-ValueRecord)` lowering decides the shape | the same pass + `definitions.rs` | the local clause: a cell 100 000 deep per local kind; the parameter clause: a cell per argument kind 100 000 deep; the refusal: a cell per shape with `@EXPECT_ERROR`, the golden message; all both backends, leak-checked, strict stores |
| **P6** | — | **closed 2026-10-25, not needed:** 1a's AST rule (above) keeps children in a `vector<Node>`, a keyed collection or an arena, never behind `reference<>` into a growable vector, and `a5e` (inline `vector<Node>` children) passes.  Binding `&` of a temporary would serve the shape the rule excludes | `Parser::is_amp_place`, `src/parser/operators.rs:1996` stays as it is | — |
| **P7** | — | **done 2026-10-25** for A5, C7, C8 and H1 (OCAML_BAR.md § Evaluation: A5 passes through `reference<>`, the by-value spelling refused by design; C7 by the advice; C8 and H1 as written); H6 stands until P5a | `doc/claude/bars/OCAML_BAR.md` | the bar's table and the cells agree |
| **P8** | — | **done 2026-10-25:** the bare-variant cure names the DECLARED enum (`Tree.Leaf`, not the instance `Tree<T#4>`); `Tree.Leaf` — a generic enum's template followed by `.` and one of its variants — is the qualified spelling, resolved by the TARGET (a typed local, a concrete parameter; `Tree::Leaf` likewise), and refused naming the typed-target cure where there is none (`tests/error_messages/cases/62_…`); a generic parameter's argument is refused too (`T` unconstrained).  Found on the way: a self-referencing generic enum that no generic function instantiates trips the two-pass contract on its own — loft#2171, filed, not fixed here.  Cell: `interfaces/G-Type` | `src/parser/objects.rs` (`parse_constant_value`, the qualifier) | the golden case; `make suite ARGS="@FR-G-Type --mode both"` |
| **P9** | — | **done 2026-10-25** — the Documentation section below is the list, every line checked; `make skills-review` remains the owner's read (every skill is listed from its 2026-10-16 watermark, not from this work) | — | `make docs-lint` adds nothing; the reference pages and `tests/docs/features/F12.loft` run on both backends |

## Phase ordering

P0 → P2 → P3 → P4 → P8 → P7 → P9, each a push of its own with its gate green on both backends.
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

## Documentation

What this plan updates outside itself; P9 is this list, and the plan closes when every line is
marked done.

- [x] LOFT_DATA.md § Structs — `..base` (`c27eb2462`); LOFT_CONTROL.md § Match — head/tail arms,
      the vector paragraph (`943ffa89b`); § `is` — a unit variant's `{` is the block's (`2da4321bc`).
- [x] DIAGNOSTICS.md — the `match-scalar-hole` row and `LOFT_NO_SCALAR_HOLE`; the pin in
      `tests/e1_code_set.rs` (`943ffa89b`).
- [x] formal/binding.md `(B-Ref-StoredRef)` — the assignment spelling, the inline refusal
      (`7e13076ba`); formal/matching.md `(M-Total)` — the vector clause, the advice (`943ffa89b`).
- [x] CHANGELOG.md — New: the record update, the complete slice match, the scalar-hole advice;
      Bug fixes: loft#2166, #2168, #2170.
- [x] tests/docs/29-match.loft — "the compiler will not remind you" rewritten, the page
      regenerated (`943ffa89b`).
- [x] doc/claude/bars/OCAML_BAR.md — A5, C7, C8, H1 re-measured (P7); H6 re-measured (P5a).
- [x] LOFT.md's index — names no literal spelling (checked 2026-10-25); nothing to add.
- [x] tests/docs reference pages — `08-struct.loft` § A record with some fields changed,
      `29-match.loft` § Slice arms that cover every length; pages regenerated with `make doc`.
- [x] the `loft-write` skill — `..base`, head/tail arms without `_`, the scalar-hole advice, a
      unit variant's `{` after `is`, `d.r = &p`; the error→fix rows; the snippets in
      `tests/reference/skill-*.loft` (`6c9764861`).  `make skills-review` still lists every skill
      from its 2026-10-16 watermark — the review itself is the owner's read.
- [x] STDLIB.md / INTERFACES.md — state nothing this plan lifted (grep 2026-10-25).
- [x] the feature catalogue — @F12 (struct records: the update) and @F29 (match: the complete
      slice match, the scalar-hole advice) edited; the shadow regenerated.

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
