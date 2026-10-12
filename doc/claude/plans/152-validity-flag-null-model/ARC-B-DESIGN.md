<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN152 arc B — how a fit-failure is made visible

The design notes behind [README.md](README.md)'s arc B: why a boolean is structural, how much
of one is needed, and where the difficulty actually sits. Nothing here is built; P2 decides
the spelling and B1/B2 are the two implementations that follow from it. Evidence:
[MEASUREMENTS.md](MEASUREMENTS.md).

## Why arc B needs a bit — and how much of one

**Five of the seven scalar widths cannot encode their own failure** — `u8`/`i8`/`u16`/`i16`/
`u32` answer `0`, where `i32` and `integer` answer `null` because they keep a bottom code back
([MEASUREMENTS.md](MEASUREMENTS.md)). On those five every code is a legitimate datum, so
there is no value that can mean *"this did not happen"* and no cleverness at the collapse
invents one. Arc B needs a channel that is not the value's own bits: the requirement is
structural, not a preference.

**But it is not Phase A's boolean.** That prototype put a bit beside *every eval-stack slot*
and paid for it on every arithmetic op — which is where the +0.5–0.8 % came from, and which
Phase C showed has no native analogue at all (native emits Rust over real locals; there is no
stack to shadow). What arc B needs is far smaller: a bit produced **at the collapse**, for
the five types that cannot represent their own failure, carried only as far as the author's
test.

### The spelling decides how far it must be carried — and that is the design lever

The owner's two sketches differ in cost, and the difference is the whole of P2:

```loft
if !(a = 300) { … }      // the test is AT the store
a: u8 = 300; if !a { … } // the test is AFTER the store
```

- **At the store**, the operands are still in scope, so the fit predicate is an expression
  evaluated where the facts are — **no bit has to persist**. This may be buildable with no IR
  change at all.
- **After the store**, the status must outlive the assignment, and by then the inputs are
  gone and `a` is an ordinary `0`. **That is where a stored bit is unavoidable** — and where
  the open question becomes where it lives: a companion local minted beside the declaration,
  a per-slot bit in the frame, or the slot's type widening to a pair.

So the two spellings are not stylistic alternatives; one may be nearly free and the other
needs new machinery. **P2 decides the spelling knowing that**, rather than picking a surface
and discovering the cost afterwards.

### Selective, not blanket — and the codebase has already made this trade once

The bit is introduced **only in the expressions that need it**: where an author actually
tested a fit status. Everywhere else, no bit, no cost, byte-identical emission. That is the
owner's call, and it is also the only version that works on both backends — Phase A's blanket
bit lived beside every eval-stack slot and native has no stack to put it beside, whereas a
per-VARIABLE marker is something both backends already have.

**`Variable::amp_link` is the precedent, and its rationale is this one:**

> *"A marker rather than `Type::RefVar` on purpose — RefVar would re-route every read and
> write through the double indirection parameters use, slowing every access to carry a
> compile-time fact."*

Same shape: a compile-time fact about ONE variable, carried as a marker on that variable
rather than by changing the representation of every access. `Variable` already holds
`const_binding`, `value_const`, `amp_link`, `uses`, `uses_at_write` — *"is this variable's fit
status observed?"* is another fact of exactly that kind.

**Sketch** (not built — this is a design to probe, not a plan of record):

1. `Variable` gains a marker: this variable's fit status is tested somewhere.
2. **Pass 1 sets it**, when it sees a status test naming that variable.
3. **Pass 2 emits** the status-producing form only for marked variables; the bit lives in a
   companion slot — a frame slot on the interpreter, a Rust local on native. Both are
   per-variable, which is why both backends can carry it.

### Type-conditional emission is routine here — the risk is a DUPLICATE, not the branch

Emitting differently for these types is well-established: **173 sites already ask the
narrow-width question** (`forced_size` / `usable_min` / `usable_max` /
`reserves_sentinel_unconditionally`) across a dozen files. Feasibility is not the concern.

The concern is which home it goes in. `scripts/rule_predicate_audit.py` reports **40 distinct
`Type::` variant lists of 3+**, one scalar list spelled at five separate sites — and the
codegen skill records that the narrow-width family already had five sites spelling one list
where **only three asked the same question**, two of them writing a RAW slot where the others
wrote an encoded field. Merging on the list alone would have folded them wrongly.

