<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 153 — Null rules: one home each, and the refusal at the point every escape ends up

Tracker: [@PLN153](https://github.com/loft-lang/plans/issues/153).

## Status

**Status — CLOSED 2026-09-08.**  All six phases ran.  Phase 6 re-measured the class and the
answer is **NO EFFECT** — null/sentinel 14.1 % → 37.3 % (→ 25.9 % with this plan's own screen
finds removed; it rose either way), the third consecutive no-effect verdict for this class.  The
Done criterion is met in its SECOND form: the residual names a mechanism this plan did not touch
— *the nullable spelling reaches the store machinery by a different road from its dense twin* —
and it is filed as **@PLN160**.

What shipped: the null rules have one home each (phases 1–2), the `(N-Store)` refusal sits at one
point (phase 3, `Parser::nstore_unwrap_report`, now a registered keystone in
`scripts/bug-review.py`), and `@FR-N-Shape` plus its ratchet (phase 4 batch 11) turned the
1520-site walk into standing enforcement instead of an unfinishable queue.

What was DECLINED rather than finished: **phase 5's citation-convergence tail**.  Converging
`@FR-L-Null`'s citations is the same activity the bug review has now scored NO EFFECT three
times — *"a rule gains nothing by being pointed at from more places"* — so the remaining 42
citations are not work this plan hands on.  Phase 4's function-by-function tail is likewise
closed by CONVERSION, not by completion.

One open debt, tracked as an issue and not as a phase: **loft#1450**'s remaining leg (a
projection through a nullable receiver drops the `?`), re-measured at 115 sites and written up in
[batch11-shape-rule.md](batch11-shape-rule.md).

The null MODEL is decided and was not reopened here: @PLN102's
[keystone](../102-stability-contract/keystone-null-model.md) chose **B** (an in-band sentinel
for scalars, out-of-band absence for references and for a struct stored inline), frozen in
[DESIGN_DECISIONS.md § C90](../../DESIGN_DECISIONS.md), with @PLN25 (the dense element
model) and @PLN116 (`x?`) shipped on top of it.  What this plan changes is the rules'
REPRESENTATION IN CODE: of the 18 `@FR-N-*` rules in [types.md](../../formal/types.md) three
are cited (7 sites); the four `@FR-L-Null*` rules in [layout.md](../../formal/layout.md) are
cited from 60 sites that the rule-led walk CITED without converging — and the bug review
scored that walk **no effect**: null/sentinel rose from 17.9 % to 25.0 % of filed bugs
([BUG_REVIEW.md](../../BUG_REVIEW.md), 2026-08, third row).  Citing is the receipt, not the
conversion.

## Goal

Every `@FR-N-*` and `@FR-L-Null*` rule names ONE predicate or emitter that decides it, every
site that asks the question reads that home, and a value of a non-null type that would be
observably null is REFUSED at the one point every escape ends up — the store into a `τ` slot.
Done when the null/sentinel share of filed bugs falls in the bug review after the plan's
watermark, or the residual is characterised as a different mechanism.

## Effort + design

- **Effort:** H — six phases, each an afternoon to a day, none straddling a PR.
- **Design:** ~ — the model is decided (C90); the homes are to be located, and phase 3's
  chokepoint is the one design call (§ Open design questions).
- **Value category:** S (silent failure).  A sentinel that collides with a real value, or a
  `τ?` that reaches a `τ` slot undischarged, answers wrong with nothing said.
- **Last touched:** 2026-09-05.

## Why null is the hard family, measured

The 2026-09-05 evaluation of the rule-led walk queue (QUALITY.md B7q–B7s and the report that
followed) put null apart from every other family for three structural reasons:

1. **One fact, two representations.**  Nullability is a type former (`Optional`, the N
   rules) and a layout sentinel (the L-Null rules).  Every question is asked twice, at
   different levels, and no predicate answers both.
2. **No chokepoint by construction.**  `τ?` is discharged at the point of use — `??`, `?`,
   `match`, or a store into a non-null slot — so the question is asked wherever a value is
   read.  `scripts/ir_walker_audit.py optional` measures it: **703** functions discriminate on
   a `Type` variant and **352** of them are opaque to a wrapped shape (QUALITY.md B6p, B6s).
   The `?` survives at DECLARATION time and at LVALUE places; both defects of the B7h walk
   sat exactly there.
3. **A family the code does not represent.**  Fifteen N rules have no citation, so the walk
   method has nothing to split.  A rule with zero sites is not walked; it is first located.

The precedent that says what moves such a class is generics: a REFUSAL at the one point every
escape ends up (the type-variable row at allocation) took its share from 15.6 % to 4.0 %,
where folding onto a fact and citing did not.  Null has no such point today; phase 3 builds it.

## Composition matrix — Stage A

The axes this plan's cells vary, written as `/tmp` probes on `--interpret` first and graduated
to `tests/scripts/` as each phase's regression suite; every cell green on both backends:

| axis | domain |
|---|---|
| type kind | `integer` · narrow (`u8`…`u32`, `i8`, `i16`) · `float`/`single` · `boolean` · `character` · `text` · reference · struct stored INLINE (`__nullable<S>`) · `vector<τ>` · keyed |
| position | local · field · element · tuple member · argument · return · capture · literal |
| spelling of nullability | declared `x: τ?` · inferred join (`N-Join`) · `v[i]` (`N-Index`) · `a / b` (`N-Div`) · `e as τ?` (`N-Cast?`) · a library return typed `τ?` |
| discharge | none · `?? d` · `x?` · `match null` · `== null` test · store into a non-null slot |
| backend | `--interpret` · `--native` |

`python3 scripts/matrix_axes.py file <guard.loft>` is run on every guard this plan writes;
the axes it reports unreached are the cells still to build, not a note.

## Sub-arcs — small steps, each able to go red on its own

`Verify` names the comparison that would go RED if the phase were done wrong.

| # | Item | Verify | Status |
|---|---|---|---|
| **0** | **Probe first.**  Is `τ??` constructible today (`N-Idem` — `Type::optional` in `data.rs` is the candidate home), and is `N-Intro` the only null-direction conversion in `⤳`?  A corpus census with an env-gated report, since `[profile.dev.package.loft]` compiles a `debug_assert` OUT of the library. | The report over the 1247-file corpus reads 0 for `τ??` — and a probe that constructs one by hand makes it read 1, so the zero is not vacuous. | **Done** 2026-09-05 — § Phase 0 result |
| **1** | **Census: one home per N rule.**  For each of the 18, the predicate or emitter that decides it today.  Candidates: `N-Coal` → `parser/operators.rs::build_null_coalesce_default`; `N-Default` → the `x?` lowering + `Data::has_default`; `N-Store`/`N-Decl` → the `(N-Store)` teeth (`keys.rs`: the DN3 gate, the call-arg gate, the heap gate); `N-Prop` → `nullflow_enabled()`; `N-Join` → the inferred-assignment join; `N-Match` → the null arm; `N-Div`/`N-Arith`/`N-Cast`/`N-Cast?` → operator typing ([float-null-domain-typing.md](../102-stability-contract/float-null-domain-typing.md)); `N-Dense` → element storage; `N-Parse` → folded into `N-Cast`; `N-Index`, `N-Reserve`, `N-Store` already cited. | Per rule, ONE probe pair on both backends — a program where the rule must hold and one where its negation must be refused — green BEFORE the `@FR-` citation is added.  `rule_tags.py check` then reports 18/18 cited; a rule whose only evidence is the citation is the B6u failure and does not count. | **Done** 2026-09-05 — § Phase 1 census |
| **2** | **`N-Prop` has 10 `nullflow_enabled()` sites in 4 files.**  Which question does each ask — propagate, gate, or warn?  Fold the fact-reading half onto one predicate; the per-site residue stays where it is per-site. | `introspect` output (IR, bytecode, Rust) byte-identical over the 1247-file corpus against the committed compiler (the B7r/B7s method), under the default AND under `LOFT_NO_NULLFLOW=1`. | **Done** 2026-09-05 — § Phase 2 fold |
| **3** | **The `N-Store` refusal at ONE point.**  Today the teeth sit at the local slot, the field, the return, the index, the call argument and the heap half as separate gates, each a spelling; a nullable reaching a non-null slot through a position none of them covers is answered wrong in silence.  One check where every store passes — the `⇐` lowering, whose ten push sites and six admission lists B6t already measured — is the chokepoint. | The Stage A matrix, position × type kind × discharge, with an `@EXPECT_WARNING` / `@EXPECT_ERROR` cell wherever a nullable meets a non-null slot undischarged and a silent cell wherever it is discharged; `make falsify` against the current build names the cells that pass silently today. | **Done** 2026-09-05 — 3a, 3b, 3c (§ Phase 3a–3c) |
| **4** | **The `optional` screen, ranked.**  The 352 opaque functions ordered by *can an undischarged value reach here*: declaration reads (a field's, a local's, a return's declared type) and lvalue places first, use-path sites last.  Each function in the top tier either peels through `base()` or is shown unreachable by a probe cell. | The gated `optional` audit row in QUALITY.md moves DOWN and every moved function has a cell; a peel added with no cell is the B6u receipt and is not counted. | **CLOSED by CONVERSION** — 11 batches, the row 359 → 338; batch 11 measured the real population (1520 TESTS, not 338 functions), wrote `@FR-N-Shape` and put a ratchet on it.  The remaining sites are gated, not queued |
| **5** | **`@FR-L-Null`'s 45 citations converged.**  The two questions B6u split — *what value means absent in this storage?* (the per-type sentinel table frozen in C90, `Stores::is_null`) and *is this the same storage?* (`base()`) — each have a home; every citation either reads it or is removed as a redundant spelling. | The site count goes DOWN, and where a change is a fold the emission is byte-identical over the corpus; where it is a fix, a probe cell. | **DECLINED** after 2 batches — the bug review has scored citation-convergence NO EFFECT three times; the remaining 42 are not work this plan hands on |
| **6** | **Re-measure.**  `make bug-review` on the null/sentinel class after the plan's watermark against the window before it. | The class falls, or the residual names a mechanism this plan did not touch and a follow-on plan is filed for it. | **DONE** 2026-09-08 — NO EFFECT (14.1 % → 37.3 %, → 25.9 % net of this plan's own finds); residual characterised and routed to @PLN160 |

## Phase 0 — result (2026-09-05)

**`τ??` is not constructible from a loft program, and the corpus reads 0 with an instrument that can read 1.**

