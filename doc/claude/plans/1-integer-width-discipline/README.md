<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 48 — Integer width discipline: `integer` is i64, `i32` is explicit, no implicit narrowing

## Status

> **CLOSURE RECORD — DONE, closed 2026-06-14** (`@PLN1`, local slug `48`,
> `status:finished`).  `integer` is i64 end-to-end, `i32` is the only 4-byte int,
> and `integer`→`i32` narrowing is never implicit (requires `as i32`).  Verified
> on both backends; `scripts/verify_external_libs.sh` shows `loft-libs-core/random`
> **ok on interpret + native** — the original **@P370** acceptance (a native
> `(i64,i64)->i64` returning the `i64::MIN` null sentinel no longer truncates to
> `0` on `--interpret`).  This file is retained as the closure record + build
> narrative.

The maintainer's rule — **a plain loft `integer` is 64-bit; the only 4-byte int is
an explicit `i32`; and `integer`→`i32` (data-losing narrowing) must NEVER be
implicit — it requires `as i32`** — now holds across the whole toolchain.

- **P1 (FFI marshal) — DONE.**  `src/extensions.rs` maps a plain `integer`
  (`forced_size.is_none()`) → `ArgT::I64`; narrow (`forced_size`) / `Character`
  → `ArgT::I32`.
- **P1b (I64 dispatch arms) — DONE.**  The full `(I64, …)` arm set is in the
  dispatcher (`extensions.rs:1389–2191`), including the trickier store/`Ref` arms
  (`save_png` / `gl_upload_*` / `set_mat4` / `rasterize_text` / `audio_play_raw`).
- **P2 (compiler enforcement) — DONE.**  Implicit `integer`→`i32`/narrow is a
  compile error; widening `i32`→`integer` stays implicit; `as i32` escape hatch +
  constant-fits literal exemption work.
- **P3 (library consequences) — DONE.**  web/server/graphics native decls + impls
  converted (imaging was already clean); `tests/native_loader.rs` vec_* suite
  green; external-libs verify passes.  (The unrelated `game_protocol` / `crypto`
  fails in that run are @PLN22 import-syntax migration debt — flat `use lib::a, b`
  → `use lib::(a, b)` — tracked separately, not a width-discipline item.)
- **P4 (@P368b divide-by-zero warning on a named-constant divisor) — routed out**
  as a standalone warning-quality follow-up (`@P368b`); never part of the width
  discipline.  See [42-warning-quality](../46-warning-quality/README.md).

## Goal

`integer` behaves as a 64-bit value everywhere (stack, fields, arithmetic, FFI);
narrowing it to `i32`/`u8`/… always requires an explicit `as`, so no conversion
silently loses data and no backend disagrees on width.

## The rule (design)

This is the existing conversion-mode framework ([INCONSISTENCIES.md #17](../../INCONSISTENCIES.md),
[LOFT_DECLARATIONS § Type-conversion rules](../../LOFT_DECLARATIONS.md)) applied to integer widths.
Rule of thumb already in LOFT.md: *infallible/widening = implicit; fallible/
narrowing = explicit `as`.*  The integer-width rows that follow from it:

| From → To | Mode | Why |
|---|---|---|
| `i32`/narrow → `integer` | **Implicit** | widening, lossless (already works) |
| `integer` → `i32` | **Explicit `as i32`** | NARROWING — loses the high 32 bits (currently wrongly implicit — the bug) |
| `integer` → `u8`/`u16`/`i8`/`i16` | **Explicit `as`** | narrowing |
| FFI: native fn declares `integer` | marshals **I64** | impl must be i64 (P1) |
| FFI: native fn declares `i32` | marshals **I32** | impl is i32; marshal widens the C-i32 return into the 8-byte slot |

LOFT.md's conversion table has no `integer → i32` row today; P2 adds it as
Explicit `as` and the typer enforces it.

## Sub-arcs

| Item | Status | Files |
|---|---|---|
| **P1a** — FFI marshal inference: plain `integer` → I64 | **Done** | `src/extensions.rs` arg+return ArgT inference (`forced_size.is_none()` gate) |
| **P1b** — I64 dispatch arms | **PENDING** (blocks graphics/rand/vec) | `src/extensions.rs` dispatch match — add the `(I64,…)` arms (signature list in § Remaining) |
| **P2** — compiler: implicit `integer`→`i32` is an error | **Done** | `convert` + `parse_assign_op` guard + `is_narrowing_int`/`int_value_fits`/`int_type_name` (`src/parser/mod.rs`, `expressions.rs`); `as` path exempts explicit narrowing (`operators.rs`); LOFT.md row added; regression tests in `102-expected-errors.loft` + `repro_p370_widen.loft` |
| **P3-web/server** — i32 end-to-end | **Done** | web/server `#native` decls + handle structs (`HttpSession.id`/`WsHandler.id`/`Server.handle`/`WebSocket.ws_id`/`WsEvent.cid`) + handle params → i32; value params (`status`/`port`) keep `integer` + `as i32` at the FFI call.  Validated both backends. |
| **P3-graphics** — impls → i64 | **impls Done; arms PENDING** | ~40 `loft_*` native fns converted i32→i64 (decls stay `integer`); blocked on P1b arms |
| **P3-fallout** — vec_* / multiplayer | **PENDING** | `tests/native_loader.rs` `vec_*` + multiplayer v2/v5 fail (same root: integer-natives need P1b arms) |
| **P4** — @P368 const-divisor (@P368b) | Not started | `src/parser/operators.rs::is_easy_proof` — const-fold a named-constant divisor via `src/const_eval.rs` |

## Progress + remaining work (2026-05-27)

### Done & validated (landed)
- **P2 enforcement** — fully working: `x: i32 = some_integer` and `f(some_integer)`
  (i32 param) error with *"cannot implicitly narrow integer to i32 — cast
  explicitly with `as i32`"*; `n as i32` (escape hatch), `x: i32 = 5` (literal
  exemption), and `y: integer = i32val` (widening) all compile.  stdlib still
  loads.  Sites: `convert` (`src/parser/mod.rs`, emit-then-fall-through), the
  assignment guard in `parse_assign_op` (`src/parser/expressions.rs`), and the
  `as`-operator exemption (`src/parser/operators.rs` — skip `convert`/`cast` when
  `is_narrowing_int`).  Helpers `is_narrowing_int` / `int_value_fits` /
  `int_type_name` in `mod.rs`.  Regression: `tests/scripts/102-expected-errors.loft`
  (2 error fns) + `tests/scripts/repro_p370_widen.loft` (allowed cases, both backends).
