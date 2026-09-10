<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 152 — Let the existing spellings reach the types they cannot reach today

## Status — DONE 2026-09-09

Both halves of the ask ship, and neither needed new syntax. `??` **chooses** the value that
lands when a result does not fit a narrow slot, and `!` **sees** that it did not:

```loft
x: u8 = 250;
x = (x + 10) ?? 255;        // 255 — the author's value instead of the type's default

health: u8 = 250;
health += 10;
if !health { … }            // true — the store did not fit, and the slot holds 0 as before
```

**Where the reference content lives now** — this file keeps only the closure record:

| what | home |
|---|---|
| the rule | [`formal/operational.md`](../../formal/operational.md) `(E-Uncomp-Seen)`, beside `(E-Uncomp-NN)` and `(E-Report)` |
| the surface a user reads | [LOFT.md](../../LOFT.md) § narrow widths |
| the adjacency trade-off | [CAVEATS.md](../../CAVEATS.md) § Accepted trade-offs |
| what is and is not fused, in full | [`src/parser/fit.rs`](../../../../src/parser/fit.rs) — the mechanism's one home |
| the lint | [DIAGNOSTICS.md](../../DIAGNOSTICS.md) `redundant-null-negation` |
| what shipped, technically | [CHANGELOG_TECHNICAL.md](../../CHANGELOG_TECHNICAL.md) |
| every number this plan measured | [MEASUREMENTS.md](MEASUREMENTS.md) |
| the design that was withdrawn | [ARC-B-DESIGN.md](ARC-B-DESIGN.md) |

