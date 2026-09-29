// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# @PLN35 — Match PEG Patterns · Implementation Plan (step-by-step, verifiable)

**Status — DONE / SHIPPED, CLOSED 2026-07-12.** All phases **P1–P7** + the **PC1–PC5** sub-rule /
parser-combinator cursor layer + streaming shipped to `main` across four PRs — #554 (design + P1 nested +
P2 `name:pat`/M-Total/`..rest`), #558 (P3–P6: multi-pattern arms, alternation, optional, repetition +
separators, literal elements + `#lexeme`), #561 (scalar captures, field projection, non-literal tails +
P7 streaming-iterator match), #562 (PC1–PC5 + the cursor-repetition fix). The plan's "Exit (full)"
criteria are met; nothing is half-implemented.
[`@PLN35`](https://github.com/loft-lang/plans/issues/35) is **`status:finished` / closed**. **This file
is now the closed historical record** (design rationale + what shipped + how it was verified); the living
reference is [LOFT.md](../../LOFT.md) (match / patterns) plus the `tests/scripts/35*.loft` guards this doc
points at. **Deferred refinements** (each a graceful limitation rejected with a diagnostic, or a future
enhancement — none a core gap): heap-payload field projection, PEG cut `~`/`!~`, sub-rule + fixed-element
mixing in one pattern, lazy streaming (eager materialise-then-match shipped), and non-scalar stream
channels (tuple / vector iterator elements). Reactivate any via a fresh issue when a consumer needs it.

> **Companion to [README.md](README.md)** (the design draft) and
> **[FORMAL-DESIGN.md](FORMAL-DESIGN.md)** (the strict-spec changes that GATE this
> build — read it first: the rules are the target these phases are built to satisfy,
> and §5 there maps each phase below to the formal rules it discharges + pins). This
> doc is the *build* plan: ordered phases, each independently shippable and verifiable
> on **both backends** (`--interpret` + `--native`), with the exact code points to
> touch. The README stays the design/rationale reference; keep design prose there,
> keep build steps here. The **sub-rule / parser-combinator layer** (invoking named grammar rules
> inside a pattern — the keystone) is its own design + steps in
> [SUBRULE-DESIGN.md](SUBRULE-DESIGN.md) (phases PC1–PC5, layered after the core P1–P7).
>
> **Weight note.** This is the **last language-syntax feature** planned for loft
> (everything after is tooling, optimization, ANSI-C library reach). It earns
> design rigor: every phase names its invariant and proves it with a red→green
> cross-mode probe before moving on.

---

## Status snapshot (historical — 2026-07-11, before the final merges)

> ⚠ Superseded by the **CLOSED** banner at the top of this file. This section predates the final merges;
> every branch it names below is now on `main` (#554/#558/#561/#562). Kept verbatim as the build-order
> record — do NOT resume from here.

**Branches.** Phase 0–2 MERGED to `main` (#554, squash). **Phase 3 (L3.7 multi-pattern arms),
Phase 4 COMPLETE (L3.2 alternation: 4.1 single-element, 4.2 `option<T>` promotion, 4.3
multi-element sequence branches via the cursor engine), Phase 5 (L3.3 optional `(a)?` as a
degenerate `(a | ε)` alternation — one empty branch on the 4.3 engine), Phase 6.1 (L3.4
repetition `( [name:] V )*` / `…+` on struct-enums — a runtime run-loop + whole-element
collect), and a fix for a pre-existing crash when a user type is named `T`** are on
`tuxedo-pln35-phase3-multipat`, rebased onto `main` (NO PR — the user does not want one; keep
committing to the branch). Full-suite **green** on this box (only the environmental
`wasm_debug_relay` fails; see memory `wasm-debug-relay-env-fail`). **Nothing is half-implemented.**

**Cursor-mode repetition fixed (2026-07-12, `tuxedo-pln35-pc-subrule`) — surfaced by the
`arguments` dogfood library.** A repetition `( [name:] V )*` / `+` inside a CURSOR match was only
correct at `pos == 0`: it read the run through `read_slice_elem(Var(end))` (which does NOT offset a
`Var` by the cursor `pos`, so it read absolute `source[end]`), it required the run to reach the
source END (`end == len − tail_len` — the whole-vector boundary, which rejected a prefix stop), and
it never advanced `cursor.pos` (the `Repetition` branch sets `multi_alt = true`, skipping the
fixed-arity advance, and `parse_slice_repetition` had no `match_cursor` writeback). Fix in
`parse_slice_repetition`: in cursor mode the run starts at an ABSOLUTE `base = pos + head_len`
(vector mode keeps `Value::Int(head_len)`, so the emitted IR stays byte-identical — 8 existing
repetition/cursor scripts unchanged on both backends, and a vector introspect shows zero
`rep_base`/`rep_adv` scaffolding), the match boolean becomes the prefix test `base <= len` (so a
trailing `(V)*` consumes the maximal run and LEAVES any non-V tail), a fixed tail after the group is
REJECTED on a cursor (its "end" is the source end, meaningless for a prefix), and the cursor is
advanced to the run end (or to `len` when a `..rest` consumed the remainder), with the PC5 `farthest`
high-water update mirrored. Guard: `tests/scripts/35t-cursor-repetition.loft` (prefix-stop, run at
`pos > 0` with value check, `*`/`+`, `..rest`, a driver loop, sub-rule tail form; cross-mode +
leak-checked).

**Pre-existing NATIVE `[]`-empty-arm E0308 — FIXED, BOTH contexts (function-return AND
assign-to-local).** In a VECTOR-returning `match`, an EMPTY `[]` arm (`_ => []` / `_ => { [] }`) lowered
to a bare `null`, which native emitted as `()` where a vector (`DbRef`) is expected → E0308. Reproduced
in plain VECTOR mode (predates the cursor work) whenever a materialised vector sits beside an empty arm —
`fn f(v) -> vector<T> { match v { [x, ..rest] => rest, _ => [] } }`, `[ (xs:V)* ] => xs`, `[a,b] =>
[a,b]` — and `cap = match v { … , _ => [] }`. ROOT (proven-sibling IR diff — a struct-enum `match` of the
SAME shape works because its arms all deliver into the pre-cleared `__retbuf`, so its `[]` is a harmless
empty `else ;`; the SLICE model materialises its `..rest`/`(V)*` capture STRAIGHT into a store and left
the `[]` arm an undelivered `null`). **FIX (one mechanism): `parse_vector_match::materialize_null_slice_
arms`** — when the match RESULT is a vector, rewrite each `null` arm of the tail to a FRESH empty vector
`{ OpDatabase(o); o }` (using the RESULT's own element type). Now every arm is a real vector: the
function-return delivery renames it onto `__retbuf` as usual, and an assign-to-local `cap = <match>` just
ALIASES the owned match result (leak-free — verified to 200 iterations both backends; the earlier
interp `2→0` was a stale-build/two-match-file artifact, not real). Gated on `result_type == Vector` so a
CURSOR/prefix match returning a struct/scalar (`[ n: rule ] => …, _ => null`) keeps its genuine `null`
arm (the `35r` regression that gate fixed). **Dead-ends recorded for the next reader** (each
suite-caught): (a) a `Delivery::Rename` rewrite delivering `[]` into `__retbuf` — worked for the return
context but is now subsumed by the materialise (removed); (b) lifting the `vec_match_candidate`
`!tail_if_has_null_arm` gate + a blanket `Value::Null` materialise DOUBLES a borrowed struct-enum return
(drops the preamble clear the join_own append relies on); (c) gating on `ownership_of` at `block_result`
can't tell the models apart (the join_own synthesis already rewrote the borrowed arm); (d) a `CopyMatch`
verdict COPYING the match into `cap` (oracle-driven, values correct) LEAKED the match-internal store
(the copy orphans it) — unnecessary once the alias path is leak-free. Guard
`tests/scripts/85-store-lifetime-empty-vector-match-arm.loft` (return + bind + block-form + reuse; both
backends, leak-clean).

**Phase 6.1 + 6.2 — DONE (repetition + separator, struct-enum), both backends.** `[ ( [name:] V )*
[, ..rest] ]` / `…+`: a runtime run-loop counts the maximal leading `V` run into `end`; `name`
collects `v[0..end]` (whole `vector<ElemType>`, reusing `..rest`'s `materialize_named_rest`),
`..rest` gets `v[end..len]`. `*` = zero-or-more (no-rest ⇒ `end == len`); `+` ⇒ `end > 0`. **6.2
separator** `( V )*(Sep)`: the run becomes `V (Sep V)*` (a `first V; loop (Sep V)` run-loop);
`Sep` is consumed not captured, so the capture reads `v[0..end]` with a STRIDE of 2 (new `step`
param on `materialize_named_rest`) to skip separators; trailing `Sep` is not part of the list.
The run-loop lives in the arm condition (yields the boolean). ONE unified look-ahead
`peek_group_kind` (`Other`/`Alt`/`Repetition`, single save/revert — two sequential peeks corrupt
the lexer replay buffer) classifies the group; routing computes `group_kind` once. Guards
`tests/scripts/35i-repetition.loft`, `35j-repetition-separator.loft`. **#556 FIXED** (was: native
`for` over a match-arm materialised vector iterated 0×; real cause = a slice-arm block dropped its
result on native — a much broader bug, now repaired).

**Phase 6.3 (literal slice elements) + `#lexeme` — DONE, both backends — the grammar-reader.** A
LITERAL element `[ 1, … ]` / `[ "kw", … ]` matches by equality against `v[pos]` (scalar → direct
`==`; a `"_"` placeholder keeps positions aligned, composes with binds/`..rest`). The `#lexeme`
EXTENSION lifts this to token streams: a token-enum variant marks its surface-text field
`#lexeme` (`Keyword { #lexeme name: text }`), so on a struct-enum element a bare `"fn"` desugars
(`build_lexeme_literal_match`) to `tag==Keyword && name=="fn"` (OR'd over eligible variants) —
`[ "fn", Ident { name }, "(", ")" ]` reads like the grammar. `#lexeme` is a parse-time `Attribute`
flag (not serialised). Guards `tests/scripts/35l-literal-slice-elements.loft`, `35k-lexeme.loft`,
`parse_errors::{lexeme_missing, unknown_field_annotation, slice_literal_type_mismatch}`.
**#557 FIXED** — a pre-existing native text-arm-unification bug (a `vector<text>` subject with
mixed interpolation/literal returns → `Str::new(String)` E0308) surfaced here (NOT caused by
literal elements); fixed in `emit.rs` (`block_tail_materialises_string` routes the block through
the work buffer), guard `tests/scripts/557-vector-text-match-interp-literal.loft`.

**Test-infra (this branch):** `find_problems.sh` runs are now named by checkout
(`/tmp/loft_test.<dir>.<hash>.…`) with a scoped `--stop` (no more broad `pkill -f nextest`
killing a sibling checkout's run), and server-test ports are offset per checkout
(`common::test_port` + `LOFT_TEST_PORT_OFFSET`) so two concurrent suites don't collide on the
engine-host / wasm-relay fixed ports.

**MID-SLICE repetition + lexeme separators — DONE, both backends — the parser-combinator keystone.**
A repetition may now sit BETWEEN a fixed head and a fixed literal tail:
`[ Ident { name }, "(", (arg: Ident)*(","), ")" ]` reads exactly like a grammar rule for a call.
`parse_slice_repetition` takes `head_len`; the run starts there, collects `v[head_len..end]`, and a
fixed LITERAL tail is matched from the END (negative index) so the run must reach exactly
`len - tail_len` (`end == len - tail_len`; `+` ⇒ `end > head_len`; a rest just needs `head_len <= len`).
Head bare-names bind before the group; head sub-patterns/literals bind inline. Separators are now a
`SepSpec` — a variant TAG `(Comma)` OR a LEXEME literal `(",")` — via `sep_match_cond`, so a
comma-separated token grammar needs no dedicated separator variant. Non-literal tail elements
(bind / variant sub-pattern) now land in slice 3; a fixed tail + `..rest` together stays deferred. Guard
`tests/scripts/35m-mid-slice-repetition.loft` (bracketed lists, function calls, callee-name bind,
value-by-index, `+`; cross-mode + leak-checked).

**Phase 5 — DONE (optional), both backends.** `(a)?` = degenerate alternation `(a | ε)`: the
empty branch always matches so the optional never fails; present → bind + advance cursor by `a`'s
width, absent → ε commits with cursor unmoved and `a`'s captures read null (option-promoted by the
4.2 hook). `..rest` resumes at the matched width (0 when skipped). Built by pushing one empty
branch in `parse_multi_element_alternation` + `peek_paren_suffix` detecting the trailing `?`.
Guard: `tests/scripts/35h-optional.loft` (single + multi-element `(A B)?`, present/absent/partial/empty).

**Phase 4.3 — DONE (steps 1–5), both backends.** `[ (A B | C), ..rest ]`: a branch may be a
SEQUENCE of varying width, dispatched PREDICTIVELY on the leading tags (ordered `if/else` over
pure per-branch predicates, no `save`/revert — a tag branch is a pure test that only advances on a
full commit); `..rest` reads `v[pos..]` from a runtime cursor = the matched branch's width. Built
per §3a: step 1 `read_slice_elem` seam (byte-identical refactor), step 2–4 `parse_multi_element_alternation`
+ `peek_multi_element_alt` lexical lookahead, step 5 `materialize_named_rest(lo, hi)` (extracted,
shared with the fixed-arity path). Guards: `tests/scripts/35g-multi-element-alternation.loft`.
**Slice 1 — DONE, both backends — scalar type-annotated captures (branch
`tuxedo-pln35-phase7-scalar-rep`, stacked on the PR-#558 tip).** `[ head…, name:Type*|+ [, lit…] ]`
on a `vector<Scalar>`: the scalar type matches EVERY element, so the run takes exactly the middle
(`end = len - tail_len`, no run-loop) and `name` collects `v[head_len..end]` as a fresh
`vector<Type>` (reusing `materialize_named_rest`); a fixed literal head/tail pins the ends. The
sibling **single** form `name:Type` (a type-as-match that always holds, previously SILENTLY
never-matched via `self.expression`) now binds one element. `peek_scalar_type_capture`
classifies the `name:Type[*|+]` element in ONE save/revert (excludes the `_` wildcard sub-pattern
and enum elements, which keep the `name:pat` / `(x:V)*` paths). Diagnostics: type≠element,
`..rest`-after-run, non-literal tail (recovers to `]`). Guards `tests/scripts/35f-repetition.loft`
(graduates the golden `g-p6-repetition.loft`), `parse_errors::scalar_rep_{type_mismatch,
rest_unsupported, nonliteral_tail}`. **Prereq fixed first: the lexer `link`/`revert` primitive** —
`cont()` re-appended a duplicate when replaying the LAST buffered token, so two look-aheads over
the same region (`peek_named_arg` + the new classifier) desynced the parse; now gated on the
edge-state captured BEFORE `next()` (guards `lexer::test::link_revert_{repeatable_same_region,
nested_links}`).

**Slice 2 — DONE, both backends — per-iteration field projection `( V { field, … } )*`.** Each
named SCALAR/TEXT field of the run's variant collects into its own fresh `vector<field_type>` (a
projection), vs a `name:` prefix which collects whole elements — the two compose (`( xs: V { n } )*`
gives both). New `materialize_field_projection` mirrors `materialize_named_rest`'s iterator but the
per-element read is `read_slice_elem` → `get_field(variant, attr_idx, ·)` (the same field read the
head sub-pattern path uses; a raw `OpVectorRefNullable` reads null — the pre-fix bug), collecting
`vector<field_type>` (scalar/text = owned, so no borrow-dep dance). Works with `*`/`+`, a trailing
`..rest`, a separator `(Sep)` (stride-2), and multiple fields. A non-scalar field, or a name that
is not a field of `V`, is rejected. Guard `tests/scripts/35n-field-projection.loft`,
`parse_errors::field_capture_{nonscalar_deferred, unknown_field}`.

**Slice 3 — DONE, both backends — non-literal tail elements after a repetition.** `[ (V)*, x ]`
(bare-name bind) and `[ (V)*, W { f } ]` (variant sub-pattern), matched from the END at
`-(tail_len - j)`, mixable with literals and a head; still mutually exclusive with `..rest`. The
tail is COUNTED first (a link/revert look-ahead — now safe after the `cont()` fix) so every element
uses a fixed NEGATIVE index; reading from the run cursor `v[end + j]` instead diverges on native
(the run's `end`, set in the arm condition, isn't reliably visible to a tail read appended after
it — E0425 / wrong result). A variant tail's tag-test + field binds use the head path's DIRECT read
(no cross-block temp — a temp assigned in the condition reads back null in the binds on native).
Guard `tests/scripts/35o-tail-elements.loft`, `parse_errors::tail_and_rest_rejected`.

**Next: Phase 7 (iterator input — the accumulating `read(pos)`, the only phase adding opcodes),
then the PC1–PC5 sub-rule layer. §3a step 6 (fold hook) waits on PC.**

**Phase 3 (L3.7) multi-pattern arms — COMPLETE (2026-07-11).** `V1 { c }, V2 { c } => body` runs
the body for whichever variant matches, binding the SAME captures (D-simple: identical name sets
at compatible types; partial overlap → `option<T>` stays Phase 4) into SHARED slots that each
listed variant assigns from ITS OWN offsets. Desugars to the hand-expanded `V1 { c } => body; V2 { c }
=> body` form — proven byte-for-byte in [`bytecode-comparisons/p3-existing-paths-corpus.loft`](bytecode-comparisons/p3-existing-paths-corpus.loft)
(existing paths stayed byte-identical) and confirmed on both backends. Union exhaustiveness: an arm
covers the union of its listed variants. Code: `parse_match` arm loop + two helpers (`variant_disc`,
`parse_multi_pattern_extra_bindings`) in `control.rs`. A guard or a field sub-pattern on a
multi-pattern arm is rejected with a "Phase 4" diagnostic. Guards: `tests/scripts/35e-multi-pattern-arm.loft`
(cross-mode, leak-checked, includes the reuse axis), `tests/parse_errors.rs::multi_pattern_*` (5 cases).
The `|` or-pattern stays the tighter same-shape tag alternation; `,` is the looser cross-shape form.

**Phase 2 (L3.1) `..rest` — COMPLETE (2026-07-11).** It binds a FRESH independent `vector<T>`
tail by **REUSING the compile-time `#Slice materialise` path** (`materialize_iterator`) — the
`slice_vector` native primitive the old "BLOCKER" section below planned was NOT needed. The
store-lifetime work that reuse exposed (3 classes) is fixed and documented in
[`rest-store-lifetime/ANALYSIS.md`](rest-store-lifetime/ANALYSIS.md); read it before touching the
slice path. Commits: `d8310875` (class-C slice miscompile), `e2742866` (A/B leaks + oracle + corpus),
`1975a2eb` (the `..rest` feature). The rest of this section is the historical record.

**Design — COMPLETE.** [README.md](README.md) (draft, reconciled), [FORMAL-DESIGN.md](FORMAL-DESIGN.md)
(spec-first rules, the `INV-Total` invariant), [SUBRULE-DESIGN.md](SUBRULE-DESIGN.md) (the
parser-combinator keystone: `Cursor`, `P-Rule`, `INV-Static`, reporting; phases PC1–PC5),
[EXAMPLES.md](EXAMPLES.md), plus the strict-spec edits in `doc/claude/formal/*`
(matching/types/grammar/binding + `collections.md` scope). Decision **C89** (no tuple variants ever;
matchers are opt-in + read like grammar, not regex) → [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md).

**Phase 0 — DONE.** D2 architecture bet CONFIRMED on both backends
(`probes/p0-d2-backtrack.loft`: slice backtracking = save/reset an index temp, **no new opcode**).
Golden target-syntax specs in `probes/g-*.loft`.

**Phase 1 (L2 nested patterns) — DONE**, both backends, byte-identical gate + full-suite green:
- **P1.2** nested struct-enum FIELD patterns `Parcel { detail: Ship { carrier } }` — commit `a0f79bf4`
  (`parse_field_sub_pattern` in `control.rs` recurses for struct-enum fields).
- **P1.3** nested variant SLICE-ELEMENT sub-patterns `[Ship { carrier }, ..]`,
  `[first, Ship { carrier }]` — commit `841ac6b7` (`parse_vector_match` detects an enum-variant
  element, reads `v[pos]`, tag-tests+binds via `parse_field_sub_pattern`; a `"_"` placeholder keeps
  positions aligned). Guard: `tests/scripts/35-nested-match.loft` (cross-mode, leak-checked).

**Phase 2 (L3.1) — COMPLETE.** `name:pat`, M-Total, P-Cap-View (below) + `..rest` (see the
`..rest` section further down) all landed, both backends, full-suite green.

**LANDED (this cycle), both backends, byte-identical for untouched paths + full-suite green:**
- **`name:pat` element captures (P-Cap).** An element may be `name:subpattern` — bind the whole
  element to `name` (a VIEW of the subject) AND require `subpattern` to match it. Detected with
  `lexer.peek_named_arg()` (identifier + single `:`); routes the sub-pattern through
  `parse_field_sub_pattern` (variant / variant+fields / literal / `_`). Head position only.
- **Totality gate (F6 / `M-Total`).** A vector match is exhaustive only if its FINAL arm is total
  (a `_` or a bare binding) — a slice pattern is length-constrained, hence non-total. `parse_vector_match`
  now emits a static error instead of the old silent typed-null fallback. Guard:
  `tests/parse_errors.rs::vector_match_not_exhaustive`.
- **P-Cap-View borrow fix (`mark_slice_element_view`).** A slice-element capture of a HEAP element
  (`Reference`/`Vector`/struct-enum) is a borrowed view: it now carries `Deps::frame1(subject-source)`
  and `skip_free`, mirroring the #429 struct-field-binding handling. Applied at all THREE sites (head
  loop, tail loop, name:pat). Fixes a **pre-existing** corruption: bare `[first, ..] => first` over a
  heap vector, capture used then subject reused, freed the subject's record (both backends). The
  frame-dep names the real subject source, not the internal `_match_subj` copy.
- **Parser progress guards.** Both loops in `parse_vector_match` (`element` + `arm`) had
  `else if !first_pass { break }` which SPINS in the first pass on any unrecognized token (a literal
  element `[0, x]`, a tail variant sub-pattern `[.., V { f }]`). Now they break unconditionally →
  clean diagnostic, no hang.
- Guard test: `tests/scripts/35b-sequence-capture.loft` (cross-mode, leak-clean; scalar/heap/Reference
  captures × variant/literal/wildcard × capture-used-locally). Byte-identical corpus:
  `bytecode-comparisons/p2-lighter-corpus.loft`.

**TWO pre-existing bugs SURFACED (not fixed — out of the lighter-bits scope; both repro on `main`):**
1. **Heap slice-view RETURNED from a fn corrupts the subject.** `fn f(ds) -> Detail { match ds {
   [tok:Ship, ..] => tok, _ => Pickup {..} } }` then reuse `ds` at the caller → corruption on BOTH
   backends. Root cause: the return-type join of a borrowed-view arm and a fresh-alloc arm (the
   totality-forced `_` arm) classifies the fn OWNED, so the caller `OpFreeRef`s the escaped view.
   The fix needs **copy-on-escape materialization** — the SAME machinery `..rest` is blocked on (below).
   The `mark_slice_element_view` fix covers the capture-used-LOCALLY case; RETURNING a raw heap view is
   the deferred remainder. Repros with a bare slice binding too (not name:pat-specific).
2. **Native `Str`/`&str` mismatch in a vector-match text arm** — FILED as **#552** (`sev:medium`
   `area:native` `wa:partial`). `match v { [..] => fn_returning_text(x), _ => "literal" }` → native
   E0308 (`expected Str, found &str`). Vector-match-specific (plain `if`/`else` and scalar-match are
   fine); repros with a SCALAR slice (the byte-identical corpus proves the PLN35 edits don't touch it).

**Syntax DECIDED: `..rest`** (two dots + adjacent name), NOT `...` — loft lexes `..`, not `...`.
Docs reconciled to `..rest` throughout.

**~~BLOCKER — the tail sub-slice materialization primitive~~ — RESOLVED by REUSE (2026-07-11).**
The old plan built a new `vector::slice_vector` native primitive; instead `..rest` in
`parse_vector_match` (`control.rs`) constructs a minimal slice `Value::Iter` over
`v[head_len .. len − tail_len]` and hands it to the EXISTING `materialize_iterator`
(`expressions.rs`, made `pub(crate)`) — the same element-type-aware deep-copy loop that
`s = v[lo..hi]` uses. No new opcode, no `slice_vector`, no generics. `rest` is a fresh
independent `vector<T>`: safe to return, mutate, index, pass on. Bug #1 below (heap slice-view
returned corrupts the subject) is subsumed — the A/B store-lifetime fixes handle the escaping
capture into an enum / `&text` return. Guards: `tests/scripts/35c-rest-capture.loft`,
`35d-slice-enum-reuse.loft`.

**Store-lifetime rigor (the payoff of the reuse path).** Reusing `#Slice materialise` exposed
three store-lifetime classes, all fixed under a @PLN85-style attack (33-probe corpus + a
reporting OBSERVER oracle `LOFT_REST_ORACLE`, both backends, 0 mismatches). See
[`rest-store-lifetime/ANALYSIS.md`](rest-store-lifetime/ANALYSIS.md): **A/B** = a `..rest` arm's
`__vdb` free landed before its alloc for non-hoisted return types (fixed in `insert_free`);
**C** = a PRE-EXISTING interpret-only miscompile of `v[lo..hi]` on struct-enum vectors (the
`comp_var = for_var` intermediate dropped the borrow-dep → the free-analysis freed the subject's
own record; found only by adding the **reuse axis** the corpus first missed; fixed by dropping the
intermediate and deep-copying the dep-carrying `for_var` directly, unifying both backends).

**Instrument (loft-codegen gate).** `bytecode-comparisons/`: one-fn-per-path corpora + `INSTRUMENT.md`
— the two-part byte-identical capture (`LOFT_LOG=static` IR + `loft --native-emit` Rust; **warm up
once** before capturing — first-run slot flake). Before/after captures are regenerable (removed from
git). Two pre-existing **TOOLING BUGS** worth filing: (1) `introspect`/`--dump` PANIC on a match with
an enum `==` (`debug.rs:1077` type-table index); (2) the static-dump first-run slot flake.

**Other decisions in play.** Bare-postfix quantifiers `Num { value: n }*` (no parens for a single
element; `()` only groups a multi-element sequence). The sub-rule/parser-combinator layer
([SUBRULE-DESIGN.md](SUBRULE-DESIGN.md), PC1–PC5) comes after the core P-phases.

---

## 1. What the code actually looks like (reconciliation with the design draft)

The README was written against an idealized syntax. Six facts from a full read of
the current `match` implementation change the plan. **Read these first — they are
why the phase order below differs from the README's L3.1→L3.7 list.**

| # | Finding | Code anchor | Consequence for the plan |
|---|---|---|---|
| **F1** | **There is no `match`/pattern IR node.** `match` is desugared *entirely in the parser* into nested `Value::If` chains + primitive ops. Both backends consume the same `If` IR. | `src/data.rs:476` (`enum Value` — has `If` at `:530`, no `Match`); `src/parser/control.rs:2620` (`parse_match`) | **Do not add an IR node.** All of L2–L3.4 is parser desugaring → both backends "for free". Adding a `Value::Match` variant would double the work (teach `fill.rs` *and* `generation/emit.rs`). |
| **F2** | **Slice backtracking needs no new opcode.** A slice cursor is a `usize` index; "revert" = reset an index temp. `parse_vector_match` already does index arithmetic with `OpGetVector`/`OpLengthVector`. | `src/parser/control.rs:3960`, `:4010`, `:4063` | `OpMatchAnchor`/`OpMatchRevert` are needed **only for iterator input (L3.6)**. L3.1–L3.4 emit no new ops. This is the single biggest de-risking. |
| **F3** | **Four separate arm-parsers, one per subject kind** — enum (inline in `parse_match`), scalar, vector, tuple. Not one recursive pattern grammar. | `control.rs:2620` (enum), `:3797` (scalar), `:3960` (vector), `:4169` (tuple) | PEG needs sub-patterns in *any* position. Phase 1 introduces one recursive `parse_pattern()` the four handlers delegate to. This unification is the backbone. |
| **F4** | **Nested sub-patterns don't parse** (`V { field: Inner { x } }`). A field with `:` only accepts a plain-enum variant name, `_`, or a scalar literal; a struct/struct-enum field falls to `self.expression()` and chokes on the inner `{`. | `control.rs:3554` (`parse_field_sub_pattern`), guard at `:3556` (`Type::Enum(_, false, _)`), scalar fallback `:3633-3636` | This is **L2**, the stated prerequisite, and it is unimplemented. It is Phase 1. |
| **F5** | **`..rest` named tail-capture does not exist.** `..` is a length-flex marker binding nothing; `[a, ..rest]` mis-parses (`rest` becomes a single last element). | `control.rs:3990` (`..` marker), element loop `:3986-4007` (bare `Vec<String>`) | True named-rest capture is a genuine gap added in Phase 2 (L3.1). |
| **F6** | **Exhaustiveness/totality is enforced for enums only.** Vector/tuple/scalar handlers fall back to a typed null with no coverage check. | enum check `control.rs:3138-3156`; vector fallback `:4141-4146` | The README rule "sequence/repetition are non-total → require a trailing `_`" has **no existing home** in the vector handler. Phase 2 adds it there. |

Two more facts inform the design but aren't obstacles:

- **F7 — loft has no tuple-style enum variants.** Only struct variants exist
  (`Num { v: integer }`, not `Num(integer)`). Every README example must be
  restated in struct-variant syntax. See **Decision D1**. (`tests/scripts/05-enums.loft:11`.)
- **F8 — recursion goes through a collection/reference, not an inline field.**
  `Neg { inner: Expr }` fails type-layout (`src/database/types.rs:364` skips the
  self-referential field → `u16::MAX` size → `:1021` "has no position"); `struct
  Node { kids: vector<Node> }` works because a collection is a fixed-size DbRef.
  AST enums in tests/docs must use `vector<Expr>` (or a ref). Plan note only.

---

## 2. Load-bearing design decisions

Each is my recommendation with the reasoning; flagged so they can be overridden
before Phase 1 code lands. **D2 and D3 are the ones I'd most want a second look at.**

- **D1 — Struct-variant syntax only; tuple variants are a PERMANENT non-goal ([C89](../../DESIGN_DECISIONS.md)).**
  loft enum payloads are always named fields (F7); positional `Num(i64)` / `Ok(a)` are never
  planned — they force match-to-read and breed a mitigation-syntax sprawl (the C89 rationale).
  Every example uses `Num { v: integer }`. **Cost:** AST examples read more verbosely — accepted.
  **Corollary (D6):** the pattern surface must read like grammar notation, not regex, even at the
  price of extra parser logic (C89) — patterns are the standardized readable parser notation
  (`|`/`?`/`*`/`+` on named elements), text/regex stays a library.

- **D2 — Parser desugaring, no new IR node, for L2–L3.4.** Follows F1/F2. Keep the
  proven "lower to `If` + existing ops" path; both backends stay free. *Rationale:*
  the alternative (a first-class PEG IR node) doubles backend work for no user-visible
  gain until iterators (L3.6). **This is the architecture bet the whole plan rests on
  — if it's wrong, the phase order is wrong.** It is falsified in Phase 0 by a probe
  (below) before any feature code.

- **D3 — One recursive `parse_pattern()`; the four handlers become callers.**
  Signature (working):
  ```rust
  // returns the boolean test (None = always-matches) + the binding statements
  // this pattern contributes, given a Value that reads the subject and its type.
  fn parse_pattern(&mut self, subject_read: Value, subject_type: &Type)
      -> PatternMatch;                 // { cond: Option<Value>, binds: Vec<Value>, total: bool }
  ```
  *Rationale:* PEG requires a sub-pattern in field, element, alternation, optional,
  and repetition positions — that is only expressible with recursion. The current
  `parse_field_sub_pattern` (`control.rs:3554`) is a degenerate 3-case version of
  exactly this; Phase 1 generalizes it and routes the field/element positions
  through it. *Risk:* it is a behavior-preserving refactor of live match lowering →
  gated by the loft-codegen rule (prove emitted IR byte-identical before adding any
  new case; see Phase 1).

- **D4 — Backtracking = save/reset an index temp (slices); a shadow anchor stack
  (iterators).** For slices (L3.2–L3.4) a "cursor" is a `usize` local; anchor =
  `Set(save, pos)`, revert = `Set(pos, save)`. For iterators (L3.6) add
  `anchors: Vec<(u32 pos, u32 epoch)>` on `State`, modeled on the existing shadow
  stacks `call_stack`/`coroutines` (`src/state/mod.rs:164,170`), plus the two new
  opcodes. *Rationale:* mirrors the design's own "Validated input shapes" table
  (slice revert cost = 1 word, no memo; iterator needs memo).
  **Refined by §3a (the matching engine):** backtracking is now the EXCEPTION, not the
  default — a tag choice is predictive (no anchor), the `save`/revert lives INSIDE a rule
  that can consume-then-fail (never at the call site), first-sets bound it statically, and
  left recursion is eliminated into repetition rather than grown. Read §3a before D4's
  slice-cursor mechanics.

- **D5 — Provisional bindings reuse the existing per-arm name-scoping.** Bindings are
  already permanent slots whose *name* visibility is saved/restored per arm
  (`control.rs:3075-3081`), with `skip_free` so an untaken arm doesn't free a slot it
  never wrote (`:3472-3474`). A failed alternative simply takes the `else` branch and
  never reaches the body — its slot is written-but-unused, not corrupt. *Rationale:*
  no new rollback machinery for slices; the "bindings epoch" in the README is only
  needed for the iterator memo (L3.6).

---

## 3. Backbone: the unified recursive pattern parser

Everything hangs off D3. The target end-state (reached incrementally, not all in
Phase 1):

```
parse_pattern(subject_read, subject_type) -> PatternMatch
├─ literal / range        → OpEq* / range test        (exists: parse_match_pattern control.rs:3663)
├─ wildcard `_`           → cond=None, binds=[]        (exists, scattered)
├─ binding `name`         → Set(mv, subject_read)      (exists: control.rs:3442)
├─ enum variant `V {..}`  → tag test + recurse fields  (Phase 1 — generalize :3403/:3554)
├─ slice `[p, p, ..r]`   → len test + recurse elems   (Phase 1/2 — generalize :3960)
├─ tuple `(p, p)`         → recurse elems              (Phase 1 — via :4169)
├─ alternation `(a|b)`    → predictive if/else on leading tag (§3a)   (Phase 4)
├─ optional `(a)?`        → try a; else bind null (contract: pos unmoved on fail)   (Phase 5)
└─ repetition `(a)* (a)+` → forward loop of a, predictive stop (§3a)   (Phase 6)
```

`PatternMatch { cond: Option<Value>, binds: Vec<Value>, total: bool }` is the
uniform currency: a handler builds its arm by AND-folding child `cond`s (as
`control.rs:3016-3028` already does for field sub-conditions + guard) and
concatenating child `binds`, then `total` drives the exhaustiveness gate (F6).

---

## 3a. The matching engine — the algorithmic core (cursor · backtracking · streaming · recursion)

> The phases below (alternation, optional, repetition, iterator input) **and** the
> parser-combinator layer ([SUBRULE-DESIGN.md](SUBRULE-DESIGN.md)) are all one engine. This
> section is that engine's design, decided end-to-end (2026-07-11) so the phases build it
> incrementally without any of them needing a redesign — read it before Phase 4.3 or the PC
> layer. It **supersedes** the "anchor; try a; revert; try b" sketch in D4 / §3: backtracking
> is the *exception* here, not the mechanism.

**The substrate — a forward `pos` cursor over a `read(pos)` seam.** Matching walks a sequence
with one logical position `pos` and reads elements through a SINGLE accessor `read(pos)` —
never a raw `OpGetVector` scattered through the desugaring. Two backings: a MATERIALIZED
vector (everything today) where `pos` is an index and `read(pos)` is `OpGetVector(v, size,
pos)`; and a STREAMED source (Phase 7) where `read(pos)` pulls from a lexer coroutine and
accumulates (below). Funnelling every element access through `read(pos)` is what lets the
streamed backing drop in as a substitution, not a rewrite.

**The cursor contract — consume on success, untouched on failure.** Every rule (and the whole
match) upholds one invariant: on SUCCESS it advances `pos` past what it matched; on FAILURE it
leaves `pos` exactly where it started. This is the parser-combinator contract, and it is what
lets rules compose without knowing each other's internals:
- an ALTERNATION never saves or reverts — a failed branch has already put `pos` back, so the
  next branch resumes correctly; the choice is just "try each branch; the one that commits
  advances `pos`";
- the revert that makes a COMPOSITE rule honour the contract — a *sequence* that matched its
  first parts then failed later — lives INSIDE that rule: it anchors its own start and
  restores `pos` before returning failure. `save = start; … ; on fail pos = start` appears
  ONLY there, as the rule's own responsibility, never the caller's;
- a TAG branch honours the contract for free — it tests its tags as a pure boolean (reads,
  moves nothing) and advances `pos` only on a full commit.

Keep the `pos`-advance factored so any rule can own advance-on-commit / restore-on-fail.

**Predictive dispatch — the fast path, no backtracking.** At a choice `( a | b | … )`,
dispatch on the LEADING element's tag, the way a recursive-descent parser predicts a
production from its first token. Because a branch condition is pure, the whole choice is an
ordered `if / else if` over those conditions: the first whose fixed-length tag sequence
matches commits, binds, and steps `pos` by its width. `&&` short-circuit gives the
first-lookup failure for free, and branches that share a leading tag just fall one tag deeper.
The common case (disjoint enum tags) does ZERO backtracking — `pos` only moves forward.

**Bounded backtracking — guaranteed by static first-sets.** Compute, per choice point, the set
of leading tags each branch can begin with:
- DISJOINT first-sets → the leading tag determines the branch → no backtracking, *proven at
  compile time* (predictive dispatch promoted from a runtime accident to a static guarantee);
- OVERLAPPING first-sets → a graded, explicit response, never silent exponential risk:
  (1) reject by default ("branches share leading tag `A` — disambiguate"), or (2) allow a
  STATIC bounded lookahead `k` (LL(k) — reject if the needed depth exceeds `k`), or (3) permit
  real backtracking only behind an explicit `cut`, capped at `max_lookahead`.

The only way to get backtracking at all is to ask for it, and even then it is bounded by
construction. This makes **"streamable" a compile-time property** — a rule needing unbounded
lookahead is flagged at compile time (fine on a materialized vector; will not stream). The
same first-set analysis closes the "no ambiguity warning" gap. Strict by default.

**Streaming — a lexer property, with monotonic accumulation.** Forward streaming lives in the
LEXER (a coroutine yielding tokens on demand), NOT the matcher. The matcher stays a `pos`
cursor over `read(pos)`; for a streamed source `read(pos)` pulls-and-appends from the coroutine
when `pos` passes the buffered edge. A coroutine can't un-yield, so backtracking needs a
rewindable WINDOW — three positions, not one:
- `cursor` — moves forward on consume, back on a rule's restore; the only thing the contract
  touches;
- `high-water` — the furthest position ever pulled; MONOTONIC, never moves back; the buffer's
  leading edge;
- `evict frontier` — the earliest position any live rule could still backtrack to; nothing
  below it is reachable, so it is droppable.

The trick that makes an un-re-readable stream compatible with "restore on failure": a failing
rule rewinds the `cursor` but leaves `high-water` alone — accumulation is monotonic; only the
cursor reverts. `max_lookahead` bounds `high-water − evict`, keeping the buffer bounded. The
materialized vector collapses all three (buffer = the whole vector; nothing evicts). The loft
LEXER already does exactly this at PARSE time (`link()`/`revert()` buffer scanned items) — the
same mechanism, one layer down at runtime.

**Left recursion — eliminate into repetition, don't seed-and-grow.** `A := A α | β`
infinite-loops in top-down matching. The general PEG fix (Warth seed-and-grow) is a
backtracking variant, but it is PACKRAT — it memoises the growing seed and re-parses the whole
construct from a pinned start, reintroducing full memoisation and an unbounded buffer, which
breaks streaming and bounded backtracking. The fit-for-this-engine answer is ELIMINATION:
rewrite `A := A α | β` to `A := β α*` — match one `β`, then iterate `α` forward. That is
forward-consuming (streaming-safe), predictive per step (peek the next `α`'s first-set), and
memo-free — and it REUSES the repetition machinery (Phase 6, `(…)*`) rather than adding a
mechanism. Left recursion becomes a rewrite into iteration. The graded line:
- static-DETECT left recursion (needed regardless — the PC correctness gate);
- DIRECT left recursion → auto-rewrite to `β α*` + a left-fold (recovers the associativity the
  left-recursive form implied);
- INDIRECT / mutual → reject with a clear diagnostic, or require a manual rewrite (the real
  complexity lives here; not worth dragging into a streaming engine);
- OPERATOR grammars → precedence climbing / Pratt layers cleanly on top and gives precedence +
  associativity as data (usually what you actually want for expressions).

**Associativity, precedence ladders, and the recursion residue.** Recursion is right for
genuine *structure*; what is NOT healthy is the fixed-depth descent a precedence LADDER
imposes on every expression. A hand-written ladder is N rules, one per precedence level
(`expr → assignment → ternary → … → additive → multiplicative → unary → primary`), and each
call descends the WHOLE tower to reach a leaf — so a bare `1` costs ~15–20 stack frames in a
real language, not because `1` is nested but because it had to fall through every level to be
reached. That per-expression constant is the cost to remove. Two moves remove it, both already
in this engine's vocabulary:
- **Associativity is a fold direction, not a recursion direction.** A left-associative rung is
  `B (op B)*` folded LEFT; a right-associative rung — the tail-recursive form `A := B (op A)?`
  — is the same `B (op B)*` folded RIGHT. Both are the repetition machinery (Phase 6); the
  author declares the associativity and the engine picks the fold. Neither associativity needs
  the recursive rule form.
- **The ladder collapses to one loop.** Expose operators as a declarative table
  `(operator, precedence, associativity)`; the engine compiles it to a single
  precedence-climbing loop (consume operators with precedence ≥ the current floor; recurse for
  the RHS at `prec (+1 for left-assoc)`). Cost becomes **O(operators actually present + genuine
  nesting depth)**, not O(precedence levels): `1 + 2` is one iteration, `1` is zero. The
  20-deep descent is gone because precedence is a *parameter*, not a stack level.

**The residue — the one recursion that stays, and should.** Structural nesting
(`primary := '(' expr ')'`) is genuinely recursive: a `(` opens a fresh sub-expression that can
nest arbitrarily, and matching brackets is not iterable — it needs a stack. But that recursion
is bounded by **nesting DEPTH, not input length**: a stream of a million flat `a + b + c + …`
is one loop and constant stack, while depth only grows with actual `((( … )))` nesting, which
is small and self-limiting in real code and never threatens streaming. So the target is not
"no recursion" — it is "no recursion as *overhead*": the ladder's per-level descent is
eliminated (it encodes precedence, which is data), and recursion is reserved for the one thing
that is actually a tree — nested structure. That is the healthy shape.

**How the phases build this incrementally.**
- **4.1 / 4.2 (done)** — single-element alternation: tag disjunction + conditional-offset
  captures, `option<T>` for partial overlap. A degenerate one-element cursor.
- **4.3** — multi-element sequence branches + predictive dispatch over a real forward `pos`;
  `..rest` reads `v[pos..]`. Pure tag branches → no backtracking emitted.
- **Phase 6 (L3.4)** — repetition `(…)*` / `(…)+`: the iteration left-recursion elimination
  reuses.
- **Phase 7 (L3.6)** — iterator input: the ACCUMULATING backing of `read(pos)` (the one place
  with new opcodes) + `max_lookahead`.
- **PC1–PC5** ([SUBRULE-DESIGN.md](SUBRULE-DESIGN.md)) — named rules + recursion: static
  first-set analysis (the backtracking bound), rule-invocation branches (each rule owns its
  cursor discipline — the `save`/revert seam finally used), and left-recursion detect →
  eliminate.

**Implementation steps this design ADDS (the extra work beyond the original phase list).** The
engine above is not free — it introduces prep and new scope §4's phases did not name. Ordered,
mapped to where each lands. These graft onto the numbered phase steps in §4; do them in phase
order.

*Phase 4.3 — DONE (2026-07-11), both backends. Establishes the cursor:*
1. **`read(pos)` seam.** Factor every slice-element read behind ONE accessor (a helper that
   emits the element load) and route today's fixed-position reads through it. A behaviour-
   PRESERVING refactor first — prove the emitted IR byte-identical (loft-codegen gate) before
   adding any new case — so Phase 7's accumulating backing swaps in without touching match
   logic. *This step alone is the biggest piece of the "extra work" and pays for streaming.*
2. **Runtime `pos` cursor.** A `pos: integer` local, seeded at `head_len`.
3. **Sequence-branch parse.** Parse a `(` branch as a SEQUENCE (`lexer.link()`/`revert()` to
   tell a multi-element branch from the single-element one 4.1 already handles).
4. **Predictive `if / else`.** Emit the ordered choice over PURE per-branch conditions
   (`len − pos ≥ wᵢ && tag(v[pos])==… && …`); the committing branch binds (conditional,
   option-promoted per 4.2) and advances `pos` by `wᵢ`.
5. **`..rest` from `pos`.** Re-point rest materialisation from the compile-time `head_len` lo
   bound to the runtime `pos` var.

*Phase 6 (L3.4 repetition) — adds the fold:*
6. **Associativity fold hook.** Repetition already collects a vector; add a fold-direction
   parameter (left / right) so recursion-elimination (steps 10, 12) can fold either
   associativity.

*Phase 7 (L3.6 iterator input) — the streaming backing:*
7. **Accumulating `read(pos)`.** Implement `read(pos)` over a bounded buffer fed by a
   lexer/iterator coroutine (the phase's new opcodes), tracking the three positions
   cursor / high-water / evict with `max_lookahead` bounding the buffer.
8. **Streamability check.** Reject at compile time a rule whose backtrack distance can exceed
   `max_lookahead` on a streamed input (fine on a materialized vector).

*PC1–PC5 ([SUBRULE-DESIGN.md](SUBRULE-DESIGN.md)) — analysis + recursion:*
9. **First-set computation** per rule + the graded overlap response (reject / static bounded-k
   / `cut`) — the compile-time backtracking bound AND the ambiguity diagnostic in one pass.
10. **Left-recursion detect → eliminate.** Direct `A := A α | β` auto-rewrites to `β α*` +
    left-fold; indirect / mutual is a clear-diagnostic REJECT.
11. **Rule-invocation branch.** A branch that calls a rule just checks the callee's matched
    flag; the callee owns its `save`/revert internally (the contract). First place the revert
    seam is actually emitted.

*New construct — the precedence table (genuinely new scope, slots with/after PC):*
12. **Declarative operator table → climbing loop.** A `(operator, precedence, associativity)`
    table compiles to one precedence-climbing loop, so an expression grammar costs
    O(operators + nesting) not O(precedence levels). Reuses the repetition fold (step 6) +
    first-sets (step 9) — it is their payoff, not a separate engine.

Sequencing note: steps 1–5 are the whole of Phase 4.3 and are do-able now; 6 waits for Phase 6;
7–8 wait for Phase 7's opcodes; 9–12 are the PC layer. Step 1 (the `read(pos)` seam) is the one
that must land *inside* 4.3 even though its payoff (7) is far later — retrofitting the seam
after the fixed-position reads have spread is the expensive path.

---

## 4. Phases

Ordering rationale: prerequisite first (L2), then the highest-value/lowest-risk
parser-only phases (L3.1, L3.7), then the backtracking trio (L3.2→L3.3→L3.4) which
share the slice-cursor desugaring, and finally the one phase that needs new runtime
primitives (L3.6). Each phase is a shippable PR.

Every phase's verification runs on **both backends** and is **leak-checked**
(`--interpret` runs `check_store_leaks`; native via the harness). Probe gate:
assertions pass · clean exit code · no leak warning · bounded runtime.

---

### Phase 0 — Reconciliation + executable spec (no feature code)

**Goal.** Lock the design to reality and write the red tests the later phases turn
green. Falsify D2 before building on it.

**Steps & verification.**
0.1 Restate README examples in struct-variant syntax (D1); add F1–F8 notes and the
    D1–D5 decisions to README § "Reconciliation". *Verify:* `make view` renders;
    doc-hygiene clean.
0.2 **D2 falsification probe.** Hand-write the *desugared* form of one alternation
    arm (`(0,x) | (1,x)`) as explicit `If`+index-temp loft and confirm it runs
    identically on both backends. *Verify:* if the hand-desugaring can express
    save/reset-index backtracking with existing ops → D2 holds, proceed. If not →
    stop and escalate to opcodes earlier. *(This is the cheap probe that could prove
    the architecture bet wrong.)*
0.3 Author the cross-mode golden files (initially `@EXPECT_ERROR`/ignored, flipped
    per phase): `tests/scripts/35-nested-match.loft` (P1),
    `35b-sequence-capture.loft` (P2), `35c-multi-pattern-arm.loft` (P3),
    `35d-alternation.loft` (P4), `35e-optional.loft` (P5), `35f-repetition.loft`
    (P6), `35g-iterator-match.loft` (P6/L3.6). *Verify:* each file exists and is red.

**Exit:** README reconciled; D2 confirmed by probe; 7 red golden files committed.

---

### Phase 1 — L2: nested sub-patterns + the recursive `parse_pattern` backbone

**Goal.** `match e { Neg { inner: Num { v } } => … }` and
`match xs { [a, Inner { x }, c] => … }` work on both backends.

**Design.** Two moves: (a) *behavior-preserving* extraction of a recursive
`parse_pattern()` (D3) that reproduces today's lowering byte-for-byte; then (b)
*additive* — route struct/struct-enum field positions and slice element positions
through it so a sub-pattern recurses.

**Code points.**
- `src/parser/control.rs:3554` `parse_field_sub_pattern` → generalize into
  `parse_pattern`; the guard at `:3556` (`Type::Enum(_, false, _)`) is the exact spot
  to also accept `Type::Enum(_, true, _)` (struct-enum) and struct `Reference` fields
  by recursing instead of falling to the scalar path at `:3633-3636`.
- `src/parser/control.rs:3403` `parse_match_enum_field_bindings` — field with `:`
  already routes to the sub-pattern parser at `:3437-3439`; point it at the new
  recursive entry.
- `src/parser/control.rs:3960` `parse_vector_match` element loop `:3986-4007` —
  replace the bare `has_identifier()` element read at `:3992` with a `parse_pattern`
  call per element; element binding at `:4055-4079` consumes `PatternMatch.binds`.
- Capture typing: binding type is read from the variant attribute at
  `control.rs:3425-3432` and assigned at `:3442`; heap bindings get the
  `Deps::frame1(src)` borrow-dep rewrite at `:3490-3508` — recursion must preserve
  both so backends don't diverge on free (F1 corollary).

**Steps & verification.**
1.1 Extract `parse_pattern` with only the existing cases; wire the four handlers to
    it. *Verify (loft-codegen gate):* `loft introspect` on a corpus of current match
    tests emits **byte-identical IR and native Rust** before/after. No behavior
    change yet.
1.2 Add the struct-enum / struct-field recursive branch (F4). *Verify:*
    `35-nested-match.loft` nested-enum arm green on both backends; leak-clean.
1.3 Add per-element recursion in slices. *Verify:* `[a, Inner { x }, c]` arm green
    both backends.
1.4 Parser diagnostics: nested depth is unbounded but well-formed; a malformed nested
    pattern gives a clean error, not a panic. *Verify:* add
    `tests/parse_errors.rs::nested_pattern_*` cases.

**Exit:** nested patterns work both backends; `parse_pattern` is the single pattern
entry; IR-identical refactor proven; `35-nested-match.loft` in the suite.

---

### Phase 2 — L3.1: sequence patterns + named sub-pattern capture + `..rest`

**Goal.** A slice arm reads like a parser rule:
`[ first:Ident, mid:Expr, ..rest ] => …`, with `rest` bound to the sub-slice.

**Design.** Generalize the slice grammar from "bare names + one `..` marker" to a
sequence of `parse_pattern` elements, each optionally `name:pat`, with **one** real
named-rest capture. `..rest` binds `subject[k..len-t]` as a sub-vector (new: F5).
Add the totality gate (F6): a slice arm using a sequence/rest is non-total → require a
trailing `_` or a full-cover set.

**Code points.**
- `src/parser/control.rs:3960` `parse_vector_match` — the head/tail `Vec<String>`
  (`:3983-3984`) becomes `Vec<PatternMatch>`; length test `:4010-4052` unchanged for
  fixed arity, `<=` branch (`:4013`) generalizes to "≥ min fixed elements" when a rest
  is present.
- Named-rest: new binding that materializes `subject[k..len-t]` — model the slice
  read on `OpGetVector` (`:4063`) + `OpLengthVector` (`:4010`); the sub-vector
  construction reuses the vector-build path (cross-check against how `[...]` literals
  build in `src/parser/vectors.rs`).
- Totality gate: add a coverage/`total` check in the vector handler's non-wildcard
  fallback at `control.rs:4141-4146` (today it silently returns typed null).

**Steps & verification.**
2.1 `name:pat` capture in element position. *Verify:* `35b-sequence-capture.loft`
    binds each named element, both backends.
2.2 `..rest` sub-slice capture. *Verify:* `rest` equals `subject[k..len-t]` by
    value **and length**; leak-clean on both backends (heap sub-vector must free
    correctly — assert store balance).
2.3 Totality gate + diagnostic. *Verify:* a sequence arm with no trailing `_`
    produces the "non-exhaustive — add a `_`" error (`tests/parse_errors.rs`).

**Exit:** parser-rule-shaped arms with captures + `..rest` work both backends,
leak-clean; totality enforced.

---

### Phase 3 — L3.7: multi-pattern arms (comma-separated patterns) — DONE

> **DONE (2026-07-11), both backends** — see the RESUME-HERE summary above. Scoped to the
> struct-enum/enum handler (`parse_match`): D-simple identical name sets at compatible types,
> shared capture slots, union exhaustiveness. Deferred to Phase 4 with clear diagnostics: partial
> name overlap (`option<T>`), a guard on a multi-pattern arm, and a field sub-pattern inside a
> non-first listed pattern. The cross-shape slice/vector multi-pattern (README's `[…]` + `Parsed{…}`
> example) needs a union subject type and is out of scope here.

**Goal.** One arm, several shapes; first match commits its captures.
```
match input { [Verb { v }, Obj { o }], Parsed { verb: v, object: o } => dispatch(v, o), … }
```

**Design.** Purely additive over Phase 1 (README: "no new cursor work"). Each listed
pattern compiles via `parse_pattern` as today; the arm becomes an OR over the
patterns' `cond`s, committing the matching pattern's `binds`. Names present in every
pattern with a common type bind at that type; names in only some become `option<T>`
(same rule as alternation — reuse the Phase-4 unify helper, or land D-simple: require
identical name sets in P3 and defer partial-overlap to P4).

**Code points.**
- Arm loop in `parse_match` `control.rs:2714-3133`: accept a comma-separated pattern
  list before `=>`; build the arm as nested `If(cond_i, {binds_i; body}, next_pattern)`.
- Exhaustiveness (F6): arm counts as total only if *every* listed pattern is total
  (`control.rs:3138-3156` coverage set — union across the listed patterns).

**Steps & verification.**
3.1 Two-shape arm, identical name sets. *Verify:* `35c-multi-pattern-arm.loft` binds
    from whichever shape matches; first-match commit (later patterns not tried) —
    both backends.
3.2 Exhaustiveness union. *Verify:* a multi-pattern arm that jointly covers an enum
    needs no `_`; a partial one does.

**Exit:** multi-pattern arms work both backends; first-match commit verified.

---

### Phase 4 — L3.2: alternation `(a | b)` + capture unification + slice cursor

> **4.1 + 4.2 DONE (2026-07-11), both backends.** Single-element alternation
> `[ (V1 { f } | V2 { f }) ]` in a slice element position (`parse_slice_alternation_element`,
> `control.rs`): a tag-disjunction condition + a conditional-offset capture read
> (`f = if tag==V1 { f@V1 } else if tag==V2 { f@V2 } … else null`) — the shared-slot idea
> nested into a slice element. **4.2:** a capture in only SOME branches promotes to
> `option<T>` (the untaken branches fall through the else-chain to null); a capture in every
> branch at a compatible type stays at that type. Enum tags are disjoint, so ordered choice
> is a disjunction — no cursor for single-element branches. Single-element alternation also
> COMPOSES with a following `..rest` and following elements at fixed positions
> (`[ (a|b), ..rest ]` and `[ first, (a|b) ]` both work). Guards:
> `tests/scripts/35f-alternation.loft` (incl. option promotion), `tests/parse_errors.rs::alternation_*`.
>
> **4.3 (open) — MULTI-element sequence branches `[ (A B | C), ..rest ]`.** The remaining
> case: a branch is a SEQUENCE of variable width, so a following `..rest` starts at a runtime
> position. Design decided below — **predictive dispatch, not blind backtracking.**

**Goal.** `[ (A B | C), ..rest ]` — a branch may be a multi-element sequence of variable
width; `..rest` picks up after whichever branch matched.

**Design — predictive dispatch on a forward `pos` cursor (decided 2026-07-11).** A
multi-element alternation `( A B | C )` over variant sub-patterns dispatches on the LEADING
element's tag — the way a recursive-descent parser predicts a production from its first
token — NOT a blind try-every-branch-then-revert. A branch's condition is PURE (tag tests
only; nothing mutates), so the whole alternation is an ordered `if / else if` over those
conditions: the first branch whose fixed-length tag sequence matches wins, binds its
captures, and advances a runtime `pos: integer` cursor by that branch's width. `&&`
short-circuit gives the first-lookup failure for free, and branches that share a leading
tag just fall one tag deeper — so no first-set-disjointness rule is needed and no
`save`/revert is emitted. `..rest` then reads `v[pos .. len − tail]` from the runtime `pos`.
(The no-tail shape `[ (A B | C) ]` is a zero-length rest — a strange edge real parsers
rarely use, so it is not the design centre.) All expressible as `If` + `Set` +
`OpGetVector` — **no new opcode**.

**The cursor contract — backtracking lives INSIDE the rule, not at the call site.** Every
rule (and the whole match) upholds one invariant about `pos`: it **CONSUMES on success**
(advances `pos` past what it matched) and **leaves `pos` UNTOUCHED on failure** (the
unmatched input stays available for whatever follows). Two consequences:
- An **alternation never saves/reverts** around its branches. A failed branch has ALREADY
  left `pos` where it started, so the next branch resumes from the right place. The
  ordered `if/else` is just "try each branch; the one that commits advances `pos`."
- The revert that makes a **composite rule** honour the contract — a *sequence* that
  matched its first parts then failed later — lives INSIDE that rule: it anchors its own
  start and restores `pos` before returning failure. That is the only place `save = start;
  … ; on fail pos = start` appears, and it is the rule's OWN responsibility, never the
  caller's.

A **tag branch** honours the contract for free: it tests its tags as a PURE boolean (reads,
moves nothing) and advances `pos` only on a full commit — so 4.3 emits no revert at all.
When the parser-combinator sub-rule layer ([SUBRULE-DESIGN.md](SUBRULE-DESIGN.md), PC1–PC5)
lands, a rule-INVOCATION branch just calls the rule and checks its matched flag; the invoked
rule's own body does the anchor/restore internally. **Keep the `pos`-advance factored so a
rule can own this contract** (advance-on-commit, restore-on-fail) without the alternation
knowing or caring how a branch matches.

**Code points.**
- Detect a multi-element branch at the `(` (parse-time): use `lexer.link()`/`revert()` to
  look ahead — a second variant before `|`/`)` means a sequence branch (whole-slice /
  cursor path); a lone variant is the single-element path already built in 4.1.
- Parse each branch as a SEQUENCE of variant sub-patterns; record per branch its width and,
  per position, `(disc, variant_def, fields)`.
- Emit the ordered `if / else if` over the pure per-branch conditions
  (`len − pos ≥ wᵢ && tag(v[pos])==… && …`), each arm advancing `pos` by `wᵢ`.
- Capture unification is the 4.2 rule: union the names; a name in every branch at a
  compatible type stays that type, a name in only some promotes to `option<T>` (untaken
  branches read null via the else-chain).  Reads use the runtime `pos` (`OpGetVector(v, sz,
  pos+j)`), not a compile-time index.
- `..rest` materialises `v[pos .. len − tail]` from the runtime `pos` var (today's `..rest`
  uses a compile-time `head_len` lo bound — swap it for the `pos` var when the head width is
  runtime-variable).

**Steps & verification.**
4.1 Same-name alternation unifies. *Verify:* `35d-alternation.loft` binds `n` at the
    unified type; both backends.
4.2 Different-name alternation promotes to `option<T>`. *Verify:* absent branch's
    capture reads null.
4.3 **DONE (2026-07-11).** Multi-element sequence branches + predictive dispatch —
    **§3a steps 1–5** (the `read(pos)` seam, the `pos` cursor, sequence-branch parse, the
    predictive `if/else`, `..rest` from `pos`; guard `tests/scripts/35g-multi-element-alternation.loft`).
    *Verify:* `[ (A B | C), ..rest ]` — a leading `A` commits branch 1
    and needs `B` next (else the arm fails, no silent fallthrough); a leading `C` commits
    branch 2; `..rest` picks up at the right runtime `pos` for EACH branch (assert `len(rest)`
    differs by branch width); both backends. Backtracking (`save`/revert) is NOT exercised
    here — 4.3 branches are pure tag sequences; the seam is left for the PC rule-invocation
    layer. Step 1 (`read(pos)` seam) is a byte-identical refactor gate BEFORE the new cases.

**Exit:** multi-element alternation with predictive dispatch + a runtime `pos` cursor +
capture unification, both backends, no new opcodes.  The `pos` cursor is factored so a
future rule-invocation branch can add `save`/revert without redesign.

---

### Phase 5 — L3.3: optional `(...)?` — DONE

> **DONE (2026-07-11), both backends.** An optional group `(a)?` in a slice pattern is a
> **degenerate alternation `(a | ε)`**: the empty ε branch always matches, so the optional
> NEVER fails — when `a` matches it binds + advances the cursor by `a`'s width; when it does
> not, the ε branch commits with the cursor UNMOVED and `a`'s captures read null. This fell
> straight out of the Phase-4.3 cursor engine with almost no new machinery — the whole
> feature is "push one empty branch."

**Design (as built).** `(a)?` desugars to the Phase-4.3 multi-element alternation with an
appended EMPTY branch (width 0, no tag test, predicate `true`). The predictive if/else already
handles it: tag branches are tried first, the always-true ε branch is the final `else`, so the
optional commits to ε exactly when `a`'s lead tag is absent. Captures are option-promoted by the
existing 4.2 hook (a name bound only in the non-empty branch → `option<T>`, null on the ε path).
`..rest` reads `v[pos..]` where `pos` is the matched branch width — `a`'s width when present, 0
when absent — so following elements pick up correctly either way. No anchor/revert is needed: the
cursor only advances on a full commit, so the ε path leaves it where it was for free (the cursor
CONTRACT).

**Code points (as built).**
- `peek_paren_suffix` — after a balanced `( … )`, returns the following token (`?` = optional).
- Head-loop `(` routing: `( … )?` (or a multi-element alt) routes to `parse_multi_element_alternation`.
- `parse_multi_element_alternation`: on a trailing `?`, `branches.push(Vec::new())` — the empty
  ε branch — with the rest of the predictive-dispatch / option-promotion / cursor-rest machinery
  from 4.3 reused unchanged. Both single-element `(A)?` and multi-element `(A B)?` work.

**Verification.** `tests/scripts/35h-optional.loft` (cross-mode, leak-checked): single-element
`(Kw)?` and multi-element `(Kw Op)?`, each with present / absent / partial-prefix / empty-input
cases; confirms option-promoted captures (null when absent) and that `..rest` resumes at the
right cursor for both the taken and the skipped path. PASS on both backends.

**Exit:** optional groups with nullable captures, both backends. ✅

---

### Phase 6 — L3.4: repetition + literals — 6.1/6.2 (rep + separator) + 6.3 (literals + `#lexeme`) DONE

> **6.1 DONE (2026-07-11), both backends.** A WHOLE-SLICE repetition
> `[ ( [name:] V )* [, ..rest] ]` / `…+` where `V` is one variant of the struct-enum element
> type: it matches the MAXIMAL leading run of consecutive `V`, `name` collects that run as a
> FRESH `vector<ElemType>` (whole elements, reusing the Phase-4.3 `..rest` materialisation),
> and a trailing `..rest` (named or a bare `..`) picks up the remainder.
>
> **6.2 (separator) DONE (2026-07-11), both backends.** `( [name:] V )*(Sep)` / `…+(Sep)`: the
> run becomes `V (Sep V)*`.  `Sep` is one variant, CONSUMED between elements but NEVER captured;
> `name` collects only the V's.  Because the V's sit at every other index (V Sep V Sep …), the
> capture reads `v[0..end]` with a STRIDE of 2 (`materialize_named_rest` gained a `step` param).
> The run-loop is `first V; loop (Sep V)` — a trailing separator is not part of the list (PEG
> `V (Sep V)*`), so `[Num, Comma]` leaves the `Comma` to `..rest` (or fails a no-rest arm).
> Parsed by `parse_repetition_separator` (the `(Sep)` after `*`/`+`). Guard:
> `tests/scripts/35j-repetition-separator.loft` (list / list+rest / `+` / trailing-comma /
> value-by-index proving separators are skipped; cross-mode + leak-checked).

**Design (as built).** A runtime **run-loop** (`Value::Loop` + `Break`) counts the leading
run into `end`: `end = 0; loop { if len <= end break; if tag(v[end]) != disc break; end += 1 }`.
The loop lives INSIDE the arm condition (it must run before the bindings materialise the
sub-slices) and the condition BLOCK yields the match boolean:
- `*` matches zero-or-more, so a no-rest slice matches only when the run IS the whole slice
  (`end == len`); with a rest it always matches (the rest absorbs the remainder);
- `+` additionally requires a non-empty run (`end > 0`, written `0 < end` — `>` has no Int form).

Then `name` (if present) materialises `v[0 .. end]` and `..rest` materialises `v[end .. len]`,
both via the shared `materialize_named_rest`. Predictive on the tag; no backtracking (the tag
test is a pure boolean, the cursor only moves forward).

**Code points (as built).** `peek_group_kind` (the ONE unified slice-`(`-group look-ahead —
`Other` / `Alt` / `Repetition`, decided in a single save/revert; two sequential peeks over the
same region corrupt the lexer replay buffer) → `SliceGroupKind`; `parse_slice_repetition`
builds the run-loop + condition + captures. Routing computes `group_kind` ONCE per element.
Guard: `tests/scripts/35i-repetition.loft` (`*`/`+`, with-rest / no-rest / bare-`..`, run stops
at the first non-`V`, value-by-index; cross-mode + leak-checked).

**Surfaced (pre-existing, filed):** iterating a match-arm-materialised vector (`..rest` OR a
repetition capture) with `for e in x { … }` runs ZERO times on **native** (`len`/indexing are
correct) — **#556**. So 35i verifies collected values by slice-INDEX, not `for`. This limits
Phase 6's collected vector exactly as it already limits the shipped `..rest`.

**6.3 (literal slice elements) DONE (2026-07-11), both backends.** A LITERAL element `[ 1, … ]`
/ `[ "kw", … ]` matches by EQUALITY against `v[pos]`.  On a SCALAR element it is a direct `==`
(`peek_is_slice_literal` + `conv_op("==")`); a `"_"` placeholder keeps the position count + length
gate aligned, so it composes with binds / `..rest`.  Guard `tests/scripts/35l-literal-slice-elements.loft`.

**`#lexeme` extension DONE (2026-07-11), both backends — the grammar-reader keystone.** A token-enum
variant marks the field carrying its surface text with `#lexeme`
(`enum Token { Keyword { #lexeme name: text }, Punct { #lexeme sym: text }, Ident { name: text } }`),
so on a STRUCT-ENUM element a bare string literal matches against it: `"fn"` stands in for
`Keyword { name: "fn" }` and reads like the grammar.  It desugars (`build_lexeme_literal_match`) to
an OR over the eligible variants of `tag(v[pos]) == disc && v[pos].<lexeme> == "fn"` (the field read
guarded by the tag test).  `#lexeme` is an `Attribute` parse-time flag (not serialised — patterns
desugar in-session).  A non-`#lexeme` field (e.g. `Ident.name`) is matched structurally, so an
`Ident` whose text is `"fn"` is NOT the keyword.  Guard `tests/scripts/35k-lexeme.loft` (a tiny
statement grammar), `tests/parse_errors.rs::{lexeme_missing, unknown_field_annotation}`.

**Deferred to a follow-up (with the prerequisites they need):**
- ~~**The scalar golden form** `[ 1, args:integer* ]`~~ — **DONE (slice 1).** Scalar bare-postfix
  `name:Type*` / `+` repetition + the single `name:Type` capture (see § RESUME HERE); guard
  `tests/scripts/35f-repetition.loft`. The `#lexeme` token-grammar form remains the more idiomatic
  loft path for struct-enum streams.
- ~~**Per-iteration field capture** inside the body `( V { n } )* → n: vector<T_n>`~~ — **DONE
  (slice 2).** Scalar/text field projection (see § RESUME HERE); guard
  `tests/scripts/35n-field-projection.loft`.  A heap-payload field projection is still deferred
  (rejected with a diagnostic).
- ~~**Mid-slice repetition** (a non-empty head before the group)~~ — **DONE** (the parser-combinator
  keystone `[ Ident { name }, "(", (arg: Ident)*(","), ")" ]`; a repetition may sit between a fixed head
  and a fixed literal/lexeme tail). Guard `tests/scripts/35m-mid-slice-repetition.loft`.
- **§3a step 6 fold hook** (associativity fold direction) — waits on the PC layer that uses it.

*(6.2 separator `*(Sep)` — DONE, see above.)*

**Exit (full):** repetition with separators + vector capture, both backends, leak-clean.

---

### Phase 7 — L3.6: iterator inputs

**Goal.** `match some_iter { … }` with backtracking over a non-random-access source.

**DESIGN REVISION (2026-07-11, user-directed) — the CURSOR is the clear object.**
The original design below (add `OpMatchAnchor`/`OpMatchRevert` + a `State` memo) is
superseded. A design-protocol probe FALSIFIED its load-bearing claim ("the backing
swaps in without touching match logic"): the match machinery has **11 `len`-based
bounds** (`OpLengthVector`) that don't translate to an unbounded iterator — a raw
backing swap is impossible. The user's steer resolves it: **streaming must NOT
influence the parser; the parser works against ONE clear object regardless of how it
is fed.** So:

- Define a **Cursor** abstraction — the ONLY thing the match engine touches:
  `read(pos)` (element there), `has(pos)` (is there an element there? — REPLACES raw
  `len`), and `anchor`/`revert` (backtracking). No `OpGetVector`/`OpLengthVector`/
  coroutine `next` in the match logic.
- Two implementations BEHIND the interface, indistinguishable to the parser:
  1. **Vector cursor** (random access): `read = v[pos]`, `has = pos < len`,
     anchor/revert = save/restore an index (the F2 "no new op" note).
  2. **Streaming cursor**: `read(pos)` pulls the coroutine via `next()` and buffers;
     `has(pos)` pulls until buffered-or-exhausted; anchor/revert = the lexer's
     `memory`/`link`/`revert` GENERALIZED (evict when no anchor is live). This IS the
     lexer's mechanism as a coroutine cursor — the buffer = `Lexer::memory`, `anchor`
     = `link`, `revert` = `revert` (the `cont()` replay-buffer bug fixed this session).
- The `len` falsification dissolves: the 11 bounds become `has(pos)` queries both
  impls answer. The one genuinely length-dependent case — tail-from-the-end (slice 3)
  — needs the stream exhausted to locate the end, so tail-on-a-stream is inherently
  bounded (finite: fine; unbounded: reject / `max_lookahead`-gate).  Streaming is an
  IMPLEMENTATION of the Cursor, not a rewrite of the parser.

**Phased build.**
1. **DONE (`d3438259`)** — Cursor len-seam (`cursor_len` beside `read_slice_elem`), the
   11 `len`-bounds routed through it, byte-identical both backends (Mode-B corpus).
2a. **DONE — streaming match, SCALAR elements, EAGER (both backends).** `match <iterator<Scalar>>`
   materialises the coroutine into a buffer `vector<Scalar>` (`collect_iterator_subject` in
   control.rs: `gen = subject; done = false; buf = []; while !done { x = OpCoroutineNext(gen);
   if OpCoroutineExhausted(gen) { done = true } else { <append triple> } }`) then runs the existing
   `parse_vector_match` over `buf` — streaming stays behind the seam, match logic untouched.
   Pull uses explicit `next`/`exhausted` (a `for` over a stored coroutine HANGS).  **The 2 opcodes
   evaporated** — the buffer holds all pulled items, so backtracking is a free index (no anchor/
   revert, no eviction).  text / vector / tuple elements ride a different `next` channel → deferred
   with a clean diagnostic (collect-idiom hint).  Guard `tests/scripts/35p-iterator-match.loft`,
   `parse_errors::stream_match_complex_deferred` (retired 2026-09-29: every element type streams
   since the buffer appends through `append_element_ops`, the comprehension's own append —
   loft#1737, `tests/scripts/a-match-streams-an-iterator-of-any-element.loft`).
2b. **DONE (text + struct-enum element channels).** `iterator<text>` (token strings) and
   `iterator<StructEnum>` (struct-enum token streams) now stream-match on both backends, leak-clean.
   The one fix: the append record var (`stream_elm`) needed `skip_free` — without it scope cleanup
   emitted `OpFreeRef` after the append and freed the just-stored record (harmless for an inline
   int, but it freed the STRING for a `text` element — the null-value bug).  Supported set now:
   scalar / text / struct-enum; plain enum / vector / tuple ride a different `next` channel (still
   gated).  Guard broadened in `35p-iterator-match.loft`.
2c. **DEFERRED by decision (dogfood).** LAZY per-read pull + per-match `max_lookahead`.  Assessed:
   the lazy read alone buys nothing because almost every pattern queries `cursor_len` (the fixed
   gate, the `end == len - tail_len` gate, `..rest`), which on a stream must exhaust — so it also
   needs the 11 `len`-bounds reframed to `has(pos)`, a large second refactor whose only payoff
   (matching a bounded pattern over an INFINITE source) has no consumer.  The infinite-iterator
   hang is already caught by `loft --timeout`.  Documented in CAVEATS.md § "@PLN35 Phase 7 —
   streaming `match` … is EAGER".  Build the lazy path when a real consumer needs unbounded-stream
   backtracking.

**Phase 7 is functionally COMPLETE for real use** — `match <iterator<scalar|text|struct-enum>> { … }`
works on both backends, leak-clean, streaming stays behind the Cursor seam, and the plan's 2 opcodes
were proven unnecessary.  What remains of @PLN35 overall is the **PC1–PC5 sub-rule layer**.
3. (folded into 2) — no separate opcode/State work needed.

**Feasibility confirmed:** loft coroutines expose explicit `next(gen)` / `exhausted(gen)`
(not just `for`), so a streaming cursor can pull incrementally; a coroutine is
forward-only, and `revert` replays the buffer (never rewinds the source) — exactly the
lexer, so no coroutine re-entrancy is needed.

---

**Original design (superseded — kept for the add-opcode mechanics reference).** An
iterator cursor can't reset an index; it must **buffer pulled items** while an anchor
is live — exactly `Lexer::memory` + the `links` refcount
(`src/lexer.rs:104,106,1371,1385`). Add:
- `OpMatchAnchor` / `OpMatchRevert` opcodes.
- `anchors: Vec<(u32 pos, u32 epoch)>` + a pulled-item memo on `State`
  (`src/state/mod.rs`, modeled on `call_stack`/`coroutines` at `:164,170`), mirrored
  in native (`src/codegen_runtime.rs`, since native ops can't reach `State`).
- A `max_lookahead` arm attribute bounding the buffer (README L3.6), so an infinite
  iterator with an always-matching body can't hang.

**Add-opcode checklist** (from the code map; the *only* new-op work in the plan):
| # | Touch point | File | Action |
|---|---|---|---|
| 1 | Define | `default/01_code.loft` (near `:83-96`) | `fn OpMatchAnchor(...);` + `fn OpMatchRevert(...);` with `#rust"…"` templates. `Op`+uppercase name = auto-registered (`src/data.rs:2847`); op_code auto-assigned at parse (`src/parser/definitions.rs:1286`). |
| 2 | Interp impl | `src/fill.rs` (@generated) | **`make fill`** regenerates from the templates; CI `tests/issues.rs::fill_rs_up_to_date` guards drift. Never hand-edit. |
| 3 | Compile/emit | `src/state/codegen.rs:2986` (generic op path) | Automatic *iff* standard stack effects. These ops touch the anchor shadow stack → add a special-case arm modeled on `OpCoroutineYield` at `codegen.rs:2887`. |
| 4 | Native | `src/generation/ops/mod.rs:148` (`build_registry`) + `src/codegen_runtime.rs` | Templates that touch `State` aren't native-valid → register custom `OpEmitter`s + native runtime helpers (model on `OpFreeRef` at `codegen_runtime.rs:353`). |
| 5 | Native-gate | `src/native_gate.rs` | If the memo can't be native at all, model iterator-match as a denylisted `Value` variant (like `Value::Yield` at `:243`). Prefer keeping it native. |

**Steps & verification.**
7.1 Opcodes + shadow stack, interp first. *Verify:* `35g-iterator-match.loft` matches
    over an `iterator<T>` with backtracking; buffer replays pulled items on revert.
7.2 Native parity. *Verify:* identical result `--native`; `make fill` fresh; clippy
    clean; native gate correct.
7.3 `max_lookahead` bound + termination. *Verify:* an always-matching body over an
    unbounded iterator errors at the bound instead of hanging (`loft --timeout`).
7.4 Side-effecting-iterator + infinite-repetition are documented limitations
    (README "What would not work") — add a `CAVEATS.md` note.

**Exit:** iterator-input matching both backends, bounded, leak-clean; the two opcodes
freshness-checked.

---

## 5. Cross-cutting verification & docs

- **Cross-mode harness.** Every `35*.loft` runs on `--interpret` and `--native`
  under the leak gate (see loft-test skill). Parser diagnostics go in
  `tests/parse_errors.rs`.
- **Rust unit tests** where a helper deserves isolation (e.g. `..rest` sub-slice
  math, capture-type unify).
- **Docs synced per phase:** `LOFT.md` § Match (syntax), `INTERMEDIATE.md` (the two
  L3.6 opcodes only), `CAVEATS.md` (iterator limits), `INCONSISTENCIES.md` #26
  (totality precedent), and this plan's Status table.
- **Freshness:** `make fill` after Phase 7; `make ci` green each phase; the only
  expected non-green is the known-flaky `wasm_debug_relay`.

## 6. Risk register

| Risk | Phase | Mitigation |
|---|---|---|
| D2 wrong (slice backtracking *does* need an op) | 0 | Falsification probe 0.2 **before** any feature code. |
| `parse_pattern` refactor regresses live match | 1 | loft-codegen gate: byte-identical IR/native proof (1.1) before adding cases. |
| Heap captures (`..rest`, repetition vec) leak or diverge across backends | 2,6 | Assert store balance + length on both backends; preserve `Deps::frame1` borrow-dep rewrite (`control.rs:3490-3508`). |
| Repetition loop-IR construction harder than assumed | 6 | Verify the `for`/`while` IR constructor in `collections.rs` at phase start; if costly, land `*`/`+` as a bounded unroll fallback. |
| Iterator memo native parity | 7 | Interp-first, then native; custom `OpEmitter` + `codegen_runtime` helper; native-gate fallback if truly non-native. |

## 7. Open questions (carry from README, decide at the phase that needs them)

1. Explicit PEG commit points (`~`/`!~`) — deferred; revisit if deep-revert errors
   are unhelpful (Phase 4).
2. Longest-partial-match error reporting — design target Phase 4, may slip.
3. Partial-name-overlap unification in multi-pattern arms — P3 requires identical
   name sets; full overlap rule lands with P4's unify helper.
4. Positional tuple-variant sugar — PERMANENTLY out ([C89](../../DESIGN_DECISIONS.md)), never
   promoted: tuple variants force match-to-read + a mitigation-syntax sprawl; loft reads by name.

## 8. See also

- [README.md](README.md) — design/rationale (the "why").
- loft-codegen skill — the IR-identical refactor gate (Phase 1) + add-opcode method (Phase 7).
- `src/parser/control.rs` — the entire pattern surface lives here.
- `src/lexer.rs` §`link`/`revert` — the anchor primitive L3.6 mirrors.