*The former.*  `Type::optional` (data.rs:1781) is `(N-Idem)`'s home: `Optional(Optional(τ))
→ Optional(τ)`, and `Optional(Never | Null)` stays what it was.  The type parser's postfix `?`
(definitions.rs:2606) builds through it, and `pln25_optional_enabled()` is default-ON, so the
spelling `integer??` cannot nest by syntax.

*The routes around it.*  Thirteen sites construct `Type::Optional(Box::new(…))` directly
(`grep -rn "Type::Optional(Box::new" src/`).  Eleven are structure-preserving rewrites
(`rewrite_type_opt`, `rewrite_unknown`, the IR deserialiser, two tests) or wrap a provably bare
inner — a fresh `Text`, a fresh `Vector`, a `.base()`, a `Reference` just built.  ONE can nest:
`typedef.rs:184–191` peels a field's `?` off an `Unknown` forward reference, resolves the stub
to the alias's `returned`, and re-wraps (`set_attr_type_keeping_optional`, :233).  If the alias
resolves to `τ?` the field becomes `τ??`.  The hand-built probe for that route is
`struct S { f: Maybe? }` with `type Maybe = integer?` declared AFTER it, measured:
**it built one.**  The first write to the field was refused with the type spelled out — *"Cannot
assign to field 'f' of type integer??"* — and the same through a chain of two forward aliases.
Fixed on the spot: the re-wrap now goes through `Type::optional`, so the route is idempotent
like every other (`typedef.rs:233`, cited `@FR-N-Idem`).  Guard
`tests/scripts/153-a-forward-alias-field-keeps-one-optional.loft` (two route cells, one
declared-before control).  The per-probe census had read 0 for this program before the fix —
VACUOUSLY, because the program was refused before `scopes::check` ran and the three census
lines it printed were the stdlib loads'; running the probe for its own output is what showed
the type.  The sweep counts a refused file as `failed`, never as a 0, for exactly this reason.

*The census.*  `src/null_census.rs` — an observer beside `ownership_cfg::oracle` in
`scopes::check`, gated on `LOFT_NULL_CENSUS`, walking every declared return, attribute and
variable type through `Type::for_each_child` and counting an `Optional` directly under another.
Env-gated rather than a `debug_assert` because `[profile.dev.package.loft]` compiles those OUT.
`scripts/null_census_sweep.sh` runs it over the corpus: **`TOTAL nested=0 files=1258 failed=0` — every file reached `scopes::check`, none carried a `τ??`**.
Non-vacuity: `null_census::tests::a_hand_built_nested_optional_reads_one` — a hand-built
`Optional(Optional(Integer))` reads 1, at depth through a `Vector` still 1, a triple reads 2.

**Three resolution defects on the same route, found by the guard, not by the census.**  Each
is one fact held in two places, which is the plan's thesis met on its first afternoon:

- **F1 — a pass-1 operator error on a wrapped stub, never retracted.**  Pass 1 defers an
  operator whose operand is unresolved (`parser/mod.rs::…`, *"a genuinely unresolvable operand
  reaches this same site on pass 2"*), but the test was `Type::is_unknown`, which sees through
  `Vector` and nothing else, so `Optional(Unknown(alias))` — a forward alias behind a `?` —
  was judged settled, resolved against `unknown?`, and refused with *"No matching operator
  '==' on 'unknown?' and 'integer'"* for a program pass 2 resolves.  Fixed with
  `Type::has_unknown`, a walker over the `Type` keystone (exhaustive by construction), used at
  the deferral; `is_unknown` keeps its meaning at the settledness guards, where a wrapper over
  a stub IS a written type that must not be overwritten (91 callers, none touched).
  Measured: the nullable-reference alias `f: MP?` under `!=` was the same defect.
- **F2 — an annotated local's `?` overwritten by its first assignment.**  `x: Maybe? = 5` with
  `Maybe` declared below: the declaration is `Optional(Unknown)`, the assignment's
  `change_var_type` found no arm for a declared type carrying a stub and fell through to the
  unconditional write, the slot read `integer`, `resolve_unknown_stub` found no stub left to
  fill, and pass 2's `integer?` was refused as *"cannot change type from integer to
  integer?"*.  `LOFT_LOG=type_timeline:x` showed all three writes.  Fixed with the mirror of
  the existing loft#1073 arm: a declared type with an unresolved component is a placeholder,
  not a baseline, and is kept for the stub rewrite to fill.
- **F3 — a forward nullable-reference alias field lints as 'not null'.**  `struct S { f: MP }`
  with `type MP = P?` below: storage is the tagged `__nullable<P>` and `S { f: null }` reads
  null back correctly, so the VALUE is right — but `redundant-null-check` fires on `s.f != null`
  because it reads `matches!(ctp, Optional)` on the field-read type, and for the forward case
  that type is the STORED spelling (`__nullable<P>`), not the source spelling
  (`Optional(Reference(P))`) the declared-before case carries.  A wrong diagnostic, not a wrong
  value; it is the stored-vs-source spelling question `(L-Null-Which)` names, so it is phase 5's
  first cell rather than a spot fix here (probes `f3`, `f7`).

Guard: `153-a-forward-alias-field-keeps-one-optional.loft` — c1/c2 (the `τ??` route and a
chain), c3 (declared-before control), c4 (the annotated local, F2); F1 is what lets c1/c2 use
`==` at all.  ⚠ The first cut of c2 "passed" through interpolation — `"{s.f}"` formats an
`unknown?` value at runtime without type-checking it — and only `==` exposed the stub; a cell
that reads a value must read it through an operator the type checker owns.

**`(N-Intro)` is NOT the only null-direction edge in the code's `⤳`, and that is phase 3's
premise stated by the compiler.**  `Parser::convert` (parser/mod.rs:4389) has both directions:
- `should = Optional(inner)` recurses on the base — `τ ⤳ τ?`, `(N-Intro)`.  ✓
- `is_type = Optional(inner)` ALSO recurses — `return self.convert(code, inner, should)` — the
  implicit `τ? ⤳ τ` UNWRAP the rules say does not exist (`types.md`: *"there is NO implicit
  `S? ⤳ S`"*).  Its own comment says why: `convert` services comparisons (`x == null`) too, so
  the `(N-Store)` teeth *"must live at the STORE / decl / index sites"*.  So the rule is
  enforced by the ABSENCE of a refusal at every site that is not a store — the scattered-teeth
  shape phase 3 exists to replace, and the reason a position no gate covers answers wrong in
  silence.
- `implicit_checked_narrow` admits `Integer[s] ⤳ τ?` for a NARROW nullable target as a
  NULL-PRODUCING conversion (the value becomes null when it does not fit) — `(N-Cast?)` made
  implicit, not `(N-Intro)`.  It gets a cell of its own in phase 3's matrix.
- The `__nullable<S> ⤳ S?` arm is a representation change under `(L-Null-Tag)`, null-preserving
  in both directions; not a null-direction edge.

**What this settles for the phases after it.**  Phase 1's `N-Idem`/`N-Opt` home is
`Type::optional`, with this census as their evidence — citing them there is honest because the
zero was measured.  Phase 3 starts from `convert`'s unwrap arm, not from the store sites: the
question is which sites may keep the unwrap (comparison, `match`, `??`'s subject) and which
must refuse it, and the answer is a list, not a per-site flag.

## Phase 1 — census: one home per N rule (2026-09-05)

`Home` is the predicate or emitter that DECIDES the rule today (line numbers at 3688a90a).
`Pair` is the probe pair the citation waits for: HOLD = a program where the rule must hold,
REFUSE = one where its negation must be refused (a diagnostic, or a census that reads 0).
A rule whose negation is not a compile-time refusal says how its negation is measured instead.

| rule | home (candidate) | already cited | pair |
|---|---|---|---|
| N-Opt | `Type::optional` data.rs:1781 | — | HOLD `x: τ? = null` for each type kind · REFUSE none (every τ admits `?`; the value-struct refusal at definitions.rs:2586 is the one exception and is `(L-Null)`'s, not this rule's) |
| N-Idem | `Type::optional` data.rs:1781 | — | HOLD the phase-0 census reads 0 over the corpus · REFUSE the hand-built `Optional(Optional)` reads 1 (`null_census::tests`) |
| N-Dense | `typedef.rs::nullable_vector_elem` :133 + `synth_nullable_target` :160 (the `__nullable<S>` synthesis) | — | HOLD `vector<S>` element reads non-null, `vector<S?>` element may be null · REFUSE `v: vector<S> = []; v += [null]` is an N-Store warning |
| N-Intro | `Parser::convert`, the `should = Optional(inner)` arm mod.rs ~4459 | — | HOLD `x: integer? = 5`, `f(5)` into `p: integer?`, `return 5` from `-> integer?` · REFUSE — the REVERSE edge: `y: integer = x?` warns (that is N-Store's teeth; the citation records that `convert` carries both edges) |
| N-Index | vector.rs:449, operators.rs:638 | ✓ 2 | (cited; keep) |
| N-Div | operators.rs:3913–3918 `div_nullable` (+ `divisor_provably_nonzero`) | — | HOLD `a / b` is `integer?`; `x / 2` is `integer` · REFUSE `c: integer = a / b` warns |
| N-Arith | operators.rs:3890 range-tracking | — | HOLD `a + b` on non-null operands is non-null `integer` · REFUSE — none (the rule is a typing, its negation is N-Prop's) |
| N-Cast | operators.rs:3455 (`as τ` is an assertion; `text as τ` folded in = N-Parse) | — | HOLD `"12" as integer` is `integer` · REFUSE `"x" as integer` errors (a parse cannot be asserted) |
| N-Cast? | operators.rs:3116 DN4 `e as τ?` lowering (not a taggable name: cited under N-Cast) | — | HOLD `"x" as integer?` is null; `300 as u8?` is null · REFUSE — none |
| N-Parse | folded into N-Cast (types.md:149) — same home | — | covered by N-Cast's pair |
| N-Coal | operators.rs:2234 `coalesce_not_null` + :2435/:2450 `build_null_coalesce_default(_inner)` | — | HOLD `(e ?? d)` is `τ` non-null · REFUSE (measured) `c: integer = a ?? b` with `b: integer?` is refused — a `τ?` default makes the coalesce `τ?`, exactly `Γ ⊢ d ⇐ τ` |
| N-Default | operators.rs:2908 `x?` lowering + data.rs:6296 `Data::has_default` | — | HOLD `x?` on `integer?` is `integer` · REFUSE `e?` on a bare-enum `E?` (no default) errors (data.rs:6346) |
| N-Match | control.rs:463 `arm_body_is_null` + :11039 `tail_if_has_null_arm` | — | HOLD `match e { null => …, x => … }` binds `x: τ` · NEGATION (measured) `match n { x => … }` on `integer?` with NO null arm is SILENT: `x` binds `τ?` and `x * 2` propagates null — no refusal exists; the rule presupposes the null arm, so this is a phase-3 matrix cell (a `match` that does not discharge), not a deviation |
| N-Store | control.rs:1231/1303 ✓ + mod.rs:4192–4270 (warn/error split, two branches) + keys.rs `pln25_dn3_enabled` / callarg / heap gates | ✓ 2 of ≥5 | HOLD `y: integer = x ?? 0` silent · REFUSE `y: integer = x` warns; `u: u8 = x` errors (narrow) — MULTI-HOME → phase 3 |
| N-Decl | the same store site (mod.rs:4192) via the declared slot's type | — | HOLD `x: integer = 2; x = 3` · REFUSE `x: integer = 2; x = v[i]` warns (types.md:159 row) |
| N-Join | variables/mod.rs:2197–2224 `change_var_type(N-Join)` (DN6 widen) | — | HOLD `a = 2; a = v[i]` → `a: integer?` · REFUSE — none (a widening) |
| N-Reserve | expressions.rs:154/163/6369 | ✓ 3 | (cited; keep) |
| N-Prop | operators.rs:3921–3927 `operand_nullable` + :3975 | — | HOLD `n + 5` on `n: integer?` is `integer?` · REFUSE `m: integer = n + 5` warns |
| N-Domain | operators.rs:3913 (float `/`,`%`) + definitions.rs:1565 (the stdlib `-> τ?` strip under `LOFT_NO_NULLFLOW`) | — | HOLD `sqrt(y)` is `float?` · REFUSE `z: float = sqrt(y)` warns |

Observations for phases 2 and 3, from locating these:
- The "10 `nullflow_enabled()` sites" split by RULE, not by "propagate/gate/warn": three are the
  N-Store warn/error split (mod.rs:4201/4246/4263), two N-Prop (operators.rs:3921/3927), two
  N-Domain (operators.rs:3918 float div, definitions.rs:1565 stdlib strip), one N-Cast
  (operators.rs:3465, the text-as-τ assertion), and two the min/max/clamp return shape
  (mod.rs:5585/5591).  Phase 2's fold is therefore "one predicate per RULE reads the flag",
  not one predicate for all ten.
- N-Store already has at least FIVE homes (control.rs ×2, mod.rs two branches, keys.rs three
  gates), which is phase 3's starting census in numbers.

`rule_tags.py check` after the citations: every one of the 18 N rules cited, each at the home the table names, each with its pair in `tests/scripts/153-n-*.loft`.

## Phase 2 — fold: one predicate per RULE reads the null-flow flag (2026-09-05)

The ten `nullflow_enabled()` reads did not split by *propagate / gate / warn* — they split by
RULE, and a site reading the bare flag said nothing about which: 3 decide `N-Store`'s
warn/error split (mod.rs, two branches and the heap target), 3 `N-Prop` (`operand_nullable`
in `operators.rs`, the null-transparent math fns in mod.rs), 3 `N-Domain` (the float `/`/`%`
typing, the stdlib `-> τ?` strip under the opt-out, the constant-in-domain elision), 1
`N-Cast` (the bare `text as τ` assertion).  Each rule now has a FACE of the flag under its own
name in `keys.rs` — `nprop_enabled`, `ndomain_enabled`, `ncast_asserts`,
`nstore_softens(narrow)` — cited `@FR-N-*`, so a rule's flag-reading sites are one grep and
the flip stays one switch.  The N-Store WIDTH test (`byte_width < 8`) was spelled by hand in
both branches and could only agree by accident; `Parser::nstore_narrow` is its one home.
`heap_target ||` stays per-site: it is the site's own fact, not the rule's.

**Verify.**  `scripts/introspect_diff.sh <before> <after>` — the B7r/B7s method made a script:
`introspect` (IR + bytecode + generated Rust, and stderr) of the pre-fold and post-fold
compilers over every corpus file.  Under the default: **IDENTICAL 1268/1268**.  Under `LOFT_NO_NULLFLOW=1`:
**IDENTICAL 1268/1268**.  Nothing a program emits moved.

**What it leaves for phase 3.**  The split's two halves now have one home each, but the
REFUSAL still has five: the two `nstore_diag` branches here, `change_var_type`'s "cannot
change type" for a declared local (an ERROR where this split would WARN), the call-argument
and heap gates in `keys.rs`, and `control.rs`'s two cited sites.  Phase 3's chokepoint is
where those five become one call to `nstore_softens` / `nstore_narrow`.

## Phase 3 — measured before the design (2026-09-05)

**The refusal's twelve sites.**  `n_store_violation` + `n_store_violation_inner` are asked
from eleven positions (`convert_callers_classified.md` in the plan's notes classifies the 61
`convert` callers as store / test / discharge / internal), and `change_var_type`'s "cannot
change type from `τ` to `τ?`" is a twelfth that refuses where the others warn.

**Stage A — 102 cells, position × element kind × discharge**, hand-checked, both backends
(`tests/scripts/153-n-*-refused*.loft` are the REFUSE cells; the HOLD cells are
`153-n-rules-hold.loft`): every discharged cell is silent; a declared LOCAL is an ERROR at
every width (stricter than `(N-Store)`, which says warn at full width); `element` / `arg` /
`return` warn, narrow errors; and SEVEN undischarged cells are SILENT — a `τ?` into a
tuple-literal member for all six kinds, and `vector<τ>?` into a non-null struct field.  They
reproduce on `main` (f4f10cc5, a fresh build): **loft#1366**, fixed by this phase's fold.

**Stage B — loft2's two-spellings cells (their @FR-O walk, F3):** one local assigned from BOTH
spellings of `S?` — the embedded field's `__nullable<S>` (`L-Null-Tag`) and the pointer
(`L-Null`) — `x = y; x = o.opt` and the reverse; the branch form; `x = o.opt` alone as the
control.  Reproduce on `main`: c1 is a use-after-free under `LOFT_STRICT_STORES=1`
`LOFT_POISON=1` (the owner witness frees the parameter's store), silent-wrong without.  The
bind of a tagged projection into a local IS this phase's junction (`L-Null-Which`: a local's
spelling is the pointer, so the bind goes through `emit_nullable_slot_read`), so it is the
phase's second matrix rather than a separate fix: **loft#1367** (loft2 filed it; this phase's
commit carries `Fixes #1367`).

