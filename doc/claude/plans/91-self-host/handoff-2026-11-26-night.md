# @PLN91 — porter handoff, 2026-11-26 (night)

Read this first.  The method, the traps and the earlier states are in `handoff-2026-11-20.md`
§ State (its top entries) and `handoff-2026-11-17.md` § Working mode.

## State — START HERE

- **int32** = 1b6753fae (loft30-gate, CLEAN: 1660/2277, 0 differ).  Both branches were rebased
  onto main 9e84211ef (2026-11-26) and are linear; promotions fast-forward int32 to loft30-gate.
- **Gating:** `git diff 1b6753fae 2feb51a44 -- selfhost/` — `par` lanes (1948 refused → agree).  It
  carries ONE expected differ, a Rust deviation (below); the tester decides how the gate counts it.
- **The tester's backlog** is `refusal-worklist.md` on loft30-gate (re-ranked by topic):
  T1 records answered from something other than a literal (current), T2 `par` workers, T3 tuples.
  T8 is not a lever.  The tester ports generic-template REGISTRATION itself (el_types.loft,
  el_definitions.loft template branches, a new el_instances.loft) — stay out of those.

## Working mode (owner, 2026-11-26)

- **No wide local sweeps.**  Per change: the rule's shape matrix (10–20 probes crossing the axes)
  and the programs the tester names; then commit and send the try.  The corpus is the tester's.
- **A new test cites the `@FR-` rule it exercises, or is not written.**  Existing cells are another
  agent's to tidy — don't edit them.
- **Read the formal rule before mirroring Rust.**  A Rust behaviour that breaks a rule is a
  deviation to REPORT; other agents fix Rust.  Open one: a plain-enum `match` with a `null` arm
  and a record arm binds a non-null local (`(N-Join)`, silent).  6af6a9bfd mirrored it in
  `el_ex_match.loft` (`null_arm && !enum_subject && sd < 0`); drop that clause once the oracle
  widens, and re-probe `rc4`/`rj_rec`/`rj_se`/`va1`.

## Lessons from today's losses (each cost a round)

- Cross the CALLEE's tail kind (a literal tail vs a renamed-local tail) in every buffer matrix.
- Rust's `ls` holds a call's own buffer as the candidate itself: "the tail borrows nothing" means
  no VISIBLE parameter's borrow — not an empty dep list (cc7aab111 → 779fe7ef3).
- A `match` without `_` ends in a synthesized `OpNullRefSentinel()`: an IR null-arm test sees it,
  Rust keys on the WRITTEN `null` (3440f55ec → 9697ee9ff).
- A method-spelled receiver is not always a call: `T.parse(…)` is a cast (86459b16a → f8f97e90d).
- Lifting a refusal exposes OLDER differences: hold the enabling change back until they are fixed.
  Before sending, score the programs a change reaches (the refusal text's programs on the base)
  and fix or re-refuse any refused → differ (2066, 2562, the `&fn` program this round).
- A shared predicate is Rust's: widening one for one road (OpVectorRef in
  `copy_source_may_be_absent`) changes every road that reads it (96a436cc3 → 5aa112b50).  And a
  container matrix crosses the LINKED layout: a keyed collection over the element anywhere.
- A lookup from the program finds a library's definition only through the definition's own
  source or source 0 (`hook_of`, `make_global`): cross "declared in a library" in any matrix.

## Open, measured (next)

- **A Rust deviation, not to mirror:** `t2 = 0; if b.mid != null { t2 += b.mid.leaf.v; }` with
  `mid: reference<Mid>?` — Rust widens `t2` to `integer?` UNDER the guard and not without it,
  backwards from `(N-Guard)` (types.md): the port keeps `integer`.  It makes
  tests/scripts/a-par-workers-record-results-reach-the-caller-with-their-stores.loft differ once
  its `par` lanes are read.  Reported to the tester (2026-11-26); for whoever fixes Rust.
- A nullable keyed join (`if c { x } else { [R {…}] }` under `-> hash<…>?`, `x: hash<…>?`): the
  `(N-Join)` arm is simple, but it exposes three older differences in
  a-nullable-keyed-collection-crosses-a-call-as-its-dense-twin (a vector parameter's tail, the
  dense accumulator, a two-parameter join's borrows) — held back.

- A link to a keyed FIELD (`a = &b.h`): Rust reads and writes through the field place
  (`t_4hash_len(OpGetField(b, …))`, `OpNewRecord(b, B, field)`) — the port has no link-place
  substitution.  Refused by name today; 1433's last function, 1836, a-null-written-….  A
  nullable keyed local's link (`h: hash<…>? ; a = &h`) is refused too.
- `g([], [Q { a: 1 }])`: the port builds the non-empty literal argument before the empty one.
- A keyed field returned off a local bound to a parameter (`b = x; b.fr_h`): Rust's body keeps
  `["x"]`, the port's derivation is not found — refused by name (2562's `fr2562_alias`).
- A struct literal's empty `[]` vector field in `main` (`S { h: [], w: [] }`): `vector<E>` made
  at another position and `main_vector<E>` left untyped (`al_2` in the session's `p/`).
- The N-Join deviation mirrored in `el_ex_match.loft` (`null_arm && !enum_subject && sd < 0`):
  drop the clause once Rust is fixed.

## Tools

The probe kit is in the c035ae98 session scratchpad (`p.sh`, `v2.sh`, `cmpv.py`, `tools4/`,
reductions in `p/`); builds in the d8b5310e scratchpad (`build.sh elabNNNN`).  `p1w.sh` now
expires its cached Rust rows when the program is newer — before, a reused reduction name read
another program's pass-1 rows.  `rs.sh FILE` / `ls.sh ELAB FILE` (Rust's / the port's listing)
are in the d87d338a scratchpad.  Attribution that works: an env-gated `panic` in the routine
(`rename_onto`) prints the loft call stack of the site.
