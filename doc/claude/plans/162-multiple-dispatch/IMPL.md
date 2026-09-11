<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 162 — implementation design, in small safe steps

Companion to [README.md](README.md) (the plan) and [DESIGN.md](DESIGN.md) (the owner's
proposal, verbatim).  **This file is written against the tree, measured 2026-09-11** — every
fact below has a file and line, and where it contradicts DESIGN.md it says so.

## What the tree already does — measured, not read

The single most important finding, and it changes the shape of the whole plan:

> **Single dispatch already works.  It is keyed on the FIRST parameter only.**

```loft
fn hit(self: Fire) -> text { return "fire"; }
fn hit(self: Ice)  -> text { return "ice"; }        // coexists — no redefine error
f.hit()    // "fire"     — method spelling
hit(f)     // "fire"     — bare spelling resolves too, by argument type
```

Add a second parameter and the two definitions collide:

```loft
fn hit(self: Fire, t: Wall)  -> text { … }
fn hit(self: Fire, t: Crate) -> text { … }
// error: cannot redefine method `hit` on `Fire`
```

So the feature is **widening an existing key**, not building a dispatcher.  That is a much
smaller and safer change than DESIGN.md assumes, and it is why the steps below are as small
as they are.

### The key, and its two homes

| Side | Site | Key today |
|---|---|---|
| **Write** (a definition registers) | `Data::get_fn(fn_name, arguments)` — `src/data.rs:7567` | `t_<LEN><T>_<name>` from `arguments[0]` when it is `self`/`both`; otherwise `n_<name>` |
| **Read** (a call resolves) | `Data::find_fn(source, fn_name, tp)` — `src/data.rs:7624` | the same, from the receiver's type; falls back to `n_<name>` |

Two helpers build the key and are the places a widening lands:
`key_type_name(type_nr)` (the base name) and `sig_type_name(base, typedef)`
(`src/data.rs:7558` — appends `?` for a nullable receiver, and nothing else).

**There is already a precedent for a key carrying more than a type.**
`Data::bound_stub_name(holder, method, arity)` (`src/data.rs:7197`) builds
`t_<LEN><holder>#g<arity>_<method>` — a marker plus the arity folded into the name.  The
widening below follows that shape rather than inventing one.

### Two DESIGN.md claims that do not hold

