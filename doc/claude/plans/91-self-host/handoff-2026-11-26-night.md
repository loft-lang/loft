# @PLN91 — porter handoff, 2026-11-26 (night)

Read this first.  The method, the traps and the earlier states are in `handoff-2026-11-20.md`
§ State (its top entries) and `handoff-2026-11-17.md` § Working mode.

## State — START HERE

- **int32** = 5aa112b50 (CLEAN on the tester's stack ec2d7a451: 1638/2269, 0 differ).  The
  tester's loft30-gate 13a8d4d67 (packages, generic instance registration) is merged in.
- **Gating:** b5ad08833 (a library's element type, hook and qualified literal found from the
  program: H-Drop/2436 and tests/docs/17-libraries.loft refused → agree), sent as a `selfhost/`
  diff from 5aa112b50.  **Next try:** 07b68dd55 (an enum-qualified variant literal, `[Ch::Beta {…}]`,
  built in place).  A try is a `selfhost/` diff on the tester's last clean base.
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
- A shared predicate is Rust's: widening one for one road (OpVectorRef in
  `copy_source_may_be_absent`) changes every road that reads it (96a436cc3 → 5aa112b50).  And a
  container matrix crosses the LINKED layout: a keyed collection over the element anywhere.
- A lookup from the program finds a library's definition only through the definition's own
  source or source 0 (`hook_of`, `make_global`): cross "declared in a library" in any matrix.

## Open, measured (next)

- T1: 2561's last refusal, `enum_call` (`shapes(3)[i]` answered as `Shape?`): Rust names the
  slot first (`enum_slot_read_named`, `__ref_2 = OpGetVector(inline_container …)`) and copies
  behind its tag test.  The local form (`v[i]`, a `Shape?` field) agrees since aad53c89c.
- A fieldless enum-qualified variant as a value (`[Ch::Alpha, …]`): refused "qualified name".
- A local then-arm beside a variant (`if (s is V) { s } else { V {…} }`): Rust's `Bind` delivery.
- `nr_g`: `[for x in a.. { x }]` with `a: u8` — schema rows 100/101 in the other order.
- The tester's T9/T8 note: a record declared BELOW the function that builds it (`fn mk() -> X {
  X { a: n } } struct X …`): `literal_def` takes DT_STRUCT only, the name is a DT_UNKNOWN stub
  on pass 1.

## Tools

The probe kit is in the c035ae98 session scratchpad (`p.sh`, `v2.sh`, `cmpv.py`, `tools4/`,
reductions in `p/`); builds in the d8b5310e scratchpad (`build.sh elabNNNN`).  `p1w.sh` now
expires its cached Rust rows when the program is newer — before, a reused reduction name read
another program's pass-1 rows.  `rs.sh FILE` / `ls.sh ELAB FILE` (Rust's / the port's listing)
are in the d87d338a scratchpad.  Attribution that works: an env-gated `panic` in the routine
(`rename_onto`) prints the loft call stack of the site.