**Design call (from the notes' candidate C):** split `convert` into `convert_store` for the
STORE flavour (the twelve asks fold onto it, one `nstore_softens` / `nstore_narrow` decision)
and leave the test/discharge/internal callers on `convert`; the `types.md:135` vs `:159`
severity inconsistency (local ERROR vs full-width WARN) is settled by the rule's own text and
the local's ERROR becomes the split's warn — recorded as `Contract: strained` if the local's
severity moves.

## Phase 3a — the τ? refusal at ONE arm, and eight silent stores reported (2026-09-05)

**Where the one point is (open question 1, answered by measurement).**  `convert`'s
Optional-SOURCE arm: a census instrument (`#[track_caller]` + `LOFT_TRACE_UNWRAP=1`, kept)
run over the corpus in a scratch worktree counted 6014 `τ? ⤳ τ` peels in 1268 files and
every one passed that arm — eleven site-level asks each followed by a peel there, and the
peels WITHOUT an ask were the admitting faces (comparisons and null-transparent callees at
the call-argument site ×2650, null tests ×16, conditions ×3) plus one hole nobody had listed:
a nullable INDEX (×6).  So the arm asks the rule's τ? half (`nstore_unwrap_report`, one body)
and `convert`'s entry asks the bare-`null` half (`nstore_null_report`); `convert_store(…,
what, at)` names the slot through a context stack the arm reads, `convert_admitting` says a
read is not a store, and a bare `convert` is asked with generic wording.  Omission is loud in
both directions: a store that forgot its name degrades the message, a face that forgot to
admit warns spuriously (measured: the if-arm join did, once — `(N-Join)` — and was named).
The lowerings that never reach `convert` (the if-join accumulator, the append routes, the
struct literal's vector-field deep copy, a `null` return's sentinel, a rewritten tuple
return) keep `n_store_violation`, now a thin caller of the same two bodies.

⚠ **The arm's own comment says "two askers and no third", and it has been wrong once: batch 6
found the `&τ?` parameter as a third SPELLING** (loft#1413, cured by asking both tests of the
pointee).  A reported FOURTH — an index expression reaching a non-null slot — was measured and
is not one (loft#1436, closed by-design 2026-09-07), and how it failed is the lesson: the cells
indexed by a CONSTANT, which `(N-Index)`'s EXCEPT trusts on purpose (@PLN25 DN3's
`index_provably_fit` types `v[<constant>]` as `τ`, because the developer typed the number), so
there was no nullable store for the gate to miss.  Varied to a VARIABLE index, all three
positions report — the direct call argument and the declared non-null local included.  The
constant/variable axis is the one that question turns on, and verifying the reported program
instead of varying it is what made a by-design contract read as a hole.  What remains is an
ENHANCEMENT with a flow question behind it: the trust is unconditional even where the length is
a literal and no append intervenes.

**Does the heap half share the chokepoint (open question 2)?**  Yes for the τ? face — a
`vector<τ>?` peels through the same arm — and the bare-`null` heap gate (loft#1313) rides the
same entry ask.  The one heap lowering that bypasses `convert` (the deep copy) asks for
itself and is named.

**Measured.**  Stage A (102 cells): exactly the seven silent cells moved, silent → WARNING
(`stageA_p3a.txt` vs `stageA_today.txt`), the 66 discharged cells silent, every other cell
unchanged.  The admitting-faces guard (`153-n-store-admitting-faces-hold.loft`, 16 reads of a
`τ?` that are not stores) is silent on both builds with every value pinned.
`scripts/introspect_diff.sh` over the corpus, stderr included: `DIFFERENT 16 of 1272` — every one a stderr line and none an emission: the thirteen corpus files that gained a warning (nine nullable indexes after a `find`, three `null` keys, one `null` into a vector field), reviewed one by one, and the three new guards, which differ by construction.  New guards:
`1366-a-nullable-into-a-tuple-literal-member-is-reported`,
`1366-a-nullable-vector-into-a-non-null-struct-field-is-reported`,
`153-a-nullable-index-is-reported` (each 0 warnings on 806a8d84).  `(N-Store)` in `types.md`
now names its slots — the index among them — and its non-stores.  Every seam first covered
warns at every width (loft#1232's doctrine), so `tuple__narrow__none` warns where
`arg__narrow__none` errors; raising it is COMPATIBILITY.md's process.

**Left for 3c.**  loft#1367's bind of a tagged projection into a pointer-spelled local
(3b took the declared local's severity below; 3c the bind).

## Phase 3b — the declared local takes the rule's severity, and the inferred one widens (2026-09-05)

**Two halves of one question — what is a LOCAL's type after a `τ?` is written to it — and the
rules answer each.**  `(N-Decl)`: a declared `x: τ` keeps τ and the write is `(N-Store)`'s, a
WARNING at full width with the store proceeding, an ERROR at a narrow one.  `(N-Join)`: an
inferred local widens to `τ?`, silently.  The code refused BOTH with `change_var_type`'s
"cannot change type from `τ` to `τ?`" — the twelfth home of the refusal, the only one erring
where the eleven others warned (Stage A's `local` row), and the inferred half was hidden by a
vacuous guard: the phase-1 hold cell wrote `j = 2; j = dv[9]` with a CONSTANT index, which
@PLN102 D1 trusts by contract and types non-null, so nothing was widened and the assert read
the in-band sentinel.  With a variable index the same program was refused.

**Where each half lives.**  A declared local — a parameter too (its type is the signature's),
and a write-back `&τ` parameter, which is the caller's slot one link away and had never been
asked at all (the `RefVar` peel carried the null in silence) — is asked through the ONE store
face (`convert_store`, worded "the local `x`" / "the parameter `x`") BEFORE the retype, at the
assignment seam and at the tuple-destructure site, so the arm reports with the rule's split
and the retype then sees the peeled type.  An inferred local takes the `(N-Join)` arm added
to `change_var_type` beside DN6's null-start arm (which now reads its source through
`base()`, so `a = null; a = v[i]` joins too): widen to `Optional(the wider width)` — `u8 ⊔
integer? = integer?` — and say nothing; the widen is PROVEN by the next declared slot it
reaches, which warns.  The declared / inferred split has one home, `Function::is_declared`
(`argument || annotated`, what `retype_would_be_refused` already read), refined at the parser
by `author_declared`: a local the compiler PROMOTED to a hidden out-parameter — the text
return buffer `text_return` hoists — carries `argument` too, but its type is the compiler's.
Without that refinement five corpus files warned spuriously (`got = maybe(i); return got ??
"<none>"` in `449`, `918`, `806`, `pln133-*`: "a `text?` is stored into the parameter `got`"),
which is the corpus showing an admitting face missed, exactly as 3a's design said it would.

**Measured.**  Stage A: the five full-width `local` cells moved ERROR → WARN, the narrow one
stayed ERROR, no other cell moved (`stageA_3b.txt` vs `stageA_p3a.txt`: 66 silent / 32 warn
/ 4 error).  The 3b cell list (`stage3b/CELLS.md`, 25 cells written before the first edit,
every value hand-computed): every declared cell reports once and holds the null, every
inferred cell is silent and widens, the controls (`?? d`, `x?`, `j: integer?`) stay silent.
`scripts/introspect_diff.sh` over the corpus: `DIFFERENT 13 of 1278` — the twelve re-pinned
or new guards, which differ by construction, and loft#859's file, which now COMPILES (its
inferred `g` widens and the return warns) — no other emission or diagnostic moved.  Both backends on every runnable guard under strict stores and poison.
Guards: `153-n-store-a-declared-local-warns-at-every-full-width-kind` (twelve slot shapes,
one report each), `153-n-join-an-inferred-local-widens-to-nullable` (eleven joins, the widen
proven by a later declared store), `153-n-store-refused-into-a-narrow-local` (every narrow
slot the local family reaches, four cells), `1103c` (the narrow branch-arm cell, split out
of `1103b` so its five full-width cells RUN), the seven phase-1 pair files re-pinned to the
rule's text, `153-n-rules-hold`'s `(N-Join)` cell indexing by a variable (refused on the
parent build — its compiling is the receipt), `859-nullable-retype-advice` covering both
spellings, and `pln102_stdlib_reachable_null_returns_are_typed_nullable` in `tests/issues.rs`.
`matrix_axes.py` on the two matrix guards: element type reaches integer, float, text,
struct, nested container, boolean, character, enum (narrow in its own file); container kind
beyond vector and tuple (a keyed collection as a local from a nullable source) is unreached,
because a keyed collection is not a vector element and no cell of this row can produce one.

**Found on the way.**  loft#1369: a nullable struct local rebound from a non-null parameter to
a nullable one leaks one record on `--native` (`x: S? = z; x = y`) — pre-existing, the
declared spelling leaks on 8498fdf1 — a neighbour of loft#1367 (a local assigned from two
sources) for 3c to read beside it.  The `join_ref` cell stays in the guard: its values pass
on both backends and the native suite does not arm the leak check.

**Contract.**  `D-Decl-Sev` in `types-history.md`, opened and closed; the worked table under
the rules corrected (`a: integer = 2; a = v[i]` → warning, `a` stays `integer`).  A documented
refusal became a warning: `Contract: strained`.  Open question 3 (whether the warning becomes
an error at the freeze) is unchanged and stays COMPATIBILITY.md's.

## Phase 3c — a tagged projection reaching a local is read through its tag (2026-09-05, loft#1367)

**The rule already said where a LOCAL's `S?` lives** — `(L-Null)`: a binding is a pointer with
`nullref` for absence; `(L-Null-Tag)`: only INLINE storage (a field, an element, a tuple
member) spends the discriminant; `(L-Null-Which)`: the slot decides.  The code left the
decision to whichever assignment parsed LAST: `x = y; x = o.opt` typed `x` as the tagged
slot and read the pointer parameter `y` as a record with a tag byte (a use-after-free, the
witness freeing `y`'s store); `x = o.opt ?? y` refused the program naming the synthetic
(`__nullable<S>?`); `d: S = o.opt` was silent and read a present record of zeroes for an
absent slot; `x = o.opt; if c { x = null }` was refused.  Stage B's other cells — `x = o.opt;
x = y` and the branch forms — were CORRECT all along under `@FR-B-Copy` (a bind off a
parameter copies, so the write through `x` lands in the copy and `y` keeps its value), and
loft#1367's table had expected write-through there; the table in this section is the
hand-computed one.

**One home.**  `Parser::read_through_tag(code, tp)`: a tagged `__nullable<S>` value reaching a
NON-SLOT position is read through its tag there — `emit_nullable_slot_read`, the read half of
`@FR-L-Null-Tag` — and both the value and its type become the pointer, on BOTH passes (a
`vector<S?>` element is the synthetic from pass 1, and converting only on pass 2 typed the
local as the slot on pass 1 and refused pass 2's pointer as a type change).  Four callers,
each a position the rule names as not-a-slot: the assignment seam (a plain local target,
never a `&` link or a field/element target), the tuple destructure (the type half,
`tagged_pointer_type`, is asked before the read exists), the `??` subject
(`handle_null_coalesce`) and the postfix `?` subject (`handle_default_fallback`) — so the
default is hinted and built for the pointer's base `S`, the shape the present arm now has,
and the special hint arm for a tagged subject beside a dense target is no longer reached.

**What the read looks like to the store-lifetime layer, and the three sites that read it.**
The read is `if <present> { <payload projection> } else { nullref }` — an `If` where the
ownership oracle saw a JOIN of a view and an owned value, `owner_witness_locals` saw
neither a mint nor a view (so `x = y; x = o.opt` freed `o`'s store at scope exit as the
copy's), and `nullable_view_locals` saw no projection (so `x = o.opt; x = null` released
`o`'s store at the rebind).  One predicate answers all three: `use_analysis::through_null_arm`
— a two-arm `if` whose other arm HOLDS NO STORE (`holds_no_store`: a `null`, the null sentinel)
delivers its present arm, and the classifier, the witness's view test and the view marking
read through it.  `holds_no_store` is also the identity of the var-level join (`x = null` says
nothing about what `x` owns elsewhere) — the fact the argument witness and the view-root walk
already spelled twice each.

**A consequence to know.**  A local viewing a tagged slot holds the PAYLOAD's address, so a
later `o.opt = null` is not visible through it (`x != null` stays true and `x.n` reads the
old payload), exactly as a view through a `reference<S>?` field behaves after the field is
cleared; the slot's own reads see the tag.  The synthetic spelling re-read the tag on every
use and saw the clear.  This is what the pointer rule implies, not a new rule; it is recorded
beside `(L-Null-Which)`.

**Measured.**  The 3c cell list (`stage3c/CELLS.md`, 17 cells written first): every cell
right on both backends under strict stores and poison, `dn` reporting `(N-Store)`'s warning
and reading null for an absent slot; the existing
`a-nullable-struct-has-one-notion-and-two-spellings` guard green on both (it caught the
pass-1 half); loft#1369's leak unchanged (its own issue).  Guard:
`1367-a-tagged-projection-bound-to-a-local-is-the-pointer` (21 cells: both orders, both
branch orders, both declared spellings, the vector element, the destructure, the pointer
null, the view handed up, `??` with a pointer default / a literal / a call in a local, a
declared dense local, a return and an argument, the postfix `?`, and the dense declared
local).  `matrix_axes.py`: element type reaches struct only — the tagged spelling exists for
a struct alone, so the other kinds are out of this family by construction.
`scripts/introspect_diff.sh` over the corpus: `DIFFERENT 15 of 1281` — every one an emission (a tagged projection now read through its tag at its bind or its `??`) and none a diagnostic, each green on both backends under strict stores after.  `Fixes #1367`, `Contract: settled` —
the rules named the local's spelling; the code failed to convert at the boundary.