1. **INCONSISTENCY #6 does not exist**, and a plain enum already takes a method.  Recorded in
   [README.md § Status](README.md#status); struck as a motivation.
2. **Untyped parameters do not exist.**  `fn twice(a) -> integer` is refused —
   *"Expecting a clear type, found unknown"*.  So `Disp-Fallback` as DESIGN.md states it (*a
   definition whose parameters are all untyped*) **has no surface**, and the worked example's
   `fn hit(a, b, world) { }` would not parse.  Step 6 below is where that gets a surface or
   gets dropped; it is not assumed.

---

## The invariant

> **One call resolves to exactly one definition, and which one is a function of the argument
> types alone.**

Everything below is that invariant re-asserted at one more position each time.  The risk the
steps are shaped against is the one this repo keeps paying for: *a fact re-derived in several
places, validated only where the derivations coincide*.  The key has **two** homes today
(write and read) and they must move together — a widened write with an unwidened read
resolves nothing; the reverse resolves to the wrong body.  That pairing is why steps 2 and 3
are one step, not two.

---

## The steps

Each step states **what it changes**, **how it goes red on its own**, and **what it is
compared against**.  A step with no red of its own is not a step.

### Step 0 — the oracle, before any change  ·  XS

Write the acceptance program from DESIGN.md's worked example as a **hand-written `match`**,
with its 12-row table as assertions, and run it on all three backends.  Nothing in `src/`
changes.

- **Red on its own:** the table disagrees with itself across backends.
- **Compared against:** the hand-computed table in DESIGN.md.
- **Why first:** it is the cheapest possible falsification of `Disp-Match-Equiv`.  If the
  three backends do not already agree on the `match` form, the rule is wrong before a line
  is written.  It also becomes the reference every later step compares to.

### Step 1 — make the key a function, with one caller  ·  XS

Extract today's key construction into one named function — `dispatch_key(name, &[Argument])`
— and have `get_fn` call it.  **Byte-identical output for every existing program.**

- **Red on its own:** `cargo test` — any key change breaks method resolution everywhere.
- **Compared against:** `loft introspect` on a corpus sample, byte-identical before/after.
- **Why:** the widening needs one home.  Doing the extraction as its own step means the
  refactor and the behaviour change are never in the same diff — the loft-codegen skill's
  rule for emit-shaped work.

### Step 2 — key on the parameter TYPES, not on a parameter NAME  ·  S

**Decided 2026-09-11 (owner).**  The key is built from the parameter types, whatever the
parameters are called.  `self` keeps its current meaning and only that meaning: *this
definition is also callable as `x.f(…)`*.  Dispatch stops depending on it.

Why this and not "append the remaining parameters": the key is only built today when
`arguments[0].name == "self"` or `"both"` (`src/data.rs:7568`).  Measured — with ordinary
parameter names, two definitions collide:

```loft
fn hit(f: Fire) { … }
fn hit(i: Ice)  { … }     // error: Cannot redefine 'hit'
```

So a programmer who *just writes the function for a specific case* — the whole point of the
feature — hits a redefine error whose cure is to rename a parameter to `self`.  Keying on
types removes the widening AND that wart in one rule instead of two.

**The constraint that shapes it, and it is a gift.**  145 sites in `src/` look a free function
up as `n_<name>` (`def_nr("n_main")`, `def_nr("n_compute")`, …).  So:

> **A name with ONE definition keys as `n_<name>`, byte-identical to today.  A name with
> SEVERAL keys each definition by its parameter types.**

That makes *"no existing program changes"* true **by construction** rather than by testing:
a program that compiles today has one definition per name — if it had two it would not
compile — so every existing key is untouched.  The only programs whose behaviour changes are
ones that are currently refused.

⚠ **Write and read must move in one commit.**  A widened write with an unwidened read
resolves nothing; the reverse resolves to the wrong body.  `Data::get_fn` (write) and
`Data::find_fn` (read) both call `dispatch_key` from step 1.

- **Red on its own:** two definitions with ordinary parameter names must stop erroring and
  must select correctly; and the `d4.loft` shape — same receiver, differing second parameter
  — must do the same.
- **Compared against:** step 0's table; the whole corpus unmoved; and `def_nr("n_<name>")`
  still resolving for every single-definition name.
- **Also verify:** `self` still gives method-call sugar and a non-`self` definition still does
  not — the two halves are now independent and a test should say so.

### Step 3 — arity in the key  ·  XS

Two definitions of one name at different arities are a real question DESIGN.md does not
answer (README's matrix flags it).  `bound_stub_name` already folds arity in; do the same
here, so `f(a)` and `f(a, b)` are distinct keys rather than a collision.

- **Red on its own:** a two-arity program must compile and select; today it collides.
- **Compared against:** the stdlib, which has same-name/different-arity pairs — they must be
  unmoved.

### Step 4 — `Disp-Ambiguous` as a refusal  ·  S

Two applicable definitions, neither more specific: refuse at the definition site, naming both.
Until this step the specificity order is not consulted at all — exact keys only.

- **Red on its own:** DESIGN.md's ambiguity program must fail to compile with both names in
  the message.
- **Compared against:** a near-miss control — the same program with one parameter made
  concrete must still compile.

### Step 5 — `Disp-Specific` over interfaces  ·  M

Until here, resolution is exact-key only: a call matches a definition or it does not.  This
step adds the partial order — a concrete struct beats an interface it implements.  The
abstract side is `bound_holder` (`src/data.rs:4226`, marked `#g`), which is **open question 1's
subject**: confirm it is the right abstract notion before building on it.

- **Red on its own:** a call with a concrete argument and both a concrete and an interface
  definition must select the concrete one; swapping which is declared first must not change
  the answer.
- **Compared against:** step 0's rows that exercise the `Projectile` fallbacks.

### Step 6 — `Disp-Fallback`  ·  S, and it needs a surface decision first

DESIGN.md's total default is *all parameters untyped*, and **untyped parameters do not
parse**.  So this step is a decision before it is an implementation: either give the fallback
a surface (a `_` parameter is the obvious candidate — confirm it is free in the grammar) or
drop the total default and let the most general interface be the fallback, as the worked
example's `Projectile × Projectile` row already does.

- **Red on its own:** the `Slime × Crate` row — a pair no definition names — must do what the
  chosen answer says, and the same program must have said something different before.
- ⚠ **Do not implement this before the decision.** It is the one step whose shape is not
  determined by the tree.

### Step 7 — `Disp-Closed` lowering  ·  M

A call site whose argument types are all statically concrete lowers to a direct call.  Note
this may already fall out of steps 2–3: exact-key resolution at parse time *is* a direct
call.  **Measure before building** — if `introspect` already shows a direct call after step 2,
this step is a test, not a change.

- **Red on its own:** `introspect` shows a table lookup where a direct call is owed.
- **Compared against:** the hand-monomorphised equivalent, byte-identical.

### Step 8 — the DCE property  ·  S

An unreferenced definition must be absent from the stripped artifact.

- **Red on its own:** artifact size, and the symbol's absence.
- **Why its own step:** it is the only property whose failure is invisible in behaviour.  An
  implementation that quietly retains every method passes every value test and silently costs
  the slim artifact its whole point.

### Step 9 — `Disp-Dynamic`  ·  M

A heterogeneous collection reaches runtime selection over the build-time set.

- **Red on its own:** step 0's rows through a `vector<Entity>`.
- **Compared against:** a control that a concrete site still emits a direct call and no table.

### Step 10 — `Disp-World`  ·  M, open profile only

- **Red on its own:** add a method mid-run; the new selection is taken AND a marker in the
  stale specialisation's body never appears.

### Step 11 — `Disp-Match-Equiv` in the differential oracle  ·  S

Pair a dispatch set with its canonical `match` as two programs that must agree.

---

## What I did not verify

Stated so nobody reads this file as more measured than it is:

- **Select-then-monomorphise vs the reverse** (open question 2) — not checked.  Step 7 is
  where it bites.
- **`bound_holder` is the right abstract notion** (open question 1) — its existence and its
  `#g` marker are measured; its fitness for `Disp-Specific` is not.
- **Whether `_` is free in the parameter grammar** — step 6's candidate surface, unchecked.
  (Step 6 is now expected to be a no-op: see the untyped-parameter decision in README.)
- **The wasm backend** — every probe above was `--interpret`, with `--native` on the
  redefine-error cases only.  Every step's matrix owes all three.
