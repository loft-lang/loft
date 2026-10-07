<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Roadmap

The open plans, grouped by value category.  A plan's own issue in
[`loft-lang/plans`](https://github.com/loft-lang/plans/issues) is canonical — its phases, its
status and its dependencies live there — so this page holds only what the tracker does not:
which KIND of value each plan buys, in the order to pick from.  The methodology (categories,
no time projections, features need plans) lives in
[`plans/README.md` § Roadmap workflow](plans/README.md#roadmap-workflow).  What the roadmap
looked like before, and the narratives of past cycles, are in
[ROADMAP-history.md](ROADMAP-history.md).

| Legend | Meaning |
|---|---|
| **S / R / G / F / U / C / Q / N** | Value category (silent-failure / regression / goal / foundation / UX / clean / quality / niche).  Pick from the highest tier with open work. |
| **status** | The plan issue's `status:*` label: `active` (being worked), `next` (the next to start), `future` (designed or sketched, not scheduled), `parked` (stopped, with a reason in the issue). |

| Companion | Purpose |
|---|---|
| [`RELEASE.md`](RELEASE.md) | What must be true before tagging, and the per-cycle gate |
| [`STABILITY_ROADMAP.md`](STABILITY_ROADMAP.md) | The stability contract's open work and its floors |
| [`PLANNING.md`](PLANNING.md) | The priority-ordered backlog of work items that are not plans |
| `make work` | The open ISSUES that are pick-up work (bugs and small items; ISSUE_TRACKING.md) |

**Project goal:** browser games anyone can play via a shared link.  Native OpenGL is supported
for desktop enthusiasts; server/multiplayer comes after the single-player browser experience
works.

## Re-measuring this page

The plan list below is a copy of the tracker's open plans, so it is only as current as its
last check.  Re-measure it before relying on a row:

```sh
gh issue list -R loft-lang/plans --state open --limit 100 \
  --json number,title,labels -q '.[] | "\(.number)\t\([.labels[].name|select(startswith("status:"))]|join(","))\t\(.title)"'
```

A plan in that list and not below is a row to add; a row below that the list no longer carries
has closed and goes (its record stays in the issue).

## Scope during a cycle — the warm feature freeze

loft is stabilising toward a release that can be trusted, so a cycle's work is:

- **Making libraries work on loft** — the library system and what the libraries need from
  the language ([`lib_plans/README.md`](lib_plans/README.md)).
- **Optimisations** — libraries depend on performance, so speed and footprint work stays in
  scope.
- **Fixing existing language features** — stabilisation, bug fixes, and closing the
  deviations `doc/claude/formal/` lists (a rule that says a construct must work makes its
  absence a defect, not a feature).
- **New language features resume only once all language features are trusted to work.**

The per-cycle ship gate and the monthly branch model are
[`RELEASE.md § Release cadence`](RELEASE.md#release-cadence).  Demo applications (Brick Buster,
the moros editor) ship on their own cadence and gate no language work
([`RELEASE.md` § Explicitly out of scope here](RELEASE.md#explicitly-out-of-scope-here)); a
language bug a demo surfaces is fixed under its own category here.

## Open plans by value category

**S — Silent failure / data-loss prevention** and **R — Regression / release-blocker**: no
open plan.  Silent-wrong defects are issues, not plans: `gh issue list --label silent-wrong`.

### G — Goal-enabling

| Plan | status | What it is |
|---|---|---|
| [@PLN64](https://github.com/loft-lang/plans/issues/64) | future | Game client library |
| [@PLN65](https://github.com/loft-lang/plans/issues/65) | future | Scriptable scenes |
| [@PLN62](https://github.com/loft-lang/plans/issues/62) | future | Web IDE |
| [@PLN147](https://github.com/loft-lang/plans/issues/147) | future | The in-browser content editor — one store, edited and played by the same code |
| [@PLN168](https://github.com/loft-lang/plans/issues/168) | future | `hex_block` — the shared world query (blocked, raycast, floor height, movement resolution) |
| [@PLN169](https://github.com/loft-lang/plans/issues/169) | future | `camera` — the orbit / follow / overview camera, picker and view-cone occlusion, extracted from moros |
| [@PLN170](https://github.com/loft-lang/plans/issues/170) | future | 3D audio — spatialisation and occlusion over `audio_bus` |
| [@PLN75](https://github.com/loft-lang/plans/issues/75) | future | Physics — 2-body |
| [@PLN76](https://github.com/loft-lang/plans/issues/76) | future | Particle system |
| [@PLN56](https://github.com/loft-lang/plans/issues/56) | future | Rigged characters |
| [@PLN49](https://github.com/loft-lang/plans/issues/49) | future | dryopea — sci-fi free-build / tower-defence game |
| [@PLN51](https://github.com/loft-lang/plans/issues/51) | future | Bumper-airplanes — the next audience demo |
| [@PLN111](https://github.com/loft-lang/plans/issues/111) | parked | `graphics` — browser (`--html`) port |
| [@PLN73](https://github.com/loft-lang/plans/issues/73) | parked | Universal editor |
| [@PLN34](https://github.com/loft-lang/plans/issues/34) | parked | Native debugging — GDB / LLDB integration for `--native` builds |

### F — Foundation

| Plan | status | What it is |
|---|---|---|
| [@PLN184](https://github.com/loft-lang/plans/issues/184) | future | loft fully functional on Windows — every routine, test and script, each count under a ratchet and proven by a Windows CI leg ([CODE.md § File access](CODE.md#file-access)) |
| [@PLN148](https://github.com/loft-lang/plans/issues/148) | next | Better PHP end to end — a web client, an HTTPS server that renews its own certificate, and the libraries between them ([WEB_STACK.md](WEB_STACK.md)) |
| [@PLN72](https://github.com/loft-lang/plans/issues/72) | future | Renderer backend boundary |
| [@PLN43](https://github.com/loft-lang/plans/issues/43) | parked | loft store durability — three-tier opt-in mmap durability |

### U — Ease of use

| Plan | status | What it is |
|---|---|---|
| [@PLN171](https://github.com/loft-lang/plans/issues/171) | future | LSP / DAP follow-ups |
| [@PLN66](https://github.com/loft-lang/plans/issues/66) | future | Viewer ↔ LSP bridge |
| [@PLN70](https://github.com/loft-lang/plans/issues/70) | future | Viewer generalisation |
| [@PLN50](https://github.com/loft-lang/plans/issues/50) | future | eagleviewer — branch-aware code and docs review viewer |

### C — Clean features

| Plan | status | What it is |
|---|---|---|
| [@PLN177](https://github.com/loft-lang/plans/issues/177) | future | Growable call stack: recursion depth stops being a runtime halt |
| [@PLN15](https://github.com/loft-lang/plans/issues/15) | future | Serialisable cross-branch record references (same-store, generation-tagged) |
| [@PLN113](https://github.com/loft-lang/plans/issues/113) | parked | Contract-keyed semantics: carry both behaviours of a changed surface, keyed on the declared contract |

### Q — Internal quality

| Plan | status | What it is |
|---|---|---|
| [@PLN185](https://github.com/loft-lang/plans/issues/185) | next | A slow routine is an ordinary bug — the measurement, the 3× bar and the classes before issue-driven performance work |
| [@PLN158](https://github.com/loft-lang/plans/issues/158) | future | Release performance pass — every routine against an industry reference twin ([PERF_PORTAL.md](PERF_PORTAL.md)) |
| [@PLN179](https://github.com/loft-lang/plans/issues/179) | future | Scripts in loft — the repo's Python/bash tooling ported, each port proven against its original |
| [@PLN82](https://github.com/loft-lang/plans/issues/82) | parked | Constant store, phases B and C |

### N — Niche / opportunistic

| Plan | status | What it is |
|---|---|---|
| [@PLN60](https://github.com/loft-lang/plans/issues/60) | future | Asset pipeline |
| [@PLN67](https://github.com/loft-lang/plans/issues/67) | future | Process management library |
| [@PLN68](https://github.com/loft-lang/plans/issues/68) | future | File-system watch |
| [@PLN69](https://github.com/loft-lang/plans/issues/69) | future | Caching library |
| [@PLN91](https://github.com/loft-lang/plans/issues/91) | future | Self-hosting epic — loft-in-loft, an ANSI-C backend, small-board portability |
