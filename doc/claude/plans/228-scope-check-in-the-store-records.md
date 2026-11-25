<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 228 — A variable read outside its scope is reported by the store-records checker

## Status

Built on `tuxedo-pr2634-next3`: the scope check (P0–P3), the one lifetime report (P5), the
tester's gate (P6), one checker (P7: the `check` oracle is gone), the drop lease on by default
(P9), and P8's shapes so far.  Open: P8's last records gaps and the defects the report found,
pinned by `records_lifetime_ratchet` (6 lines over `tests/scripts` and the suite; each line is
a records gap to close in the records, or a compiler defect fixed under the three-place rule
or filed).  P4 stays closed unless the checks find what the suite did not.
[loft-lang/plans#228](https://github.com/loft-lang/plans/issues/228) is canonical.  This plan
was first written as a separate IR verifier with five rules, its own switch and a Lean model;
the owner narrowed it to the one rule no instrument checked, built into the checker that
already existed.  The reasons are under [§ Why this shape](#why-this-shape).

## Goal

After the scope pass, every use of a variable lies inside the variable's scope.  When it does
not, the store-records checker reports the variable, the use and the scope, and names the rule.
A report is a compiler defect.

The rule is `@FR-B-Scope` (`formal/`): a local belongs to the statement that declares it.  The
scope pass can break that rule without any other instrument seeing it.  In loft#2636 a local was
homed in the arm that first bound it.  A `for` loop after the `if` then took the local's slot,
and the first arm's value read back empty.  The answer was wrong on both backends and nothing
reported it.

## Why this shape

- **The store-records checker already checks the rest.**  `src/store_records.rs` and
  `src/ownership_cfg.rs` report a leak, a double end, a use after end and a wrong end, walking
  each function's control-flow graph.  `LOFT_OWN_ORACLE=check` turns them on, and
  `scripts/own_sweep.py` scores them against the run-time detectors (MISS / CAUGHT / STATIC).
  The first draft's V-Free and V-Live would have been a second copy of these checks.  The fix
  for loft#2608 already went through the store-records checker.
- **The slot validator does not see this defect.**  P0 was measured on 2026-11-25.  The build
  before #2636's fix (02c3cf75d) was run with `debug_assertions` on, so
  `variables::validate_slots` ran.  The #2636 cell compiled without a report and answered
  wrong.  The scope invariant I7 is skipped there, because the slot allocator does not track
  scopes (`scopes.rs`, "V2 is scope-blind").  So arming the slot validator in release, the
  first draft's P2, would not have caught #2636.  A debug-only guard did fire afterwards: a
  write to the locked constant store, the miscompile's effect, which no shipped build checks.
- **One checker, no new switch.**  The owner ruled that there is no switch under which loft
  verifies its own facts as a program runs (`formal/facts.md` `(Fact-Oracle)`).  The
  store-records checker is the exception already in place.  The scope check joins it under the
  same switch and the same sweep, and it keeps that checker's independence rule
  (STORE_RECORDS.md § Why a record): it reads the IR and the variable table, never a predicate
  of `src/scopes/`.

## The check

`src/scope_check.rs`: a walk over each function body of the closed program, beside the store
records' observer at the end of `close_program` (so it sees the IR the backends emit, after every
rewrite), under `LOFT_OWN_ORACLE=check` and `=check-records`.

- It keeps the stack of enclosing block scopes (`Block::scope`, for `Block` and `Loop`).
- At each use of a variable `v`, the scope `Function::scope(v)` must be on that stack.
  Arguments and function-level variables are in scope everywhere.
- **Every spelling of a use counts, not only `Value::Var`.**  `TupleGet`, `TuplePut`,
  `CallRef`, `FnRef`, `FnRefDnr`, `Set` and `Iter` carry a variable number outside a `Var`
  node (`scopes::dominance_walk` in `src/scopes/confinement.rs` is the model).  A walk that
  reads only `Var` is right about every use it sees and blind to the rest.
- It names every `Value` form, with no `_ =>` fallback.
- Not reported, each because it reads no released value: an initialisation to a constant, a
  release, a variable that holds no store (measured: on #2636's shape with the fix reversed the
  integer and text versions answer right, the vector answers empty), a block's tail variable
  after the block (the block's value, handed on), and a binder of an `if`'s condition read in
  its then branch (a pattern binds into its guard and its body).
- A report names the rule (`@FR-B-Scope`), the function, the variable, the use's IR node and
  both scopes.  Its kind in the checker's table is **out of scope**, beside leak, double end,
  use after end and wrong end.

## Phases

Each phase has a positive control that must go red.

| Phase | Deliverable | Validated by (what goes red) | E |
|---|---|---|---|
| P0 probe | The slot validator over #2636 on the build before its fix | **Done 2026-11-25:** not caught (see § Why this shape) | XS |
| P1 the check | The walk above; the new kind in STORE_RECORDS.md § The checks | **Done:** #2636's cell with the fix reversed reports `v` in all five functions, clean with it.  The suite (2139 cells) and the gate corpus (619 files) at HEAD: 2600 reports first, 0 after.  Fixed in the compiler: `(R-ForwardWalk)`'s index, `(R-LiftReuse)`'s shared lift, a hoisted vector literal's element records (`scopes/args.rs`); the rest were the check's own, fixed in the check | S |
| P2 the sweep | `own_sweep.py --wrong` scores the kind: a failed test is a fault (`wrong`) the kind answers | **Done:** #2636's cell CAUGHT (matched) with the fix reversed, CLEAN at HEAD.  The binding chapter under every `LOFT_NO_*` switch (16016 runs): the kind fires in none, so no false finding.  Its 32 wrong-only MISS rows are other rewrites' classes (`OUT_OF_LINE`, `CALLEE_DISTURB`, `OWNER_WITNESS`), and `ENDED_CAPTURE_SCOPE`'s is a parse-time name binding the IR keeps consistent | S |
| P3 the rule | The owner's three-place rule for store-lifetime defects (STORE_RECORDS.md § Every new defect) also names a scope defect: the compiler, the records, and the check reporting it with the cure off | **Done:** STORE_RECORDS.md § Every new defect; `formal/binding.md` `(B-Scope)` § Checked; the check's site cites `@FR-B-Scope` | XS |
| P5 one check, one report | `LOFT_LIFETIME_CHECK=<file>` (`-` for standard error): the store records and the scope check together, on any compile (`--emit-image` included, which runs no program); one TSV line per problem — program, function, rule, kind, site, reason — in a fixed order, and one summary line | **Done** (`src/lifetime_check.rs`): #2636's cell with the fix reversed, compile only: its five `out-of-scope` lines, `# no problems` with the fix; #2612's cell under `LOFT_NO_REF_HOME`: eight lines, the eight `RED` lines `check-records` prints on that run | S |
| P6 the gate | The tester's try gate runs it per program, compile only, and diffs the lines against the base try | A seeded defect shows as a NEW line in the gate's delta | XS (tester) |
| P7 one checker | `LOFT_OWN_ORACLE=check` (the shipped `ownership_cfg` checks) retires into the records: in 5831 runs (heap, ownership and closures chapters × 17 cure settings) it caught nothing the records did not, and reported 112 findings on runs the records judged clean | **Done.**  `own_sweep.py` reads the records alone.  Every user moved to the one report (`LOFT_LIFETIME_CHECK`, compile-only) and proved there by its own harness: `tests/ownership_oracle.rs` — the clean corpus and the 54 fuzz cells read no problem, each injected defect (a dropped free of an owned and of an adopted store, a free of a view and of a nullable view, a free of a never-free binding) reads red and clean without it, the report leaves `introspect` byte-identical; `tests/placement_parity.rs` reads no problem placed in-process and in a worker; CI's advisory `leak-scan` runs `records_lifetime_ratchet` over `tests/scripts` and the suite (3074 reports, baseline 40 lines in 18 files — the suite's attic included — each a records gap or a defect for P8).  The modes `check`, `check-dev` and `check-leak` and their code are gone; Check A's injected disagreement and Check D's dead-free `NOTE` (red on no switch) retired with them; the `cfg`, `rd`, `own` dumps and the `census` report stay | S |
| P8 the missed shapes | One shape at a time, each with its cell, under the three-place rule: loft#2511 (a view returned through a `??` local reads as a fresh copy), loft#2621 (a holder re-minted ends the records built in its store), loft#2552 (a drop hook run twice), then the sweep's 6 MISS, 8 unmatched CAUGHT and 17 STATIC | Each: its cell with the cure off reads CAUGHT, matched.  **loft#2511 done:** a stored reference read off a parameter answers `Slot`/`JoinSlot`, the caller resolves it to what its records know the argument's slot holds, and a reassigning join adopts as a first bind does; its cell and a 10-line probe read CAUGHT (re-free, matched) with the fix reversed, CLEAN with it; the whole suite reads the same with the reading on and off (`LOFT_RECORDS_NO_STORED_REF_PARAM`).  **loft#2621 done:** a record placed in a holder's store ends at that store's re-mint, and writing it into a field is reported; its cell with the fix reversed reports every function (`use-after-end`), CLEAN with the fix; the suite reads the same on and off (`LOFT_RECORDS_NO_REMINT_ENDS_PLACED`).  **loft#2552 open — needs a model of the drop lease:** a first model (a compiler copy of a type owning a droppable with no `OpCopy`; a hook on a value its hand-off flag says moved; a return of a store a path ran the hook on; an `OpFreeRef` on a store no path ran the hook on) catches all three cure switches (`NCR_ARM_MOVE`, `RETURNED_HOOK_GUARD`, `VIEW_WITNESS_BUFFER_HOOK`) and reads the cell clean with the cures on, but raises 39 STATIC rows over the suite: it does not know that a vector wrapper, a tuple member or a closure record releases its droppable by its own cascade, that a whole-value copy whose source is freed bare is a move, or which return paths settle the lease.  The lease is a fact the records must carry per store (who holds it, how it is settled), not a test at four sites.  **The sweep's 17 STATIC rows done:** one cell (O-Borrow/2338), a records gap — a loop's copy into one local is re-initialised in place, not a store per pass; heap/ownership/closures × 17 cure settings now read 0 STATIC, nothing worse.  **The two `LOFT_NO_REF_HOME` refusal cells:** `own_sweep` cannot score a refusal cell (`--tests` stops before the close); the compile-only report finds the escaping references once a holder is rooted through element reads too (`a-stored-reference-cannot-outlive-…`: 4 of its refused functions), and the suite reads the same with that on and off  **The ratchet's lines** (`records_lifetime_ratchet`, 40 when it was set): a local bound to a retained generator handle holds a hold of its own (`(G-Hold)`, control `LOFT_NO_HANDLE_RETAIN`: 2 lines), and written into a field it is that holder's hold (attic 1708); compiler defects the report found on paths their cells do not run — loft#2637 fixed (a call result in the caller's buffer passed as an argument, `LOFT_NO_RETBUF_ARG_LIFT`; 157's line), loft#2638 fixed (a callee answering its argument, where the argument is a `??` whose default the caller builds, or a literal hoisted out of a first bind: 2498, I-Comp, 1105, 1107, the keyed-projection cell — 20 lines), loft#2639 fixed (a parameter's capture cell on a path that builds no closure: 685, 695).  A scalar local holds no store (a value read through a `&integer` link: 85-poison).  loft#2641 fixed (a value-record tuple answer dropped the copy its record form handed out: attic 1820).  Still to read: B-Copy's `cond` rebound into its own `NdIf` (likely a records gap: the old value moves into the new one's field), the attic's remaining lines (1157, 1225).  A struct literal hoisted out of its argument block now takes that block's scope along, as a vector literal did (`LOFT_NO_STRUCT_LITERAL_ABSORB`): its vector field's element records kept a scope no emitted code opened | M |
| P9 the drop lease | The records carry, per store, its LEASE — the obligation to run a type's drop hook — and how it is settled: by the hook or its cascade (a vector wrapper's, a tuple's, a closure record's own), by a hand-off (`__hoff_<v>` true: the value moved), by a copy that takes it over (the displacement before a rebind), by being answered to the caller.  Reported: a lease settled twice (`@FR-H-DropOnce`), a store freed with its lease unsettled (`@FR-H-Drop`), a copy that duplicates one (`@FR-H-Lease`) | loft#2552's three cure switches CAUGHT, cures on CLEAN, and the whole suite reads the same with the lease model on and off.  The release-site tests tried first (saved attempt) caught the three switches but left 29 STATIC rows after the hand-off flag was read: tuple members, closure-record cascades, displacement copies, returns of a closure record whose captures' hooks ran, whole-value copies that hand their lease on — the shapes the model must carry.  **Done, on by default** (`LOFT_RECORDS_NO_LEASE=1` turns it off): first built as a ratchet behind `LOFT_RECORDS_LEASE=1` with a baseline of 21 STATIC rows (192 lines), each resolved in the records or the compiler: a type with no release function (a closure record that only borrows its captures) holds no lease; a free the generator does not emit releases nothing; a generator copy out of a local takes its source's lease per store — a move while the source's store is freed bare, two drops only where a hook still runs on it (reported at that hook); a record copy and an append hand their source's lease over, through a displacement, a tuple member, a member of a nested tuple held whole, and a source a block computes.  One was the compiler's: @PLN180 built a tuple twin of `m7`, a record owning a droppable, whose body freed it bare — never served (every carrier declines at the hook's mention), and no longer built.  Measured on the dev build: the suite compile-only (2139 cells) reads no `copied-lease`; with the lease off it reads exactly as before (4 lines, the two STATIC cells under P8); loft#2552's three cure switches read caught, cures on clean | M |
| P4 (later) read before write | A static check that every read of a heap or reference variable follows a write on every path: the static twin of `LOFT_VERIFY_STACK` | Opened only if P1–P2 find defects the suite did not.  Control: loft#1386's shape on the build before its fix | M |

## Out of scope

- A wrong value with no store fault behind it: an evaluation order (loft#2582), an
  interpreter-only text (loft#2526).  No record describes it; the suite's values guard it.

- A separate verifier, a `LOFT_VERIFY_IR` switch, or a second model of the store rules.
- Arming `validate.rs` in release: P0 showed it does not reach this class.
- A Lean model of the rules.
- Heap-held references (loft#1973's class): the store records' own open question.

## Documentation

- `doc/claude/STORE_RECORDS.md` § The checks: the **out of scope** kind (P1), and § Every new
  defect: scope defects included (P3).
- `doc/claude/formal/` `@FR-B-Scope`: the check named as its checking site.
- `CHANGELOG_TECHNICAL.md`: one entry per shipped phase.

## See also

- loft-lang/loft#2636 (the motivating case; its fix is 9787b6084).
- `src/store_records.rs`, `src/ownership_cfg.rs`, `scripts/own_sweep.py`.
- [STORE_RECORDS.md](../STORE_RECORDS.md), [OWNERSHIP_MODEL.md](../OWNERSHIP_MODEL.md).
