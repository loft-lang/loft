<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 00 — Matrix freeze + harness wiring

**Status: open**

## Goal

Lock the coroutine-validation matrix and wire
`tests/coroutine_matrix.rs` to the existing `cross_mode!` harness
from `tests/common/cross_mode.rs`.  No new harness code is needed;
the @PLAN14 phase-00 infrastructure is reused as-is.

## The frozen matrix

Cell legend: `PASS:test_name` / `FIX:phase` / `BLOCKED:@PNNN` /
`CLOSED:reason` / `PASS-i, FIX-n:phase` (interp passes, native fixes
in named phase).

| | X1 — for-loop | X2 — manual `next()` | X3 — higher-order (map/filter/reduce) | X4 — comprehension | X5 — yield from |
|---|---|---|---|---|---|
| **Y1** scalar | PASS:y1_x1_* | PASS:y1_x2_int_manual_next | PASS:y1_x3_int_closure_map + closure_reduce | PASS:y1_x4_int_comprehension | CLOSED:CO1.4-deferred |
| **Y2** text | PASS:y2_x1_* | PASS:y2_x2_text_manual_next | PASS:y2_x3_text_closure_chain | PASS:y2_x4_text_comprehension | CLOSED:CO1.4-deferred |
| **Y3** Reference | PASS:y3_x1_ref_for_loop | PASS:y3_x2_ref_manual_next | FIX:03 | FIX:03 | CLOSED:CO1.4-deferred |
| **Y4** tuple | PASS:y4_x1_tuple_for_loop | PASS:y4_x2_tuple_manual_next | FIX:04 | FIX:04 | CLOSED:CO1.4-deferred |
| **Y5** closure | PASS:y5_x1_closure_for_loop_noncapturing | PASS:y5_x2_closure_manual_next_capturing | FIX:05 | FIX:05 | CLOSED:CO1.4-deferred |
| **Y6** vector | CLOSED:non-goal | CLOSED:non-goal | CLOSED:non-goal | CLOSED:non-goal | CLOSED:CO1.4-deferred |

### Phase 01 (Y1 scalar) results — 2026-05-23

**5 cells PASS, 2 BLOCKED on new P-issues filed this session.**