## Phase 4 — opened: the instrument and the first measurement (2026-09-05)

`scripts/optional_rank.py` ranks the screen's opaque functions the way this phase wants to
walk them: tier 0 reads a DECLARED type (a field's, a local's or parameter's, a return's —
where a `τ?` arrives with its wrapper on), tier 1 decides an lvalue place, tier 2 sits on the
use path.  On the tree that holds phase 3 and the `@FR-O-Witness` walk: **353 opaque
functions — 180 in tier 0, 18 in tier 1, 155 in tier 2**; tier 0 by file: `parser/control.rs`
24, `scopes.rs` 18, `parser/mod.rs` 17, `parser/vectors.rs` 14, `state/codegen.rs` 12,
`parser/definitions.rs` 11.  The tiers are regex evidence, not proof — tier 0 also holds
emitters matching `Void` or `Text` on types that cannot be nullable — so the list is the
ORDER, and each top-tier function is closed only by reading it: it peels through `base()`,
or a probe cell shows a `τ?` cannot arrive there.  The walk itself is the long tail the
ordering section names.

**Batch 1 — the opaque `data.rs` verbs' bare callers (2026-09-05).**  The screen's caller
half names the verbs that are opaque THEMSELVES — `is_dbref(Optional(Reference))` is false,
`heap_dep` and `heap_def_nr` answer `None`, `is_scalar(Optional(Integer))` is false — so
every bare caller answers wrong for a `τ?` at once; the walk went caller by caller
(`stage4/cells/CELLS.md`, eight cells; `stage4/amp/`, the `&` matrix).  Closed by cells:
the `par` clause refuses a `-> S?` worker (a refusal, honest); a generator cannot spell
`iterator<S?>` at all (refused at the type); a `S?` parameter rebound in its body delivers
correctly; a `-> S?` return delivers correctly.  The finding: the `&` lowering's `is_scalar`
closure and record test — a `&` of a nullable LOCAL fell past both arms and bound a SILENT
COPY (`q = &x; q = 7` left `x: integer?` unchanged, both backends), and the cure opened the
whole `&τ?` family: every read and write site asks the link's inner type bare.  That is a
feature the rules promise (`(B-Ref-Intro)`, `(F-ParamRef)`) and the lowerings do not carry,
so both spellings were DECLINED at the link's one builder with a message naming the cure
(`D-bind-17`, loft#1372).  **The decline lasted a day: loft#1372 closed it on 2026-09-06** —
the cure was `Type::base()` at the nine sites that asked the link's inner type bare, since
`Optional(τ)` shares `τ`'s storage and a `&τ?` needs no representation of its own.  The
guard's cells stayed and its expectation flipped, from
`153-a-link-to-a-nullable-slot-is-declined` to
`153-a-link-to-a-nullable-slot-carries-its-slot`.  The matrix's NON-nullable controls found loft#1371 on the way: a whole-value
write through a `&` link to a text, a struct or a vector does not reach the source, and the
struct leaks.

**Batch 2 — the CFG ownership oracle's heap filter (2026-09-05).**  `ownership_cfg.rs` asked
`heap_dep()` of a local's type bare at its four "is this a heap local?" filters (the leak scan's
candidate set, the minted-store scan, the over-free check, the return dump), so a NULLABLE heap
local was never a candidate: the over-free positive control (`08-overfree-positive-control`)
with its view declared `vector<integer>?` (`08b`, the twin) went unflagged under the same
injected free the dense one is flagged for — the oracle green over exactly the twin this plan
is about.  Peeled through `base()` at all four; the cell is
`oracle_over_free_check_sees_a_nullable_view_local` in `tests/ownership_oracle.rs` (the
un-injected twin clean, the injected one RED on `bview`).  The leak scan's two filters are
shown unreached by a nullable-typed var on today's lowering — a nullable local's record is
minted into a work-ref it then adopts (`__ref_p2_1`), never into the local itself — and are
peeled alongside so a lowering that mints into the local directly is covered by existing.

