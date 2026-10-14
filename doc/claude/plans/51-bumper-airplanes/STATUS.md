<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN51 — current state + handoff

**Last updated:** 2026-05-27 (end of design / probe-prototype
session).

This document captures the running state of PLAN50 (bumper
airplanes audience demo) across multiple branches, so a future
session can resume without re-reading the chat history.  For the
design itself read [README.md](README.md); for tunables read
[NUMBERS.md](NUMBERS.md); for the phase-0a probe spec read
[00a-network-probe.md](00a-network-probe.md).

## Branches active in this work

| Branch | State | Purpose |
|---|---|---|
| `bumper_plane` | 13 commits ahead of `origin/libraries`.  Pushed. | All PLAN50 design + tooling.  This branch.  Not yet PR'd; design is still iterating |
| `dryopea-fixes` | 2 commits ahead of `origin/libraries`.  Pushed. **PR-pending.** | Fix for @P368c (div-by-zero on `float / int_literal`) + PROBLEMS.md doc update for @P376.  Ready to open a PR when convenient |
| `store-persist-bind` | **MERGED to main** via PR #222 as `4a7e775`. | The path-backed Store binding (`store_persist_bind`).  Done; dryopea will adopt on their next pass |
| `aid_dryopea` | 1 commit ahead of `origin/libraries`.  Pushed, not merged. | Original branch where `store_persist_bind` was first developed.  Superseded by `store-persist-bind` → PR #222.  Can be deleted |

## What ships in the PLAN50 design (already on `bumper_plane`)

### Documents (in this directory)

| File | Lines | What it captures |
|---|---|---|
| [README.md](README.md) | ~600 | Full design narrative — pitch, world, projector, phone, controls, sound, scoring, wire protocol, sub-arcs, open questions |
| [NUMBERS.md](NUMBERS.md) | ~210 | 51 tunable parameters in 11 sections (physics, scoring, audio, networking, world, QoS, smooth-render, storm, …) |
| [00a-network-probe.md](00a-network-probe.md) | ~190 | Phase 0a probe spec — throughput + LOD-stress test design, pass/fail thresholds, decision table |
| **STATUS.md** | this file | Handoff state — what's done, what's open, what to do next |

### Tooling (`tools/audience-demo-50/`)

| File | Purpose | Status |
|---|---|---|
| `probe_server.loft` (~170 lines) | MVP echo server — INPUT → POSE/EXIT per 30 Hz tick + sight-range filter | Works.  Throughput observed ~12 Hz/peer vs design 30 Hz — known limitation; protocol shape is sound |
| `probe.loft` (~150 lines) | Synthetic-client harness — 4 clients on a grid, verifies sight filter + EXIT exactly-once | Works.  4/4 protocol expectations met |
| `interp_test.loft` (~280 lines) | Standalone interp correctness — 3 trajectories × 3 rates × 2 methods × ±keyframe | Works.  Drove the design decisions on `view.interp_method` default (linear+kf wins for bumper motion) and bounce keyframes (must come from the server, no interp alone recovers a missed bounce) |
| `forecast_test.loft` (~480 lines) | Server bounce-forecast feasibility — Q1 (no-input ballistic accuracy), Q2 (user-input invalidation), Q4 (nose/body/wings contact classification), Q5 (plane-on-plane scoring) | Works.  Q5 four scoring scenarios all PASS (head-on 0/0, T-bone 5/0, tail-chase 5/0, wingtip 5/5) |
| `README.md` | Run + interpret instructions | Up to date |

## Design decisions locked in (with the data that drove them)

| Decision | Data source | Outcome |
|---|---|---|
| **Phone first-person view** (cockpit-only, score-top-only HUD); projector wide overview | UX design choice, locked in conversation | Clean role separation; no chase-camera tuning problem |
| **Sight-range filter (server-side)** | Throughput math; `peer_sight_range = 80 m` | Per-phone outbound bandwidth scales with typical visible-peer count (~5–10), not N − 1 (~29) |
| **Three-tier rate-LOD by distance** (30/15/7.5 Hz at `_full_radius` / `_half_radius` / `peer_sight_range`) | Throughput math; compounds with sight-filter for ~3–4× further reduction | Inner-band twitch fidelity; outer-band saves bandwidth without visible motion artifacts |
| **Explicit `EXIT` signal from server** (not silence-timeout primary) | Latency analysis: ~500 ms perceptible-ghost time with silence-only detection | Fade-out starts immediately at boundary; silence-timeout becomes a 250 ms backstop |
| **Bounces are FORECASTS, not reactive keyframes** | Wall-clock synchronisation across devices + animation-prep time | Server emits `(t_bounce, pos, vel_after)` at `bounce_lookahead_ms` ahead; client animates locally to hit `t_bounce` exactly |
| **`view.interp_method = "linear"`** (was Hermite for a few commits) | `interp_test.loft` data: linear + keyframe = 0 mm error for piecewise-linear motion; Hermite still has 167 mm error on a kf'd bounce | Sharp corners (bounces) are more common than smooth orbits in this game |
| **Sound: phone-only own-plane events; no projector audio, no music** | UX call: "room provides ambience" | Each phone is its own arcade cabinet; physical co-location delivers spatial audio for free |
| **No "ENTER" signal** | First-pose-for-unseen-plane-id is itself a trigger | Smaller wire surface |
| **Per-phone adaptive QoS (post-v1)** | Real audience venues span 10× RTT range | Phase 7 sub-arc; passive measurement, no client cooperation; classifies into Good/OK/Limited tiers |
| **Plane = nose + body + wings AABBs** (red nose, player-coloured body) | Anti-coordination needs visual rule "don't ram red" | Scoring rule reduces to "what did my contact touch on the other plane?"; clean, no aggressor identification needed |
| **Bounce-forecast feasibility verified** | `forecast_test.loft` Q1 / Q2 / Q4 / Q5 data | Ballistic prediction is exact within 5 ms at no-input; gentle inputs drift <1 m; hard direction reversals invalidate; contact classification works |