| Test | Status |
|---|---|
| `y1_x1_int_for_loop_smoke` | PASS |
| `y1_x1_int_for_yield_in_generator` | PASS |
| `y1_x1_int_consumes_another_generator` | PASS | (renamed 2026-09-25 — it never tested a helper yield, which `@FR-G-Yield` refuses) |
| `y1_x2_int_manual_next` | PASS (after fixing test's exhausted() semantics — needs an extra `next()` past the last yield) |
| `y1_x3_int_closure_reduce` | PASS (integer accumulator — no store mutation) |
| `y1_x3_int_closure_map` (vector accumulate) | **BLOCKED:@P324** — interp-only; `out += [f(v)]` inside `for v in gen()` fires the CO1.9/S28 stale-DbRef guard even though the generator holds no DbRefs.  Native works. |
| `y1_x4_int_comprehension` | **BLOCKED:@P325** — both backends broken: interp = @P324 guard, native = 17 GB store-offset overflow in the eager-collect factory. |

Cells stay in `tests/coroutine_matrix.rs` commented out with a P-issue
link; they un-comment automatically once the P-issue closes.  Re-run
with `cargo test --release --test coroutine_matrix -- --ignored` to
verify they pass post-fix.

### Phase 02 (Y2 text) results — 2026-05-23

**4 cells PASS, 1 BLOCKED on the same @P325 comprehension path.**

| Test | Status |
|---|---|
| `y2_x1_text_for_loop` | PASS — for-loop over `iterator<text>` yielding string literals |
| `y2_x1_text_format_with_param` | PASS — `while`-driven generator yielding format-string with a captured parameter (@P218 regression carrier) |
| `y2_x2_text_manual_next` | PASS — explicit `next()` calls; `exhausted()` flips after the extra-step pump |
| `y2_x3_text_closure_chain` | PASS — closure transforms each yielded text; consumer concats into a text accumulator (no vector store mutation between yields) |
| `y2_x4_text_comprehension` | **BLOCKED:@P325** — same eager-collect factory bug as `y1_x4` |

Y2 is healthier than the plan's "active risk" framing suggested — the
yielded-text lifetime works cleanly across the for-loop, manual-next,
and closure-chain drive contexts.  The remaining hole is the same
shape as Y1 (comprehension over a generator).

### Phase 03 (Y3 Reference) results — 2026-05-23

**0 cells PASS, ENTIRE ROW BLOCKED on a single root-cause native gap.**

| Test | Status |
|---|---|
| `y3_x1_ref_for_loop` | **BLOCKED:@P326** — native lacks the DbRef state-machine channel.  Interp passes (21). |
| `y3_x2_ref_manual_next` | **BLOCKED:@P326** — same gap. |
| `y3_x3_ref_closure_chain` | (not written; the @P326 gap blocks it identically) |
| `y3_x4_ref_comprehension` | (not written; @P325 stacks on top of @P326) |

@P326 is the user-facing form of the plan README "Y3 (Reference) cells
still need re-probe with the new yield-type infrastructure" line.
Native state machine needs:
  - `coroutine_next_dbref` runtime helper (mirrors `coroutine_next_i64`
    / `coroutine_next_text`)
  - dispatch arm in the state machine for `Type::Reference(_, _)` yields
  - codegen path that emits `coroutine_next_dbref(...)` instead of the
    current `coroutine_next_i64(...) as i32` mis-cast
This is the SINGLE highest-leverage native coroutine fix to land; it
unblocks the entire Y3 row, and any planned Y4 tuple path that yields
struct fields.

### Phase 04 (Y4 tuples) results — 2026-05-23

**Highest-severity find of this session: silent wrong answer on interp.**

Probe `for p in pairs()` where `pairs()` yields `(integer, integer)`
tuples — body **never executes** on interp (no panic, no error,
`count == 0`).  Manual `next()` returns the first tuple correctly:
`p.0=1 p.1=10`.  So the divergence is specifically the for-loop driver
× tuple-yielded type combination on interp.  Native fails to compile
on the same shape (E0600 on `!tuple` in the loop's exhaustion test,
plus the @P326-family `as i64` casts on tuple values).

| Test | Status |
|---|---|
| `y4_x1_tuple_for_loop` | **BLOCKED:@P327** (silent wrong answer + native compile fail) |
| `y4_x2_tuple_manual_next` | (not written; interp probe shows it works, but `tuple` return convention may need T1.8a — un-ignore later) |

Phase 04 is the strongest argument for why the validation plan needs to
be active: silently iterating zero times in production code is the
worst-tier failure mode.  Without the matrix, this would have shipped
in a consumer library and produced wrong end-user output with no
crash, no panic, no error message.

**Phase 04 closure (later 2026-05-23):** both halves shipped via
@P327's interp fix + unified `next_into` channel (plan-16 phase 01),
which absorbed the native-side fix.  16/16 matrix cells passing
through Y4.

### Phase 05 (Y5 closure) results — 2026-05-23

**Closure-yielding generators surface a new bug class — filed @P328.**

| Test | Status |
|---|---|
| `y5_x1_basic_closure_for_loop` (`for f in fns()`) | **BLOCKED:@P328** — interp SIGBUS at OpCoroutineNext; native E0308 (`as i32` mis-cast on 20-byte fn-ref).  For-loop's break check IS fixed (extended @P327's `Tuple → Tuple | Function` exhausted check). |
| `y5_x2_closure_manual_next` | **BLOCKED:@P328** — same root: 20-byte yield value has no channel today. |

The fix lives at the same layer as @P326 (DbRef) and @P327 (tuple):
fn-refs need a YIELD CHANNEL in the state machine.  The unified
`next_into(stores, &mut [i64])` channel (plan-16 phase 01) is the
natural next user — write `d_nr` into `dest[0]` and the closure
DbRef into `dest[1..3]` (12 bytes), consumer rebuilds the fn-ref
from those slots.  No new trait method.  Interp also needs a parallel
fix in the bytecode coroutine machinery (`OpCoroutineNext` for
fn-ref sized values doesn't slide the value to the consumer's stack
correctly).

Y5 phase 05 leaves a partial closure: the **for-loop's `OpNot(fnref)`
SIGBUS-and-codegen-fail is fixed** (the @P327 break-check extension);
only the yield-channel half remains.  Other Y5 cells (X3 / X4) compose
on top of this — same gap.

## Harness reuse