**Batch 3 — the return-delivery and materialise families (2026-09-05).**  One matrix for the
`control.rs` / `scopes.rs` tier-0 group (`stage4/cells3/ret.loft`): a `-> S?` and a
`-> vector<integer>?` delivering a parameter, a nullable parameter, a parameter's dense and
tagged field, an element, a local, a nullable local, a tail call and a value branch — eleven
cells right on both backends under strict stores, and one wrong: an ABSENT element handed back
as `S?` reads present at the handle's null test, garbage after the bind (**loft#1374**: the
out-of-range read answers the SLOT spelling of absence, a zero record with a live store
number, and the handle test reads the store number alone — one absence, two null tests; the
local's `== null` sees it, the parameter's `!= null` does not).  That is `@FR-L-Null`'s
converge-the-citations question and opens phase 5.  The materialise site
(`dispatch.rs::materialises_element`, the scopes twin) was walked with a dense control first
and the CONTROL was the finding: a view of a vector element taken before the vector grows past
its allocation is stale on both backends, the materialise walk listing no view for a `+=`
after the bind (**loft#1373**, not an `Optional` defect; its nullable twin waits on it).
Shown unreached by a wrong answer, with a positive cell each: a tuple with a nullable vector
member (the tuple-set dispatch), a nullable scalar hoisted out of a branch (the native
prologue's `is_scalar`), an absent element bound to a local.  Refusals met on the way, each
honest and recorded: a `par` worker cannot answer `S?` (*not supported*, the concurrency
chapter silent on it), a generator cannot spell `iterator<S?>` (refused at the type).

**Batch 4 — the closure-capture CELL family, and the refusals that rode on it (2026-09-07).**
The `parser/vectors.rs` tier-0 group (14 tier-0, 3 tier-1 — the most tier-1 of any file) is one
family: the functions that decide **what storage a captured variable gets**.  A closure that
MUTATES a captured scalar boxes it into a shared `__cell_<T>`, and `accumulate_scalars_to_box`
chose that set with a bare `matches!` over the scalar `Type` variants — so `Optional(τ)` matched
no arm.  The hypothesis written before the run was that the write is therefore LOST, and it was
**falsified**: thirteen cells green on both backends, because an un-boxed capture still copies
out at a direct call.  The var table is what carried the finding instead — `x: integer` becomes
`ref(__cell_integer)`, `x: integer?` stays `int?` — so the question moved from *is it broken* to
*where does the un-boxed path stop agreeing with the boxed one*, and round 2 varied the
compositions a COPY cannot survive.  Four disagreements, both backends, identical: the write is
lost when the closure is called THROUGH another function, the interleaved shape answers the
closure's private copy (`11 21 100` against the twin's `11 110 110`), and — the part that made
this more than a lost write — **three refusals read the same list**, so a `τ?` capture was not
merely un-boxed but unguarded: sharing between two closures is refused for a dense local and
silently wrong for a nullable one, `const` on a nullable parameter is silently not enforced, and
C115's `&`-from-closure refusal is never asked.  **loft#1408**, `silent-wrong`.

Cured by the peel at seven sites of the one family — the boxable SET (`accumulate_scalars_to_box`),
the cell NAME and its `value` type (`cell_struct_name`, restructured around a `cell_stem` so the
boxable set is spelled once; `cell_value_type`, wrapping through `Type::optional`, phase 0's
`(N-Idem)` home), the read and write ops (`auto_deref_boxed_scalar`, `cell_value_set_op` — both
choose the op from `.base()` because `Optional(τ)` shares `τ`'s storage in-band under C90, while
the declared type keeps its `?` so ordinary discharge still applies), the type-flip preservation
guard, and the boxed-text LHS test.  A nullable takes a cell of its OWN (`__cell_opt_<T>`)
rather than a widened one, because the `value` field has to declare the nullability or
`(N-Store)` is violated at the cell — the rule this plan built in phase 3, met from the inside.
The seventh site was found BY the guard, not by reading: spelled bare, `is_boxed_text_lhs` sent a
boxed `text?` down the text-special branch and emitted `Set(65535, …)`, loft#1206's ICE reached
through the cell instead of through a field — and the `+=` test six lines below it already
peeled, so the drift was already in the file.

**The rules were silent on the half that was wrong, so the rule was extended.**  `(L-CapScalar)`
says a capture is by value at creation, which makes the read direction correct and is why the
`d3`/`d8` cells agree across nullability and are CONTROLS rather than defects.  Nothing said
where the closure's own WRITE lands, which is precisely what the `__cell_` machinery implements
and what a `τ?` was missing.  Added as `(L-CapWrite)` in [closures.md](../../formal/closures.md),
cited at its two enforcing sites.

Guards: `1408-…` (ten cells, each a nullable/dense PAIR across five scalar kinds × five
compositions, plus the `(L-CapScalar)` controls and the struct/vector `(L-CapHeap)` controls),
`1408b-…` (the sharing refusal) and `1408c-…` (the two parameter refusals) — split because a
firing `@EXPECT_ERROR` stops a file and the sharing errors abort before the pass that asks the
parameter ones, which left three cells unreached in the first cut.  All three falsified against
`2808e183` on both backends.  **The `optional` row moved DOWN for the first time in this phase:
`728 | 369 | 5 | 354` from `728 | 364 | 5 | 359`, tier 0 from 180 to 177** — five functions
moved, every one with a cell.

Filed rather than bundled: **loft#1409**, the same user-visible symptom from a different
mechanism — a FORCED-SIZE integer capture (`u8`, `i16`, `u32`) is in `scalars_to_box`, so the
refusals fire for it, but `cell_struct_name` has no template for it, so an indirect call loses
its write exactly as the nullable did.  It is `synthesize_cell_structs`'s own documented "phase
02d-ii silent gap".  Kept apart because the cures differ and a blanket refusal would break the
direct-call form, which compiles and is correct today (`f_u8_direct` measured green).

**Batch 5 — the cursor-match family, and a finding left OPEN on purpose (2026-09-07).**  The
`parser/control.rs` tier-0 group that batch 3 did not reach: `cursor_shape`, `parse_cursor_match`,
`peek_subrule_capture`.  `cursor_shape` decides whether a `match` subject IS a cursor with three
bare `Type` tests — the SUBJECT is a `Reference` to a struct, the first `Vector` field is the
source, a field named `pos` is an `Integer` — so a `τ?` in any of the three makes the answer
`None` and the struct silently stops being a cursor.

Three of the positions CLOSED, measured on both backends and guarded
(`153-a-cursor-match-over-a-nullable-subject-advances`): a nullable SUBJECT works, because the
subject reaches `cursor_shape` already discharged and its bare test is never asked a wrapped
question (the var table shows the local is `ref(732)?` while the cursor temps are still minted);
an unrelated nullable field beside a well-formed cursor is ignored; and a nullable `vector<T>?`
declared BEFORE the real source is skipped by the first-vector-field rule rather than chosen and
found null.  Every cell asserts the cursor POSITION as well as the bound value, because a cursor
match that degraded to a plain struct match could bind the right value while failing to advance.

**Two positions do NOT work, and the second one is why this batch ships without a fix
(loft#1410).**  A `?` on the `src` or `pos` field turns cursor matching off and the failure surfaces as a parse error
**Two positions do NOT work, and the second one is why this batch ships without a fix.**  A `?`
on the `src` or `pos` field turns cursor matching off and the failure surfaces as a parse error
pointing at the arm PATTERN, naming nothing the author can act on.  And a nullable ELEMENT
(`src: vector<Tok?>`, or the same shape with no cursor at all — `match v { [Id { x }] => … }`
over a `vector<Tok?>`) is refused the same way, while its dense twin runs; `control.rs:8541` and
`:8641` ask the element's ENUM identity bare, and the `Type::Iterator` arm forty lines above
already asks its own through `elm_tp.base()`, so the drift was in the file.

The peel was BUILT, MEASURED AND REVERTED rather than landed.  It routes correctly and then
surfaces `No matching operator '==' on 'Tok?' and 'Tok?'` from `parse_field_sub_pattern`, whose
`if let Type::Enum(e_nr, true, _) = field_type` is bare too — and peeling THAT one would be
wrong rather than merely incomplete: the tag test it builds is
`OpEqInt(OpConvIntFromEnum(OpGetEnum(field_val, 0)), disc)`, and for a `vector<S?>` element the
byte at offset 0 is @PLN25's nullable TAG, not the variant discriminant, so a peeled tag test
answers a variant question with an absence bit.  The element has to be read THROUGH its tag and
the pattern must FAIL on an absent element — `(L-Null-Which)`, the rule phase 5 batch 1 closed
for field reads and method calls, now asked of the @PLN35 slice machinery.  That is a design
step in the pattern machinery rather than a peel, so it is the batch's OPEN item — filed as
**loft#1410** with the measurement, rather than half-fixed.  The guard says in its header what it does not
cover and why.

**Batch 6 — the `&` PARAMETER group in `scopes.rs`, and the THIRD spelling of `τ?` at phase 3's
own gate (2026-09-07).**  `reassigned_ref_params`, `removed_ref_params`, `removed_params_map`,
`def_reshape_refusals`, `amp_writeback_owned_copy`, `reclaim_safe` — the tier-0 group that
reasons about `&` parameters in the scope pass, a different layer from batch 1's `&` LOCAL link.

Reading them reshaped the hypothesis before a single probe: they match the OUTER
`Type::RefVar(_)`, and a `&integer?` is `RefVar(Optional(Integer))`, so the shallow tests fire
for a nullable parameter exactly as for a dense one.  The matrix therefore aimed at the deeper
question — does a `&τ?` behave like a `&τ` — as eight nullable/dense pairs.  Seven pairs
delivered identical values on both backends.  What every nullable cell carried instead was a
DIAGNOSTIC its dense twin did not: *"a nullable `integer?` is stored into parameter 1 of `bump`
of the non-null type `&integer?`"* — a correct program, told that the store its own signature
asks for makes the value null, in a "non-null type" that is spelled with a `?`.

**The cause is phase 3's own gate, and it is this plan's thesis in one line.**  `τ?` has THREE
spellings at `nstore_unwrap_report`; two were handled — `Type::Optional` and the synthetic
`__nullable<S>` (loft#1123, whose comment sits at that guard) — and the third is a `&`
parameter, which carries its nullability INSIDE the reference.  Asking `Type::Optional` of the
outer type answers about the REFERENCE, not about the slot the store lands in.  Cured by asking
both tests of the pointee, because the pointee is the slot (**loft#1413**).  `warning` is the
tier that gates library CI, so a library exposing a `&τ?` parameter failed its own CI on correct
code with a self-contradictory message.

The negative control is what makes it a fix rather than a deletion: a nullable argument into a
`&integer` parameter must STILL warn, and does.  Both directions are in
`tests/callarg_nstore.rs`, which counts the warning and so can go red — measured failing with
the peel reverted, its twin staying green.  That Rust pair carries the falsification, because
`make falsify` compares exit, asserts, leak, panic and refusals and a WARNING moves none of
them: the `.loft` guard is INERT against the pre-fix build, correctly, and says so in place of
a falsify stamp.  The `.loft` guard carries the other half — that the writes still reach the
caller, so the silence is not bought by dropping the store.

**One face left open, and it is a different site.**  A `&vector<T>?` parameter still reports.
`control.rs::un_ref` converts `RefVar(τ?)` to `τ?`, and `convert` peels the destination's
`Optional` before the `RefVar`, so the chain ends up asking *"store `vector<T>?` into
`vector<T>`"* — a nullability question the dereference never posed.  Localised by instrument
rather than by reading (`parse_index` → `un_ref` → `convert` ×3 → the gate, `what = "a slot"`,
i.e. a bare convert with no store context); the identical read and write on a plain nullable
vector LOCAL are clean, which is what makes it `&`-specific.  A recursion-order defect in
`convert`, recorded on loft#1413 rather than bundled.

Filed on the way, unrelated to null and found as the DENSE control of the removal pair:
**loft#1411** — `v.remove(i)` through a `&vector<T>` parameter corrupts the caller's vector
while `len` stays right (`[10,20,30,40,50]` with `remove(1)` gives `[10,20,50,0]`, both
backends, already wrong inside the callee).  `sev:high`, `silent-wrong`.

**Batch 7 — the slice-pattern element family, and the measurement that reversed batch 5's own
reading (2026-09-07).**  Batch 5 left loft#1410 OPEN with a design step recorded in it: the
variant tag test reads the byte at offset 0, which for a `vector<S?>` element is @PLN25's
nullable TAG, so the element would have to be read THROUGH its tag and the pattern made to fail
on an absence.  **That reading is right for the shape it was measured on and wrong for the shape
the issue is about**, and `loft introspect` says which in one line: a `vector<Tok?>` element
where `Tok` is a struct-ENUM is **16 bytes, identical to `vector<Tok>`**, read through the same
`OpGetField(elem, 0, kt)` projection (`type_def_nr` peels `Optional` already), and its variant
discriminants start at **1** because *"0 is the null/undefined value"* the variants are numbered
away from (`definitions.rs`).  So an absence is discriminant 0, every variant tag test already
answers FALSE for it, and `(M-Variant)` — an absence is no variant — needs no tag read and no
is-present condition at all.  The tagged `__nullable<S>` the reverted peel would have broken is
the STRUCT element, which no variant pattern can name.  `(L-Null)`, not `(L-Null-Tag)`.

The cure is therefore the peel PLUS the exclusion that keeps the synthetic out of it, at ONE
home: `Parser::pattern_variant_enum(tp)` — `Type::Enum` of `tp.base()`, `None` for a
`__nullable<S>` — read by the **ten** sites that asked the element's variant identity with a bare
`match Type` (the scalar-capture guard, the variant sub-pattern peek, the repetition and its TAIL
sub-pattern, both alternations, `build_literal_match` and its diagnostic twin, and BOTH arms of
`parse_field_sub_pattern` — that last pair is where the issue's second error came from, the FIELD
position rather than a peeled tag test: a nullable field sub-pattern `A { t: Id { x } }` was
broken the same way and is fixed by the same home).

**The silent half nobody had measured.**  The refusal was only the loud face.  The UNIT spelling
`[Id]` over a `vector<Tok?>` fell past the bare peek into the bare-name branch and became a
BINDING named `Id`, so it matched **every** element — another variant, and an absent one — where
its dense twin correctly answers "no match".  `silent-wrong`, both backends.

**loft#1414, the same walk one layer down.**  `@FR-O-Deps` says what a value borrows is read off
its type, so a read that borrows and says nothing is read as OWNED — and *"this element read
views its subject"* was spelled **five** times in `control.rs`, each a three-arm `match` over
`Reference | Vector | Enum`.  Three of them let an `Optional` fall past, so a nullable element
binding carried EMPTY deps: `match v { [a, ..] => a }` over a local `vector<Tok?>` was classified
as returning an OWNED value, the subject's store was freed at the callee's exit, and the caller
read **101** — the first element of the vector allocated next — where the dense twin answers 7,
on both backends, with `LOFT_POISON=1` blind to it because the store was already live again.
The other two spellings peeled correctly and were merely duplicates.  One home,
`Parser::element_view_of`, which is a call to `Type::with_deps` — that already writes through the
wrapper, which is why the fold is a call and not a sixth `match`.  The second face of the same
missing dep appeared the moment the peel let a repetition run over a nullable element: the
materialisation's per-element read was freed while the subject still owned it (a poisoned read on
`--interpret`, an `E0425` on `--native` for a free emitted outside the temp's block).

**The refusal that stayed, and now says why.**  `cursor_shape`'s source and position tests keep
REFUSING a `vector<T>?` source and an `integer?` `pos` — peeling the source test would move which
field becomes the cursor source, which batch 5's landed guard pins (`CurFirst`).  What was wrong
was only what the author was told: `[` reached the STRUCT-pattern parser, the first pass broke out
silently and the run died on a brace, so the message was *"Expect token }"*.  Now one error names
the field (*"its `pos` field is `integer?`, and a cursor's position must be an integer"*), the arm
is skipped on BOTH passes so pass 2 can print it, and the element-type refusals recover through
the closing `]` — which also cures a cascade the DENSE non-enum repetition has always had, and
makes a forward-aliased cursor source (`src: Toks` with `type Toks = vector<Tok>` below) compile
where it did not.  The `__nullable<S>` no longer leaks into a message either (*"'S' is not a
variant of __nullable<S>"* → *"…needs a struct-enum element type, not `S?`"*).

**The rules gained what the table was missing.**  `types.md`'s per-type null table had no ENUM
row and `(L-Null)`'s sentinel list no enum sentinel, which is exactly the gap that made the
tagged reading look like the only one available: the enum row is now *in-band discriminant 0,
variants numbered from 1, a plain enum reading 255 as absent too*, `(L-Enum)` states the numbering
and the 254-variant limit the parser already enforces, and `matching.md` carries the paragraph
that a nullable subject or element names the same variants and an absence matches none.

**Measured.**  Guards `1410-a-slice-pattern-over-a-nullable-element-names-its-variant.loft`
(22 cells, every nullable cell beside its dense twin: variant sub-pattern, unit variant, bare
binding, two-element sequence, named capture, alternation, repetition with and without a rest,
wildcard, rest, a call-result subject, a LOOP-driven cursor, and the value-enum and tagged
controls), `1410b-…` (the five refusals) and `1414-…` (the escaping view, its tagged and
parameter controls).  All three falsified against `d6e665ae`: 1410 and 1410b on exit/expectations,
1414 on its own **assertion** — its repetition cell moved into 1410's file precisely because a
file that cannot COMPILE on the control proves itself through the exit channel instead of through
the value it is about.  `scripts/introspect_diff.sh` over the corpus: **DIFFERENT 3 of 1338**, all
three the new guards — no existing program's IR, bytecode, generated Rust or stderr moved.
`matrix_axes.py` reports container kind `vector` only and element type enum/integer/struct, both
by construction (a slice pattern is worn by a vector or a cursor over one, and only an enum HAS
variants to name); its A3/A9 columns describe ARGUMENT positions and do not see a match subject,
so the call-result and loop cells are recorded here rather than in its output.  **The `optional`
row moves DOWN again: `730 | 374 | 5 | 351` from `728 | 369 | 5 | 354`** — three functions off the
opaque column, two new ones on the seeing-through side.

Filed rather than fixed, each a different mechanism: **loft#1415** (a dense `-> τ?` element
binding over a vector PARAMETER emits its free outside the block that declares it — `--native`
E0425, pre-existing on `main`), **loft#1416** (`vector<Color?>` over a VALUE enum drops the `?`
at the declaration and the null store is refused naming `vector<Color>`, while a `Color?` local
and field hold null fine), **loft#1417** (`match e { null => … }` is refused for every HEAP
subject — `(N-Match)` holds for a scalar only).

step in the pattern machinery rather than a peel, so it is the batch's OPEN item, recorded here
with its measurement rather than half-fixed.  The guard says in its header what it does not
cover and why.
**Batch 8 — the TUPLE tier-0 group, and a rule that promises a type the model cannot hold
(2026-09-07).**  Seven functions in `parser/mod.rs` decide what a tuple's shape IS —
`refuse_forward_tuple_returns`, `refuse_forward_ref_tuple_params`,
`promote_par_worker_tuple_returns`, `stored_tuple_elements`, `emit_tuple_set_ops`,
`tuple_elem_tag_read` / `_write` — and each matches the whole type bare, so the shape they are
opaque to is a NULLABLE WHOLE TUPLE.  The cell list (`stage8/CELLS.md`) was written first, and
its very first cell answered the question the other ten were built on: **`(integer, integer)?`
cannot be spelled**.  The type parser's tuple branch returns before `parse_type`'s postfix-`?`
handling, so the `?` was left in the stream and every position reported a syntax cascade naming
nothing — *"Expect token ;"* for a local, *"Expect token )"* for a parameter, *"Expect token >"*
inside a `vector<…>`, *"unexpected '?'"* for a field or an alias.

**`(N-Opt)` says `τ?` is a type for ANY τ, and the model has no absence for a tuple.**  A tuple
is its members' bytes: `(L-Null)`'s sentinel needs a value the type RESERVES and a tuple reserves
none, while `(L-Null-Tag)`'s discriminant is for a STRUCT stored inline.  A `value struct` is the
same case and has been refused BY NAME since @PLN101 — five lines away in the same function.
So the refusal is right and the silence was the defect: it now names the tuple the author wrote
and the two cures (nullable MEMBERS, or a `struct` wrapper).  Recorded as **D-Opt-NoNull**, the
TYPES register's only open entry, because closing it is a representation decision rather than a fix —
loft#1423 carries it, with the option that reads best (give a STORED tuple `(L-Null-Tag)`'s
treatment) and what it would cost (one written type nullable in one position and not in another).

**The shape exists anyway, and that is where the batch found its defects.**  Two routes build a
nullable tuple without passing the type parser: `(N-Index)` — `v[i]` on a `vector<(τ, τ)>` IS
`(τ, τ)?` — and a generic `-> T?` instantiated at a tuple.  Measured on that shape:

- an undischarged member read reported *"Expect a field name"*: a message about a NAME, for a
  program that wrote a number, on a receiver whose real problem is the `?`.  A nullable STRUCT
  receiver reads correctly through its absence (`(L-Null-Which)`), which is what makes the
  tuple's answer a defect rather than a rule.  A naive peel of the projection was BUILT first
  and it ICEs in codegen — the value has no representation, exactly as D-Opt-NoNull says — so
  the cure is the named refusal plus the discharges, and those are guarded, because a message
  naming a cure that does not work is the next defect.
- the DESTRUCTURE of the same value said *"Cannot destructure a non-tuple value"*, which is not
  true; it now names the type and the same cure, and binds its targets anyway so the refusal is
  the only report rather than one "Unknown variable" per name.
- **`(N-Default)` did not hold for a tuple** (**loft#1424**, `silent-wrong`, both backends):
  `v[j]?` on a miss answered NULL MEMBERS from a slot typed `(integer, integer)`, while the
  struct twin `s[j]?.a` answered its `0`.  `Data::has_default` recurses over a tuple's members
  and says yes; `Parser::build_default` had no tuple arm and said no; and the recovery for that
  disagreement — *"should not happen in practice"* — typed the absent value as the non-null base.
  It happened in practice.  Fixed at both ends: the tuple default is its members' defaults (the
  value the `??` spelling hands over), and the recovery REPORTS instead of proceeding, so the
  next disagreement between those two predicates cannot be silent.

**Measured.**  Guards `1423-a-nullable-tuple-type-is-refused-by-name.loft` (seven positions, each
naming its own tuple so no two expectations share a substring, plus the nullable-MEMBER cell that
proves the refusal is about the tuple TYPE), `1423b-…` (the member read, through a local and
direct, the stored spelling, and the destructure) and `1424-a-tuple-default-is-its-members-defaults.loft`
(the member kinds a default has to build, both representations, hit and miss, `?` against `??`,
and the destructure as a second consumer).  All three falsified against `d6e665ae` — 1424 on its
own assertion.  `scripts/introspect_diff.sh` over the corpus: **DIFFERENT 6 of 1341**, all six the
new guards of batches 7 and 8 — no existing program moved.  The `optional` row does NOT move
(`731 | 375 | 5 | 351`): these seven functions are closed by a probe cell rather than by a peel,
which is the other half of the phase's Verify line, and the denominator gains the one predicate
the two refusals share.

Filed rather than fixed: **loft#1425** — a `??` on a vector element whose tuple has a TUPLE
member does not compile on `--native` (the generated null test reads `.0` as a bool); pre-existing,
`--interpret` is correct, and it is why this batch's member-kind row stops at three flat members.

**Batch 9 — the DECLARATION-time group in `parser/definitions.rs`, where three questions
answered NO because a `?` was written (2026-09-07).**  Eleven tier-0 functions, and the family
question is one: *at a declaration, which facts are read off a written type, and does a `?` on
that type change an answer that must not change?*  Three of them did, each with a wrong VALUE
behind the silence, and every one of the three was a bare `matches!` over a `Type` variant that
a neighbouring function in the SAME FILE already peeled by hand.

- **loft#1427, `sev:high` `silent-wrong`** — a method declared on a NULLABLE receiver
  (`fn area(self: Square?)`) implemented no variant at all, so the synthesised dispatcher (@F20)
  never emitted its arm and the dispatch fell to the no-variant-matched tail: a garbage number on
  `--interpret` (8858, 9303 — it varies run to run) and `0` on `--native`, where the DIRECT call
  on the same variant answers 16 throughout.  With every implementation spelled that way there is
  no dispatcher at all and the call is read as a field access.  The rules were silent on which
  type a method belongs to, so the rule was written: **`(F-Recv)`** in
  [calls.md](../../formal/calls.md) — the receiver's base type names the method's type, two
  overloads may differ in the `?` (`fn_key` keys them apart, @PLN25), and the SET of methods of
  `τ` is the set of methods of `τ?`, which is what every site that ENUMERATES it must see.  One
  home, `Data::receiver_def_nr`, read by the scan, the arm's discriminant and the warning.
- **loft#1428, `sev:high` `silent-wrong`** — two `index<E[k]>` in one struct are refused because
  an index keeps its tree links in a field OF the record; spell either `index<E[k]>?` and the
  refusal did not fire.  The two trees then overwrote each other's links: `c.a[2]` answered
  `null` for a record `for e in c.a` yields, and `c.b[1]` answered `a`'s record for a key `b`
  never received.  `(Col-Group)` already said a nullable member IS a member — the rule was
  written and the code did not read it — so this is `contract: settled`.
- **loft#1429, `sev:medium` `silent-wrong`** — `spatial<W[w]>` with `w: text?` was ACCEPTED and
  then answered null for a point just inserted: loft#799's failure, which the DENSE spelling has
  been refused for since, reached through the `?`.  The same predicate refused a trie's `text?`
  key with advice for a field that is not a text.  `(Col-Spatial)` already said *integer-not-null
  coord keys*; `(Col-Trie)` now says `text`-NOT-NULL, and collections.md carries the paragraph
  that the three VALUE-keyed kinds do hold an absent key — `hash` finds one, `sorted` orders it
  first — which is what the two refusals name as the cure and what `1429b` measures.

**The corpus emission diff is what found the batch's fourth defect**, and it is the largest:
`scripts/introspect_diff.sh` reported two EXISTING files moved, and `05-enums.loft` had gained a
dispatcher it never had.  The scan's `todo` map is keyed by the ENUM, not by the method — so an
enum with two methods per variant put both lists in one bucket, `create_enum_dispatch_fn` named
the dispatcher after whichever definition came first, and the other method's call site failed as
a field read; where the two lists' parameter shapes disagreed the bucket bailed and NEITHER was
built (`05-enums.loft`, three `area` and three `describe`, no dispatcher for either — invisible
only because it calls both on concrete variants).  The same bucket fed the missing-implementation
warning, so a variant implementing one method silenced the warning about the other.  **loft#1435**,
`sev:high`; the key is `(enum, method name)` and the walk over it is SORTED, because the
dispatchers are emitted in creation order and a `HashMap`'s order is not stable across runs.

Guards: `1427-…` (seven cells, each a dispatch beside the direct call it must agree with —
integer, text and heap payloads, an extra argument, a repeat in a loop, both overloads for one
variant, and the all-dense control), `1427b-…` (the all-nullable enum, apart because it does not
COMPILE on the control), `1428-…` (three refusals, the dense pair among them) + `1428b-…` (the
pairs that stay legal, filled and read), `1429-…` (five refusals, the two type-KIND ones as
controls) + `1429b-…` (the kinds that hold an absent key, and the two cures the messages name),
`1435-…` (two and three methods per enum, complete and partial, three return kinds).  The two
warning halves are in `tests/parse_errors.rs`, whose harness FAILS on an unasserted warning — a
`.loft` guard cannot assert a warning's absence, which is exactly what both need.  All four
value guards falsified against `e9f45817` on both backends; the two control files are INERT by
design and say so.  **The `optional` row moves DOWN by four, the phase's largest single step:
`732 | 380 | 5 | 347` from `733 | 377 | 5 | 351`** — `enum_numbers` and
`warn_missing_enum_variants` no longer discriminate on a `Type` at all.

Filed rather than fixed, each a different mechanism: **loft#1430** (two implementations of one
method, one of which builds its text in a branch and so carries a hidden accumulator its twin
lacks: the dispatcher is abandoned with no diagnostic — pre-existing, and the reason this batch's
text cells are symmetric), **loft#1431** (`spatial` admits any non-text axis; `float` and `i16`
then answer null at the point lookup, which is `(Col-Spatial)`'s integer half and a
COMPATIBILITY question), **loft#1432** (a `τ?` receiver overload after its dense twin is
unreachable while the reverse order is refused as a redefinition — one of the two is wrong), and
**loft#1434** (a nullable collection cannot be ITERATED but can be INDEXED, and the refusal names
neither the `?` nor the `?? []` that works — the ITERATION family, which is batch 10's group).

