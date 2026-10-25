<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN102 — The LOFT_NULLFLOW default-flip: staged conversion plan

> **Status: measured 2026-07-11, not yet flipped.** All five null-flow phases are landed +
> gated (`LOFT_NULLFLOW`, opt-in). The default-flip (make it default-on) is the one-way cutover.
> Flipping it against the full suite gave **24/2826 failures**; this doc enumerates them by
> category with the fix steps and the EXACT sites (measured by running the corpus under
> `LOFT_NULLFLOW=1` — reproduce with `LOFT_NULLFLOW=1 loft <file>`).
>
> **D1 LANDED (2026-07-11, commit 21ec7837).** Category D is closed in-tree by two source-level
> language fixes (no consumer `?? d` churn): a computed integer-arithmetic index over constants +
> active loop vars (`m[k*4+row]`) now types non-null (`index_provably_fit`/`index_arith_trusted`),
> and a custom iterator's `next(self) -> Item?` binds the loop variable non-null (`for_type` peels
> the terminator `?`). This also removes the mat4-multiply friction from E's numeric libs at the
> source. Remaining: A, B, C, E, F, then the flip.

## The categories at a glance

| | Category | Fix | In-tree sites | Scope |
|---|---|---|---|---|
| **A** | bare `text as numeric` → error (N-Cast, Phase 4) | **DONE — `as τ?`** | ~~44~~ **43 done**, F5 at flip | this repo |
| **B** | nullable returned/stored into a non-null slot → warning | `?? d`, or declare `τ?` | 18 (5 files) | this repo |
| **C** | `possibly-null as non-null` (e.g. `float? as integer`) → error | `as τ?` or `?? d` | 18 (5 files) | this repo |
| **D** | matrix / vector arithmetic `v[i]*v[j]` → `τ?` (DN3-index × N-Prop) | **DONE — range-track (D1)** | ~~3 (2 files)~~ **0 in-tree**; helps EXTERNAL | this repo + range-tracking |
| **E** | external registry libs break | republish | ~9 libs | loft-libs-* (out of repo) |
| **F** | goldens shift (new warnings) | regenerate | 2 test files | this repo |

The one-way blocker is **E** — the suite cannot go fully green in this repo until the registry
libs are republished. A/B/C/F are tractable in-repo; **D** is where a *language* fix (better
vector-index range-tracking) beats a mechanical `?? d` sweep.

---

## Category A — text-parse assertion (`text as numeric` → error) — DONE (commit ccbac29f)

**Cause.** Phase 4: a cast `as τ` is an assertion, and a text parse can't be proven, so bare
`s as integer` / `s as float` is a compile error. **Fix used: uniformly the checked `as τ?`** — it
is faithful because the pre-flip DN3 N-Parse ALREADY auto-wraps `text as numeric` to `τ?`, so the
gate-off type is unchanged AND the null-on-bad-parse test cases (`!("abc" as integer?)`) keep their
meaning. (`?? d` would force non-null and destroy those cases; only reach for it where a *non-null*
value is genuinely needed.) Verified each file clean on both gates.

**Sites (44) — 43 converted:**
- `tests/scripts/01-integers.loft` (7) · `03-text.loft` (6) · `25-tuple-nstore.loft` (4) ·
  `52-single.loft` (2) · `02-floats.loft` (1 A + its 2 C `float?`-source casts, see below)
- `tests/docs/03-integer.loft` (2) · `05-float.loft` (1). **`16-parser.loft`/`15-lexer.loft`/
  `lib/parser.loft`/`lib/docs.loft` needed NO direct edit** — all four number-parse casts live in
  the shared **`lib/lexer.loft`** (`int`/`long_int`/`get_float`/`get_single`, each returns `τ?` and
  null-checks `result`), so fixing it once cleared every consumer.
