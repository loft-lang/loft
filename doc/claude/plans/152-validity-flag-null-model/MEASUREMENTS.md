<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN152 measurements — phases A and B

The numbers behind [README.md](README.md)'s status. Probes: [`probes/`](probes/).

## Phase A result — measured 2026-09-02

**Verdict: the clean design survives.** The gate was *"under a few percent, build it;
around ten, fall back to computing the flag only where an op can be invalid."*

| bench | flag off | flag on | 2× traffic | flag cost | 2× cost |
|---|---|---|---|---|---|
| `02_sum_loop` (treatment — `AddInt` is the hot op) | 1383 ms | 1390 ms | 1405 ms | **+0.51 %** | +1.59 % |
| `01_fibonacci` (treatment — `MinInt`/`AddInt` + calls) | 12581 ms | 12678 ms | 12846 ms | **+0.77 %** | +2.11 % |
| `08_word_count` (**control** — text scanning) | 332 ms | 332 ms | 337 ms | **0.00 %** | +1.51 % |

Medians, `--interpret`, release build, 15 interleaved reps (5 for fibonacci). That is
**0.35 ns/op ≈ 1.1 cycles at 3 GHz** over `02_sum_loop`'s 20 M `AddInt` — physically
credible for three L1-resident, pipelined byte accesses, which is the check that the
number is a measurement rather than an artefact.

**The mechanism is confirmed in the disassembly, not just the timing.** In the flag arm
the operand sentinel tests are *gone* — baseline's `cmp $0x8000…,%rdx; je` (with `v1`'s
test folded into `neg; jo`) is replaced by `add (%rax),%rbx; seto %al`. The plan's claim
that *"the flag is a bit the CPU already sets"* is `seto`, at the instruction level.

**The method matters more than the number, and phases C/D inherit it.** The first three
attempts all failed, each for a different reason worth recording:

1. **A shared box.** Another checkout ran `rustc` + six `mold` linkers mid-run; load hit
   56 and `08_word_count` read 1017, 602, 457, 361, 340, 332, 310 across seven reps — a
   monotone settle, not a measurement.
2. **Separate binaries are not comparable at this effect size.** A build whose only change
   was *adding the shadow field* — ops untouched, the new code dead — moved every
   benchmark by **+2 to +4 %**, treatment and control alike. Later, a flag build made the
   text control **2.6 % faster**, which no change to integer arithmetic can do. Adding
   24 bytes to `State` shifts every field offset and the whole code layout, and that
   swamps what is being measured.
3. **So the measurement must be ONE binary with a runtime-selected path** — identical
   struct, identical layout, only the executed instructions differ. That is where the
   numbers above come from, and it is how C and D must compare before/after too.

The instrument was checked in both directions rather than trusted: the **control must not
move** (0.00 %) and a **double-traffic mode must** (+1.5 to +2.1 %). Both held. The
double-traffic arm scaled 2.7–3.1× rather than a clean 2×, and the control moved 1.5 % in
that arm, which bounds the residual noise at ~1.5 % — so the flag cost is quoted as
*under 1 %*, not to two decimals.

**What this does NOT measure — the honest residual.** The probe carried the flag through
`add` / `sub` / `mul` on integers. A full implementation also needs every *value-producing*
op to write its slot's flag (one store), because the shadow is indexed by stack position
and would otherwise read a stale entry. Scaling from the measured per-access cost, that is
roughly another **0.6–1 %**, putting a complete implementation near **1.5–2 %** — still
well inside the gate, but it is an estimate, and Phase C's parallel run is where it becomes
a measurement.

Probe artefacts are throwaway and were reverted; `src/fill.rs` is generated, so the real
change belongs in the `#rust` templates and the generator, not in that file.

## Phase B result — measured 2026-09-02

Three instruments, each with its own control, in [`probes/`](probes/). The value matrix
runs under `scripts/probe-matrix`, which enforces hand-written `@EXPECT`s, rejects vacuous
cells, and requires a control that FAILS.

### What the matrix says

**19 value cells, both backends, all green; control red; no vacuous cells.** Expectations
were hand-derived from `formal/types.md`'s spare-code table *before* the first run.

