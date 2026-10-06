<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# formal/types-history.md — the deviation register for [types.md](types.md)

> **The rules are next door.**  [types.md](types.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

OPEN: **0** — `D-Opt-Value` was opened and CLOSED 2026-09-08 by owner ruling (loft#1471, below):
a `value struct` is nullable through `(L-Null-Tag)`'s discriminant, and nothing had to be built.
`D-Domain-Guard` (opened 2026-09-08, CLOSED 2026-09-12, below): `(N-Domain)`'s GUARD licence was not
uniform across its three families — the index and divisor rows are closed, and whether the
domain-partial MATH row is a code gap or a rule that over-promises is an owner call.
`D-Null-Chain` was opened and CLOSED 2026-09-08 (loft#1450, below): the last of
that issue's three legs, a field read through a nullable receiver typing non-null, closed by the
new `(N-Chain)` / `(N-Chain-Place)` pair.  `D-Null-Recv`, `D-Null-Guard` and `D-Null-Place` were opened and CLOSED 2026-09-07
(loft#1450, below): an element read through an ABSENT collection typed non-null, the `!= null`
guard that discharges it narrowed SCALARS only, and the narrowing it did perform described the
assignment TARGET.  `D-Opt-NoNull` was opened and CLOSED 2026-09-07 (loft#1423, below): `(N-Opt)` gained
its `has_null(τ)` precondition by owner ruling, and the tuple's absence is tuples.md `(T-Absent)`
(code half `D-tup-10` there).  `D-Var-Enum` was opened and closed 2026-09-06 (loft#1390, below); `D-Decl-Sev` was opened and closed 2026-09-05 (below); `D-Narrow-Res`, `D-Narrow-Asgn` and `D-Null-Elem` were all opened and closed 2026-08-31 (below); `D-Chk-Yield` was opened and closed 2026-08-28 (below); `D-Var-Join` was opened and closed 2026-08-27 (below); `D-Null-Join` was opened and closed 2026-08-26 (below); `D-Opt-Zero` is CLOSED (2026-08-24, below); the @PLN25 nullability flip (DN1–DN6) is CLOSED (2026-07-02); D1/D2/D4 closed by
fix/reconciliation.  The **@PLN102 DN3-Float extension** (below) is also CLOSED — SHIPPED
default-on 2026-07-11 (#559): float `/`/`%` and the domain-partial float functions type `τ?`
exactly like integer `/`/`%`.  Every DN1–DN6 + DN3-Float entry is CLOSED, retained as the
record.  Per-situation mitigation catalogue:
[../plans/25-nullable-sequences/DN1-MITIGATION.md](../plans/25-nullable-sequences/DN1-MITIGATION.md).

### D-Narrow-Limit — OPENED AND CLOSED (2026-09-22, loft#1593): a user-written `limit(a, b)` was outside all three narrowing rules

`(I-Narrow)`, `(I-Lit)` and `(N-Cast?)` are stated over ANY `Integer[a, b]`, and the stdlib's
width aliases (`u8`, `i8`, `u16`, `i16`, `u32`) obeyed all three: a store, call or literal not
provably in range refused at compile time with the cure named, the checked cast answering
null.  A user-written `integer limit(a, b)` obeyed none: the narrowing predicate keyed
"narrow" on the alias's `forced_size`, and a `limit` sets none, so every such slot read as the
FULL integer to the rules — while the runtime range guard (`OpRangeDefault`, loft#984) then
answered a legal in-range DEFAULT that an ordinary run never prints.  Measured over seven
types on both backends: `limit(1, 8)` read 1 for 1000, for −9 and for the literal 9;
`limit(−5, 5)` read 0 for all three; `1000 as integer limit(0, 7)?` read 1000.  The
language's own documentation test (`tests/docs/08-struct.loft`) taught the hole: *"a value
outside the range takes the type's default rather than being rejected."*

**Where the unsound step was.**  One predicate, `Parser::is_narrowing_int`, whose doc said
*"a `dst` with no `forced_size` is the FULL integer"*.  The full integer has two bound
encodings the spec already names (`is_signed32_template`, `is_wide_template`), and
`source_name` reads exactly those; keying on them instead makes every other spec — alias or
`limit` — a narrow target, and all three rules follow from the one predicate: the literal
exemption tests fit against the range, and the checked cast is built where the predicate
says a narrowing stands.  A cascade closed with it: a `limit` bound the type cannot CARRY
(loft#1037) is refused and then RECOVERS as the plain `integer` its own message recommends,
where before the defaulted bound left `integer(0, 2147483647)` — narrow, and a second
error about a type never written.  And the `redundant-coalesce` lint, which fired on the
very `?? d` the refusal names as the cure (`c.b = over ?? 0` on a `u8` — pre-existing on
the alias, and now the cure every `limit` slot is sent to), is asked through the same
predicate, so lint and refusal cannot disagree about which stores are narrowings.

**What loft#984 keeps.**  Its three regressions — an aliased, a dropped and a wrapped store
in the store layer — are still guarded, on the one seam the alias also leaves open: a slot
stepped past its range by its OWN arithmetic (`+=`, C85's overflow edge, `(E-Uncomp-NN)`).
Cells 984, 1009, 1379 and the boxed-capture cell re-spell every out-of-range arrival as `+=`
from a known value, so the same value reaches the same setter; the plain-store forms are
pinned as refusals in `tests/scripts/1593-a-limit-slot-refuses-the-narrowing-the-alias-refuses.loft`,
and the checked cast in 984's nullable section.

**The cost, and the policy that licenses it.**  Nine files of our own corpus and the
`imaging` library's tests (seven sites, three of them the worked example that TEACHES the
default) are refused until each writes its fallback — `Pixel{r: hi.r * 2 ?? 0}`, which stores
exactly the 0 the library documents, with the intent at the site.  COMPATIBILITY.md § The
error surface: a silent-wrong is a missing-error finding, contract 0 is the one time an error
can be added, and *"convert the programs it catches"* is the rest of the work.  `hex_world`,
the other library that spells `limit`, is untouched.  The type-inferred `integer(0, 255)`
(a `u8` element read `?? 0`) reaches the same rule and is sound: the compiler refuses every
implicit narrowing into the alias, so a `u8` slot genuinely holds `[0, 255]`.

Falsifier: the refusal cell above fails (five lines accepted, `0 0 0 0 0` printed) on the
parent build 87089b1fb.

### D-Opt-Value — OPENED AND CLOSED (2026-09-08, loft#1471, owner ruling): a `value struct` was refused a `τ?` it could represent

`(N-Opt)` licenses `τ?` for a τ with a value to spend on absence, and names TWO representations:
a reserved sentinel `(L-Null)` or a discriminant `(L-Null-Tag)`.  **Either suffices.**  A `value
struct` is stored INLINE, which is precisely the case `(L-Null-Tag)` governs — and it was refused
at the declaration anyway, on the reasoning that an inline value has no `store_nr` sentinel.  That
is true of the OTHER representation, and `(L-Null)` is not the rule for an inline slot.

Owner ruling 2026-09-08: *"a `Pt?` implementation is an enum variant of the record so it can be
null"* — which is `(L-Null-Tag)` verbatim.  **Nothing had to be built.**  `synth_nullable_target`
already admits a `value struct` (it is a `DefType::Struct`), so lifting the refusal turned on the
declaration, the local, the struct FIELD, the tagged vector ELEMENT and the `??` discharge at
once, on both backends, with one test failing in the whole suite set — the one that pinned the
refusal.

**The half that outlives this entry: a side condition enforced at ONE site is not enforced.**
`has_null(τ)` gated only the DECLARATION, while `(N-Domain)` and `(N-Chain)` construct a `τ?`
without asking.  So `v[i]` on a `vector<Pt>` and a `Pt` field read through a nullable receiver
minted `Pt?` regardless, the diagnostic NAMED a type the declaration forbade the author to write,
and its advertised `?? d` cure was dead code — the value could not be null, so the default never
ran.  The predicate still has no implementation in `src/`; it is re-derived inline at each site
that happens to ask, and the three lists still differ.

⚠ **What this does NOT close.**  loft#1471's original cell — an out-of-range read on a DENSE
`vector<Pt>` — still fabricates `Pt{0,0}` and reports `== null` false, because `(N-Dense)` says a
dense element pays no discriminant and the ruling does not change that.  What the ruling unblocks
is the honest fix: `(N-Domain)` can now type that read `Pt?` and MEAN it, because the tagged
absent value finally exists to hand back.  That was impossible while `Pt?` was unrepresentable,
and it is a change to the READ rather than a deletion.

### D-Domain-Guard — OPENED 2026-09-08, CLOSED 2026-09-12 (loft#1450's walk): `(N-Domain)`'s GUARD licence is not uniform across its families

`(N-Domain)` promises ONE elision — *"Non-null when the input is PROVABLY in-domain (constant /
range / guard)"* — and states it over every partial operation it names.  Walked as a rule rather
than as a site, the promise is kept unevenly.  Measured, one tree, both backends:

| licence | index `v[i]` | divisor `a / d` | `sqrt` `ln` `asin` … |
|---|---|---|---|
| constant | ✓ | ✓ | ✓ |
| expression proof | ✓ arithmetic over trusted leaves | ✓ `x / 2.0` | ✓ sign/interval lattice |
| guard, positive (`if d != 0 { … }`) | ✓ | ✓ | ✗ |
| guard, early return (`if d == 0 { return }`) | ✓ (2026-09-08) | ✓ (2026-09-08) | ✗ |

**The divisor row was the index row's defect, one family over, and is CLOSED the same day.**
`if d == 0 { return -1; } … a / d` typed `integer?`, and `divisor_proof_from_condition` already
answered *"proven on the ELSE side"* for `d == 0` — the fact existed and nothing consumed it on
the fall-through.  Both now take the guard-clause twin the null model has had since loft#585.

⚠ **The TRUTHY spelling is not part of it, and the attempt is the entry's most useful line.**
`if !d { return … } … a / d` reads like the same guard without the zero, and an arm for it was
written and reverted within the hour: in loft an INTEGER CONDITION IS ALWAYS TRUE — measured,
`if 0 { … }` takes the THEN branch and `!0` is `false` exactly as `!3` is — so that guard never
fires, and eliding on it made `f(10, 0)` answer a silent null through a non-null slot.  A
widening that REMOVES a diagnostic must be measured on the cell the diagnostic was ABOUT, not
only on the cells where it was noise; what caught it was the guard file's refusal assertion,
while every positive cell passed.  (That `if 0` takes the then branch is its own open question,
not decided here: it is what makes the C-shaped reading of `if !d` plausible to a reader.)

**The math row is a RULES-vs-HISTORY disagreement, not a plain deviation, and the difference
matters.**  `DN3-Float` above never claimed flow guards for the domain-partial functions; it
claimed a sign/interval lattice over EXPRESSIONS, and that claim was re-measured here rather than
read off the word "shipped": `sqrt(dx*dx + dy*dy)` and `sqrt(max(x, 0.01))` are non-null and a
bare `sqrt(x)` is correctly nullable.  So the code does what its history promised; it is
`types.md`'s own sentence that promises more, by naming "guard" for the whole family list.  Which
side moves is an owner call — narrow the rule to say where each licence applies, or widen the
lattice to take flow facts.

**Why it recurred, and the generalisation.**  *"Is this input provably in-domain?"* is answered in
THREE places — `index_bounded`, `divisor_nonzero`, and the float lattice — each with its own set
of admissible spellings, and none of them a home the others read.  That is
`one-question-many-decoders` at the level of a RULE rather than a routine, and it is why fixing
the index guard on the morning of 2026-09-08 left the identical hole in the divisor that
afternoon.  A single "what does this guard prove about this value" home, consulted by each
partial op, is what would make the rule's three licences one thing.

**CLOSED 2026-09-12 — the owner ruled that the LATTICE widens, not that the rule narrows.**
Narrowing would have enshrined the three-decoder split as intentional and left
`if x >= 0.0 { sqrt(x) }` typing `float?` — the same over-wide `?` that reached consumers as a
REFUSAL in `web` 0.3.0 and `stage` 0.18.1 once `(N-Chain)` carried it into a cast.

The cure is smaller than this entry implies, because the home already existed.  `Sign` already
carried the right lattice (`Pos ⊑ NonNeg ⊑ Unknown`) and `math_arg_in_domain` already consulted
it through `domain_sign` — which had **no `Value::Var` arm**, so a variable fell to `Unknown`.
A guard now contributes a `Sign` for the slot (`Parser::math_sign_proven`, the same
push/truncate/invalidate discipline as `divisor_nonzero`), and the consult side needed that one
arm.  So the guard COMPOSES with the expression lattice rather than sitting beside it.

Only `<` and `<=` exist as ops, so every spelling lowers into one of four shapes — derived, not
assumed: `x > 0` → `OpLt(0, x)` proves `Pos` in THEN; `x >= 0` → `OpLe(0, x)` proves `NonNeg` in
THEN; `x < 0` → `OpLt(x, 0)` proves `NonNeg` in ELSE; `x <= 0` → `OpLe(x, 0)` proves `Pos` in
ELSE.  Re-measured on both backends by the type `introspect` prints:

| licence | index `v[i]` | divisor `a / d` | `sqrt` `ln` … |
|---|---|---|---|
| constant · expression | ✓ | ✓ | ✓ (unchanged) |
| guard, positive | ✓ | ✓ | **✓ — was `float?`** |
| guard, early return | ✓ | ✓ | **✓ — was `float?`** |

⚠ **The widening REMOVES a diagnostic, so it is scored on the cells the diagnostic was ABOUT.**
Each of these still types `float?`: the ELSE side of `x >= 0.0`, an `x != 0.0` that proves no
sign, an upper bound alone (`x < 5.0`), the truthy `if x`, a slot reassigned after its guard —
and `ln` given only `NonNeg`, which is the cell that says the `Sign` reuse is doing real work
rather than acting as a boolean.  Guards:
`tests/scripts/1450h-the-math-guard-is-read-in-every-spelling.loft` (the positive direction,
with the two sibling families as controls that must not move) and
`tests/math_domain.rs::math_domain_guard_proves_only_what_it_proves` (the refusals, which cannot
share a file with cells that must compile).  The positive test was mutation-tested: with the
`Value::Var` arm returning `Unknown` it FAILS while the other six pass.

⚠ **This does NOT retire the three decoders.**  `index_bounded`, `divisor_nonzero` and
`math_sign_proven` are still three stacks; what is shared now is the rule's PROMISE, not its
implementation.  The generalisation named above — one "what does this guard prove about this
value" home — remains the open structural work.

⚠ **Found because a too-CONSERVATIVE type became observable.**  None of this was visible while a
projection dropped the receiver's `?`: the over-wide `?` on an index sat harmless until
`(N-Chain)` carried it one step further into a CAST, and `(N-Cast)` refuses a possibly-null
subject — so strictness the rules never asked for surfaced as a REFUSAL in code that was always
correct, in two PUBLISHED libraries (`web` 0.3.0, `stage` 0.18.1).  The direction is the lesson:
widening one rule is what audits another, and a value assertion cannot see a type that is merely
too wide — only something that READS the type can.

### D-Null-Chain — OPENED AND CLOSED (2026-09-08, loft#1450): a field read through a nullable receiver typed non-null

The last of loft#1450's three legs, and the one the register could not see: `D-Null-Recv` closed
the ABSENT-collection read and `D-col-lookup` the keyed MISS, but a plain `s.f` on an `s: S?`
still typed `f`'s declared type.  `get_field` reads the FIELD's own nullness, which is the field's
promise about itself and says nothing about whether the receiver is there to hold it — reading
`s.f` on an absent `s` yields the C80 null whatever `f` declares.

**The site already knew and could not say it.**  `fields.rs`'s own header has said since @PLN102
that *"a field access can't be MORE non-null than its receiver"*, and it acted on that by clearing
`expr_not_null` — the input to the redundant-check lints, which is why `s.f ?? d` was never called
redundant.  That is the same failure `D-col-lookup` recorded one door over: **a lint switch is not
a type, and `(N-Store)` reads the type.**  Two legs of one issue reaching for the same wrong
instrument is what makes it a class rather than a slip, and the closing question worth carrying is
*when a rule gains a site, is the site a TYPE or a flag?*

**What closed it is a rule, not a patch.**  `(N-Chain)` states the projection reading of
`(N-Prop)`: absence at any link reaches the chain's result type, and ONE discharge at the tail
covers every link.  That is what the language already DID — measured before the change, `a.b`
absent and `a.b.c.v ?? -1` answers `-1` two links later — so the type was the only half missing.
`wrap_projection_nullable` carries `wrap_keyed_lookup_nullable`'s guards for its reasons: no
marker on an unresolved type, the null model's switch, and no second wrap on a `__nullable<S>`
receiver that `@FR-N-Idem` forbids.

**The place half was falsified before it shipped, by the matrix.**  The first version of
`(N-Chain)` claimed PLACE position needed no exemption, because the assignment chokepoint already
peels a discharge off a target (loft#1205).  The write-side probe matrix said otherwise on its
first run: with the chain widened, `w.here.items.remove(0)` was refused — *"cannot call `.remove`
on `vector<Item>?` because it is NULLABLE"* — and the refusal fired on a PRESENT receiver too,
because it reads the TYPE.  `(N-Chain-Place)` is the answer (owner ruling 2026-09-08: *allow the
refused spelling, and do nothing in that case*): a mutating method's receiver is a place, peeled
to its dense type before dispatch.  The runtime needed nothing — a mutation through an absent
chain was already a total no-op on every mutation kind and both backends, which is the measurement
that made the ruling cheap to adopt.

⚠ **The peel names `remove` and `clear` as a LIST, and that is the entry's loose end.**  They are
the collection surface's only mutating methods today (insertion is `+=`, and the keyed kinds spell
removal as a statement), but a `both:` receiver carries no mutability marker, so nothing derives
the set.  `is_mutating_op` in `parser/control.rs` is the SAME question asked a second time for the
`parallel {}` capture check, from a hand-written allow-list of 18 IR ops that omits
`OpRemoveVector` — and that omission is a live both-backend crash, filed separately.  Two lists
answering one question is the shape `one-question-many-decoders` warns about; the derived home is
the work this entry leaves open.
### `(N-Shape)` — ADDED 2026-09-08 (@PLN153 phase 4 batch 11): the rule the peels were re-deriving one site at a time

Nine of phase 4's batches cured the same mechanism — a `matches!` / `if let` / catch-all `match`
over a `Type` that answers "no" for `Optional(τ)` — and each cured it at the CALLERS it had
reached.  `Type::peel_optional`'s doc block had stated the governing fact since @PLN25
(*"the nullability-agnostic majority of `match Type` sites peel through this; only the discharge
/ store / cast checks read the bool"*), with no rule, no citation and no gate, so every batch
re-derived it.  Written down as `(N-Shape)` in [types.md](types.md), cited at its home
(`peel_optional`) and at the verbs below.

**Why the design's own guarantee does not fire.**  `Optional` is a `Type` VARIANT rather than a
`nullable: bool` precisely so that *"every exhaustive `match Type` is a COMPILE ERROR until it
handles nullability (loud omission)"* — and `matches!`, `if let` and a catch-all arm are not
exhaustive matches.  Measured by `scripts/ir_walker_audit.py optional`: **2268** shape tests,
**1520** opaque on their own scrutinee.  A population that size is not a walk, so the enforcement
is a RATCHET on the count (`make optional-ratchet`, baseline `index/optional_ratchet.json`) —
the same argument `asan_leak_ratchet.sh` makes for a count over an allowlist: the opaque tests are
indistinguishable from each other by any pattern a suppression could name.

**Three verbs fixed at the verb, not at their callers**, so ~48 bare call sites are answered at
one home: `is_scalar`, `Type::heap_def_nr` and `Type::heap_dep`.  Corpus-measured with
`scripts/introspect_diff.sh`: **9 of 1411 files move**, all nine null guards, every one identical
in VALUE on both backends and under `LOFT_STRICT_STORES=1` / `LOFT_POISON=1` / `LOFT_STORES=warn`.
Eight take the copy-or-adopt lowering their dense twin already took (`heap_def_nr` is `(B-Copy)`'s
home, and loft#1319 had cured only the local-to-local site); the ninth moves a hoisted nullable
scalar's declaration into the native prologue, which is where batch 3's own cell said it belonged.
`heap_dep` alone reads IDENTICAL 1411/1411 — insurance, exactly as batch 2 argued when it peeled
that verb's four `scopes.rs` callers by hand.

**⚠ `is_dbref` is the one OPEN exception, and it is measured rather than assumed.**  Peeling it
moves **102** files and breaks **12** guards, because at least thirteen callers read its blindness
as a NULLABILITY test — they ask *"is this a non-null heap slot?"* and take the answer from a
SHAPE predicate's missing arm.  One of them is named and cured here without peeling the verb:
`nstore_null_report_as`'s `heap_target` tests the synthetic `__nullable<S>` spelling of `τ?`
explicitly and left the `Type::Optional` spelling to `is_dbref` answering false — one notion, two
spellings, one of them looked for.  Spelling the second is byte-identical today and makes the
gate's reason local.  Splitting the rest is a walk of its own; until it lands,
`data::tests::is_dbref_is_the_documented_exception_to_shape_agreement` holds the exception open
and FLIPS when it closes.

**And two of the audit's rows are FALSE POSITIVES**, recorded so the baseline is honest rather
than merely frozen: `borrow_deps` and `rewrap_deps` name `Rewritten` with no `Optional` arm but
delegate to `deps_ref` / `with_deps`, which are dep-transparent by construction.  `is_unknown`
(106 bare callers) and `find_fn` are DELIBERATE — phase 0 F1 settled that a wrapper over a stub
IS a written type at the settledness guards, and `(F-Recv)` keys two overloads apart BY the `?`.

### D-Null-Recv — OPENED AND CLOSED (2026-09-07, loft#1450): an element read through an ABSENT collection typed non-null

`(N-Domain)` types a partial operation `τ?` when the reserved null is reachable from an input,
and elides that only where the input is *"PROVABLY in-domain (constant / range / guard)"*.  The
elision is `index_provably_fit`, and what it proves is about the INDEX — a literal the developer
typed, a loop variable, a bounded computation (the loft#1436 trust).  It says nothing about
whether the COLLECTION exists.  Reading an element of an absent collection answers the element
type's null (C80) whatever the index names, so the elision was being asked a question it does
not answer:

```loft
m: vector<It>? = null;
z: It = m[0];        // silent — `z` typed non-null It, holds null
z2: It = m[i];       // warned — same receiver, same absence, untrusted index
```

The falsifying pair is those two lines: obeying the rule reports both, obeying the code reports
one, so the promise depended on the SPELLING OF THE SUBSCRIPT rather than on anything about the
value.  All four keyed kinds answered the same way (`hash`, `sorted`, `index`, `trie`) — the
keyed arms carried the `expr_not_null` clear and never a type.

The `?` is discarded by the `base()` peel that lets a `text?` dispatch its methods at all, so
the fix reads the receiver's nullability BEFORE that peel and carries it to the result type.
Closed at `parse_index`; the dense-receiver trust is untouched, which is the control.

This also closes the *"nullable-receiver index typing"* half that loft#1434's ruling
(`(Col-Insert-Absent)`, C118) left open for whoever took the fix.

⚠ **Two legs of loft#1450 stay open and are deliberately NOT closed here**, because they are
migrations rather than one-site fixes and were measured before the split was chosen: a FIELD
read through a nullable receiver (`n.v` with `n: It?`, which `(N-Prop)` types `τ?`) costs 37
corpus sites, and a keyed lookup's own `τ?` for a MISSING key in a PRESENT collection
(`(Col-Lookup)`) costs 351.  This leg costs none.  `(Col-Lookup)`'s cited anchor is
`fields.rs:700-706`, *the `expr_not_null` clear* — a typing rule whose recorded enforcement is
a LINT FLAG, which is why `collections.md` could read `OPEN: 0` over it.

### D-Null-Guard — OPENED AND CLOSED (2026-09-07, loft#1450): the discharge narrowed SCALARS only

`(N-Store)` requires a discharge, so a discharge has to exist for every type it governs.  Flow
narrowing is the one that costs nothing to write — `if x != null { … }` — and it worked for a
scalar and for no heap value at all:

```loft
fn g(s: S?) -> S { if s != null { s } else { S{n:0} } }   // warned, though guarded
```

A heap value asks "is this absent?" through its own opcode — `OpRefIsNull`, `OpVectorIsNull` —
where a scalar compares against a `…FromNull` literal, and only the scalar spelling was read out
of the condition.  So the guarded form and the unguarded form were reported identically, and the
author who did the right thing got the same diagnostic as the author who did not.

⚠ **This is `D-Null-Heap`'s class, recurred on the other side.**  That entry (2026-09-03,
loft#1313) closed an ENFORCEMENT gated on a scalar-only predicate; this is the DISCHARGE gated on
a scalar-only spelling, and the two together give the rule a reader can check the next
implementation against: **when the null model gains a site, ask whether its predicate reads a
SCALAR spelling of absence** — the heap spelling is a different opcode, not a different value,
and every one of these has been invisible to a reader checking the code because the scalar story
is coherent on its own.

That this leg had to be closed BEFORE `D-Null-Recv` could ship is the measurement worth keeping:
with no working guard, correct guarded code and silent-wrong code produce the same warning, and a
rule enforced that way cannot be adopted whatever the register says about it.

### D-Null-Place — OPENED AND CLOSED (2026-09-07, loft#1450): a narrowing described the assignment TARGET

`(N-Decl)` makes a declared slot a COMMITMENT: `x: τ?` is `τ?` for the whole of its life.  A flow
narrowing is the opposite kind of fact — it says what the slot currently HOLDS, and it dies at the
next write, which the parser already knew (it drops the proof once the store is built).  It was
still allowed to describe the target of that very write:

```loft
cur: It? = src;
if cur == null { return -1; }
cur = src;            // "a nullable `It?` is stored into the local `cur` of the non-null type `It`"
```

The target is parsed as an expression, and a proven-non-null variable reads as its peeled base —
so a declared `τ?` slot answered `τ`, and writing a `τ?` into its own declared type was reported
as a nullable reaching a non-null slot.  Two separate store checks consumed that same peeled
type, which is why the fix is at the assignment chokepoint rather than at either of them: the
left-hand side of an assignment is a PLACE, not a value read.

The entry is a deviation and not a nicety because the diagnostic is `warning`, and `warning` gates
a library's CI (`LOFT_DENY_WARNINGS=1`) — so a library that guarded a nullable and then rebound it
failed its own gate on correct code.  It pre-dates loft#1450 on the scalar path and was reachable
there; the heap narrowing above is what made it reachable for a struct or a vector, and what
turned it up.

### D-Opt-NoNull — CLOSED (2026-09-07, loft#1423, by RULE): `(N-Opt)` gains the precondition `has_null(τ)`; a tuple that arrives absent is a present tuple of null members

Closed the same day by the owner's ruling rather than by code: *"the tuple type itself is not
nullable, but if we read it as null we present it as a tuple that exists with all its members
null"* — a tuple has no faithful document form anyway.  `(N-Opt)` now reads `τ wf, has_null(τ)
⟹ τ? wf`, which is what it always meant (two formers were refused by name under it), and the
representation of a tuple's absence is `tuples.md (T-Absent)`: `optional((τ₁, …, τₙ)) ≡
(τ₁?, …, τₙ?)`, no `Optional(Tuple)` even in flight.  The stack-tag layout that would have
made `(τ, τ)?` a type is declined in DESIGN_DECISIONS C119 on brittleness (ten-plus silent
re-assertion sites for a one-clause rule).  The CODE half — `Type::optional` still wraps a
tuple, so a generic `T?` at a tuple answers garbage / E0308 (loft#1451), `v[i].0` is refused
where the rule types it `τ?`, and `t == null` has no tuple home — is `D-tup-10` in tuples.md.
The entry as opened follows.

#### As opened — OPEN (2026-09-07, @PLN153 phase 4 batch 8, loft#1423): `(N-Opt)` licenses `τ?` for every τ, and two types have no null to spend

`(N-Opt)` is `τ wf ⟹ τ? wf` — *"`τ?` is a type for any τ"*.  Two types are refused at the
declaration instead, and for the same reason: the null MODEL (the @PLN102 keystone, option B,
frozen as C90) gives absence either `(L-Null)`'s in-band sentinel — a value the type RESERVES —
or `(L-Null-Tag)`'s discriminant, which is for a STRUCT stored inline.  A `value struct` has
neither (@PLN101, refused by name since then), and neither does a TUPLE: it is its members'
bytes, and the synthetic `__tuple<…>` is a struct only where the tuple is STORED — a stack-backed
tuple, which `(T-Ref-Rep)` says is what every-member-scalar gives, has nowhere to put a tag.

Measured 2026-09-07: `(integer, integer)?` was not merely refused, it was not consumed.  The
tuple branch of `parse_type_full` returned before `parse_type`'s postfix-`?` handling, so the
`?` was left for whatever came next and every position reported a syntax cascade naming
nothing — *"Expect token ;"* for a local, *"Expect token )"* for a parameter, *"Expect token >"*
inside a `vector<…>`, *"unexpected '?'"* for a field or an alias.  A nullable MEMBER
(`(integer?, integer)`) has always worked and is unaffected.

**Mitigated, not closed.**  The refusal now names the tuple the author wrote and the two cures
(make the members nullable, or wrap the tuple in a `struct`), beside the `value struct` case in
the same function.  Closing it needs the design call loft#1423 carries: give a tuple
`(L-Null-Tag)`'s treatment where it is stored — which would make one written type nullable in
one position and not in another — or state the precondition in `(N-Opt)` itself, that τ must
have a null to spend.  Guard: `tests/scripts/1423-a-nullable-tuple-type-is-refused-by-name.loft`
(seven positions, each naming its own tuple so no two expectations share a substring), plus the
nullable-member cell that proves the refusal is about the tuple TYPE.

**The shape EXISTS, which is what makes the deviation more than a spelling.**  The walk's first
reading was that the seven TUPLE tier-0 functions in `parser/mod.rs` are opaque to a shape no
program can build.  Two routes build it anyway, and neither goes through the type parser:
`(N-Index)` — `v[i]` on a `vector<(τ, τ)>` IS `(τ, τ)?` — and a generic `-> T?` instantiated at a
tuple, whose own refusal names the type (*"No matching operator '==' on '(integer, integer)?' and
'null'"*).  Measured on that shape, the same afternoon:

- an undischarged member read had no answer and no message: `v[i].0` reported *"Expect a field
  name"* — a message about a NAME, for a program that wrote a number, on a receiver whose real
  problem is the `?`.  A nullable STRUCT receiver reads correctly through its absence
  (`(L-Null-Which)`), which is what makes the tuple's silence read as a defect rather than a
  rule.  Refused by name now, with the two discharges that work
  (`tests/scripts/1423b-…`); a naive peel of the projection was built and measured first, and it
  ICEs in codegen — the value has no representation, exactly as this entry says.
- the `?` discharge answered NULL MEMBERS in a slot typed non-null — `(N-Default)` broken for a
  tuple, while its struct twin was right — because `Data::has_default` recursed over the members
  and `Parser::build_default` had no tuple arm, and the recovery for that disagreement was
  silent.  Fixed (loft#1424): the tuple default is its members' defaults, the value the `??`
  spelling hands over, and the recovery now REPORTS instead of typing an absent value non-null.

So the seven functions are closed by their own measurement — a nullable tuple reaches none of
them, because every route to one is refused or discharged before it gets that far — and the
deviation stays open on the rule.

### D-Var-Enum — OPENED AND CLOSED (2026-09-06, loft#1390): an arm answering in the ENUM was asked to convert to its sibling's VARIANT, and the `match` join never widened

`(C-Var)` licenses `Reference(S) ⤳ Enum(E)` for `S ∈ variants(E)` and licenses nothing between
two variants.  So wherever the arms of a branch have settled on ONE variant, "does this arm
convert to that?" is the wrong question for exactly two kinds of later arm — another variant of
the same enum, and the ENUM itself — because both join to `E`, which is WIDER than what the arms
had settled on.  loft#1117 closed the variant half in 2026-08.  The enum half was still asked, and
answered *"expected Circle, got Sh"*:

```loft
e: Sh = Circle{r: 1};
e = match e { Circle{r} => Circle{r: r + 1}, _ => e };   // refused: expected Circle, got Sh
e = if e is Circle { Circle{r: 2} } else { e };          // refused: expected Circle, got Sh on else
```

`_ => e` hands back the very binding the statement assigns, which is the ordinary shape of
"replace it when …" over an enum.  Both backends agreed (it is a typing refusal), the wildcard
and the named-arm spelling both failed, and the recorded workaround — returning the new variant
through a function typed as the enum — worked only because a CALL arm already types as `E`.

**And the acceptance was only half the deviation.**  The `match` join kept the FIRST arm's type
however many arms widened it, so the variant-typed destination `(C-Var)` exists to refuse was
admitted:

```loft
e: Sh = Square{s: 7, extra: "hello"};
v: Circle = match e { Circle{r} => Circle{r: r + 1}, Square{s} => Square{s: s, extra: "x"} };
println("v.r={v.r}");                                    // 7 — a Square's `s` read at Circle's `r`
```

That is loft#980's class (a field read at another variant's offsets, tag never consulted), silent
on both backends, and the `if` twin has refused it throughout — `parse_if` widens to the enum
before the destination is checked.

**Closed by giving the acceptance and the join ONE predicate.**  `Parser::joins_to_enum` is the
home; `arm_joins_to_enum` asks it at the two acceptance sites (`block_result`'s `else`/chain arm
and `parse_match_arm_body`), and `join_arm_into` asks it at the six `match` arm sites, where the
settled type and the new arm are both in hand.  An arm the acceptance waves past is therefore
exactly an arm the join widens for — the invariant that was missing, and the reason the two
halves could disagree.  The predicate itself gained the check its new callers need: a
`Reference` arm must really be a variant of THIS enum, not merely a different def, since an
acceptance site sees types a join site only saw after `convert` had already refused them.  The
cross-arm gate reads the same rule (`match_arms_unify`), because an arm keeps its own variant
type on purpose so the join can still see the two differ.

Guards `a-variant-arm-joins-with-its-enum` (7 cells: wildcard, named arm, the `if` twin,
enum-first, variant × variant, the same variant twice, a call arm) and
`a-variant-typed-destination-refuses-a-widened-match` (the refusal), both falsified at
32e36462 — exit 1 -> 0 on both backends, and the refusal guard's expectation FAIL -> matched.

### D-Decl-Sev — OPENED AND CLOSED (2026-09-05, @PLN153 phase 3b): the declared LOCAL was refused at every width where `(N-Store)` warns, and the inferred one was refused where `(N-Join)` widens

`(N-Store)` gives one severity for a `τ?` reaching a non-null slot undischarged — *"a WARNING for
most τ … a hard ERROR only for narrow widths"* — and `(N-Decl)` says a declared `x: τ` makes such
a write `(N-Store)`-illegal, which is that severity, not a stricter one.  The worked table under
the rules said otherwise (`a: integer = 2; a = v[i]` → *"type error"*), the pre-cutover reading
that was never updated when @PLN102 shipped the split, and the code did what the table said: a
DECLARED local was refused by `change_var_type`'s "cannot change type from `τ` to `τ?`" at every
width — the twelfth home of the refusal, and the only one that erred where the eleven others
warned.  The Stage A matrix made it exact: of the 102 cells the `local` row was the one row that
read ERROR for every full-width kind while `element`, `argument` and `return` read WARN.

The second half was hidden by a vacuous guard.  `(N-Join)` says an INFERRED `a = 2; a = v[i]`
widens `a` to `integer?`; the phase-1 hold cell wrote exactly that and passed — with a CONSTANT
index, which @PLN102 D1 trusts by contract and types non-null, so nothing was ever widened and the
assert read the in-band sentinel.  With a variable index the same program was REFUSED with the
declared local's message: the inferred local had no arm at all.

```loft
fn d(v: vector<integer>, i: integer) -> integer { x: integer = v[i]; return x; }   // was ERROR; the rule: WARNING, x holds null
fn j(v: vector<integer>, i: integer) -> integer { a = 2; a = v[i]; return a; }     // was ERROR; the rule: a: integer?, the RETURN warns
```

Closed by giving each half its home.  A declared local (a parameter too — its type is the
signature's — and a write-back `&τ` parameter, which is the caller's slot one link away) is asked
through the one store face (`convert_store`, "the local `x`" / "the parameter `x`") BEFORE the
retype, at the assignment seam and at the tuple-destructure site, so the arm reports with the
rule's split and the retype sees the peeled type; an inferred local takes `change_var_type`'s
`(N-Join)` arm, which widens to `Optional(the wider width)` and stays silent — the widen is proven
by the next declared slot it reaches, which warns.  `retype_would_be_refused`, the loft#1145 gate
that predicts the verdict, already read the two through one predicate (`decl_accepts`) and agreed.
Stage A: the five full-width `local` cells moved ERROR → WARN, the narrow one stayed ERROR, no
other cell moved.  Guards: `153-n-store-a-declared-local-warns-at-every-full-width-kind` (nine
slot shapes, one report each), `153-n-join-an-inferred-local-widens-to-nullable` (seven joins, the
widen proven by a later declared store), `153-n-store-refused-into-a-narrow-local` (every narrow
slot the local family reaches, four cells), and the seven phase-1 pair files re-pinned to the
rule's text; `153-n-rules-hold`'s `(N-Join)` cell now indexes by a variable, which the parent
build refuses, so its compiling is the receipt.  The `&τ` parameter had never been asked at all
(the `RefVar` peel carried the null in silence) and reports with the rest.  A documented refusal
became a warning — `Contract: strained`; a program that compiled yesterday still compiles, and
`x: integer = find(…)` now runs with a warning where it used to stop.

### D-Null-Assign — OPENED AND CLOSED (2026-09-06, loft#1404): the ASSIGNMENT TARGET was the fifth position

`(N-Store)` names the slots a bare `null` may not enter — *"a local, a field, a collection
element, a tuple member, a call argument, a return, an INDEX"*.  `D-Null-Heap` above wired the
heap half at four of them; the assignment target asked only for a SCALAR target, so two shapes
answered WRONG in silence, on both backends:

```loft
s.rec = null;    // does not happen — `s.rec.n` still reads 5, where `S{rec: null}` reads 0
v[i]  = null;    // a no-op — the element and the length are untouched
```

The same statement therefore meant one thing where `(N-Store)` was asked and another where it
was not.

**The filed scope was wrong, and the matrix is what corrected it.**  `x = null` at a heap
target spells FIVE things, and three of them are not stores — so widening the ask to
`is_dbref`, which is what the cause as filed implied, would have reported correct code.
Measured at the lowering, all on both backends:

| spelling | lowers to | what it is |
|---|---|---|
| `c[key] = null`, keyed | `OpHashRemove` | `(Col-Remove)`'s by-key DELETE |
| `s.coll = null` | `OpClearVector` / `OpClearKeyed` | that field's CLEAR (@P307) |
| `n.next = null`, `reference<T>` field | `OpSetDbRef(…, sentinel)` | a store that LANDS |
| `s.rec = null` | `OpCopyRecord(null, …)` | the dropped write |
| `v[i] = null` | `OpCopyRecord(null, …)` | the no-op |

So the ask went to `Parser::copy_ref`, the one site that BUILDS the no-op: only the last two
arrive there, and it needs no container test, no keyed test and no pointer-marker test.  A gate
at the parse site needed all three and still got the POINTER field wrong, because #328's share
marker is not on the resolved target type by then —
`issues::issue_328_reference_field_pointer_semantics` is the cell that caught it, and it is the
reason the chokepoint moved.

**The VALUE is unchanged, and that is settled rather than deferred.**  A field declared
`rec: E` is DENSE: `synth_nullable_struct_fields` gives a discriminant only to the `?` the
author wrote, so a dense slot has nowhere to put "absent" and no store into it can make it hold
null.  The cure the message already named — *declare it `E?`* — is therefore the real one; the
`?` is what creates the room.

What did change is the CONSEQUENCE clause, and only at this position.  *"The slot holds null"*
is measured true for a scalar and for a record travelling as a HANDLE — a `null` argument
arrives null, a `return null` reads back null — and false here, where nothing is written.  The
four positions `D-Null-Heap` wired keep their wording to the byte, pinned by
`heap_nstore::the_four_shipped_positions_keep_their_wording`; the clause now comes from the
reporting position, which is the only thing that knows.

`Contract: settled` — the rule already named the field and the collection element among its
slots.  Guarded by six cells in `tests/heap_nstore.rs`, four of them negative controls, because
the three sanctioned spellings above are correct code that must stay silent.

### D-Null-Heap — OPENED AND CLOSED (2026-09-03, loft#1313): `(N-Store)` was enforced for the SCALARS only

`(N-Opt)` states the default for every type — *"Storage is non-null by default: a binding, field,
or `vector` element of type `τ` never holds `null` — `τ?` is the only way a slot admits it"* — and
`(N-Store)` gives the store rule with no type restriction in it.  DN1 landed that model, and the
enforcement it landed was gated on `Parser::is_non_null_scalar`.  So a bare `null` reaching a
non-null REFERENCE, collection or struct-enum was neither refused nor reported, at four positions
where the scalar twin warns:

```loft
struct It { v: integer }
fn f(k: integer) -> It { if k > 0 { return null; } It { v: 1 } }   // silent
fn g(k: integer) -> integer { if k > 0 { return null; } 1 }        // warned
```

A heap LOCAL was never part of it — `change_var` refuses `x: It = null` on its own — which is why
the gap read as deliberate: the shape a reader is most likely to try is the one already covered.

The falsifying program is the pair above: obeying the rule reports both, obeying the code reports
one.  The caller's half is what makes it a deviation rather than a missing nicety — `f(9)` hands
back a value the type says is non-null and the null then propagates, so a promise the type system
makes does not hold, with nothing said.

⚠ **The code stated the carve-out as though it were the rule.**  `is_non_null_scalar`'s doc read
*"Heap-nullable types (reference / vector / enum / keyed) are NOT here — they stay nullable"*, and
a reader checking the code against the model would find a coherent story there.  Two other homes
disagreed with it: `LOFT.md` § Types has always said *"you cannot store a `null` into a plain
`integer` / `text` / `Row`"* — `Row` being a struct — and `keys::callarg_nstore_enabled` described
its own split as *"a non-narrow scalar/heap param WARNS"*.  The heap half was specified in two
places and implemented in none.

Closed by extending the DN1 branch to `data::is_dbref` — the full handle set, called rather than
respelled, because its own doc records how a hand-written copy drifts short of the five keyed
collections.  The synthetic `__nullable<S>` is excluded, as it is in the DN3 branch: it is the
inline spelling of `S?` and is exactly as nullable as the `?` it stands for.

WARNING, never an error, and the tier is `(N-Store)`'s own Phase-1 split rather than a new call:
there is no narrow heap width to run out of room the way a `u8` does, and D-Null-Elem settled the
compatibility half — reporting where there was silence is a strict gain, refusing what a shipped
package already compiles is the break the freeze forbids.  Measured across the 1083-file script
corpus: **4 sites in 2 files**, all true positives, no exit code moved.  Opt out with
`LOFT_NO_HEAP_NSTORE`.

⚠ **One shape is excused, and it is a language gap rather than a carve-out.**  A `reference<T>`
field on a reference CYCLE back to its own struct has no nullable spelling at all — the `?` form
fails layout validation (loft#1316) — so a linked list's terminator must be a bare `null` in a
non-null slot.  The program really is in the state `(N-Store)` forbids and the author has nothing
else to write, so reporting it would name a cure that does not compile.  The exclusion is exact:
it is the CYCLE that excuses the field, not the `reference<…>` spelling and not the type being
recursive — an acyclic `reference<Leaf>` field warns, and a cyclic type's `-> Node` return warns.
It lifts when loft#1316 closes, and the guard cells flip with it.

Guarded by `tests/heap_nstore.rs`, which COUNTS notices on both backends — the four positions, the
four handle kinds including a keyed one, and the negative controls (`τ?` targets, the inline
`__nullable<S>`, a present value) that a `.loft` guard has no way to express, since it can declare
a notice it expects but not one that must not fire.

### D-Narrow-Res — OPENED AND CLOSED (2026-08-31, loft#1249): `(N-Reserve)` held for a packed slot and not a register one

`(N-Reserve)` says a reserved null is a value OF THE TYPE and is excluded from `τ?`'s range
everywhere the value can be.  loft#334 implemented that for a nullable byte-width FIELD (its
option 1: *"nullable byte ranges cap at 255 values"*) and the reservation stopped at the store
layer.  A local is a full i64 slot that never packs, so the sentinel survives there and dies on
the way to any packed position:

```loft
x: u8? = 255;            // 255      <- a value the type does not have
t.a    = 255;            // null
n = 250;  t.a = n + 5;   // null     <- ordinary in-range arithmetic, destroyed in silence
```

The last line is the sharp end: `250 + 5` is `255`, a legal `u8`.  Nothing overflows, nothing is
uncomputable, and the result reaches the field as `null` with no diagnostic.  Measured identical
on both backends; the register positions that keep 255 are the local, the parameter, the return,
the vector element read and the cast result, and the packed ones that answer null are the struct
field and the vector element write.

`IntegerSpec::usable_min/usable_max` already answers which specs spend an edge, precisely (only a
fixed 1- or 2-byte width whose range exactly fills it — an `i32?` has a spare code outside its
range and an `integer limit(0,255)?` widens to get one).  **What is missing is not the bound but
a way to ask "is this target a nullable SLOT?"**, and two cure directions were built, measured
and rejected on 2026-08-31:

1. **Bound the CAST** (`dn4_checked_cast` reads `usable_*`).  Rejected: `(mat as u8?) ?? 0` is
   the shipped idiom for *"narrow this, or 0"*, it never keeps a `u8?` at all, and the bound
   turned its legal `255` into the default.  `hex_field` 0.1.0 asserts exactly that value and
   failed `scripts/revalidate_libs_local.sh` — the gate `make ci` cannot give, because a language
   change that retro-breaks a shipped library is invisible to every branch gate.  A lexical peek
   at the cast cannot separate the two either: a parenthesised `(e as u8?) ?? d` reads `)` at
   that moment.
2. **Bound the STORE SEAM** (`declared_range` answers for a nullable narrow alias, so every
   store guard applies the usable range).  Rejected for a sharper reason: **`Type::Optional` at
   a write target means two different things.**  An element write `e.m[i] = …` on a
   `vector<u8>` — a NON-nullable element — presents its target as `integer(0,255)?`, because
   `(N-Domain)` makes an index expression nullable for the miss.  The guard then bounded a
   non-null slot by the usable range and wrote the null sentinel into it, which the store
   flattened to `0`: `hex_field`'s `edge_set_mat` stored `255` as `0` (measured
   `OpRangeDefault(…, 0, 254, i64::MIN)` emitted for a `vector<u8>` element).

**Closed by resolving the index-write conflation first**, which turned out to be one predicate
rather than the larger change it was feared to be.  `expressions::target_holds_null` is the one
home for *"does the slot this store TARGETS hold null?"*: `parent_tp` is what the place is read
out of, so a collection there carries the DECLARED element type and `Type::content` unwraps it —
`vector<u8>` says no, `vector<u8?>` says yes, and anything that is not a collection falls back to
the target's own wrapper.  The three range-guard seams take the answer as a REQUIRED PARAMETER
rather than re-deriving it, which is what made the compiler enumerate them (there are exactly
three, and a grep for the diagnostic text would have found fewer).

With that in place cure 2 became correct, and it is the fix: `declared_range` answers for a
nullable narrow alias — the compile-time narrowing refusal does not cover one, because
`(I-Narrow-Opt)` makes that narrowing implicit and checked — and bounds it by `usable_*`.

Both rejected cures are now CELLS in the guard rather than only prose here, because each looked
obviously right: `test_a_cast_is_not_a_slot` fails on the build where cure 1 was live, and
`test_a_non_null_element_is_not_a_nullable_slot` on the build where cure 2 was.  Guard
`tests/scripts/1249-a-nullable-narrow-sentinel-is-not-a-value.loft`, falsified at 95a7f949 on
both backends.

⚠ **`(N-Store)` is deliberately NOT changed.** That seam's `f_type` feeds `n_store_violation`
too, and passing it the peeled answer would make storing a nullable into a `vector<u8>` element
a violation where today it silently is not — which is `(N-Dense)` working rather than a
regression, but it is a second behaviour change and it wants its own measurement and its own
guard.  The range guards take the new fact; the null-store check still reads the target type.

### D-Narrow-Asgn — OPENED AND CLOSED (2026-08-31, loft#1246): a narrowing store into a NULLABLE narrow LOCAL was neither refused nor checked

`(I-Narrow)` says a narrowing store needs an explicit `as` or a literal that plainly fits, and
the 2026-07-10 refinement below completes it for a NULLABLE narrow target: no `as`, but a
CHECKED narrowing — the value when it fits, `null` when it does not.  An annotated local
assignment got neither:

```loft
p = 250;
d: u8? = p + 10;                 // kept 260 — outside the range its own type declares
s.un = p + 10;   f(p + 10);      // null, correctly — field and argument
S { un: p + 10 };                // null, correctly — struct literal
```

The refinement lives in `convert`, which the struct literal, the argument and the return all
reach.  The ASSIGNMENT seam never reaches `convert` for this pair, because `integer` and
`integer(0,255)?` are `is_equal` — so it runs its own narrowing test instead, and that test is a
REFUSAL with no checked-cast arm which additionally did not peel the `Optional` wrapper.  It
therefore refused nothing and checked nothing, and the value landed raw.

`implicit_checked_narrow` (`parser/mod.rs`) is now the one home for the refinement, asked by
`convert` and by the assignment seam.  The rule gained the clause it was missing —
`(I-Narrow-Opt)` in types.md — because a two-clause `(I-Narrow)` cannot express a target whose
type already says what an out-of-range value becomes, which is why this register could read
`OPEN: 0` while the defect stood.

⚠ **The refinement's own guard was green throughout.**
`tests/scripts/25-nullable-narrow-implicit-checked.loft` has seven cells over four functions,
two source types and in-range/out-of-range arms — and every cell is a RETURN, so all seven enter
through `convert`.  GUARDS.md § How a guard reads green carries the general shape ("every cell
reaches the same SEAM").  The replacement guard,
`tests/scripts/1246-a-nullable-narrow-slot-answers-null.loft`, is written as seams first and
values second: local, field, struct literal, vector element, argument, return and compound, each
in both spellings of the range.

### D-Null-Elem — OPENED AND CLOSED (2026-08-31, loft#1232): a nullable stored into a collection LITERAL's element, in silence

`(N-Dense)` says a `vector<τ>`'s elements are non-null unless the type is written `vector<τ?>`,
and `(N-Store)` says storing `e:τ?` into a `τ` slot is at least a warning where the null is
representable.  Both were enforced at the scalar seam and at the append seam, and neither was
asked of the elements INSIDE a literal — so the same value, one spelling over, went unremarked:

```loft
n: integer? = null;
x: integer = n;                  // ERROR, correctly
d.c += n;                        // ERROR, correctly (loft#1223's bracket rule)
v: vector<integer> = [n];        // compiled, silent — and v[0] reads null
d.c = [n];   e = D { c: [n] };   // the same, silent
```

This is the entry the register's own bound predicted.  `types.md`'s `OPEN: 0` was earned by a
verification the ROADMAP row states the limit of in the same breath — *"verified both backends —
for the DIRECT store, which is the bound on that verification"* — and a literal's element is not
a direct store.  `D-Null-Join` came from outside that same bound at a branch join; this one comes
from inside a collection literal.  An `OPEN: 0` is only as strong as the shape its oracle reached.

It also mattered more than an ordinary gap, because it was the cure the language NAMES: loft#1223
refuses `d.c += n` and tells the reader to write `d.c += [n]`, which was the un-diagnosed
spelling — so closing that issue moved the reader from warned to silent along the path the
diagnostic recommends.

Closed by asking `n_store_violation` — the shared home the other two seams already use — of each
element as it is parsed against the declared element type, which reaches all three spellings above
and their nested forms at one point.

⚠ **The tier is held to WARNING here, including at the narrow widths the shared split escalates.**
That escalation is right about the SLOT — a `u8` spends all 256 values on real values, so a null
there has no room — and wrong about the moment: this seam was silent until now, so refusing at it
retro-breaks code that compiles today.  Measured on the whole registry: `assets 0.2.0` writes
`bp += [0 as u8?]`, whose value is never actually null, and the gate went from 42 pass to a
COMPILE-BREAK.  Reporting where there was silence is a strict gain; refusing what a shipped
package relies on is a break the freeze forbids, and raising the tier later is COMPATIBILITY.md's
process rather than this seam's call.

### D-Chk-Yield — OPENED AND CLOSED (2026-08-28, loft#1130): `yield` carried no expected type

`(T-Chk)` is the **single** carrier of an expected type, *"pushed structurally into
sub-expressions"*, and a `yield` hands a value out of the function against a type the
declaration already names — the same position `return` occupies. It was not a push site at
all: `yield e` parsed `e` in synthesis mode only.

A collection literal cannot synthesise its KIND — `[K { … }]` is a `vector<K>` wherever it
stands, and `(T-Chk-Vec)` is what says otherwise — so a keyed literal yielded from a generator
was BUILT as a vector:

```loft
fn g() -> iterator<hash<A[k]>> { yield [A { k: 1, v: 11 }, A { k: 2, v: 22 }]; }
//  len 1, and c[1] misses — both backends, no diagnostic
fn ok() -> iterator<hash<A[k]>> { a: hash<A[k]> = [ … ]; yield a; }   // len 2, c[1] found
```

The bound route was correct because a declared local reads its destination from `var_tp` and
never needed the channel. `hash` lost length AND lookup, `index` / `trie` / `spatial` stuck at
length 1, `sorted` and `vector` were correct — five kinds, one missing push site.

`yield` and the block tail / `return` now share ONE admission list
(`Parser::seed_leaving_value_hint`): they are two spellings of one act against one declared
type. The census of this channel's remaining push sites — ten of them, carrying six different
admission lists, none admitting `Type::Tuple` — is [QUALITY-history.md § B6t](../QUALITY-history.md); the
general rule LOFT.md already states is *the expected type wherever there is one*.

⚠ **The incident-shaped patch that stood in for the rule was one screen above the fix.** The
same `yield` branch already rewrote a bare `Value::Int(d_nr)` into a full fn-ref when the
element type was `Function` (@P328) — a per-type repair of exactly the missing channel, which
is what a missing `(T-Chk)` push site looks like from inside one bug.

Guard: `tests/scripts/1130-a-yielded-collection-literal-takes-its-declared-kind.loft`, all six
kinds plus the bound route, each cell with its own element struct.

### D-Var-Join — OPENED AND CLOSED (2026-08-27, loft#1117): an `if` whose arms are two variants of one enum was refused

`(C-Var)` licenses `Reference(S) ⤳ Enum(E)` for `S ∈ variants(E)` and licenses NOTHING between
two variants; `(T-Chk-Var)` checks each variant against the enum. So two arms that are two
variants join to `E`, and asking whether one converts to its SIBLING is a question the relation
does not answer. `parse_if` asked exactly that — it handed the THEN arm's type down as the else
arm's expected type — and refused a legal program, on both backends, at parse time:

```loft
enum E { A { x: integer = 1 }, B { x: integer = 2 } }

fn pick_if(c: boolean)    -> E { if c { E::A { x: 7 } } else { E::B { x: 9 } } }   // ERROR: expected A, got B on else
fn pick_match(k: integer) -> E { match k { 0 => E::A { x: 7 }, _ => E::B { x: 9 } } }   // fine
fn pick_return(c: boolean)-> E { if c { return E::A { x: 7 }; } E::B { x: 9 } }         // fine
```

Two spellings of one program disagreeing is what made this a deviation rather than a design
choice — and `match` lowers to the very node `if` builds, so the accepting spelling was already
running the code the refused one could not reach.

**Fixed as a JOIN, not as a conversion.** `parse_block` no longer asks `convert` about a sibling
arm (`sibling_variants`) and lets it keep its OWN type; `parse_if` then joins two differing
variants to their enum. Both halves are load-bearing:

* without the first, the refusal stands;
* without the second, `v: A = if c { E::A { … } } else { E::B { … } }` is ACCEPTED and a slot
  declared as one variant holds another, read at this variant's offsets — loft#980's class,
  silent. The widening makes that declaration fail where it should, naming the real conflict
  (*"cannot change type from A to E"*) instead of blaming the else arm.

Two arms of the SAME variant widen nothing, so a variant-typed destination stays legal for
them — that was verified against a release control, having been broken by an earlier attempt
that widened unconditionally. Guarded by
`tests/scripts/1117-an-if-joins-two-variants-to-their-enum.loft` (falsified at `cd263f7c`:
interpret exit 1 → 0, native exit 1 → 0). The four refusals that must SURVIVE — a sibling
enum's variant, an unrelated struct, an integer, and the variant-typed destination — were each
re-measured against that control.

### D-Null-Join — OPENED AND CLOSED (2026-08-26, loft#1103): a nullable in a LATER branch arm stored into a non-null slot in silence

`(N-Store)` says storing `e:τ?` into a `τ` slot without discharge is REJECTED, and `(N-Decl)`
says a declared slot is a commitment that FORBIDS a later nullable write.  Four spellings of that
store were refused.  The same store written in a later branch ARM compiled, and the non-null slot
held `null` — both backends, so a rule/code divergence and not a backend split:

```loft
fn maybe(k: integer) -> integer? { if k > 5 { 7 } else { null } }

x: integer = maybe(k);                            // ERROR, correctly
x: integer = if k == 9 { maybe(k) } else { 1 };   // ERROR, correctly  (the FIRST arm)
x: integer = if k == 9 { 1 } else { maybe(k) };   // compiles; x is `integer` and holds null
```

The variable is genuinely non-null — the IR reads `x(1):integer` and `LOFT_VAR_TABLE` agrees, so
nothing was widened to `integer?` behind the declaration. The same hole reaches a RETURN
(`fn c(k) -> integer { if k == 9 { 1 } else { maybe(k) } }` returns null), a non-null FIELD, a
`text` slot, a struct slot, and — worst — a NARROW width, where this rule keeps a hard error
precisely because *"the null would collide with a real value"*: `x: u8 = if k == 9 { 1 as u8 }
else { maybe8(k) }` compiles and `x` is null.

**Mechanism.** The FIRST arm's type becomes the join type, and a later arm is checked against the
first arm's type rather than against the declaration, so its `Optional` is dropped instead of
reported (`parse_if` hands the else arm the then arm's type as its expected type — the loft#978
site in `src/parser/control.rs`). `(N-Join)` — *"made OPTIONAL iff some `τᵢ` is optional"* — is
the rule the join is missing; the declared destination should then take `(N-Decl)` + `(N-Store)`
against that join, exactly as the direct spelling does.

A LITERAL null in a later arm IS caught, by a different mechanism — the DN1 null-arm walkers
match the `OpConv*FromNull` spelling. So this is the same shape QUALITY-history.md § B6g names: one
notion with two spellings, and only one of them is looked for. Nothing asks about a
nullable-TYPED value at a join.

⚠ **This entry is why the `OPEN: 0` above needed re-measuring.** The register recorded `(N-Store)`
as *"CLOSED + verified both backends"*; that zero is bounded by its oracle, and every cell here is
a BRANCH JOIN while the verified spelling is the direct one.

Tracked as **loft#1103**. Workaround (verified both backends): discharge inside the arm
(`else { maybe(k) ?? 0 }`) or at the join (`(if … else …) ?? 0`).

x: integer = if k == 9 { 1 } else { maybe(k) };   // compiled; x was `integer` and held null
```

The variable was genuinely non-null — the IR read `x(1):integer` and `LOFT_VAR_TABLE` agreed, so
nothing had been widened behind the declaration.  The same hole reached a RETURN, a non-null
FIELD, a `text` slot, a struct slot, and — worst — a NARROW width, where this rule keeps a hard
error precisely because *"the null would collide with a real value"*: `x: u8 = if k == 9 { 1 as
u8 } else { maybe8(k) }` compiled and `x` was null, indistinguishable from `255`.

**Mechanism.** The FIRST arm's type became the join type.  `merge_dependencies` is
`a.joined_deps(b)` — it keeps `a`'s shape and merges only the borrow set — and `a` is the then
arm, so a later arm's `Optional` was dropped rather than joined.  The arm's own type was erased
before that even: `parse_if` hands the else arm the THEN arm's type as its expected type, and
`block_result` returns that expected type (the loft#978 site).

Closed by `(N-Join)` — *"made OPTIONAL iff some `τᵢ` is optional"* — in the two places a join is
formed: the else arm keeps its own nullability through `block_result`, and `parse_if` /
`parse_match` widen the join when any arm is optional.  **The widening is all that was added; the
REPORTING is unchanged**, which is what keeps the three site verdicts distinct instead of
flattening them: a declared slot is refused by `(N-Decl)`, a return and a field WARN and hold the
null by `(N-Store)`, and a narrow width is refused wherever it appears.  Each now answers in a
branch exactly as it already answered for the direct spelling.

**One home, six producers.**  A `match` reaches the join through six arm sites — the ordinary
arm, the wildcard, the struct and enum arms, the vector-match arm — all spelling
`result_type.joined_deps(&self.arm_join_type(…))`.  They fold through one `join_arm_into` rather
than six copies of the rule, because a per-site fix answers the same question six times and
drifts at the first one anybody forgets.  An `else if` CHAIN needed carrying separately: it keeps
its SHAPE out of the join deliberately (loft#936), so only what it BORROWED was being read.

⚠ **This entry is why the `OPEN: 0` above needed re-measuring.**  The register recorded
`(N-Store)` as *"CLOSED + verified both backends"*; that zero was bounded by its oracle, and every
cell here is a BRANCH JOIN while the verified spelling is the direct one.  **A zero is only as
strong as the spellings its oracle contains.**

⚠ **The class, and it is the third instance in a week: one notion, two spellings, one looked
for.**  A `null` LITERAL in a later arm was ALWAYS caught — by the DN1 walkers, which match the
`OpConv*FromNull` node a literal lowers to.  A nullable-TYPED value produces no null-shaped node
at all, so nothing asked about it.  The blindness cannot be found from the symptom: searching for
the spelling you DO match returns every site that gets it right, and the sites that get it wrong
contain nothing to search for.  Asking the TYPE is the spelling-free form of the question.  See
`IMPLEMENTATIONS.md` § *One notion, how many SPELLINGS?* for the other two instances.

**Measured.**  Thirteen cells, both backends identical, covering every spelling in the issue plus
the narrow width.  Emitted IR over the corpus: **3 of 970** programs change, two of them these
guards.  The third is `25-nullable-branch-join.loft`, and it is worth reading — the file exists
to assert THIS RULE (*"@PLN25 — branch-join widening: `integer ⊔ integer? = integer?`"*), and it
was GREEN while its own `pick_local` inferred `x` as plain `integer`.  The assertions passed
because the runtime answered null anyway; the emitted type disagreed with the title of the file
testing it.  **A green gate is not coverage** — it checked the value and never the type it was
named for, and the fix is what brings the IR to what the file already claimed.  Guards:
`tests/scripts/1103-a-nullable-in-a-later-branch-arm.loft` (the value half — the join really is
`τ?`, the null arrives, and the non-null joins beside it did not move) and
`tests/scripts/1103b-a-nullable-branch-arm-refused.loft` (the refusals — six `@EXPECT_ERROR`s,
none of which fire on a control binary built at `9c1a0e4e`).  **Neither half implies the other,
and the value half alone is VACUOUS**: it declares the nullable type the rules require, and a
join that wrongly types non-null still stores into a nullable slot happily, so it passes on the
pre-fix binary.  Only the refusal file scores the change.

### D-Opt-Zero — CLOSED (2026-08-24): a nullable field defaults to its BASE ZERO, not null

`(D-Opt)` says `construct_default(τ?) = null` — *"an optional's default IS null"* — and
`(D-Rec)` composes it: a field with no `= expr` takes `construct_default` of its own type.
The code does not do that for a scalar or text base. Measured on **both** backends, so this
is a rule/code divergence and not a backend split:

```loft
struct S { a: integer?, b: text?, c: boolean?, d: Colour? }
s = S {};                       // a=0   b=''   c=false   d=null
```

Only the enum base answers `null`. `data::to_default` handles it as
`Type::Optional(inner) => to_default(inner, data)` and states the choice outright: base-zero
is *"the settled design call"* (@PLN25), because a bare `null` would fall to the `_` arm and
render as native unit into a scalar slot (E0308) — and the field is still writable to null
through an explicit `= null`.

**So the disagreement is real and deliberate, which is the awkward part**: a decision was
taken in code and `(D-Opt)` was never updated to match, exactly as
[`formal/layout.md`'s `L-Tuple`] was left naming a function a rename had removed. The
doctrine says the CODE changes to match the rules, so this stays OPEN rather than being
written away — but the decision behind it has a stated reason and a plan number, and the
resolution is a design call the owner should make, not a silent edit in either direction:

- amend `(D-Opt)` to state base-zero for a scalar/text base and `null` for the rest, or
- change `to_default` and give the E0308 problem a different answer.

**Resolved by the owner (2026-08-24): the rule stands — a nullable value in a field is null
at the start.** `to_default`'s `Optional` arm now builds the base type's null SENTINEL
through the new `data::to_null`, which is the `OpConv…FromNull` op for that base. Those ops
already existed and already produced the same sentinels as the runtime's
`Stores::set_default_value_nullable` (`i64::MIN`, `255` for the tri-state boolean,
`char::from(0)`), so the two paths now agree instead of contradicting each other.

The E0308 objection was real but misdirected: a bare `Value::Null` does render as native
unit, and the answer is the TYPED null op rather than the base's zero. Nothing needed
inventing.

**What the old behaviour rested on, checked:** the code cited *"an omitted field gets the
zero value for its type (LOFT.md § constructors; 06-structs.loft locks it)"*. `LOFT.md` has
no such section and does not contain that sentence, and `06-structs.loft` declares no
nullable field at all. The only thing actually locking it was one half of
`issues::issue_332_nullable_narrow_field_null_roundtrip`, written on the strength of those
two citations; it now asserts the rule instead, across omitted / assigned / re-nulled for
`i16?`, `i32?` and `integer?`.

**Blast radius: one test in 4 434.** Verified on both backends for `i8? u8? i16? u16? i32?
integer? float? single? boolean? text? character?` and an enum, a reference and a vector —
and the non-nullable defaults (`0`, `false`, `""`) are unchanged.

⚠ `character?` is the one base whose null is indistinguishable from its zero: both are
`char::from(0)`, which is also what the runtime writes (its content-type-6 arm ignores
`nullable`). Consistent between the two paths, so not a divergence — but it means
`character?` cannot represent absence distinctly. Separate question, not opened here.

Found by citing: the `D-*` family has one home (`data::to_default` + `Data::has_default`),
and writing *"enforces `@FR-D-Opt`"* on it is what forced the comparison.

### DN-SE-inline — CLOSED (2026-08-22): a nullable struct-enum in INLINE storage
The representation rule above derives `τ?`'s null from `τ`'s storage, and gives a reference the
out-of-band `nullref`. A struct-enum is carried as a `DbRef`, so `Shape?` takes the reference
sentinel — which loft#1065 measured it did NOT: several sites answered "what is this type's
null" without telling a struct-enum from a value enum, and it took the value enum's `255`
BYTE into a handle slot. `--interpret` then read the slot back as a live ref (its own
store-lifetime guard fired) and `--native` refused to compile (`non-primitive cast: u8 as
DbRef`). **Closed for a LOCAL, a parameter and a return** by discriminating `Enum(_, true, _)`
from `Enum(_, false, _)` at each site, plus `base()` where a shape was read without peeling
`Optional` (six sites; guard `tests/scripts/1065-nullable-struct-enum.loft`).

**INLINE storage closed too** (loft#1071) — a struct FIELD and a `vector<Shape?>` element are
a four-byte record pointer inside the holder's record, which cannot hold the twelve-byte
sentinel at all. The rule says the representation follows the base type's STORAGE, and it does
here: absence is pointer `0`, which is what the field prime already writes. Three sites had to
agree on that one word — the construction (`Box { s: null }`), the assignment (`b.s = null`,
which had been silently a no-op), and the test, which must read the stored WORD because
`OpGetField` answers a sub-reference whose own record is the HOLDER's and so is never null. No
`__nullable<…>` tag was needed: a record pointer already has an in-band absent value, exactly
as a narrow scalar does, so the element rides the `Optional` marker like a scalar element.

Iteration closed with it: `for e in v { e == null }` binds a sub-reference to the element
SLOT, so it reads that slot's word. Which of the two a `Var` is turns on what it VIEWS,
followed through the DEP CHAIN — a loop variable's own dep names itself, and only its
declaration's dep names the vector, so reading the first link answers no.

This entry is also the answer to the "OPEN: 0" line above having been too strong: the rule was
written, and the code disagreed with it for a whole type former, in both directions at once.

### DN1 — CLOSED (2026-07-02): scalar / field storage is non-null by default
`(N-Dense)` now holds for scalars + struct fields, not just vector elements: a plain
`integer`/`text`/`bool`/`float`/`char` field/local/return is NON-null by default (nullability
rides `?`/`Optional`), and `not null` is redundant — stripped from in-tree source; the parser
still ACCEPTS it for backward compat pending the registry republish (task #4), and (since #546)
WARNS "deprecated and has no effect" rather than staying silent. `a.x = null`
on `x: integer` is now a compile error ("declare it `integer?`"). Landed: the DN1 default flip
(PR #471, default-on; `LOFT_PLN25_OFF` opts the whole model out) + the F2 field-attribute flip for
ALL scalars + the `not null` source strip + parser no-op. Enforced by `(N-Store)`/`(N-Decl)` at
return / field / typed-store / index / call-argument sites — the last (a nullable `τ?` passed
into a non-null PARAMETER) closed 2026-07-16 (`callarg_nstore_enabled`, `src/keys.rs`; the
identical warn/error split at the `process_call_args` chokepoint, opt-out
`LOFT_NO_CALLARG_NSTORE`). Both backends.

### DN2 — CLOSED (2026-07-02): no implicit `S? ⤳ S` unwrap
The implicit `Enum(__nullable<S>) ⤳ Reference(S)` (and scalar `τ? ⤳ τ`) unwrap is gone — a `τ?`
cannot reach a `τ` slot without a discharge. Verified: `x: S? = …; y: S = x` (no `?`) is a compile
error ("cannot change type from S to S?"); `?? default` / `match` are the only elims
(`(N-Coal)`/`(N-Match)`). Closed by the `(N-Store)` / `change_var` teeth (DN1) + the DN5 `as`-cast
closure. Both backends.

### DN3 — CLOSED (2026-07-02): fit-failing ops type `τ?`  ·  overflow-arith = decided edge
A fit-failing op now TYPES its result `τ?` (the runtime already nulled per `E-Uncomp`/C80), so an
un-discharged result stored into non-null storage is a compile error — the developer must guard,
`?? d`, or declare the target `τ?`. **DONE:** integer `/` and `%` (the division root-cause fix — the
`handle_operator` arithmetic-branch wrap); `v[i]` / `s[i]` indexing (the index flip, default-on,
with const / iter-var / `if i < len` guard fit-proofs); and **text→numeric parse** (`(N-Parse)`:
`s as integer` / `as float` / `as single` now type `integer?` / `float?` / `single?` — a bad parse
is a reachable fault, exactly like `÷0` and OOB). The `as` handler in `parser/operators.rs` wraps a
`Text`-source numeric cast in `Optional`; discharge with `?? d`, `as τ?`, or `match`. In-tree
consumers migrated (lib/lexer number/escape accessors already return `τ?`; the test scripts +
audience-demo servers `?? 0`). The runtime "may produce null" warnings are RETIRED (the type +
`(N-Store)` is the enforcement). Regressions: `25-division-nullable.loft`, `25-index-nullable.loft`,
`inc17_text_to_integer_requires_as` + `102` reject twins; both backends. **DECIDED EDGE (not a
deviation) — overflow arithmetic** `a*b`/`a+b`/`a-b` stays NON-null: overflow → the null sentinel +
continue (C80, no trap), NOT `τ?`. The fault is extraordinary (operands ~3×10⁹) while the op is
ubiquitous, so forcing discharge on all arithmetic is disproportionate + (given no traps) would
block a game over a fault its player never hits —
[DESIGN_DECISIONS C85](../DESIGN_DECISIONS_VALUES.md#c85--overflow-arithmetic-types-non-null-the-game-keeps-running-dont-force-integer-on-every--). Range-tracking keeps provably-fit multiplies exact.

### DN4 — CLOSED (2026-07-02, F5 cutover): `as` to a narrower type enforces the range
`400 as u8` was UB (the cast asserted the *type* but left an out-of-range value in a `u8`
slot). Now per `(N-Cast)`/`(N-Cast?)`: **`as u8` requires a PROVABLE fit** (`400 as u8`, and
`b: integer; b as u8`, are **compile errors** — "use `as u8?`"), and **`as u8?` is the CHECKED
cast** (value or `null`, never out-of-range) — a pure parse-time range-guard desugar (`OpLeInt`
+ `if` + `OpConvIntFromNull`, no new runtime op; the guard types as a full nullable integer so
the `i64::MIN` sentinel keeps full width). Range-tracking makes masked values provably-fit, so
`(x & 255) as u8` / `(non-neg) % c as u8` need no `?`. **Enforcement is UNCONDITIONAL** — the
interim `LOFT_NO_DN4` opt-out (which reverted to the silent width-tag) is RETIRED, so DN4 is
consistent with its nullness sibling DN5. Validated both backends (`tests/dn4_cast.rs`: the
value matrix, the compile-error cases, and the opt-out-retired guard).

### DN5 — CLOSED (2026-07-02, F3): `as τ` no longer launders `null` / `τ?` into a non-null scalar
`null as integer`, `x:integer? as integer`, `(a/b) as integer`, `v[i] as integer` used to
type-check and store `null` into a non-null slot (bypassing `(N-Store)`). Now a `Null`/`Optional`
source cast to a non-null scalar (no `?`) is a **compile error** directing to `as τ?` (checked →
`null`) or `?? d`; `as τ?` types `Optional<τ>` so the laundering stays closed downstream
(`z: τ = (e as τ?)` still requires a discharge). This is the **nullness dimension of DN4** — the
two are ONE domain-containment fit-check at the `as` chokepoint (`operators.rs`): a scalar
target's domain is its value RANGE × nullness, and `null` is the reserved out-of-domain element
(so `is_narrowing_int` peels `Optional` — a nullable source no longer slips past the range
check). A `null as S` heap ref stays legal (`is_non_null_scalar` is scalar-only). Regression:
`tests/scripts/102-expected-errors.loft` twins + the `25-*-nullable.loft` accept paths.

### Refinement (2026-07-10): implicit checked narrowing into a NULLABLE narrow target
DN4's `as`-required rule is for a **non-null** narrow target. Coercing an integer / `integer?`
into a **nullable** narrow target (`Optional<narrow>`, e.g. `u8?`) needs **no explicit `as`**:
`convert` routes it through the same `dn4_checked_cast` range-guard (in-range → the value,
out-of-range → `null`). This is sound without ceremony because the target is nullable — an
out-of-range value becomes a VISIBLE `null`, never the silent truncation that `as u8` into a
non-null slot would be. Only nullable narrow targets are affected; a non-null `u8` still
requires an explicit `as`. Guard: `tests/scripts/25-nullable-narrow-implicit-checked.loft`
(both backends).

### DN6 — CLOSED (2026-07-02, F4): the inferred `null`-join widens to `τ?` instead of rejecting
Per `(N-Join)` an INFERRED `a = null; a = 5` (no annotation) now infers `a : integer?` — the join
of `null` and the scalar — via `change_var_type` (`variables/mod.rs`). `var_tp == null` is
inherently the inferred case (a variable cannot be annotated `null`), so the widen never overrides
an explicit non-null contract: annotated `a: integer = null` still rejects, as does the reverse
`a = 5; a = null`; a widened `τ?` into a non-null slot still requires a discharge. **Scoped to
INLINE scalars** (Integer/Boolean/Float/Single/Character): the retroactive widen reuses the slot
the first `= null` allocated, sound only when Null and `τ?` share it (an in-slot sentinel).
`Text` — the one heap-backed scalar — is EXCLUDED (its Null slot is not a text?-heap slot;
widening it underflowed `fn_return`'s discard / native E0308), so a text null-start must annotate
`s: text? = null`. Regression: `tests/scripts/25-null-join.loft` + reject twins in
`102-expected-errors.loft`.

### DN3-Float — CLOSED (2026-07-11, shipped default-on, @PLN102 #559): float/single `/`, `%`, and domain-partial functions type `τ?`

> This is the **float instance** of the general **§ Null-flow laws** (N-Domain / N-Prop /
> N-Cast / N-Store) above — those laws hold for every type; this entry records the float
> classification (which ops) and the conversion set.

DN3 typed integer `/`/`%`, indexing, and text-parse `τ?`, but its division wrap is **gated
to `Type::Integer`** (`src/parser/operators.rs:2300`), so float/single `/`/`%` — and the
domain-partial float functions — keep a **non-null** `float`/`single` return while
producing null (a reserved NaN) at runtime. The type therefore **lies** about a null the
integer side already surfaces: `f.g = 1.0 / b` and `f.g = ln(-1.0)` store null straight
into a non-null `float` field with no diagnostic, where the integer `s.f = 10 / y`
equivalent is a compile error. Return types freeze at contract 1, so the honest signature
is chosen now (pre-freeze-only).

**Rule.** A float/single op types its result `τ?` **iff it can yield the reserved NaN-null
from an input a normal program reaches** — the DN3 boundary read across to floats, with
[C85](../DESIGN_DECISIONS_VALUES.md#c85--overflow-arithmetic-types-non-null-the-game-keeps-running-dont-force-integer-on-every--)
(overflow → non-null) as its complement:

- **`τ?`:** `/` and `%` (÷0 — mirror of integer `/`; the existing `divisor_provably_nonzero`
  proof keeps `x / 2.0` and a guarded `if b != 0.0` non-null); `sqrt` (arg `< 0`);
  `ln`/`log`/`log2`/`log10` (arg `≤ 0`); `asin`/`acos` (arg outside `[-1, 1]`); `pow` (base
  `< 0` ∧ fractional exp — resolved 2026-07-11: a genuine domain error, folded in). A
  **provably-in-domain argument blocks the `τ?`** — not just a literal constant (`sqrt(4.0)`,
  `sqrt(PI)`, `ln(2.0)`) but any expression the sign/interval lattice proves in-domain
  (`sqrt(dx*dx + dy*dy)`, `sqrt(max(x, 0.01))`, `asin(clamp(e, -1, 1))` all stay non-null,
  @PLN102 #581, 2026-07-16 — [design](../plans/102-stability-contract/soften-nullflow-discharge.md));
  an argument that is **not** provably in-domain is `τ?` (bare `sqrt(x)` on an unknown-sign `x`
  stays nullable). A constant *out-of-domain* argument (`sqrt(-1.0)`) may warn
  *"always null"*, the parallel of the existing constant-`/0` warning.
- **Non-null (C85-style decided edge):** `sin`/`cos`/`tan`/`atan`/`atan2`/`exp`/`abs`/
  `ceil`/`floor`/`round` — a *finite* argument is always finite; NaN arises only from an
  `±inf` argument, itself reachable only through a C85 overflow, so forcing `?` on the
  ubiquitous op is disproportionate.

**Runtime is UNCHANGED** — null + continue (C80); **no runtime error is added** (owner:
*"a null is fine, errors never"*). Two type-level rules do the work, refining DN3 **across
the shipped integer model** (@PLN25) too — the runtime *already* propagates the null
sentinel through arithmetic (`n+5` / `n*5` / `5-n` / `abs(n)` on a null `n` all stay null,
both backends), so the type only has to stop lying:

- **(N-Prop) — nullability propagates through arithmetic.** An arithmetic op with any
  nullable operand yields a nullable result: `integer? + integer → integer?`,
  `float - float? → float?` (either operand position). Already true for `text? + text`; now
  uniform over `integer` / `float` / `single`. **C85 is untouched** — non-null × non-null
  stays non-null (overflow → sentinel silently, no `?` forced); propagation fires only when
  an operand is *already* nullable, so the two compose (non-null arithmetic stays non-null;
  a null, once present, stays visible).
- **(N-Warn) — a nullable into a non-null slot is a WARNING, not an error — EXCEPT narrow
  width types.** The relaxation applies where the target's null pattern is available in its
  *non-null* form: `integer` (reserves `i64::MIN` even non-null — a stored null reads back as
  null), `float`/`single` (NaN), `text` (out-of-band). There the program still compiles + runs
  (the slot holds null) and the warning nudges toward `?? d` / `match` — a warning because a
  hard error would break every existing non-null store of a `sqrt` / float-`/` result
  (compatibility) and Goal F reserves warnings as the programmer-billing channel; this RELAXES
  DN3's current integer *hard error* to a warning. **Narrow width integers**
  (`u8`/`i8`/`u16`/`i16`/`i32`/`u32`) spend their whole width on real values (a non-null `u8`
  holds `255`), so they have no spare bit-pattern for null — a null there is unrepresentable
  and would silently corrupt to a real value. They **keep the hard error** (`?? d`, or widen
  the target to `u8?`). Split principle: *warn iff the null is representable-and-observable in
  the non-null slot* — the in-band-sentinel property C85 already relies on. Narrow stores
  already error (DN1/DN4/DN5), so keeping them costs zero compatibility.

Together, a `float?` rides through inference, arithmetic, comparison, interpolation, and
calls, and is nudged only at a non-null STORAGE site (field / return / explicitly-typed
local / call-argument). Mechanism (shipped): the
`div_nullable` gate was extended to `Float`/`Single` (the nullable runtime peers
`OpDiv{Float,Single}Nullable` + the `??`-swap, phase 4f.5); the domain-partial function return
types in `default/01_code.loft` are `float?`/`single?`. **Shipped, default-on 2026-07-11
(@PLN102 #559), verified both backends; the pow / domain-proving forks and the conversion set
are recorded in
[../plans/102-stability-contract/float-null-domain-typing.md](../plans/102-stability-contract/float-null-domain-typing.md).**

### D2 — CLOSED by reconciliation (2026-06-24): `integer` = i64 is a *user-visible* contract met by a *compact* internal encoding

D2 was framed as a deviation to *remove* by widening the IR (`Value::Int` → i64) so the default
integer is "i64 end-to-end." That framing is **declined** — see
[DESIGN_DECISIONS.md C83](../DESIGN_DECISIONS_OWNERSHIP.md#c83--the-internal-representation-follows-the-user-visible-contract-never-widen-storage-for-implementation-convenience).
The reconciliation:

- **The user-visible contract is met.** `integer` *is* i64 everywhere a user can observe it — a
  boundary matrix (graduated to `tests/scripts/438-integer-i64-user-visible.loft`) confirms a
  value above i32 range survives arithmetic (`* / % -`), bare literals, struct fields, vector
  elements, fn args/returns, comparison, negation, tuples, and field mutation, **identically on
  the interpreter and `--native`**. The runtime computes on `i64` throughout.
- **The internal model is *supposed* to be compact.** `Value::Int(i32)`/`Value::Long(i64)` is a
  deliberate value-size encoding (i32 for the small-value majority, i64 when needed), and
  `forced_size = None` marks the full i64 range. Per **C83** the internal representation *follows*
  the user-visible contract and is memory-bandwidth-conscious — it is **never widened for
  implementation convenience**. Blanket i64 storage would double every integer node/field for
  zero user-visible gain; the earlier "widen `Value::Int`" attempt was correctly **reverted** (it
  introduced a silent `as i32` truncation in a narrow storage path — solving the wrong problem).
- **The rule, restated to match the intended design:** *the default `integer` denotes the i64
  value range; storage uses the smallest sufficient encoding, with `forced_size = None` /
  `Long` as the full-range carriers.* Under this rule the code is **conformant** — `forced_size`
  as the full-integer marker is the intended encoding, not a width hack to remove. Narrowing is
  range-driven (this already closed D3/D5); signedness is correct (`i8` does not fit `u8`); the
  parser agrees with codegen. Guard: `d2_signed_narrowing_i8_to_u8_needs_cast` (tests/issues.rs)
  + the i64 user-visible regression above.

**If** a *user-visible* i64 truncation is ever found (a value a user can observe being clipped),
that narrow path is fixed — still without blanket widening (C83 § Revisit). The site audit in
[plans/88-integer-i64.md](../plans/88-integer-i64.md) remains the reference for any such targeted
fix. @PLN88's storage-rework rungs are **not** pursued (off the path per C83).

## Carried by types.md until 2026-09-04

The rules doc used to carry these beside its `OPEN` line — closure summaries, and notes on
the times the count read 0 over a live entry.  They are timeline, so they moved here
unchanged; [types.md](types.md) now states only what is open.

### the times this register read OPEN: 0 over live entries

⚠ **This line read `OPEN: 0` while D-Narrow-Asgn and D-Narrow-Res were both live, and the
oracle under it could not have moved either** — `(I-Narrow)` had only two clauses, so a
nullable target was not a case the rule could be checked against at all; and the sentinel's
exclusion from `τ?`'s range was prose in a table rather than a rule, so nothing could be
checked against it either.  Both gained the clause they were missing — `(I-Narrow-Opt)`, which
closed D-Narrow-Asgn, and `(N-Reserve)`, which turns an open design question into a stated
deviation.  A register is only as strong as the completeness of the rules above it.

⚠ **And it read `OPEN: 0` again while D-Null-Heap was live, where the rules were NOT the weak
part.**  `(N-Opt)` and `(N-Store)` are written for every `τ` and always were; what disagreed was
the enforcement, gated on a scalar-only predicate whose own doc comment stated the carve-out as
if it were the model.  So a register can also be wrong when its rules are complete and nothing
re-measures the code against them — the second failure mode, and the one a reading of this doc
alone cannot catch.

⚠ **And a third time, over two entries closed the day they were named (2026-09-06, D-If-Coal and
D-If-Chain below).**  The rules were complete — `(N-Decl)`, `(I-Narrow)` and `(N-Store)` settle
that a value-`if` and an `else if` chain stored into a declared slot are checked like every other
spelling — and the enforcement existed for every spelling but those two.  Nothing re-measured the
code against the rules for the SPELLING axis of a store, so a register at zero sat over a `u8`
that held `null`, a condition that was silently rewritten, and a float whose bits printed as an
integer.  The second failure mode again, on the axis the fixes' own guards had held fixed.

### the status line formal/README.md's area table carried until 2026-09-04

**0 open** — D-Null-Join (loft#1103) opened and closed 2026-08-26: at a branch JOIN a nullable in a LATER arm stored into a non-null slot in silence, whichever arm and however spelled.  @PLN25 value/null model landed (DN1–DN6 + D2 closed); the @PLN102 null-flow generalisation (N-Prop/N-Domain/N-Cast/N-Store incl. call-arg + DN3-Float) SHIPPED default-on and verified both backends — for the DIRECT store, which is the bound on that verification

## Closed after the 2026-09-04 move

### D-If-Coal — CLOSED (2026-09-06, loft#1379): a value-`if` stored into a narrow slot was claimed as a `??` coalesce
- Was: `Parser::range_guard_inside_discharge` (@PLN152) recognised the bare-variable `??`
  lowering — a plain `if coalesce_not_null(v) { v } else { d }`, unmarked — by the node alone,
  so every author's value-`if` stored into a `u8`, `i16`, `u32` or `limit(…)` slot was treated
  as a discharge: its then arm became a checked cast (`c: u8 = if t { a + b } else { a }` read
  `null` in a slot with no code for one, and a `limit(10,20)` slot lost `(E-Uncomp-NN)`'s
  default), the first operand of the author's CONDITION was range-cast (`c: u8 = if k == 1000
  { a } else { b }` compared `(k as u8?) == 1000` and took the else arm), and the `(N-Decl)` /
  `(I-Narrow)` refusal every other spelling gets never fired.  Both backends; in 2e6a04ba, so
  released in 2026.9.0.
- Rules: `(N-Decl)`, `(I-Narrow)`, `(N-Store)` — settled; no rule moved.
- Fixed at one home: `Parser::bare_variable_discharge` asks the BUILDER (`coalesce_not_null`)
  whether the condition is the not-null test of the then arm's variable; an author's `if`, and
  an author's hand-written `if x != null { x } else { d }`, are not.  Guards
  `tests/scripts/1379-a-value-if-into-a-narrow-slot-is-not-a-coalesce.loft` and `1379b-…`,
  falsified at 2b992851 on both backends.

### D-If-Chain — CLOSED (2026-09-06, loft#1380): an `else if` chain was never held to the then arm's type
- Was: `parse_if` parsed an `else if` chain through a recursive `parse_if` that expected
  nothing of the chain's then arm and kept the chain's type out of the join (loft#936/#978),
  so the if-expression reported the then arm's type while an arm of another type sat behind
  it, never converted or refused: `x: integer = if a { 1 } else if b { 2.5 } else { 3 }`
  printed the float's bits, `f: float = … else if b { 2 } …` the integer's, and 260 reached a
  `u8` local, argument and return.  Both backends; pre-existing on b1ccf0e9.
- Rules: `(N-Decl)`, `(I-Narrow)`, `(C-Var)` — settled; no rule moved.
- Fixed at one home: `parse_if_expecting` threads the enclosing then arm's type into the chain's
  then block, so `parse_block`'s tail conversion — the one the plain `else` already takes —
  covers it; its three `else`-only carve-outs (the loft#1350 tuple boxing, the sibling-variant
  carve-out, the loft#978/#1103 honest deps) read `arm_of_sibling`.  Guards
  `tests/scripts/1380-an-else-if-chain-answers-in-its-first-arms-type.loft` and `1380b-…`,
  falsified at 2b992851 on both backends.

## Deviations carried by types.md until 2026-10-06

Opened and closed in the same change; the chapter's `OPEN:` count never carried them.

* **D-types-26** *(opened 2026-10-06, CLOSED 2026-10-06)* — `(C-Refl)` for a vector element,
  decided on STORAGE: two element types are one when their bytes mean the same values, and an
  element is a place the callee may write, so bytes that mean other values never convert.  The
  one predicate that answers it, `Type::same_element_storage`, was wrong both ways.  It took an
  element's nullability from `!spec.not_null`, where the storage home
  (`Data::narrow_vector_element`) reserves a null code for a `τ?` element only, so a user range
  alias without `size` (`type Byte = integer limit(0, 255)`, one byte in every position) read
  as two, and `vector<u8>` / `vector<Byte>` were refused against each other — *"expected
  `vector<integer(0, 255)>`, got `vector<integer(0, 255)>`"* — as were `vector<i16>` /
  `vector<Coord>` and the narrow tuples.  And it compared only the width and the SIGN of `min`,
  while a narrow element is stored as `value - part_min`: `vector<integer limit(100, 355)
  size(1)>` met `vector<u8>` in both directions and each read the other's bytes as its own (355
  for `[100, 200, 355]`, 303 for `[0, 1, 2]`), silently, on both backends.  **Fix.**  The same
  nullability and width, and below 8 bytes the same `part_min`; an 8-byte element is stored raw
  and is one storage whatever range it declares.  A first version compared the declared RANGE
  instead, and refused `vector<integer limit(-99999, 99999)>` into `vector<integer>` — a
  program that ran (bench/10_sort) and whose bytes mean the same; storage is the basis, and a
  callee writing outside a declared range it received this way is the range's promise, not a
  misread.  `u8?` and `Byte?` stay two storages (the sized one gives its top code to null, the
  unsized one widens, `(N-Reserve)`).  `IntegerSpec::is_full_integer` now answers the three
  identical copies `parser/mod.rs` spelled.  Guards
  `tests/scripts/a-vector-element-of-one-range-meets-every-spelling-of-it.loft`,
  `tests/scripts/a-vector-element-of-another-range-is-refused.loft`.  Published libraries: 41
  pass, 0 compile-break.  `Contract: strained` — a misreading conversion that compiled is
  refused.

* **D-types-25** *(opened 2026-10-06, CLOSED 2026-10-06)* — `(C-Never)`: a jump is a `Never` that
  fits wherever a value is expected, and four positions disagreed.  (1) Whether a VALUE follows
  `break`, `return` or `?? return` was answered three times, each asking only `;` and `}`: in
  `k = match i { 4 => break, _ => i }` the `, _ => i` was parsed as the break's value, so the
  loop RETURNED from the function — 4 on the interpreter, 0 on native, where 0+1+2+3 = 6, with
  no diagnostic; `4 => return,` in a void function was refused.  (2) `?? break` and
  `?? continue` were refused by name, though `(N-Coal)` checks the default against τ and
  `(C-Never)` admits a jump there; the refusal had replaced an "Expect token ;" message and
  recorded no decision.  (3) A labelled `i#break` / `i#continue` was typed `void`, not
  `Never`, so `p = f() ?? i#break` was refused as a default of the wrong type; `i#break` outside a
  loop said "Cannot continue".  (4) On `--native` a diverging `match` arm beside a text or boolean
  arm took the arms' unify wrapper — `&*(return …)`, `({ return … } as u8)`, rustc E0614 / E0605
  — and a `??` exit over a TEXT yielded a borrow of its own temporary (E0597), so text
  `?? return` had never compiled natively.  **Fix.**  `Parser::control_value_follows`, the one
  answer (`;` `}` `,` `)` `]`, end of input), asked by all three words; `?? break` / `?? continue`
  parse the word as the expression it is anywhere else and share `?? return`'s block
  (`null_coalesce_exit`); labelled jumps type `Never`; the emitter gives a diverging arm none of
  the unify wrappers, and a text value block whose tail is its own local moves it out.
  Measured: 17 cells before (13 held, `?? break`/`?? continue` refused, one silent-wrong, one
  native compile failure), 15 more after, both backends.  Guards
  `tests/scripts/a-bare-break-or-return-before-a-comma-carries-no-value.loft`,
  `tests/scripts/a-null-can-leave-the-loop-through-a-coalesce.loft`,
  `tests/scripts/a-diverging-arm-beside-a-text-or-boolean-arm-compiles-natively.loft`.
  `Contract: strained` — `?? break` / `?? continue` were a documented refusal (@F2) and compile.

## Deviations carried by types.md until 2026-09-29

Closed entries moved here from the rules chapter's register (RELEASE.md § 5b), as written.

* **D-types-24** *(opened 2026-09-29, CLOSED 2026-09-29)* — `(T-Syn)`: one expression, one
  synthesised type on both passes.  Pass 1 marks a variant literal `Rewritten(Circle)` (built in
  place) and pass 2 does not, and `variant_parent_enum` / `joins_to_enum` stopped at the marker —
  so on pass 1 the arms of `if c { Circle {…} } else { Sq {…} }` never joined: the `Sq` arm was
  checked against `Circle` and ACCEPTED as one, and the join was typed `Circle`.  Pass 2 typed it
  `Shape`.  That disagreement is what D-types-22's inferred-local refusal was; after D-types-22's
  `variant_join` it was absorbed, and a pass-1 decision could still read the wrong type.  **Fix.**
  Both read through `Rewritten`.  **Instrument.**  `LOFT_AUDIT_RETYPE=1` reports every PASS-2
  retype of a variable pass 1 typed as another shape — each is an expression synthesised twice
  differently.  Over the corpus it named this one and, through parameter defaults (the one place
  pass 1's tree is replayed), led to calls.md D-call-25 and D-call-26; the rest were inference
  completing (a generic resolved on pass 2), `#663`'s width adoption and the nullable-struct
  synthesis, both documented.
* **D-types-22, D-types-23** *(opened 2026-09-29, CLOSED 2026-09-29)* — `(C-Var)` composed with
  `(I-Join)`/`(N-Join)` and `(C-Tuple)`: two DIFFERENT variants of one enum join to the enum
  wherever they meet, and nothing is licensed between two variants.  Walking `(T-Sub)` found
  `C-Var` holding at 24 of 25 checking positions; every failure was a JOIN.  **-22** (over-refusal):
  an INFERRED local assigned two variants — an `if`, a `match`, an `else if` chain, or
  `x = Circle {…}; x = Sq {…}` — was refused *"cannot change type from Circle to Shape"* (pass 1
  types the arm join by its first arm, pass 2 by the enum, and `change_var_type` had no variant
  join); tuple arms naming different variants in one member were refused in every `if` and
  `match` form, even into a declared `(Shape, integer)`; and a vector literal of such tuples,
  `[(Circle {…}, 1), (Sq {…}, 2)]`, was *"No common type"* while `[Circle {…}, Sq {…}]` compiled.
  **-23** (silent-wrong, found by -22's refusal cells): the arm join's member test was
  `Type::is_same`, a KIND test to which any two records are one type, so `(null, Circle {…})`
  beside `(3, Tri {…})` — another enum — joined as `(integer?, Circle)` and the `Tri` was read at
  `Circle`'s offsets on both backends; loft#1682's own join carried it.  Once -22 let variant
  tuples reach that join, a pair that did not join was also converted SILENTLY in `parse_if`
  and in the scalar `match`, which has no second check.  Before this change the null-member
  shape already slipped through there: `(3, Tri {…})` after `(null, Circle {…})` printed the
  `Tri` as `{r:2.5}`, and a text member stopped code generation with an internal compiler error.  **Fix.**  `Variables::variant_join` answers the inferred local's join,
  element-wise; `Parser::tuple_join_open` makes a variant member leave a tuple arm unpinned, as a
  `null` member did; `join_tuple_arms` tests members by `is_equal`, joins nested tuples and
  either order of variant and enum, through a `?`; a pair that does not join is refused where
  the join is asked (`parse_if`, `join_null_tuple_arm` for every match site); and an inferred
  vector literal stores a tuple element's variant members as their enum
  (`widen_variant_members`), as it stores a bare variant element.  Guards
  `tests/scripts/sibling-variants-join-to-their-enum.loft` (the value half, scored by `{}`, which
  renders by the STATIC type) and `sibling-variants-refuse-*.loft` (three refusals).
* **D-types-20, D-types-21** *(opened 2026-09-29, CLOSED 2026-09-29)* — `(T-Chk)` pushes the
  expected type "structurally into sub-expressions", and at an argument, a tail or a default
  it stopped at the literal's brackets.  **-20** `(T-Chk-Vec)`: the expected `vector<τ>` reached
  a bare literal's elements only when `τ` was a narrow integer (#432's scope), so `f([1, 2])`
  into a `vector<float>` parameter, `fn g() -> vector<(float, integer)> { [(1, 2)] }`, a lambda
  tail and a parameter default were refused while `v: vector<float> = [1, 2]` compiled; a
  `vector<S>` or `vector<vector<float>>` parameter DEFAULT failed with *"Variable '_elm_1' cannot
  change type"* once seeded, because it was parsed into a variable slot nothing occupied.  The
  hint now seeds any element type naming no type variable, and a literal that is the RECEIVER
  of a chain (`[1, 2].map(…)`) takes none.  **-21** `(T-Chk-Var)`: the list of expected-type
  shapes an argument pushes was written five times and disagreed — a named argument, a method
  argument and a parameter default reported a bare variant ambiguous where a positional
  argument resolved it, a method argument took no tuple member types, and a struct-field
  default took no collection.  One list, `argument_hint`, now serves every spelling.  All were
  refusals.  Guard `tests/scripts/an-expected-element-type-reaches-every-literal-position.loft`.
* **D-types-18, D-types-19** *(opened 2026-09-29, CLOSED 2026-09-29)* — two field stores that
  skipped `convert`, so `(C-Num)`, `(C-Tuple)` and the width rules did not reach them.  **-18**:
  a field DEFAULT was converted only integer → integer, so `p: float = 7` stored the integer's
  BITS — the field read 3.5e-323 on the interpreter and did not compile natively (E0308) — and a
  `(float, integer) = (1, 2)` default likewise; the default now converts like a parameter's
  (a collection's is still delivered at the field type by the function it is hoisted into).
  **-19**: a tuple field REASSIGNED (`w.p = (3, 4)`) was written member by member with no
  conversion and no check, so an `integer` member stored its bits into a `float`, `(300, 4)` into
  a `(u8, integer)` field kept 44 on both backends, and a `text` member was written as a float;
  the value is now converted and checked against the field.  Guards
  `tests/scripts/a-field-store-converts-its-value.loft`,
  `tests/scripts/a-field-store-refuses-a-narrowed-member.loft`,
  `tests/scripts/a-field-store-refuses-a-text-member-into-a-float.loft`.
* **D-types-17** *(opened 2026-09-29, CLOSED 2026-09-29, loft#1742)* — a tuple carrying a HEAP
  member (so returned in its stored spelling, `__tuple<…>`) whose member widens on the way out
  of a function was REFUSED when the returned value was a CALL: `fn back(k) -> (text, float) {
  mk(k) }` over `mk -> (text, integer)` said *"expected (text, float), got (text, integer) on
  return from block"*, and so did an inferred local holding the call's result, an `if` or
  `match` of calls, and a `return mk(k)` statement.  The return's tuple rewrite takes a
  STACK-tuple tail only; converting the members inside `convert` instead left it a literal it
  wrote as `null`.  Both return sites now ask `unbox_stored_tuple_tail`, which binds such a tail
  to a stack-tuple local at its OWN members — the shape an annotated local already had — and
  the rewrite writes each member at the declared type, converting it.  Guard
  `tests/scripts/a-heap-tuple-returned-from-a-call-widens-its-members.loft`.
* **D-types-15, D-types-16** *(opened 2026-09-29, CLOSED 2026-09-29)* — `(C-Num)`, which this
  chapter did not state although LOFT.md's conversion table promises it, through `(C-Tuple)`.
  **-15**: a tuple that is not a LITERAL was stored bit for bit, so a member widening from
  `integer` to `float` or `single` was REINTERPRETED — `a: (integer, integer) = (1, 2)` stored
  as `(integer, float)` read `(1, 1e-323)` on the interpreter and did not compile natively (E0308),
  at every site: a local, an argument, a return, a field, a vector element, nested members.  A
  stored-spelling source (a heap-carrying tuple, `__tuple<…>`) had no conversion at all and was
  refused.  Such a tuple is now rebuilt from its converted member reads (`convert`'s tuple arm;
  a stored source is unboxed at its OWN members first; a returned variable is rewritten before
  the synthetic return stores it).  **-16**: the scalar `f: float = a` — `(C-Num)` into a LOCAL —
  was refused as *"cannot change type from float to integer"*, naming the change backwards,
  while the other four sites converted; the variable seam now converts before it retypes.
  Guards `tests/scripts/a-tuple-converts-member-by-member.loft` (-15's wrong values) and
  `tests/scripts/a-numeric-widening-reaches-a-local-and-a-stored-tuple.loft` (the refusals).
* **D-types-10 to D-types-14** *(opened 2026-09-29, CLOSED 2026-09-29)* — `(I-Lit)`,
  `(I-Narrow)` and `(I-Narrow-Opt)` name no exception for HOW a value reaches a narrow slot, and
  a walk of the ways one does found five that disagreed.  Each held for the direct store and
  missed where the value arrived another way.
  **-10**, a tuple LITERAL: `t: (integer, u8) = (1, 256)` held 256, and in a
  `vector<(integer, u8)>` read 0 — loft#1640 had given only the member WRITE `t.1 = …` the check.
  **-11**, a `??` default: `x: u8 = g() ?? 300` held 300, because the discharge exempted the
  whole value where it answers only for the subject; a chain's inner subject is guarded like
  the outer one.  **-12**, a struct field DEFAULT met no conversion at all: `f: u8 = 256` read 0,
  and `f: Lim? = 12` held 12 past its `limit(0, 10)`.  **-13**, an `if` or `match` whose every
  arm is a fitting literal was REFUSED — the expected width reaches each arm (`(T-Chk)`), so the
  fit test now asks the arms.  **-14**, `fn g(a: u8? = 5)` did not parse: pass 2 turned the
  default into a checked cast with a temporary and hoisted it into a function pass 1 had not
  minted; and kept in place, `u8? = 300` reached the parameter as 300, because a call site
  replays pass 1's unconverted tree.  A constant now folds in the cast and the range guard, and
  a default meeting a nullable narrow parameter or field is always hoisted, so pass 2 converts
  its body.  Guards `tests/scripts/a-narrow-slot-is-checked-wherever-a-value-reaches-it.loft`
  (-10 to -12, whose values the pre-fix tree gets wrong), `tests/scripts/arms-and-nullable-parameter-defaults-meet-a-narrow-slot.loft`
  (-13 and -14, which it refused), and six refusal pins in `tests/parse_errors.rs`.
* **D-types-9** *(opened 2026-09-29, CLOSED 2026-09-29)* — the "never null" lints against
  `(N-Store)` and `(E-Truthy-1)`.  `redundant-null-negation`, `redundant-null-check`,
  `redundant-coalesce` and `redundant-default-fallback` read a FIELD as never null whenever it
  was not declared `?`, so they called `!s.i`, `s.i == null`, `s.i ?? d` and `s.i?` constant on
  every full-width field — integer, float, text, character, enum — while the slot held the null
  `(N-Store)` let into it, and while C85's overflow reached a plain `integer` field with nothing
  said at all (`s.i = a * b` read null, `!s.i` read true).  The deleted check was the only
  thing that saw it.  A local and a parameter of the same types were already quiet, so the
  spelling decided the answer.  `constant-condition` made the same claim about `text` in every
  position (`if t` on a null text took the ELSE branch), and `IntegerSpec::non_null_reads_null`
  read `not_null`, a flag every field carries and that has no effect on the bytes.  **Fix.**
  One home, `Type::non_null_slot_reads_null`: yes for the in-band kinds, no for a declared
  narrow range (C127), a dense record and a collection; `get_field` and `constant-condition`
  ask it.  Guards `tests/runtime_warnings.rs` `never_null_lints_*` (a lint × field-kind matrix with
  each cell's value) and `tests/constant_condition_lint.rs` (the text cell).  loft#1297 had softened the
  `== null` message for the integer case; its table rated a `text` field "genuinely always
  false", which `(N-Store)` had since made untrue.  `Contract: settled` — both rules already
  said the slot holds null and that null is falsy.
* **D-types-8** *(opened 2026-09-29, CLOSED 2026-09-29)* — `(N-Store)` for a PLAIN enum: a bare
  `null` into an `E` field, argument or return passed in silence while the slot held null (all
  three read back null, both backends).  The report's heap half asked `is_dbref` and the scalar
  half `is_non_null_scalar`; a plain enum is neither, the gap loft#1313 (`D-Null-Heap`) closed for
  the handles.  **Fix.**  `nstore_null_report` asks a plain enum with the heap half's wording.
  Guard `tests/scripts/153-n-store-a-bare-null-into-a-declared-local-warns.loft` (its three enum
  cells).  `Contract: strained` — silence became a warning.
* **D-types-7** *(opened 2026-09-29, CLOSED 2026-09-29)* — `(N-Store)` / `(N-Join)` for a bare
  `null` into a LOCAL.  A DECLARED full-width local was refused (*"cannot hold both `null` and the
  non-null type"*) at every kind where the same `null` WARNED at a field, an argument and a
  return — `D-Decl-Sev`'s shape for the literal, which that entry had closed for a `τ?` value
  only.  An INFERRED local joined with `null` was refused in seven of eight cells: only
  `a = null; a = 5` over an inline scalar widened; `a = 5; a = null` was refused even for an
  integer, and `text`, a vector and a record were refused in either order — the DN6 note had
  deferred the one direction ("cannot be told apart from an annotated local here", which
  `author_declared` now tells) and held `text` out on a slot fear that did not reproduce.
  **Fix.**  At the assignment seam a bare `null` into a local takes the two arms a `τ?` takes:
  a declared local asks the store face and keeps its type, an inferred one converts against
  `τ?` and `change_var_type`'s `(N-Join)` arm widens it; the null-first arm admits every kind
  whose `τ?` is declarable.  A narrow integer keeps the error at both.  Measured: 32 cells
  (4 kinds × {local, field, argument, return} × {bare, `τ?`}) and 8 inferred joins before;
  22 case cells after, both backends under `LOFT_STRICT_STORES`.  Guards
  `tests/scripts/153-n-store-a-bare-null-into-a-declared-local-warns.loft`,
  `tests/scripts/153-n-join-a-bare-null-widens-an-inferred-local.loft`; the two pass-1
  `@EXPECT_ERROR` cells in `102b-pass1-expected-errors.loft` that pinned the refusal are gone.
  `Contract: strained` — a refusal became a warning, and a refused join compiles.

* **D-types-6** *(opened 2026-09-28, CLOSED 2026-09-28; loft#1720)* — `(N-Store)` with
  `(C-Var)`: a nullable value reached a non-null slot in silence in three shapes.  A declared
  ENUM local written a nullable VARIANT (`d: Shape = mc(i)`, `mc -> Circle?`): the store face
  asked only when the value's base EQUALLED the slot's type, and a variant does not.  A FIELD or
  ELEMENT written a nullable `text`, enum or record: a scalar slot is asked at the typed store,
  and these three leave the assignment before it (`assign_text`; `towards_set`'s copy) — a text
  or plain-enum slot then held null, and a dense record, which has no null to hold, kept the
  value it had (`h.p = q` with `q` null left `h.p` as it was).  And the cure the issue itself
  named, `mc(i) ?? Shape::Square {…}`, was refused: the result took the value's VARIANT type,
  and a sibling variant does not convert to it.  **Fix.**  `Parser::slot_takes_base` admits a
  variant of the slot's enum wherever the store face asks; the assignment asks `(N-Store)` at a
  place for the three kinds the typed store never sees, with the consequence the slot has — a
  dense record or payload enum keeps its value (loft#1404's clause for a bare `null`), a
  `reference<T>` pointer field repoints and holds null; and `??` joins a sibling variant (or the
  enum) to the enum, as `if`/`else` does.  Guards
  `tests/scripts/1720-a-nullable-written-to-an-enum-text-or-record-slot-is-reported.loft`,
  `tests/scripts/1720b-a-nullable-written-to-an-element-is-reported.loft`.

* **D-types-5** *(opened 2026-09-28, CLOSED 2026-09-28; loft#1714)* — `(N-Join)`: a branch that
  joins `null` with a struct, a collection or an enum typed the result NON-optional — `a = if n ==
  0 { null } else { [n] }` gave `a: vector<integer>` while the integer twin gave `int?` — on the
  premise *"heap types stay nullable"*, which `(N-Opt)` contradicts.  The cost was a lost write:
  `(Col-Insert-Absent)` is written for `<kind>?`, so `a += [7]` after the null arm wrote through
  a null handle, silently, on both backends; and the tuple and vector `match` chains did not
  widen at all, scalars included.  **Fix.**  One home for the question, `Parser::null_arm_widens`
  (every type `data::has_null` admits), asked by all five constructs (`if`; `match` over an enum,
  a scalar, a tuple, a vector).  What a `null` arm IS is read from the SOURCE token
  (`block_tail_null_literal`, `null_literal_arms`): the lowered arm is its sibling's typed null by
  then, and on the first pass a comprehension arm is a placeholder of the same shape — reading
  the lowered code widened `if c { [for …] } else { … }`.  Two consumers of the wider type were
  closed with it: a variant joined with null is admitted into its enum through the `?`
  (`change_var_type`), and a cursor sub-rule may be declared `-> N?` (`peek_subrule_capture`).
  **Measured before the change** with `loft --check` over every file containing `null` in the
  corpus, the libraries, the consumers and the registry: 65 sites, all functions declared `-> T`
  whose branch returns `null` — a warning, as `(N-Store)` says — plus one library signature that
  was wrong (`imaging` `png() -> Image`, which answers null; published as 0.4.0 `-> Image?`).
  Guards `tests/scripts/a-branch-joining-null-is-optional-for-every-kind.loft`,
  `tests/scripts/a-sub-rule-may-answer-an-optional.loft`.

* **D-types-4** *(opened 2026-09-26, CLOSED 2026-09-26; loft#1692)* — `(N-Reserve)` /
  `layout.md (L-Narrow-Enc)`: the loop variable of `for x in v`, over a vector of a NULLABLE
  narrow integer, read a null element as its stored code — `255` for `u8?`, `-32768` for
  `i16?`, `2` for `u16?` — so `x ?? d` never discharged and `!x` never fired, on both backends,
  while `v[i]`, a copy and a struct field decoded the same element as null.  **Where.**  The
  loop's element read (`collections.rs`) called `get_val` with nullability hard-coded `false`;
  the indexed read (`fields.rs`) passes the element's declared nullability.  One question —
  does this slot decode a null — and two decoders.  **Fix.**  The loop passes it too.  Guard
  `tests/scripts/1692-a-loop-variable-reads-a-nullable-narrow-element-as-null.loft`, which also
  pins `(N-Reserve)`'s edge: a `255` written into a `u8?` slot IS its null.  Found at the
  narrow × `??` crossing (16 corpus files).

* **D-types-3** *(opened 2026-09-26, CLOSED 2026-09-26; loft#1682)* — `(T-Chk)` / `(I-Join)`: a
  value-position `if` or `match` whose FIRST arm is a tuple literal with a `null` member was
  REFUSED — *"expected (null, integer), got (integer, integer) on else"* — whatever the declared
  destination (a `(τ?, τ)` return, a typed local, no annotation), in an `if`, an `else if`
  chain and every `match` form, while the same program with the concrete arm first compiled
  and answered.  **Where (measured).**  A sibling arm is parsed against the type the arms so
  far settled on (`parse_if`'s else arm, `match_arm_expected`), and a tuple literal with a
  `null` element synthesises `(null, integer)`, a type nothing converts a concrete member to;
  a bare scalar `null` arm had the carve-out that leaves the sibling to decide (`Type::Null`
  is "not settled"), and a tuple WITH a null member did not.  `(I-Join)` says the join is
  `⨆ τᵢ`, optional iff some member is — `(null, τ) ⊔ (τ, τ) = (τ?, τ)` — and `(T-Chk)` pushes
  the expected type into sub-expressions, so the first arm's synthesised member was never the
  type to hand down.  **Fix.**  `Parser::tuple_has_null_member` makes such a result "not
  settled" wherever a sibling is handed its expected type; `join_tuple_arms` joins the arms
  ELEMENT-WISE (`null ⊔ τ = τ?`, two sibling variants to their enum, `(C-Var)`); both tails
  are converted to the join (`convert_arm_tail`), and every match site reconverts the arms
  it had already assembled — measured necessary: with the TYPE alone joined, the first arm
  of `match z { 1 => (null, 3), _ => (4, 5) }` read `1` for its null member, silently.  Six
  sites parse a match arm against the settled type (the enum loop, its `null`-arm and
  wildcard paths, the scalar, vector and tuple pattern matches); the join is one helper
  (`join_null_tuple_arm`) and each site owns its reconversion.  Guard:
  `tests/scripts/1682-a-null-member-in-the-first-tuple-arm-does-not-pin-the-join.loft`.
  Found by `matrix_axes.py cross A7 A3`: the corpus's thinnest crossing was a nested
  container in a tuple-element position, and the probe there met the refusal on its ninth
  cell.

* **D-types-2** *(opened 2026-09-23, CLOSED 2026-09-23; loft#1640)* — `(N-Reserve)`: a TUPLE MEMBER of a narrow
  type is not bounded by its declared range on a plain assignment.  `t: (u8, u8) = (250, 7);
  t.0 = 300` stores `300`, and `t.0 >= 0 and t.0 <= 255` — the type's own range, written out
  — reads `false` for a value the type is holding; copying that tuple into a
  `vector<(u8, u8)>` element then reads `44`, the low byte, with nothing reported at either
  step.  `i8` takes `5000` and `u16` takes `999999` the same way.  The three slots the rule
  DOES name refuse it: a local, a struct field and a vector element all answer *"cannot
  implicitly narrow integer to u8"*.  **Where (measured).**  The COMPOUND path is correct —
  `t.0 += 10` from 250 answers `0` like the local — because loft#1228 routed tuple members
  through the one seam `Parser::guard_compound_range` sits at; it is the compile-time
  `is_narrowing_int_store` check on the PLAIN assignment that does not see a tuple member as
  a narrow store place.  Both backends agree, so a coverage gap and not a divergence.  A
  tuple member inside a CONTAINER is a second, louder gap: `v: vector<(u8, u8)> = [(1, 2)];
  v[0].0 += 10` is refused with *"Not implemented operation + for type integer(0, 255)"*, a
  message about an operator that is plainly implemented — loft#1228's own shape one level
  deeper.  Found by `scripts/matrix_axes.py`, which reports `A3 … MISSING tuple-element`
  against the C127 guards.  **Closed** the same day: the narrowing refusal and loft#984's range
  guard now live in one method (`Parser::narrow_store_checks`) that the general assign path and
  the tuple branch both call, so a tuple member is bounded like every other slot and the third
  slot kind that reaches neither has one place to be added to.  Guard
  `tests/scripts/1640-a-tuple-member-is-a-narrow-slot-like-any-other.loft` for what must still
  be true, and two `@EXPECT_ERROR` cells in `102-expected-errors.loft` for the refusal itself.
  The second half — `v[0].0 += 10` on a `vector<(u8, u8)>` refused as *"Not implemented
  operation +"* — is a tuple-in-a-container ROUTING question rather than a narrowing one and
  stays open on the issue.
