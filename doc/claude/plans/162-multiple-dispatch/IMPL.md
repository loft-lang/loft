<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 162 — implementation design, in small safe steps

Companion to [README.md](README.md) (the plan), [DESIGN.md](DESIGN.md) (the owner's proposal,
verbatim) and [RULES.md](RULES.md) (the amended rule set).  **Written against the tree,
measured 2026-09-11** — every claim has a file and line.

---

## The three facts that shape everything below

**1. Single dispatch already works, keyed on the FIRST parameter, and only when it is named
`self`/`both`.**

```loft
fn hit(self: Fire) { … }   fn hit(self: Ice) { … }    // coexist; f.hit() and hit(f) both work
fn hit(f: Fire)    { … }   fn hit(i: Ice)    { … }    // error: Cannot redefine 'hit'
fn hit(self: Fire, t: Wall) { … }                     // error: cannot redefine method on Fire
fn hit(self: Fire, t: Crate) { … }
```

**2. The key has exactly two homes, and the write side has exactly ONE caller.**

| Side | Function | Callers |
|---|---|---|
| Write | `Data::get_fn` — `src/data.rs:7567` | **1** — `parser/definitions.rs:1622` |
| Read | `Data::find_fn` — `src/data.rs:7624` | 11 — `parser/fields.rs` ×2, `parser/mod.rs` ×6, `parser/definitions.rs` ×1, … |

`Data::bound_stub_name` (`src/data.rs:7197`) already folds a marker and an arity into a key —
`t_<LEN><holder>#g<arity>_<method>` — so the widening has a shape to follow, not invent.

**3. ⚠ THE HARD PART: resolution happens BEFORE the argument types exist.**  Both spellings.

| Spelling | Resolves at | Argument types known at |
|---|---|---|
| `x.f(a)` | `find_fn` — `parser/fields.rs:617` | `parse_method` — `:622` |
| `f(x, a)` | `def_nr("n_<name>")` — `parser/control.rs:16602` | `types.push(t)` — `:16785` |

`find_fn`'s own doc says so: *"a method call's arguments are not parsed when its receiver is
resolved."*  **Multi-parameter dispatch needs the opposite order.**  This — not the key — is
the work, and it is why the plan is `H` rather than `S`.

The move it forces, and today's code is the degenerate case of it:

> **Resolution becomes two-phase: a CANDIDATE SET at the name, then SELECTION once the
> argument types are known.**

`find_fn` already does a version of this for bound holders — *"probe the arities and answer
only when ONE bound signature carries the name"* — so the precedent is in the same function.

---

## The invariant

> **One call resolves to exactly one definition, and which one is a function of the argument
> types alone.**

Every step is that sentence asserted at one more position.  The two key homes must always
agree: a widened write with an unwidened read resolves nothing, the reverse resolves to the
wrong body.

---

## Phase A — behaviour-preserving (steps 1–5)

**Nothing in this phase changes what any program does.**  Each step is provable by
`loft introspect` byte-identity over a corpus sample, which is the strongest comparison
available and the reason the risky part comes last.  A step here that changes output is a bug
in the step, not a feature.

### Step 1 — `dispatch_key` as a function  ·  XS

Extract the key construction from `get_fn` into `Data::dispatch_key(name, &[Argument]) ->
String`.  One caller.

- **Red on its own:** `cargo test` — any key drift breaks method resolution everywhere.
- **Compared against:** `introspect` byte-identical, before/after.

### Step 2 — `candidates()` beside `find_fn`  ·  S

Add `Data::candidates(source, fn_name, tp) -> SmallVec<[u32; 4]>` returning the definitions a
name could resolve to.  Re-express `find_fn` as *"`candidates` returned exactly one"*, keeping
its existing fallback ladder (`τ?` → `τ`, `n_<name>`, the operator map) inside it.

- **Red on its own:** `find_fn` must answer identically at all 11 call sites.
- **Compared against:** `introspect` byte-identical.
- **Why separate:** the set is the new concept; introducing it while it always has one element
  means the concept and the behaviour change never share a diff.

### Step 3 — move the BARE path's selection after the arguments  ·  M

In `parse_call`, defer the choice: collect the candidate set at `control.rs:16602`, parse the
arguments, then select at the point `types` is complete (`:16785`).  With one candidate this
is a pure reordering.

- **Red on its own:** any program whose diagnostics depend on resolving early — the
  `hint_d_nr` uses at `:16618` and `:16687` — changes its message.
- **Compared against:** `introspect` byte-identical, **and** the diagnostic corpus unmoved.
  ⚠ The error-message comparison is the one that matters here; the IR is the easy half.

### Step 4 — move the METHOD path's selection after the arguments  ·  M

The same for `x.f(a)`: `fields.rs:617` collects candidates, `parse_method` selects once the
argument types are known.