- **`features/F5.loft` (1) — DEFERRED to the flip landing.** It is GENERATED from the canonical
  `loft-lang/features#5`; the drift guard forbids editing the generated file, so the fix is an
  ISSUE edit (`"42" as integer` → `as integer?`) + `make features-fetch && make features-gen`.
  Bundled with the flip's other outward steps (E republish, contract bump).

**Bonus (commit 60e2937c) — the checked cast now accepts a nullable SCALAR source.** `float? as
integer?` used to report "Unknown cast" (the per-type `OpCast` is base-keyed); it now peels the
`Optional`, letting the runtime propagate the in-band null. This also pre-clears the category-C
`float?`/`single? as integer` sites — a `?? d` sweep is NOT needed for them, just `as τ?`.

---

## Category B — nullable stored into a non-null slot (warning) — MOSTLY FIXED (commit cf87333f)

**Cause.** A nullable value (div / `sqrt` / … result) flows into a non-null return. Full-width types
(`integer`/`float`/…) WARN (compile + run); this is the nudge, not a break.

**The named-constant-divisor B sites are now FIXED at the source** by the divisor const-fold
(commit cf87333f — see § E). `audience_crystal` `connect_factor`/`fill_factor`
(`x as float / CONNECT_TICKS as float`) went from 3 warnings → 0: the compiler now const-folds a
named-constant divisor to its nonzero literal, so the division is non-null — closing the exact gap
the design's `PI`-style constant subset anticipated. This was the same fix graphics-0.3.0 needed, so
one compiler change cleared both.

**One genuine nudge remains, correctly:** `testlib::distance` — `sqrt(self.x*self.x + self.y*self.y)`.
The sum-of-squares is always ≥ 0 but the compiler can't prove it (full range-tracking is deferred by
design), so `sqrt` is honestly `float?`. It can't be discharged cheaply (`?? d` is redundant on the
gate-off path, a current-state regression) and nothing fails on it (`library_suite` checks exit code
+ crash/`test result` markers, not warnings — verified pass under `LOFT_NULLFLOW`). Left as the
correct N-Warn nudge until range-tracking lands. Same class as hex_terrain's `sqrt(max(x,0.01))` /
`sqrt(dx²+dy²)` (§ E).

---

## Category C — cast of a nullable to a non-null scalar (`float? as integer`) — DONE (commit 4ee284e5)

**Cause.** DN5: a `possibly-null` value cast to a non-null scalar. **Fix used:** `as τ? ?? d` (the
checked cast — enabled for nullable scalar sources by 60e2937c — discharged with a default; this is
clean on both gates because `as τ?` is always nullable so the `??` is never redundant).

**Sites — all cleared:**
- `lib/audience_crystal/src/audience_crystal.loft` (4): the two `(ray_len_d / branch_spacing_world)
  as integer` fork counts → `as integer? ?? 0`; the `cdx[kk]`/`cdz[kk]` vertex casts (kk is a
  reassigned local, NOT covered by D1) → `as single? ?? 0.0f`. `cdx[gh_k]` (loop var) is already
  non-null via D1. All 3 crystal test files `use` this source, so they move with it.
- `tests/scripts/02-floats.loft` (2) — done under A.
- **Also surfaced + fixed:** `tools/audience-demo/server_kernel.loft` (2 bare `payload[..i] as
  integer` text-parses → `?? 0`), which broke `engine_host_audience::kernel_port_matches_original`.

Verified: `engine_host_audience` + `library_suite` pass under `LOFT_NULLFLOW`; default suite green.

---

## Category D — matrix / vector-arithmetic friction (`v[i]*v[j]` → `τ?`)

**Cause.** A possibly-OOB vector index is `float?` (DN3), and N-Prop propagates it through the
multiply, so `sum += v[i]*v[j]` fails (`cannot change type … to …?`). **This is the one category
where a `?? d` sweep is the WRONG fix** — the honest fix is better range-tracking so a
provably-in-bounds index (`m[k*4+row]` with bounded `k`,`row`) stays non-null.

