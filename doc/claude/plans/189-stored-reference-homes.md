<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 189 — Stored references without refusals: a record lives as long as its holder's store

Tracker: [@PLN189](https://github.com/loft-lang/plans/issues/189).

## Status — DONE 2026-10-11

`(H-RefHome)` reads *"a record a stored reference names lives as long as the store of its
holder"*.  The rule and the code that keeps it are in [formal/heap.md](../formal/heap.md)
§ `(H-RefHome)`; the history of the change is in
[formal/heap-history.md](../formal/heap-history.md) § *(H-RefHome): the holder's store keeps the
record*.

What the compiler does where a record's home would end before its holder:

- a named local held from an enclosing block moves out to that block (built before this plan);
- a literal, a call result whose callee builds into the record it is offered, or a refilled
  loop local's own record is built in the holder's store (`OpPlaceRecord`);
- a record a parameter or the returned value names is copied into that holder's store just
  before its home's store is freed (`OpEvacuate`), one copy per record and store.

Still refused, each for a stated reason (the refusal guard's header lists them): an element of
a parameter's collection or a copy into one, two parameters naming one record, a record holding
a stored reference, a callee answering a local, and a refilled local's record held from two
blocks or held in a nullable local.

Declined: reclaiming a placed record when its reference is re-pointed (step 10).  A variable
can still name the record, so a safe free needs a test on every reference write and either a
check on variable reads or a re-point counted as a view disturbance; each re-point would still
leave its 8-byte removal mark.  Recorded as a clause of C145
([DESIGN_DECISIONS_OWNERSHIP.md](../DESIGN_DECISIONS_OWNERSHIP.md)).

Found and fixed on the way: loft#1978, a reference written into a keyed collection's element
was never judged.

## Guards

- `tests/scripts/a-record-built-for-a-stored-reference-lives-in-its-holders-store.loft` — the
  placements and copy-outs (29 cells, both backends).
- `tests/scripts/a-stored-reference-cannot-outlive-the-record-it-names.loft` — the shapes still
  refused.
- `tests/scripts/a-record-a-stored-reference-names-lives-as-long-as-its-holder.loft` — the
  moved-out home.