- **The value collapse is already uniform across every boundary.** Nine cells put the same
  fault into a local, a struct field, a vector element, a call argument, a return value and
  a tuple element — all answer identically. So `(E-Collapse)` **describes** today's value
  behaviour rather than changing it, which makes Phase D a genuine no-op and its
  identical-output gate both cheap and meaningful. That was the plan's biggest unknown.
- **The divergence is entirely along the TYPE axis**, not the boundary axis: `integer` and
  `i32` answer `null`; `u8`/`i8`/`u16`/`i16` (range fills the width) and `u32` (spare code
  at the top, which no non-null read tests) answer the type's default.

### The residual has exactly ONE door

A five-cell refusal probe, with a control that must compile:

| fault reaching a non-null `u8` | outcome |
|---|---|
| `/` by zero | **refused** — *cannot cast a possibly-null `integer?` to the non-null `u8`* |
| out-of-range index | **refused** — same |
| failed text parse | **refused** — `error[text-parse-may-fail]` |
| out-of-range narrowing cast | **refused** — *use `u8?` for a checked cast* |
| **overflow** (the control) | **compiles, answers `0`** |

Every fault that types `τ?` is stopped by `(N-Store)`/`(N-Cast)` before it can reach a slot
that cannot represent its answer. **Overflow is the only one that gets through, and it gets
through precisely because C85 types it non-null.** So C85's exception is not merely an
inconsistency in the rule set — it is the single reason the unrepresentable case is
reachable at all. That is the sharpest argument for this plan, and it was not known before
this phase.

### The diagnostic channel is NOT uniform

Nine cells, scored by severity and code (a coarse "any diagnostic fired" test scored a
`dead-assignment` as a pass and hid both findings below):

- `(N-Store)` warns at the **field, element, call-argument, return, tuple and keyed**
  boundaries — uniformly.