**In-tree sites — DONE (2026-07-11, commit 21ec7837).** Both cleared by the D1 language fix, no
`?? d` churn, verified on both backends + both gates (`tests/nullflow_index_dn.rs` guards them):
- `tests/scripts/85-yield-resume.loft` — the mat4-multiply `ma.m[mul_k*4+mul_row] * mb.m[…]`: the
  computed index is now trusted non-null. Fix: `index_arith_trusted` in `src/parser/fields.rs`
  recurses through `+ - * / %` when every leaf is a constant or an **active loop var** (the
  matrix-indexing contract — trusted like a bare loop var; a real OOB still faults → null, @C80). It
  deliberately does **not** thread the `i < len(v)` guard through arithmetic (that proof is specific
  to `v[i]`, not `v[i*2]`).
- `tests/scripts/81-iterator-protocol.loft` — a DIFFERENT root cause the original triage lumped in
  here: a custom iterator's `next(self) -> integer?` returns null as the loop **terminator**, so
  `for x in c` never binds null in the body, yet `x` typed `integer?` and N-Prop nullified
  `total + x`. Fix: `for_type` (`src/parser/control.rs`) peels the `Optional` off the custom-iterator
  element type; termination is structural (`parse_for_iter_setup`), so peeling is safe.

**External (the bulk):** `graphics-0.3.0/src/math.loft` (mat4 multiply) and any registry lib doing
matrix/vector math now benefit from the SAME fix at the source — the mat4 multiply no longer needs a
`?? d` sweep once republished (see E). Verify per-lib under `LOFT_NULLFLOW=1`.

---

## Category E — external registry libs — SUITE-BLOCKING PART DONE at the source; 2 libs need republish

**Audit (2026-07-11, all 14 installed libs under `LOFT_NULLFLOW=1`).** The blast radius was far
smaller than feared, and the biggest hit was fixed in the COMPILER, not by republishing:

- **graphics-0.3.0 — WORKS AS-IS, no republish.** Its two breaks were the mat4 multiply (cleared by
  **D1** — the `m[k*4+row]` matrix index) and `sd_ph = (… + sd_f / SFX_RATE as single) % 1.0f` (a
  named-constant divisor, cleared by the **divisor const-fold**, commit cf87333f — § B). Because
  both fixes are in the compiler, the *installed* graphics-0.3.0 now compiles clean under the flip.
  `p310_graphics_vector_ffi`, `moros_glb_cli_end_to_end`, and `p171_native_copy` all pass.
- **hex_world-0.1.2, gridmesh, hex_grid, glb, crypto, markdown, mesh3d, shapes, time, web — CLEAN.**
  (hex_world's per-file "errors" in the first pass were cross-module standalone-compile artifacts;
  its entry compiles clean.)

**Two libs still break — NOT suite-blocking (no failing test exercises them under the flip), so
they're ecosystem-completeness, republished from their loft-libs-* source (the consumer half):**
- **hex_terrain-0.1.0** (`loft-libs-world`): 4 sites, all `sqrt` of a provably-non-negative but
  not-compiler-provable arg — `sqrt(max(x, 0.01))`, `sqrt(dx²+dy²)`. Same class as `testlib::distance`
  (§ B); no cheap compiler fix (range-tracking deferred). Source fix: `?? 0.0` on the `sqrt` results.
- **server-0.3.0** (`loft-libs-net`): 3 bare `payload… as integer` text-parses (Category A). Source
  fix: `as integer?` / `?? 0`.

**Step E (remaining).** For hex_terrain + server: convert in the loft-libs-* source, bump version,
re-sign, publish (loft-library skill — touch-gated signing). Tracks the `registry-validation` CI leg.
The touch-gated signing is an owner action.

---

## Category F — goldens — IN-REPO PART DONE (commit 4c29c6fe); rest is F5-coupled

**Cause.** New warnings change diagnostic goldens. **The audit (2026-07-11) found exactly one
gate-dependent in-repo golden** — the rest of the "F" flip failures are F5, not wording shifts.

