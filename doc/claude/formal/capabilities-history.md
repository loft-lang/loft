<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# formal/capabilities-history.md — the deviation register for [capabilities.md](capabilities.md)

> **The rules are next door.**  [capabilities.md](capabilities.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

OPEN: **0** — ✓ the capabilities area is now FORMAL. `Cap-Call` (the call gate + closures),
`Cap-Read`/`Cap-Write` (the field rights), `Cap-Set` (the parameter `#default` lock), AND `Cap-Own`
(script-owned mutation, incl. vector-element writes) are all enforced today, each with a RED/GREEN
adversarial pair. Every host-touching operation in a restricted context is decided by exactly one
of the six rules.

**`Cap-Set` (the parameter `#default` lock) — CLOSED (2026-07-04).** A `group#default` link on a
parameter (`count: integer = 1 spawn.count#default`) now parses (`definitions.rs`, first pass →
`pending_param_locks` → `param_locks`) and gates at a sandboxed call site: an argument that
DIFFERS from the parameter's default without the lock granted is a violation
(`param_lock_violations`; an argument equal to the default is not an override). Group-existence
validation + `member_access` IR persistence (6.8) landed alongside. Proven by the non-vacuous
RED/GREEN twin `param #default lock override` in `access_corpus_red_green` (`src/parser/mod.rs`):
`spawn("g", 5)` is rejected under `world#append` alone and admitted once `spawn.count#default` is
granted.

### D-cap-2 — a closure may carry authority across the boundary — CLOSED (2026-07-04)
- **Violates:** Cap-Call (completeness — an indirect call must resolve to its callee's gate)
- **What it WAS (corrected by probing — the earlier framing was off).** A lambda def created in
  a sandboxed body was never added to `def_sandbox`, so the admission walk treated it as an
  **untagged leaf** (`sandboxed \`f\` reaches \`__lambda_N\` … neither an allowed library nor a
  granted capability`). That was SOUND (no escape — every lambda rejected) but a **blunt
  over-reject**: even a script-only `[1,2,3].map(|y| y*2)` was rejected, making lambdas unusable
  in sandboxed code, and a lambda that DID reach a host cap was rejected without naming the real
  reach.
- **The fix (`src/parser/vectors.rs::mark_lambda_sandboxed`, called from `parse_lambda` /
  `parse_lambda_short`).** A lambda created while its enclosing def is sandboxed is itself marked
  sandboxed under the SAME profile, so the admission walk **DESCENDS into its body** (checks its
  calls / fn-refs / raw-writes precisely) instead of stopping at an untagged leaf. Nested lambdas
  inherit transitively; a no-op outside a sandbox (`def_sandbox` empty), so non-sandbox lowering
  is byte-identical.
- **Why this is complete for the closure class** (probed): (1) a host call in the lambda body →
  caught by descending, naming the reach; (2) a captured host **fn-ref** (`cap = host_fn; …|y|
  cap()…`) → gated at its CREATION site in the enclosing def (the `cap = host_fn` Set that
  `referenced_defs` records — the capture cannot outrun that); (3) a raw write to a captured host
  **struct** is not an escape — writing a captured struct field is an unsupported construct that
  panics codegen on BOTH backends, so it can never run. A fn-ref laundered through a host-call
  RETURN then captured is the separate L4-return residual (`sandbox.rs::referenced_defs` §RESIDUAL,
  not closure-specific), not D-cap-2.
- **Proven by:** the escape `cap: lambda body reaches ungranted host (D-cap-2)` + the control
  `script-only lambda is usable (D-cap-2)` in `admission_escape_suite_rejects_every_breakout`, and
  the non-vacuous RED/GREEN twin `lambda body reaches host cap (D-cap-2)` in
  `access_corpus_red_green`.

### D-cap-3 — script-owned vector-element writes were rejected — CLOSED (2026-07-04)
- **Violates:** Cap-Own (completeness — a script-owned mutation should be admitted, not rejected)
- **What it WAS.** `src/parser/expressions.rs::raw_write_is_host_owned` gates a raw field/index
  write before it is recorded. It admitted the common owned case (`s.f = v` on a non-parameter
  local of a script-defined struct) but rejected EVERY write whose root type was not a
  `Reference(struct)` — so a **script-owned vector element write** `v[i] = e` on a local `v`
  fell to host and was rejected. Sound but incomplete.
- **The boundary — corrected by @PLN102 F6 (2026-07-11).** A **plain** vector bind copies (a
  literal, `c = v`, a projection `fv = e.items`, a slice `s = v[0..2]` — all leave the source
  untouched, `x[0] = 99` ⇒ `orig[0] == 1`), so a plain-copied local vector is script-owned. But
  the original close made a **false claim** — that "even `r = &v` copies". It does NOT: an explicit
  `&`-bind is a **live reference** (heap.md `H-Ref`, C77), so `r = &v; r[0] = 99` ⇒ `v[0] == 99`.
  A vector **parameter** likewise aliases the caller. So a write is host iff its target root is a
  **parameter OR aliases one through a `&`-bind** (directly `r = &v`, or transitively `b = &a;
  a = &v`) — not merely "the root is a parameter".
- **The hole this left (found + closed).** The first fix keyed only on `arguments()` (the direct
  parameter root). A `r = &param` binds `r` typed `Vector` (with a dep on the arg), so `r[i] = e`
  slipped past the `arguments()` check into the owned arm and was **admitted** — laundering a host
  write. The admission escape suite's `raw-write: & alias launders param` confirmed it.
- **The fix.** `raw_write_is_host_owned`'s `Type::Vector` arm now **follows the dep chain**
  (`root_aliases_argument`): a local vector whose deps reach a parameter is host; a genuinely-copied
  one (deps only on a fresh local store) stays script-owned (`Cap-Own`). A `&`/`RefVar` scalar
  borrow, an iterator (an inline slice), a scalar, or an unresolvable base still falls to the
  conservative host default; a direct parameter root is still caught by `arguments()` first. The
  struct handling is unchanged. So there is **no aliasing residual** — the `&` cases are no longer
  assumed copies, they are followed.
- **Proven by:** the escapes `raw-write: index` (a `v[i] = …` on a PARAMETER root → rejected) and
  `raw-write: & alias launders param` (a `r = &v; r[i] = …` alias of a parameter → rejected, the
  F6 fix) + the control `script-owned vector element write (D-cap-3)` (a `v[i] = …` on a plain
  LOCAL → admitted) in `admission_escape_suite_rejects_every_breakout`; lib suite green (722). RESIDUAL (conservative, safe): a `v[i] = …` on a locally-owned KEYED
  collection (hash/sorted/…) still falls to the host default — rarer, and an over-reject not an
  escape; widen the arm the same way if a consumer needs it.

## Carried by capabilities.md until 2026-09-04

The rules doc used to carry these beside its `OPEN` line — closure summaries, and notes on
the times the count read 0 over a live entry.  They are timeline, so they moved here
unchanged; [capabilities.md](capabilities.md) now states only what is open.

### the D-cap-3 gap (@PLN102 F6) — the week this read OPEN 0 over a live escape

> **History — the D-cap-3 gap (@PLN102 F6).** From 2026-07-04 to 2026-07-11 this "OPEN 0" claim
> was **over-stated**: the script-owned-vector rule rested on a false memory-model premise ("even
> `r = &v` copies"), leaving a real escape — a `r = &param; r[i] = e` laundered a host-vector
> write past the gate. It was found the right way, by *extending the falsifying suite* with the
> `&`-alias case, and closed by making the gate follow the dep chain (above). The lesson the
> freeze records: "OPEN 0 / by construction" is only as strong as the escape suite is complete —
> here a spec contradiction (heap.md said `r = &v` copies; reality aliases) was the tell that the
> suite had a blind spot.

### the status line formal/README.md's area table carried until 2026-09-04

**0 open** (2026-07-04) — the 6-rule judgment `P;ctx ⊢ e ✓` fully enforced; D-cap-1/2/3 CLOSED, each with a RED/GREEN adversarial pair. Cites ownership.md/heap.md for the owned-vs-host fact