**Batch 10 — the `scopes.rs` tier-0 group, where the null spelling costs a STORE (2026-09-07).**
Seventeen tier-0 functions, of which batch 6 had walked the `&`-parameter six.  The remaining
group is the store-lifetime pass's own: who owns a store, who frees it, and which locals a
statement gives a new one.  Two defects, both `sev:high` `silent-wrong`, both use-after-free,
both on the two backends alike — and neither one visible on the value channel, which is the
methodological finding of this batch.

- **loft#1439** — a closure that OUTLIVES its frame and captures a nullable heap local reads a
  released record.  `c: S? = S { … }` lowers through a work-ref (the store is minted into
  `__ref_p2_1` and the local adopts it) where the dense spelling has no work-ref at all, and
  that work-ref's free ran unconditionally right after `OpSetDbRef(___clos_1, 0, n)` handed the
  store to the closure record.  loft#1317 had made that free conditional for the RETURNED local
  and its comment states the boundary it chose — *"Where the local is NOT returned, this plain
  free is the store's only release — the local may be captured (the record adopted it and the
  frame emits nothing)"* — which is right for a closure that STAYS and wrong for one that
  LEAVES.  THREE facts have to meet, and each alone is a wrong answer: the record adopted the
  capture, the record ESCAPES (`DepEntry::CalleeFrame` in the declared return type), and the
  capture attribute is one the cascade FOLLOWS.  The third was found by the matrix, not by
  reading: a record-`Enum` capture is adopted for the suppression's purposes and not followed by
  the cascade, so declining the frame's free there LEAKS (`kt=83 Circle50×1`).
