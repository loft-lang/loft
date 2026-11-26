# @PLN91 tester handoff — 2026-11-26

Resume point and backlog for the tester session (`tester`).  Peers: `porter` (the port, branches
tuxedo-pln204-int32 / tuxedo-pln204-next), `merger` (joins and PRs).

## State

- **Gate at the porter's 5347ca465** (int32 98288eb30): try 907a4f037 = 1613 of 2258 agree,
  0 differ, 644 refused, 228 the Rust front end refuses, 1 unaligned; lifetime 0 problems.
- **In flight at handoff:** the porter's try 5347ca465→7c957eaf2 (a call-bound arm local beside
  call arms; new cell selfhost/front/cells/a-call-bound-arm-local-beside-call-arms-keeps-the-buffer.loft),
  try commit 338eb367c on 907a4f037.  Its answer: `~/workspace/kr-tools/try-338eb367c/report.txt`
  (and the old session's scratchpad `p7c95-try.out`).  FIRST ACT on resume: read it and send the
  porter the verdict — they reported 8 wins, 0 losses locally.  The porter's next topic is "a
  function returning a record from something other than a literal" (5 programs alone).
- **Today's promotions, each clean when promoted:** 6d2205bdd (a vector match joins what each
  arm borrows), 332cba7b8 (the renamed-vector-buffer family; three rounds), c3223c9e0 (a renamed
  block-arm local beside a literal arm), 5347ca465 (a match tail answering a parameter beside a
  variant it builds; one agree→differ caught and fixed in a second round).
- **Score by day** (the try reports): 11-24 1408 → 1468, 11-25 1469 → 1595 (with the 84 port
  cells added to the corpus), 11-26 1595 → 1613.

## The refused programs, measured at 907a4f037

644 refused of the 2030 programs the Rust front end compiles (`verdicts.tsv`, families = the
reason before its `: detail`):

| families blocking a program | 1 | 2 | 3 | 4 | 5 | 6+ |
|---|---|---|---|---|---|---|
| programs | 190 | 188 | 83 | 68 | 48 | 67 |

- The 190 single-family programs spread over 148 families (122 of them hold one program).  A
  greedy clearing order admits 53 programs in its first 15 families, 3 to 5 each.
- 361 families in all: 107 met by one program, 61 by two, 66 by ten or more.  The most met:
  a call that is not of a function by its bare name (33), `if` arms of different types (28),
  a tuple with a member of this type (27).
- By area, refused of compared: suite tuples 19/22, interfaces 28/44, lib 11/18; binding, calls,
  matching, closures, iteration 43–51 %; types 74/172; ownership 50/132; tests/scripts 130/564;
  rewrites 31/309.
- Themes (programs meeting one, overlapping): vectors 257, record literals 178, returns and
  buffers 148, enum/match/pattern 104, tuples 99, nullable 96, text 88, closures 67, keyed 62,
  generics 58.
- 291 of the 551 refused programs with a first-pass reading agree on the first pass in every
  function: they need only second-pass lowering.
- 1661 definitions in 337 refused programs already differ with no refusal of their own (the
  hidden differs): each family cleared exposes some, as pln133 did twice today.

The tester's recommendation (an estimate from the overlaps, not measured): tuples as one topic
move more programs than the next fifteen small families.

## Backlog

1. **Every porter try, first.**  Base the next on the try that measured the promoted tip.
2. **The gate's DIFFER line names only a program's FIRST differing definition** (`elab_gate.py`
   `one()`); a definition not named there is not shown to agree.  Make the try report every
   differing definition of a differ program, so an exposed program's whole list arrives in one
   round.
3. **A topic for the porter** from the refused analysis above: the tuple lowering, measured on
   a port build before it is sent.
4. **Carried from 11-24-b, not re-checked:** the four interpreter faults running elab.loft
   `--interpret`; R9 (the tuple-vector registration, refusal-worklist.md § R9).
5. **@PLN229 (the C split build), the tester's own lane:**
   - the B1 stack is in `kr-tools/tmp/land-wt2`, not pushed: 186e32ca6 … c74b30e0f, then
     10bc4274a (a one-line callback definition goes to the shared unit);
   - the reuse fixes, committed there as 150a7bef1 (not pushed): constant stores named by their
     constant (`Cx::const_ident`) and each unit's head cut to the items it names
     (`units::unit_head`, with `head_items` / `declared_names`); its 5 unit tests pass
     (`cargo test --release --lib c::units`), it is NOT yet compiled on a real program;
   - next: build land-wt2's release binary, emit the elaborator split
     (`LOFT_C_UNITS=1 --native-c --native-emit`) and compile every unit with the gate flags
     (`-std=c11 -O1 -DLOFT_CALL_CHAIN`, the shared unit with `-DLOFT_SPLIT_OWN`, the others with
     `-DLOFT_SPLIT_USE`) and link — an item `unit_head` misses is a compile error, never a
     silent change; then rerun `~/workspace/kr-tools/p229/reuse.sh` / `reuse.py` (`reuse.sh LOFT OUT COMMIT…` emits from one fixed directory; `reuse.py OUT COMMIT…`
     prints units / head KB / reused) (the constant-naming run's table: `p229/reuse2.out`) over de75217e5 4e9b90aca 689edb529 d73217b03 df21c5542
     39aef5128 827e3ec4d 538a0060f 36f3b0f19 into `/var/tmp/p229-r3` — step 538a0060f is the one
     to watch (0 reused with one shared head; the head is 1526 KB per unit);
   - measured with the constant naming alone, over nine porter commits
     (`reuse.sh` / `reuse.py`): 57–63 of 63 units reused at 7 of 8 steps (it was 0 at 5 of 8);
     the one step still at 0 is a real struct change (538a0060f), the per-unit heads' target;
   - the 153-rule split-suite rerun waits for the owner's OK (the last was killed for memory);
   - before landing: rebase onto the gate tip, clippy, the suite;
   - R gaps: `opt/names.rs::disambiguated_fn_ident` hashes the absolute path; the C head
     carries a shim's absolute path (`loft_clibs[]`); repro-output.sh has no C-shim program.

## Tooling notes learned today

- An exposed differ (refused → differ) is told from a regression by `hidden.tsv` at the BASE:
  a definition listed there already differed behind the refusal.
- A refusal family is listed per program in `verdicts.tsv` (column 3, ` | `-separated); the
  programs one family alone blocks are the rows whose column 3 has no ` | `.
