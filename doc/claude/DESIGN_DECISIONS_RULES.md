<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Keeping the Decision Register

How the entries of the decision register ([DESIGN_DECISIONS.md](DESIGN_DECISIONS.md) and its subject files) are read, added, reopened and held
to what they say.

## Using the register

- **Decided is not backlog.**  A decided question stays out of ROADMAP.md's milestones,
  PLANNING.md's priorities and QUALITY.md's open work; a pointer in "Out of scope" is enough.
- **Reopening needs new evidence** — a use case, an incident or a measurement not available at
  the decision.  Add it to the entry's record and change the entry here; never flip one silently.
- **Adding an entry:** take the id `./scripts/idx next-decision` names — ids are one flat
  sequence across the subject files, never a per-subject range, so a citation survives an entry
  moving to another subject.  Append the deliberation — question, evaluation, dated decision,
  revisit trigger — to the record, write the compact entry in the `DESIGN_DECISIONS_<SUBJECT>.md`
  file of the subject it decides, in the shape its entries use (**Decision** and **Why**, then
  **Revisit when**, the date and the record link), and add its row to the index,
  [DESIGN_DECISIONS.md](DESIGN_DECISIONS.md).  A decision that fits no subject file starts a new
  one.  In the source doc, strike the question (`~~…~~`) and point at the entry.
- **A decision is held to what it says (@PLN175).**  Every place that keeps it — the refusal
  site, the code that implements it, the doc that states it, and at least one guard under
  `tests/` that fails on a build breaking it — cites `@C<n>`.  `./scripts/idx tag:@C<n>` lists
  them and `./scripts/idx decisions` counts them per entry (`make index` first).  A decision no
  site can keep is not a decision: reopen it.  Where code and entry disagree, the code moves,
  unless the owner reopens the entry.  Gate (`tests/index_hygiene.rs`): every `@C<n>` names an entry, no id is
  defined twice, the index lists exactly the entries, and every entry has its guard — a new one
  lands with it.
- **A decision about a library is guarded in that library** — a test under its `tests/` citing
  `@C<n>`, run by its own CI where the library is edited.  `make guards-fetch` reads every
  `loft-libs-*` repo at `origin/main` into `index/library_guards.json` (committed), which
  `idx decisions` counts (`library`) and the gate above accepts; the `lib-main-health`
  nightly fails when a library guard appears or disappears without that file following.
- **`Catalogue:`** names the `@F`/`@I` catalogue entries a decision limits or shapes, so
  `./scripts/idx tag:@F<n>` shows a feature's design bounds beside its code (@PLN92).
