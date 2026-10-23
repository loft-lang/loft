<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 196 — Store upgrade: a changed definition converts its data, or says why it cannot

Tracker: [@PLN196](https://github.com/loft-lang/plans/issues/196).

**Status — DONE 2026-10-23.**  The reference for what shipped is
[DATABASE.md § A changed definition: the store upgrade](../DATABASE.md#a-changed-definition-the-store-upgrade)
(the map, the rules per change, the callers, skipped releases, the baseline, the running world);
the rules are `(L-Sound)`, `(L-Release)`, `(L-Upgrade-Routine)`, `(L-Upgrade-Old)` and
`(L-Upgrade-Source)` in [formal/layout.md](../formal/layout.md), `(F-Default-Field)` in
formal/calls.md; the warning is `layout-upgrade-refused` in DIAGNOSTICS.md.  This file is a
closure record only.

It continued the deferred E2 of [@PLN97](97-layout-contract/README.md), which shipped the layout
identity, the `.dschema` sidecar and the refusal on load.  Before this plan every reader refused
a store whose layout changed, even by one added field, and nothing converted it.

## What shipped, per step

| # | step | outcome | guard |
|---|---|---|---|
| 0 | One name-keyed description from the live table, a sidecar and the source | `LayoutDesc::from_dump` reads every corpus program's dump back into its live description (1763 programs, 1353 with types); the facts a dump lacks were counted — nullability in 388, a reference's target in 34, an index group in 50, a hash element's linkedness in 17 — and became the version-2 sidecar's facts section | `every_corpus_layout_reads_back_from_its_dump` |
| 1, 3, 7 | The converter and `loft layout upgrade` | `Stores::upgrade_store` reads through the old layout, writes through `record_new`/`record_finish`; the file is written beside, swapped in, the old kept as `.old` | `tests/store_upgrade.rs` cells |
| 2 | The map and the check | `UpgradeMap` from two descriptions only; `loft layout check` prints it and exits 1 on a refusal | the verdict table in `tests/store_upgrade.rs` |
| 4 | Keyed collections, the linked flip, duplicates | rebuilt by insertion; a displacing insert refuses the whole upgrade | keyed cells |
| 5 | Expression defaults; the two-step rename | a default may read `$`; the upgrade runs it per record through `__fill_<T>__<f>` | `tests/store_upgrade.rs`, `loft-suite rules/calls/F-Default-Field` |
| 6 | References | in-store re-pointed; cross-store refused | reference cells |
| 8 | Callers | `store_load*` convert in memory, `store_persist_bind` upgrades the file, `swap_world` carries the world (native S5, browser S6) | `store_persist_loft.rs` `*_converts_or_refuses_a_changed_layout_both_backends`; `engine_host_kernel.rs` `s5_native_swap_under_running_world`; `engine_host_connector.rs` `s6_browser_swap_under_living_page` |
| 9 | Skipped releases | the sidecar records the writing release; `upgrade(from, old, item)` routines; `data_compatible_with` / `data_bridge`; the `layout.lock` baseline and the `layout-upgrade-refused` warning | the `cell_a_*` and `layout_check_*` cells |
| — | Acceptance: an IDE linked to a running game | `loft --serve`: Save compiles against the running world and rebuilds in the background; ⇄ Apply activates | `tests/serve.rs` `serve_apply_holds_saved_edits_and_carries_the_world` |

Not converted: the working-set loaders (`store_load_key*`) still refuse a changed layout
(STRONG_POINTS.md § 2 names it as the next step).

## Defects the probes found, fixed in the arc

- The descriptor rendered a `trie` differently from the dump.
- An enum line named a variant rather than its type, so two programs with same-named variants
  shared one identity and `store_load` misread (loft#2096).
- A method on an enum was stored as one of its variants: `"label" as E` parsed, and adding a
  method refused every stored `E` (loft#2100).
- A stored `reference<T>` kept the slot number of the program that wrote it, so a load into
  another slot read another store (loft#2099).
- `loft layout check` refused nothing and printed "auto-defaulted on load" for a change every
  loader refused.
- The S6 browser test passed any page that threw nothing: without `--expect-log` its harness
  slept the whole window.

## Owner rulings

- **Rename syntax (2026-10-20):** none new; an added field's default, an expression over `$`,
  fills it.  A rename is two upgrades.
- **Upgrade on load (2026-10-21):** `store_load` converts in memory and leaves the file;
  `store_persist_bind` upgrades the file and keeps one `<path>.old`.
- **Long-lived structures (2026-10-21):** saved stores, the swap world, every global store.
- **Skipped releases (2026-10-21):** the version is the identity; one routine per type; the
  floor is the programmer's.  Two readings that differ from the design's wording were accepted
  (no preference, provided in-process replacements are right): the routine runs for every store
  older than the program, and a field of `old` the stored record lacks takes its default.
- **The swap's verdict (2026-10-21):** reached when the next build compiles against the running
  world, so a refused change fails the build and no swap is offered.
- **The baseline (2026-10-21, 2026-10-22):** written at release by `loft package`, for a
  library and an application alike; stored in `layout.lock` beside `loft.toml` (the old
  `.loft/layout.lock` sat in an ignored directory and is still read).
- **A refused upgrade is a warning (2026-10-22),** not an error: legality stays a matter of the
  source, and a refactor that drops a field before writing the routine still runs its tests.
- **Which types count (2026-10-22):** what a store or `swap_world` call hands over, every `pub`
  type, and what they hold; a reload happens only at the host's main loop, so frames never count.
- **Activation (2026-10-22):** in the IDE a new build becomes active only by an explicit button
  or keystroke — code is reviewed before it runs, since a warning does not stop a program.
  Function edits wait for the same Apply.

## See also

- [@PLN97](97-layout-contract/README.md) — the layout identity this plan extends.
- [18-engine-host/08-live-build-swap.md](18-engine-host/08-live-build-swap.md) — the background
  rebuild and the swaps (S4–S6) the world crosses through.
- [16-debugger/IDE.md](16-debugger/IDE.md) — the server-backed IDE's Save and Apply.
- [@PLN43](https://github.com/loft-lang/plans/issues/43) durable stores (parked): a write-ahead
  log would need the same upgrade for its records.