- **loft#1442** — a view into a nullable local is not materialised when the local is reassigned.
  `established_stores` names the record kinds bare at three tests, so an `Optional(Reference)`
  local establishes nothing however it is assigned: `o: Q? = Q { … }; v = o.p; o = Q { … }; v.a`
  read a released record while its dense twin copied `v` out and printed the advice that says
  writes through it no longer reach the source.  Both faces are one miss — no materialisation
  AND no report — so the nullable case is the one where the user is told nothing.

**The rules were silent on who frees a capture, so the rule was written.**  `(L-CapOwn)` in
[closures.md](../../formal/closures.md): a captured heap store is freed ONCE, by whichever of the
record and the frame outlives the other; a record left behind never frees at all (the fn-ref type
carries its frame dep, so the sweep skips it), and the record's reach is its CASCADE.  That is
the sentence the three facts above are each a clause of.  Writing it also moved the chapter's
deviation count off zero: **D-clo-24** (loft#1440) is TWO closures over ONE local where only one
escapes — both records adopt the store, the one left behind frees it, and the caller reads a
released record.  Its DENSE twin fails identically, which is what says it is adoption itself —
an ownership TRANSFER where `(L-CapHeap)` says SHARE — and not anything about the `?`.  Filed,
not fixed: the cure is a design step in the capture model.

**Why the chapter's `OPEN: 0` did not see either.**  Its oracle paragraph names what the closing
guards hold FIXED, and the first item is *"every closure is built in the frame that calls it"* —
which is precisely the axis both defects live on.  An `OPEN: 0` is only as strong as the oracle
under it, met here as plainly as anywhere in this plan.

**And the value channel does not score these.**  Both guards read INERT under a plain `make
falsify` and are falsified with the instrument armed (`LOFT_STRICT_STORES=1` — that half alone,
measured: the question is WHEN the store was released, not what its bytes read afterwards, so
`LOFT_POISON` adds nothing to these two),
because a released record keeps its bytes until something writes over them: the same probe
answered `-2401053088876216593` in one program and the right value in another whose only
difference was what ran before it.  Written into both guards' `@falsified-at` lines, and into the
issues, because a repro whose garbage is not reproducible reads as a fix when it is a coincidence.

`scripts/introspect_diff.sh` over the corpus against the pre-batch-9 compiler: **DIFFERENT 10 of
1356**, and every one is accounted for — eight are this stream's own new guards, and the two
pre-existing files (`05-enums.loft`, `09-enum.loft`) are batch 9's loft#1435, which gave them
dispatchers they never had.  Batch 10 changes WHICH FREES ARE EMITTED and moved no existing
program's emission at all: the two conditions it adds fire only for a nullable local a view
outlives and for a nullable capture a closure carries out of its frame, and the corpus has
neither.

Guards: `1439-an-escaping-closure-keeps-its-nullable-capture.loft` (the escaping nullable capture,
with the dense, absent, vector, hash, text, KEPT and record-enum cells that make the three facts
separable) and `1442-a-view-into-a-nullable-local-is-materialised.loft` (a field view and an
element view across a reassignment, each beside its dense twin, plus the no-reassignment control
that must NOT be copied).  The `optional` row stays at **347**: `established_stores` peels onto
the seeing-through side and `escaping_record_holds` joins the opaque side, where a
compiler-minted closure-record local belongs.

Closed by probe cells rather than by a peel: the text-return-buffer family
(`any_text_return_buffer`, `text_return_buffer_for`, `check_text_return_path`,
`has_return_buffer`) delivers a `-> text?` correctly in five shapes, and the capture-adoption
family (`mark_borrowed_captures`, `capture_is_adopted`, `record_adopts_capture`) answers
`adopts=true` for every nullable capture — measured with an env-gated print rather than assumed,
which is what moved the search one layer down to the work-ref.  `scan_inner`'s block-tail HOIST (whose absence is a
codegen panic, not a wrong value) and `scan_if`'s text test are closed the same way: a block
whose tail is a nullable record, a `text?` bound across two `if` arms, and their dense twins,
right on both backends with no leak.  `check_ref_leaks` is left alone
on purpose and is recorded here as the group's known blind spot: it asks `Type::Reference` bare,
so it is blind to every nullable local, and it is `#[cfg(debug_assertions)]`, which
`[profile.dev.package.loft]` strips from every build this project makes — widening an assert
nothing runs is not a fix.

**Batch 10's follow-on — the three issues the walk had filed, closed (2026-09-07).**  The owner
asked for the backlog cleared before a PR, so the batch's own filed items came back rather than
waiting for a peer's branch.

- **loft#1430** — a dispatcher's arms do not share the compiler's work buffers.  A body that
  builds its text in a branch is given a hidden `___acc_1: &text`; one returning a literal is
  not, and `create_enum_dispatch_fn` counted those among the parameters every implementation
  must have in COMMON, so one branching arm beside a literal one drove `common` to 0 and the
  dispatcher was abandoned in silence.  Three parts, each measured by breaking it: hidden
  attributes are out of the shared count; the dispatcher takes its buffer from whichever arm HAS
  one (reading only the first left the arm that needed a destination without one, which
  `--native` renders `let _ret = ;`); and buffers are forwarded only to arms that DECLARE one
  (handed to all, the literal arm gets *"Too many parameters"*).  A required VISIBLE parameter
  one implementation does not share is still no dispatcher, and is now refused by name where
  that implementation is written instead of failing later as a field read.
- **loft#1440** — one STORE, one owner.  Two closures over one local both adopted its store and
  their deaths are independent, so the record left behind released what the escaped one still
  held; the DENSE twin failed identically, which is what said it was adoption-as-transfer rather
  than a `?` question.  Among the adopters exactly one keeps it — the one that LEAVES the frame
  — and the rest borrow.  **The key is the STORE, and getting that wrong is what the first cut
  did**: grouped by the capture's NAME, a local assigned between two builds looked like one
  store with two owners, so the second record was made to borrow one the first never held and
  the one it did hold was freed by nobody — a leaked `S` that the existing
  `a-captured-local-reassigned-after-the-build-frees-its-own-store.loft` caught in the gate.
  Excluding reassigned locals from the grouping made that green and was still wrong, because it
  drops the rule for every program that reassigns the local at all.  The build walk already
  counts assignments, so `(capture local, generation at the build)` identifies the store, and
  the exclusion is gone.  D-clo-24 closed in `closures.md`.
- **loft#1444, found by the cell that separates the two, and fixed with them** — build the
  escaping closure FIRST and the defect stays, for a different reason: the declared return
  type's `DepEntry::CalleeFrame` is published once per LAMBDA and OVERWRITTEN, so wherever a
  function builds more than one it names the last one BUILT.  Two consumers read that note for
  two questions — *is this fn-ref handed out, so its closure store must not be freed*
  (`get_free_vars`) and *which record outlives the frame* (`record_leaves_frame`) — and both
  were answered about the wrong record whenever the escaping closure was not written last.
  Neither trusts it alone now: the free-suppression also treats a fn-ref that is a RETURN SOURCE
  as handed out (the path-local fact that frame already had, and the half that actually frees
  the store), and the ownership question reads `returned_closure_records` — the records named by
  the values in RETURN POSITION, off the tail and off every `return`.
  **The first cut collected every `FnRef` in the body and that is the trap worth keeping**: a
  KEPT lambda's record then looks delivered and is handed a capture the escaping one owns, which
  answered `3` where `5` is right.  Return position only.  D-clo-24 and D-clo-25 both closed,
  and `closures.md` is back to `OPEN: 0` — a count that went 0 → 1 → 2 → 0 in one day, each step
  a re-measurement of the same oracle sentence.