```rust
// tests/coroutine_matrix.rs (new binary)

mod common;

cross_mode!(my_coroutine_cell, r#"
    fn count_up() -> iterator<integer> {
        i = 0;
        while i < 3 {
            yield i;
            i += 1;
        }
    }
    fn test() {
        sum = 0;
        for v in count_up() {
            sum += v;
        }
        print("{sum}\n");
        assert(sum == 3, "y1_x1 for-loop sum");
    }
"#);
```

**No new harness code.**  Same cross-mode contract as @PLAN14 and
@PLAN15.  Run with:

```bash
cargo test --release --test coroutine_matrix -- --ignored
# or single cell:
cargo test --release --test coroutine_matrix -- --ignored y2_x1_text_for_loop
```

## Per-cell test inventory

Cell names use the coroutine-specific prefix `y<Y>_x<X>_<sub>` so
they don't collide with @PLAN14's `e<E>_d<D>` or @PLAN15's
`c<C>_d<D>` namespaces.

```
y1_x1_int_for_loop                   // for v in gen() { sum += v }
y1_x1_int_for_loop_helper_yield      // generator calls helper that yields
y1_x2_int_manual_next                // it = gen(); v = next(it)
y1_x3_int_map                        // map(gen(), |v| { v * 2 })
y1_x3_int_filter                     // filter(gen(), |v| { v > 0 })
y1_x3_int_reduce                     // reduce(gen(), 0, |a, b| { a + b })
y1_x4_int_comprehension              // [v for v in gen()]

y2_x1_text_for_loop                  // active risk — yielded text lifetime
y2_x2_text_manual_next
y2_x3_text_map
y2_x4_text_comprehension

y3_x1_ref_for_loop                   // yielded DbRef; record owned by gen
y3_x1_ref_for_loop_parent_owned      // record owned by caller (no rebase)
y3_x2_ref_manual_next
y3_x3_ref_map
y3_x4_ref_comprehension

y4_x1_tuple_for_loop                 // for (a, b) in gen() — destructuring loop
y4_x2_tuple_manual_next              // requires T1.8a — #[ignore]
y4_x3_tuple_map
y4_x4_tuple_comprehension

y5_x1_closure_for_loop               // depends on plan-15
y5_x2_closure_manual_next
y5_x3_closure_invoked_through_map
y5_x4_closure_comprehension

y6_*_vector_yield_rejected           // CLOSED — assert exact diagnostic
y*_x5_yield_from_rejected            // CLOSED — CO1.4 not yet implemented
```

A CLOSED cell uses a manual `#[test]` running `code!(...).error(...)`
in `tests/parse_errors.rs` rather than `cross_mode!`.

## Acceptance for phase 00

- New file `tests/coroutine_matrix.rs` exists with one smoke test
  exercising the harness end-to-end (e.g. `y1_x1_int_for_loop`).
- Matrix table in this file is fully populated — no "TBD" cells.
- README phase ladder matches matrix.
- `make ci` green.
- No production code change.

## Risks

| Risk | Mitigation |
|---|---|
| Yielded text (Y2) surfaces a state-machine lowering bug | Phase 02 starts with X1 alone (smallest cell); if the bug surfaces, the plan pauses, the issue is filed, and a fix lands before extending to X2–X4. |
| Y4 manual-next requires T1.8a | The cell carries `#[ignore = "T1.8a — plan-06 phase 9a"]` until T1.8a lands; un-ignore in a one-line follow-up.  Other Y4 cells (X1/X3/X4) don't need T1.8a because the for-loop / map / comprehension paths receive yielded values via the iterator protocol, not return-by-value. |
| Y5 closure cells exercise both coroutine state-machine AND closure dep tracking; failures may mis-attribute | Phase 05 only opens after @PLAN15 phases 01-04 are green.  If a Y5 cell fails when @PLAN15 is green, the bug is in coroutine × closure interaction, not in either subsystem alone. |
| Test runtime balloons (each cell shells out to interp + native) | Same mitigation as @PLAN14 / @PLAN15: cells `#[ignore]`d by default; on-demand run with `-- --ignored`. |

## Cross-references

- [README.md](README.md) — full matrix; this phase fixes its shape.
- `tests/common/cross_mode.rs` — shared harness.
- [@PLAN14 phase 00](../14-tuple-validation/00-matrix.md) — donor
  template.
- [@PLAN15 README](../15-closure-validation/README.md) — phase 05
  prerequisite (now SHIPPED 2026-05-12).
- [COROUTINE.md](../../../COROUTINE.md) — coroutine design.