- **Red on its own:** `x.f()` on a `τ?` receiver must still reach `m(τ)` (`@FR-F-Recv`), and
  the `t_`-prefix guard at `:618` must still decline a free function.
- **Compared against:** `introspect` byte-identical.

### Step 5 — selection reads the FULL argument list  ·  S

Selection now takes every argument type, and with single-definition names still answers what
it answered before. The last behaviour-preserving step, and the one that proves the two
phases actually carry the types.

- **Red on its own:** an assertion that the selected definition equals `find_fn`'s answer, run
  over the whole corpus, must not fire.
- **Compared against:** `introspect` byte-identical.

---

## Phase B — the feature (steps 6–10)

### Step 6 — `Disp-Key`: key on the TYPES  ·  S  ← first behaviour change

`dispatch_key` uses every parameter's type and stops testing `arguments[0].name`.  Write and
read in ONE commit.  A name with one definition keeps `n_<name>` — 145 sites in `src/` look
free functions up that way, so this is a hard constraint and it is what makes existing
programs safe by construction.

- **Red on its own:** all four shapes in fact 1 above must now compile and select correctly.
- **Compared against:** step 0's table (below); `def_nr("n_<name>")` still resolving for every
  single-definition name; `self` still giving method sugar and a non-`self` definition still
  not.

### Step 7 — arity in the key  ·  XS

`f(a)` and `f(a, b)` become distinct rather than colliding, following `bound_stub_name`.

- **Compared against:** the stdlib's existing same-name/different-arity pairs, unmoved.

### Step 8 — `Disp-Ambiguous`  ·  S

Two applicable definitions, neither more specific: refuse, naming both.

- **Red on its own:** DESIGN.md's ambiguity program must fail to compile with both names in
  the message; the near-miss control (one parameter made concrete) must still compile.

### Step 9 — `Disp-Specific` over interfaces  ·  M

The partial order: a concrete struct beats an interface it implements.  The abstract side is
`bound_holder` (`src/data.rs:4226`, marked `#g`) — **open question 1's subject; confirm before
building.**

- **Red on its own:** a concrete argument with both a concrete and an interface definition
  must select the concrete one, and swapping declaration order must not change the answer.

### Step 10 — `Disp-Exhaustive`  ·  S

A call with no definition applicable to its STATIC argument types is refused
([RULES.md](RULES.md)).

- **Red on its own:** a program that omits the total case must fail to compile, naming the
  argument types; adding the total case must make it compile.
- **Why here:** it needs `Disp-Specific` (step 9) to know what "applicable to the static
  types" means for an interface, and it must precede `Disp-Dynamic`, because it is what
  guarantees the runtime step always has an answer.

---

## Phase C — the profiles (steps 11–14)

### Step 11 — `Disp-Closed` lowering  ·  M, **measure before building**

A statically-concrete call site lowers to a direct call.  This may already fall out of phase
B — exact-key resolution at parse time *is* a direct call.  If `introspect` already shows one
after step 6, this step is a test, not a change.

### Step 12 — the DCE property  ·  S

An unreferenced definition is absent from the stripped artifact.

- **Why its own step:** the only property whose failure is invisible in behaviour.  An
  implementation that quietly retains every method passes every value test and silently costs
  the slim artifact its whole point.

### Step 13 — `Disp-Dynamic`  ·  M

Runtime selection for heterogeneous sites.  Measured today: with `f: Entity = Fire{…}`,
`f.hit()` selects the `Entity` definition — dispatch is on the static type — so this is
genuinely new machinery.  `Disp-Exhaustive` makes it a lookup with a guaranteed answer rather
than a search.

### Step 14 — `Disp-World`, then `Disp-Match-Equiv` in the oracle  ·  M + S

Open profile only; then pair a dispatch set with its canonical `match` as two programs that
must agree.

---

## Step 0 — before any of it  ·  XS

Write DESIGN.md's acceptance program as a **hand-written `match`**, assert its 12 rows, run on
all three backends.  Nothing in `src/` changes.

- **Why first:** the cheapest possible falsification of `Disp-Match-Equiv` — if the three
  backends do not already agree on the `match` form, the rule is wrong before a line is
  written.  It is also the reference every later step compares to.

---

## What I did not verify

- **Select-then-monomorphise vs the reverse** (open question 2) — unchecked; step 11 is where
  it bites.
- **`bound_holder` is the right abstract notion** (open question 1) — its existence and `#g`
  marker are measured, its fitness is not.
- **The 11 `find_fn` call sites** — I read two (`fields.rs:617`, `mod.rs:8055`).  The other
  nine may have their own ordering assumptions; steps 3–4 must enumerate them first.
- **The wasm backend** — every probe was `--interpret`, with `--native` on the redefine cases
  only.  Every step's matrix owes all three.
