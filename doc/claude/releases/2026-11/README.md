<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `2026-11` — release state (opened mid-cycle 2026-10-16)

> The record of ONE release cycle — its blockers, the evidence each gate produced, and the
> decisions taken.  The process every cycle follows lives in
> [RELEASE.md](../../RELEASE.md); the index of cycles in [releases/README.md](../README.md).

Opened at the cycle's **halfway point**: `make release-checklist ARGS="--version 2026.11.0
--phase mid"` is the view, read on `tuxedo-release-2026-11-mid` (forked from the join tip
`1a4ff58d4`, which carries `origin/main` at `46ec215e0`).  `2026.10.0` shipped on
2026-10-01; this cycle ships at the start of November, gated on stability.

## The halfway view — what each `[mid]` row reads

| row | state | evidence |
|---|---|---|
| `A-audit` | ✅ | four rustls advisories (RUSTSEC-2026-0346..0349) against 0.23.45; `cargo update -p rustls` → 0.23.46 (`f14aef881`).  Warnings left: `memmap` 0.7 and `ttf-parser` unmaintained, `zip` 2.4.2 yanked (the whole 2.x line is; 9.x is the live major — an API move, not a bump) |
| `A-docs-lint` | ✅ | 111 over the baseline, all in the files the join split out of main.rs and friends (the count-keyed baseline cannot see a move).  The 51 stamps and the real narration rewritten; the narration pattern no longer fires on the word `removed` alone, which `@FR-H-Removed` makes domain vocabulary; baseline re-pinned at 4069 (`a9d4063a1`) |
| `A-ci` | ⏳ | the local gate on `0a923a169`; main's own CI at `46ec215e0` was red on the `optional` audit row (276 vs 275 — this branch's join already carries 275) and on two Windows tests handed `--native-emit /dev/null` (cured, `bb4da4cbe`) |
| `A-reference-review` | ✅ | 11 chapters re-read against the interpreter (probes, not prose): 17-libraries carried the pre-#1855 model in five sentences, 25-generics declared `<`/`==` where the interface declares `compare`, 24-json promised one `#errors` entry per error — fixed in `bb4da4cbe`; 40/40 at their source |
| `A-skills-review` | ✅ | 12 skills read on the three axes; the untrue claims (engineering-rigor's rebase licence, loft-write's `fn Op…` and divide-by-zero Warn, loft-test's unsharded leg, loft-plan-workflow's closed canon) fixed in `bb4da4cbe`; 13/13 at their sources |
| `M-doc-validation-mid` | ⏳ | part 1: 51 libraries moved since their watermark, every one with zero `pub fn` lines changed — dismissed as the method allows, watermarks bumped; part 3: 20 `@F` entries re-read, 11 corrected on the tracker (`features#2 #16 #11 #47 #124 #120 #121 #6 #5 #100 #126`), the catalogue shadow regenerates after the gate; part 2 (gendoc, `.doc_review_ledger`) after the gate |
| `M-monthly-bugs` | read | narrow-int/width is the one RISING class; `Parser::nstore_unwrap_report` reads NO EFFECT on null/sentinel (10.8 % → 18.6 %) — a premise to re-open |
| `M-liveness` | read | one stale rationale (re-justified); chronic reds named below |
| `M-ignores` | ✅ | every reason's job exists and the sweeps job was green on the 2026-10-16 nightly |
| `M-close-plans` | ✅ | plans#178 and #183 already closed |
| `M-file-sizes` | read | `src/parser/mod.rs` (27 205) and `src/parser/control.rs` (23 123) are the picks |
| `M-falsify-receipts` | read | 117 receipts the census cannot classify (108 `hand-measured`) |

## Red on main's tip when the pass opened, and what this branch does about each

- **The optional audit row** (all three CI legs): 276 in QUALITY.md, 275 measured — the join re-measured it.
- **Two Windows tests** (`ctor_literal`, `fresh_text_hold`): `--native-emit /dev/null` — `NULL_SINK`.
- **Nightly native ASan**: `tests/scripts/10-match.loft` absent at `46ec215e0` — the join's `1c7e3c440` put the five programs back.
- **Nightly doc index hygiene**: 28 decisions "unguarded" because the job never checked out the language suite — `checkout-suite` added.
- **Nightly LOFT_POISON**: `1994-…` w2 reads freed memory — loft#2022, claimed, the fix is this pass's code work.
- **Nightly debug-assertions**: `issue_328` leaks a store — loft#2023, claimed.
- **Dependency advisories**: rustls, cured.
- **revalidate-libs** (chronic, 9 of 14): E0514 from rlibs a toolchain bump left in the cached `deps/` — the toolchain is in the cache key now.
- **browser-threads** (chronic, 7 of 14): `par-scaling-bench.sh` 147 % < 150 % and `par-ui-responsive.sh` — timing bars on a shared runner; open.
- **consumer-main-health** (chronic): dryopea ×14, moros/crawler ×8 — the consumers' own agents, per their `LOFT_HANDOFF.md`.

## Found by the reviews and filed

loft#2024 `**` on integers answers a `single` (silent-wrong past 2^24) · loft#2025
`variant-field-unchecked` over-fires on a `name:` binding · loft#2026 `lost-write` misdiagnoses
a write through null · loft#2027 C80 claims a run-time Warn no site emits · loft#2028 the two
JSON parse forms disagree about `#errors`.

## What the cycle still owes before the tag

`M-ops-census`, `M-perf-pass`, `M-docs-review`, `M-doc-review` (one doc under
DOC_CONTRACT, its own PR), `M-file-split` (the two picks), `M-libs`, the `[cand]` sweeps on
the candidate, and loft#2022 / loft#2023 fixed.
