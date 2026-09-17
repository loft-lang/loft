<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Releases — one directory per cycle

Each release cycle (the `YYYY-MM` branch) has a directory here.  The process that every
cycle follows is [RELEASE.md](../RELEASE.md); this tree is what each cycle did with it.

| cycle | what the record holds |
|---|---|
| [`0.8.4`](0.8.4/README.md) | pre-calendar; the 0.8.4 milestone tracker |
| [`2026-06`](2026-06/README.md) | first monthly cycle, and the switch to calendar versioning |
| [`2026-07`](2026-07/README.md) | candidate release blockers — stability, the registry, library hardening |
| [`2026-08`](2026-08/README.md) | release state (prep 2026-08-01): the heap-correctness body of work |
| [`2026-09`](2026-09/README.md) | release state (prep 2026-09-04): both streams joined; valgrind red put to the owner |
| [`2026-10`](2026-10/README.md) | release state (opened mid-cycle 2026-09-16): the halfway audit — what the mid rows report and what the cycle owes |

## What a cycle's directory holds

- **`README.md`** — the state write-up: the tracker census at prep time, the gate evidence
  (which run, on which commit, what it reported), the reviews' findings, and every decision
  taken or put to the owner.  It is written during prep and finished when the tag lands.
- **`checklist.json`** — `make release-checklist`'s record of the manual items: which
  `M-*` step was marked done, when, and the evidence note recorded with it.  The script
  reads and writes it here, so a tick made on one machine is a tick everywhere, and the
  evidence survives the box it was gathered on.  (It used to live in a git-ignored
  `.release-checklist/`, which made every `[x]` local to whoever ran it.)  The trade is
  that a tick dirties the tree: `A-clean` reads red until the tick is committed, which is
  the point — the record is part of the release.

Anything else a cycle produces that is worth keeping beside its record — a valgrind
`results.tsv`, a release-gate run id — goes in the same directory.

## What does NOT go here

The gates themselves, the review procedures, the tag-and-publish mechanics: those are the
same for every cycle and live once, in [RELEASE.md](../RELEASE.md).  A cycle's directory
cites them; it never restates them.  The same rule the plans tree keeps between
[plans/README.md](../plans/README.md) and the docs that describe how things work.
