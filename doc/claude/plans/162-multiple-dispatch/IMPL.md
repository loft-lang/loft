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

⚠ **Corrected by step 1 (2026-09-14):** the write is `Data::add_fn` on pass 1, keyed by
`Data::fn_key`; `get_fn` is the pass-2 re-lookup.  And the key's SPELLING had no single home
until step 1 gave it one (`Data::mangle_method`) — see § Step 1.

`Data::bound_stub_name` (`src/data.rs:7197`) already folds a marker and an arity into a key —
`t_<LEN><holder>#g<arity>_<method>` — so the widening has a shape to follow, not invent.

**3. The bare path ALREADY has the argument types.  The method path does not.**  Corrected
2026-09-11 after reading all 13 `find_fn` sites — an earlier draft of this file said the order
was the hard part everywhere, and that was too strong.

| Spelling | Real resolution | Types in hand there? |
|---|---|---|
| `f(x, a)` | `parser/mod.rs:5922` | **YES** — `types` is the full argument list, and the site already inspects ALL of it for nullability (*"routes `max(5, a?)` … arg0-only dispatch missed the arg1 case"*).  It then collapses to `types[0]` when building the key. |
| `x.f(a)` | `parser/fields.rs:617` | **NO** — `parse_method` parses the arguments at `:622`. |

