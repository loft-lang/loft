# formal/operational-history.md — the deviation register for [operational.md](operational.md)

> **The rules are next door.**  [operational.md](operational.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

OPEN: **3** (D-op-1/2, and D-op-5 opened 2026-08-25 — two spellings of a following
null-check still report, the sibling of a wrapper-list drift fixed the same day; the null-model keystone deviations D-op-null-1/2 both CLOSED 2026-07-10 by
keystone steps 2–3, D-op-6 opened AND closed 2026-08-29 by the first `@FR-E-NullArg` walk,
and D-op-7/8 opened AND closed 2026-08-31 by loft#1246 — the sized-integer overflow pair.
Opened 2026-07-10 by the @PLN102 pre-freeze audit —
[the null-model keystone decision](../plans/102-stability-contract/keystone-null-model.md).)

⚠ **This register read `OPEN: 2` while D-op-6 was live, and no measurement could have moved
it** — the rule it violates was itself incomplete. `(E-NullArg)` named comparisons as the only
exception to contagion and never mentioned truthiness, so a `&&` that answered `null` looked
like the rule being OBEYED rather than a shipped decision (C73) being broken. An `OPEN: n` is
only as strong as the rules above it, not only as strong as its oracle.

### D-op-7 — CLOSED (2026-08-31, loft#1246): (E-Uncomp-NN)'s default was the range's FLOOR
- Was: `OpRangeDefault(value, lo, hi, dflt)` is emitted for every declared range, and TWO
  functions filled `dflt` with `lo` — `declared_range` for the `limit(…)` spelling and
  `compound_range` for the `+=`-on-a-local path.  `u8` and `u16` have a floor of zero, so they
  looked right and hid the rest: every signed width answered its minimum (`i8` `-128`, `i16`
  `-32768`, `i32` `-2147483647`), which is in range, type-correct, and as unrelated to the
  computation as a wrapped value would be.
- The rule it violates did not exist when the code was written.  `(E-Uncomp)` says an
  uncomputable result is `null` and carves out only float/single; it has nothing to say about a
  slot that cannot HOLD null, which is what a non-nullable declared range is.  Closing this
  needed the rule EXTENDED — `(E-Uncomp-NN)`, added the same day: the next best thing to null is
  the value the slot would have had if nobody had assigned, never a value derived from the
  machine.  ("Letting the way the hardware of a processor works dictate the outcome is the worst
  choice.")
- Fixed at one home: both range paths call `range_default(lo, hi)` — zero wherever the range
  admits it, else the bound nearest zero.  Three shipped guards asserted the superseded answer
  and were rewritten to the rule rather than worked around (`1009-width-type-bounds`,
  `1030-compound-range-both-spellings`, `984-limit-field-out-of-range-defaults`).  Commit
  361c6b77; verified both backends cell for cell.

### D-op-8 — CLOSED (2026-08-31, loft#1246): (E-Uncomp) did not reach a nullable narrow ALIAS
- Was: `u8?` answered `0` on overflow and `??` — the documented recovery — was inert on it,
  while `integer limit(0,255)?`, the SAME range spelled differently, answered null.  Two
  separate defects held it there, and a fix for either one leaves the other standing:
  - the OVERFLOW path — the two range functions each decided the default for themselves, and
    only the `limit(…)` one asked whether the slot was nullable.  `uncomputable_default(tp, lo,
    hi)` is now the one home both ask: `null` where the slot admits it (E-Uncomp), the type's
    default where it does not (E-Uncomp-NN).
  - the STORE path — `d: u8? = p + 10` kept **260**, a value outside the range its own type
    declares, while `f(p + 10)` on a `u8?` parameter and `S { u: p + 10 }` on a `u8?` field both
    answered null.  `(I-Narrow)`'s nullable completion — DN4's implicit checked cast, which
    types.md § Null-flow names as the reason a narrowing into a *declared-narrow slot* yields
    `τ?` — lives in `convert`, and the ASSIGNMENT seam never reaches `convert` for this pair,
    because `integer` and `integer(0,255)?` are `is_equal`.  So that seam ran its own narrowing
    test, a REFUSAL with no checked-cast arm, and one that did not peel the `Optional` wrapper
    either — so it refused nothing and guarded nothing.  `implicit_checked_narrow` is now the one
    home, asked by both seams.
- The two SPELLINGS still reach the answer by different machinery — the alias through DN4's
  parse-time checked cast, `limit(lo, hi)` through the runtime `OpRangeDefault`.  They are
  scored as PAIRS in the guard for exactly that reason; unifying them is not this fix.
- Guard `tests/scripts/1246-a-nullable-narrow-slot-answers-null.loft`, falsified at 341a7bff
  (interpret and native both exit 1 → 0); verified on both backends across local / field /
  struct literal / vector element / argument / return, both directions, `+ - * /`, and inside a
  loop and both arms of an `if`.

> **A number collision worth knowing about.** These two were filed as `D-op-6` and `D-op-7` by
> commit 361c6b77, which did not notice that `D-op-6` was already taken by the entry below —
> closed two days earlier and cited from QUALITY.md.  They were renumbered to 7 and 8 when both
> closed.  A deviation id is only useful while it names one thing, and the OPEN list is not
> where a used id can be seen: every closed one has already moved here.

### D-op-6 — CLOSED (2026-08-29, the `@FR-E-NullArg` walk): `&&`/`||` kept a null right operand
- Was: `&&` and `||` coerced a null LEFT operand to `false` and let a null RIGHT operand through
  unchanged — `true && null` answered `null` where C73 says `false`. The parser types the whole
  expression the non-null `Type::Boolean`, so the null flowed out of a position the type system
  had already promised could not hold one: `r: boolean = t && maybe()` compiled clean and
  `r == null` was `true`; the same value reached a `boolean` STRUCT FIELD and a
  `vector<boolean>` element, and `(t && maybe()) ?? true` discharged it to `true`. On the field
  the compiler even emitted `redundant-null-check`, *"'on' is 'not null', comparison is always
  false"*, beside a comparison that answered true.
- Cause, and why it is one hole and not two: the lowering is `a && b` → `if a { b } else
  { false }`, so the LEFT operand becomes the `if` CONDITION and the jump coerces it
  (`OpGotoFalse` tests `!= 1`) while the RIGHT operand becomes a branch VALUE that nothing
  coerces. `convert` does not close it — every OTHER nullable type reaching a boolean position
  picks up a real conversion (`integer?` gets `OpConvBoolFromInt`, whose `!= i64::MIN` is
  already 0/1), but `boolean?` → `boolean` shares a base type and converts to nothing at all.
  So the coercion had one home, the jump, and the right operand never reached it.
- Fixed in `Parser::boolean_operator` (`src/parser/vectors.rs`), the one site that knows both
  operands are truthiness positions: a nullable-boolean right operand is wrapped in
  `b == true`, C73's raw compare, which answers `false` for the 255 sentinel. Parser-side, so
  both backends inherit it from one IR change; short-circuit is unaffected (the wrap stays
  inside the branch, measured by a counting right operand). Guard
  `tests/scripts/a-boolean-operator-answers-a-definite-boolean.loft`, falsified at `48544f1e`
  on both backends.
- Also corrected by the same walk, in the rule rather than the code: `(E-NullArg)`'s ordering
  clause claimed `boolean`, which has no ordering operator at all, and `(E-Truthy)` did not
  exist.

### D-op-null-1 — CLOSED (2026-07-10, keystone step 2): float/single null comparison now uniform
- Was: `float`/`single` null (a NaN) made `null == null` **false** and ordering unordered, where
  integer/char null is reflexive and orders low — violating `(E-NullArg)`'s uniformity.
- Fixed at the single source: the `Op{Eq,Ne,Lt,Le}{Float,Single}` `#rust` bodies in
  `default/01_code.loft` (which drive BOTH the interpreter via `fill.rs` regen and native codegen)
  now treat a NaN operand as null definitely — `null == null` true, `!=` its exact complement,
  null orders at the low extreme — matching `(E-NullArg)` and the integer/char behavior. Verified
  both backends against the matrix; guard `tests/scripts/pln102-null-comparison-uniform.loft`. The
  conversion set (docs/tests on the old `x != x` NaN idiom → `== null`) was migrated in the same
  change.

### D-op-null-2 — CLOSED (2026-07-10, keystone step 3): collision sites report, no longer silent
- Was: an op whose true result is the reserved `i64::MIN` pattern (or an out-of-range shift/cast)
  silently masked, saturated, or nulled a real value — the silent-wrong class `(E-Null)` forbids.
- Fixed at the single `#rust` source (drives BOTH backends), mirroring `÷0` (report + null +
  continue):
  - **Shifts** (step 3a): `OpSLeftInt`/`OpSRightInt` report `ShiftOutOfRange` on an amount outside
    `[0, 64)` or a left shift landing on `i64::MIN` (`1 << 63`); null operands stay contagious.
  - **Casts** (step 3b): `OpCastIntFromFloat` reports `CastOutOfRange` on a float outside integer
    range (was: saturate to `i64::MAX`); `OpConvCharacterFromInt`/`OpCastCharacterFromInt` report on
    an invalid code point (was: silent NUL); `OpCastIntFromText` reports when a *valid* number parses
    to exactly `i64::MIN` (an unparseable text stays DN3-nullable → null, silently, unchanged). NaN
    floats and null integers stay contagious.
- Distinct from **C85** overflow of ordinary arithmetic, which is a decided edge and stays silent.
- Both backends; guards `tests/scripts/pln102-shift-collision-guard.loft` +
  `tests/scripts/pln102-cast-collision-guard.loft`; the conversion set was one assertion
  (`inf as integer` saturate → null in `02-floats.loft`).

### D-op-1 — there is no shared operational semantics; the interpreter is the spec
- **Violates:** the premise of this doc (a single evaluation relation both backends obey)
- **Where:** `src/state/` (the interpreter) is the de-facto *executable* definition;
  `src/generation/` (native) is a *separate* generator. The rules across this operational
  family — this file's scalar core plus [heap](heap.md) / [iteration](iteration.md) /
  [coroutines](coroutines.md) / [concurrency](concurrency.md) / [calls](calls.md) /
  [matching](matching.md) / [tuples](tuples.md) / [closures](closures.md), all written
  2026-07-04 and each at 0 own deviations — are a written contract the code is *supposed* to
  meet, but none is mechanically checked against either backend.
- **Effect:** correctness for native means "matches the interpreter on the tests we ran",
  not "obeys the semantics". As of 2026-07-04 the gap is **no longer missing rules** — the
  operational rules are now written for every core area (store alloc/read/write/copy/free,
  iteration + combinators, coroutines, `par`, calls, `match`, tuples, closures, text
  formatting, interfaces/generics — the last two added 2026-07-05). What remains is that those
  written rules are not enforced against a *single evaluation relation both backends share*:
  they GUIDE the differential oracle rather than mechanically defining agreement. Nothing is
  left "spec = the interpreter's code" now — only the differential-vs-definitional gap itself.
- **Status:** OPEN — **the oracle is BUILT and growing (@PLN89).** `tests/differential_oracle.rs`
  runs `tests/oracle/*.loft` on BOTH backends and asserts they AGREE on stdout
  (value/null), exit code (halt), **stderr (what the program SAID — warnings and the diagnostic
  a fault renders)**, and leak-freedom, with a positive control proving the detector
  fires.  **2026-08-21 — the stderr channel landed (loft#1056).**  It had been captured since
  the oracle was built and never compared, which is how the same failed `assert` came to print
  a loft diagnostic on `--interpret` and a Rust panic naming a generated temp file on
  `--native`, for as long as both existed.  Seven corpus programs write to that channel, so it
  is exercised rather than agreeing by having nothing in it; the leak line is filtered out
  because leaks are their own channel and the native binary prints one only under
  `LOFT_NATIVE_LEAK_CHECK`.  **2026-08-21 — the call-stack cap converged (loft#1058).**  It was
  the last halting fault rendered two ways, and folding it found more than a rendering: the two
  guards on one cap counted different things, so `rec(9999)` printed an answer on `--interpret`
  and halted on `--native`.  The interpreter tested a `call_depth` counter that never counted
  `main` and was left untouched when a coroutine truncated the stack; it now reads
  `call_stack.len()`, the same quantity `cr_call_push` tests and `stack_trace()` reports on both
  backends, and native checks BEFORE pushing as the interpreter does, so a refused call is not on
  the stack the diagnostic reports.  `32-stack-overflow-halt.loft` pins the boundary from both
  sides.  Two guards enforcing one cap is the shape that drifts, and it drifted in silence
  because the corpus had no program near the cap.  **2026-07-04 coverage push** — the corpus now spans the divergence-prone areas where the
  two backends use the most different mechanisms: coroutines/generators (native state machine vs
  interp suspend), collection combinators (map/filter/comprehension), parallel reductions (par
  dispatch vs sequential), text (Rust String vs interp store), keyed collections (hash/sorted walk
  order + storage), and tuples/recursion — plus the two graduated cross-backend bugs (10/11).  **2026-07-04 — the DRIVER-AGREEMENT scope addition landed**: well-typedness is one static
  judgment, so `--dump` (pure parse+typecheck) / `--interpret` / `--native` must agree on
  accept-vs-reject; `statically_rejected()` (empty-stdout guard so a runtime panic isn't mistaken
  for a static reject) makes the #433 class — interp accepts what native rejects at rustc — a
  first-class caught property.  The oracle now catches real divergences in practice — three
  found this cycle: **#495** (runtime-Join over-free, FIXED), **#500** (native E0308 on a
  nested-ncc optional-text return, FIXED), **#501** (`.map`/`.filter` on a vector literal
  receiver, FILED).  Corpus is **32 programs** spanning coroutines / collections / parallel /
  text / keyed collections / tuples / nullability / nested enums / recursion / closures + the
  graduated bugs.  **NIGHTLY CI GATE WIRED (ci.yml, commit `971150dd`)**: the full
  `--ignored` sweep runs on the 03:00 UTC schedule + push-to-main (Linux-only, never on a PR),
  failing the nightly on any cross-backend divergence — the manual `-- --ignored` run is now
  a standing automatic guard.  Stays OPEN (the deviation closes only when a shared executable
  semantics replaces "the interpreter is the spec", or is reconciled): the corpus keeps growing.
  **2026-08-23 — the corpus can now FAIL, not only DISAGREE.**  The oracle asserts that the
  backends agree, and two backends wrong in the same way satisfy that perfectly — measured
  three times this cycle (the tuple-`&` local, the Join-arm ownership, the JSON walker),
  each identical on both sides and each structurally invisible here.  Re-measured, **32 of
  33 corpus programs carried no `assert` at all**: they printed, and the harness compared
  the two prints.  The hand-computed expected values existed — as COMMENTS (`// 55`,
  `// 3 2`, `// 0+1+1+2+3+5+8+13 = 33`), roughly 157 of them, checked by nobody.  They are
  assertions now (153 of them, 31 of 33 programs; a statically-rejected program is exempt
  because it never runs).  Converting a comment rather than recording a golden is what
  keeps the expectation independent of the implementation — a golden captured today would
  enshrine today's answers, shared mistakes included.
  Three ratchets keep the corpus from regrowing the hole, each proven able to fire:
  a program must produce OUTPUT (`@ORACLE_STATIC_REJECT:` to opt out), must EXIT 0
  (`@ORACLE_HALTS:`), and must carry an `assert`.  The first two are what make the third
  worth writing: a self-check that fails identically on both backends is otherwise just
  more agreement.
- **Removal:** build a **differential oracle** — run a growing program corpus on BOTH
  backends and assert they AGREE (value / null / halt / stdout / stderr / leak); these rules stay the
  written contract that GUIDES the corpus (what behaviour to cover), not a third
  implementation. A mismatch is then a divergence caught before ship, and every fixed
  divergence grows the corpus. *Chosen for now over an executable shared semantics (both
  backends conforming to one definition) — switchable to that later; these rules are reused
  either way.*

### D-op-2 — interp/native divergences are test-caught, not definition-caught
- **Violates:** E-Op / E-Uncomp / the shared-contract premise
- **Where:** the two backends are kept in agreement by the suite, so a divergence ships
  until a test happens to exercise it. **#433** is the canonical case: a program the
  interpreter evaluated fine failed to *compile* natively (`E0308`), i.e. the backends
  disagreed on a program both should accept — caught by a test, not by the definition.
- **Effect:** every codegen fix this session (the bool-arg E0308, the `__native_tail_ret`
  lift) was a backend disagreeing with the interpreter; under a shared semantics each is a
  definitional error, found before shipping.
- **Status:** OPEN — downstream of D-op-1 (the differential oracle).  The oracle now covers
  BOTH facets of "backends disagree": the run-both-and-compare (value/halt/leak) for a program
  both ACCEPT, and — as of 2026-07-04 — the accept-vs-reject *driver-agreement* for the #433
  facet itself (interp accepts a program native rejects at rustc). Closes with D-op-1.
- **Removal:** the differential oracle (D-op-1) makes "interp and native disagree on a
  program both accept" a *caught* failure (run-both-and-compare), not a coverage lottery —
  the corpus, not luck, decides what is exercised.
- ⚠ **What the differential oracle structurally CANNOT catch** (2026-08-25): a defect in the
  code the two backends SHARE.  Both consume the same parser IR, so a bad parser rewrite makes
  them agree — wrongly — and agreement is the oracle's pass condition.  Worked example: the
  defended-fault-site pass carried its own stale copy of the wrapper-getter list, so a defended
  OOB read of a non-integer vector reported while `vector<integer>` stayed silent — measured
  identical on interp and native, before AND after the fix.  So D-op-1 closing would not have
  found this, and the shared front end needs its own oracle rather than a differential one.
  Related: the reference-route discipline in [DEBUG.md](../DEBUG.md).

### D-op-10 — CLOSED (2026-09-17, loft#1548): an appended record literal minted its element before its field expressions ran

- **Violates:** `(E-Asgn-Compound)` — *step 3 reduces the right-hand side before step 4
  applies the operator*.  For `c += [S { … }]` the right-hand side is the literal, so every
  field expression runs before the append.
- **Where:** the element road of `Parser::new_record` (`parser/vectors.rs`) emitted
  `_elm = OpNewRecord(c, …)` first and each field write, with its expression, after it.
- **Symptom:** a field expression that grows `c` claimed the slot the element already held,
  and one element's writes landed on the other's — `s.qs += [Q { a: i, b: g(s) }]` read `0`
  for `24` on both backends, silently; a read of `len(c)` inside the literal saw the length
  after the mint.
- **Closed at:** `Parser::stage_append_fields` — when a field value reads the container's
  root, every scalar or text value is evaluated into a temp and every nested construction
  runs, in source order, ahead of the mint.  A record or collection VIEW keeps its place (a
  view taken before the mint could name the array the append moves).  Switch
  `LOFT_NO_APPEND_STAGING`; guard `tests/scripts/1548-an-appended-literal-reads-its-own-container.loft`.

### D-op-5 — CLOSED (2026-09-02, opened 2026-08-25): two spellings of a following null-check still reported

- **Violates:** `(E-Report)` — *"a GUARDED site (the operand of `??` / **a following
  null-check**) emits the silent `*Nullable` op and reports NOTHING (the guard owns the null)"*.
- **Where:** `rewrite_defended_fault_sites` (`parser/control.rs`) recognises a guard as
  *a following sibling `if` that tests the variable*.  The rule says "a following null-check",
  which is wider.  Measured over six spellings of the same defended read, with a logger
  attached, on both backends:

  | spelling | reports? |
  |---|---|
  | `x = v[i]; if x == null` | no ✓ |
  | `x = v[i]; if x != null` | no ✓ |
  | `x = v[i]; if x == null \|\| …` | no ✓ |
  | `x = v[i]; g = 1; if x == null` | no ✓ — **was yes; fixed 2026-08-25** |
  | `if v[i] == null` (no binding) | no ✓ — **was yes; fixed 2026-09-02** |
  | `if v[i] != null` / `if null == v[i]` (no binding) | no ✓ — same fix |
  | `if v[i] == null \|\| …` (no binding) | no ✓ — same fix |
  | `x = v[i]; match x { null => … }` | no ✓ — **was yes; fixed 2026-09-02** |

- **Effect:** a correctly defended program emits a runtime warning it did not earn.  The value
  is right in every cell — this is a REPORT-channel deviation, which is why a value-scored
  probe of this matrix comes back clean.
- **They were never one problem, and the first is now closed.** The no-binding form put the
  fault site INSIDE the test rather than before it, so there was no `Set` to rewrite and the
  pass had nothing to key on. That needed neither adjacency nor dataflow: the guard is the
  SAME expression, so the null the site produces is consumed by the comparison and no other
  reader can observe it. `rewrite_direct_null_test` swaps the operand of an equality against
  the null literal, and it DESCENDS to find that equality — `||` and `&&` short-circuit, so
  they reach the IR as a nested `if` rather than a call, and a top-level-only match saw
  nothing in `if v[i] == null || …`.
- **The `match` form closed the same day**, and needed one copy followed rather than the
  dataflow this entry expected. It has no `Value::Match` in the IR: it lowers to a nested
  block whose FIRST act is `_match_subj_N = x`, so the arms test the temp and the adjacency
  scan saw a `Block` and gave up. `block_null_tests_a_copy` follows exactly that one copy,
  from the block's own first statement, which keeps it adjacency.

  ⚠ **Its arm reads like a truthiness test and is not one**, and the difference is the whole
  licence: `match x { null => … }` becomes `OpNot(OpConvBoolFromInt(subj))`, and
  `op_conv_bool_from_long` is `val != i64::MIN` — so it asks precisely "is this not the null
  sentinel" and its negation is precisely "is this null". Measured before relying on it:
  `x = 0` and `x = 5` both take the VALUE arm, and only a real null takes the null arm.
  (An earlier note here called that lowering a truthiness test and treated it as a blocking
  question. It was neither.)

  ⚠ The arm must be a NULL test, not merely a test. `match x { 5 => … }` lowers to the same
  copy followed by `OpEqInt(subj, 5)`, the null flows through it as an ordinary operand, and
  that site still reports — `a_match_on_a_value_still_reports` is the control.
- **Status:** OPEN.  Deliberately not closed by loosening the predicate: widening "guarded"
  SUPPRESSES a diagnostic, so an over-approximation goes silent on real faults while an
  under-approximation is merely noisy.  The 2026-08-25 widening was taken only as far as a
  soundness condition allows — *nothing else touches the variable between the site and the
  check* — with three negative cells (`a_null_that_escapes_before_its_check_still_reports`)
  pinning the loud direction.
- **Guards:** `tests/runtime_logging.rs` — `a_defended_fault_site_reports_for_no_element_type`,
  `a_null_check_after_an_unrelated_statement_still_owns_the_null`,
  `a_fault_site_inside_its_own_null_test_is_guarded` (four spellings, both backends),
  `a_match_on_null_guards_its_subject`, and
  the control cells: `a_fault_site_in_a_non_null_comparison_still_reports` — `if v[i] > 3`,
  which the null flows straight through — `a_match_on_a_value_still_reports`, and
  `a_null_that_escapes_before_its_check_still_reports`.
  The controls are the load-bearing half here, because widening "guarded" SUPPRESSES.

> **D-op-4 — CLOSED (formalize4), so it is deleted from the list above.** The runtime no
> longer traps/halts on an uncomputable: div/mod-by-zero and integer overflow yield the null
> sentinel and continue on BOTH backends (E-Uncomp + E-Report), OOB already complied, and
> `NullDereference` was never raised. Guard: `tests/scripts/184-i333-div-zero-null-continues.loft`.
> The `??` trap-suppression mode is gone behaviourally, but **the `*Nullable` op split is NOT
> dead code** — an earlier version of this line called it "a separable cleanup", and measuring
> it (2026-08-25) says otherwise. The peers no longer differ in VALUE (both answer the null
> sentinel and continue), which is what that reading saw; they differ on the REPORT channel,
> which is E-Report's half of C80. The peers never call `s.raise`, so a swapped site is silent
> while an unswapped one logs — that is exactly what distinguishes a fault the program
> DEFENDED (`v[i] ?? fb`, or `x = v[i]; if x == null {…}`) from one it did not. Deleting the
> split would make every defended site report. Measured: of three sites in one program, only
> the undefended one logs, on both backends. Kept as a one-line tombstone because it reshaped two rules
> (E-Report's logging policy + the C80 refinement); see `git log` for the full entry.

## Carried by operational.md until 2026-09-04

The rules doc used to carry these beside its `OPEN` line — closure summaries, and notes on
the times the count read 0 over a live entry.  They are timeline, so they moved here
unchanged; [operational.md](operational.md) now states only what is open.

### D-op-9 (its entry, and the count/prose disagreement), D-op-7/8 pointer

D-op-9 was listed here as a fourth while the paragraph below already recorded it CLOSED
(2026-09-01) — the count and the prose disagreed, and the count is the half a reader trusts.
Kept below for its reasoning, which is still the live question for any future check on a hot
op: whether the checking form is emitted always or only under a flag is a measurement.

- **D-op-9 (CLOSED)** — (E-Report)'s soft-halt clause reaches div0 and OOB but not OVERFLOW
  (loft#1265, both backends).  Overflow is the one recoverable fault the rule ALSO
  denies a log record, so the triage flag is its only channel and it is silent there
  too.  It never reaches `raise`: `ops::op_add_long` folds it into
  `checked_add(..).unwrap_or(i64::MIN)`, a pure function with no `State`.  Closing it
  puts a null-result test on the hottest op in the language, so whether the checking
  form is emitted always or only under the flag (the `LOFT_HOIST_VERIFY` shape) is a
  measurement, not a judgement.

D-op-7 and D-op-8, both loft#1246 and both CLOSED 2026-08-31, are in the history: the
(E-Uncomp-NN) default was the range's lower bound rather than the type's, and (E-Uncomp)
did not reach a nullable narrow ALIAS at all.

**D-op-9** (loft#1265) is CLOSED 2026-09-01: `--dev-soft-halt` surfaced div0 and OOB but
not overflow, though (E-Report) names all three in one breath. Overflow was the one with
no other channel — the other two write a log record and it deliberately does not — so the
flag was the whole of its observability. It is reported from `checked_long!`'s `None` arm,
the single place an overflow becomes the sentinel and a branch that already existed, so
nothing that does not overflow gained a test; and its guarded peer `checked_long_nullable!`
stays silent, which is the same answer (E-Report) already gives that site's div0 half.
Guard `tests/dev_soft_halt_surfaces_overflow.rs`.

### the status line formal/README.md's area table carried until 2026-09-04

**rules complete for the core, 2 open** — values/null sentinels, left-to-right order, uncomputable→null (C80) + `??`, state steps; the 2 open are the META deviation D-op-1/2 (differential-not-definitional conformance), inherited by every operational file below