- A **declared local is a hard ERROR**, not a warning (`(N-Decl)`'s commitment). So the
  boundary set is uniform in VALUE and split in SEVERITY, and `(E-Collapse)` must say so
  rather than implying one answer.
- The **narrow-width overflow collapse is silent** — the case row 3 of the table proposes
  to report.

### The correction this phase forces

**All eight issues this plan cited are `fixed-pending-merge` on this branch**, and loft#1296's
fix (`c75dce65`, the same morning) is what my expectations were derived from: it made `i32`
answer `null` and *added the spare-code table to the rules*. So:

- the plan's four-answer table is post-fix and accurate, but
- the **residual is narrower than the issue body claims**: `u8`/`i8`/`u16`/`i16`/`u32`, and it
  is a documented representation limit rather than an open defect. This plan does not "fix
  loft#1296" — it makes the limit **speak**;
- loft#1284 (the tuple `(N-Store)` gap) is fixed here, which my `d06` cell independently
  re-verified.

**So the bug-fixing case for this plan is largely already banked by other work.** What
survives untouched is the structural case: three encodings of one question, C85's exception
as the single reachability door, the 14 `Op*Nullable` duplicates, the `_nn` lattice this
makes unnecessary, the silent narrow-width default, and provenance. That is still worth the
work — but it is a *consolidation* argument now, not a defect-count argument, and the issue
body should be corrected to say so.

### Not graduated yet, deliberately

The boundary cells pin behaviour that was never broken, so `make falsify` would report them
INERT — there is no commit they would fail on. They are the **before-half of Phase D's
parallel run**, not a regression guard, so they graduate to `tests/scripts/` in D with a real
falsification answer, rather than landing now as an inert guard.


## Phase C result — measured 2026-09-02

**Phase C as cut is the wrong phase, and running it is what showed that.** The finding is
better than the phase: the collapse site already exists on both backends, and it carries
exactly the defect the proposed rule predicts.

### Native has no eval stack, so the flag has no native analogue

`loft introspect` on a minimal add:

```rust
fn n_add2(cell: …, mut var_a: i64, mut var_b: i64) -> i64 {
  return ops::op_add_int((var_a), (var_b))
}
```

Direct Rust over real locals. Phase A's shadow-array-indexed-by-`stack_pos` has nothing to
attach to here — native would have to thread `(value, ok)` pairs through generated
expressions, and the `#rust` template is an *expression* pasted into arbitrary positions, so
a template returning a pair cannot be substituted where a scalar is expected. **The two
backends need two different mechanisms**, which the phase's one-line description hid.

### But the collapse site already exists — and already reports

A narrow-width store lowers to `OpRangeDefault` on BOTH backends, from one `#rust` template
(`default/01_code.loft:158`):

```rust
{ let _rv = @val;
  if _rv == i64::MIN || (_rv >= @lo && _rv <= @hi) { _rv }
  else { s.raise_recoverable(RangeDefaulted { value: _rv, lo: @lo, hi: @hi }); @dflt } }
```

That is `(E-Collapse)` row 3, already built, already reporting — and the message is the one
the plan wanted: *"value 260 is outside the declared range 0..=255, so the slot took its
default instead"*. It is **opt-in behind `--dev-soft-halt`, not absent.** So row 3 is a
policy question (should this raise be default-on?), not an implementation phase.

### And it has exactly the defect the rule predicts

`_rv == i64::MIN` exempts the sentinel **unconditionally**. Whether the sentinel is a legal
answer depends on the TARGET's nullability — `u8?` must keep it, `u8` cannot hold it — and
`OpRangeDefault` is never told which. **The collapse site is missing precisely the input
`(E-Collapse)` branches on.** That is this plan's thesis, confirmed at one line rather than
argued.

### Two defects, from an axis Phase B pinned

Phase B's cells all overflowed to `260` — merely out of range. None landed *exactly on the
sentinel*, which is a different path through the guard. Crossing overflow-shape × storage-kind:

| | local | field | element | return |
|---|---|---|---|---|
| out of range (`+= 10` → 260) | 0 | 0 | 0 | 0 |
| **on the sentinel** (`+= i64::MAX`) | **null** | 0 | 0 | **interpret null / native 0** |

1. **A non-null `u8` LOCAL reads back `null`.** A narrow local is an `i64` until it is
   materialised (`let mut var_x: i64` in the emitted Rust), the guard exempts the sentinel,
   so it survives in the slot. Field, element and return materialise and narrow it to 0. The
   same declared type answers `0` for one overflow and `null` for another, decided by whether
   the result happens to land on the sentinel.
2. **The return boundary DIVERGES between backends.** The interpreter returns
   `integer(0,255)` as an 8-byte slot (`Return(value=8)`) and the sentinel survives; native
   declares `-> u8` and emits `(var_x) as u8`, truncating it to `0`. Same IR, two lowerings —
   the divergence class the codegen rule says is itself the bug.

### What this does to the plan

- **C is retired as cut.** The flag is not what the narrow-width case needs; the collapse
  site needs ONE INPUT (the target's nullability). That is a far smaller change than
  threading a pair through two backends.
- **E moves up and shrinks**: give `OpRangeDefault` the target's nullability, and decide
  whether `RangeDefaulted` is default-on. Both defects above close with that one input.
- **What still argues for the flag** is unchanged and unproven-against: removing the operand
  sentinel tests from the hot path (measured in A), collapsing the 14 `Op*Nullable`
  duplicates, and provenance. None of those is a correctness argument any more.

Both reproduce on `main` (e9643ff6), verified against a worktree build — so they are
mainline defects, not this branch's, and both are filed with a verified workaround:
**loft#1305** (the sentinel exemption) and **loft#1306** (the return-width divergence).

Probes: [`probes/axis/`](probes/axis/).

## Phase E result — the fix, 2026-09-02

**Both defects closed at one site, with no new argument.** Phase C found the collapse site
(`OpRangeDefault`) missing the input `(E-Collapse)` branches on. It turned out the input was
already there: `parser/expressions.rs::uncomputable_default` sets `dflt` to `i64::MIN` exactly
when the slot can read back null, so the guard reads the target's nullability off an argument
it already receives.

```rust
// before — the sentinel is exempted whatever the target is
if _rv == i64::MIN || (_rv >= @lo && _rv <= @hi) { _rv } else { …raise…; @dflt }

// after — pass it through only where null is representable
if _rv == i64::MIN { if @dflt == i64::MIN { _rv } else { @dflt } }
else if _rv >= @lo && _rv <= @hi { _rv }
else { …raise…; @dflt }
```

One template, both backends, because `src/fill.rs` is generated from it and the native
emitter substitutes the same body.

### Measured against a prediction written first

| cell | before | after | predicted |
|---|---|---|---|
| `sent_local` (non-null `u8`) | `null` | **`0`** | changes ✓ |
| `sent_ret` | interpret `null` / native `0` | **`0` on both** | changes ✓ |
| `sent_field`, `sent_elem` | `0` | `0` | unchanged ✓ |
| `oor_*` (260) | `0` | `0` | unchanged ✓ |
| `a02` `i32`, `a08`–`a10` nullable | `null` | `null` | unchanged ✓ |
| refusal channel (5 cells) | — | — | unchanged ✓ |
| diagnostic channel (9 cells) | — | — | unchanged ✓ |

All eight axis cells now AGREE across backends, where two did not.

The controls are what make that a result rather than a coincidence: a fix that collapsed
*every* sentinel would satisfy both changed cells. `i32` and the nullable spellings must
still answer `null`, and they do — which is why the guard reads `dflt` and not the width.

The sentinel arm is silent by design. The overflow that produced it reported at its own site;
reporting again would name `-9223372036854775808` as out of range, a value the program never
computed. The merely-out-of-range arm still reports, pinned by the guard's `c_` cell.

Guard: `tests/scripts/1305-a-sentinel-collapses-where-null-is-not-representable.loft`,
falsified at `214f88b6` — exit 1 → 0 and one assertion failure → 0 on **both** backends.

### What is left of this plan

The flag is not needed for correctness anywhere the three phases looked. What remains is the
consolidation case — the hot-path sentinel tests (A measured the saving), the 14
`Op*Nullable` duplicates, the `_nn` lattice this would make unnecessary, and provenance —
and that is a value judgement about scope, not a defect list. **F and G should not start
before that call is made.**

## The fallback surface, measured 2026-09-02 (the rewrite's basis)

The owner's ask — *"let users use the `?? 1.0` notation for the overflow case, opt-in"* —
turned out to be half-shipped, and the missing half fails for a reason worth stating exactly.

**On a type that keeps a sentinel, `??` already supplies the author's fallback**, at runtime,
with no static knowledge:

```loft
fn bump(v: integer, d: integer) -> integer { (v + d) ?? 42 }
fn plain(v: integer, d: integer) -> integer { v + d }
```

| call | answer |
|---|---|
| `bump(i64::MAX, 1)` | **42** — the author's fallback on a real overflow |
| `plain(i64::MAX, 1)` | `null` — C85's default |
| `bump(2, 3)` | `5` — the ordinary path is untouched |

No `redundant-coalesce`, no static range knowledge needed. So for `integer` and `i32` the
feature exists and is pinned by nothing.

**The narrow widths do not fail because the `??` is missing — their fault is not a null.**

| fault | null? | can a coalesce see it? |
|---|---|---|
| `x: integer` overflow → the sentinel | yes | yes — works today |
| `x: u8 = 250; x += 10` → `260` | **no**, an ordinary number | **no** |

`260` never becomes null, so no `??` can fire on it; the only thing that handles it is
`OpRangeDefault`'s `dflt`, which is always compiler-chosen. That is the gap — a fallback the
author cannot reach, not an operator that is absent.

And the diagnostic already promises the cure. The narrowing refusal reads *"guard the value
(`?? d`, mask, or an `if` range check)"*, while `a = (a + 10) ?? 255` into a `u8` is refused:
`a + 10` has range `[10, 265]` and the `??` clamps nothing, because there is no null to
substitute for. **The message advertises a cure that does not work in the position it is
offered.** One of the two has to move, which is an open question of the rewritten plan.

## What a failing narrow store can be TESTED for, measured 2026-09-02

The owner's second ask — *"it should be possible to test if that fails and do something
useful with the result … I think we currently drop that one on the floor"* — is right for one
of the three shapes and already satisfied for the other two.

```loft
x: u8  = 250;  x += 10;   // x=0     !x=false   x==null false
y: u8  = 200;  y += 10;   // y=210   !y=false
z: u8? = 250;  z += 10;   // z=null  !z=true
```

| shape | today | testable? |
|---|---|---|
| `a: u8 = 300` — a literal that cannot fit | **compile error** (*"cannot implicitly narrow integer to u8"*) | n/a — caught, not dropped |
| `x: u8` — a RUNTIME fit-failure | `0`, and every test an author can write says *ordinary zero*: `!x` is false, `x == null` is false, `x == 0` is true exactly as it is for a real `0` | **no — dropped on the floor** |
| `z: u8?` | `null` | **yes — `if !z` already works** |

So the owner's `if !a { … }` sketch is already the right spelling and already works; what it
cannot reach is the NON-NULL narrow slot, which by construction has no code for null to
occupy. `x=0` and `y=210` differ only in that one of them is a fabricated answer, and nothing
in the program can tell.

**This is the same root as the `??` gap and should share its cure.** A fallback the author
chooses (`?? 255`) removes the surprise by making the substituted value deliberate; a
testable outcome removes it by making the substitution visible. They are two answers to
*"the language picked a value and did not say so"*, and a design that gives one should say
why it does not give the other.

Note the literal case is refused with a NARROWING message (*"cannot implicitly narrow"*)
rather than a doesn't-fit one, though `cast-constant-out-of-range` exists for exactly *"a
constant does not fit the type it is bare-cast to"*. Worth checking whether the constant
initialiser should route to that code instead — a smaller, separate question.

## Which widths can encode their own failure, measured 2026-09-02

```loft
a: u8  = 250;        a += 10;   b: i8  = 120;        b += 10;
c: u16 = 65530;      c += 10;   d: i16 = 32760;      d += 10;
e: u32 = 4294967290; e += 10;   f: i32 = 2147483640; f += 10;
g: integer = 9223372036854775807; g += 1;
```

| | `u8` | `i8` | `u16` | `i16` | `u32` | `i32` | `integer` |
|---|---|---|---|---|---|---|---|
| overflow answers | `0` | `0` | `0` | `0` | `0` | `null` | `null` |

Five of seven fill their width, so every code is a legitimate datum and the failure has
nowhere to live. `i32` and `integer` keep a bottom code back, which is why `if !x` already
reads a failure on those two and cannot on the others. This is what makes arc B's boolean
structural rather than a preference — and it is also the bound on it: the bit is needed for
five types, not for the type system.

## A free expression cannot fail — measured 2026-09-02 (step 4 retired)

Step 4 was cut as *"`!` in-expression: `if !(x + 10) { … }` on a non-null narrow"*. It is not
implementable, and the reason is worth keeping because it also explains why `??` works.

```loft
x: u8 = 250;
y = x + 10;      // y = 260
```

`x + 10` **widens to `integer`**, and `260` is a perfectly good one. Nothing failed. There is
no narrow type on the expression, so there is no range for `!` to test against and no failure
for it to read.

**The failure is a property of a STORE into a narrow slot, never of a free expression.** That
is exactly why step 2 works: `x = (x + 10) ?? 255` has a target, and the target supplies the
`lo`/`hi` the guard needs. Strip the target and the whole question dissolves.

Consequences:

- **Step 4 is retired**, and folded into step 5. `!` can only read a failure where an
  assignment names the slot — which is the adjacent form.
- The owner's `if !(a = 300)` sketch would have worked for the same reason step 5 does (the
  assignment supplies the target), but it is new syntax and was ruled out on that ground.
- It sharpens what the adjacent form IS: not a convenience over the in-expression spelling,
  but the only spelling in which the question can be asked at all without new syntax.

## The closing measurement — the capability was never missing (2026-09-02)

Run on the **pre-arc-A build** (`4f229521`), before any of this plan's code:

```loft
x: u8 = 250;
x = ((x + 10) as u8?) ?? 255;        // -> 255   choose the fallback
f.i = 200;
fit = (f.i + 100) as u8?;
f.i = fit ?? 0;
if !fit { … }                        // -> detects, f.i = 0   see the failure
```

Both halves — choose the value AND know it happened — **already worked**, using `as τ?` (the
DN4 checked cast, shipped 2026-07-02) with `??` and `!`. This plan opened on the premise that
an author *cannot write correct code* for the fit-failure edge. **That premise was false**, and
it stayed unexamined for most of the plan's life because every probe tested the spellings the
plan proposed rather than the ones that already existed.

What was genuinely wrong is narrower and still worth the fix that shipped:

- the narrowing refusal **advertised `?? d` as its cure** and `?? d` did not work in the
  position it was offered — a diagnostic promising something untrue, which arc A made true;
- the natural spelling `(x + 10) ?? 255` required an explicit `as u8?` that the author had no
  reason to expect.

So arc A closed a message/behaviour mismatch and removed a ceremony step. Arc B would have
removed one more (the explicit `fit` temp) from a form that already works. That is ergonomics
on a working capability, not a missing one — which is why the plan closes here rather than
carrying arc B as an obligation.

**The lesson worth keeping is the method one.** Every probe in this plan tested what the design
proposed. None tested whether the existing surface could already express the case, and that
question was one build away the entire time — the same cached falsify worktree that answered it
in the end had been sitting there since step 2.

## Step 5 — the adjacent form, measured 2026-09-09

### The hand-written target shape was wrong at 255, and two cells could not see it

The shape [`probes/step5-target-shape.loft`](probes/step5-target-shape.loft) had recorded as
*proven, so the peephole is translation not invention* was run at the top of the range before
any compiler edit:

```loft
a: u8 = 250;
fit = (a + 5) as u8?;   // 255 — an ordinary u8
a = fit ?? 0;
```

| `a` before | `a += 5` today | the hand-written shape | `!fit` |
|---|---|---|---|
| 0, 100, 245, 246 | 5, 105, 250, 251 | same | false |
| **250** | **255** | **0** | **TRUE** |

A `u8?` gives its TOP code up to hold null (`@FR-N-Reserve`), so the inferred `fit` cannot
hold `255` and reports it as a failure. The probe's two cells — 300 (does not fit) and 200
(fits, mid-range) — straddle the boundary without touching it, which is exactly the
counting failure `scripts/matrix_axes.py` exists for: one axis varied, its BOUNDARY not
among the values. Declaring the temp `integer?` answers correctly at every value 0..=255 on
both backends, and is what shipped.

### The lint the design leans on had holes in two of its three spellings

README § *The adjacency rule polices itself, with a lint that already ships* rested on
`redundant-null-negation` reporting a `!` that is always false. Measured on the three
spellings of one type:

| `if !` on a non-null `u8` | before | after |
|---|---|---|
| a struct FIELD — `!f.i` | warns | warns |
| a LOCAL — `!a` | **silent** | warns |
| a vector ELEMENT — `!v[0]` | **silent** | warns |

The predicate was `spec.not_null`, a flag only a FIELD declaration sets; the local and the
element carry the same `u8` with the flag clear. So two thirds of the guard rail the design
depends on did not exist, and `a: u8 = 250; if !a { … }` was dead code with nothing said
about it. The question is now `IntegerSpec::non_null_reads_null` — *can a non-nullable slot
of this spec read back as null?* — which answers `true` for the two templates that keep a
bottom code (`integer`, `i32`) and `false` for every range that fills its own width.

Blast radius of widening it, over `tests/scripts` + `tests/docs` + `examples` + `tools` +
`lib` + `default`: **one** site draws the new warning, and it is the deliberate one in this
plan's own step-1 guard.

### The value matrix — both backends, hand-computed first

`probes/` plus the graduated guard. Every cell asserts the VALUE the slot holds as well as
what the test answered, because the mechanism's whole claim is that it changes only the
second.

| axis | cells | result |
|---|---|---|
| width | `u8` `i8` `u16` `i16` `u32` `limit(0,100)` | all report; all store the same default as before |
| fits | `u8` at 255, `i8` at 127, an ordinary 110 | none reports; values unchanged |
| fault shape | out of range high, below the low bound, landing exactly on the sentinel | all three report |
| place | local · field · nested field · element (const index) · element (variable index) | all report |
| condition | `!a` · `!a and c` · `c and !a` · `!a or !a` | all read the fit; the last mints one temp |
| controls | `i32` `integer` `u8?` | unchanged — they keep their own code and `!` reads it |
| adjacency | one statement apart · in the `if` body · in a `while` · in an `else if` arm | none fuses, all four warn |
| purity | `w[bump()] += 10; if !w[bump()]` | does not fuse — the read is not a fetch |
| nesting | a fused pair inside an `if` body, inside a `for` body, inside a lambda | all fuse, per iteration |

Both backends agree on every cell. `LOFT_STORES=warn` and `LOFT_NATIVE_LEAK_CHECK=1` are
clean.

The strongest value check is not in the matrix: `probes/step6-cost/`'s two arms differ only
by the `if !acc { … }` line and run 20 000 000 iterations with 271 739 fit failures, and
they print the same accumulated result.

### The opt-in claim, proven rather than argued

`scripts/introspect_diff.sh <pre-change> <post-change>` over the whole corpus (IR + bytecode
+ generated Rust + stderr, per file):

```
DIFF tests/scripts/152-a-store-that-does-not-fit-is-testable-where-it-happened.loft
DIFF tests/scripts/152-the-fallback-and-failure-spellings-that-already-work.loft
DIFFERENT 2 of 1437
```

Both are this plan's own guards — one new, one rewritten by step 5. **1435 corpus files
emit byte-identically**, diagnostics included.

### The reporting channel: a fused store is a defended one

```
$ loft --interpret --dev-soft-halt report.loft
fused: detected a=0
soft-halt: value 260 is outside the declared range 0..=255, so the slot took its default instead
unfused b=0
```

The undefended store still announces its fault; the fused one does not, because the author
said what happens — `(E-Report)`'s guarded-site rule, the same one the `??` path follows
(`152-the-narrowing-message-names-cures-that-work.loft` § `d_a_defended_store_reports_nothing`).

### Step 6 — what it costs where it is live

A REPORT, never a gate. `probes/step6-cost/run.sh`, three arms interleaved, medians of nine
reps. Two of the arms compare two BINARIES on one program and must read 0 % — their emission
is byte-identical — so whatever they show is the box, and that is the noise bound. The
`mechanism` row compares two PROGRAMS on ONE binary, which is the cost itself.

| run | narrow-plain (noise) | integer-ctrl (noise) | **mechanism** |
|---|---|---|---|
| 1 | +1.1 % | +3.9 % | **+85 %** |
| 2 | −3.6 % | −23.6 % | **+110 %** |
| 3 | −35.9 % | +18.9 % | **+97 %** |

**The interpreter arm roughly DOUBLES**, and the box it was measured on was running two
other checkouts' gates throughout (load 9–40), which is what the wandering controls say. The
effect is far outside that noise and it is also predictable from op count: the loop body is
three source operations, the fused form adds a temp bind, two `OpLeInt` comparisons and the
`!` test, and every one of those is an interpreter dispatch. **This is close to the worst
case by construction** — a body that did any real work would divide the same fixed addition
by a larger number.

**On `--native` no slowdown was measurable at all.** Medians over three reps, and a
`nofault` twin of each arm (the accumulator is reset every iteration, so no store ever fails
and no `RangeDefaulted` raise fires in either arm) to separate the checks from the
reporting:

| | plain | tested |
|---|---|---|
| native, faults present | 335 ms | 217 ms |
| native, no faults | 620 ms | 449 ms |
| interpret, no faults | 4360 ms | 7294 ms |

The tested arm ran at or below the untested one on every native rep, in both the faulting
and the non-faulting twin. Two integer comparisons are something rustc schedules; the
interpreter has to dispatch them. **The honest statement is therefore backend-shaped**: the
mechanism is a real per-iteration cost on the interpreter, near-free on the backend that is
the default, and paid only where an author wrote the test — the corpus diff above says
1435 of 1437 files pay nothing at all.
