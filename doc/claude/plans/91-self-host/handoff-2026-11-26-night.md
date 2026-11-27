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

- **2026-11-27 state:** loft30-gate 12d1c0290 (1666/2277, 0 differ).  `tuxedo-pln204-next` now
  MERGES loft30-gate (a force-push after a rebase was refused by the session's classifier); a
  try is its `selfhost/` diff from loft30-gate, or the listed commits.  Gating: 0d3a4a6e8 +
  9f2334531 (a type constructed or named above its declaration: Rust's construction and
  bare-name stubs, adopted by a struct or a variant, across files too).
- **Rust deviation, reported (T-Record):** a comprehension whose tuple body computes a text
  member (`[for i in 0..3 { (i, "x{i}") }]`) types the element `(integer, text["__work_1"])` and
  the `__tuple<integer,text>` record's FIELD `_1: text["#4"]`, a frame work text's borrow inside
  a record field.  `formal/tuples.md` (T-Record): a text member is the tuple's own text, as a
  field's.  The port keeps the shape refused (el_ex_loop `made_tup`); re-probe `tc2` once Rust
  is fixed.
- **A lambda whose TAIL is a call through a captured fn value that borrows its parameter**
  (1323's `fwd = fn(q) { inner(q) }`, probe `fr3`): Rust's tail result borrows `q` and its
  `__retbuf`/result take attribute 0 (`borrowed_view: 1`); the port's `fnref_result_type`
  mapping gives nothing for a parameter argument.  The same call NOT at the tail agrees (`fr5`).
  `call_ref` keeps such a call refused (`crt_borrows`); start from Rust's ref_return over a
  CallRef tail.
- **Score a try per DEFINITION** (the tester's gate), not per program: a change that registers
  or reorders definitions shifts numbers in refused programs too.  `defcmp.sh BASE NEW FILE`
  (c035ae98 scratchpad) counts the differing `#N` headers, `defbody.py BASE NEW FILE…` the
  definitions whose BODY equals Rust's on BASE but not on NEW.  `score_par.sh BASE NEW LIST [J]`
  is the same check on J workers (12 → ~1 min for 600 programs, the serial `score.sh` an hour
  or more).  `score.sh BASE NEW LIST` runs
  both and prints `CLEAN` only where no verdict worsens (refused → differ included), no program
  gains differing headers and no body is lost — proven on 1990's `n_j20` (a body, 2026-11-27)
  and H-Free/a-discharged-call-answer's refused → differ (a verdict).  Build the base from a detached worktree of
  loft30-gate.  Choose the LIST by the construct's keyword (every program with a `match `,
  ~1200), not by a regex for the new spelling: a lifted refusal exposes whatever older gap the
  program holds, and a narrow regex missed 10-match.loft that way.  A struct-enum twin of a shape that agrees for a record is a good place to look
  for a SILENT differ (four found on 2026-11-27: `τ?` match and `if` joins, a returned join, a
  held view-call subject).  Open: Rust's stub for a qualified `Color::Green` written above
  `enum Color` (371-p375).
- **Rust's tail prune is not ported** (`stale_visible`, control.rs ~14450, loft#1888/#2179): at
  the tail a VISIBLE parameter that neither the tail nor a body `return` reads is dropped from
  the result — one merged mid-return parameter beside a store the function made leaves the
  buffer alone; two or more stay.  Rust's pass-1 `return p` is already the buffer copy, so it
  reads the buffer, not `p`.  The port's `rename_onto` joins instead: with
  `if c { return p; } v = [8]; v` (dense or `?`) the pass-1 rows keep `p` (2 rows).  Port the
  prune at the tail, and make the pass-1 parameter `return` the copy.
- A lambda that rebinds a captured vector or keyed local (`f = fn() { v = [5]; }`): refused
  ("a binding the first pass did not see" / "written by it, of this type TyHash").  Rust clears
  the closure field (`OpClearVector(OpGetDbRef(__closure, 0))`) and builds into it.

- **A guarded projection narrows on PASS 2 only** (measured, 2026-11-26): Rust's
  `narrowed_non_null_exprs` (`b.mid != null`'s then, `h.r == null`'s else) types a block tail
  `τ`, while an inferred local keeps the `τ?` its first-pass read gave it (`z = b.mid.n` →
  `integer?`).  Not a Rust deviation, though I first reported it as one, then removed the port's
  narrowing outright (both wrong).  Probe the TYPE of each step on both sides, in a tail AND in a
  bind, before deciding what a rule does.
- 1320: the `match` arm-borrow union is fixed (both passes, 886b8e0b2); the pass-2 parameter-borrow
  mapping in `vector_buffer_call` (`named(some)` borrowing `some`) can now be retried.
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
