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

### Step 2 — `candidates()` beside `find_fn`  ·  S  ·  DONE 2026-09-14

Add `Data::candidates(source, fn_name, tp) -> Vec<u32>` (no `smallvec` in the tree; a
parse-time lookup does not earn a dependency) returning the definitions a name could resolve
to.  Re-express `find_fn` as *"`candidates` returned exactly one"*, keeping its existing
fallback ladder (`τ?` → `τ`, `n_<name>`, the operator map) inside it.

- **Red on its own:** `find_fn` must answer identically at all **13** call sites (the earlier
  count of 11 was low).
- **Compared against:** `introspect` byte-identical.
- **Why separate:** the set is the new concept; introducing it while it always has one element
  means the concept and the behaviour change never share a diff.

**Done, and the shape the ladder forced:** the walk stops at the first RUNG that yields
anything, and a rung's yield is a set.  Only one rung can yield more than one today — a bound
holder carrying the name at two arities — and that is the one ambiguity `find_fn` already
refused (loft#1275: no arity to offer at this entry point), so *"exactly one"* is the faithful
reading and "first of all rungs collected" would not have been (it would have picked one of
the two stubs).  The method rung yields the FIRST spelling that resolves, `@FR-F-Recv`'s own-
nullability-first order — which is a specificity order in disguise (the exact nullability is
the more specific definition), so `Disp-Select` inherits it rather than replacing it.
Proven: `bytecode-comparisons/step2-corpus.loft` (step 1's cells plus a user `next` protocol,
the name two callers resolve) byte-identical before/after and clean on both backends with the
leak checks armed; `scripts/introspect_diff.sh` **IDENTICAL 1504/1504**.

### Step 3 — the BARE path passes the whole type list  ·  S  ·  DONE 2026-09-14

`parser/mod.rs:5922` already holds `types`; stop collapsing it to `types[0]` and hand the
whole list to selection.  With one candidate the answer is unchanged.

⚠ **This is where the hint circularity is decided** (fact 3).  Pick (a) or (b) before writing
it, and write the decision into RULES.md — it is user-visible in error messages either way.

- **Red on its own:** the `hint_d_nr` uses at `control.rs:16618` and `:16687` — a lambda
  argument and a width-carrying vector literal must still infer.
- **Compared against:** `introspect` byte-identical, **and the diagnostic corpus unmoved**.
  ⚠ The error-message comparison is the one that matters; the IR is the easy half.

**Done:** `Data::select_fn(source, name, &[Type])` is `Disp-Select`'s entry point; it holds
the collapse `Parser::call` used to do inline (the receiver off the first argument, the `τ?`
routing off ALL of them — @PLN25 F1b(b)) and asks `find_fn`.  **Decided: (b)**, as
`Disp-Hint` in [RULES.md](RULES.md) — a multi-definition name offers no hint and an argument
that needs one is refused naming the cure; chosen by the principle (refusal is reversible,
agreement-hinting is additive) and because it falls out of `Disp-Key` for free (no `n_<name>`
key, nothing else for the hint sites to consult).  Nothing user-visible moves at this step: a
single-definition name keeps its hint, and `introspect_diff.sh` compares stderr too, so the
diagnostic corpus is proven unmoved by the same run.  Proven:
`bytecode-comparisons/step3-corpus.loft` (a `both` fn with a `τ?` overload called with the
nullable in position 0, position 1 and as a null; the stdlib `max` the same way; an empty
list; a width-carrying literal; a lambda; a defaulted argument) byte-identical before/after and
clean on both backends with the leak checks armed; `scripts/introspect_diff.sh`
**IDENTICAL 1504/1504**.

### Step 4 — the METHOD path selects after its arguments  ·  M  ·  DONE 2026-09-14

`fields.rs:617` collects candidates; `parse_method` selects once the argument types exist.
This is the one genuine reordering in the plan.

- **Red on its own:** `x.f()` on a `τ?` receiver must still reach `m(τ)` (`@FR-F-Recv`), and
  the `t_`-prefix guard at `:618` must still decline a free function.
- **Compared against:** `introspect` byte-identical.

**Done:** `parse_method_selecting(val, hint_nr, on, &MethodSelect)` parses the arguments
under `hint_nr` — `Disp-Hint`: the ONE `t_` candidate the name has at this receiver
(`Data::candidates` at the name), else the attribute slot's routine, which is exactly what
`find_fn`'s single answer was — and only then asks `select_method_def`: `Fixed` where the
caller already knows the definition (a bound's stub, an enum variant's method; the old
`parse_method` is that wrapper), `ByName` for `x.m(…)` on a concrete receiver, which is
`Data::select_method(source, name, dispatch, &types)` with the slot's routine as the
fallback when it names no `t_` method.  Proven: `bytecode-comparisons/step4-corpus.loft`
(both receiver directions of `@FR-F-Recv`, the hinted arguments — a width-carrying literal, a
typed lambda, named and defaulted arguments — an empty list, a chained call, a method through
a bound, and the two spellings with a nullable argument) byte-identical before/after and clean
on both backends with the leak checks armed; `scripts/introspect_diff.sh` **IDENTICAL
1504/1504**.

**Two findings, recorded not fixed (this step is behaviour-preserving):**

1. **The two spellings route differently on a nullable ARGUMENT.**  With `mix(both: P, n:
   integer)` and `mix(both: P?, n: integer?)` both declared and `n: integer?`, `p.mix(n)`
   reaches the dense overload (and warns `(N-Store)` on the argument) while `mix(p, n)`
   reaches the `τ?` one: the method path selects on the RECEIVER's type alone, the bare path
   on ANY nullable argument (@PLN25 F1b(b)).  `Data::select_method`'s doc says so; the corpus
   cell records both answers as they are.  `@FR-F-Recv` gives `x.m()` and `m(x)` one
   resolution for the RECEIVER's nullability and says nothing about an argument's.  **Step 5
   is where one rule must cover both** — and it is a behaviour change on one of the two
   spellings, so it lands with a matrix, not under byte-identity.
2. **A `|v|` lambda cannot infer on the method path.**  `b.each(|v| { v * 10 })` is refused
   (*cannot infer type for lambda parameter*) where `each(b, |v| { v * 10 })` infers: the
   method hint seeds collection and interpolation targets only (`seeds_collection_hint`),
   the bare hint seeds lambdas too (`seeds_lambda_hint`).  A refusal, not a wrong answer,
   and an admission to make — so not here; noted as a follow-on beside `Disp-Hint`, whose
   cure text names the typed form the method path already takes.

### Step 5 — selection reads the FULL argument list  ·  S  ·  DONE 2026-09-14

Selection now takes every argument type, and with single-definition names still answers what
it answered before. The last behaviour-preserving step, and the one that proves the two
phases actually carry the types.

- **Red on its own:** an assertion that the selected definition equals `find_fn`'s answer, run
  over the whole corpus, must not fire.
- **Compared against:** `introspect` byte-identical.

**Done in two commits, because step 4's first finding made half of it a behaviour change:**

- **5a (byte-identical):** `Data::select(source, name, &[Type])` is the ONE entry point —
  the receiver's dispatch type first, every argument after it — and `select_fn` /
  `select_method` are its two callers.  The asymmetry step 4 measured was carried for one
  commit as an explicit `NullRoute` parameter, so the fold could be proven byte-identical
  (`introspect_diff.sh` IDENTICAL 1504/1504) before the rule touched it.  The "selected ==
  `find_fn`" assertion the plan asked for is that corpus-wide byte-identity: the emission IS
  the selected definition.
- **5b (the rule):** `formal/calls.md` `(F-Recv)` gains the ARGUMENT clause @PLN25 F1b(b)
  had only in a code comment — a `τ?` argument in any position reaches `m(τ?, …)` when it is
  declared, whichever spelling — and `D-call-21` records that the method spelling read the
  receiver alone.  The parameter is gone; `select` routes on any nullable argument for both
  spellings.  Guard `tests/scripts/a-nullable-argument-routes-both-call-spellings-alike.loft`:
  the 3 × 3 × 2 matrix with every answer naming the body that ran, plus the two one-overload
  controls that do not move; falsified against 5a's commit, both backends.

**Phase A is closed.**  Both call spellings reach ONE selection over the FULL argument list,
the key has one spelling and one write, and the candidate set exists with one member.  Step 6
is the first behaviour change of the feature itself.

---

## Phase B — the feature (steps 6–10)

### Step 6 — `Disp-Key`: key on the TYPES  ·  S  ← first behaviour change  ·  DONE 2026-09-14

**Done — and M, not S, because the tree had a mechanism to reuse rather than a key to widen.**
A `both` name was already an overload set: a bare `Dynamic` dispatcher whose attributes are the
overloads, reached by the first parameter's key (`abs(integer)` / `abs(float)`).  Step 6 lets
any name enter it (`add_fn`: a second definition with a different FULL parameter spelling is
admitted, keyed by that spelling with `#` as the separator the emitter already sanitises; a
free incumbent is re-keyed and `n_<name>` retired), makes the pass-2 re-lookup (`get_fn`) ask
the full-spelling key first, and puts an EXACT rung in front of `candidates`' ladder that only
the two real call paths reach (`find_fn`'s thirteen protocol callers pass no list and stay on
the ladder).  `select` hands the routed list to it.  The unknown-function site names the
overload set: *no definition of `hit` takes (Rock) — declared: hit(Fire), hit(Ice)*, and the
two-exact case *`amb(Fire)` is ambiguous — it is taken by amb(Fire) and amb(Fire, integer)*.
RULES.md § Disp-Key carries the implementation form and its three consequences (one-source
scope; the key's coarseness over `vector<τ>`; exact selection until step 9).

Guards: `tests/scripts/a-name-may-have-several-definitions-keyed-by-parameter-types.loft`
(free / `self` / `both`; position 0, position 1, arity; struct, enum, scalar, `τ?`;
declaration order; a defaulted trailing parameter; both call spellings; the `Disp-Hint` cure)
and `…-refuses-what-is-not-an-overload.loft` (the same types twice; a free definition is not
a method; no definition takes the call; ambiguous by a defaulted parameter; a literal that
needed the hint).  Every existing program is byte-identical: `introspect_diff.sh` against the
pre-step binary reports DIFFERENT 2 of 1508, and the two are the new guards themselves.  Two
regressions the gate caught on the way, both mine: the first join branch re-keyed every
second `both` overload in the stdlib (1421 files moved — `atan2`, `log`, `pow`, `max`…), so
joining is for FREE definitions only and never the stdlib; and an arity-one free overload
keyed `t_4Fire_hit` wore a METHOD key, so the method-receiver reader reported *did you mean
the method `x.hit(…)`* for a call no definition took — free overloads have their own `f_`
prefix now.  What step 0's table needs beyond this is steps 9 and
13 (the enum lattice, and a value held at `Entity`).

**The library cell, measured the same day:** a library's overload set reaches its consumers
by every import spelling (wildcard bare, selective, qualified, aliased, the method spelling,
the library's own calls) — the "untested" flag was wrong in the safe direction, because the
set is reached through its bare dispatcher and not through the `n_<name>` alias an import
copies.  What the cell DID find: a consumer's `hit(f: Fire)` beside `use overloadlib`
exporting `hit(Fire)` / `hit(Ice)` registered in silence and a bare `hit(Fire {…})` answered
the LIBRARY's body — the pre-step analogue (a single imported `hit`) is refused as a
redefinition, and so is a method of the same receiver (`shadows_a_method`), but neither check
saw a dispatcher's overloads.  Closed in `add_fn`: a definition whose full spelling the name's
bare dispatcher already carries, from any source visible bare, is a redefinition naming the
library's position; another spelling still joins.  Guards: `a-library-exports-an-overload-
set`, `…-reaches-a-selective-and-an-aliased-import`, and `a-consumer-definition-of-a-carried-
signature-is-a-redefinition` (which also pins loft#788's two-package refusal), over the
fixtures `tests/lib/overloadlib.loft` / `overloadlib2.loft`.

`dispatch_key` uses every parameter's type and stops testing `arguments[0].name`.  Write and
read in ONE commit.  A name with one definition keeps `n_<name>` — 145 sites in `src/` look
free functions up that way, so this is a hard constraint and it is what makes existing
programs safe by construction.

- **Red on its own:** all four shapes in fact 1 above must now compile and select correctly.
- **Compared against:** step 0's table (below); `def_nr("n_<name>")` still resolving for every
  single-definition name; `self` still giving method sugar and a non-`self` definition still
  not.

### Step 7 — arity in the key  ·  XS  ·  DONE 2026-09-14

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

**Done — the arity half landed with step 6** (the full spelling carries every parameter, so
`cast(Fire)` / `cast(Fire, integer)` and a `both` at two arities are distinct keys, guarded
there), and the defaulted half is decided as the principle says: **a call that omits an
argument two definitions can take — a defaulted trailing parameter beside a shorter
definition, or two defaulted parameters at one arity — is `Disp-Ambiguous`, refused naming
both, in BOTH call spellings; a call that supplies the argument reaches the one definition
that takes it.**  Declaration order plays no part.  The control the plan asked for does not
exist: before step 6 such a pair could not be written (the second definition was a
redefinition), so nothing about today's programs moves.  The alternative — rank the
exact-arity definition above the default-filled one — is additive and can follow; refusing
first keeps it open.

**What the measurement found, and this step fixed:** the METHOD spelling of the ambiguous
call, `a.pg()`, took the attribute slot's routine (the incumbent) in silence where the bare
`pg(a)` refused — `select_method_def` fell back to the slot whenever selection had no single
answer.  It now refuses with the same message, rendered by one `Data::overload_signature`
both spellings use; the call still binds to the slot so nothing cascades, the diagnostic is
what refuses the program.  And for a `both` pair the bare site reported *did you mean the
method `x.pg(…)`*, because the method-receiver hint outranked the ambiguity check — the
ambiguity is asked first now.

### Step 8 — `Disp-Ambiguous`  ·  S  ·  DONE 2026-09-14 (with step 9's static half)

Two applicable definitions, neither more specific: refuse, naming both.

- **Red on its own:** DESIGN.md's ambiguity program must fail to compile with both names in
  the message; the near-miss control (one parameter made concrete) must still compile.

**Done — and it could not go red on its own after all**, because the design's program has
two definitions that both take a call only through a WIDENING (a variant to its enum), and an
overload set matched exactly, step 6's rung, could not see either.  So steps 8 and 9's static
half landed together, in `src/parser/dispatch.rs`:

- **`Disp-Applicable`** is the parser's own `can_convert` — the one home of *"may this value
  satisfy that slot"*, `@FR-C-Var` (a variant satisfies its enum) among its arms — and not a
  second spelling of it.  That is why selection moved up from `Data` to the parser: the
  predicate lives there.
- **`Disp-Specific`** is a rank per position — exact 0, a widening 1 (variant to enum, `τ`
  into `τ?`), a lossy discharge 2 (`τ?` into `τ`, `(N-Store)`), any other conversion 3 — and
  a definition is more specific when it is no worse at every position and better at one.
  **`Disp-Select`** is the unique minimal element; two minimal ones nothing ranks are
  **`Disp-Ambiguous`**, refused naming both, in both call spellings; none applicable falls to
  today's ladder (the free `n_<name>` beside a `both` set, the operator map) and is
  `Disp-Exhaustive`'s message only when that finds nothing either — the first cut refused
  right there and the stdlib stopped loading at `exists("…")` beside `exists(File)`.
- **The nullability routing runs FIRST** (`Data::routed_types`, uniform `τ?` when any argument
  is nullable), so `(F-Recv)`'s argument clause holds: without it `mix(p, n?)` ranked
  `(0, 2)` against `(1, 0)` and read as ambiguous.
- **@F20's synthesised enum dispatcher yields to an author's enum-level definition.**  A free
  overload set over variants retires `n_<name>`, so `enum_fn` found no enum-level definition
  and synthesised `hit(self: Entity, …)` beside the author's `hit(p: Entity, …)` — the
  missing-variant warnings, then a redefinition.  When the set carries a definition RECEIVING
  the enum (`Disp-Fallback`'s most general type), nothing is synthesised.

**Measured:** DESIGN.md's ambiguity program refuses naming both (with and without a total
fallback beside it); the near-miss picks the concrete definition; the 3-by-2 matrix with a
fallback answers every cell as hand-computed; a variant, its enum and the enum's `τ?` rank
in that order; a value held STATICALLY at the enum reaches the enum-level definition.  And
**phase 4's static rows are green**: `tests/scripts/162-step4-pairwise-interaction-through-
dispatch.loft` reaches step 0's twelve rows and its control through a `hit` overload set,
arguments held at their variant types, on both backends — `Disp-Match-Equiv` measured for
every static cell.  `damage` stays a `match` inside `hit(p: Entity, t: Player, …)`, because
there the projectile is held at `Entity`: the runtime cell.  Every existing program is
byte-identical (`introspect_diff.sh` IDENTICAL 1513/1513).

**Filed, not fixed:** loft#1528 — a `Fireball?` passed to an `Entity?` parameter warns as
stored into the dense `Entity` (a false `(N-Store)` warning after a correct selection;
pre-existing on both binaries).

### Step 9 — `Disp-Specific` over the ENUM lattice  ·  M  ·  static half DONE 2026-09-14 (see step 8)

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

### Step 10 — `Disp-Exhaustive`  ·  S  ·  DONE 2026-09-14

A call with no definition applicable to its STATIC argument types is refused
([RULES.md](RULES.md)).

- **Red on its own:** a program that omits the total case must fail to compile, naming the
  argument types; adding the total case must make it compile.
- **Why here:** it needs `Disp-Specific` (step 9) to know what "applicable to the static
  types" means for an interface, and it must precede `Disp-Dynamic`, because it is what
  guarantees the runtime step always has an answer.

**Done — one line of code moved, found by the guard:** the refusal landed with step 6's
message and step 8's applicability, but a FREE overload set over variants with no enum-level
member still had @F20 synthesise `hit(self: Entity, …)` on the enum — a method spelling for
functions that have none, and the uncovered call read as *did you mean the method
`x.hit(…)`*.  The synthesiser now yields to a set that carries a free member as well as to one
receiving the enum (`Data::overload_set_owns_dispatch`).  Otherwise this step is the rule's
guards and its record.  `an-uncovered-call-is-refused-naming-what-
was-passed-and-what-is-declared` pins the three uncovered shapes (a variant pair no definition
names; two values held at the enum with no enum-level definition — the soundness clause; a
scalar set asked for a type it lacks), and `a-total-case-covers-every-pair` adds the total
case and reaches it with the same calls.  RULES.md carries the implementation form (the
method spelling is refused by the receiver's method's own argument check; the pairs advice is
not built) and opens **D-disp-1**: @F20's synthesised dispatcher IS a runtime no-method path —
a missing variant is a warning and an empty value at runtime, both backends — which step 13
closes, and which is a compatibility decision because programs compile with that warning
today.

---

## Phase C — the profiles (steps 11–14)

### Step 11 — `Disp-Closed` lowering  ·  M, **measure before building**  ·  DONE 2026-09-14 (a test, not a change)

A statically-concrete call site lowers to a direct call.  This may already fall out of phase
B — exact-key resolution at parse time *is* a direct call.  If `introspect` already shows one
after step 6, this step is a test, not a change.

**Measured, and it does:** `loft introspect` on a dispatched program (`hit` over three
overloads, two statically-concrete sites) shows each site as a plain `Call(d_nr=…,
fn=f_16Fireball#IceWall_hit)` — no table, no dispatcher — and the native `main` calls the
selected function directly.  Against the hand-monomorphised twin (unique names, no overload
set) the whole dump differs only in the callee names, the def numbers (shifted by the
dispatcher's own def) and the live-reload table.  `tests/introspect_dispatch.rs` pins it: the
dispatched `main`'s bytecode is byte-identical to the twin's once the callee names are
normalised, both sites name their overload, and no enum-level dispatcher is synthesised.

**Finding for step 14:** the emitted live-reload table `LOFT_LIVE_FNS` lists neither
`f_`-keyed overload where the twin lists both `hit_fw` and `hit_fc` — the promote path
enumerates functions by name shape, so an overload cannot be live-flipped today.  Recorded
here, measured there.

### Step 12 — the DCE property  ·  S  ·  DONE 2026-09-14 (a test, not a change)

An unreferenced definition is absent from the stripped artifact.

- **Why its own step:** the only property whose failure is invisible in behaviour.  An
  implementation that quietly retains every method passes every value test and silently costs
  the slim artifact its whole point.

**Measured, and it holds by construction:** every static site is a direct call (step 11), so an
overload nothing calls is unreachable, and the SHIPPED lane — `--native-release`, which emits
only reachable functions — leaves it out.  On a `hit` set of four (two called, the total
fallback and a `(Slime, Crate)` overload uncalled) the release emission holds exactly the two
called ones; the semantics lane (`--native`, which keeps every tier by design — NATIVE.md
§ Optimisation tiers) emits all four, which is the control that lets the assertion fail.
The dispatched source is smaller than its `match`-form twin, not larger (28 359 against
31 733 bytes of emitted Rust for the same three cases): the `match` carries every arm in one
body.  `tests/introspect_dispatch.rs` pins both lanes.  The plan's "artifact size unchanged"
cell is subsumed: the emitted-function list is the property's direct witness, where a byte
count would also move with anything else in the file.

### Step 13 — `Disp-Dynamic`  ·  M  ·  DONE 2026-09-14

Runtime selection for heterogeneous sites.  Measured today: with `f: Entity = Fire{…}`,
`f.hit()` selects the `Entity` definition — dispatch is on the static type — so this is
genuinely new machinery.  `Disp-Exhaustive` makes it a lookup with a guaranteed answer rather
than a search.

**Done — and the machinery was not new after all, only one position deep.**  @F20's
synthesised enum dispatcher already IS `Disp-Match-Equiv`'s canonical `match` built by the
compiler for a `self` set: a match on the receiver's discriminant whose arms call the name
with the variant's type in the argument position, so each arm's callee is what selection
picks for that static type.  `parser::dispatch::dynamic_dispatcher` is that shape over ANY
position: for a call whose routed types hold an enum at a position the set decides by variant,
it synthesises — once per (name, spelling), on pass 2 only, because the set is complete only
then — an ordinary function over the routed types whose body tests the discriminant at each
dynamic position and, for every variant tuple, calls the definition `Disp-Select` picks for
those types.  Both backends compile it as any function; the runtime answer equals the static
one for the runtime types, deterministically; a tuple no definition takes, or two take without
ranking, is refused at compile time naming the tuple.  A set that covers every variant tuple
without an enum-level definition is admitted as covered — the closed enum's version of
`Disp-Exhaustive`, and D-disp-1's closure for every set that owns its dispatch.

Three things the tree forced, all measured: an enum VALUE has two spellings — `Enum(e, true,
…)` for a local, `Reference(e, …)` for an element of a `vector<E>` — and a position test keyed
on one was blind to the loop over a collection, the design's very shape; H5's two-pass
contract refuses a pass-2-only definition unless it is one of the lazy kinds, and the
dispatcher is a fifth of the same shape (name-keyed, idempotent, appended), admitted by its
`synthetic` mark; and the dispatcher is built as its own function inside a call site, so the
caller's parsing context is put aside and restored around it.

**Measured:** one position, two positions over the elements of a `vector<Entity>`, a mixed
static-plus-dynamic site with a struct parameter, the method spelling — on both backends;
and **phase 6 is green**: `tests/scripts/162-step6-every-pair-through-dispatch.loft` runs the
design's physics loop, every ordered pair of a `vector<Entity>` through the `hit` set, and the
world ends exactly where step 0's `match` left it, removals in the same visiting order.  Two
interim guards flipped as they said they would (the enum-held cells that read the enum-level
definition now read the runtime variant's), and the corpus differs from the pre-step binary
only at files that hold a dynamic site.

**A transcription flaw the runtime rows exposed:** the design's *any PROJECTILE versus
player* is a BOUND, and the enum-level `hit(p: Entity, t: Player, …)` the static rows carried
is wider — over every pair a Slime hitting the player counted.  Loft has no bound at a
parameter today (README Q1; the generics ranking is deferred, not declined), so the acceptance
program spells the three projectile definitions the design itself lists, and `damage` becomes
the design's own three overloads.  Step 0's `match` had exactly those three arms.

**Left open, narrowed:** D-disp-1 for `self` sets over variants, in two facets — the `self`-only
set, where @F20's warning and empty value stand pending the owner's compatibility decision, and
the `self` set WITH an enum-level member, which has no bare dispatcher (its keys never collide),
so @F20 yields and an enum-held receiver reaches the enum-level definition rather than its
runtime variant's; both pinned as measured.  Closing them means treating `t_<V>_name` over the
variants of one enum and `t_<E>_name` as one set.  The `Entity?` position (a nullable enum)
stays static — its runtime variant is the next cell.

### Step 14 — `Disp-World`, then `Disp-Match-Equiv` in the oracle  ·  M + S  ·  DONE 2026-09-14

Open profile only; then pair a dispatch set with its canonical `match` as two programs that
must agree.

**The promote path, read (closes § What I did not verify's last item).**  There is no
interpret-then-promote path in the tree.  What exists is @PLN18's tier 0 — `LOFT_LIVE_RELOAD=1`
watches every parsed file, and a changed **body** of an existing named fn is re-parsed under a
versioned temp name in a shadow session, its bytecode appended, and the original def's
dispatch targets patched (`fn_positions` + every recorded `OpCall` operand) — and the S2 flip,
by which a `--native` binary routes a compiled fn into a parked interpreter.  Nothing adds a
script at runtime: a brand-new `fn` block is skipped with the comment *"nothing calls it yet"*.
The design's premise *"already adds scripts at runtime and promotes them"* is therefore a
claim about the tree that does not hold, and Disp-World lands on tier 0, not on a promoter.

**Measured on the tree with an overload set (a `hit` set called from a running loop):**

1. Appending a more specific `hit(Slime, IceWall)` mid-run is refused as *`'hit' is not a
   known fn; skipped`* — the watcher keys `fn` blocks by NAME, so the add read as an edit of
   "hit", and `n_hit` is the set's `Dynamic` dispatcher, not a function — and the stale
   selection keeps running.  Silent for a set: the comment's *"nothing calls it yet"* is
   false there, every site of the set is a caller.  This is the runs-right-once-wrong-later
   family `Disp-World` exists for.
2. Editing the body of the FIRST overload of the name is skipped in silence (the name-keyed
   map holds only the last block); editing the LAST block is refused with the same wrong
   message.

**Design (built here):** the open profile is `LOFT_LIVE_RELOAD=1`, read once, and under it
every call into an overload set lowers to a call of a per-(name, spelling) synthesised
function — the dynamic sites already did (`n_<name>__dyn_<spelling>`, step 13); a static site
now calls `n_<name>__sel_<spelling>`, whose body is the one direct call `Disp-Select` picked.
That function IS the specialisation `Disp-World` speaks of: the world is the reload host's
version, and an add rebuilds every specialisation of the name in the new world and swaps it
in through tier 0's own patch, so the running loop's next call takes the new selection and no
body selected in an earlier world runs again.  The closed profile is untouched — the env var
is unset, the lowering is step 11's direct call, and the corpus is byte-identical.  Refusals,
all naming the cure (restart): an add that would make a served tuple ambiguous (Q3 — the ADD
is refused, the running world unchanged, as the design proposes and RULES.md's principle
requires); a removed or re-signatured overload; an add that would make a single definition a
set the program did not start with.  The watcher keys blocks by their declaration head, so an
overload's body edit reaches the overload it belongs to.

**Found on the way — each fixed here, none of them dispatch's own (the reload boundary is
where two parses of one program meet, and every disagreement between them is a frame
mismatch):**

1. **The shadow session's parse pipeline was a subset of `parse`'s.**  `parse_source`,
   `parse_virtual`, `parse_str` (the REPL's and the shadow's whole-program load) and
   `parse_snippet` (tier 0's per-edit parse) ran three of the six between-pass promotions
   (`promote_late_text_buffers` and `reserve_late_return_buffers` among the missing) and
   none of the post-pass-2 text-return promotion (@PLN104's `report_tret_promotions` +
   `targeted_tret_promotion`).  So a definition had FEWER hidden parameters in the shadow
   than in the running program — the synthesised dispatcher, an owned text return, is
   promoted to a `___tret` retbuf by `parse` and was not by `parse_str` — and a body
   generated from the shadow and called from the running program's sites read its buffer
   off the wrong slot: `SIGSEGV` at the first `OpAppendText`.  Now `Parser::between_passes`
   and `Parser::after_pass2` are the ONE home of each tail and every two-pass entry runs
   both; the parity check at install (names only) could not see this and still cannot — the
   pipeline being one function is what closes it.
2. **The REPL and the shadow parsed the user's program under source 0, the prelude's id,
   where `parse` and `parse_source` use `MAIN_SOURCE`** — so every gate keyed on "is this
   the stdlib?" by id misfired on user code.  Measured: the third overload of a name
   registered as a fresh `n_<name>` there (the set-join branch was guarded by the id), the
   first overload's pass-2 body then read *Unknown variable* for its own parameter, and
   every program with three overloads lost its watcher at install.  The join guard
   tests the FILE (`is_stdlib_source`), not the id, and so does the text-return promotion's
   owner test (`report_tret_promotions`).  `parse_str` KEEPS source 0: the REPL, the
   debugger's eval and the test harness resolve under that scope, and seven pinned
   diagnostics (a definition colliding with a stdlib one) depend on it — moving it to
   `MAIN_SOURCE` was tried and the gate refused it.  A session's synthetic eval
   (`replmain_`, `__eval_`) is never promoted: `Definition::is_reentered_eval` is the one
   home, since `parse_snippet` now runs the post-pass promotion too.
3. **The dynamic dispatcher mis-ordered a defaulted trailing parameter behind a forwarded
   text buffer** (step 13's own, closed profile, both backends): `tag(e)` over
   `tag(f: Fireball, k: integer = 7) -> text` beside `tag(e: Entity, k: integer = 7) -> text`
   was refused *expected integer, got &text on argument 2* — the leaf call appended the
   dispatcher's buffers straight after the supplied arguments, into `k`'s slot.  The leaf's
   omitted defaults are filled first now, as at a direct call.  Guarded by the oracle pair
   below, whose set carries the default.
4. **The watcher's entry file was recorded under source 0** — harmless while every lookup
   fell back to the global one, wrong for a set: it is now read off the
   shadow's own definitions of that file.
5. **A rolled-back definition stayed a member of its set** — `Data::rollback_to` truncated
   the definitions and left the dispatcher's `Routine(r)` attribute pointing past the end;
   every later selection would have ranked a routine nothing defines.  The rollback prunes
   them now (one home: the REPL's failed statement had the same hole).

**Measured — the matrix, each cell a running program observed through its stdout
(`tests/live_world.rs`, four sessions):** (a) a static site in the live `main` loop with
arguments at their variants and (b) a dynamic site with two positions held at the enum both
print the added definition's answer from the next round on, and the old answer never appears
again for that pair, while (c) the control pair the add does not serve is unchanged; (d) an
add that ties a served pair is refused naming the tuple, and the loop keeps its selections;
(e) a body edit of the FIRST overload reaches that overload and the other is untouched;
(f) a re-signatured overload is refused and the last good body serves; (g) a brand-new name
reports nothing and changes nothing.  `LOFT_RELOAD_DEBUG=1` lists every def a reload
generated with its parameters and every swap — the instrument that found the frame
mismatches above.  Closed profile: `introspect_diff.sh` IDENTICAL 1522/1522 against the
step-13 binary (the defaulted-parameter fix restored the three dispatcher files to identity
once it filled defaults by hand rather than minting dead buffers); every dispatch guard green
on both backends, and on the interpreter under the open profile too; tier 0's own tests
green after one of them was re-shaped — its module edit called its importer's `double`,
which a fresh parse refuses, so the reload had been accepting a program the language refuses.

**Step 14b — `Disp-Match-Equiv` in the oracle:** `tests/oracle/34-dispatch-set.loft` (the
set: a static site, a dynamic site over every pair of a `vector<Entity>`, the omitted and the
supplied defaulted parameter) declares `@ORACLE_TWIN: 34-dispatch-set-as-match.loft` (the
same three definitions as ONE function over the canonical `match`, arms in specificity order,
the fallback as `_`); the sweep now holds a program and its declared twin to one stdout on the
interpreter, with a positive control that a differing line or exit is caught.  The full sweep
is green with the pair in it.

**Not reached, recorded:** the native live-flip binary — a compiled caller keeps the world it
was built in until it is flipped (S3's contract), and `LOFT_LIVE_FNS` lists no `f_` overload
(step 11's finding), so an add under `LOFT_LIVE_FLIP=1` reaches only flipped callers; and a
`self` set (D-disp-1's remainder), whose @F20 dispatcher is not a set specialisation.

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
- ~~**`Disp-World` against the actual promote path** — its current shape is unread.~~ Read at
  step 14: there is no promote path; tier 0 is the boundary, and the rule landed on it.