**What remains of the family, filed rather than fixed: loft#1446.**  A capture the closures
share, with the local reassigned AFTER both builds, is still a use-after-free — and the
ownership half is right there: both records adopt one store, the escaping one is chosen, the
kept one borrows.  The free that lands is the FRAME's, which `capture_adoption_owns_free`
declines to suppress for a reassigned local (loft#1324/#1388) — correct for the store the local
names afterwards, wrong for the one the records hold.  That wants the owner witness's
release-by-IDENTITY (`@FR-O-Witness`), applied to a capture rather than to a mixed-ownership
local, which is a mechanism `formal/ownership.md` already carries and a change of a different
size.  Pre-existing on `main`, both backends.

**Three instruments, three questions** — worth stating once, since this batch used all three and
each is silent where the next one speaks: `LOFT_STRICT_STORES=1` answers WHEN a store was
released (both of batch 10's guards); `LOFT_POISON=1` answers what a released record's bytes read
afterwards, which is what makes a use-after-free assertable as a VALUE; and valgrind answers
about a Rust `String` inside a record, which is not a store at all — the loft2 checkout's
loft#1406 leak was 8 bytes of exactly that kind, invisible to the store gate, which reads clean
because nothing was retained.

**Batch 11 — the mechanism the first ten batches kept curing, written as a rule and gated
(2026-09-08).**  Ten batches had cured one mechanism — a `matches!` / `if let` / catch-all `match`
over a `Type` that answers "no" for `Optional(τ)` — and each cured it at the callers it happened
to reach.  The governing fact was already written, in `Type::peel_optional`'s doc block since
@PLN25: *"the nullability-agnostic majority of `match Type` sites peel through this; only the
discharge / store / cast checks read the bool."*  It had no rule, no citation and no gate, so
every batch re-derived it.  It is now **`(N-Shape)`** in [types.md](../../formal/types.md), cited
at its home and at the verbs below, and the register entry is in
[types-history.md](../../formal/types-history.md).

**The re-scope, and it is the batch's real finding.**  The phase has been reading its progress off
the FUNCTION row (359 → 338).  The unit that carries the defect is the TEST, and
`ir_walker_audit.py optional` has been printing that number all along: **2268** shape tests,
**1520** opaque on their own scrutinee.  At three to five functions a batch, the function row is a
seventy-batch queue and the test population is not a walk at all.  So the enforcement is the
DERIVATIVE, not the level: `make optional-ratchet` pins both counts in
`index/optional_ratchet.json` and fails when either GROWS, on `asan_leak_ratchet.sh`'s argument
for a count over an allowlist — the opaque tests are ordinary `matches!` on a `Type` and no
suppression pattern can tell a known one from a new one.  A FALL is not a failure: it prints the
re-pin command and exits 0, so a PR is never blocked by its own improvement.

**Three verbs fixed at the verb rather than at their callers**, which answers ~48 bare call sites
at one home: `is_scalar`, `Type::heap_def_nr`, `Type::heap_dep`.  Measured with
`scripts/introspect_diff.sh`: **DIFFERENT 9 of 1411**, every one a null guard, every one identical
in VALUE on both backends and under `LOFT_STRICT_STORES=1` / `LOFT_POISON=1` / `LOFT_STORES=warn`.
Eight now take the copy-or-adopt lowering their DENSE twin already took — `heap_def_nr` is
`(B-Copy)`'s home and loft#1319 had cured only the local-to-local site, leaving the call-result and
fn-ref-result sites bare — and the ninth moves a hoisted nullable scalar's declaration into the
native prologue, which is where batch 3's own cell said it belonged.  `heap_dep` alone reads
IDENTICAL 1411/1411: insurance, on batch 2's own argument for peeling its four `scopes.rs` callers.

**⚠ `is_dbref` is the one OPEN exception, and the measurement is why.**  The prediction written
before the probe was 10–30 files; it moves **102** and breaks **12** guards, because at least
thirteen callers read its blindness as a NULLABILITY test — they ask *"is this a non-null heap
slot?"* and take the answer from a SHAPE predicate's missing arm.  Two of four predictions in
`DESIGN.md` were wrong and both wrong ones changed the plan, which is the probe earning its keep.
One caller is named and cured WITHOUT peeling the verb: `nstore_null_report_as`'s `heap_target`
tests the synthetic `__nullable<S>` spelling of `τ?` explicitly and left the `Type::Optional`
spelling to `is_dbref` answering false — one notion, two spellings, one of them looked for.
Spelling the second is byte-identical today and makes the gate's reason local.  A second site,
`coroutine_layout::channel_0_carries`, is the rule broken INSIDE ONE EXPRESSION: `Text` peels, the
handle test did not, the scalar list peels.  Also byte-identical, and the clearest single
illustration of why the rule is worth naming.

**Guards.**  `data::tests::every_shape_verb_answers_alike_for_a_nullable_type` is `(N-Shape)` as an
executable property — one τ per storage class the model has, each asserted against its `τ?` for
all five verbs — and it goes RED naming the verb and the type when a peel is reverted (measured).
`is_dbref_is_the_documented_exception_to_shape_agreement` holds the exception open and FLIPS when
the caller split lands, so closing it cannot be forgotten.  A `.loft` guard would be inert here:
the change is behaviour-neutral by construction, which is the claim, so the cell has to be the
INVARIANT rather than a program.

**Two of the audit's rows are FALSE POSITIVES**, recorded so the baseline is honest rather than
merely frozen: `borrow_deps` and `rewrap_deps` name `Rewritten` with no `Optional` arm but delegate
to `deps_ref` / `with_deps`, which are dep-transparent by construction.  Two more are DELIBERATE
and must stay bare: `is_unknown` (106 bare callers — phase 0 F1 settled that a wrapper over a stub
IS a written type at the settledness guards) and `find_fn` (`(F-Recv)` keys two overloads apart BY
the `?`).

Filed rather than fixed, and NOT a null defect — the dense control is what says so: **loft#1467 — a generator
yielding a tuple with a store-handle member emits its correct refusal AND a spurious rustc
`E0308` beside it**, so the author gets loft's own message with a codegen dump attached, which is
the outcome loft#1132's write-up says must not happen.  Pre-existing, dense and nullable alike;
found because the first cut of this batch's yield probe had no dense control, and adding one
killed a finding that had looked like a nullability defect.

## Phase 5 — opened: the value spelling of absence has one home (2026-09-06)

The opening cell was loft#1374, filed by phase 4's return-delivery matrix: an absent element
handed back as `S?` read PRESENT at the handle test and garbage after the bind.  The matrix
for it (`~/workspace/pln153-scratch/stage5/cells`, written before the first run): four sources
of a slot-spelled absence — a vector index past the end and before the front, a `hash`, a
`sorted` and an `index` miss — across nine sinks (an inferred bind and its `==`/`!=`, a
declared local, a `S?` argument, a `-> S?` return by tail, by inferred and by declared local,
the return's bind and its field, a copy-bind, an inline test, a field read), with a
`reference<S>?` field and every source PRESENT as controls, on both backends under strict
stores and poison.  Every `S?` sink of every source was wrong before; the controls were right.
Every cell reads by a VARIABLE index or an absent key, because a constant index is trusted by
contract and its overrun is a C80 null that no rule above the runtime sees (`(N-Index)`'s
EXCEPT) — measured beside the promised cells and found to answer the same `nullref`.

**Batch 1 — the READ mints the value.**  B6u split `@FR-L-Null` into two questions, and this
is the second one's home for a pointer: *what value means absent* in a reference that has
left its slot is `nullref`, and the point that value is minted is the read that names no
record.  `DbRef::or_null` is the one predicate; `vector::get_vector`, the two
`vec_get_or_raise` twins, `Stores::get_ref` and the exits of the interpreter's and the native
keyed lookup call it.  A runtime change on its own (the emission moves below are the two
parser halves' — `DIFFERENT 18 of 1289` against the pre-batch compiler: the two new guards,
fourteen record binds into a nullable local now null-aware, two tagged field reads now
through the tag), and every consumer that tested `rec == 0` still does, since `nullref` has
`rec == 0` too; the
`get_vector` doc had claimed the two out-of-range answers "read as the same absent value" for
a year while the code answered two.  The same walk found the record bind choosing its
null-aware form by the SOURCE's type alone on both backends, so `x = t.h[k]; y: S? = x` on a
miss allocated a record for `y` and copied nothing into it; `Variables::bind_admits_absence`
asks both sides for both emitters.  Two more readers of a `vector<S?>` element came out of the
tagged control cell: a variable index wrapped the tagged element `Optional` — the `τ??`
`(N-Idem)` forbids, which refused the program as a type change between passes and which the
phase-0 census could not see because it counted `Optional(Optional)` alone (it now counts an
`Optional` over the synthetic, with a hand-built cell as its control) — and the field read
`v[i].n` projected the payload without the discriminant, answering an absent element's field
as `0` (`(L-Null-Which)`: the receiver of a field read or a method call is not a slot and is
read through its tag).  Records: `formal/layout-history.md` D-layout-5 and D-layout-6.

**What remains of phase 5.**  The 44 `@FR-L-Null` citations are still mostly the PEEL half's
(`base()` before a shape question); the walk that reads each and either keeps it as a reader
of one of the two homes or removes it as a redundant spelling is the long tail, batch by
batch, each with a cell.

**Batch 2 — the issues the walks filed, closed with the rules as arbiter (2026-09-06).**
loft#1373 (a view stale once its container grows) is the first: `(B-Disturb)` listed three
place-ending events and an append is a fourth, so the fix extends the rule rather than working
around it — and the sentence that looked like it settled the question, `(B-View-Depth)`'s *"the
view survives a source realloc"*, rested on a guard cell that appends to the INNER vector of a
nested one and never measured the realloc of the container the view names.  Recorded as
D-bind-18; the collection-typed twin is loft#1377, filed rather than bundled because admitting
its type makes the advice fire over a copy the materialise arm does not make.

## Phase 6 — re-measured: the class did NOT fall, and a third of it is not null (2026-09-08)

**The verdict, by the project's own instrument.**  `Parser::nstore_unwrap_report` is registered
in `scripts/bug-review.py`'s `KEYSTONES` as phase 3's chokepoint, landed at #1366, so the payoff
check judges it rather than a hand-rolled regex.  It reads **NO EFFECT — re-open the premise**:
null/sentinel **14.1 % → 37.3 %**.  Section 2 agrees from the other direction — `RISING +10.4pp
vs peak 17.5 %`, the largest class in the newest band.  That is the THIRD consecutive no-effect
verdict for this class, after the 2026-08 collapse-×5 and the citation walk.

⚠ **The check abstains at the default band width and has to be pushed to 14 bands to speak**,
because bands are equal-width in issue NUMBER and none starts above #1366 until then.  Recorded
because the abstention is the honest reading of a three-day window: the pass is monthly for a
reason, and this number should be re-read next cycle before anything is built on it.

**The bias phase 6 found in the instrument, and fixed.**  A keystone whose plan also ran a SCREEN
for its own class cannot be judged on the raw share — the screen FILES that class's bugs into the
window that scores it.  Of the 35 null/sentinel bugs at #1366+, **21 are phase 4's own finds**.
The payoff check now prints both lines when a keystone names its plan: `14.1 % → 37.3 %` raw and
`14.1 % → 25.9 %` with @PLN153's own finds removed.  **It rose either way**, which is what makes
this a correction to the instrument rather than a rescue of the verdict — and it is now the fourth
entry in BUG_REVIEW.md's list of traps that produced a wrong answer first.

**The residual, read by hand — the fourteen the plan did not file.**

- **Five are null × STORE LIFETIME, and that is the mechanism this plan did not touch**
  (loft#1421, #1422, #1447, #1456, #1466).  Each is the same sentence: *the nullable spelling
  takes a different LOWERING from its dense twin, so every ownership decision downstream is posed
  a second time and answered once.*  A nullable heap local mints through a work-ref the dense one
  has no need of; a nullable element binding publishes a borrow on pass 1 and materialises on
  pass 2; a nullable capture keeps its build-time value where the dense twin reads its rebind.
  This plan gave the null RULES one home each.  It never asked the two SPELLINGS to share a
  lowering, and that is where the class still lives.  Filed as **@PLN160**.
- **Three are not null defects at all** (loft#1437, #1438, #1457): a `u32` above `i32::MAX`
  decoding negative, a narrow field's raw byte read through reflection, a `spatial` accepting a
  one-key subscript.  In each the ANSWER is null and the CAUSE is width or key arity.  The
  classifier is a title regex, so any bug whose symptom is *"answers null"* lands in this class —
  the share is therefore an over-count of unknown size, and reading the titles is the only way to
  see it.  **That alone is enough to say the 37.3 % should not be acted on as a magnitude.**
- **Two are holes in this plan's OWN payload** (loft#1404, #1450): the chokepoint is not asked at
  the assignment target, and a projection through a nullable receiver drops the `?` before the
  gate can see it.  #1404 is fixed; #1450's remaining leg is measured at 115 sites and is the
  plan's one open debt.
- Two are diagnostics rather than values (loft#1453, #1459) and two are other mechanisms
  (loft#1385, #1386).

**So the Done criterion is met in its second form**, not its first: the class did not fall, and
the residual names a mechanism this plan did not touch, with a follow-on plan filed for it.

## Phase ordering

0 before 1 (a probe that kills the census cheaply).  1 before 2 and 3 (the homes have to be
known before they are folded or moved).  2 and 3 are independent of each other.  4 and 5 can
interleave with 3 and are the long tail; 4's top tier is worth doing before 3's matrix is cut,
because the declaration-time sites it names are where the matrix's undischarged cells land.
6 last, and only after a bug-review window has passed.

## Open design questions

1. **Where exactly is "the one point every escape ends up" for a store?**  B6t counted ten
   `⇐` push sites and six admission lists.  Phase 3 either finds the one lowering they share
   or first folds them — which is a phase of its own if the count is right.
2. **Does the heap half share the chokepoint?**  A nullable reference into a non-null field is
   `OpSetDbRef`-shaped, a scalar is `OpSetInt`-shaped; loft#1313 gave the heap half its own
   gate.  One check with two emitters, or two checks?
3. **`N-Store` is a WARNING except for narrow widths.**  Whether the warning becomes an error
   at the freeze is COMPATIBILITY.md's one-directional question and is NOT decided here; the
   plan lands the check where a later flip is one line.

## Cross-arc dependencies

- **@PLN152** (`status:next`) — `??` and `!` reaching the narrow widths.  Its steps 5–7 read
  the narrow-width `N-Store` ERROR and its message; phase 3 must keep that refusal where it
  is and its text what @PLN152 advertises.
- **@PLN102** (finished) — the keystone decision B and the null-flow flip
  ([nullflow-flip-plan.md](../102-stability-contract/nullflow-flip-plan.md)); phase 2 folds
  the sites that flip left behind.
- **@PLN25** (finished) — the dense element model; `N-Dense`'s home is its storage decision.
- The rule-led walk records this plan continues: QUALITY.md B6p, B6s (the `optional`
  screen), B6u (`@FR-L-Null`), B7h (`@FR-L-Null-Tag`).

## See also

- [formal/types.md](../../formal/types.md) — the N rules; [formal/layout.md](../../formal/layout.md)
  — the L-Null rules; [formal/README.md](../../formal/README.md) § When to reach for this doc.
- [keystone-null-model.md](../102-stability-contract/keystone-null-model.md) — the decided
  representation; [DESIGN_DECISIONS.md § C90](../../DESIGN_DECISIONS.md) — the frozen table.
- [STABILITY_METHOD.md § The rule-led walk](../../STABILITY_METHOD.md) — the method; the
  2026-09-05 evaluation in the session record that put null apart from lifetime.
- [BUG_REVIEW.md](../../BUG_REVIEW.md) — the null/sentinel class rows this plan is measured by.
- `scripts/ir_walker_audit.py optional`, `scripts/matrix_axes.py`, `make falsify` — the
  instruments; `LOFT_LOG=type_timeline:<var>` and `LOFT_DUMP_TYPES=1` for a single cell.
- [@PLN153](https://github.com/loft-lang/plans/issues/153) — this plan's issue.
