<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN157 — Design

Per-phase invariants, the code sites each phase touches, the load-bearing claims
with their falsifying probes, and the predicted numbers the build is validated
against.  Facts below were read from the tree on 2026-09-07; the measured shares
come from [loft#1426](https://github.com/loft-lang/loft/issues/1426)'s comments.

Baseline per-call/per-pixel budget (the `hash` row): Rust 1.6 ns/call, loft-native
17.7 ns/call ≈ 7 ns prelude (M1) + ~5 ns sentinel/NaN floor (M3) + call + body.
The 4× bar is 6.4 ns/call — no single phase reaches it; P1+P3 together must.

**Priority principle (owner, 2026-09-07): the target is Rust the LLVM layer
CANNOT reduce** — the sentinel branch, the NaN expansion, the opaque store
call.  And the win compounds: LLVM optimises best-effort, declining the
expensive rewrites (vectorise, unroll, LICM) where the inner-loop IR is
already bloated — so emitting the SIMPLE form directly buys both the removed
work and the optimisations LLVM then starts attempting on what remains.
The converse bounds P2: inlining/LTO only exposes a complicated body, which
LLVM then half-optimises — cheap to take, never the lever.  So the emitted-
shape phases (P3, P4) are the priority; P2's `#[inline]` step is minutes and
taken, its LTO half is a probe not an investment.

---

## P0 — the pass runs from this tree

**Invariant:** every later phase's number is produced by one command against one
committed workload whose *correctness* is asserted by the same run (the output
hash), so "faster" can never silently mean "different".

**Shape.** `bench/` rows are auto-discovered directories
(`bench/run_bench.sh:46`); a new row needs `{bench.loft, bench.rs, bench.py}`.
Add **`bench/12_drawing/`**: the issue's standalone reproduction (`noise.loft` +
`brush.loft` concatenated, 24 fns / 567 lines, running the `lock` workload plus
the `hash` 100 000-call row) as `bench.loft`; the consumer's `bench.rs` port; a
trivial `bench.py`.  Both programs **print the FNV-1a-32 hash** and `bench.loft`
asserts it, so any run — timing or not — is a correctness gate.

