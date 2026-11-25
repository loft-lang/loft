<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 228 — A variable read outside its scope is reported by the store-records checker

## Status

Open, nothing built.  [loft-lang/plans#228](https://github.com/loft-lang/plans/issues/228) is
canonical.  This plan was first written as a separate IR verifier with five rules, its own
switch and a Lean model.  On 2026-11-25 the owner narrowed it to the one rule no instrument
checks yet, built into the checker that already exists.  The reasons are under
[§ Why this shape](#why-this-shape).

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

A walk over each function body after the scope pass, beside `oracle_free_checks`
(`scopes.rs`), reached only under `LOFT_OWN_ORACLE=check`.

- It keeps the stack of enclosing block scopes (`Block::scope`, for `Block` and `Loop`).
- At each use of a variable `v`, the scope `Function::scope(v)` must be on that stack.
  Arguments and function-level variables are in scope everywhere.
- **Every spelling of a use counts, not only `Value::Var`.**  `TupleGet`, `TuplePut`,
  `CallRef`, `FnRef`, `FnRefDnr`, `Set` and `Iter` carry a variable number outside a `Var`
  node (`scopes::dominance_walk` in `src/scopes/confinement.rs` is the model).  A walk that
  reads only `Var` is right about every use it sees and blind to the rest.
- Its `_ =>` fallback states why the forms it does not name carry no variable use.
- A report names the rule (`@FR-B-Scope`), the function, the variable, the use's IR node and
  both scopes.  Its kind in the checker's table is **out of scope**, beside leak, double end,
  use after end and wrong end.

## Phases

Each phase has a positive control that must go red.

| Phase | Deliverable | Validated by (what goes red) | E |
|---|---|---|---|
| P0 probe | The slot validator over #2636 on the build before its fix | **Done 2026-11-25:** not caught (see § Why this shape) | XS |
| P1 the check | The walk above, under `LOFT_OWN_ORACLE=check`; the new kind in STORE_RECORDS.md § The checks | Control: #2636's cell (`loft-suite` `rules/binding/B-Scope/2636-…`) is reported on 02c3cf75d and clean on the build with the fix.  Then the suite and the gate corpus at HEAD: each report is fixed, or is the check's own bug, fixed in the check | S |
| P2 the sweep | `own_sweep.py` scores the kind: a report with no run-time finding is STATIC; a run-time wrong answer the check missed is MISS | `own_sweep.py --suite ../loft-suite` under every `LOFT_NO_*` switch: MISS = 0 and STATIC = 0, or each one resolved as a records gap or a real defect | S |
| P3 the rule | The owner's three-place rule for store-lifetime defects (STORE_RECORDS.md § Every new defect) also names a scope defect: the compiler, the records, and the check reporting it with the cure off | `rule_tags.py check`: the check's site cites `@FR-B-Scope` | XS |
| P4 (later) read before write | A static check that every read of a heap or reference variable follows a write on every path: the static twin of `LOFT_VERIFY_STACK` | Opened only if P1–P2 find defects the suite did not.  Control: loft#1386's shape on the build before its fix | M |

## Out of scope

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