Guards: `tests/scripts/152-a-store-that-does-not-fit-is-testable-where-it-happened.loft`
(falsified at `e6922e9fa`, both backends) ·
`152-a-coalesce-chooses-the-fallback-for-a-narrow-slot.loft` ·
`152-the-fallback-reaches-every-position-a-narrow-store-takes.loft` ·
`152-the-fallback-and-failure-spellings-that-already-work.loft` (a LOCK, inert by design) ·
`152-the-narrowing-message-names-cures-that-work.loft`.
Shipped along the way: loft#1305, loft#1306.
Tracker: [@PLN152](https://github.com/loft-lang/plans/issues/152).

## What it was for

`u8`, `i8`, `u16`, `i16`, `u32` and every `integer limit(lo, hi)` use every code they have.
So a value that does not fit takes the type's **default**, and nothing in the program can
tell that answer from a computed one:

```loft
x: u8 = 250;  x += 10;   // x = 0 — and !x is false, x == null is false, x == 0 is true,
y: u8 = 200;  y += 10;   // y = 210      exactly as they are for a real, computed 0
```

`integer` and `i32` keep a bottom code, so their failure IS a null and `!` has always read
it. The five widths that fill their own range had nowhere for a failure to live. The plan's
one test was: **can an author who cares write a correct handler, without learning anything
new?**

## The steps, and what each one settled

| # | Step | Outcome |
|---|---|---|
| **0** | Where must the opt-in gate live? | The store guard — `guard_declared_range` / `guard_compound_range` already receive the whole stored expression, so no parse-time flag was needed. Answered by inspection. |
| **1** | Pin the three spellings that ship today and are guarded by nothing | A LOCK, inert against any past build by design. |
| **2** | `??` reaches a non-null narrow | Guard INSIDE the discharge, sentinel as its default. Falsified at `4f229521`. |
| **3** | `??` at field, element, argument, return, struct literal | Came free with step 2 — wiring both seams covered all five. |
| **4** | ~~`!` in-expression~~ | **RETIRED.** `x + 10` widens to `integer` and `260` is a good one, so a free expression has no narrow type and no failure to read. The failure is a property of a STORE. Folded into step 5. |
| **5** | `!` in the adjacent form | Shipped. `src/parser/fit.rs`; rule `@FR-E-Uncomp-Seen`. |
| **5a** | The lint step 5 leans on had holes | `redundant-null-negation` asked `IntegerSpec::not_null`, a flag only a FIELD declaration sets, so two of the three spellings of one type said nothing. Now `non_null_reads_null`. |
| **6** | Cost where it is live | ~2× on a deliberately minimal interpreter loop; no measurable cost on `--native`. A report, never a gate. |
| **7** | Docs and diagnostics agree with what shipped | The narrowing message's `?? d` cure works in the position it is offered (CONTROL.md N1/N2). |

## What this plan got wrong, four times — the part worth keeping

Every redirection came from a measurement, and each one was cheaper than the work it
cancelled. The numbers are in [MEASUREMENTS.md](MEASUREMENTS.md); the shape of the mistakes
is here because it repeats.

1. **A validity BIT beside every value** (phases A–C). Measured at under 1 % on the
   interpreter — and then Phase C found native has no eval stack to put it beside, so the two
   backends would have needed two mechanisms. The collapse site already existed on both and
   was missing one input.
2. **That one input was already there** (phase E). `uncomputable_default` sets `dflt` to the
   sentinel exactly when the slot can read back null, so the guard could read the target's
   nullability off an argument it already received. Two shipped defects closed at one line.
3. **The premise itself was false** (the closing measurement). `fit = (x + 10) as u8?;
   x = fit ?? 0; if !fit` already worked, on a build predating every line of this plan. Every
   probe had tested the spellings the design proposed; none had asked whether the existing
   surface could already say it, and that question was one build away the whole time.
4. **…and the shape that answered it was itself wrong** (step 5). The hand-written "proven"
   target reports `255` — an ordinary `u8` — as a failure, because an inferred `fit` takes
   the checked cast's `u8?` and a `u8?` gives its top code up to hold null. It had been
   checked at `300` and at `200`, which straddle the range boundary without touching it.

**Finding 4 is what makes finding 3 only half right.** The fused form removes one temp from a
working idiom — ergonomics — but the temp it removes is the one an author gets wrong, and the
shipped mechanism binds a full-width `integer?` so the trap is not reachable from the surface
at all. That is a correctness argument.

**The method lesson, twice over:** a probe that varies one axis with two values has not
tested the axis, it has tested two points on it — and `scripts/matrix_axes.py` names the
range BOUNDARY as an axis for exactly this reason. A shape called *proven* was proven on the
cells someone chose.

## What bounded the design

- **Nothing is stored.** Not in a struct, not in an element, not in a variable's slot. Narrow
  types exist for density — a `vector<u8>` of 200 000 elements is **0.362 MB** against
  `vector<integer>`'s **5.909 MB**, and a status bit beside every element hands half of that
  back. The status lives in a temp across exactly the pair of statements the author wrote,
  which is what makes the adjacency rule a consequence rather than a preference.
- **Opt-in, proved rather than argued.** `scripts/introspect_diff.sh` over the corpus reads
  **DIFFERENT 2 of 1437** — both are this plan's own guard files. 1435 files emit
  byte-identically, diagnostics included.
- **The blast radius was measured before the work.** Narrow widths are barely used for
  arithmetic: 2 files in `tools/`, 0 in `lib/`. That made step 6 a bound rather than a gate,
  and it is the falsifier the plan leaves behind — if narrow-width arithmetic still does not
  appear now that it can be written safely, the types were niche and this reading was wrong.

## See also

- [MEASUREMENTS.md](MEASUREMENTS.md) — every measurement, including the three that redirected
  the plan and the one that reopened it.
- [ARC-B-DESIGN.md](ARC-B-DESIGN.md) — the per-variable marker design, withdrawn by the
  owner's expression-local constraint. Kept as the record of a rejected shape.
- [`formal/types.md`](../../formal/types.md) § Null-flow laws ·
  [`formal/operational.md`](../../formal/operational.md) `(E-Uncomp)` / `(E-Uncomp-NN)` /
  `(E-Uncomp-Seen)` / `(E-Report)` ·
  [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md) C79, C80, C85, C90.
- [CONTROL.md](../../CONTROL.md) item 1 — the census row this closed one half of.
