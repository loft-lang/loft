<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN25 dense-default — RESUME HERE (cold-start handoff)

Single resume point for the dense-default value-model rewrite. Written so a fresh
session can pick up after a `/clear`. **Read order:** this file →
[full-design.md](full-design.md) (the consolidated design) →
[storage-vs-access-nullability.md](storage-vs-access-nullability.md) (the invariant +
probe verdicts) → [implementation-steps.md](implementation-steps.md) (the phase order) →
[`../../formal/types.md` § Nullability](../../formal/types.md) (the `N-*` rules).

---

## TL;DR — where we are (updated 2026-07-10)

> **The null/dense value model has LANDED on `main` (#480, 2026-07-02): `formal/types.md` is at
> 0 open deviations — DN1–DN6 all closed, default-on, both backends. F1–F5 are done.** What
> remains is close-out + a handful of deferred edge cases, none blocking the model — see
> § VERIFIED-OPEN RESIDUALS directly below. The F1–F5 detail is kept in § FINISH LINE as the record
> (that section predates the 2026-07-10 re-probe and is stale on flow-narrowing + `u8?`).

## VERIFIED-OPEN RESIDUALS (re-probed both backends 2026-07-10)

All are **deferred edge cases / close-out** — the value model itself is complete. Probed on the
current tree, so this supersedes the older FINISH-LINE notes where they disagree.

- **`not null` HARD parser-reject — open by design.** Still accepted as a silent no-op
  (`src/parser/definitions.rs`); a hard "retired" error is blocked on the registry republish
  (task #4 — graphics/web/gridmesh/crypto/cbor still carry `not null`, and green tests load them).
- **Call-arg (N-Store) hole — ✅ FIXED (2026-07-16, `callarg_nstore_enabled` / `LOFT_NO_CALLARG_NSTORE`).**
  The `n_store_violation` check now runs at the param-binding chokepoint (`process_call_args`), with
  the identical Phase-1 warn/error split — a nullable into a non-narrow param WARNS (the null binds),
  into a NARROW width hard-errors. Null-transparent fns (`min`/`max`/`clamp`/`abs`/… — they PROPAGATE
  null via `wrap_null_transparent`) are exempt; operators already dodge the path via the nullable-op
  swap. Blast radius (measured, both backends): 4 warning-assertion tests, fixed at the source — the
  stdlib `json_number(v: float)` → `float?` (its doc already says it handles NaN/null → `JNull`, so
  the `float` was the lie) + `call_with_null` migrated to a nullable param. Regression
  `tests/callarg_nstore.rs`. **With this + `?? null` closed, all three gate-2 SOUNDNESS edges are
  fixed; only the non-soundness close-out (`not null` registry hard-reject + F6) remains for gate 2.**
- **`?? null` unsoundness — ✅ FIXED (2026-07-16, `qq_null_typing_enabled` / `LOFT_NO_QQ_NULL`).**
  `a ?? b` now stays `τ?` when the FALLBACK `b` is itself nullable (a bare `null` literal, or a
  `τ?`-typed expression), so `y: integer = x ?? null` (and `?? <nullableVar>`) is REJECTED — the
  N-Store check at the consuming slot fires. Non-null fallbacks still discharge to the non-null
  base (`x ?? 0` → `integer`), and a chain discharges iff its LAST fallback is non-null
  (`x ?? (a/b) ?? 7` → `integer`). Fix in `operators.rs::build_null_coalesce_default` (report `*ctp`
  Optional; the code still builds with the base since the null sentinel is representable there) +
  the native `text_string_unify` predicate peels `Optional` (`emit.rs` — a `text?` if-arm has the
  same `String`/`&str` hazard). Blast radius (measured, both backends): five corpus sites relying on
  the `?? null` no-op, all corrected to the honest form (`as τ?` / `-> τ?`). Regression
  `tests/qq_null_typing.rs`.
- **`u8?`-return native codegen bug — ✅ FIXED (verified 2026-07-16).** `fn f(i) -> u8? { r = i as
  u8?; r }` no longer reproduces — `--native` runs and agrees with interp (`f(300)` → `null` on
  both). Re-probed on the current tree; closed off the residual list.
- **F6 — close-out**: the `Closes @PLN25` PR + `CHANGELOG` + the deviation-register confirmation.

**Removed as FIXED (2026-07-10):** the old "interp full-width vs native narrow-packed" `u8?`
FIELD/LOCAL divergence no longer reproduces — both backends now agree (255 in a `u8?` field reads
back `null` on both; a `u8?` local holds 255 on both). And **flow-narrowing works** — `if x != null
{ y: integer = x }` and the `== null`-else branch both narrow with no `?? d`, both backends; the
"Deferred" note in § FINISH LINE is superseded.

### ⭐ MOST RECENT — cold-start read this first (branch `tuxedo-pln85-fuzz-proof-gate`)

> **🟢 DN3-PARSE FLIP LANDED (2026-07-02) — the LAST types.md deviation is CLOSED; formal spec updated.**
> `text as integer` / `as float` / `as single` is a **fallible PARSE**, so it now TYPES `τ?`
> (`(N-Parse)`), exactly like `÷0` and `v[i]`. The `as` handler (`parser/operators.rs`) wraps a
> `Text`-source numeric cast in `Optional`; discharge with `?? d`, `as τ?`, or `match`.
> **Measured blast radius by running the full suite** ([[measure-flip-by-running-suite]]): 11 gated
> tests → all migrated + GREEN on both backends. Committed set:
> - **core:** `src/parser/operators.rs` (the parse-nullable wrap, under `pln25_dn1_enabled()`).
> - **lib:** `lib/lexer.loft:395` escape decoder `nr = total as integer ?? 0` (the `\x` cascade that
>   fed `nr as character` → 16-parser via `use parser`).
> - **suite-gated servers:** `tools/audience-demo/server_kernel.loft` (audience test kernel leg) +
>   `tools/audience-demo-50/probe_server_kernel.loft` (udp test) — protocol parses `?? 0`. The
>   audience **original leg auto-skips** when registry `server-0.2.0` rejects (`server_available()`
>   gate) → **no republish needed for green**.
> - **tests:** `03-text.loft`, `29-strings.loft` (`?? 0`); `strings.rs` loop_variable/string_scope
>   (`?? 0` + slot refresh — the `?? 0` adds an `ncc` null-coalesce block); `issues.rs` inc17.
> - **docs:** `formal/types.md` (DN3 → CLOSED), `formal/ROADMAP.md` (types.md **1 → 0**),
>   `LOFT.md` + `loft-write` skill (parse-is-fallible `τ?` guidance — user-asked).
>
> **Formal state: types.md is now 0 open** — @PLN25 DN1–DN6 + D2 all CLOSED. Overflow-arith stays a
> DECIDED EDGE (@C85, non-null, no trap). **types.md joins binding.md + grammar.md at 0.**
>
> **DEMO-TOOL SWEEP + SKILL DN1 PASS — DONE (`b32771ad` demos, `90023008` skill).** The `tools/*` +
> `bench/*` demos carried MULTI-DN stragglers (parse + division `a/var`,`a%var` + `v[i]` index); all
> LOCAL fit-op sites discharged (`?? 0`/`0.0`/`""`; `??` binds looser than `/`). COMPILE CLEAN:
> bench/08_word_count, crystal_stress, probe_client_kernel (indexer/scan already clean). REGISTRY-BLOCKED
> (bodies now DN1-clean; only registry-pkg errors remain, pending task #4): server + viewer/main
> (`web-0.2.1`), projector (`gridmesh-0.1.1`), brick-buster (`shapes`/`graphics`) — brick-buster is the
> gallery showpiece so `doc/gallery-examples.js` was regenerated. The **`loft-write` skill** DN1
> consistency pass rewrote every stale `not null`/"default nullable" line (scalars/tuples/vectors are
> non-null by default; `?` for nullable; `not null` = retired no-op; plain `boolean` 2-state vs
> `boolean?` 3-state; the vector-field-`not null`-to-silence-warning idiom is obsolete — that warning no
> longer fires). Each claim verified against the binary first. **The tuple-element gap the sweep
> surfaced is now FIXED (`4698ee52`): `n_store_violation` recurses ELEMENT-WISE** — it normalizes both
> sides to element lists (literal `Type::Tuple` OR the `Reference(__tuple<…>)` synthetic-struct rewrite
> a tuple RETURN type takes; elements = the def's attribute typedefs) and each element rides the
> existing DN3/DN1 checks ("element 0 of the return value", nested chains compose). Matrix both
> backends: return(tail/explicit)/field/nested/bare-null REJECT; discharged/`τ?`-element ACCEPT.
> Suite: ZERO new failures (the sweep's defensive discharges were exactly the now-required sites).
> Guards: 25-tuple-nstore.loft + 4 reject twins in 102. Call-ARG stays the known separate slice.
> **Formal addendum (`ad517fe1`): ROADMAP D1 (@PLN89 oracle) scope now includes DRIVER agreement**
> (accept/reject identical across --interpret/--dump/--native/--native-wasm; a reported --dump
> divergence traced to MIXED BINARIES — PATH loft 2026.6.0 predates DN1 — but the one-static-judgment
> property now has a guard home; investigation: routing/docs/loft-feedback.md § 2026-07-02).
>
> **⚠️ ANOMALY FIXED:** working tree had an uncommitted regression in the GENERATED `tests/docs/features/
> F1.loft` + `F2.loft` (`null as integer?` → `null as integer`, which DN5-rejects). Restored to the
> committed `null as integer?` (via `git show HEAD:` → write, NOT `checkout`). If `tools/features/gen.loft`
> reproduces the broken form, the GENERATOR needs the `?` — check before regenerating features.

> **🟢 CORE/IN-TREE SUITE NOW GREEN — the mid-step-f DN1/DN3 debt is cleared (2026-07-01b). Run the full
> suite via `find_problems`, NOT raw `cargo nextest`** (it rebuilds cdylibs + clears stale `.loftc`; a raw
> run shows stale-fixture FALSE failures). What was fixed this session (all committed + pushed):
> 1. **✅ store codec serializes `Type::Optional`** (`73fe4f6c`, `TyOptional` variant disc 25) — cleared
>    all 11 store-codec failures (`ir_read::*round_trip*`, `corpus_store_codec_round_trips`, `g2_ir_check`).
> 2. **✅ 9 DN1/DN3 Rust-test migrations + div-warning retirement + clippy** (`fa698799`): `expressions::
>    {call_int_null,call_text_null}` → `-> τ?`; `{bounded_mixed,bounded_unary}` → `?? 0`; the div/mod
>    runtime warning is RETIRED under DN1 (already in code — `emit_undefended_warning` early-returns for
>    Div/Rem; `runtime_warnings` + `exit_codes::div_by_literal_constant_no_warning` migrated to assert the
>    absence + the type/N-Store enforcement); `error_messages` goldens 17/18/19/28 regenerated; clippy
>    `handle_operator` allow.
>
> **REMAINING (NOT core — registry-gated + environmental, none is compiler DN1/DN3 debt):**
> - **Registry republish gates multiplayer(v2/v3/v5) + viewer_markdown** — the `web-0.2.1` (`web.loft:206
>   ws_client_recv_native → return null`) and `server-0.2.0` (`server.loft:180`) registry libs still
>   `return null` into `-> text`. The in-tree web *fixture* is migrated but the REGISTRY copies need the
>   `text?` migration + a **touch-gated republish** (loft-ship; canonical home loft-libs-net) — USER-GATED.
>   The in-tree `game_protocol` example server (`tictactoe_server_v2.loft`) also needs the `text?` sweep,
>   but fixing it alone won't green multiplayer (server-0.2.0 still rejects).
> - **Environmental**: `html_asyncify` (chrome timeout + a stale GL cdylib), `error_messages::38_import_unknown`
>   (sandbox DNS to the registry — golden left as-committed, passes on CI).
>
> So the compiler-side @PLN25 green gate is MET; the multiplayer/viewer suites wait on the registry republish.
> Detail: [index-f1a-landing.md](index-f1a-landing.md) § BRANCH IS RED.

**INDEX FLIP (Steps 2-6) IN PROGRESS (2026-07-02).** DONE + pushed: element-WRITE mechanism `v[i] = h`
peel (`81641e7d`, collections.rs:761); Step 4 issues p124/p155/p170 + 85-borrow (`b46413d0`, discharge
`?? d`); audience_crystal FULLY migrated (`cc7cb722`+`f6d1c8d2` — src 0 rejects under flip, library_suite
green); graduate test `25-index-nullable.loft` written. **DEFERRED DEPS GAP:** bare `-> Item?` escaping-
BORROWER return leaks (Optional-return × copy-elision; plain `-> S?` clean) — avoided via discharge.
**⚠️ CORRECTION:** an earlier "wrap PASSES → Step 2 done" was a DEAD-BACKGROUND-RUN measurement error
([[measure-flip-by-running-suite]]). **A full corpus sweep (`LOFT_INDEX_DEV=1 find_problems`) mapped the
REMAINING index-flip failures:** (1) **lib/code.loft** — 4 cast-from-nullable rejects `self.code[b] as
Block/If/If/Loop` (the loft-in-loft codegen, DELICATE; needs a `nullable as T` discharge idiom, keep DN5
closed) — lib/parser.loft is clean; (2) html_wasm moros_editor; (3) runtime_warnings
skip_loop_bounded_arithmetic (`sum += m[i*4+j]` loop-bounded ARITH index → migrate `?? 0.0`); (4)
loft_suite. Pre-existing/environmental (NOT index): error_messages 38 DNS, html_asyncify chrome,
multiplayer registry.

**lib/code.loft cast-from-nullable FIXED (`0cae0a6d`) — but a DEEPER runtime corruption surfaced.**
The 4 downcast sites `self.code[b] as Block/If/If/Loop` (b=blocks[len-1]) now use the GUARD idiom:
`b = self.blocks[len-1] ?? 0; if b < len(self.code) { bl = self.code[b] as Block; bl.field = ... }`.
The guard (NOT `?? default`) is required — it keeps `bl` a MUTABLE BORROW so the field writes write
back; a `?? <variant>` fallback LEAKS (proven). gate-OFF safe (wrap dir/last/parser_debug/wasm_dir +
loft_suite 5/5); lib/code.loft + lib/parser.loft compile 0 rejects under the flip. **⚠️ BUT clearing
the casts UNMASKED a DEEPER runtime corruption:** under the flip the loft-in-loft compiler produces a
GARBAGE token position ("Expected : on :281479271743489" at lib/lexer.loft:476) — a SILENT wrong-value
(it was a MEMORY corruption). **✅ ROOT-CAUSE FIXED (`9a5ec692`) — `Type` is now Optional-dep-transparent.**
The debug session pinned it: `Type::depend()`/`depending()`/`deps_ref()` recursed through `RefVar` but NOT
`Optional`, so a borrow whose type is `Optional`-wrapped (`e = v[i]`, or a field-append target
`self.types[in_type].type_fields += [Field]`) LOST its lifetime dep → the deps pass treated the element ref
as OWNING → spurious `OpDatabase` + the record constructed in the wrong slot, clobbering adjacent fields
(SIGSEGV / 0x0001_0001_0001_0001). Fix: recurse through `Optional` like `RefVar` (deps are a lifetime
property, agnostic to the nullability marker). gate-OFF byte-identical; issues 748/0. Minimal repro that
pinned it: appending a struct-with-a-heap-field to the vector field of an Optional element (scratch:
structapp.loft). Then the doc/fixture consumers migrated (`e8fb2019` 25-generics `-> T?` + p248_pkg;
`a40fb977` moros_editor) + `doc/examples.js` regenerated (`1a746f9b`, doc_hygiene green).
**RESULT: wrap dir/last/parser_debug/wasm_dir + loft_suite 5/5 BOTH gate-OFF and under LOFT_INDEX_DEV=1.**
**REBASED (2026-07-02) onto fetched origin/main @ 64a1be71** (#476 html modules + #474 @PLN92; 55 commits
replayed clean; force-pushed).

**✅✅ INDEX FLIP COMPLETE — Step 6 LANDED (2026-07-02): `v[i]`/`s[i]` type `τ?` by DEFAULT.**
Gate flipped (`74fb229d`, fields.rs now just `pln25_dn1_enabled()`); the VectorIndex/TextIndex fault
warnings RETIRED (joining Div/Rem — the type + N-Store is the enforcement); graduate `25-index-nullable.loft`
(accept: const/iter-var/guard/`??`/honest) runs in loft_suite; `102-expected-errors.loft` undefended-`v[i]`
reject twin added (`9951d9ca`). **FULL DEFAULT SUITE GREEN** — the only failures are pre-existing
ENVIRONMENTAL: multiplayer_v2/v5 (registry republish of web/server, task #4) + html_asyncify (chrome).
runtime_warnings 44/0, error_messages 2/0 (22_runtime_index_negative baseline regenerated), clippy clean.
**⚠️ CONSEQUENCE: `#null_safe` (@PLN46 W2/W3) is SUPERSEDED under DN1** — its only effect was suppressing
these fault warnings (all four now retired), so it is moot; the affected runtime_warnings tests are migrated
to assert the retirement (and wrong_field_guard → the stronger N-Store REJECT). A follow-up should formally
deprecate/remove #null_safe. **NEXT for @PLN25: F2 retire `not null` · F3 DN5 · F4 DN6 · F5 DN4 cutover · F6
final PR (Closes @PLN25)** — see § FINISH LINE. (The index flip was the last fault-op source; the model's
fault-op TYPES are now complete.) Full detail: [index-f1a-landing.md](index-f1a-landing.md).

**✅ INDEX F1a Step 1 (copy-elision `Optional`-peel) DONE** — `use_analysis` borrower filter now peels
`.base()` before reading deps, so a mutated/escaping `e = v[i]: Item?` KEEPS the copy (was a SILENT
leak-to-source). Boundary matrix + stress, both backends, both gates, leak-clean; safe-by-construction
for the default suite (only ever refuses a plan = keeps a copy). Regression: `25-index-elision-borrower.loft`
+ `pln25_dn1_consumption::index_dev_elision_borrower_*`. Detail in [index-f1a-landing.md](index-f1a-landing.md) § Step 1.

The **DN1 flip is DEFAULT-ON** (`LOFT_PLN25_OFF` opts out; gate-OFF is now DEAD — F1b(b)'s `τ?`
min/max overloads collide with it). Full detail per topic is in **§ FINISH LINE → F1a/F1b** below.
Latest work, newest first (all on this branch, both backends, suites green unless noted):

1. **F1b(b) CLOSED (`e7c0f17b`)** — min/max/clamp non-null bodies cleaned + STD_SOURCE (N-Store)
   exemption REMOVED; 7 issues field-null tests + 17-min-max-clamp migrated to `τ?`.
2. **DN3-division ROOT-CAUSE FIXED (`7042d94c`)** — the `ecd4cab3` "division slice" wrap was DEAD
   CODE (wrong `handle_operator` branch); MOVED to the arithmetic branch, so `a/b`/`a%b` finally
   type `integer?`. Closed the reported gaps #1 (const-fold) + #2 (via-var). Else-side divisor
   narrowing added. Regressions: `25-division-nullable.loft` + a `102` reject twin.
3. **INDEX (`v[i]`) FLIP — foundation built + measured, DEV-GATED, landing BACKED OFF (`c58d7623`).**
   Mechanism + 3 fit-proofs (const via `const_int` / iter-var via `vars.is_active_loop_var` / `if
   idx<len(v)` guard) all work under `LOFT_INDEX_DEV`. **Do NOT just flip the gate + migrate 8 sites
   — the reject-count measurement UNDERCOUNTED.** Gate-ON breaks 6 wrap tests incl. a SILENT
   copy-elision wrong-answer. Landing = a proper F1a-style phase (see F1b index note for the 6 steps).

**Two known DN3 holes, both DEFERRED as their own slices (see F1b notes):** gap #3 (`??` discharges
regardless of fallback nullability — `x ?? null` unsoundly non-null); and N-Store is NOT enforced at
CALL ARGS (`takes(v[j])` doesn't reject — affects division equally).

**NEXT SESSION = the index F1a landing phase.** Step-by-step instrument (with the exact copy-elision
starting points, the 6-test failure map, and the migration inventory) is in a dedicated doc:
**[index-f1a-landing.md](index-f1a-landing.md) — start there.** Step 1 is the load-bearing +
riskiest piece: the copy-elision `Optional`-peel in the deps subsystem (`use_analysis::ElidePlan` /
`scopes`), proven on the 85-borrow read/mut/esc matrix, both backends. (Deferred alternatives if you
want a smaller unit instead: **gap #3 `??`-typing**, or the **call-arg N-Store** slice.) Then F2–F6.

- **PR #471 MERGED to `main` (squash `03d8899f`, 2026-07-01)** — the whole scalars-half:
  `Type::Optional` + slice 3a/b/c (N-Store) + the gated DN1 flip + enforcement + sweep(1)
  lib/lexer. All behind opt-in `LOFT_PLN25_OPT` / `_DN3` / `_DN1`; gate-OFF byte-identical.
  Branch `tuxedo-pln85-fuzz-proof-gate` reset onto merged main; new sweep work stacks on it.
- **Sweep (2): web.loft DONE + 3 read-peels harvested (`c9f512c5`).** web `try_recv` →
  `-> text?` (repo fixture + zt-c staging copies; registry needs a republish for the
  multiplayer DN1 suite). The migration surfaced the consumer ripple: text? READ sites
  (index/slice/format) didn't peel `Optional(Text)` like method dispatch does — fixed all 3
  (parser index+format, native `&str` borrow), both backends, gate-OFF byte-identical. Every
  text? READ (method/index/slice/format/fn-arg) now behaves as plain text. The ONE remaining
  consumer pattern — `got = raw` reassigning text? into an existing `text` local — is
  correctly REJECTED (maybe-null into non-null local); discharge with `?? ""`. **loft has no
  flow-narrowing → that is the eventual ergonomic chokepoint (separate feature).** Regression:
  `tests/scripts/25-nullable-read-consumption.loft` + `tests/pln25_dn1_consumption.rs`.
- **Sweep (4): the null-using test scripts migrated to `?` (`7b13f5cd`).** ~13 DN1-failing (not
  ~5). Fix = `?` on the DECLARATION (return type / struct field) that holds null; gate-OFF
  byte-identical. **9 fully DN1-green** (08-functions, 81-iterator, 79, 84, 91, 299, 32,
  389-narrow-alias, inline-construct); **2 migrated but with a DN1-native-only codegen gap** (kept +
  committed, gate-OFF green): 389-h6 (native full-range nullable narrow-int FIELD in a `vector<struct>`
  not widened to 2-byte → max value swallowed) and 407 (native `character?` null-sentinel E0308);
  **2 reverted pristine, blocked** on the scalar-`vector<τ?>` slice (292, flagship 25-nullable-sequences).
- **DN1 gaps surfaced by the sweeps (all plan-internal, DN1-gated, NOT GH issues):** (i) ✅ **DONE
  (`73c45ade`)** — scalar `vector<τ?>` element-null: `e2_nullable_elem` now wraps scalar elements in
  `Optional` (OPT-gated, byte-identical gate-OFF; storage unchanged — sentinel null + Optional peels).
  Unblocked the flagship + 292 (both now fully DN1-green). (ii) ✅ **DONE (`af7d80eb`)** — native
  full-range nullable narrow-int FIELD in `vector<struct>`: `emit_field` (generation/mod.rs) missed
  `Optional(Integer)` so the struct sized to 8 bytes (2 on interp) → vector appended at 8-byte stride,
  read at 2-byte → elements past index 0 read null. Fixed by folding Optional into `nullable` + peeling
  `typedef` to base in emit_field. Unblocks 389-h6 + 407. (iii) ✅ **DONE (`08f3837d`)** — native
  `character?` null-sentinel E0308: peel `.base()` on the four char-wrap type checks in calls.rs. **ALL
  THREE gaps closed; every sweep(4) script is fully DN1-green on both backends; NO native gap remains.**
  Also: the web consumer `got = raw` store correctly rejects (no flow-narrowing — the eventual
  ergonomic chokepoint, a separate feature).
- **Step f (the flip) is prototyped + measured.** `keys.rs` default-on (`LOFT_PLN25_OFF` opts out)
  works; the invalidation catalogue + per-case mitigation (auto-fix vs precise-diagnostic vs
  semantic-fix) is [DN1-MITIGATION.md](DN1-MITIGATION.md). Landing needs: the `change_var` local-null
  message fix (it wrongly suggests `as`), the stdlib min/max/clamp migration + `STD_SOURCE`-exemption
  removal, and the two newly-tracked deviations **DN5** (`as` launders `null`/`τ?` into a non-null
  scalar — the nullness sibling of DN4) + **DN6** (inferred `a = null; a = 5` should widen to `τ?`
  per `(N-Join)` but rejects). Both are "enforcement incomplete", closed AFTER the flip lands.
- **`lima-default-borrow-elision` is MERGED to `main`** (via #467/#468/#469 etc.) and the
  branch is deleted. Its scalars Phase-0 + DN4 + N-Arith work is now ON `main`.
- **@PLN25 now continues on `tuxedo-pln85-fuzz-proof-gate`** (the single live branch, off
  `main`, pushed, no PR). Step 1 (`Type::Optional`) landed here (`d121f94c`).
- **Suite green** — `find_problems.sh` 0 failures on both backends after Step 1.
- **The vectors half is DONE and on `main`.** `vector<S>` is dense (`main_vector<S>`,
  no `__nullable`); `vector<S?>` is the nullable opt-in; `v[1] == null` is true; the
  canonical incoherence probe is coherent on both backends. The #465 borrowed-view
  over-free is fixed. Merged via `#412` (gate flip + keyed-dense), `#467` (dense
  vectors + copy-vs-borrow elision, vectors-green checkpoint), `#468` (borrow elision
  default-on + Tier-1.5).
- **The scalars + TIGHTEN half is IN-FLIGHT on this branch:**
  - **Scalars Phase 0 (EXPAND) — done.** `integer?` / `text?` / `S?` parse in every type
    position (decl, param, return, `as`-cast, nested). Today a no-op (plain types are still
    nullable). Regression: `tests/scripts/25-scalar-optional-syntax.loft`.
  - **DN4 (narrowing-cast enforcement) — done, UNCONDITIONAL** (F5 cutover 2026-07-02 retired
    the `LOFT_NO_DN4` opt-out). A narrowing integer cast of a not-provably-fit value is a compile
    error; `as τ?` is the checked form. DN4 is integer-range-only, needs none of the scalar
    `τ?` representation, so it **shipped ahead of the scalar default flip**. The error
    baseline caught a real silent overflow (`big as i32` was `10000000000`). Regression:
    `tests/dn4_cast.rs` (3) + the `tests/scripts/389-narrow-*` family.
  - **N-Arith range-tracking — done.** `&` and `%` narrow the static range so masked/modded
    values are provably-fit (feeds DN4's fit proof). Regression: `389-narrow-alias-ranges`,
    `389-narrow-vector-full-range`.
  - **Scalar `τ?` representation — BUILT (Step 1, `d121f94c`).** `Type::Optional(Box<Type>)`
    is in `src/data.rs` with the idempotent `Type::optional` former (N-Idem + normalises
    `Optional(Never|Null)`) and `peel_optional`/`base`. 8 exhaustive `match Type` sites
    handled (peel for the layout-agnostic majority; `τ?` rendering in `name()`). Compile-time
    only, sentinel storage, nothing constructs it yet → additive, suite unchanged. N-Idem
    pinned by a unit test. Design: [scalar-optional-representation.md](scalar-optional-representation.md).
- **gridmesh 0.1.2 published (2026-06-29).** The DN4 cutover masked the gridmesh + hex_world
  fixtures and relocked `audience_crystal` to gridmesh 0.1.2; the suite was RED until that
  version was signed into the registry index. It is now published + signed (registry commit
  `056c08c`), which is why the suite is green.

## The one invariant (what the whole rewrite installs)

> `vector<τ>` is dense and uniform for every `τ` (incl. generic `N`); nullability is
> carried only by an explicit `τ?`; lookup-partiality only by the fallible ops
> (`v[i] ⇒ τ?`, etc.). No implicit container rewrite, no implicit unwrap. (The integer
> model applied to null — one former, representation derived.)

---

## Landed ledger (what is true on this branch, both backends)

| Area | Phase | State |
|---|---|---|
| Vectors `vector<S?>` | 0–2 EXPAND/MIGRATE/CONTRACT | ✅ on `main` (`#467`) |
| Borrow elision Tier-0 + Tier-1.5 | — | ✅ on `main`, default-on (`#468`) |
| #465 borrowed-view over-free | 4 | ✅ on `main` |
| Scalars `integer?`/`S?` syntax | 0 EXPAND | ✅ this branch (no-op) |
| DN4 `as τ` fit-check / `as τ?` | 3 TIGHTEN (early) | ✅ this branch, default-on |
| N-Arith `&`/`%` range-tracking | 3 support | ✅ this branch |
| Scalar `τ?` = `Type::Optional` | 0/2 prereq | ✅ Step 1 built (`d121f94c`, `tuxedo-pln85`) |
| Scalar `(N-Store)` teeth (return/field/typed-store/index) | 3c CONTRACT | ✅ gated, on `main` (`#471`) |
| DN1 flip + enforcement (opt-in `LOFT_PLN25_DN1`) | 3 CONTRACT | ✅ gated, on `main` (`#471`) |
| text? read-peels (index/slice/format) | 3 support | ✅ this branch (`c9f512c5`) |
| scalar `vector<τ?>` element-null | 3 support | ✅ this branch (`73c45ade`) |
| native `character?` null-sentinel | 3 support | ✅ this branch (`08f3837d`) |
| native narrow-field struct-size in `vector<struct>` | 3 support | ✅ this branch (`af7d80eb`) |
| sweeps: lexer / web / 13 test-scripts migrated | 3e | ✅ this branch (all DN1-green) |
| `change_var` null-local message (names `τ?`, no `as`) | 3f step b | ✅ this branch (`85af2b18`) |
| the flip default-on (`keys.rs`) | 3f / step f | 🟡 **prototyped + measured, WIP uncommitted** |
| Borrow elision Tier 1 (local source) | — | 🔵 implemented, **parked off** by design |

---

## FINISH LINE — the remaining steps to CLOSE @PLN25 (2026-07-01)

The whole model is **built + gated + validated on both backends** (scalars `Optional`,
`(N-Store)` teeth, `vector<τ?>` incl. scalar elements, text?, character?, narrow-field,
DN4 range-check). What remains is **landing the flip as the default + the cleanup
tightenings**. In order, each ends green (never carry two phases' breakage):

- **F1 — Land the flip (make DN1 the default).** `keys.rs` default-on (`LOFT_PLN25_OFF`
  opts out) is prototyped + measured (WIP, uncommitted). **The FULL suite is GREEN under the
  flip** (nextest 2583/0; `wrap` 51/0, `issues` 748/0 cross-checked), with the `STD_SOURCE`
  exemption still scaffolding the stdlib — so the flip is suite-green and nearly landable.
  - **F1a — ✅ DONE.** The five flip red tests are green: `wrap::{dir,last,parser_debug,wasm_dir}`
    + `issues::p54`. Roots + fixes (all gate-OFF byte-identical, both backends): the loft-in-loft
    compiler ripple (`lib/parser.loft` lexer-`text?` discharges + `lib/code.loft` `cur_def`
    `?? 0` — `e29cb6f6`); `to_default` base-zero for `Optional` fields (native `(())`→scalar —
    `e29cb6f6`); `time.loft` parse/combine `-> integer?` (`a65a28e2`); the nested-exhaustive-match
    false-widening (`arm_yields_direct_null` — `d5402391`). (`31-ref-forward` was environmental —
    registry DNS.)
  - **F1b — ✅ DONE** — the six non-null `min`/`max`/`clamp` bodies are CLEAN (dead-under-DN3
    null-prop `if !a||!b {return null}` dropped; a nullable arg — explicit `τ?` OR a DN3-typed
    division result — routes to the `τ?` overload instead), and the **`STD_SOURCE` exemption is
    REMOVED** from `n_store_violation`: the stdlib is now held to the SAME DN1 rule as user code.
    Fallout migrated: 7 `issues.rs` field-null tests were pre-DN1 (non-null scalar field `= null`,
    kept alive ONLY by the source-0 quirk of `code!`) → declared `τ?` so they exercise the real
    DN1-reachable null-sentinel roundtrip; `tests/scripts/17-min-max-clamp.loft` null-prop section
    rewritten to explicit `τ?` inputs + one runtime-division case (`nulldiv(0)`). issues 748/0,
    wrap 51/0, format 11/0, both backends.
    - **DN3-division ROOT-CAUSE FIXED (`7042d94c`): the `ecd4cab3` "division slice" wrap was DEAD
      CODE.** It sat in the COMPARISON-operator branch of `handle_operator` (the `else` handling
      `<`/`<=`), where `div_nullable` (checking `operator == "/"`) is always false — so `a / b`
      NEVER actually typed `integer?` in ANY context. Everything that "worked" (`ret`/discharged/
      inferred `== null`) rode the runtime sentinel + declared `integer?` return types; the claimed
      "undefended `a/b` into non-null → rejects" was never real (dn3div never called the `bad` fn).
      **Fix = MOVE the wrap to the ARITHMETIC branch** (the `} else {` where `/`/`%` route through
      `call_op`): capture `div_nullable` before `call_op` consumes `second_code`, wrap `*ctp` after
      the range-narrowing. Now `a / b` types `integer?` everywhere → `(N-Store)` catches undefended
      stores. This ONE move closed **gaps #1 (constant-fold) AND #2 (via-var)** — both were the same
      dead-wrap root, not two bugs. Plus the divisor narrowing gained the ELSE side: `if v == 0 { … }
      else { a / v }` now proves `v != 0` in the else (was THEN-only) — `divisor_proof_from_condition`
      → `Option<(u16, bool)>`. Real blast radius was ONE site (`83-return-in-if-expr` `safe_div_true`,
      the else-idiom — fixed by the else-narrowing, not migrated). Regressions:
      `tests/scripts/25-division-nullable.loft` (accept: const/mod/then-guard/else-guard/`??`/honest,
      both backends) + a reject twin in `102-expected-errors.loft`. issues 748/0, wrap 51/0, full suite clean.
    - **REMAINING gap #3 — `??` discharges regardless of the fallback's nullability** (`handle_null_coalesce`
      operators.rs:1366 `*ctp = ctp.base()`). `x ?? null` (and `x ?? nullableVar`) types `integer`
      though it can be null → `y: integer = x ?? null` unsoundly accepts + holds null. This is the
      `??` OPERATOR, not division; the correct rule is "result is `τ?` iff the fallback is nullable,"
      but flipping it has its own blast radius (every `a ?? nullableVar` currently discharges). `?? null`
      itself is a pathological no-op idiom. Deferred as a distinct `??`-typing slice.
    - **INDEX (`v[i]`) FLIP — foundation DONE + measured, DEV-GATED (`c0aad2b2`).** `v[i]` types `τ?`
      (OOB → null sentinel), the last fault-op source. INERT by default (gate `LOFT_INDEX_DEV`; default
      suite byte-behaviour unchanged) while landing. Mechanism: `parse_vector_index` sets `last_index_fit`,
      `parse_index` wraps the element `Optional` when unfit. Fit-proofs (`index_provably_fit`, matching the
      warning walk's skips): non-negative constant; for-loop iter var (via `vars.is_active_loop_var` — no new
      stack); `if idx < len(vec)` guard (new `index_bounded: Vec<(u16,VecKey)>` fed by REUSING the walk's now-
      `pub(crate)` `collect_guard_pairs`/`VecKey`/`vec_key`, pushed THEN-only in `parse_if`). **MEASURED
      (dev-gate ON): 165 undefended-read warnings → just ~8 GENUINE N-Store rejects** — 3 corpus (repro_p356
      OOB-probes ×2, 85-borrow `xs[i]`), 5 issues (p124 `arr[idx]` ×2, p155, p170 `v[len-1]`, p379). The lib
      compiler (130 warnings) stores NONE of its v[i] into non-null → 0 rejects.
    - **⚠️ LANDING ATTEMPT (`<const_int commit>`) — the reject-count measurement UNDERCOUNTED; flip BACKED
      OFF to dev-gate, tree green.** Two enhancements KEPT (inert under the dev-gate): (a) `index_provably_fit`
      now trusts ANY compile-time-constant index via `const_int` (pub(crate)) — positive OR negative, so the
      `v[-1]` Python last-element idiom is fit and repro_p356 needs no migration (`-1` lowers to a negation, not
      an `Int(-1)` literal); (b) that fixed 2 of the 3 corpus rejects. BUT flipping the gate ON revealed the
      real blast radius is FAR beyond the ~8 compile rejects — **6 wrap tests fail**: `dir`/`last`/`parser_debug`/
      `wasm_dir` (the loft-in-loft COMPILER ripple through lib/parser.loft + lib/code.loft — same class as the
      DN1 flip's F1a phase), `library_suite` (audience_crystal), and `loft_suite`. **The killer is SILENT, not a
      reject:** `v[i]` typing `Item?` mis-classifies a MUTATED/ESCAPING element borrower (`e = v[i]; e.x = …`)
      in the copy-elision analysis (`use_analysis::ElidePlan` / `scopes`), so the copy-keep decision flips and
      the mutation LEAKS to the source (85-borrow `mut_elem` fails both backends — a wrong-answer, not an error).
      **LESSON: counting compile REJECTS undercounts a type-flip's blast radius — silent behaviour changes
      (copy-elision keyed on the element type) + downstream compiler-tests are invisible to a reject scan.**
      **TO LAND (proper F1a-style phase, NOT a quick migrate):** (1) make copy-elision peel `Optional` when
      classifying element borrowers (deps subsystem — loft's #1 weakness, delicate, both backends); (2) migrate
      the lib compiler v[i] sites (the dir/last/parser_debug ripple); (3) audience_crystal; (4) the ~8 direct
      rejects; (5) add len-capture proof if surfaced; (6) flip gate + graduate `25-index-nullable.loft`.
      **Orthogonal gap (separate slice): a nullable passed into a non-null CALL ARG is not an N-Store site**
      (verified `takes(v[j])` doesn't reject) — affects division equally, not index-specific.
  - **F1c** — ✅ DONE (`85af2b18`): the `change_var` null-local message names `τ?`, no `as`.
  - Gate: `find_problems` green as the DEFAULT (no env) on both backends — ✅ met (exemption in place).
- **F2 — Retire `not null`** (Phase-5 CLEANUP; ordering load-bearing). **⚠️ PREMISE CORRECTED
  (2026-07-02): `not null` does NOT "carry nothing" — it carries the FULL-RANGE meaning.** A
  boundary matrix (field/local/param/return × plain/`not null`/`?`, value 255) proved: a plain
  narrow scalar (`u8` field or local) still RESERVES the null sentinel (`255` rejected: *"reserved
  as the null sentinel of a nullable u8"*), and `not null` is the ONLY declaration-level opt-in to
  the full range. The range machinery keys off `IntegerSpec.not_null` (`typedef.rs:685`
  `field_nullable = nullable && !not_null`; the literal-fit check; `database/types.rs`), which the
  DN1 flip left ALONE (DN1 moved *nullability* to `Type::Optional` but not the *range*). So
  retiring `not null` REQUIRES the deferred **range reconciliation** first: under DN1 a non-`Optional`
  narrow scalar is non-null and should be full-range; only `?` reserves the sentinel — i.e. the
  `IntegerSpec.not_null` default-flip (`keys.rs:347`) the flip punted on. This is a delicate
  narrow-int STORAGE/range change next to loft's #1 weakness. **Also surfaced (latent bug, own
  slice): `u8?` is INCONSISTENT** — a `u8?` FIELD silently swallows literal `255`→null (both
  backends), a `u8?` LOCAL keeps `255` as a value; the packed-nullable-narrow-field should
  *compile-error* on the sentinel literal, not silently null.
  - **REORDERED (path B, 2026-07-02): did F3+F4+F5 first** (independent, low-risk, no storage
    touch), THEN the range-reconciliation-and-retire as a dedicated focused F2.
  - **🟡 F2 STARTED (2026-07-02) — gated CHECKPOINT committed, NOT default-on.** The invariant
    is installed behind opt-in **`LOFT_PLN25_F2`** (`keys.rs::pln25_f2_enabled`, default OFF so
    the tree stays green): **reserve the null sentinel iff the type is `Optional`-wrapped** — a
    plain (non-`Optional`) narrow scalar is non-null → FULL range. Two gated chokepoint changes:
    (a) field attr-`nullable` = `matches!(a_type, Optional)` for Integer fields (`definitions.rs`
    field parse) → plain narrow field = full-range storage; (b) `nullable_sentinel_hint`
    (`parser/mod.rs`) returns `None` for a non-`Optional` narrow (fit-check accepts the top value).
    Matrix verified BOTH backends: F2-off unchanged (plain rejects 255), F2-on reconciled (plain
    narrow field/local full-range; `not null` redundant; `u8?` still reserves); single-struct
    read-back correct.
  - **BLAST RADIUS MEASURED (`LOFT_PLN25_F2=1 find_problems`): 8 NEW failures** (2573/2594 vs the
    13-pre-existing baseline) — SMALL + mostly additive (stdlib narrow fields all use `not null`
    already, so it barely moves). But **one is a real storage bug, root-caused (both backends):**
    the WHOLE-STRUCT FORMATTER (`{x}`) reads a narrow `not_null:true` field at the WRONG WIDTH →
    prints `i32::MIN` (integer-null sentinel) instead of the value, while direct field access
    (`.height`) reads it correctly. So F2's `not_null` change desyncs read paths that derive field
    width from nullability — the struct-format desugar is one; likely others. The rest of the 8:
    `runtime_warnings::hint_4h_high_read_count_suggests_not_null` (the @PLN46 "suggest `not null`"
    hint — MOOT under F2, retire like `#null_safe`); `wrap::{dir,structs,loft_suite}` +
    `native_scripts` (roll-ups of 06-structs/08-struct, the formatter bug); `issues::issue_328` +
    `errors_accessor_{nested_path,path_on_failure}` (unchecked — likely the same width desync).
  - **✅ (1) STRUCT-FORMAT READ-WIDTH FIXED (`ac432914`) — a PRE-EXISTING corruption, not F2-only.**
    Root (both backends): a NON-null 2-byte narrow field (`u16 not null`/`i16 not null`, and under
    F2 a plain `u16`) had its DB schema built as `Parts::Short` (the `+1` sentinel encoding) while
    the codegen picks the op via the ONE width→op home `NarrowIntKind::of(2, non-null, false)` =
    `ShortFull` — writing DIRECT (`OpSetShortRaw`) + reading field-access DIRECT (`OpGetShortFull`).
    So field access was correct but EVERY schema-driven read (whole-struct `{x}`, `to_json`,
    `show_loft`, the debugger, **store round-trip**) applied the `+1` shift the write never did →
    off-by-one (`7→6`) / `i32::MIN` at the top of range (a silent reload corruption). Fix = align
    the schema Part with the codegen op: non-null 2-byte → `Parts::ShortRaw` (direct). Interp
    `typedef.rs`; native `generation/mod.rs::emit_field`. Regression `25-narrow-field-format.loft`
    (value + boundary, both backends). F2-OFF suite: 13 pre-existing, ZERO new (the fixed values
    were previously wrong, not asserted). This is a real `main`-bound bug fix, landed ungated.
  - **BLAST RADIUS RE-MEASURED after the format fix (`LOFT_PLN25_F2=1`): 6 NEW** (2575/2594; was 8).
    The format fix cleared 06-structs/08-struct/wrap-dir/wrap-structs. Remaining 6:
    (a) **`389-narrow-runtime-collision`** ("runtime byte sentinel collision reads back null") — a
    BYTE-level sentinel case the reconciliation touches (new signal, separate from the short fix —
    likely the same schema/codegen encoding class for `Parts::Byte` vs a byte full-range, OR a test
    whose whole point is the pre-F2 sentinel behavior → migrate); (b) **`hint_4h`** the `not null`
    read-count hint — MOOT under F2 (a plain field is non-null; you'd never suggest `not null` for a
    `?`), retire at the default-on flip (retiring under DN1 now would break the F2-OFF default tree,
    where plain fields are still attr-nullable); (c) **`issue_328`** + **`errors_accessor_{nested_
    path,path_on_failure}`** — separate (survived the format fix; need checking); (d) `wrap
    loft_suite`/`native_scripts` roll-ups of (a).
  - **TO LAND F2 (remaining):** (1) ✅ struct-format fix (`ac432914`). (2) ✅ 389 byte-sentinel —
    test-premise migration to `u8?`/`u16?` (`208f9c87`; not a code bug — plain narrow reads 255
    fine). (3) ✅ accessor tests → `integer?` (`2e04920c`) + ✅ issue_328 — a cross-statement
    `expr_not_null` leak (`edee7ec8`, general @PLN46 hygiene fix: reset the transient flag per
    statement in `parse_block`; the within-statement `?? d`/`== null` tracking is untouched). So
    (4) ✅ **`not null` read-count hint RETIRED** + (5) ✅ **`LOFT_PLN25_F2` FLIPPED DEFAULT-ON**
    (both in the flip commit) — `pln25_f2_enabled` now rides `pln25_dn1_enabled`;
    `emit_not_null_hints` early-returns under DN1 (superseded like the div/index warnings);
    `hint_4h` migrated to assert the retirement. **THE DEFAULT SUITE IS GREEN under F2-on: 2581
    passed, 13 pre-existing, ZERO new** (both backends via find_problems). So the range
    reconciliation is now the DEFAULT: a plain non-`Optional` narrow scalar is non-null/full-range,
    `not null` is redundant. (6) ✅ **`not null` STRIPPED from .loft source** (`5af21e84`: ~992
    code sites / 313 files; comments+strings left; pe_classify moved to a 102 expected-error since
    a non-null return's missing path is now an error, not the retired "may return null" warning)
    + **parser retirement = ACCEPTED NO-OP** (`not null` still parses, does nothing under F2).
    **A HARD "retired" error is BLOCKED on the registry republish (task #4):** the registry libs
    (graphics/web/gridmesh/crypto/cbor) still carry ~103 code `not null`, and green tests load
    them — a hard error would break those. Once task #4 republishes them without `not null`, the
    parser can reject it.
  - **(7) ✅ DEPRECATION WARNING added (2026-07-10) — the missing rung between "silent no-op"
    and the task-#4 hard error.** `not null` now emits a `Level::Warning` at every parse site
    (`definitions.rs::has_deprecated_not_null`, one shared consume-and-warn helper routing the
    return / integer-field / keyed / vector-element / struct-field sites; pass-2-gated so each
    use warns once). It's contract-COMPATIBLE (still parses as a no-op, so libraries that carry
    `not null` keep loading) but the warning drives the gate attrition: a package fails
    `loft test --deny-warnings` (the per-package CI default unless `.allow_warnings`) until it
    drops `not null` — so libraries shed the remaining uses on their own next PR, converging on
    the state where task #4's hard error becomes unblocking rather than breaking. First-party
    libraries are deliberately NOT swept here (maker's call: let attrition do it). Regression:
    `tests/scripts/25-not-null-deprecated.loft` (`@EXPECT_WARNING`, both backends; asserts the
    no-op still runs). NOTE the warning rides the normal diagnostics channel, so a WARM
    whole-program cache hit suppresses it (as with every loft warning) — `cargo test`/cold CI
    (cache-off) see it reliably, which is where the gate lives. **REMAINING to close F2:** F6
    (deviation-register + CHANGELOG) and the task-#4 registry republish; the model is
    functionally complete.
    **Separate slice (Part 2) — the NULLABLE-NARROW REPRESENTATION is inconsistent across backends,
    a DESIGN decision that blocks several gaps:** a `u8?` FIELD silently swallows literal `255`→null
    while a `u8?` LOCAL keeps 255; and (found 2026-07-02 while evaluating `i as u8?`) INTERP
    represents `u8?` FULL-WIDTH (holds 255-as-value AND null=`i64::MIN` distinctly) while NATIVE
    represents it NARROW-PACKED (1 byte → 0..254 + null; 255 IS the sentinel). A narrow byte only has
    256 codes, so a narrow `u8?` fundamentally **cannot hold both 255 and null**. Consequences:
    - **`fn f(i) -> u8? { i as u8? }` errors** ("cannot implicitly narrow integer to u8") — the
      checked cast (`dn4_checked_cast`) yields a FULL-WIDTH value (null = `i64::MIN`) that can't be
      returned into the declared narrow `u8?` (interp tolerates via full-width; native truncates,
      `i64::MIN as u8 == 0`, losing the null). Attempted fix (type the expr `Optional(u8)`): interp
      OK, native E0308 — REVERTED (not landable, both-backends rule). The working forms are fine:
      `255 as u8` = 255, `255 as u8? ?? 0` = 255, `300 as u8? ?? 0` = 0 (255 is NOT a null situation
      — only `>255` goes null); `i as u8 ?? 0` (no `?`) does NOT work, must be `i as u8? ?? 0`.
    - **The DECISION to settle first:** (A) `u8?` is narrow (0..254 + null; 255 unrepresentable in
      the nullable form — make interp match native) — compact; or (B) `u8?` is full-width (0..255 +
      `i64::MIN` null; nullable-narrow FIELDS grow past 1 byte — make native match interp). Pick one,
      make interp/native consistent, THEN the `-> u8?` return gap AND the `u8?`-field-255 swallow both
      fall out. Substantial storage change (loft's #1-weakness area) — its own focused effort.
- **F3 — Close DN5** (the `as` laundering hole) — 🟡 **IN PROGRESS (2026-07-02).** `null` / `τ?`
  `as <non-null scalar>` silently launders the null → now a compile error (`as τ?` / `??`).
  **Broadened per user directive** ("every integer has a range — detect out-of-range values
  generally"): the cast target is a DOMAIN (value range × nullness); DN4 (range) and DN5 (nullness)
  are ONE containment test, and `null` is just the reserved out-of-domain element. Concrete bug it
  fixed: `is_narrowing_int` bailed on an `Optional` source, so `integer? as u8` slipped past the
  range check ENTIRELY — fixed by peeling `Optional` for the range dimension + adding the null
  dimension at the `as` chokepoint (`operators.rs`), and `as τ?` now types `Optional<τ>` so the
  hole stays closed downstream (`z: τ = (e as τ?)` still requires discharge). Rides
  `pln25_dn1_enabled()`. Matrix green both backends; DN4 checked-cast regression clean both
  backends; heap `null as S` stays legal. Raw grep over-predicted blast radius — the index-flip
  fit-proofs (const index, loop iter-var, guard) already make most `v[i] as T` sources non-null
  (audience_crystal 0 errors, glb uses const indices). **BLAST RADIUS MEASURED (full suite):
  exactly 8 DN5-caused failures, ALL the `null as <scalar>` typed-null idiom → migrated to `null
  as <scalar>?`:** `tests/scripts/01-integers.loft` (6 casts, null-preservation section),
  `tests/issues.rs` q3/q4×2 NaN JSON tests (`null as float?`), `tests/runtime_warnings.rs` fmt43,
  and the two GENERATED feature examples F1/F2 (bridge-edited — see follow-up). All 8 pass both
  backends; fmt + clippy clean. **Follow-ups (NOT DN5 regressions, do NOT block):** (a) feature
  ISSUES loft-lang/features#1/#2 need the same `null as integer?` update + `make features-gen`
  regen (else the F1/F2 `.loft` bridge edits drift on next gen — flagged, external); (b)
  `native_dir` red is PRE-EXISTING (`25-generics` `last_element<T> -> T?` native generic-`Optional`-
  return ABI gap — no `as` cast, DN5-independent, Family-D thread); (c) engine_host_kernel +
  multiplayer/viewer reds are the known registry/DN1 server-lib migration (task #4) — engine_host
  + web libs compile 0 errors under DN5. **DN5 is effectively CLOSED (default-on, enforced,
  fallout migrated).**
- **F4 — Close DN6** (inferred null-join) — ✅ **DONE (2026-07-02).** `change_var_type`
  (`variables/mod.rs`) now joins `Null ⊔ τ = τ?`: an INFERRED local first assigned a bare
  `null`, then a non-null INLINE scalar `τ`, widens to `τ?` instead of erroring (the canonical
  `a = null; if … { a = 5 }` idiom). `var_tp == Null` is inherently the inferred case (a var
  cannot be annotated `null`), so an annotated `a: integer = null` still rejects (case-1
  nullable-mix), as does the reverse `a = 5; a = null`; a widened `τ?` into a non-null slot still
  requires discharge. **SOUNDNESS BOUNDARY (found via the matrix): INLINE scalars only**
  (Integer/Boolean/Float/Single/Character). `Text` — the one heap-backed scalar — is EXCLUDED:
  the Null inline slot cannot be retroactively widened to a text?-heap slot (it underflowed
  `fn_return`'s discard / native E0308); a text null-start must annotate `s: text? = null` and
  `s = null; s = "hi"` falls to the existing "declare it `text?`" error. Verified both backends
  (integer/float/char widen; return/arith/discharge; leak-clean). Graduate `25-null-join.loft` +
  two reject twins (annotated + text-boundary) in `102-expected-errors.loft`. Full suite: 13
  pre-existing failures, ZERO new. DN1-gated.
- **F5 — DN4 cutover** (the range sibling of F3) — ✅ **DONE (2026-07-02).** The premise was
  STALE: DN4 was already default-on (opt-**out** `LOFT_NO_DN4`, no `LOFT_DN4` opt-in), and the
  267 in-tree `as <narrow>` casts are all provably-fit (masked `& 255`/`& 65535`, constants,
  `len() & N`) which DN4 ACCEPTS — so the flip + migration were already complete. The cutover
  finalized it: the `LOFT_NO_DN4` opt-out (which reverted to the silent width-tag `400 as u8 ==
  400` — the truncation the model eliminates) is **RETIRED**, making DN4 UNCONDITIONAL and
  consistent with its nullness sibling DN5 (no opt-out). `tests/dn4_cast.rs` escape-hatch guard
  converted to `dn4_no_optout_flag_is_retired` (asserts `LOFT_NO_DN4=1` no longer disables DN4).
  formal/types.md DN4/DN5/DN6 marked CLOSED. Verified both backends; dn4_cast 3/3.
- **F6 — Final PR: `Closes @PLN25`.** formal/types.md deviations DN4/DN5/DN6 already marked
  CLOSED (F3/F4/F5, 2026-07-02); remaining: confirm DN1+DN2 CLOSED + `CHANGELOG` + F2 (retire
  `not null`, blocked on the range reconciliation — see F2 above).

**Flow-narrowing — ✅ DONE (verified both backends 2026-07-10).** `if x != null { x : τ }` and the
`== null`-else branch both narrow a `τ?` to `τ` with no `?? d` discharge; the unguarded store is
still correctly rejected. This closed the `got = raw`-style ergonomic chokepoint. (Historical note:
this section originally listed it as a deferred separate feature — see § VERIFIED-OPEN RESIDUALS at
the top for the current, re-probed status.)

---

## NEXT STEPS (detailed history — Steps 1–4 are the record of the built model)

The remaining critical path to "done" (full null-model coherence) is the **scalars half**,
then **Phase 3 DN3/DN2**, then **Phase 5 cleanup**. Each step ends green; never carry two
phases' breakage at once.

### Step 1 — Build `Type::Optional(Box<Type>)` — ✅ DONE (`d121f94c`, `tuxedo-pln85`)
The variant + idempotent `Type::optional` former (N-Idem; normalises `Optional(Never|Null)`)
+ `peel_optional`/`base` are in `src/data.rs`. 8 flagged `match Type` sites handled: the
layout-agnostic majority peel to the base (Optional shares the base's sentinel runtime
layout), `name()`/`short_type` render `τ?`, `for_each_child` visits the child. Compile-time
only, additive — nothing constructs `Optional` yet, so the suite is unchanged (0 failures,
both backends); N-Idem pinned by a unit test. `IntegerSpec.not_null` reconciliation deferred
to DN1/DN3 (per `scalar-optional-representation.md`), as designed.

### Step 2 — Scalars Phase 1 MIGRATE: annotate nullable scalar/field sites with `?` — IN PROGRESS
While the scalar default is STILL nullable, mark every site that genuinely holds null.
Under today's default these are **no-ops** — pre-position them before the flip can surprise them.

**Survey done (2026-06-30) — the blast radius is SMALL.** Raw null-signal counts
(`= null` / `?? ` / `==/!=null`): **default/ (stdlib) = 0** · **lib/ ≈ 20** · **tests/ ≈ 867**.
But the raw counts are dominated by sites that are **NOT scalar/field migration targets**:
- **vector-/lookup-coalescing** (`v[i] ?? d`, `obj.field_lookup ?? d`) — already correct
  (the vectors half made `v[i] ⇒ τ?`); the `??` discharges it. (~all of audience_crystal's.)
- **inferred locals** (`nr = def_names[name].nr`, `l = data[index]`, `x = null`) — nullability
  is *inferred* from the fallible source, not a declared scalar type; inference-governed.
- **`==/!=null` on references/enums** — heap-nullable already (separate from the scalar flip).

The **genuine MIGRATE targets** are *explicitly-typed scalar fields/vars that hold null*. In
the **controlled surface (stdlib + lib) there is exactly ONE**: `Code.cur_def: i32` in
`lib/code.loft` (`self.cur_def = null` at `end_define`) → annotated **`cur_def: i32?`**
(`i32?`/`text?` verified to parse as a no-op both backends). The stdlib needs none.

So pre-annotation is light; the codebase carries null via fallible-lookup (handled) far more
than via nullable scalar fields. The remaining test-side sites are mostly intentional null
tests / inferred locals — left to **Step 3's flip**, which surfaces any genuine miss as a
loud error fixed one-character (`?`), exactly the design's catch-all. **DN1 blast-radius
estimate: very low for the shared surface.**
- **Validation:** suite stays green after annotation (no behaviour change) — ✅ `find_problems`
  0 failures both backends after the `cur_def` annotation.

### Step 3 — Scalars Phase 2 CONTRACT: flip the scalar/field default to non-null (DN1)
The default flip. `IntegerSpec.not_null` default `false → true` (and the bool/char/text
analog); the plain-type parse stops meaning nullable; `not null` becomes an **accepted no-op**.
- **Where:** `src/data.rs` (`not_null` defaults at the `IntegerSpec` constructors, lines
  ~93–167) + `src/parser/definitions.rs` (the scalar `not null` parse — consume + set nothing).

> **⚠️ SCOPING (2026-06-30, before flipping) — DN1 is bigger than "flip + one-char sweep",
> and is intertwined with DN3.** Read this before starting:
> 1. **The flip alone produces WARNINGS, not the clean errors implied.** The type-checker's
>    only consumer of a type's nullability is `expr_not_null` → the **redundant-null-check
>    *warning*** (`operators.rs`, @PLN46 W2). Flipping the default fires that warning on
>    *every* `int == null`/`int != null` (now "always-redundant") — noise, not the bounded
>    error-sweep. **The hard rejection of `x: integer = null` is DN3's `(N-Store)` check,
>    which does not exist yet.** So DN1's flip is only *meaningful + cleanly-bounded* once
>    `(N-Store)` lands — **DN1 and DN3 are one step, not two.**
> 2. **Non-Integer scalars (`text`/`bool`/`char`/`float`) have NO `not_null` flag** — their
>    "non-null analog" must be carried by `Type::Optional` (Step 1) + the `(N-Store)` check,
>    i.e. it is the *same* type-checker work as DN3, not a separate flag flip.
> 3. **Prerequisite — wire `τ?` → `Type::optional(τ)`** in the scalar parse (today Phase-0
>    `?` is a no-op). This is additive, BUT constructing `Optional` exercises **non-exhaustive
>    `match Type` sites with a `_` arm** (Step 1 fixed only the 8 *exhaustive* ones) — an
>    `Integer`-special match with a `_` fallthrough would mis-handle `Optional(Integer)`
>    *silently*. That audit (find the `_`-arm Type matches that must peel) is the real DN1
>    worklist, and it is a correctness audit, not a one-char sweep — **measured surface:
>    ~280 `Type::Integer` match-arm sites across 39 files** (`grep -rn 'Type::Integer' src/`).
> 4. **47 sites read `not_null`**, with double-duty (nullability + bounds).
>
> **Recommended approach:** treat **DN1+DN3 as one gated effort** (an `LOFT_NO_DN1` opt-out
> like DN4, so the suite stays green while sweeping): (a) wire `?`→`Optional`; (b) audit +
> fix the non-exhaustive `_` Type matches to peel; (c) add the `(N-Store)` reject-null check
> gated on; (d) flip the default; (e) `find_problems` sweep the `.loft` misses to `?`, both
> backends; (f) flip the gate default-on. Multi-session; the survey says the *shared-surface*
> `.loft` sweep is small, but the *compiler* work (b)+(c) is the substance.

**Slice (a)+(b) DONE for the current corpus (`8e279c7c`, gated `LOFT_PLN25_OPT` opt-in).**
(a) the postfix `?` constructs `Type::optional` gate-ON (OFF = Phase-0 no-op, byte-identical).
(b) the consuming-site peel audit — **~19 sites peeled** across type-check, layout, interp +
native codegen: `convert` (incl. the null→typed-null transform for a nullable target),
`get_val`/`set_field_check`/`gen_set_first_at_tos`/`generate_var`, `size`×2 +
`element_size`/`element_align` + `typedef` DB-layout (the SIGSEGV — an Optional field got a
wrong record layout, overflowing an adjacent store), `type_def_nr`, `??`
(`handle_null_coalesce`), `null(tp)`, and native `rust_type`/`write_typed_null`/the
`text?`-return ABI. Each behaviour-preserving, a no-op gate-OFF.

**Result: the FULL suite is green gate-ON.** Only **3 `.loft` files** use a `?` annotation
today (25-scalar-optional-syntax, 81-iterator-protocol, + the lib MIGRATE site), so the
*exercised* audit surface is small — all pass on BOTH backends gate-ON; `find_problems`
gate-ON shows no Optional-related failure. The ~280-site count is the THEORETICAL surface
(live only once DN1 makes plain types Optional); for the current `?`-usage the sweep is
complete. **The gate stays opt-in** — `?`→Optional default-on is inert until DN1 gives it
teeth, so flipping it early adds risk (unexercised sites) for no value. Validation: gate-OFF
byte-identical; fmt + both clippy clean.

**NEXT = slice (c)–(f) = the DN1+DN3 effort:** add the `(N-Store)` reject-null check (gated),
flip the scalar default non-null, sweep the `.loft` misses, then flip the gate default-on.
This is where the ~280 sites become *live* (plain types → Optional) — the big multi-session
phase the scoping above describes.

**Slice (c) `(N-Store)` — SCOPED (gate `LOFT_PLN25_DN3` added, implies OPT; check reverted).**
First attempt put the reject-un-discharged-`τ?` check in `convert()` — **wrong granularity.**
`convert` also services COMPARISONS, so it wrongly flagged `s.a == null` (the null-CHECK that
is how you *test* nullability) as an illegal nullable→non-null use. Reverted. **Finding:
`(N-Store)` must live at the STORE / decl / index / return sites (the design's per-site
`N-Store`/`N-Decl`/`N-Coal`/`N-Match` checks), and `== null` / `!= null` null-compares must stay
legal on a nullable.** The probe confirmed the *enforcement direction* is right: an
un-discharged `bad: integer = e.hp` errors; `e.hp ?? 0` passes; and it surfaced a genuine
sweep target — `lib/code.loft`'s `definitions[cur_def]` uses the annotated `cur_def: i32?` as a
non-null index (29 sites) → must discharge post-DN1.

**Store-site implementation DONE (`def34450`):** a `n_store_violation` helper called at the
STORE sites — the typed scalar assignment (`expressions.rs`) + field construction
(`objects.rs`). Right granularity confirmed: the 25-probe (`s.a == null`) is GREEN DN3-ON on
BOTH backends (the convert false-positive is gone), `bad: integer = e.hp` errors, `?? 0`
passes. Gated `LOFT_PLN25_DN3`.

**INDEX site DONE (`65ef931e`):** a nullable cannot be a vector index (`fields.rs:783`,
`n_store_violation(&index_t, &I32, "a vector index")`).

**RETURN site DONE (this commit):** all three return store-paths now run `(N-Store)` — the
explicit `return e` (`control.rs::parse_return`), the implicit function-tail `{ … e }`
(`control.rs::block_result`, gated to `context == "return from block"` so an `if`/`match` arm
whose `result` is legitimately nullable is untouched), and `lhs ?? return e`
(`operators.rs::build_null_coalesce_return`). Matrix probed on BOTH backends: an un-discharged
`integer?` returned into a non-null return errors gate-ON (single diagnostic — `convert` still
peels `Optional`, no double-diagnose); `?? d` / a nullable return type pass; gate-OFF
byte-identical (the value still flows through as the null sentinel). Artifacts:
`bytecode-comparisons/25-nstore-return-{LEGAL,VIOLATION}.loft`.

All store sites for the current corpus are now covered.

**DN1 `_`-arm AUDIT — COMPLETE (`dn1-audit/findings.md`).** Before the default flip, the audit
that the scoping below calls "the real DN1 worklist" is DONE — 5 parallel subsystem audits +
an empirical instrument (`dn1-audit/optional-flow-instrument.loft`, green both backends). Result:
**69 NEEDS-FIX** sites where an `Optional(τ)` value silently takes a non-Optional `_` arm
(panic / wrong size·align·stride / leak / wrong ABI). They collapse into 7 families with ONE
uniform fix — **peel `.base()` before the type dispatch** (byte-identical gate-OFF, additive):
- **A — layout/size/align** (SIGSEGV/panic, HIGHEST): the sibling-pair misses where slice (b)
  fixed one twin and missed the other — `size`✓/`align`✗ (variables 1753), `type_def_nr`✓/
  `type_elm`✗ (data 4752, the root), `element_align`✓/`tuple_def`-align✗ (data 3971),
  `generation::rust_type`✓/`Data::rust_type`✗ (data 4832, panic). Fix first.
- **B — the `(N-Decl)` gate**: `change_var` (variables 1257) rejects `τ ↔ Optional(τ)` as a type
  change → nullable LOCALS unusable today (`x: integer? = 5` fails — even non-null). This is the
  local-half gate; it makes most interp-codegen sites latent-but-UNREACHABLE until fixed.
- **C** deps/leak holes for `Optional(Text/ref)` · **D** the `text?` return-buffer ABI sub-thread
  · **E** the `matches!`-predicate second sweep (`is_scalar`/`slot_kind`/~40 `Type::Text` ABI
  gates) · **F** feature gaps (`match`/`for`/`+`/`float? == null`) · **G** the empirical bugs
  (E1 native `null` tuple-element → `(())`; E2 `"{x}"` format reject). Full rows + the staged
  fix-sequence in `dn1-audit/findings.md § SYNTHESIS`.

**NEXT = the staged fix-sequence (findings.md): Family A ungated first (`type_elm` neutralises
downstream), then B (`change_var`) → reachable interp peels → C leak holes → E/F → D text? ABI →
G → THEN (d) the default flip (`IntegerSpec.not_null` `false → true`), (e) `.loft` sweep of
misses to `?` (incl. `lib/code.loft`'s `definitions[cur_def]`), (f) flip the gate default-on.**
The bare-`null` var-DECL (`x: integer? = null`) failure is one face of Family B's `change_var`
gate (`x: integer = null` errors gate-OFF too) — settle it there.

### Step 4 — Phase 3 TIGHTEN, the rest: DN2 then DN3 (the measured blast radius, LAST)
DN4 already shipped (above). Remaining, least-to-most breaking:
- **DN2** — remove the implicit `τ? ⤳ τ` unwrap in `convert()` (`parser/mod.rs:1585`). After
  this, `??` / `match` are the only ways down from `τ?`. Breakage: code relying on silent
  unwrap.
- **DN3** — type fit-failing ops as `τ?` (`/`, `%`, `[]`, `parse`, overflow) and make
  `(N-Store)` reject an un-discharged `τ?` into non-null storage. **Biggest blast radius** —
  every `b = a / x` without a `??`. The runtime already nulls (`fill.rs`); DN3 is type-level
  (carry `τ?`) + the discharge check.
- **MANDATORY — measure before DN3 lands:** count sites assigning a fit-failing result into
  non-null storage without `??` (the gating number). Migrate with `?? d` / `as τ?` / a mask.
- **Validation:** green after the discharge migration, both backends. This is the
  "willing-to-rewrite-tests" step — it lands last, after everything else is green, with the
  blast radius counted first.

### Step 5 — Phase 5 CLEANUP: retire `not null`  (ordering is load-bearing)
By now `not null` is a no-op everywhere. Remove it — **in this order**, or the **1015
occurrences across 300 `.loft` files** all become parse errors at once:
1. **Strip `not null` from all `.loft` source** (stdlib, `tests/**`, `lib/` + consumers).
   Mechanical, per-area, re-running the targeted suite after each — behaviour-preserving since
   it is already a no-op.
2. **`grep -rn "not null" --include='*.loft'` is clean** (0 occurrences) — the gate before
   touching the parser.
3. **Remove `not null` from the parser** (`definitions.rs`: scalar ~line 1470, the vector arm's
   `has_keyword("not")`, the keyed arms). After this `not null` is a **syntax error** — the
   retirement is enforced, not conventional.
4. **Docs/skills** — drop `not null` from `LOFT.md`, the `loft-write` skill, `formal/types.md`
   notation, examples. `?` becomes the only nullability marker.
- **Validation:** steps 1–2 keep the suite green throughout; step 3 only "breaks" code step 2
  proved nonexistent. End state: one nullability notation (`?`), no non-null marker.

### Step 6 — Land it: single PR to `main`
Per the `@PLN25` decision, the rewrite reaches `main` via **one final PR** when E2 is fully
coherent (scalars done + Phase 3 + Phase 5). The finishing PR carries `Closes @PLN25`. Before
opening: `git fetch`, confirm the head is a descendant of `origin/main`, full
`find_problems.sh` green on both backends.

---

## Parallel sub-thread (NOT on the critical path) — copy-vs-borrow elision

The performance face of the dense model ([copy-elision-design.md](copy-elision-design.md),
Cluster C / `OWNERSHIP_MODEL.md`). Tier-0 + Tier-1.5 are **default-on, on `main`**.
- **Tier 1 (local-struct source) is IMPLEMENTED but parked off** (`LOFT_ELIDE_T1`,
  `use_analysis.rs:423`). The design's own conclusion (2026-06-27 crawler dogfood):
  **do NOT cut it default-on** — it adds only 2 cold-path borrows, no measured local-sourced
  hot copy exists to capture; keep it as a turn-on-and-compare flag until a consumer surfaces
  one. So this is a *deliberate stop*, not unfinished work.
- **Tiers 2–3** (mutable source; unify assignment + return delivery onto one
  `materialization_mode` predicate — where this meets #465 / Cluster C) are **design-only,
  gated behind a measured need**. Do not build speculatively.

---

## Verify / probe commands

```sh
# @PLN25 SCALAR-HALF GATES (this session, opt-in; cached OnceLock in src/keys.rs):
#   LOFT_PLN25_OPT=1  -> the postfix `?` constructs the real Type::Optional (else a no-op)
#   LOFT_PLN25_DN3=1  -> the (N-Store) teeth (reject un-discharged τ? into non-null); implies OPT
# gate-OFF (both unset) = byte-identical default. Verify the scalar-Optional path:
LOFT_PLN25_DN3=1 loft --interpret tests/scripts/25-scalar-optional-syntax.loft   # green both backends
LOFT_PLN25_DN3=1 loft --native    tests/scripts/25-scalar-optional-syntax.loft
LOFT_PLN25_DN3=1 loft introspect  lib/code.loft | grep -c "vector index"          # 14 (cur_def index sweep targets)
# the (N-Store) catch: a nullable used as a non-null store/index errors; `?? d` discharges:
printf 'fn main(){ v:vector<integer>=[1]; i:integer?=null; x=v[i]; }' > /tmp/n.loft
LOFT_PLN25_DN3=1 loft --interpret /tmp/n.loft   # error: discharge with `?? <default>`

# dense default + nullable opt-in (the core correctness, already green):
loft --interpret /tmp/a.loft               # vector<S?>: v[1]==null -> true
loft introspect /tmp/a.loft | grep main_vector   # vector<S> -> main_vector<S> (dense)
# DN4 enforcement (default-on):
echo 'fn main(){ x = 400 as u8; print("{x}"); }' > /tmp/dn4.loft
loft --interpret /tmp/dn4.loft             # -> compile error: use `as u8?`  (DN4 unconditional; opt-out retired F5)
# full baseline (expect green: 2564/0):
./scripts/find_problems.sh --bg ; ./scripts/find_problems.sh --wait
```

## Facts a fresh session needs

- **`Type::Optional` is not in `src/data.rs` yet** — Step 1 builds it. The commit
  `885eab1d` only *decided* the representation.
- **The scalar default is still nullable** (`IntegerSpec.not_null` defaults to `false`,
  `data.rs:97–167`). Step 3 flips it. DN4 shipped before this because it is integer-range-only.
- **Family A** (`?? <vector-literal>` codegen panic) is fixed on `main` (`6ea779be`). Do NOT
  re-investigate.
- **#465** (dense borrowed-view over-free) is **fixed** and on `main` — it is no longer a
  blocker; the Tier-3 elision unification is its forward-looking sibling, not a re-fix.
- **Branch discipline:** keep `main` off this branch until the scalars half is green at a phase
  boundary; reach `main` via the single final PR (`Closes @PLN25`).
- **Do not build on `2026-07-mac`** — the abandoned enum-synthesis approach, long stale. The
  live `LOFT_E2_SYNTH` references are vestigial from it; the dense-default design (this branch)
  superseded it.