**Gate vs report.** `run_bench.sh` asserts nothing and `make speed` is doctrine-
bound to never gate; `scripts/profile_corpus.sh` is the one oracle-gates /
drift-reports precedent.  Copy it: `scripts/native_ratio.sh` reads
`bench/ratio_oracle.tsv` (rows: `program · routine · rust-ns · bar`), runs the
loft-native and Rust lanes, **exits 1 on a hash mismatch always** (deterministic,
correctness) and on a ratio breach **only under `--gate`** (timing is noisy; CI
runs report mode, this plan's phases and the release checklist run `--gate`).
The full 14-routine table stays in the consumer
(`loft-libs-graphics/drawing/bench/compare.py`, branch `drawing-lock`) and is
re-run at each phase boundary; the in-tree row is the tight loop.

The baseline table from the issue lands in PERFORMANCE.md beside the N-class
designs, dated, with the command that regenerates it.

**Red:** the asserted hash moves; a ratio in the oracle grows past its recorded
baseline.
**Effort:** XS–S.

---

## P1 — the per-call prelude (M1, ≈7 ns/call)

**Facts.** The prelude is one template
(`src/generation/mod.rs:5039-5044`, injected at
`src/generation/emit.rs:2228`): `live_flipped(idx)` + `cr_call_push(name, file,
line)` + `CallGuard`.  `--lean` strips only `live_flipped`.  `CALL_STACK` is a
thread-local `RefCell<Vec<(&str, &str, u32)>>` (`src/codegen_runtime.rs:5168`)
with **five consumers**: the recursion cap itself (`cr_call_push` tests
`MAX_CALL_DEPTH`, the *unified* depth mechanism — `src/state/mod.rs:693`,
loft#1058 removed the separate counter), native `assert`/`panic` frame blocks
(`runtime_error.rs:281`), the browser panic hook, the `stack_trace()` builtin
(`codegen_runtime.rs:5455`), and the watchdog breadcrumb
(`timeout::checkpoint_fn`, one relaxed load when disarmed — keep it).

**Invariant:** *every observable of the shadow stack — the overflow report, the
frame walk, `stack_trace()`'s vector, unwind balance — is produced by the same
single container; only the container's cost changes.*  No tier split, no
diagnostic divergence between backends, no second depth mechanism.

**Design (primary): make the push cheap instead of optional.**  Replace
`RefCell<Vec<_>>` with a fixed-capacity thread-local: `Cell<usize>` depth + an
`UnsafeCell<[(…); MAX_CALL_DEPTH]>` (240 KB/thread, allocated lazily on first
push).  Push = bounds test + 3 stores + increment; pop (in `CallGuard::drop`,
kept for unwind balance) = decrement.  No borrow flag is needed: the only
reentrancy into the container is `stack_trace()`/panic walks, which read while
no push is mid-flight on the same thread.  All five consumers keep working in
every tier; `--lean`'s meaning is unchanged.

**Load-bearing claims → probes (run before committing to the design):**

1. *"The RefCell+Vec machinery is the ~7 ns, not the two string/int stores."*
   Probe: hand-edit `codegen_runtime.rs` to the array form, rebuild the rlib,
   rerun the standalone `hash` row.  Predict ≤ 2 ns/call (row 1.77 → ≤ 0.9 ms).
   **If the probe lands ≥ 4 ns**, the fallback is the issue's tier: `--lean`
   additionally demotes the push to a bare depth counter, and `stack_trace()`
   /panic frames honestly report the lean tier — a documented, opt-in
   degradation.  Decide on the measurement, not in prose.
2. *"No consumer reads the stack concurrently from another thread."* Probe: grep
   the five consumers for cross-thread access (`CALL_STACK.with` is
   thread-local by construction — confirm no channel hands frames across).
3. *"Unwind balance survives."* The existing overflow + `stack_trace()` tests,
   plus one new test that panics under `catch` (coroutine/`parallel` paths) and
   asserts depth returns to its pre-call value.

**Red:** the A/B (same box, alternating runs, min-of-runs) does not cut the
`hash` row by ≥ 25 % (the ~5 ns/call the push costs, of ~16–18 ns/call); the
recursion-cap test (`cr_stack_overflow`) or any stack-trace/panic-frame test
changes output.  (An earlier draft said "≤ 0.6 ms/100k" — that number was the
issue's P1+P3 combined estimate, not P1's alone; corrected 2026-09-07.)
**Predicted:** `hash` 10.9× → ~4×; `smooth` 262× → < 40×; `lock` −5 %.
**Measured (shipped 2026-09-07):** runtime-only change — `CALL_DEPTH: Cell` +
`CALL_FRAMES: UnsafeCell<Vec>` replace the `RefCell<Vec>`; pop is one `Cell`
decrement; every reader slices `[..depth]` and none can fail the way
`try_borrow` could; the emitted prelude is unchanged, so old generated code
keeps working.  A/B min-of-runs: `hash` 1.61M → 1.18M ns/op (**0.73×**,
prediction ≤ 0.75), `lock` −5 %; hash ratio vs Rust ~3.6–4× on the P0
instrument.  Balance guard `i1058_after_a_contained_overflow_the_call_stack_is_balanced`
(falsified by neutering `cr_call_pop`: red, restored: green); depth-cap,
driver-containment and stack-trace suites unchanged.  The consumer's full
14-row pass (this tree's binary + rlib): **every hash agrees**; `hash`
10.9→8.9×, `smooth` 262→210×.  The `smooth < 40×` prediction was
over-optimistic — the push was ~4 ns of the per-call cost, and the remaining
floor (the `live_flipped` check, the watchdog breadcrumb, the call itself, and
`smooth`'s sentinel-heavy float body) is P3's, so `smooth`'s bulk moves there.
Ops note: a stale `target/release/loft` binary beside the rebuilt rlib makes
`--native-release` fail its LINK with undefined thread-local internals —
rebuild both (`cargo build --release`), per the known lib-only-rebuild trap.

---

## P2 — let the rlib inline (M4)

**Facts.** `--native-release` differs from `--native` by exactly `-O`
(`src/main.rs:11040-11042`).  `Cargo.toml` has **no `[profile.release]`**, so
cargo passes `-Cembed-bitcode=no` and the shipped rlib has no `.llvmbc`
(verified on the artifact) — `-C lto` is impossible today.  The per-element
path is asymmetric: the read chain is `#[inline]`
(`vector::get_vector`, `Stores::store`, `Store::get_*/set_*`) but the write
resolution is not — `Stores::store_mut` (`src/database/allocation.rs:1924`),
`Stores::vec_get_or_raise_runtime` (`src/database/mod.rs:1810`), and
`vector::length_vector` (`src/vector.rs:490`, the one member PERFORMANCE.md:625
lists as marked but isn't).

**Invariant:** *every helper on the per-element read/write path is inlinable at
the consumer's opt level — `#[inline]` in the rlib metadata, or bitcode present
so thin LTO can do it.*

**Two ordered steps, measured apart:**

1. **`#[inline]` the write path** — MEASURED 2026-09-07, and the read-chain
   precedent did NOT transfer wholesale: all three (`store_mut`,
   `vec_get_or_raise_runtime`, `length_vector`) together ran ~+1.5 % SLOWER
   on `lock` — `vec_get_or_raise`'s cold raise machinery duplicates at
   every site.  `store_mut` ALONE (a tiny body, the twin of the
   already-inlined `store`) is a consistent −2 % on `lock`: kept; the other
   two declined by measurement.
2. **Bitcode + thin LTO.** Add `[profile.release] lto = "thin"` to `Cargo.toml`
   (which also stops `-Cembed-bitcode=no`), and `-C lto=thin` beside the `-O`
   in the `native_release` arm.  The profile is the **single chokepoint** — it
   covers `make all`, both `install-artifacts` targets, `rebuild_runtime`
   (`src/native_utils.rs:150`) and `find_problems.sh`'s rebuild; the
   alternative (RUSTFLAGS) is five Makefile sites plus one Rust site, each a
   silent omission.  Omission *stays loud* anyway: `-C lto=thin` against a
   bitcode-free rlib is a hard rustc error — which is also the upgrade hazard:
   a stale installed rlib turns every `--native-release` red until rebuilt.
   Mitigation: the existing self-heal path rebuilds through the same profile;
   `make check-rlib` names it.

**Costs to accept, named for the owner:** `[profile.release]` changes the
pinned release binary's sha (RELEASE.md pins per-release, so the next cycle
absorbs it), lengthens `cargo build --release`, and shifts `make speed`
baselines (a report; re-bless).  If the owner wants the binary untouched
mid-cycle, the fallback is a dedicated `[profile.rlib] inherits = "release"`
used by the `--lib` builds — one more `--profile` flag at each build site, but
still caught loudly by the LTO error if missed.

**Claims → probes:** *"thin LTO moves the number at all"* is untested — the
issue could only try (and fail) `lto=fat`.  Probe before wiring: rebuild the
rlib locally with `RUSTFLAGS=-Cembed-bitcode=yes`, hand-run the issue's rustc
line plus `-C lto=thin`, read the `lock`/`hash` delta.  If it moves nothing
after step 1's inlines, ship step 1 alone and record the negative — do not pay
the build-time cost for a null result.

**Red:** the by-hand rebuild table does not move; `-C lto=thin` stops erroring
on a bitcode-free rlib (the enforcement is gone).
**Predicted:** step 1 alone cuts a visible slice of `lock`'s 7 writes/pixel
(share unmeasured — this probe measures it); step 2 additionally lets
`t_5float_sqrt` and other tiny generated fns fold.
**Effort:** S.

---

## P3 — plain ops for provably non-sentinel operands (M3)

**Facts.** The NaN-aware compares and `ops::op_*` sentinel arithmetic are
`#rust` templates in `default/01_code.loft` (compares :671-678 f64, :450-457
f32; arithmetic :238-260); `src/fill.rs` is generated from the same templates,
so both backends share one definition.  Non-null fast variants
(`op_add_long_nn` …) already exist at `src/ops.rs:487-537` — **dead, zero
callers**.  The emitter already picks cheaper forms from static facts at single
decision sites (`IntCompareEmitter`, `narrow_int_cast`,
`fused_element_read` — each with template fallback).  The counted `for` lowers
with the counter's *null state meaning "not started"*
(`src/parser/objects.rs:3167-3195`), which is why every iteration pays
`op_conv_bool_from_int` + sentinel `op_add_int`.

**The trap this design must own:** "typed non-null" ≠ "cannot hold the
sentinel".  The C80 escape (an overrun `v[i]` typed non-null reads null) means
a non-`Optional` value can dynamically be `i64::MIN`/NaN.  Emitting plain `+`
or `<` there is **native-only silent-wrong** — the worst class in the repo's
own taxonomy.  So the rewrite keys on a *provably non-sentinel* proof, not on
`Type::Optional` absence alone.

**Invariant:** *one predicate, one home, answers "this operand cannot be the
sentinel here", and every cheaper-form emission cites it; every value class the
predicate cannot prove keeps the template form.*  (This is the float/int twin
of @PLN25's `index_provably_fit`, and the home should sit beside it or beside
the Sign lattice in `src/parser/`.)

**Leaf-trust set, first cut (conservative, extensible):** integer/float
literals; results of ops already in the rewritten set (closure); loop induction
variables with proven bounds (`is_active_loop_var` is already a trusted leaf at
`src/parser/fields.rs:1292`); `sqrt`-family results the Sign lattice already
peels.  **Explicitly NOT trusted:** parameters, vector element reads, record
field reads — each can carry the C80 escape.  Widen only with a measured
reason.

**Three sub-steps, measured apart:**

1. **Counted `for`** — when both bounds prove non-sentinel, lower with an
   explicit `i = lo` init and plain increment/compare instead of the
   null-first-iteration encoding.  Parser-side (one home, both backends
   benefit; the interpreter speeds up too).  Matrix cells before building
   (Stage A): exclusive/inclusive × forward/`rev` × literal/expr bounds ×
   nullable-bound (keeps old form) × `break`/`continue` × counter read after
   the loop — hand-computed expectations, both backends.
2. **Arithmetic** — wire the dead `_nn` family behind the predicate at emit
   time (a `NonNullArithEmitter` beside `IntCompareEmitter`, template
   fallback), floats analogously with plain `<`/`<=`/`+`/`*`.
3. **Verification instrument, built FIRST:** `LOFT_NN_VERIFY=1` emits the
   *checking* form — `debug_assert!(v != i64::MIN)` / `!v.is_nan()` before the
   plain op — mirroring `LOFT_HOIST_VERIFY`'s monomorphisation pattern.  The
   full suite plus the drawing pass run under it before either sub-step ships;
   an assert firing names the leaf the proof wrongly trusted.

**Red:** `is_nan()` count in the standalone's emitted file not ~0 on the hot
paths; `ops::op_*` count not sharply down; any `LOFT_NN_VERIFY` assert fires;
any nullable-operand cell loses its sentinel form (checked by the Stage A
matrix's negative cells).
**Predicted:** the inlined `hash` loop 5 ns → ~2 ns/iter; with P1, the row
reaches ~1.2–2×.
**Effort:** M.

**Shipped so far (2026-09-07):**

- **P3a — float compares.** `generation::non_sentinel` (the predicate: float
  literals, the parser's `#ncc` discharge shape — `?? d` and `x?` both lower
  to `if OpConvBoolFromFloat(t) t else d` — negation, if-merge, and a
  re-walking per-function var fixpoint whose escapes are `RefVar`-typed
  arguments, fn-ref args, `TuplePut`/`Iter` slots) + `FloatCompareEmitter`
  for the eight Eq/Ne/Lt/Le × Single/Float ops.  Float arithmetic closure is
  deliberately NOT taken (`inf − inf` mints NaN from non-NaN operands).
  `LOFT_NN_VERIFY=1` emits the checking form; `LOFT_NO_NN_FAST=1` is the
  bisect switch.  Guard `tests/scripts/157-nn-float-compare.loft` (both-side
  cells; falsified by sabotage — proof check disabled turns the null-order
  cell red on native).  Measured: the inner-loop clamp compares in
  `raster_segment` go plain, but only 7 of 83 `is_nan` sites — the remaining
  bulk is the division template's `note_format_fault` (3 tests × 2 divisions
  per pixel, semantics-bound) and parameter-fed compares; `lock` unmoved, as
  the share predicts.
- **P3b — counted `for`.** A forward loop over a literal non-negative `lo`
  lowers with the counter starting at `lo − 1` (folded at parse time) and an
  unconditional increment — no null-encoded first iteration on EITHER
  backend.  The break test needs no proof: a null bound is `i64::MIN`, which
  sorts below every `lo` under both orders, so the empty behaviour is
  unchanged.  Declines: reverse loops, computed `lo`, and any counter whose
  `IntegerSpec.min` cannot hold `lo − 1` (a narrow unsigned counter's −1 IS
  its null sentinel).  16-cell matrix (`157-for-lowering-matrix`,
  hand-computed expectations: empties, break/continue, `_`-binder nesting,
  rev, null-discharged bounds) byte-identical before/after on both backends;
  emitted `op_conv_bool_from*` 34 → 11 on the drawing bench.
- **Measured honestly (idle box, n=50, alternating):** P3a+P3b together move
  the `hash` row ≤ ~5 % — inside run noise — and `lock` not at all.  The
  shapes are the deliverable (they are what LLVM could never remove, and the
  16-cell matrix + verify sweep pin them); M3's TIME on `hash` sits in the
  still-open `_nn` integer wiring (`op_mul`/`op_xor`/`op_and` sentinel chain
  per call) and the division template's `note_format_fault`, and on `lock`
  M2 dominates everything.  The codegen-subject suite under `LOFT_NN_VERIFY`
  ran fully clean (19 binaries, no assert fired).
- **P3c — the `_nn` integer wiring (shipped 2026-09-07).**  One merged
  predicate now serves both families; the integer closure adds literals, the
  int discharge shape, negation, `& non-neg-literal` (result ∈ [0, lit]),
  and `>> lit(1..64)`.  **A SELF-STEP induction (`v = v ± proven`, argued
  from C85) was built and then RETRACTED the same day** — the corpus
  falsified it: `1246-a-nullable-narrow-slot-answers-null.loft` pins that a
  plain integer driven past `i64::MAX` reads null AND that `??` fires on
  it, so the overflow sentinel is an observable value contract; marking a
  self-stepping var proven let `OpConvBoolFromInt → true` disarm exactly
  that discharge.  Its measured worth had been ~0 anyway; a self-stepped
  var now proves nothing, and both 1246's cells and a copy of them in this
  plan's own guard pin the boundary.  `IntArithEmitter`: `+`/`-`/`*`/neg → the (previously
  dead) `ops::*_long_nn` family — checked, so overflow→MIN is preserved
  exactly; `&`/`|`/`^` and literal `>>` → plain operators
  (release-identical — `sentinel_long!` is a debug-only assert);
  `OpConvFloatFromInt` → `as f64`; `OpConvBoolFromInt` → `true`.  Division
  stays out (zero-divisor null is not this proof).  Deliberately NOT closed:
  `+`/`-`/`*` results (overflow), `^`/`|` of proven pairs (two negative
  non-sentinels can compose exactly MIN).  Guard
  `tests/scripts/157-nn-int-arith.loft` (proven cells, null propagation and
  ordering, and the overflow edge answering null THROUGH `_nn`), falsified
  by sabotage (proof skipped → "null propagates through +" red on native).
  Measured: 40 `_nn` sites on the drawing bench (every loop counter);
  timings flat (lock −0.8 %, hash within noise) — as coverage predicts,
  because `seed_hash`'s body roots in PARAMETERS, which stay untrusted.
- **Open in P3 (the coverage frontier):** interprocedural parameter facts —
  a param is proven when every call site passes a proven argument (whole-
  program conjunction at emit time; `hash01` unlocks only through it), and
  store-rooted reads stay blocked on C80 by design.  Weigh against P4's
  ~90 % share before investing.

**The rustc-first probe protocol (owner, 2026-09-07 — use it before EVERY
remaining phase).**  A candidate emission is verified by hand-editing the
already-emitted `.rs` into the proposed form and compiling with bare
`rustc -O` — seconds, no loft rebuild, no validation pass; the asserted
output hashes make each hand-edit self-checking.  Only a transform that
MOVES the number earns emitter code.  First application (probe binaries in
the session scratchpad; timed dirty under a running gate, clean numbers
pending):

Clean matrix (2026-09-07, idle-ish box, `nice -19` single-threaded, median
of 6 × `--n 50`, all four binaries hash-verified `7698ffff`/`33e56005`):

| hand-edit of the emitted bench | `hash` ns/call | `lock` ms/op |
|---|---:|---:|
| P3c as emitted | 11.8 | 32.9 |
| params-proven arithmetic only (would-be P3d) | 10.5 (−11 %) | 32.8 (−0.3 %) |
| **prelude stripped only** (`live_flipped` + `cr_call_push` + guard) | **5.1 (−57 %)** | 29.8 (−9.5 %) |
| both | ~5–6 (≈ prelude alone) | 29.8 |

Rust reference: 3.3 ns/call — the prelude-stripped form is **~1.55×**, well
inside the bar, and the arithmetic's 1.3 ns mostly vanishes once the opaque
prelude calls are gone (the priority principle in numbers: simple emitted
code lets LLVM fold the rest).  **Rulings (owner, 2026-09-07): build the
lean-tier prelude; P3d is measured-and-DECLINED (−11 % alone, ~0 on top of
the prelude fix); P4 gets the same rustc-first probe before any emitter
work** — its share is everything left on the pixel rows (29.8 vs Rust's
1.0–1.7 ms).

**The lean tier is BUILT (same day):** `cr_call_push_lean(file, line)` —
the depth cap without the frame record — emitted whenever `emit_live` is
off (`--lean`); `CallGuard` balances unchanged, `with_call_frames` clamps
so lean readers degrade instead of panicking.  Cells verified: the cap
fires with the clean typed StackOverflow at the entry site, `stack_trace()`
answers zero frames without panicking, all four plan guards green under
`--native --lean`.  Measured real `--lean`: `hash` 8.5 ns/call (~2.6× Rust;
the hand probe's 6.4 ns gap is the cross-crate thread-local access thin-LTO
would inline — P2's probe), `lock` −6 %.

**The probe LEDGER for `lock` (n=20 medians, each layer a hand-edit of the
emitted Rust, hashes exact throughout):**

| cumulative layer | `lock` ms/op | delta |
|---|---:|---|
| P3c baseline | 32.9 | — |
| lean prelude | 31.0 | −6 % |
| + element access via hoisted bases (P4a/b shape: 1 read + 7 writes through per-loop-resolved element-0 `DbRef`s) | 26.3 | **−15 %** |
| + plain `??`-divisions and idx arithmetic | 25.9 | −2 % (the division machinery already inlines) |
| + N4 leaf-strip (no prelude on `chan`/`ramp`/`brush_sample`/`seed_hash`) | 24.1 | −7 %; also takes `hash` 1.11M → 0.68M (−39 %) |

**The value-struct-return probe (2026-09-07, after P4d): the next mechanism,
measured.**  `brush_sample` returns a 4-float `Smp`: the emitted call passes
a NULL retbuf, the callee allocates the record (`OpDatabase`), and the
caller frees per pixel TWICE (`OpFreeRef`) — and that allocation is what
(rightly) blocks every hoist in the resolve loop.  Hand-rewriting the
emitted pair to a `(f64, f64, f64, f64)` return: `lock` 25.1M → 17.1M
(**−32 % — the largest single lever measured in this plan**).  Adding the
resolve-loop element hoists the unblocking enables — the shapes the P4
emitter already produces once the blocker is gone — 17.1M → 14.2M (−17 %
more).  Cumulative: `lock` 32.9M → 14.2M since the plan opened, ≈6.5× Rust.
Implementing value-struct returns for small all-scalar structs is therefore
the queue's head (it buys its own −32 % AND unlocks the committed machinery
for the rest); it is a real mechanism with design surface — which structs
qualify, both backends' call ABI, the retbuf/FnRefBuf interplay — adjacent
to @PLN101's value-struct copy-elision family.  Remaining after it: P4c
record scalars and the bound-via-header fix (`h.len` IS the bound where a
header exists; `len(v)` is a `t_` call per iteration today — probed, hashes
hold).

Interpreter line-profile agrees: the pixel loop's PROJECTION/dist/idx lines
dominate, the writes don't rank — M2's write share was overestimated for
`lock`.  Still ~23× after all layers: the RESOLVE loop (untouched —
`brush_sample`'s four element reads, `ll_out` writes) and the loop/conv
machinery are the next probe targets before any P4 emitter work.

**N4 SHIPPED off the ledger (same day):** `Output::is_elidable_leaf` — the
structural gate (no user calls / fn-refs / `parallel` / `yield` in the
body; see PERFORMANCE.md § Design: N4 for the deviation from the
annotation-driven design and why the cap and diagnostics stay sound) —
elides the whole frame push on leaves in both tiers, live-flip check kept.
The emitter reproduced the probe: `hash` 1.26M → 0.81M named (−36 %,
~2.3× Rust) / 0.75M lean, `lock` −7 %.  All guards, 55-stack-trace and the
error suites green on both backends; `LOFT_NO_LEAF_PRELUDE=1` bisects.

---

## P4 — element access and invariant hoisting under writes (M2, ~90 % of `lock`)

**Facts.** The #885 hoist (`src/generation/hoist.rs`) is all-or-nothing per
loop: ONE write anywhere in the body (`writes_store`, hoist.rs:102-108) kills
every vector's hoist — and an element write `v[i] = x` lowers to
`OpSetFloat(OpGetVector(...), 0, val)` whose params fail the allow-list, so
**the drawing loops (1 read + 7 writes/pixel) hoist nothing today**.
Field-reached vectors (`lay.pix[i]`) are excluded by the bare-`Var`-only rule
at three agreeing sites (hoist.rs:121, :193, `ops/vector_ops.rs:26-31`).
Struct-scalar reads (`lay.x0`) have **no hoist mechanism at all**; pre-eval is
per-statement, keyed by node address — there is no CSE.  Element writes are
emitted by the default template path (`01_code.loft:993-1005` +
`vec_get_or_raise` at :1172, rewritten in `calls.rs:858`); `OpSet*` has no
registry emitter.  The gate is deliberately an **allow-list**
(hoist.rs:302-311) — the parser's `find_written_vars` is a deny-list with known
gaps and `_ => {}` holes (`CallRef`/`Parallel`/`Yield` not descended), so P4
extends hoist.rs's own classification and does **not** import the parser's.
(P8's "one home for the leaf set" remains a sibling concern, not a blocker.)

**Invariant (the one rule):** *a hoisted `VecHeader` `(store_nr, rec, len)` —
and a hoisted record scalar — stays valid across exactly those ops that cannot
change any store's allocation map or any collection's length; a fixed-width
in-place write through an existing element or field is such an op* (element
writes go through `vec_get_or_raise`, which raises on out-of-bounds and never
grows).  Aliasing is free for headers under this rule: an in-place write moves
nothing, so it cannot invalidate any header, aliased or not.  Hoisted scalar
*values* are different — a write CAN target the hoisted field through an alias
— so scalars get the stricter rule below.

**Four sub-steps, each red on its own:**

1. **P4a — headers survive in-place writes.** Add a third classification to
   hoist.rs: `IN_PLACE_WRITE_OPS` — the `OpSet{Int,Float,Single,Byte,Short}`
   shapes whose target is an element address (`OpGetVector(...)`) or a record
   field with const offset.  `hoistable_vectors` becomes two-tier: a body
   whose writes are all in-place keeps its vector headers (scalar hoists
   decline in this tier — see P4c).  Whole-vector/growing ops
   (`OpAppend*`, `OpSetVector`, `OpReplaceVector`, remove/insert/resize/new/
   free) stay writers.  Allow-list doctrine holds: an op not listed costs the
   optimisation, never correctness.
2. **P4b — the fused element WRITE.** Registry entries for the `OpSet*` family
   (`ops/mod.rs:352-362`), a `set_elem_hoisted::<T, VERIFY>` in
   `src/vector.rs` beside `get_elem_hoisted` (bounds test + one store;
   VERIFY monomorphisation re-derives and asserts, riding `LOFT_HOIST_VERIFY`
   unchanged), fallback to the template whenever the header is absent.
3. **P4c — loop-invariant record scalars.** The enumerated missing pieces: a
   collector for `OpGet<scalar>(Var(v), const fld)` with `v` typed
   `Type::Reference`, a second frame map beside `vec_headers` keyed
   `(var, fld)`, an emitter check beside `header_for`.  Validity rule under
   the in-place tier: a hoisted `(v, fld)` scalar is invalidated by **any**
   in-place write whose const offset equals `fld` — offset-keyed, alias-safe,
   cheap; bodies writing that offset simply don't hoist that scalar.  Rebind
   (`Value::Set(v, _)`) removes `v`'s scalars exactly as it removes headers
   today (hoist.rs:117-119).
4. **P4d — field-reached vectors.** Widen the candidate key from `Var(v)` to a
   const path `Var(v).off₁.off₂…` (pure, so the prelude may evaluate it once
   and the fallback may re-emit it — the double-evaluation objection at
   hoist.rs:170-171 only bars *impure* operands).  All three agreeing sites
   move together; `tests/hoist_gate.rs` pins the new shape set.

**Claims → probes:**

- *"In-place element writes never move a record."* Read `Store::set_float` +
  `vec_get_or_raise` for any resize/claim path (expected none — raise on OOB);
  then the constructive probe: `LOFT_HOIST_VERIFY=1` over the drawing pass and
  the full suite — the checking form re-derives every header per access and
  panics on a stale one.  This instrument already exists and is the phase's
  falsifier; extend its coverage to the new write form (P4b's VERIFY arm)
  **before** enabling the tier by default.
- *"The lock loop is covered by P4a+P4c+P4d together"* — the axes are nesting
  (per-pixel loop inside per-segment loop), reads-through-field (`lay.pix`),
  scalar reads (`lay.x0/lw/y0`), and the 7 write sites.  Probe with
  `--native-emit` on the standalone after each sub-step and count the remaining
  `vec_get_or_raise_runtime` / `stores.store(` occurrences in the inner loop —
  the count, not the timing, is the per-sub-step gate; the timing lands at the
  end.
- Composition matrix (Stage A) before P4a ships: pure body × in-place-only ×
  growing-write × rebind × aliased vectors (`w = v; w[i] = x; v[j]`) ×
  field-write at the hoisted offset × nested loop with inner write —
  hand-computed cells, both backends, graduated to `tests/scripts/`.

**Kill switches:** `LOFT_NO_VECTOR_HOIST` covers the whole mechanism already;
the new tier gets its own `LOFT_NO_WRITE_HOIST` for one-step bisection,
following the P2/N-switch precedent.

**P4a/b SHIPPED 2026-09-07.**  `blocks_header_hoist` carries the two-tier
gate (`IN_PLACE_SET_OPS`, 12 scalar setters; a user call that writes stays
blocking — interprocedural in-place is not worth its soundness surface);
`hoist::fused_element_write` + `Stores::vec_set_hoisted_or_raise_runtime`
(bounds test + one typed store; off-fast-path falls back into
`vec_get_or_raise_runtime` + the template's `rec != 0` write, VERIFY
monomorphisation rides `LOFT_HOIST_VERIFY`); the emitter and pre-eval share
the one recogniser.  Two findings along the way: the drawing's RASTER loops
are field-reached (`lay.best`) — P4d's shape, so this slice's bench coverage
is `hair_brush` — and `OpMathFuncFloat`/`Single` (the libm dispatchers) had
never been on the allow-list, so ANY loop calling `sin`/`sqrt` through a
helper never hoisted at all, reads included; they are pure scalar math whose
`const` is a function selector, and joining the list unblocks those loops
generally.  Guards: `tests/scripts/157-write-hoist.loft` (four lanes;
falsified by shifting the fused write's field offset — writes land in
neighbours, c1 red) + four new `hoist_gate` pins (in-place keeps headers,
alias write keeps both, grow-beside-in-place declines, the write shape).
Measured: `hair` 57.2k → 48.4k ns/op (−15 %, ≈3.6× Rust — under the bar);
`hash`/`lock` hashes exact in all four modes.  A cell also pinned the
copy-bind: `u = a` copies, and the tier must not change that.

**P4d SHIPPED (same day): the path key.**  The hoist key widens from a bare
var to `(root, [const offsets])` — `lay.best` is `(lay, [8])` — with ONE
recogniser (`hoist::vector_path`, a `Var` or `OpGetField(path, const,
const)` chain) feeding all three previously-agreeing sites: the candidate
collector, the fused read/write recognisers, and the emission (the prelude
now re-emits the cloned pure path expression; `active_vec_header` keys on
the path).  A rebind of the ROOT invalidates every path under it;
repointing the field itself is `OpSetRef`, outside `IN_PLACE_SET_OPS`, so
it blocks outright.  The raster pixel loop now emits 7 fused writes + 1
fused read + 7 path headers and ZERO per-element resolutions — the exact
shape the hand probe measured.  Guard cells: two SIBLING fields written in
one loop prove the path keys stay apart by value; `hoist_gate` pins the
path recogniser.  Measured (quiet rounds): `lock` −13–18 % (~25.3–27.9M
from ~30.8–32.4M), the ledger's element-hoist share; hashes exact on
interpret / native / `LOFT_HOIST_VERIFY`.

**Red:** `lock` not ≤ 4 ms; `fill_poly`/`composite` not within 4×; any
`LOFT_HOIST_VERIFY` panic; any hash movement; the matrix's negative cells
(growing write, rebind) not declining the hoist.
**Effort:** L (P4a S · P4b S · P4c M · P4d M).

---

## Consumer 14-row re-run (2026-09-07, after P4d)

`compare.py --skip-interp` with this tree's binary + auto-built
`--native-release` cdylibs — so these carry the NAMED prelude, not the
`--lean` gate row (`hash` reads 15.8x here vs 5.4x lean; the gap is M1,
which `--lean` or P2's LTO closes for a consumer).  Every row hashes agree.

| routine | baseline | now | note |
|---|---:|---:|---|
| hash | 10.9 | 15.8* | cdylib carries the named prelude |
| hair | 4.3 | ~3.6 | under the bar |
| smooth | 262 | 202 | call+alloc bound (Pt-vector append) |
| fronds | 49 | 53 | alloc bound (Frond records) |
| lock | 30 | 17.2 | P4 write fusion |
| lock_curved | 34 | 18.0 | P4 write fusion |
| composite | 26 | 21.8 | |
| fill_circle | 17 | 7.1 | |
| fill_star | 17 | 7.5 | |
| wide_line | 17 | 14.0 | |

The write-heavy rasters moved most (P4); smooth/fronds barely moved because
they are call-and-alloc bound — the VALUE-RETURN and vector-append classes,
not anything P1-P4 shaped.  So the two worst rows and lock's bulk all route
to the same next lever (below): the re-run SHARPENED the target rather than
shrinking it.

## Consumer 14-row re-run (2026-09-08, after § V + § V-c)

Same recipe as the 2026-09-07 run (`compare.py --skip-interp`, this tree's
binary, auto-built `--native-release` cdylibs from a scratch clone of
`drawing-lock` — so these carry the NAMED prelude and the cdylib boundary; the
standalone gate row reads `lock` 5.3× where this lane reads 9.2×).  Every row's
hash agrees.  The box had just finished a gate (load ~12 falling), so the
absolute numbers are noisier than the 09-07 column; the ratios are the claim.

| routine | baseline | after P4d | after § V + V-c | note |
|---|---:|---:|---:|---|
| hash | 10.9 | 15.8* | 10.6* | cdylib prelude; the lean gate row is 4.5× |
| hair | 4.3 | ~3.6 | 3.3 | under the bar |
| smooth | 262 | 202 | **200** | UNMOVED — not this lever (below) |
| fronds | 49 | 53 | **53** | UNMOVED — not this lever (below) |
| lock | 30 | 17.2 | **9.2** | Route R + the hoist unblock |
| lock_curved | 34 | 18.0 | **11.9** | same |
| composite | 26 | 21.8 | 22.1 | |
| fill_circle | 17 | 7.1 | 7.5 | |
| fill_star | 17 | 7.5 | 7.9 | |
| wide_line | 17 | 14.0 | 14.4 | |

**The finding corrects § V's own hypothesis.**  The 09-07 re-run read
`smooth`/`fronds` as *"call-and-alloc bound — the VALUE-RETURN and
vector-append classes"* and routed them to the same lever as `lock`.  They did
not move by a point, so their allocation class is NOT a struct-literal return:
it is per-element RECORD CONSTRUCTION into a vector — `pts += [Pt{…}]`,
`fronds += [Frond{…}]`, each element an `OpNewRecord` + field writes +
`OpFinishRecord` in the loop (the ops the § V-c probe showed declining the
ribbon-building loops).  That is a different mechanism with its own design
surface (element construction that neither allocates a temp record nor
copies it into the vector — the append-in-place twin of Route R), and it is
now the lever for the two worst rows.  Re-ranked in the README.

## V-d — append in place: the element record as the retbuf (design 2026-09-08)

**The shape** (from the consumer source, not inferred): `sp_out += [raster::pt(x, y)]`
in `smooth_pts`' inner loop, `fd_pts += [raster::pt(…), raster::pt(…), raster::pt(…)]`
in `fronds` — a vector literal whose ELEMENT is a call returning a struct through
a buffer.  Thirteen such sites in the library (`raster` 3, `drawing` 10).  What
the IR emits per element today:

```
OpPreAllocVector(out, 1, 16);
_elm_1 = OpNewRecord(out, Pt, …);          // the element's record, claimed in out's store
__lift_1 = n_pt(a, b, __ref_1);            // a FRESH store per call — the lift shape declines the buffer
OpCopyRecord(__lift_1, _elm_1, 0x8000|Pt); // 16 bytes copied into the element, the source freed
OpFinishRecord(out, _elm_1, Pt, …);
OpFreeRef(__lift_1);
```

The element record exists BEFORE the call.  The twin of Route R is to hand it to
the callee as its `__retbuf`: R1's guard (*the buffer addresses a record → write
the fields straight into it*) already does the rest.

**Invariant** — the same one as Route R, one place further: *a record built for
a place is built in that place.*  Route R made the place a local's buffer; this
makes it a vector element (and, by the identical lowering at
`objects.rs`'s field-init copy, a struct field: `Box { s: mk(3) }`).

**Measured ceiling (2026-09-08, hand-patched native emit, the Route R method):**
`smooth(20000)` — 20 000 appends of `pt(t², 1−t)`, hash of the result asserted
equal — **12.57M → 2.24M ns/op (−82 %, 5.6×)**, 628 → 112 ns per element, medians
of 7 alternating runs, hashes exact.  The patch was exactly `n_pt(…, _elm)` in
place of the lift + copy + free.

**Two mechanisms, and the first is a contract change:**

1. *Every record-returning callee must treat an offered record as
   reuse-or-allocate — never clear its store.*  R1's `BuildIntoBuffer` does (the
   guarded `OpDatabase`); the NRVO `Rename` path does NOT — a promoted local's
   literal emits a bare `OpDatabase(t)` (the in-place arm of `parse_object` for a
   compiler-generated destination: *"writing the caller's buffer IS its
   purpose"*), and `OpDatabase` clears the whole STORE it is handed.  With an
   element record offered, that clears the vector.  So the same guard lands on
   that arm for a hidden-buffer destination (one site), which also makes the
   R2 buffer reuse sound for NRVO callees.  The same all-scalar-record
   exclusion applies (a synthetic-nullable field relies on the zeroed record).
2. *The element lowering* (`vectors.rs`, the call-element arm that sets the
   free-source bit; and the field-init twin in `objects.rs`): for an element
   that is a call carrying a hidden record buffer whose record is all-scalar,
   emit `_elm = OpNewRecord(v); r = call(…, _elm); if OpDistinctStore(r, _elm)
   { OpCopyRecord(r, _elm, 0x8000|tp) }; OpFinishRecord(v, _elm)`.  The runtime
   test is what keeps every callee shape correct: a callee that minted its own
   store on some path (the 1128 chain), or returned a parameter's record, takes
   the copy — and the free-source bit stays exactly where `is_struct_returning_call`
   puts it today (never on a borrowed return).  `r` is a parser-minted temp
   marked never-free (the copy's bit releases a foreign store; in place there
   is nothing to release), so `scopes` neither lifts the call again nor frees
   the element's store through it.

**Qualifying set:** the callee has a hidden RECORD buffer whose type is
all-scalar (`Pt` yes; `Frond` no — but `fronds`' hot loop appends `Pt`s); the
return is non-nullable AND dep-free (`return_adopts_fresh_store`, Route R's
own shape — the matrix's A2/A4 showed why: a return naming its hidden buffer
sends the temp's `Set` down the copy dispatch, whose `OpDatabase` re-claims
the store the copy's free bit released a turn earlier); the element
expression is the call itself (not a projection of it).  Every miss keeps
today's copy.

**Matrix before code** (each hand-computed, both backends, `LOFT_STRICT_STORES` +
`LOFT_POISON` + leak, and `LOFT_HOIST_VERIFY` where a loop hoists):
- A1 the R1 callee (`pt`) in a loop — the target;
- A2 an NRVO callee (`t = S{…}; t.b = 1.0; t`) — mechanism 1's cell: without the
  guard the vector's store is cleared, so this cell is the falsifier for it;
- A3 a callee returning its PARAMETER (`fn id(p: S) -> S { p }`) — copy path,
  and the free bit must NOT fire on the borrowed return;
- A4 the 1128 chain (early `return r` then a minting tail) — copy path;
- A5 a multi-element literal `[pt(a), pt(b), pt(c)]` — three elements, three
  records, one `OpPreAllocVector`;
- A6 the struct-field twin `Box { s: mk(3) }`;
- A7 an element that is a PROJECTION of a call (`[mk(3).inner]`) — excluded,
  today's lowering;
- A8 a nullable callee (`-> S?`) — excluded;
- A9 the element vector reached through a field (`b.pts += [pt(…)]`) and a
  captured one (a closure appending) — the place is a field DbRef;
- A10 the appended vector read back after the loop, and appended to again
  after a read — the element records are live and finished.

**Predicted:** the standalone probe reaches the hand-patched 2.24M; the
consumer `smooth` row moves from 200× toward the interpolation arithmetic's
own floor (the append was ~5/6 of its per-element cost), `fronds` from 53×
by its inner-loop share; `lock` unchanged; hashes exact.  **Red:** A2 wiping
a vector on either backend is the contract change's failure, and every other
cell is a copy-path or exclusion check.  **Effort:** M.

**SHIPPED 2026-09-08.**  Both mechanisms, both backends, all ten cells clean
under `LOFT_STRICT_STORES` / `LOFT_POISON` / leak; the older guards and the
bench hashes unchanged.  The matrix earned its keep three times over:

- **A3** — the design's own fear: read directly instead of through the lift's
  private copy, the copy's free-source bit released the PARAMETER's store
  (native answered garbage; the interpreter's free-protection bracket hid it).
  The bit now follows the callee's fact: off for a `returns_borrowed_view`.
- **A2 / A4** — one mechanism, on the CALLER side: `nrvo` and `chain` return
  `S["<hidden buffer>"]`, so `return_adopts_fresh_store` is false and the
  temp's `Set` takes the copy dispatch, whose `OpDatabase` re-claims the store
  the copy's free bit released a turn earlier — the recycled number was the
  vector's (native: `vector_append: … the record in this slot is not the one
  the field offset was computed for`; interpreter: the vector store read after
  its free).  The scope is therefore Route R's own: only a callee whose
  return the caller ADOPTS raw (dep-free) receives the element.  The NRVO
  callee keeps today's copy — and joins the § V-b type-level item, since it is
  the same missing fact (a hidden-buffer dep the adopt lowering accepts).
- The NRVO guard (mechanism 1) stays: it makes the callee contract uniform
  and the R2 reuse sound for NRVO callees, at no cost.

*Measured.*  Standalone probe (`smooth(20000)`, native, hashes exact): 15.2M
→ **3.2M** ns/op (the hand-patched ceiling 2.24M; the rest is the temp's
`Set` and the `OpDistinctStore` test); interpreter 29.5M → 14.4M.  Consumer
table, hashes agreeing on every row:

| routine | before | after § V-d |
|---|---:|---:|
| smooth | 200× (75.8 µs) | **104×** (39.6 µs) |
| fronds | 53× (2.75 ms) | **37×** (1.90 ms) |
| lock / lock_curved | 9.2× / 11.9× | 9.3× / 11.9× |

`smooth`'s remaining half is its interpolation arithmetic, the non-call
appends (`sp_out += [p]`, `+= [pts[0]?]` — a copy from a live record, not this
lever) and `half_chord`; the next decomposition is a `make profile` on the
consumer bench.  Switch: `LOFT_NO_APPEND_IN_PLACE`.  Guard:
`tests/scripts/157-a-vector-element-is-built-in-place.loft` (c1–c10) and
`tests/append_in_place.rs`, which pins the EMITTED shape (element claimed,
called with the element, `OpDistinctStore`, no lift) and the NRVO exclusion.

## V-e — the remaining half of `smooth` is the runtime's per-allocation overhead (2026-09-08)

**Instrument.** `perf` became usable on this box (`kernel.perf_event_paranoid` 4 → 2;
the binary was already installed), and the engine profile answered in one run what
the subtraction method would have taken a day of variants to reach.  On a
standalone copy of the consumer's `smooth` row (6-point petal outline, 61 output
points, the same FNV hash), self time by symbol:

| symbol | share | what it is |
|---|---:|---|
| `Vec<Field>::clone` | 7.5 % | the type's field list cloned on every record copy / free walk |
| `getenv` + `strncmp` + `getenv::{closure}` | **~20 %** | `LOFT_STORES` read on every allocation AND every free (`database_named`, `free_named`); `LOFT_LOG=locks` on every free-protect bracket; `LOFT_TRACE_COPY` on every `OpCopyRecord` |
| `enum_parent_size` | 5.7 % | a scan of EVERY type on every record allocation |
| `set_default_value_nullable` | 3.6 % | `Parts` (with its field list) cloned per allocation |
| `set_free_protected` + `format` + `String::clone` + malloc/free | ~5 % | a formatted origin `String` per call with a `const` collection argument |
| `HashMap<u32,()>::insert` + `hash_one` + `Store::valid` + `owned_walk` + `remove_claims_mode` | ~11 % | the claims bookkeeping, SipHash per claim |
| `n_smooth_pts` + `n_pt` + `n_half_chord` | **8 %** | the program |

The program was 8 % of its own row.  The allocations come from `half_chord`'s
borrow-copies (`hc_a = ctrl(…)`, 4 per segment) and the two tangent joins per
segment — ~36 stores per call — and each paid the table above.

**Five behaviour-preserving fixes, each a chokepoint:**

1. `keys::stores_mode()` — `LOFT_STORES` read once (`OnceLock`), matched at both
   sites; `log_config::lock_trace_enabled()` cached; `keys::trace_copy()` cached.
   The policy this file already states — *one cached env read* — applied to the
   three readers that had escaped it.
2. `Stores::enum_parent_size` — the variant row's own `parents` index (a
   `BTreeSet`, written where the variant is registered) instead of a scan of every
   type; the scan stays as the debug-build oracle (`enum_parent_size_by_scan`).
3. The four per-field walks (`copy_claims`, `owned_walk`, the copy-compare and
   size walks) iterate by index through `Stores::field_at`, which yields the two
   numbers a walk reads (position, content) — no list clone, no `Field` clone (a
   `Field` owns its name).
4. `set_default_value_nullable` reads the row's shape into a small `Shape` first
   and matches on that, so the writes borrow `self` mutably without cloning
   `Parts`; `write_declared_default` looks its field up by `(type, index)`.
   The gate caught the inlining dropping a guard: a top-level or array-element
   JSON target carries `field == u16::MAX` and `rec_tp == u16::MAX`, and the
   former `declared_field` refused those BEFORE indexing `types` — inlined, the
   lookup indexed `types[65535]` (`json-walker-absent-field`, 3 of 7 functions,
   both backends).  The sentinel test now sits in `write_declared_default`
   itself, beside `field_declared_nullable`, which asks the same question.
5. `Store::lock_origin` is a `Cow<'static, str>`: the free-protect bracket passes
   `"call_bracket"` borrowed and formats the store/record only under
   `LOFT_LOG=locks`, where someone reads it.

**Measured** (hashes exact everywhere; the fp build of the standalone, medians of
3): 36.5–40k → **18.6–19.4k ns/op** (−50 %); after it `n_smooth_pts` is the largest
line at 13.5 %.  Consumer table, every row's hash agreeing:

| routine | before | after § V-e |
|---|---:|---:|
| smooth | 104× (39.6 µs) | **44×** (16.8 µs) |
| fronds | 37× (1.90 ms) | **24×** (1.23 ms) |
| composite | 23.6× | 19.3× |
| fill_circle / fill_star | 7.2× / 8.0× | 6.0× / 6.9× |
| wide_line | 14.2× | 12.1× |
| lock / lock_curved | 9.3× / 11.9× | 9.0× / 11.6× |
| hair | 3.3× | 3.3× |

**Residual, in profile order** (the next unit's queue): the claims bookkeeping —
`claims: HashSet<u32>` with `RandomState` per claim (`insert` 5.5 %, `hash_one`
4.0 %, `Sip13` 1.6 %), `Store::valid` 3.7 %, `owned_walk` 3.5 %,
`remove_claims_mode` 3.2 % — a faster hasher is one line IF nothing iterates the
set in an order that matters (to be checked, not assumed); the per-field
recursion of `copy_claims` (8.3 %) and `set_default_value_nullable` (6.9 %) on
an ALL-SCALAR record, where a per-type "owns no heap" fact would skip the walk;
`keys::strict_stores()` at 1.6 % per store access (a `OnceLock` read that could
be an `AtomicBool`); and the allocation COUNT itself — the borrow-copies
(`hc_a = ctrl(…)`, a copy of a live element only ever read: the @PLN102
link-widen shape, measured ~0 before and worth ~24 stores per call here) and
the tangent joins.

**Tooling note:** `scripts/profile.sh` refused `perf_event_paranoid = 2`, while
`perf record -e cpu-clock:u -g` on a `-Cforce-frame-pointers=yes -g` native build
works there — the script's check was stricter than user-space sampling needs.
Fixed in this unit: the script accepts `<= 2`, samples `cpu-clock:u` with
`--call-graph=fp`, and PERFORMANCE.md / DEBUG.md state the same bound.

## V-f — the runtime's per-record bookkeeping (2026-09-08)

The head of § V-e's residual queue, worked in profile order on the same
standalone `smooth` row (perf, `cpu-clock:u`, 100 000 reps; timings are
`--native-release` medians of 3 at 4 000 reps).  Every step is
behaviour-preserving: hash `1a36fee4` exact, every consumer hash agreeing.

**1. The claims set is a bitset** (residual 1).  `Store::claims` was a
`HashSet<u32>` under `RandomState`: SipHash per claim and per free and a
hashbrown table per store — `insert` 5.6 %, `hash_one` 4.2 %, Sip13 2.0 %.
The precondition held (nothing iterates the set; a release build reads only
its count), so it is one bit per word position plus a live counter: membership
is a shift and a mask, the set grows on demand to the highest position claimed
(a mapped image this process never allocates into keeps no bits; a fresh
100-word store keeps two words), and the count is kept rather than walked.  The
debug asserts and `fl_validate` read the same `contains`.  18.3–20.7k →
16.1–17.9k ns/op (−10 %); the three hashing symbols left the profile.

**2. Per-type heap facts** (residual 2, and 3).  A record of scalars paid a
per-field descent three times per life: `copy_claims` recursed into every
field to claim nothing, `remove_claims_mode` built an `OwnedChild` per field to
free nothing, and `set_default_value_nullable` wrote each field's zero one call
at a time, after a string compare per field for the variant tag — ~20 % of the
row.  `Stores::heap_facts` derives two facts from `parts` once per type and
caches them on the row (`Type::facts`, an atomic byte — derived, so it takes no
part in equality or the stored form, and `rollback_types_to` forgets it):

- *owns_heap* — can a value own a heap record (text, reference, a collection,
  a child record, a struct or enum variant holding one)?  The two walks return
  at once when not.
- *zero_default* — is the default all zero bytes under every `Absent` mode (no
  nullable field, no declared default, no text, no variant tag, recursively)?
  Then the fill is one `zero_range`.

The match is exhaustive over `Parts` with no catch-all.  It subsumes loft#730's
`type_owns_heap`, which answered `false` for every `Parts::Enum` — a
struct-enum whose variant holds a collection now answers `true`, as
`owned_walk` already walked it.  The memo hit had to be INLINED: as a
recursive function `derive_facts` cost 3 % on its own (a `Vec::new()` and a
call per ask); `heap_facts` now inlines the load and calls it only on a miss.
Beside it: `keys::strict_stores` (read on every store access, 1.8 %) is one
relaxed atomic load with a cold init; `database_named` took its live-store
count — a scan of EVERY slot — on every allocation while only the `LOFT_STORES`
modes read it; and `OpFreeRef` / the interpreter's free resolved `"File"`
through the name map on every free (`hash_one::<&str>`, 1.8–3.7 %) to close a
file handle — both backends now call one `Stores::close_file_handle`, which
tests the stored type's NAME instead.  16.1–17.9k → **12.2–13.8k ns/op**.

**3. `Store::valid` inlined** (the probe the residual list named).  Every raw
accessor (`set_u32_raw`, `get_float`, …) calls `valid(rec, fld)` first; in a
release build its body is the debug asserts' operand — a bounds test and a
header load whose result nothing reads — and, not inlined across the rlib, the
call, the test and the load were all paid: 4.8 % self time.  With `#[inline]`
the dead read and the duplicated bound fall out of every accessor, and the
effect is far larger than the self time said: 12.2–13.8k → **9.7–10.7k ns/op**
(−20 %) on `smooth`, and every consumer row moved — `composite` 17.8× → 12.1×
is the pixel loops' accessor count showing.  P2 had declined two `#[inline]`
candidates for a +1.5 % `lock` regression (duplicated cold raise paths); this
one was A/B'd the same way and `lock` did not move on the P0 instrument
(13.7–14.3M vs 14.0–15.2M ns/op) while it gained 8 % in the consumer lane.
The lesson is the one PERFORMANCE.md § Design already carries for the guard
helpers: a guard's cost is the work it keeps alive in its callers, not its own
body, so an accessor guard is measured inlined and out, never assumed.

**Measured.**  Standalone 18.3–20.7k → 12.2–13.8k (1+2, −35 %) → 9.7–10.7k
ns/op (3, −48 % over the unit); after it the program (`n_smooth_pts`) is the
largest line again.  Consumer table (best of 3, every hash agreeing):

| routine | after § V-e | after 1+2 | after 3 (`valid` inlined) |
|---|---:|---:|---:|
| smooth | 44× (16.8 µs) | 29× (11.0 µs) | **25×** (10.2 µs) |
| fronds | 24× (1.23 ms) | 19× (1.00 ms) | **19×** (0.99 ms) |
| composite | 19.3× | 17.8× | **12.1×** |
| fill_circle / fill_star | 6.0× / 6.9× | 5.2× / 6.0× | **4.5× / 5.2×** |
| wide_line | 12.1× | 11.1× | 9.5× |
| lock / lock_curved | 9.0× / 11.6× | 8.7× / 11.2× | **8.0× / 10.8×** |
| hair | 3.3× | 3.3× | 3.2× |

**An instrument note.**  The P0 gate read `lock` at 4.4×–7.8× across four
back-to-back runs while loft's own number held at 14.0–15.2M ns/op (the
recorded 15.7M): the swing is the Rust REFERENCE lane (1.9M–3.2M) on a laptop
whose clock moves, so a single gate run is not a measurement of loft — the
consumer lane's best-of-3 is (`lock` 9.0× → 8.7×).  The bars stay where they
are; a ratchet reads several runs.

**Residual, in profile order** (what the profile leaves after § V-f):

- **The allocation COUNT** (the largest class now): `database_named` 3.7 %,
  `Store::init` 2.3 %, `claim_block` 2.6 %, `free_named` 2.6 %, `OpFreeRef`
  3.3 %, `OpDatabase` — per-STORE work for ~36 stores per `smooth` call, most of
  them the borrow-copies (`hc_a = ctrl(…)`, the @PLN102 link-widen shape) and
  the tangent joins.  A copy of a live element that is only ever read needs no
  store at all; that is a compiler fact, not a runtime one.
- The vector path: `vector_append` 4.2 %, `length_vector` 3.2 %, `get_vector`
  2.7 %, `vector_finish` 1.8 % — the per-element append machinery § V-d left.
- `set_default_value_nullable` 3.9 % — now the `zero_range` and its call; a
  literal that writes every field could skip the fill (an emitter fact).
- `Parts::clone` 1.7 % — one more per-allocation clone of a row's parts, not
  yet attributed.

## V-g — read-only view elision at a record join (design 2026-09-08)

**The count.**  After § V-f a `smooth` call makes 38 stores: 24 are `hc_a = ctrl(pts,
i - 1, closed)` / `hc_b` — a `Pt` returned by value out of a `const vector<Pt>` parameter,
bound to a local that is only ever READ (`hc_a.ptx`, `hc_a.pty`) — 12 are the tangent-join
retbufs, 2 the vector-literal buffers.  Probed (`bc_probe*.loft`, both backends): the
callee's return type already carries the borrow (`-> Pt["pts"]`, `returns_borrowed_view`),
the ownership oracle already answers `Own::Join { base: pts }` (the `?` fallback mints a
default `Pt` on the out-of-range arm, so the return is a view OR a fresh store per
execution), and the caller's `OpBindOrCopy` MATERIALISES the view arm into an owned store —
that store is the borrow-copy.  The same projection written inline (`a = pts[1]?`) is a
VIEW local with a hidden owner for the minted arm and allocates nothing in range: the
working form exists, one call boundary away.

**The rule, and why this is an elision rather than a change.**  `(O-Move)`: *if the
return borrows a parameter, the caller COPIES to obtain its own store* — a record bind is
independent, and a write through it must not reach the source.  That stays the
semantics.  Where the local is never written, never escapes, and its source cannot be
written while it is live, no program can observe whether it holds the copy or the view;
the copy is then dead work and is ELIDED under a static proof — the same move as a
compiler's copy elision, admissible without touching the rule.  The delivery it elides
to is one the language already performs for a COLLECTION join (loft#1257 / loft#1320,
D-own-16's route): keep the dep, and release the minted arm by STORE IDENTITY
(`OpFreeRefIfDistinct(v, base)`) at scope exit and at a re-Set, per `(O-Detach)`.

**Invariant.**  A record local bound once from a call whose return borrows a nameable,
value-const, single-assigned caller variable, and read only through projections until
scope exit, observes exactly the values a copy would; so it keeps the dep (a view) and owns
nothing but the per-execution minted arm, which store identity releases.

**One predicate, three readers** (`use_analysis::read_only_view_bind`, the loft#810
discipline — the strip in `scan_set` and both backends' delivery must name the same
binds):

1. the bind is `v = call(…)` on a loft-defined callee with a non-nullable record return,
   and `ownership_of` answers `Borrowed { base }` or `Join { base }` with a nameable base;
2. `base` is VALUE-CONST at the caller (`v: const T` — `Const-Value`: no write through it
   in this frame) and single-assigned (`multi_assigned`), so it names the same store at
   every later free (the collection twin's `stable` test; a snapshot witness is the
   widening, not v1);
3. `v` is assigned once, is not a parameter, not never-free, and every other occurrence
   of `Var(v)` is a pure read: arg 0 of a projection or value-reader op.  NOT admitted, each
   a control cell: a call argument (a by-value struct parameter is written through,
   loft#894), a Set-target root, a literal element (`out += [v]` carries the append's
   source-free bit), a return, a closure capture, a `&` bind, a `match` subject, a nullable
   local;
4. `LOFT_NO_VIEW_ELISION=1` restores the copy (the control's name).

**Lowering.**  `scan_set` (the `record_shaped && !adopts_fresh_store` strip): an
elidable bind keeps its deps and registers `lift_join_witness[v] = base` — the transition
free at a re-Set and the scope-exit `OpFreeRefIfDistinct(v, base)` then come from the
collection twin unchanged.  `codegen.rs`'s Call arm and `generation/dispatch.rs` deliver the
result directly (`OpPutRef` / `let var_v = call(…)`) instead of `OpBindOrCopy` /
`OpCopyRecord`.  Nothing new at scope exit.

**Cells — written before the first is worked** (store counts via `LOFT_STORES=log`
labels; value + leak + strict-stores on BOTH backends; c1/c3/c12 must allocate again under
the switch, or the elision is not what moved):

| cell | shape | expected |
|---|---|---|
| c1 | const-param source, one read-only local, in range (the consumer shape) | 0 stores, value exact |
| c2 | out of range — the minted default | value 0, 1 store, freed (no leak) |
| c3 | two locals from one source (`hc_a`/`hc_b`) | 0 stores |
| c4 | a loop re-binding the local, one iteration out of range | identity transition free; no leak, no double free |
| c5 | CONTROL: local written (`a.ptx = 9.0`) | copies; the source reads unchanged after |
| c6 | CONTROL: local passed to a callee | copies |
| c7 | CONTROL: local appended (`out += [a]`) | copies |
| c8 | CONTROL: local returned | copies |
| c9 | CONTROL: local captured by a closure | copies |
| c10 | CONTROL: non-const source (`pts: vector<Pt>`) | copies (v1) |
| c11 | CONTROL: base reassigned after the bind | copies (`multi_assigned`) |
| c12 | `Borrowed`: a field projection return (`fn inner(o: const O) -> Pt { o.p }`) | 0 stores; the identity free a no-op |
| c13 | CONTROL: nullable local (`a: Pt? = …`) | copies |

**Built and measured (2026-09-08).**  The predicate (`use_analysis::view_elision_bind`,
one home), the strip's keep-and-register branch in `scan_set`, the two backends' copy arms
gated on the mark, `LOFT_NO_VIEW_ELISION`, and the mark as a STORED variable field (finding
3).  Store count per run (interpreter / native), elision on → off; every cell's value exact
on both backends under `LOFT_STRICT_STORES=1 LOFT_POISON=1 LOFT_NATIVE_LEAK_CHECK=1`:

| cell | on | off | what moved |
|---|---:|---:|---|
| c1 the consumer shape | 3 / 3 | 4 / 4 | −1: the copy's store |
| c2 out of range | 4 / 4 | 4 / 5 | the minted default kept, then freed |
| c3 two locals | 3 / 3 | 5 / 5 | −2 |
| c4 the loop, one turn out of range | 5 / 5 | 7 / 8 | one minted turn, freed at the block's exit |
| c5–c11, c13, c16 (controls) | = | = | every control still copies |
| c12 a field-projection return (`Borrowed`) | 4 / 4 | 5 / 5 | −1; the identity free a no-op |
| c14 a text element off a LOCAL source | 4 / 4 | 4 / 4 | declined in v1 (the base is not an argument) |
| c15 a bind inside an if-arm | 3 / 3 | 4 / 4 | −1 |
| c17 a field-reached source | 4 / 4 | 5 / 5 | −1 |

**Three findings the matrix produced, each a cell now:**

1. **c11 — a parameter's rebind is not "multi-assigned".**  `v = other` on a value-const
   vector parameter lowers as a copy into a fresh vector buffer that re-points the slot,
   and a parameter has no `Set` of its own, so one rebind leaves it single-assigned.  The
   first build witnessed against the LIVE `v` and freed the CALLER's vector through the
   identity free — invisible in the cell alone (nothing read the vector afterwards) and
   fatal the moment the next cell allocated: the c11;c13 pair under `LOFT_POISON` read
   `0xDEADBEEF` out of main's vector store, found by a prefix-then-pair bisect of the
   guard.  The predicate now declines an argument base with ANY `Set` (`assigned_in`),
   and the collection twin's `stable` reads the same fact — it snapshotted only a
   multi-assigned base, the same hole one mechanism over.
2. **The store the census counted was never the copy's.**  Native pre-allocates a
   `null_named` slot store for every record local bound from a borrowing call (the copy
   lands in it), and the interpreter's `OpDatabase` at the slot does the same; with the
   copy elided the pre-allocation was minted only to be freed as displaced by the adopt
   arm, and the count did not move until the elided local was made to start as the null
   sentinel.  Read off the labelled `LOFT_STORES=log`, not the totals.
3. **The mark is a fact the EMITTERS read, so it must survive the program cache** —
   ownership.md's `__own_<name>` lesson, re-measured before trusting it: under
   `LOFT_PROGRAM_CACHE=1` (a `target/` binary skips the cache otherwise, which made the
   first warm probe vacuous) the warm run re-emitted `OpBindOrCopy` beside the stored
   identity free — correct, and the copy back.  `view_elided` is the eleventh stored
   variable field (`VARIABLE_STRIDE` 37 → 38, `CACHE_FORMAT_VERSION` 5 → 6); the forced
   warm runs then hold at 3 / 3.

**Measured.**  Standalone `smooth` 9.7–10.7k → **7.0–7.8k ns/op** (−28 %); stores per run
82 → 34 — 38 → 14 per call, the prediction.  Consumer (best of 3, every hash agreeing):
`smooth` 25.5× → **20.8×** (10.2 → 7.5 µs); `fronds` 18.9× → 18.9× — UNMOVED, so its
allocations are not this class and the next unit starts from its own census; `lock`
8.0× / 8.2×, `composite` 12.1× / 12.9×, fills 4.5× / 5.2×, `wide_line` 9.5× / 9.4×,
`hair` 3.2× / 3.3× — noise.  Verify: the corpus guard
`tests/scripts/157-a-read-only-record-local-keeps-the-view.loft` (17 cells, a LOCK) and
`tests/view_elision.rs` (the emitted shape on both backends; RED under
`LOFT_NO_VIEW_ELISION=1`, falsified).  `bytecode-comparisons/V-g-read-only-view.md` holds
the before/after IR beside the inline twin.

**Residual.**  `fronds`' class (a census with labels first); the v1 exclusions, each a
widening with its own cell: a LOCAL base bound once and outliving the view (the
collection twin's snapshot-witness route — c14), a non-const source proven undisturbed
between the bind and the last read, a `CallRef` callee (its join has its own witness); and
the callee's `__retbuf` for a projection-returning callee, which is threaded but never
adopted (not a store today — the labelled log showed none per `ctrl` call — so a
signature question, not an allocation one).

## The shipped tier — `--native-release` is lean and fully optimised (owner's rule, 2026-09-08)

Not a compiler unit: a flags decision, asked for by the owner as a clear divergence —
*semantics tests may run unoptimised, performance tests and every built binary run fully
optimised* — and measured before it was made (NATIVE.md § Optimisation tiers).  The
consumer lane had been paying the named per-call frame push and `-O` in every row, which
the gate row (lean, hand-compiled) never did; that was the 4× between the two `hash`
numbers.  Measured on the consumer bench at `--n 50`, two runs each:

| row | `-O`, named prelude | `--lean` | lean + opt3 + `codegen-units=1` |
|---|---:|---:|---:|
| hash | 4.7–4.8M | 1.1–1.3M | 0.9–1.1M |
| hair | 84k | 38k | 37k |
| wide_line | 77k | 52k | 51k |
| composite | 1.34M | 1.12M | 1.08M |
| lock | 13.9M | 12.1M | 11.9M |
| smooth | 8.9k | 7.2k | 7.3k |
| fronds | 1.03M | 0.95M | 0.98M |

Opt-level 3 alone and `-C target-cpu=native` moved nothing; the runtime rlib rebuilt with
one codegen unit moved neither `lock` nor `hash` (its hot accessors are `#[inline]`
already), so the rlib stays on cargo's default profile.  Now the default for
`--native-release` programs and for every library cdylib (`native_lib.rs`, whose recorded
rustc arguments had been `-C opt-level=2` with the named prelude); `native_ratio.sh` and
`run_bench.sh` measure the same tier.  Consumer table after (best of 3, every hash
agreeing): `hash` **9.1×** (16×), `hair` **2.6×** (3.3×), `smooth` 18.9× (20.8×),
`fronds` 18.4×, `lock` 7.8× (8.0×), `lock_curved` 10.7×, `composite` **10.9×** (12.9×),
fills 4.5× / 5.0×, `wide_line` 8.9× (9.4×).  The P0 gate's loft lanes: `hash`
0.72–0.83M, `lock` 12.6–13.3M ns/op (bars kept; the reference lane's swing is the
instrument note in § V-f).  A number in this document taken before this date is on the
old tier and is not comparable to one taken after.

## The floor — what is design and what is closable (2026-09-08)

The owner's question after the shipped tier: *can we still increase performance, and what
are the fundamental reasons to be slower than rustc?*  Answered from two instruments: the
post-§ V-g profile of `smooth` (the program's arithmetic is 28 % of the row; the runtime is
the rest) and the emitted Rust of the simplest row, `n_fnv` — a byte loop at 9× on the
shipped tier.  Per iteration that loop does, beside Rust's four ALU ops:

1. `vector::length_vector(&var__vector_1, &stores.allocations)` for the loop bound — a
   runtime call through the store table, on every iteration, in a loop whose HEADER was
   hoisted one line above (`__vh_1 = vec_header(…)`, `get_elem_hoisted` for the read).
2. `ops::op_logical_and_int`, `op_mul_int`, `op_exclusive_or_int` — each tests both
   operands for `i64::MIN`.  Not because a fact failed to cross the call: the owner's
   ruling (2026-09-08) is that null is MADE by ordinary arithmetic — `a/b`, `sqrt(a)`,
   `a+b` and `a*b` on overflow — so `h0: integer` is non-null at entry and nowhere
   else by declaration, and a check per op is the semantics of a language that never
   halts.  What licenses eliding one is a proof about the value: here `& 0xFFFFFFFF`
   bounds every operand, and a bounded operand cannot overflow.  Range propagation
   over the P3 facts, which already refuse arithmetic for exactly this reason.
3. A `(0..64).contains(&_v_v2)` range test on `x >> 8`, whose amount is a literal.

None of the three is the store model.  What IS the design, and stays:

- **Records and vectors live in a store, not on the stack.**  A field read is one
  indirection through a store base plus a bounds test; Rust reads a stack offset.  With
  headers hoisted the per-access cost is one load, one add, one compare — the price of
  serialisation, live editing, shared stores and the never-crash goal (GOALS.md).
- **Every scalar op can make a null, so every op checks** — `a/b`, `sqrt(a)`, `a+b`
  and `a*b` on overflow produce the sentinel, which is how loft never halts on
  arithmetic.  A declaration says nothing past the entry.  The owner named the only
  two ways to owe fewer checks (2026-09-08): a proof about the VALUE (a bound, a mask,
  a divisor proven finite and non-zero — finding 2 above, compiler work inside this
  plan), or an explicit opt-in to the PROCESSOR's semantics — wrapping integers and
  IEEE floats as plain values, machine-dependent by declaration, in a scope or a type
  the author chooses, with a stated rule for a value that crosses back into the
  null-checked world.  The second is a language surface: frozen, owner-decided, its
  own plan when wanted; it is recorded here so the fork is not re-derived.

What is NOT the design, each with its queue item (README § Phase ordering):

- ownership decided at run time (copy-or-adopt, identity frees, the call bracket) — each
  decision § V-g showed can move to compile time;
- a struct temporary is a store creation and a vector growth is a claim in a general
  arena — frame-local records and a vector-specific growth path;
- the program and the runtime are two crates, so only `#[inline]` crosses — LTO.

**The floor this predicts:** a record-heavy loop with hoisted headers, known non-null
facts and frame-resident temporaries sits near 1.5–2× of Rust.  `hair` at 2.6× is at
that floor already; every row at 8–19× is there for a reason in the closable list, and
`hash`'s three are the cheapest of them.

## fronds — the census, the ceiling, the profile, and the bump claim (2026-09-08)

**The instrument.**  A standalone copy of the consumer's `fronds` row (drawing.loft's
`fronds` + helpers, noise's `seed_wave`, raster's `Pt`, the bench's four-byte FNV), hash
`ebcfd875` and 1296 points exactly as the consumer bench prints them, on both backends —
`vr_fronds.loft` in this session's scratchpad, rebuilt from the consumer's sources by the
same recipe as `vr_smooth.loft`.  Baseline 0.97–1.06M ns/op at `--native-release`.

**The census** (labelled `LOFT_STORES=log`, per call): a `fd_sides` literal per frond, a
`fd_pts` and a `fd_wid` builder per side, a `FrondSpec` record per sub-array, the
recursive call's return buffer and its result vector — ~150 stores per call.

**The ceiling, measured before any compiler work.**  Variant A rewrites the source the
way a compiler could: the sides literal becomes an index loop, the two builders append
straight into the appended element (`fd_out += [Frond { fpts: [], fwid: [] }]` then
`fd_out[fk].fpts += […]`), the sub-array spec is made once with its seed re-assigned.
Hash exact on both backends, stores per run 305 → 109 (−64 %) — and only **−9 %** in time
(0.89–0.93M).  After § V-e/V-f a store is cheap enough that `fronds`' allocation class is
worth a tenth of the row, not the half the census suggested.  The lesson for the queue:
count stores to find a class, but time a ceiling before ranking it.

**The profile says where the row is.**  The program (`n_fronds`) is 6 % of its own row.
Two runtime halves carry the rest: DEEP RECORD COPIES — `copy_claims` 8.6 %, `owned_walk`
7.2 %, `remove_claims_mode` 2.8 %, `copy_block` + `memmove` 3.6 % — the sub-call's result
Fronds copied one by one into the parent (`for f in fronds(…) { fd_out += [f] }`, each
with its two inner vectors, then the source freed); and the STORE ARENA — `addr_mut` 13 %,
`claim_block` 5.1 %, `claim` 2 %, `fl_find_ge` 2 %, `fl_delete_node` 1.7 % — the free-list
allocator per claim.  Two probes on the second half:

- `#[inline(always)]` on `addr`/`addr_mut`/`offset_in_bounds` (they were `#[inline]` and
  still showed as symbols inside the runtime): within noise on `fronds`, `smooth`, `lock`
  and `hash` — reverted; not worth a lint allowance.
- **The bump claim** (`Store::bump_tail`, shipped): a store that has freed nothing has
  ONE free block, its tail, and every claim takes its front — through the tree that was a
  delete of the root, a split and an insert of the remainder, three LLRB walks to move one
  number.  The remainder now stays the root in place; it declines for any other shape
  (two nodes, a non-tail block, a tail the split rule claims whole, a remainder below
  `MIN_FREE_TREE`), so the layout is byte-identical to the tree path's — pinned by
  `store::tests::bump_tail_claims_are_the_tree_paths_layout`, the store subject suite's
  layout goldens, and the `fronds`/`smooth` hashes.  `fronds` 0.97–1.06M → **0.92–0.93M**
  (−7 %); `smooth` and the gate rows inside noise.

**What is left for `fronds`, in order:** the deep-copy class (a quarter of the row) — the
sub-call's result is a temporary whose elements die after the loop, so appending them
should MOVE the records and adopt their inner vectors rather than re-claim and copy them;
or the callee builds into the caller's vector (an accumulator shape at the language level,
a Route-R-for-vectors question at the compiler level); then the allocation class (the
9 % variant A measured) as a set of small emitter items (a constant vector literal hoisted,
struct-field collections built in the element, a loop-scoped record literal reusing its
store).

## V — value-struct returns (the queue's head after P4)

**Invariant:** *a qualifying return has no identity — no consumer can
observe its record.*  loft's copy-bind semantics already make a returned
struct value-like (`u = a` copies; the 157-write-hoist guard pins it), so a
small all-scalar struct that is constructed at return position and only
ever field-read by its caller can travel as a Rust tuple: no `OpDatabase`
allocation, no retbuf, no per-call `OpFreeRef` pair — and, decisively, the
CALL stops being a store-writer, so the enclosing loop's hoists unblock
(the committed P4 machinery fires by itself).

**Measured (the hand probe, 2026-09-07):** `brush_sample`'s `Smp` rewritten
to `(f64, f64, f64, f64)`: `lock` 25.1M → 17.1M (−32 %); + the unblocked
resolve hoists → 14.2M.  ≈6.5× Rust.

**Qualifying set, first cut (every miss keeps the record form):**
- the return type is a struct whose EVERY field is a fixed-width scalar
  (integer/float/single/boolean/character/plain enum) — no text, vector,
  reference or nested-struct fields; ≤ 8 fields;
- the return is NON-nullable (`-> Smp`, not `-> Smp?` — null needs the
  record's absence spelling);
- the function is not a coroutine, takes no fn-ref dispatch (the live tier
  already declines undispatchable returns via `live_entry_check`, so
  live-flip naturally excludes these), and is not itself reachable through
  a fn-ref;
- every CALL SITE either only field-reads the result or materialises: a
  site that stores/passes/returns the record gets an emitted
  materialisation (allocate + write the tuple back) — correctness by
  fallback, the same doctrine as every hoist in this plan.

**Failure paths to pin before code (each a matrix cell):** an escaping call
site (stored into a struct/vector/returned onward) · a nullable twin ·
interpreter parity on every cell (records there, values here — outputs must
match byte-for-byte) · a fn-ref taken to a qualifying fn · a coroutine
yielding one · the `__retbuf`/FnRefBufGuard interplay at mixed call sites.

**Pre-build probes:** (a) the measured probe above; (b) a corpus census —
how many definitions qualify and what their call-site shapes are (the
blast radius, and whether materialisation is rare enough to be the
fallback); (c) `LOFT_NO_VALUE_RETURN=1` as the bisect switch from day one.

**The implementation fork (found 2026-09-07; owner input welcome):** struct
literals are ALREADY lowered to `OpDatabase` + `OpSet*` sequences in the
IR both backends share — there is no `Value::Object` node to re-emit.  Two
routes:

- **Route T (value tuples — the probed ceiling):** native-only recognition
  of the lowered constructor block at return position, re-emitted as tuple
  construction; call sites receive registers.  Cleanest result, hardest
  emission surgery (pattern-matching a statement RUN, the shape this plan
  has otherwise avoided).
- **Route R (the retbuf contract — smaller, reuses machinery):** the
  constructor honours a caller-PROVIDED retbuf instead of allocating
  (`var___retbuf` already exists in the ABI, today ignored by struct
  constructors); the caller hoists ONE buffer out of its loop.  Kills the
  per-call alloc + both frees.  The loop-hoist unblock then needs one
  def-level fact — "this callee's only store writes are into its retbuf" —
  a narrow, attributable exception to the no-interprocedural-in-place rule,
  auditable per def.  Less than the tuple ceiling (the record write/read
  round-trip stays) but most of the alloc win, at S–M instead of L.

**Both ceilings probed (2026-09-07, quiet box, n=50 medians, hashes exact):**

| form | `lock` ns/op | vs P4d |
|---|---:|---|
| P4d (committed) | 25.1M | — |
| Route R (retbuf reused, alloc + 2 frees/pixel gone, resolve hoists on) | 17.8M | **−29 %** |
| Route T (value tuple, no record round-trip) | 14.2M | −43 % |

**Decision: Route R.**  It captures −29 % of the −43 % available — nearly
all the allocation win — at S–M effort reusing the existing `__retbuf`
ABI slot, versus Route T's L-effort statement-run pattern match (the one
shape this plan has otherwise refused).  T's extra −13 pts is the record
write/read round-trip that R keeps; a later, separate item if `lock` needs
it after R + P4c.  R's one new fact — "this callee writes only into its
retbuf" — is a narrow, per-def-auditable exception to the
no-interprocedural-in-place rule, gated by `LOFT_NO_VALUE_RETURN=1`.

**Effort:** M–L (T) / S–M (R).  **Red:** any consumer-pass hash moves; the
census's escaping-site cells answer differently than the record form;
`lock` not ≤ ~14.5M (T) / ~17M (R) on the P0 instrument once landed.

**SHIPPED 2026-09-08 — Route R, both halves, both backends, hashes exact.**

*Callee half* (`Parser::classify_reference_delivery` → `RefDelivery::BuildIntoBuffer`):
a return-position `"Object"` block's work-ref is substituted BY the `__retbuf`
variable, and its `OpDatabase` becomes
`if <__retbuf addresses a record> null else OpDatabase(__retbuf, tp)` — the field
writes land in the caller's record when one was offered, and a site offering
none (a fn-ref dispatch, the host entry, a `parallel` worker, a caller whose
buffer the gate below declined) mints exactly as before.  The SIGNATURE is
untouched, so `return_adopts_fresh_store` stays true and the caller keeps its
adopt-with-witness lowering (`OpFreeRefIfDistinct(v, __ref_N)`), which is the
ABI leg that always answered *did the callee fill my buffer or mint its own?*
at run time — this makes the first case common instead of rare.  Two shapes
were built and refused on the way: `ref_return`'s `Rename` (publishes a return
dep → the caller COPIES via `gen_set_first_ref_call_copy`, one record copy per
call — the `s = S{…}; s` shape is SLOWER than the literal it replaces), and an
unconditional `OpDatabase(__retbuf)` (it clears the whole STORE, which for the
placement wire's return arena destroys the record the other process reads
back — `placement_parity` red, 4 cells).  Refused for good: a struct with a
synthetic `__nullable<S>` field, which `object_init` leaves to the zeroed
record and a reused record is not zeroed.  And *"addresses a record"* has TWO
spellings, both refused: the null store (`rec == 0`) and the freed slot
(`store_nr == u16::MAX`, which a native free leaves with `rec` standing) — a
`rec`-only test wrote into store 65535 when `rb_cond`'s chain freed its buffer
variable and handed it on (`1128-…-frees-what-it-displaces`, native only).
Switch: `LOFT_NO_VALUE_RETURN`.

*Caller half* (`scopes::reuse_record_buffers`): `OpDatabase(__ref_N, tp)` is
inserted after the buffer's preamble null-init — an IR edit, so both
generators emit it and neither knows why (the pair `parse_object`'s in-place
arm and the vector twin already emit).  **The gate is `witness_buffer`:** only
a buffer whose result's free is already `OpFreeRefIfDistinct(v, __ref_N)`
(@P378(a)), which exactly one user call receives, and whose result local is
assigned ONCE — a reassignment (`v = mk(i); v = other`) frees the store it
displaces through each backend's set lowering, and under reuse that store is
the buffer's (found by the free-guard matrix the day after shipping: a
use-after-free on every turn after the first, values still right, both
`--interpret` under `LOFT_STRICT_STORES` and — by the same trace — native).  A buffer reached any
other way — `keep += [mk(i)]`, whose result lands in a `__lift_N` temp with a
plain free — is left null.  Positive control `LOFT_NO_RETBUF_WITNESS_GATE=1`
(allocate every buffer): `LOFT_STRICT_STORES=1` reports USE AFTER FREE at
exactly that cell, automated in `tests/retbuf_reuse.rs`.  Switch:
`LOFT_NO_RETBUF_REUSE`.  Guard: `tests/scripts/157-a-struct-return-builds-
into-the-callers-buffer.loft` (ten cells: inline read, local bind, vector
escape, field escape, forwarded, two live, survives a later call, declared
default, zero default, nested).

*The one static fact that moved:* `Definition::site_is_fresh` read an ARGUMENT
tail as *a store the caller already holds*, right for `fn id(a) -> T { a }` and
wrong for the hidden buffer, which exists FOR the return; `value_return_buffer_var`
names it as the one argument that answers owned, while the return publishes no
dep.  Found by `leak_cases/clean/i1273_generic_delegating_return_inline` —
40 records leaked once `OpAdd`'s literal built into its buffer and the caller's
lift stopped owning the answer.

*Census* (the `LOFT_NO_VALUE_RETURN` A/B on one binary, `loft introspect`):
the whole stdlib is byte-identical — zero sites qualify there (every heap
return is a promoted local, a vector, or `#rust`); the bench moves six literal
sites (`Smp`, `Row`×2, `PathPt`, `Brush`, `LockStyle`).

*Measured* (`--native-emit` + `rustc -O`, medians, hashes exact).  Two boxes:
the first pass ran with a post-reboot load of ~3, the re-measure after the
gate narrowing on the quiet box — the row to compare with the P4d baseline
above (25.1M) is the quiet one:

| form | `lock` ns/op | `hash` ns/op |
|---|---:|---:|
| loaded box: both switches off (P4d) | 47.8M | 2.23M |
| loaded box: callee half alone | 48.6M (a wash, as predicted) | — |
| loaded box: both halves, gate on | 35.3M (−26 %) | 1.88M (−16 %) |
| loaded box: both halves, gate off (the ceiling) | 32.2M (−33 %) | — |
| **quiet box: both switches off** | 25.6M | 1.50M |
| **quiet box: both halves, gate on (shipped)** | **17.9M (−30 %)** | 0.83M |

The shipped form lands on the § V prediction for Route R (17.8M) to the
decimal.  The 7 pts between the gated and ungated forms are the sites the
gate declines.  The widening that lifts every one of them is to guard the FREE
instead of gating the allocation: each release of a buffer-fed value — the
lift temp's, the displacement on reassignment, the scope exit — becomes
`OpFreeRefIfDistinct(v, __ref_N)`, after which "allocate every buffer" is
sound by construction.  The free-guard matrix (M4–M9 in the guard, cells
c12–c17) is the instrument for it, and its native control reads a WRONG
value (`c5 forwarded: 100`) where the interpreter reads a use-after-free —
a third spelling of absent the callee cannot see, the caller's buffer naming
a store the lift's free released.

### V-b — guard the free, not the allocation (the widening; design 2026-09-08)

**Invariant:** *a store two variables name is released by exactly one of them,
and never while the other still names it.*  A buffer-fed result local `v` and
its buffer `__ref_N` name one store after every call that reused the buffer;
today the gate keeps the allocation away from any `v` whose releases are not
all guarded.  The widening turns it round: every release of `v` — scope exit,
per-iteration, the lift temp's, and the displacement a reassignment performs
— is `OpFreeRefIfDistinct(v, __ref_N)`, after which allocating every buffer
is sound by construction and the gate's declined sites (7 pts) land.

**The fact already exists.**  `Scopes::witness_buffer` (v → buffers, @P378(a))
and `paired_witness` (buffer → v) are *"v may hold `__ref_N`'s store"* in two
directions; `fed_by(v)` is their union.  Two producers are missing and are
registered at creation: a **lift temp** bound to a buffer-carrying call
(`keep += [mk(i)]`, `t += wrap(i).a` — `Parser::collect_hidden_ref_args` on
the lifted value names the buffer), and a same-scope local through the
`paired_witness` direction (`v = mk(5); v = mk2(6)`).

**The one new reader** is a rung in the Set arm's `transition_free` ladder,
placed first: for `Set(v, rhs)` with `fed_by(v)` non-empty and `v` in scope,
the prefix becomes
`if OpDistinctStore(v, B1) { if OpDistinctStore(v, B2) { OpFreeRef(v) } }` —
the nested emitter the scope-exit sweep already uses for a multi-buffer `v` —
followed by `OpInitRefSentinel(v)`.  The sentinel is what makes the change
IR-only: both backends' set lowerings still emit their own displacement free
(`owns_displaced_store` → the pre-Set `OpFreeRef` on the interpreter; the
`_old_<name>` stash on native), and both are no-ops on the null sentinel
(`Stores::free` and `codegen_runtime::OpFreeRef` return on `u16::MAX`).  It is
the detach shape `parse_object` already emits for a rebindable parameter
(`OpFreeRefIfDistinct(p, orig); OpInitRefSentinel(p)`, @FR-O-Detach), so
neither generator learns anything new and the IR codec carries no new field.
The rhs is untouched: a copy (`v = other`) allocates fresh from the nulled slot
instead of clearing the buffer's store in place, which is the point.

**The allocation rule** (`reuse_record_buffers`) then reads: one user call per
buffer, and the call's result local has a recorded witness (`fed_by` reaches
it).  `multi_assigned` goes.  A buffer with no witness — a call whose result
the scan never paired — stays null: the widening is a widening, not a removal
of the gate.

**Positive control:** `LOFT_NO_RETBUF_FREE_GUARD=1` — allocate every buffer
AND drop the guards (the lift registration and the new rung); the two
automated controls (escaping, displaced) read the use-after-free under it, as
they do today under the allocation-only switch it replaces.

**Matrix before code** (cells c12–c17 stand; these are added, each with a
hand-computed value, both backends, `LOFT_STRICT_STORES` + `LOFT_POISON` +
leak):
- M10 a NULLABLE-typed local fed by the call (`v: S? = mk(i)`), reassigned in
  the loop — the nullable set lowering routes through its own guarded free;
- M11 the rhs READS `v` (`v = mk(v.a + 1)`): old == new == the buffer, so the
  guarded prefix must decline and the call must still see the old value;
- M12 a buffer-fed nullable local displaced by `null` — `Set(v, Null)` takes
  the early exit in the Set arm and must still release nothing it does not own;
- M13 a lift inside an `if` arm inside the loop (`if c { keep += [mk(i)] }`) —
  the lift's scope is the arm;
- M14 the buffer-fed local RETURNED after a reassignment
  (`v = mk(1); v = mk2(2); v`) — `in_ret` suppresses `v`'s exit free, B1's
  exit free must fire, B2's must decline;
- M15 two live buffer-fed locals from one site through a copy
  (`a = mk(1); b = a; a = mk(2)`) — `b` is a copy and owns its store.

**Predicted:** `lock` 17.9M → ~16.3M on the quiet box (the ungated ceiling's
ratio, 32.2/35.3), hashes exact; every c12–c17 and M10–M15 cell clean on both
backends; the two controls red under `LOFT_NO_RETBUF_FREE_GUARD=1`.
**Red:** any cell moves, either control goes quiet, or the ceiling is not
reached — the last says a site still declines, and the census names it.

**BUILT AND REVERTED the same day (2026-09-08) — the prediction was falsified,
and what falsified it is the finding.**  The rung, the lift registration and
the widened rule were built exactly as above and reverted in the working tree
by inverse edit — nothing of it is in git; the shapes are fully specified in
this section, and the matrix cells are in the guard (c12–c23).  Result: M11 answered
`null` on native, c5 answered 202, the escaping control went quiet on the
guarded side (92 strict-store violations on the guard), and `lock` read 18.3M
— no better than the gated 17.9M.  The IR shows why, and it is not a bug in
the rung: the lift temp's own scope-exit free IS guarded after registration.
Three OTHER release sites act on the static premise *"a dep-free return is a
fresh store"* — the premise Route R deliberately keeps, since it is what makes
the caller adopt instead of copy:

1. the vector append's `OpCopyRecord` **source-claim bit** (`0x8000`,
   loft#953's `answers_caller_buffer`): `keep += [mk(i)]` copies the lift's
   record into the element and FREES THE SOURCE — the buffer's store, so the
   next turn writes a freed one (c3's use-after-free, both backends);
2. a forwarding wrapper's return type names its buffer (`wrap -> S["__retbuf"]`),
   so the caller takes the adopt-or-copy dispatch (`gen_set_first_ref_call_copy`)
   and clears a store in place that the guarded free had already released (c5);
3. the rung itself nulls the slot BEFORE the rhs runs, so an rhs that reads the
   local (`v = mk(v.a + 1)`) reads the sentinel (M11) — a snapshot form fixes
   that one, but it is the least of the three.

Each of these is a PARSER-level ownership decision, so guarding the frees one
site at a time is the per-shape accretion the codegen method refuses (loft-codegen
skill, *"the patch is growing per-shape conditions → the fact belongs in the
type"*).  The honest next design is its *"types next"* step: the fact *this
value may be the caller's buffer* has to travel in the return TYPE — a dep
naming the buffer attribute that the ADOPT lowering accepts without copying
(today a buffer dep means *copy*, which is why Route R kept the dep off) — so
every static reader (the copy claim, the lift adoption, `site_is_fresh`, the
displacement) reads one fact.  That is M–L and a separate item; the gated form
above is what ships.  `LOFT_NO_RETBUF_WITNESS_GATE` stays the control's name.

### V-c — the hoist unblock (design 2026-09-08)

**Measured blocker.**  An env-gated print in `blocks_header_hoist`'s call arm
(inserted and removed the same hour) names what declines the resolve loop
(`lock_layer`'s `loop_74`): `n_brush_sample` alone — inside it the retbuf
writes (`OpDatabase(__retbuf)`, four `OpSetFloat(__retbuf, …)`) and
`OpRefIsNull(__retbuf)`, which the allow-list reads as a writer only because
its parameter is a reference (it is `store_nr == u16::MAX`, a pure read; the
R1 guard is what put it in a hot body).  The guarded frees in the loop are
already non-blocking.  So the unblock is two allow-list entries, each with an
argument, not a new mechanism.

**Invariant:** *a hoisted vector header stays valid across a call whose only
store writes are fixed-width scalars into its own retbuf record.*  The
argument is `IN_PLACE_SET_OPS`'s, one call deep: a scalar set through an
address moves nothing and changes no length; `OpDatabase` on the buffer
allocates from a null slot or clears the buffer's own store, and a store is
its own allocation, so no other store's header moves; and the buffer's store
cannot itself host a hoisted header, because the record is ALL-SCALAR — no
collection, text or reference field — and the loop never names the buffer
variable.  A callee with a collection field in its record (the record grows),
one that writes a caller-passed record, or one that calls a writer, stays
blocking.

**The fact** (`hoist::retbuf_only_writer`, memoised beside `call_writes_store`):
the def has a hidden return buffer whose record type is all-scalar; every
store-writing native op in the body is `OpDatabase` or an `IN_PLACE_SET_OPS`
member whose first argument IS the buffer variable; every other native op is
store-free; every user call is non-writing; no `CallRef`/`Parallel`/`Yield`.
`call_writes_store` answers *false* for such a def.  Switch:
`LOFT_NO_RETBUF_HOIST=1` (generation-time, the before-half of the A/B).

**Pinned:** `tests/hoist_gate.rs` — a loop calling a `brush_sample`-shaped
callee hoists; a callee whose record has a vector field, one that writes its
parameter, and one that calls a writer keep the loop declined.
**Falsifier:** `LOFT_HOIST_VERIFY=1` on the bench and the guard (the checking
form re-derives every hoisted header and panics on a stale one).
**Predicted:** `lock` 17.9M → ~15M on the quiet box (the −17 pts § V attributed
to the resolve hoists), hashes exact.  **Red:** a `VERIFY` panic, a hash
moving, or `lock` not moving — the last says another op in the loop still
declines, and the same print names it.

**SHIPPED 2026-09-08.**  The callee verdict alone moved nothing — the same
print, re-armed, named the SECOND blocker: `OpFreeRefIfDistinct(ll_smp,
__ref_2)`, the guarded per-iteration free of the buffer-fed result, a writer
to the allow-list only because its operands are references.  Admitted by the
operand's TYPE (`hoist::frees_a_record`): a free releases one store and moves
no other, and the store a hoisted header describes is a loop-invariant
vector's, live across the loop — so a RECORD variable's release cannot be it;
a vector-typed operand (a per-iteration vector local, a loft#1201 vector
work-ref) keeps declining, and so does a free whose body the gate cannot see
(`vars == None`, the external `may_write_store` callers).  Both admissions ride
one switch, `LOFT_NO_RETBUF_HOIST`.  With both, `lock_layer`'s resolve loop
carries **9** hoisted reads and fused writes (was 0); `LOFT_HOIST_VERIFY=1` is
clean on the bench and the guard, hashes exact.  Measured on the quiet box,
nine alternations of one binary pair: `lock` 18.6M → **15.7M** (−16 %; minima
18.1M → 15.0M), on the prediction; `hash` is untouched — its two functions are byte-identical between the halves, so the row's sample noise is noise.  P0
instrument: `lock` **5.3×** (bar 10 → 8), `hash` 4.5×.  Pinned in
`tests/hoist_gate.rs` (`a_scalar_retbuf_writer_does_not_decline_the_hoist`):
the scalar-retbuf callee and the record free hoist; a record with a vector
field, a write to a parameter, and a write to a scratch local decline — and
the promoted-local shape (`t = mk(u); t.b = 1.0; t`) is a retbuf-only writer,
which the first cut of the pin got wrong: NRVO makes `t` the buffer.  P0 instrument on the idle box:
`lock` **6.2×** (bar 16 → 10), `hash` 5.1× (bar 7).  Still open from this
section: the hoist unblock — the def-level *"writes only into its retbuf"*
fact the resolve loop's P4 hoists wait on (the −17 pts attributed above).

## P5 — the pass as the per-library standard

A `LIBRARY_CHECKLIST.md` row: a published library carries a `bench/` with a
same-hash reference lane and a recorded ratio table; `drawing` is the first
entry.  Wording routes through the owner (it changes what a review accepts).
**Effort:** XS.

---

## Order and the numbers to validate against

P0 → P1 → P2 → P3 → P4 → P5; P1–P3 are independent of each other.  Written
prediction for the judged rows at plan end (validate the build against these,
per the design protocol — a divergence is an alarm to route, not to override):

| row | today | after P1 | +P2/P3 | +P4 (bar) |
|---|---:|---:|---:|---:|
| `hash` | 10.9× | ~4× | ~1.5–2× | — |
| `smooth` | 262× | <40× | <10× | ≤4× |
| `lock` | 30× | ~28× | ~20× | ≤4× |
| `fill_poly` / `composite` / `wide_line` | 17–26× | ~16–24× | ~12–18× | ≤4× |

If a phase lands and its column does not move as predicted, attribute before
proceeding: `make profile PROFILE_FLAGS=--engine` on the standalone.