**So arc B extends the existing answer rather than spelling a sixth list.** That answer is
`IntegerSpec::reserves_sentinel_unconditionally`, reached through `uncomputable_default` —
which is exactly *"can this type represent its own failure?"*, the predicate arc B needs.
Phase E already proved it is the single home by reading its result off `dflt` instead of
re-deriving it. Before adding any arm: run the audit, and read what each candidate site
ASKS rather than which variants it lists.

### The upside: ordinary arithmetic stays single-variable

This is what bounds the algorithmic impact, and Phase A's own benchmarks are the evidence.

`float`, `single` and `integer` **keep a sentinel**, so their failure is already a value and
they never need a companion bit. They are also what real algorithms are written in. So under
the selective design the hot path of ordinary code carries **no second variable at all** —
the bit exists only for the five widths that cannot represent their own failure, and only at
sites where a status test was written.

**Phase A's +0.5–0.8 % therefore does not transfer.** That number came from a blanket design
that put a bit beside every eval-stack slot, and it was measured on `02_sum_loop` and
`01_fibonacci` — both of which are **plain `integer` and contain no narrow width at all**.
Run against the selective design, those same two benchmarks would carry no bit and show no
change. The measurement did not evaluate this design; it evaluated the one this replaces.

That is the trade the owner is buying: the cost lands on the narrow widths, where the fault
is frequent (a `u8` overflows at 256) and the author has asked to handle it — and stays off
`integer`/`float`/`single`, where the fault is rare, already representable, and the code is
hot. It is the same shape as @C85's proportionality argument, applied to the bit instead of to
the type.

⚠ Unmeasured, and it should be: a narrow-width loop that DOES carry the bit. The claim above
bounds the impact on ordinary arithmetic; it says nothing about the cost where the bit is
live, and B1/B2 owe that number before shipping — with a benchmark that actually contains a
narrow width, which `bench/` currently does not.

### SUPERSEDED — the bit is expression-local (owner ruling, 2026-09-02)

Everything below this line described a per-VARIABLE marker: a flag on `Variable`, set in
pass 1 and emitted in pass 2, with the status carried in a companion slot so it could outlive
the statement that produced it. **That design is withdrawn.** The owner's constraint is that
the bit is never stored anywhere — not in a struct, not in an element, not in a variable slot
— and lives only beside the expression being validated.

What that removes, all of it risk this plan no longer carries:

- no marker on `Variable`, and no propagation rule for `b = a`;
- **no pass-1/pass-2 split**, which was this design's sharpest hazard and a shape this tree
  has been bitten by repeatedly;
- no companion slot on either backend;
- no way for an implementation to drift into widening stored layout, because there is no
  storage in the design at all.

What it costs: `a: u8 = 300; … if !a { … }` cannot work, because between the two statements
only the variable could carry the status and the variable is storage. The test has to be
asked **in the expression** — `if !(x + 10) { … }`.

The density measurement below still stands and is why the constraint is right; the
implementation notes after it are kept only as the record of a rejected design.

---

### The density constraint bounds where the bit may live

Narrow types are a density tool — 1 byte per element against 8, measured at 16× on store
capacity for a 200 000-element vector. A per-element or per-field status bit would give back
half of that, so the bit must never touch stored layout. It lives with the OBSERVATION: one
companion slot per marked variable. See README § Hard constraint.

That also rules out one implementation that would otherwise look natural — widening the
slot's type to a pair — for anything reachable from a container. A marked LOCAL may widen; a
`vector<u8>`'s element may not.

### Where the difficulty actually is

The owner is right that it is harder, and it is worth naming where, because it is not evenly
spread:

- **`if !(a = 300) { … }`** — the demand is visible *before* the store is emitted. No
  backward flow, possibly no marker at all.
- **`a: u8 = 300; … if !a { … }`** — the demand appears *after*. Pass 1 must observe the test
  and pass 2 emit accordingly, which is exactly what the two-pass parser is for and what
  `amp_link` already does.

⚠ **The known hazard is the pass split, and this tree has been bitten by it.** The safe
framing is that the marker records a fact OBSERVED in pass 1 (*"a status test names this
variable"*), never a PREDICTION about what pass 2 will do — pass-stable data, which is what
`amp_link` is. A pass-1 predicate about pass-2 behaviour is a recurring defect shape here, so
the probe for this design should be a variable whose test appears textually after its last
store, checked on both passes.