`parser/control.rs:16602`'s `def_nr("n_<name>")` is **not** the resolution — it is `fn_def_nr`,
a HINT used to type-direct argument parsing (lambda inference, loft#1067; vector-literal
element widths, #432).

⚠ **So the real difficulty is not ordering — it is a circularity in the hint.**  Argument
parsing is type-directed by the chosen definition, and the choice depends on the argument
types.  With one definition that is fine.  With several, an argument that *needs* the hint —
a lambda, a width-carrying vector literal, a named argument — cannot be parsed until the
choice is made, and the choice cannot be made until it is parsed.

Two ways out, and the plan does not pick yet: **(a)** hint only where the candidate set AGREES
on that parameter's type, refusing otherwise; **(b)** parse hint-free and refuse if the result
is ambiguous.  (a) is more permissive and more code; (b) is a one-line rule with a worse error
message.  This is the design's genuinely open implementation question and it belongs to step 3.

**4. The remaining 11 sites are fixed-shape protocol lookups, not user call paths.**  `next`
(iteration) ×2, `Op<name>` (operators, dispatching on operand 1), interface conformance,
generic stubs.  Each looks up a known signature and none needs multi-parameter dispatch — so
steps 3–4 touch **two** sites, not thirteen.

**5. Select-then-monomorphise — confirmed, with a precedent for the two-phase move.**
`try_generic_instantiation(first_id, &types)` (`parser/builtins.rs:257`) takes the argument
types and REPLACES the chosen `def_nr` with a monomorph.  So a resolution already gets revised
once the types are known; the candidate-set selection is an extension of that shape, not a new
mechanism.  (Open question 2, answered.)

**6. `interface` is NOT a type — the abstract position must be the ENUM.**  Measured:

```loft
fn describe(x: Shape) -> integer { … }   // error: Expecting a type
fn describe<T: Shape>(x: T) -> integer   // works — an interface is a BOUND
fn take(e: Entity) -> text { … }  take(Fire{n:1})   // works — variant widens to enum
```

So `Disp-Specific`'s *"a concrete struct is more specific than any interface it implements"*
has no surface: there is no interface-typed parameter to be less specific than.  The subtype
relation loft actually has is **enum ⊃ variant**, and it is exactly what the rule needs — a
variant argument already widens to an enum parameter, and `fn tag(self: Entity)` already
coexists with `fn tag(self: Fire)` as separate keys.  (Open question 1, answered: the enum.)

**7. All three backends agree on every probe above** — `--interpret`, `--native`, and
`--native-wasm` run under `wasmtime`.  ⚠ `--native-wasm` COMPILES to `.loft/<script>.wasm`; it
does not run, so a bare invocation exits 0 having printed nothing.  Verifying wasm behaviour
means running the artifact.

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

### Step 1 — one spelling for the method key  ·  XS  ·  DONE 2026-09-14

Planned as *"extract the key construction from `get_fn` into `Data::dispatch_key`"*; the tree
had more than fact 2 above says.  The definition-side key already had a home,
**`Data::fn_key(name, &[Argument])`**, which `add_fn` uses on pass 1 — that is the WRITE;
`get_fn` is the pass-2 RE-LOOKUP of a definition already registered, and it re-spelled the
same key inline.  Beside those, `Data::method_key(type_nr, method, arity)` keyed from a type
and `Data::bound_stub_name` from a holder, and the raw `t_<LEN><Type>_<method>` spelling was
written out at **22** further sites (`find_fn`, `find_op_method`, the `OpDrop*` cascade
lookups, the `to_text` / `lit` / `hole_*` hooks, the native-method registry, an enum-variant
stub) — three homes and no primitive under them, so a key minted under one spelling and
sought under another would be silently unresolvable (the hazard `bound_stub_name`'s own doc
names).

**Done:** one associated **`Data::mangle_method(spelling, method)`** is the only place the
spelling exists; `fn_key`, `method_key`, `bound_stub_name` and every inline site call it.
Two PREFIX builders stay apart on purpose (they enumerate a type's keys rather than name one):
the `t_4Self_` scan in `parser/mod.rs` and the REPL's completion prefix.  Steps 6–7 change
`fn_key` (what the spelling carries) and `mangle_method` (how it is spelled) — two functions,
not twenty-five sites.

- **Compared against:** `bytecode-comparisons/step1-corpus.loft` (one function per key path:
  free, `self`, `both`, a `τ?` overload beside `τ`, the `τ?`→`τ` fallback, a user method on a
  stdlib type, a user operator, the `to_text` hook, bound stubs) — `introspect` byte-identical
  before/after and clean on both backends with the leak checks armed; and
  `scripts/introspect_diff.sh` over the whole corpus: **IDENTICAL 1504/1504**.
- **Not done here, deliberately:** `get_fn` still computes `(base, sig)` beside `fn_key`
  rather than calling it — it also needs `base` for the `τ?`→`τ` fallback, and the ORDER in
  which each site tries the spellings differs on purpose (`find_fn` tries both directions per
  `@FR-F-Recv`; `get_fn` and `find_op_method` only `sig` then `base`).  Unifying the order is a
  behaviour question, not step 1's.

### Step 2 — `candidates()` beside `find_fn`  ·  S

Add `Data::candidates(source, fn_name, tp) -> SmallVec<[u32; 4]>` returning the definitions a
name could resolve to.  Re-express `find_fn` as *"`candidates` returned exactly one"*, keeping
its existing fallback ladder (`τ?` → `τ`, `n_<name>`, the operator map) inside it.

- **Red on its own:** `find_fn` must answer identically at all 11 call sites.
- **Compared against:** `introspect` byte-identical.
- **Why separate:** the set is the new concept; introducing it while it always has one element
  means the concept and the behaviour change never share a diff.

### Step 3 — the BARE path passes the whole type list  ·  S

`parser/mod.rs:5922` already holds `types`; stop collapsing it to `types[0]` and hand the
whole list to selection.  With one candidate the answer is unchanged.

⚠ **This is where the hint circularity is decided** (fact 3).  Pick (a) or (b) before writing
it, and write the decision into RULES.md — it is user-visible in error messages either way.

- **Red on its own:** the `hint_d_nr` uses at `control.rs:16618` and `:16687` — a lambda
  argument and a width-carrying vector literal must still infer.
- **Compared against:** `introspect` byte-identical, **and the diagnostic corpus unmoved**.
  ⚠ The error-message comparison is the one that matters; the IR is the easy half.

### Step 4 — the METHOD path selects after its arguments  ·  M

`fields.rs:617` collects candidates; `parse_method` selects once the argument types exist.
This is the one genuine reordering in the plan.

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

⚠ **A defaulted parameter is the one optionality a definition carries** (owner, 2026-09-14 —
[README.md § Decisions taken](README.md#decisions-taken-owner), item 5), and it meets the
arity component here.  `fn f(a: A, b: integer = 0)` answers a two-argument call AND a
one-argument call, so beside a separate `fn f(a: A)` the one-argument call has two applicable
definitions.  Under the principle in [RULES.md](RULES.md#the-principle-the-rules-keep-landing-on)
that is a `Disp-Ambiguous` refusal naming both, not a preference for the exact arity — and it
must be measured, not assumed: how the parser answers `f(a)` for that pair TODAY (before any
step) is the control this step compares against.

- **Compared against:** the stdlib's existing same-name/different-arity pairs, unmoved; and
  the defaulted pair above, before/after.

### Step 8 — `Disp-Ambiguous`  ·  S

Two applicable definitions, neither more specific: refuse, naming both.

- **Red on its own:** DESIGN.md's ambiguity program must fail to compile with both names in
  the message; the near-miss control (one parameter made concrete) must still compile.

### Step 9 — `Disp-Specific` over the ENUM lattice  ·  M

The partial order: a VARIANT is more specific than its ENUM.  **Not interfaces** — fact 6
measured that an interface cannot be a parameter type, so the design's interface-based
specificity has no surface.  The enum/variant relation is real, already widens on argument
passing, and already produces two distinct keys.

- **Red on its own:** with `fn tag(self: Entity)` and `fn tag(self: Fire)` both declared and a
  value held at `Entity`, selection must reach the `Fire` definition — today it reaches
  `Entity` (measured: `generic generic`).  Swapping declaration order must not change it.
- ⚠ This overlaps `Disp-Dynamic` (step 13): held at `Entity`, the variant is a RUNTIME fact.
  So step 9 is the static half — a value whose static type IS the variant — and step 13 is
  the rest.  Cut them apart or step 9 cannot go red on its own.

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

## Step 0 — before any of it  ·  XS  ·  DONE 2026-09-14

Write DESIGN.md's acceptance program as a **hand-written `match`**, assert its 12 rows, run on
all three backends.  Nothing in `src/` changes.

- **Why first:** the cheapest possible falsification of `Disp-Match-Equiv` — if the three
  backends do not already agree on the `match` form, the rule is wrong before a line is
  written.  It is also the reference every later step compares to.

**Done:** `tests/scripts/162-step0-pairwise-interaction-as-a-central-match.loft` — the 12 rows
of DESIGN.md § Expected results, each from a fresh world and asserting values, lengths and WHICH
ids the world removed; a 13th row the table does not carry (the pair REVERSED reaches the
fallback — the control that a `match` matching every pair cannot pass); and an every-pair
scenario over a `vector<Entity>`, the design's physics loop.  Green on `--interpret`,
`--native` and `--native-wasm` through the corpus runners.  **"Nothing in `src/` changes" did
not hold** — finding 1 below was a wrong answer in the way, and inside a plan's own
verification step that is fixed on the spot.

**Findings — what the transcription met, and what each means for the later steps:**

1. **The design's central `match` did not parse, and the nearest loft spelling answered WRONG
   in silence.**  `match (a, b) { (Fireball(f), IceWall(w)) => … }` is not loft; the loft
   spelling `(Fireball, IceWall) => …` compiled, bound two locals named `Fireball` and
   `IceWall`, and matched every pair — no diagnostic, both backends.  A tuple element could
   not name a variant at all.  **Fixed in this step** (`formal/matching.md` D-match-5; guards
   `a-tuple-pattern-names-a-variant`, `a-tuple-pattern-refuses-a-name-that-is-no-variant`):
   an enum-typed element now takes `V` and `V { fields }` through the slice head's own
   lowering.  Consequence for step 8 / step 14: the canonical `match` of `Disp-Match-Equiv`
   EXISTS now, and its element form is one lowering shared with the top-level arm — the
   oracle pairing has a real second program to run.
2. **A scalar payload bound by a pattern is a COPY** (LOFT.md § Match expressions — a
   documented rule, not a deviation), so the design's `w.hp -= 3` through a binding does not
   land.  The program keeps each entity's mutable state in a nested `Status` record, which a
   binding views.  Consequence for `Disp-Match-Equiv`: the equivalence is stated over the
   canonical `match`, whose arm sees a VIEW of a record payload and a COPY of a scalar one; a
   dispatch definition's by-value parameter `w: IceWall` aliases the record (measured: writes
   through it land, P10), so the observable writes agree — but a scalar-field write is where
   they could NOT agree if a definition's parameter ever bound a scalar by value, and the
   oracle must carry that cell.
3. **A `&Entity` parameter cannot be a `match` or `is` subject** — refused at parse time on
   both backends (loft#1526; workaround: take it by value, a heap record aliases).  Not needed
   by the program; recorded because a dispatch definition that REBINDS its parameter would
   want `&`.
4. **Two spellings stay refused, as at a top-level arm:** an or-pattern between struct-enum
   variants in a tuple element, and the qualified `Kind.KFire` (whose message, *"'Kind' is
   not a variant of Kind"*, is wrong in both places and worth sharpening).  Refusals, not
   wrong answers.  The program spells the three projectile × player arms out one by one —
   which is precisely the repetition the design's `match` form exists to show.
5. **A variant IS a parameter type in every position** — `fn hit(f: Fireball, w: IceWall)`
   compiles as a free function and takes variant literals; an `Entity`-typed argument is
   refused (*expected Fireball, got Entity*), which is the static-type dispatch that fact 6
   and step 13 already record.  Closes the second item of § What I did not verify.

---

## What I did not verify

- **The hint circularity's two ways out** (fact 3) — neither (a) nor (b) has been prototyped;
  step 3 owes a probe before it is written.
- ~~**Whether a variant can be spelled as a parameter type in every position**~~ — verified
  by step 0 (finding 5): `fn hit(f: Fireball, w: IceWall)` compiles as a free function.
- **`Disp-World` against the actual promote path** — its current shape is unread.