## Open items at handoff

### Immediate (can be done in one short session)

1. **Open PR for `dryopea-fixes`** to land the @P368c fix (and PROBLEMS.md doc update).  Branch is push-clean, tests pass locally.  URL: https://github.com/loft-lang/loft/pull/new/dryopea-fixes
2. **Delete `aid_dryopea` branch** — superseded by `store-persist-bind` PR #222 which merged.

### Open loft-side bug — @P376 (dryopea-surfaced)

**`vector<Struct>` with trailing `u8 not null` fields, wrapped in a parent struct, serialised via `:j` → all fields zero out.**

Standalone vector `:j` works; only the wrapped path corrupts.  Specific trigger: `hash<Pair[k]>` → `for`-iterate-into-vector → embed in parent struct → `:j`.

- Documented in PROBLEMS.md (dryopea-fixes branch).
- Fix path identified: `src/database/format.rs::write_list` nested-walk reconciliation.
- **Estimated effort:** half-day deep dive into `ShowDb`'s sub-call construction across `write_fields` (struct field) and `write_list` (vector walk).
- Workaround already shipped in dryopea (widen u8→integer when building the save vector — mirrors the `PaintedHex`/`GroundEntry` pattern).

### PLAN50 design — likely next refinements (deferred, not blocking)

Listed roughly in declining importance:

| # | Item | Source |
|---|---|---|
| a | Stall-recovery dynamics test — verify the random-tumble bounce + dampened control window actually decays to stable | NUMBERS.md `stall.*` parameters; not yet tested |
| b | Target-bumper cooldown state machine — primed → spent → reprimed at 15s; verify no farming | Section "Target bumpers" in README; not yet tested |
| c | The "FORECAST then animate locally" client timeline test — clock skew handling, correction window blend | Section "Bounces as forecast" in README; not yet tested |
| d | Multi-plane N² forecast cost — at N=30, that's 900 pair-tests per tick; measure if it fits a 33 ms tick budget | Future scaling concern; not yet tested |

### PLAN50 implementation — phase-0 prerequisites

The probe MVP works; the v2 probe (full rate-LOD bands + projector simulator + EVENT messages + latency histograms + per-tier ramp) is described in 00a-network-probe.md but not yet implemented.  Phase 0a v2 should land before phase 0 (real phone client) commits substantial code.

After phase 0a v2:

- **Phase 0** — phone client HTML/JS/WebGL/WebAudio
- **Phase 1** — loft server with per-plane pose state, 30 Hz broadcast, event dispatch
- **Phase 2** — static world loader: reads dryopea MapFile + MarkerFile, extrudes per palette
- **Phase 3** — projector renderer
- **Phase 4** — physics module
- **Phase 5** — scoring + ambience
- **Phase 6** — playtest
- **Phase 7** — adaptive QoS (post-v1)

## Dryopea integration option (recommended starting point for phase 2)

Dryopea's editor produces exactly the static-world data PLAN50
consumes (recent dryopea commit `1b10e72` explicitly added target
markers as "@PLN51 substrate").  Three approaches to integrate
(rationale in the design-doc discussion):

- **A. Copy loader modules** into `tools/audience-demo-50/` (~150 lines from dryopea's `world.loft` / `painted.loft` / `palette.loft` / `markers.loft` / `map_file.loft` / `marker_file.loft`).  Quick.  Drifts over time.
- **B. Path-dep on `../../dryopea`** in `loft.toml`.  Auto-tracks.  Requires the dryopea checkout in a known location.
- **C. Extract `lib/hex_world/`** as a shared loft library, consumed by both dryopea and the audience demo.  Architecturally cleanest; substantial work (matches `lib_plan 24` "universal hex-world editor" scope).

**Recommended for v1 prototype:** option A (copy + a deliberate forking note).  Option C is the right long-term move; pursue when `lib_plan 24` activates.

## Pointers for resuming

- **Where the work is:** `tools/audience-demo-50/` (probe + tests), `doc/claude/plans/51-bumper-airplanes/` (design)
- **Where dryopea lives:** `~/workspace/dryopea/` (sibling repo, on `main` branch)
- **Where the open dryopea questions are:** `~/workspace/dryopea/QUESTIONS_FOR_LOFT.md`
- **Where loft's bug index is:** `doc/claude/PROBLEMS.md` (fast-index at top, full Quick Reference below)
- **PR status:** PR #222 (store_persist_bind) — merged.  No other loft PR open.  `dryopea-fixes` branch ready to open.
- **`bumper_plane` push state:** in sync with `origin/bumper_plane`.

## When you resume — suggested first moves

1. **Quick win, ~10 min:** open the PR for `dryopea-fixes` so @P368c lands.  URL above.
2. **Decide direction:** PLAN50 design is mature enough to stop refining and start phase 0a v2 (full benchmark probe).  OR investigate @P376 (half-day deep dive).  OR pivot to a different plan entirely.
3. **If continuing PLAN50:** the natural next test slice is **stall recovery dynamics** (item a above) — extends `forecast_test.loft` with the random-impulse-at-bounce-then-decay scenario.  Validates the `stall.*` parameters before phase 4 (physics module) commits to them.

That's the state.  Branch is push-clean; no uncommitted work.
