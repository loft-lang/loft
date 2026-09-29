<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->

<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# GOALS-history.md — dated readings and progress records

The timeline behind [GOALS.md](GOALS.md): dated readings of the two floors and progress notes.  The working doc states the bar and the Check; this file keeps what they read on the day.

## Two floors — the 2026-08-09 readings

### Soundness floor

  **Reading (2026-08-09): the gate is RED, and coverage is still narrower than the Checks
  state.** @PLN85 closed and every *tracked* store-lifetime bug is fixed, but three things
  keep this from reading MET.
  (i) **The nightly is failing now.** `miri.yml` on `main` was green 2026-08-05 through
  2026-08-08 and went red on 2026-08-09: the ASan interpreter leak gate fails on *both*
  ubuntu and macOS, and the debug-assertions sweep fails with it. This is the floor's own
  Check, so until it is green again the floor cannot clear. A fresh regression, not a
  standing state — the failing run swept `8350a2ab`; the current tip is not yet swept.
  (ii) **Coverage is narrower than the Checks above state**, though less so than it was.
  The `stack_align_guard` sweep runs 16 in-process test binaries — but that is 16 of 179,
  and `tests/data_structures.rs` runs wholly in-process yet is not among them, so a sweep
  described as covering "the corpus" does not. `LOFT_STORE_GUARD=1` (Goal E's Check) *is*
  wired now: `miri.yml`'s debug-assertions sweep sets it, and its enforced twin — the
  `reclaim_guard` `assert_eq!` in `scopes.rs` — hard-gates there through
  `cfg(debug_assertions)`. But it reaches 5 targets (`--lib --test issues --test wrap
  --test strings --test frame_vars`), and `[profile.dev.package.loft] debug-assertions =
  false` compiles the assert out of ordinary test builds, so most of the corpus never arms
  it. (iii) **The class is not yet retired by construction**: the fuzz/sanitizer corpora
  that must prove it are now standing (@PLN53 + @PLN54 both closed 2026-07-10), so the
  **one** remaining step is the Cluster C / H10 fold — land it and the silence becomes
  proof. Tracked in [STABILITY_ROADMAP.md](STABILITY_ROADMAP.md).

### Structure floor

  **Reading (2026-08-09): MET, and the caveat that used to qualify it is closed.**
  The [`loft-lang/registry`](https://github.com/loft-lang/registry) carries 34 signed
  (`index.json` + Ed25519 `.sig`), per-version-`sha256` packages with dependency resolution
  (`server` → `web >=0.1`; `hex_terrain` → `hex_grid`) — `graphics`, `game_protocol`, `server`,
  the hex-world stack, `web`, `crypto` (the zero-trust lib), assets, docs — each on its own
  semver track. `loft install <name>` resolves and fetches end to end, including transitive
  deps. Extraction + installability + version-stability read true.
  **`installable` now means a working artifact, not just resolve + fetch.** The nightly
  `registry-validation` gate passes every one of the 34 package legs, and has been green on
  `main` on most nights since 2026-07-31 (08-07, 08-08 and 08-09 consecutively). Both faults
  that used to hold it red are fixed: the workflow installs `libasound2-dev`, so `graphics`
  builds `--native` on a clean runner, and `hex_terrain` validates green against current
  loft. That closes the ecosystem rot this caveat recorded — including the C86 H-Copy
  compatibility break, which is the exact failure the wide-release bar's **gate 5** exists
  to prevent.

### The pause

The structure floor's bar reads MET without a caveat, and the soundness floor is close
on substance but currently **red on its own gate**, so **the pause is at its end, not its
middle** — held open by a regression to clear, not by a body of work still to do.

## Goal E — reference counting removed

**Progress — reference counting removed** (plan-57,
[`@PLN2`](https://github.com/loft-lang/plans/issues/2), closed 2026-06). The store
reference count is gone (`ref_count` / `inc_rc` / `dec_rc` / `OpIncRc` deleted;
`Store.pinned` for const/global). It is replaced by a single-ownership free at scope
end plus a closure-record cascade. This advances Goal E directly: **no hidden
counter decides when a value dies — the scope does**, which is the plain reading of
the source. The reference count was the clearest case of sound-looking machinery
that hid the real lifetime; removing it makes the lifetime visible.


