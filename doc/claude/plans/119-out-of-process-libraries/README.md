<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 119 — Out-of-process libraries: placement as policy, the store as the wire

## Status — DONE, 2026-08-11

Every arc **A–F** built and every consumer this plan named shipped.  A library
declares where it runs in one line of its own manifest, and its consumers call it
unchanged — in this process, in a worker, or on another machine.

**The mechanism and the authoring rules live in
[PLACEMENT.md](../../PLACEMENT.md)**; the user-facing declaration is in
[PACKAGES_MANIFEST_TABLES.md § `placement`](../../PACKAGES_MANIFEST_TABLES.md).  This file is the closure record:
what was decided, what the plan got wrong about itself, and what it found on the
way.

| arc | shipped |
|---|---|
| **A** declaration + attach handshake | `placement = "process"`, a worker per placed library, scalars and text crossing |
| **B** the boundary marshal | the **arena** — a shared store, with `copy_claims` as the marshal; structs and vectors at any depth |
| **C** ownership across the boundary | the delivery three-way, the `const` no-write skip, a failed crossing that maps neither arena |
| **D** fault isolation | a worker that dies is an error rather than a hang, and the caller's stores are checked before it is told |
| **E** `placement = "remote"` | `loft --lib-server`, the arena's live bytes on a socket |
| **F** consumers | `lib/git` (which deleted the viewer's 135 lines of bash), the engine-host wire, the tracker index |

Gates: `tests/placement_parity.rs` · `tests/placement_remote.rs` ·
`tests/placement_worker.rs` · `tests/lib_git.rs` · `tests/engine_host_placed.rs`
· the git-carries property in `tests/index_hygiene.rs`.

## The one invariant

> **A call to a library is indistinguishable — in type, effect,
> ownership/lifetime, and error behaviour — from the same call in-process.
> Where it runs is deployment policy, not source.**

Held on three placements, by one consumer and library run unchanged across all of
them.

## Why not a subprocess primitive

> **Superseded 2026-09-30 (owner):** loft MAY run a subprocess — [@PLN179 strand
> 4](../179-scripts-in-loft/README.md) — under two rules that keep what this section
> protects: the command is a typed format string (@PLN124), so a value can never become
> syntax, and `run` drains every stream, so a child can never deadlock the caller.  The
> argument below is kept as the record of why a `{stdout, stderr, code}` triple alone was
> declined, and the closed-vocabulary `lib/git` stays as the first library twinned over `run`.

`run(cmd, args)` is a *second, weaker interface* beside the one loft already has.
The library interface carries typed signatures, structs, enums, vectors, tuples,
methods, coroutines, effects and capability admission; a `{stdout, stderr, code}`
triple carries bytes and an exit status, and every consumer of it re-parses text
loft already knows how to type.  A general `run()` stays declined
(this section is its record — no `DESIGN_DECISIONS.md` entry exists, and @PLN179 relies
on it); `lib_plans/67-process` is
superseded, and its whole consumer list was served by the typed-library route.

The one thing experience added: **"sealed behind the contract" has to be taken
literally**, or it degrades into the argv-not-a-string rule this argument says it
beats.  `lib/git`'s native is a closed query vocabulary — the caller names a
question and loft builds the command — so `git -c core.sshCommand=…` is
unreachable by construction rather than by filtering.

## Answered design questions

1. **Text residency** — GREEN.  Text, `vector<text>` included, is store-resident
   and crosses intact; the paged relocator's `vector<text>` refusal is a different
   path (it RELOCATES into a different store) and never bounded this design.  Its
   sub-question — is a caller's local already arena-resident? — answered **one
   copy at the boundary, and for an argument, two**.  In-place was never
   available: the arena must be a store neither side's crash can corrupt, so it is
   a THIRD store and a local is not in it.  Sharing the caller's heap would have
   deleted the fault isolation arc D exists to provide.
2. **Ownership of a returned graph** — answered, and **not with the hypothesis**.
   The hypothesis was "the callee allocates in the arena the caller owns, and
   `OpFreeRef` frees it".  The arena is not the caller's; it is a third store,
   reset per call.  The rule is instead **the caller's own emitted code decides,
   and the crossing must match it** — @PLN103's delivery three-way, now one home
   that both marking and the worker's free consult.
3. **Effect classification** — **no new category, structurally**.  Marking happens
   after `scopes::check`, so a placed call carries exactly the effects the
   in-process one does: there was never a cross-placement effect to classify.
   Par-safety is therefore a runtime question, and it is green under both
   parent-sharing modes.
4. **Crossing cost** — affordable, and it **dictated the handshake**: the naive
   "wake the worker" futex is 30× the spin-then-sleep one, so the naive version is
   a performance bug rather than a simpler variant.  Numbers in
   [PLACEMENT.md § Cost](../../PLACEMENT.md).
5. **Writer discipline under real database support** — still open, and the only
   open question left here.  The MVP is share-read-only plus explicit write-back;
   when full DB access lands, does the writer path become a journal transaction,
   and does that change the invariant's error behaviour?  Nothing in the built
   system prejudges it.

## Where the plan was wrong about itself

Worth keeping, because each was a coherent belief that a measurement or a test
overturned.

- **Q4's 130 ns was a bare wire ping, and taking it for the cost of a *call* was
  wrong by 30×.**  A real call cost 4.7 µs, of which 4.4 µs was the fault-site
  span table being deep-cloned on every entry into loft — nothing to do with the
  wire.  What made it findable was a **sweep**: three spin budgets, 2000 through
  100000, produced the same time, and that flat line said "look elsewhere" before
  any code changed on the strength of a coherent story.
- **Arc E's assumed mechanism was the wrong one.**  The row read "over the
  existing paged / Range reader" and marked itself blocked on
  [#632](https://github.com/loft-lang/loft/issues/632).  The Range reader is right
  for data AT REST and wrong for a CALL — an arena is small, written once and read
  once, so a page-fetch round trip per page is strictly worse than sending it.
  #632 was never a blocker.
- **"Run the @PLN94 oracle on both placements" is vacuous.**  It runs inside
  `scopes::check`, before marking, so it sees byte-identical IR either way and
  agrees by having nothing to disagree about.  The question that CAN fail is
  whether the runtime marshal obeys the ownership the static analysis already
  assigns — and that found a leak on the first try.
- **Arc B's ownership rule was a two-way where the language has a three-way.**  A
  `View` return leaks per call when placed.
- **The gate's leak half was corroboration, not a gate.**  `check_store_leaks`
  reports what is unfreed AT EXIT, so it is structurally blind to
  allocate-per-call-and-free — exactly a placed call's shape.  Made falsifiable
  with `LOFT_STRICT_STORES`, it immediately caught three faults.

## What the dogfood found

None of these were this plan's own code.  They are the argument for building a
real consumer rather than a probe.

| found | surfaced by |
|---|---|
| **An internal compiler error** — assigning to a file-scope `NAME: text = …` (a CONSTANT, inlined at every use) crashed the compiler instead of diagnosing | writing `lib/git` |
| **A placed library resolved a relative path elsewhere** — TWO anchors (the process cwd, and loft's `source_dir`), and a worker inherited neither | `lib/git` answering "not a git repository" in one placement |
| **Two shared-allocator bugs** — `adopt_store` pushed a slot instead of taking the watermark; `release_slot` never lowered `max` | the arena borrowing a slot per CALL |
| **`fl_rebuild` looped forever on a zero-size block** — a malformed store was a hang in release rather than a diagnosis | a remote arena image's short tail |
| **`vector_def` left a stale `def_names` alias** — killing a live-reload session on the first bad edit | `engine_host::turn() -> Turn` |
| **`lib/git` was interpreter-only** — the native backend keys its runtime table by loft DEF NAME, and nothing registered `n_git_query` | `make index` compiling its scanner |
| **The tracker index's file list had drifted both ways** — four tracked source trees never indexed, one scratch directory indexed | asking git instead of mirroring `.gitignore` |

Two lessons generalise beyond this plan:

- **A worker inherits an ENVIRONMENT, not only a frame.**  Every matrix cell
  before the cwd one passed a value ACROSS the boundary; that one asked what the
  far side already IS, and nothing had tested it.
- **A hand-maintained mirror of another tool's knowledge is stale in both
  directions.**  The indexer's skip list and its root list were both copies of
  `.gitignore`, and each had a gap the other did not.

## Deliberately not done

- A polymorphic **enum** and a **keyed collection** do not cross.  Both are
  reference-shaped and so look placeable; the risk was never that they fail but
  that they are quietly marked and read as the wrong shape, so the refusal is
  pinned by a test.
- **`--native` does not place**, and says so under `LOFT_REQUIRE_PLACEMENT=1`.
  Making it place is its own arc: the generated Rust would have to call the
  dispatcher rather than the body.
- **`lib/engine_host` declares no `placement`.**  It is placeable and proven
  against a live client; flipping it changes where a game's sockets live, and the
  games that consume it are in other repositories.  That is the owner's call.

## Probes

`probes/q4_crossing.rs` + `probes/q4_inproc.loft` are Q4's measurement, kept as
the evidence for the handshake design — including the control row (a zero spin
budget lands on the futex number, proving the sleep path is genuinely taken).
The behavioural probes graduated to the gates listed under Status.

## See also

- [PLACEMENT.md](../../PLACEMENT.md) — the mechanism and the authoring rules.
- [PACKAGES.md](../../PACKAGES.md) — declaring `placement`; what a consumer sees.
- § Why not a subprocess primitive, above
- [lib_plans/67-process](../../lib_plans/67-process/README.md) — superseded; its
  consumer list is fully discharged.
- @PLN119 — <https://github.com/loft-lang/plans/issues/119> (this plan).