- **`tests/runtime_warnings.rs::wrong_field_guard_still_rejects` — DONE, made gate-robust.** It
  pinned the gate-off hard-error wording (`"cannot be stored into the return value"`); under
  LOFT_NULLFLOW the N-Store teeth relax to a WARNING for a full-width `text` return (`"is stored
  into the return value … becomes null there"`). Both contain `"stored into the return value of the
  non-null type"` — assert on that shared substring → passes on BOTH gates now (no flip-atomic edit
  needed). This is the pattern for any error→warning golden: match the shared wording, don't pin the
  verb.
- **`tests/features.rs::features_examples_interpret` + `tests/native.rs::native_features` — NOT
  golden shifts.** They fail only on the GENERATED `F5.loft` bare `"42" as integer` (Category A/F5).
  The `as integer?` fix is OUTPUT-NEUTRAL (prints `3 42 65` either way), so these goldens do not
  move — they simply need F5's issue edit + regen, which lands in the flip's outward bundle.
- **Audit of other pinned wording (no change needed):** `nullflow_phase1.rs` deliberately pins BOTH
  the gate-off `"cannot be stored"` and gate-on `"is stored into"` (it sets the flag per-case — a
  positive control, leave it); `threading_chars.rs`/`parse_errors.rs` `"cannot be stored"` are about
  CLOSURES/references (unrelated); `tests/scripts/102-expected-errors.loft` `@EXPECT_ERROR: … cannot
  be stored` still fire under the flip (NOT among the 8 flip failures — narrow/element stores keep
  the hard error). Output goldens are unaffected: the `as τ?` conversions print identical values.

---

## Ordering + the flip itself

1. ~~**D1 first (recommended)** — vector-index range-tracking, so numeric code (in-tree D + much of
   E) needs no `??`.~~ **DONE 2026-07-11 (commit 21ec7837).** In-tree D closed; E's numeric libs
   benefit at the source.
2. ~~**A, then B+C** in-repo.~~ **A DONE (ccbac29f), C DONE (4ee284e5), B mostly fixed by the divisor
   const-fold (cf87333f) — all 2026-07-11.** 43/44 A converted (F5 at the flip); C discharged with
   `as τ? ?? d`; B's named-const-divisor sites now non-null (one `sqrt` nudge remains, § B).
3. ~~**E** — republish the ~9 registry libs.~~ **SUITE-BLOCKING PART DONE at the source
   (2026-07-11).** graphics + hex_world + 9 others compile clean under the flip with NO republish
   (D1 + divisor const-fold in the compiler). Only **hex_terrain** (`loft-libs-world`, `sqrt` nudges)
   and **server** (`loft-libs-net`, text-parse) still need a source `?? d` + republish — neither is
   suite-blocking (§ E). Republish is touch-gated (owner action).
4. ~~**F** — regenerate goldens.~~ **IN-REPO PART DONE 2026-07-11 (commit 4c29c6fe).** The one
   gate-dependent golden (`wrong_field_guard_still_rejects`) is now gate-robust. The remaining F
   flip failures are F5-coupled (features/native) — they move with F5 below.
5. **The flip** — `nullflow_enabled()` → default-on with `LOFT_NO_NULLFLOW` opt-out
   (`src/keys.rs`); repoint the `tests/nullflow_phase*.rs` OFF cases to `LOFT_NO_NULLFLOW`; bump
   `CONTRACT_VERSION` 0 → 1 (`src/manifest.rs`). Land as ONE coordinated set so nothing stays red.

## See also

- [float-null-domain-typing.md](float-null-domain-typing.md) — the design + the phase implementation plan.
- [formal/types.md](../../formal/types.md) § Null-flow — the general laws.
- The five landed phases: commits 4b0c11e4, 5b3dd581, f8a307d7/0e135ab8/f582b6d8, c7caca54, 889e388b.