- **P1a marshal inference** + **P3 web/server** + **P3 graphics impls** + LOFT.md row.

### P1b: I64 dispatch arms — DONE (landed; build narrative below)

Add these `(params) -> ret` arms to the `src/extensions.rs` dispatcher (mirror the
existing `I32` twins; use `i64_arg!` + `stores.put::<i64>(…)` with NO `widen_int`
for an I64 return; LoftStore/LoftRef handling identical to the I32 arms).  Full
set the converted graphics impls + external `rand` now require:

```
(I64) -> Bool                     (I64,I64) -> void           (I64,I64) -> I64
(I64,I64,I64) -> void             (I64,I64,I64,I64) -> void   (I64,I64,I64) -> I64
(Text) -> I64                     (Text,Text) -> I64          (Text,I64,I64) -> Bool
(I64,F64) -> I64                  (I64,F64) -> void           (I64,F64) -> F64
(I64,Text,F64) -> void            (I64,Text,I64) -> void      (I64,Text,F64) -> F64
(I64,Text,F64,F64,F64) -> void    (I64,Text,Vec) -> void
(Vec,I64) -> I64                  (Vec,I64,I64) -> I64        (Vec,I64,F64) -> I64
(Text,I64,I64,Vec) -> Bool        (I64,Text,F64,Vec) -> I64
```
(`Vec`/store arms — `save_png`/`gl_upload_*`/`set_mat4`/`rasterize_text`/
`audio_play_raw` — are the trickier ones: copy the existing `…Vec…` arms'
`make_loft_store`/`first_ref_store` handling.)  Method: add them, rebuild loft,
run a graphics gold test, and let each `auto-marshal: unsupported signature (…)`
name any still-missing arm; iterate to zero.

**This is the LAST time arms are hand-written.**  [FFI.1 / FFI.3
(lib_plans/61-game-infra)](../../lib_plans/61-game-infra/README.md)
— § "Design decision (2026-05-27)" — generate this dispatch per-library from each
native fn's real signature (direct typed calls, no central match), deleting the
hand-arms.  P1b is the one-time unblock; FFI.1/FFI.3 remove the recurrence.
(Native already links C-style — direct calls, no marshal — so only the interpret
path needs this; perf is unaffected.  A uniform-cell shim / libffi were
considered and rejected — see that section.)

### P3 fallout + verification — DONE
- `tests/native_loader.rs` `vec_*` (`vec_i32_sum`, `vec_from_returned_struct`,
  `scalar_before_vec`, `vec_between_scalars`, `vec_in_loop_if`, …) and multiplayer
  v2/v5: re-run after P1b; diagnose any residual (likely the same missing-arm /
  a narrow-vector element path).
