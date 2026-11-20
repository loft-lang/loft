<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 — tester handoff, 2026-11-19 late

The tester of the @PLN91 port (gate E): gate the porter's tips and tries, analyse every result
to *what to code*, keep two prepared subjects ahead of the porter, and fix the Rust defects the
work turns up (matrix first, chokepoint, guard cell, both backends).  The rules of the role:
[porter-ledger.md § What the testing side is for](porter-ledger.md); the instruments and the
earlier state: [tester-handoff-2026-11-19.md](tester-handoff-2026-11-19.md).

## Peers

| who | session (address with `SendMessage`) | list name | what |
|---|---|---|---|
| the porter | `bridge:session_015fP1u2RTCyGqEvFJ5edvkL` | "Front-end lambda, iterator match handoff" [58066d] | ports the elaborator; its branch `tuxedo-pln204-next`, promoted tips on `tuxedo-pln204-int32`; its own handoff `handoff-2026-11-18.md` § "2026-11-19 night" |
| loft3 | `bridge:session_015fP1u3cfvftqdhgoqxjCF9` | "Parallel threading test" [a82a0d] | joins and PRs to main; its next branch `tuxedo-pr2586-next` |
| the tester (this role) | `bridge:session_015fP1u4eHgrMvpsjdzE7AUg` | "loft-36" [50a9da] | |

The owner cleared the porter's session at the end of 2026-11-19: its address may stay, its
context does not.  Check `ListAgents` before writing to it, and point a fresh porter session at
this page and at its own handoff.  Another agent is bringing the C and interpreter stores closer
together; a defect in the store runtimes is its, not the tester's (report with a repro).

## State at the hand-over

- `loft30-gate` (the porter's oracle, worktree `~/workspace/loft-front` / `loft-docs`) carries
  main 369fcd3ef (PR #2586) and every tester fix below.  Last porter cycle gated: b5c1dd355
  (1223 of 2114).  Last try analysed: **b94e088a2 on acf37b75e — 1226 of 2114**, two regressions
  of one cause, not to be promoted.
- **518543a5e was gated as a TRY** (a bundle of 7a644f558..518543a5e through `try.sh`, so
  `try-518543a5e/` holds verdicts and a tools_base): the route for a porter base that cannot be
  promoted.  Never `cycle.sh` a commit by hand: `tip_test.sh` can reset `loft30-gate` to it.
- The watcher (`~/workspace/kr-tools/auto.sh`, run under Monitor with the `^REPORT|FAILED|^TRY|
  ^MAINCHECK|BASELINE|DID NOT BUILD|NOT FOUND|MERGE` filter) expires every 30 minutes: re-arm it
  at every expiry notice; a lapse delayed the porter's reports by an hour on 11-19.

## The porter's next task (its handoff names it too)

b94e088a2's two regressions (a-run-of-pushes-through-a-loops-header…, R-ElementPath's schema):
a declared-empty NARROW vector (`b: vector<u8> = []`) registers nothing on pass 1 in Rust; its
rows come at the append, in source order (`100 vector<float>, 101 vector<byte>`), and alone
after the structs (`100:S 101:main_vector<integer(0,255)> 102:vector<byte>`).  The porter's
lead: `first_vector_bind` calls `make_vector_def` with `narrow_early: true`.  Also on its tip,
unfinished: 4c24c290d (a `null` operand of `&&`/`||`).

## Backlog (tracker `~/workspace/kr-tools/backlog.md`)

Untouched, judged on three backends: `call-index-element-write.md` (10 programs),
`postfix-default.md` (`e?` lowers as `e ?? []`, 18 programs).  In progress (porter):
`record-return-verdicts.md` — Bind group (vector_match, scalar_match, `if` arms, fn-value
calls) done or in its tries; the `if`-arm copy declines where an arm hands back a parameter.
Refill to two untouched on every porter event; rank families by programs they ALONE block
(the `awk` over `verdicts.tsv` in this conversation's history: sole vs met per family).

## Rust fixes of 2026-11-19 (all on loft30-gate)

| issue | what | on loft3's next branch |
|---|---|---|
| #2579–#2582 | loop sources held once; a place addressed once; a block index once; indices before a lifted right side | in main (PR #2586) |
| #2580 follow-up, #2584 | `=` of a collection lifts call indices into temps; a reading index stays inline only where pass 1 saw a right side with no call or block (`effect_free_rhs`) | in main |
| 62a90f9de | a block counts as an addressing effect only when it writes (PR #2586's ICE) | in main |
| #2585 (closed) | a parameter's element tail, a nullable answer (present → copy), an early `return NUMS` — all copied into the return buffer; census decline of 17_consumer_moros_dryopea re-pinned | yes (96698809a…) |
| #2587 | a boolean literal sub-pattern is compared, not taken as the test (`(P-Point)`) | yes (3f9ee67c0) |

## Habits the day taught

- Quote the LANDED sha (`land.sh` cherry-picks), never the local one.
- Before landing a parser fix: the optional-shape ratchet (`ir_walker_audit.py optional
  --check-ratchet`, re-pin a fall with QUALITY.md's row) and `doc_hygiene`.
- Check a formal rule exists before citing it (`(P-Lit)` did not; it is `(P-Point)`).
- "Applies cleanly" means a real cherry-pick and build on the peer's tip; `merge-tree` piped
  into `head` hides its exit status.
- A suggested Rust rule for the porter is checked against the porter's other sites of the same
  construct first (two wrong rules on 11-19: the `while` bookkeeping, the `??` owner).
- `cargo test --test slot_matrix` runs alone, or under `systemd-run --user --scope -p
  MemoryMax=8G`: unserialised it OOM-killed the box and this session.
