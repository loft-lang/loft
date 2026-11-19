# @PLN91 — the tester's handoff, 2026-11-19

Supersedes [tester-handoff-2026-11-18-night.md](tester-handoff-2026-11-18-night.md) for the
state; that file still describes the try-gate and the worktrees, and
[tester-handoff-2026-11-18.md](tester-handoff-2026-11-18.md) the cycle scripts.  Read
[porter-ledger.md](porter-ledger.md) § What the testing side is for FIRST.

## The owner's priority

"Make sure you do not reimplement bugs and I am happy, as long as loft gets better for it you
can take months of time", and: "your current way of working with testing before giving it to
the porter is exactly what will prevent that from happening.  Stay vigilant."  So a form is
JUDGED before the port carries it — matrix, both backends, poison and leak checks — and the
agreeing count is not the goal.  It is in the ledger.

## State

- Porter tip **ca073838f** on `tuxedo-pln204-int32`: gate E **1158 of 2088** agree, 0 differ, no
  definition drop.  Its real-gate baseline (run 44426165159) was still running.
- Try **e5540ec14** on that tip had ISSUES (1 differ) and is ANSWERED: the analysis is on
  loft#2560 and was sent.  The try is sound; it uncovers the `?` discharge of a nullable vector
  (`for q in src.split(',')? { … }`): Rust's fallback is a null vector local `_vec_1`, the port
  builds `__vdb_1`.  Judged usable (eight cases, both backends): the porter carries it or
  refuses the shape by name.  Expect a stacked try.
- The porter is on [vector-slice.md](vector-slice.md): § 1 and § 2 carried, § 3 (vector
  elements) next.  Prepared and unused: [linked-group-literal.md](linked-group-literal.md).
  The buffer is at ONE: pick the next from `kr-tools/next_moves.py <tip>`.

## Ruled and judged since the last handoff

- **@C168 (owner):** where a function is declared changes nothing a program can observe; its
  internal numbers may differ.  The numbering change that was built is dropped.  Guard:
  `tests/scripts/a-callee-declared-below-its-caller-changes-nothing-a-program-can-see.loft`
  (the port agrees on it).
- **loft#2564, fixed on `loft30-gate` (0b5e39f8d):** a local bound from a later-declared
  function and then changed with `+=` or `*=` was refused as a type change.  A compound
  assignment now leaves such a local untyped on pass 1 (`Parser::untyped_on_pass1`).  The
  definitions of all corpus programs are unchanged.  Open until the branch reaches main; it
  came after the gather session's PR cut (595074b4b), so it is for the next PR.
- **loft#2561's fix leaked** on PR #2565's language suite (an arm answering a call's own
  result was bound as a view).  The gather session fixed it forward (516000b77, not pushed
  when this was written) and ran [probes/branch-arm-absence.loft](probes/branch-arm-absence.loft)
  on it: all eight pass.  The lesson: a Rust fix that CHANGES lowering runs the language
  suite on both backends before it lands; one proven byte-identical over the corpus does not.
- Judged harmless, the porter may carry or refuse: the element borrow of a vector literal of
  vector-returning calls ([probes/vector-literal-of-calls.loft](probes/vector-literal-of-calls.loft));
  the keyed local bound from a later-declared function; where the `vector<R>` type is created
  for a slice that is its first use (`field_first_bind` in the slice probe).
- Judged a rule, not a defect: `(H-Drop-Not)` — a keyed collection's records run no `OpDrop`.

## Instruments built this session (the tester's box)

- **Per-case gating of a probe:** split it into one-function programs and run
  `selfhost/front/elab_gate.py --show 5 <file>` in `~/workspace/kr-try` with
  `LOFT_ELAB_DEFS` / `LOFT_ELAB_BIN` set to `kr-tools/try-<sha>/{defs/defs,elab/elab}`.  A
  helper whose tail is a call (`fn f() -> vector<T> { g() }`) is itself refused and pollutes
  every case.
- **Above/below matrix:** one caller compiled with its callees above and below, both
  backends, the caller's definition compared (`kr-tools/order-free/gen.py`, `run.sh`).
- **Corpus-wide definition dump** (`kr-tools/order-free/dump_all.py LOFT DEFS OUT FILELIST`,
  2086 programs in about 20 seconds) and the diff of two dumps (`cmp.py`): what a Rust change
  moves, before it lands.
- `kr-tools/try_poll.sh` pages the mailbox's comments now (it read one page of 100).
- `scripts/find_problems.sh --bg` removes the previous run's summary first (88f660f50): the
  file is named for the checkout, and an old one read as the new run's verdict.

## The slot matrix in the tester's inspections (owner, 2026-11-19)

loft30-gate carries @PLN221's `scripts/slot_matrix.py` (merged from tuxedo-pr2438-post at
53d2de5a8). Use it wherever a judgment is "does this statement hold for every form of its
operand":

- **Judging a porter subject:** `python3 scripts/slot_matrix.py cell '<stmt, <S> the operand>'
  --type <int|vint|text|rec|coll|…> --backend both` runs the shape over every filler of that
  type on the interpreter and C with the count, order, value and leak checks, each against its
  bind-first twin. A statement may declare its own locals (`tq = (1, "a"); tq.0 += <S>;`). A
  failing cell is an oracle defect: check `found.md` and loft3's -next before treating it as
  new, and name it on the subject page as a form NOT to mirror.
- **Verifying a Rust fix:** `slot_matrix.py gate` (or `gate --family F`) before landing. A fix
  may only remove baseline lines (`gate --update`, then read the diff). A cell the fix makes
  pass can expose its twin as NEW (the rd-coal-rec-left twin did).
- **Another build:** `LOFT=<binary>` points the tool at it, e.g. a peer's -next built in a
  worktree.
- **Load:** one gate runs half the cores' worth of C compiles. `cargo test --test slot_matrix`
  serialises its seven family gates (4e19e802d): run unserialised, they OOM-killed this 14 GiB
  laptop. Run it alone, or under `systemd-run --user --scope -p MemoryMax=8G`.

## Quoting a commit to a peer

`kr-tools/land.sh` CHERRY-PICKS onto the remote tip, so the landed commit has a new sha: the one
it prints (`LANDED <sha>`) is the one on origin, and the local commit's sha exists only in the
landing worktree. Quote the LANDED sha in every message and issue comment. On 2026-11-19 two
messages to loft3 named the local shas (288939d44, 120d959a9), which origin does not have.

## Peers

- The PORTER: `SendMessage` to `bridge:session_015fP1u2RTCyGqEvFJ5edvkL` (shown as
  "tuxedo-robust-lobster").  Its replies arrive; the tool never confirms delivery.
- The gather session (loft3, the PRs): `bridge:session_015fP1u3cfvftqdhgoqxjCF9`.

## Not done

- rustc-native was not run on the loft#2564 fix (first-pass typing only).
- `index_hygiene_clean` fails on the branch for decisions 161 and 162 (no guard), not for 168.
- Two of the seven programs under the later-declared refusal were not run alone
  (`L-Narrow/1437`, `lib/audience_crystal`).