- @P368 golden baselines were already regenerated for the wording change; re-run
  `find_problems.sh` to confirm the whole suite is green on both backends.
- Re-verify external libs: `scripts/lib_audit.sh --local` (rand must now give
  `null` for `rand(lo>hi)` on **both** backends — the original @P370 acceptance).

### Lesson (for the pacing)
The P1 marshal change is **repo-wide**, not surgical: it touches *every* native
lib declaring `integer` (web, server, graphics; imaging was clean).  The
inspection under-counted (it called graphics clean).  Pace it lib-by-lib behind
P1b, each lib re-verified on both backends before the next.

### Working-tree state (landed — historical, ~10 files)
`src/extensions.rs` (P1a marshal), `src/parser/{mod,expressions,operators}.rs`
(P2), `lib/web/src/web.loft` + `lib/server/src/server.loft` (P3 decls/casts),
`lib/graphics/native/src/{lib,audio}.rs` (P3 impls→i64), `doc/claude/LOFT.md`
(table row), `tests/scripts/{102-expected-errors,repro_p370_widen}.loft`.
Nothing committed yet for this arc.  The loft binary BUILDS; graphics/vec/
multiplayer fail at RUNTIME (missing P1b arms).  A clean checkpoint = commit P2 +
web/server + docs (self-consistent, green) separately from the graphics/marshal-arms
push.
| _(related)_ @P370 | Fixed by P1 | the interpret/native integer-ABI divergence that motivated this plan |

## Why i32-on-exit was the right call (P3 rationale)

Returns are the **safe** direction: an `i32` return widens to `integer` implicitly
and losslessly, so a native fn declaring `i32` flows into any `integer` context
with no cast (`status = http_do(...)` → `HttpResponse.status: integer`).  Entries
are the lossy direction: an `i32` param fed an `integer` value narrows — so the
fix is to carry i32 end-to-end (struct fields + wrapper params hold the same
4-byte handle/status), making the internal flow `i32`→`i32` (no coercion) and
only widening (`i32`→`integer`) when *user* code drops a handle into an integer.
Only values genuinely bounded to 4 bytes use `i32` (HTTP status, slot handles,
opcodes, byte values, counts); anything that can exceed i32 stays `integer`.

## Phase ordering

1. **P1** (done) — unblocks the external `random` package on both backends and
   stops the silent i64→i32 truncation.  Commit with P3 (they're coupled: P1
   broke the web/server libs until their decls moved to i32).
2. **P3** — finish i32 end-to-end so the libs are self-consistent BEFORE P2 turns
   implicit narrowing into an error (otherwise P2 flags the lib wrappers).
3. **P2** — enforce in the compiler; fix the resulting fallout across stdlib /
   lib / tests (add `as i32` where narrowing is intended).  Largest blast radius
   — scope it first (grep the typer's integer→i32 coercion sites + count breaks).
4. **P4** — independent; land any time.

## Open questions

1. **P3 shape — i32 end-to-end vs. all-i64.**  Option (b) chosen (explicit i32
   for 4-byte values); the alternative (a) keeps everything `integer` and moves
   the ~30 web/server Rust impls to i64 — zero i32, zero narrowing, but more
   `.rs` churn + new I64 dispatch arms.  (b) is in flight; revisit if the
   end-to-end i32 surface (public struct fields + wrapper params + scattered
   `as i32`) proves larger than (a).
2. **P2 blast radius.**  How much existing loft (stdlib `default/*.loft`, `lib/*`,
   `tests/`) relies on implicit `integer`→`i32` today?  Must be measured before
   enforcing, so the fallout (`as i32` insertions) is bounded.
3. **P2 narrow aliases.**  Does `integer`→`u8`/`u16`/`i8`/`i16` get the same
   explicit-`as` treatment, or only `i32`?  (The rule-of-thumb says all
   narrowings; confirm no stdlib pattern depends on implicit byte-narrowing.)

## See also

- [INCONSISTENCIES.md #17](../../INCONSISTENCIES.md) — the implicit/format/explicit
  conversion-mode framework this plan extends.
- [LOFT_DECLARATIONS § Type-conversion rules](../../LOFT_DECLARATIONS.md) — the conversion table P2 adds the
  `integer → i32` row to; also § `integer` (64-bit end-to-end) and the null-sentinel table.
- [PROBLEMS.md](../../PROBLEMS.md) — @P370 (fixed by P1), @P368 (partial; P4).
- `src/extensions.rs` (FFI auto-marshal), `src/parser/operators.rs` (@P368),
  `lib/web/src/web.loft` + `lib/server/src/server.loft` (P3).
- External: `loft-lang/loft-libs-core/random` (the consumer that surfaced @P370;
  its i64 `n_rand` impl is the *correct* model).
