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
(@P378(a)), and which exactly one user call receives.  A buffer reached any
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

*Measured* (idle box, n=7 medians, `--native-emit` + `rustc -O`, hashes exact):

| form | `lock` ns/op | `hash` ns/op |
|---|---:|---:|
| both switches off (P4d) | 47.8M | 2.23M |
| callee half alone | 48.6M (a wash, as predicted) | — |
| both halves, gate on | **35.3M (−26 %)** | 1.88M (−16 %) |
| both halves, gate off (the ceiling) | 32.2M (−33 %) | — |

The 7 pts between the gated and ungated forms are the sites the gate
declines; `paired_witness` sites where the result OUTLIVES the buffer are also
safe and are the first widening to measure.  P0 instrument on the idle box:
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
