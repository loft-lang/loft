<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 156 — Release evidence: every gate proves it ran on this commit

Tracker: [@PLN156](https://github.com/loft-lang/plans/issues/156).

## Status

**DONE — SHIPPED 2026-09-07.**  All five phases built, measured against the live 2026.9.0
release, and falsified (each instrument's red case ran, not only its green).  The REFERENCE
content lives in [RELEASE.md](../../RELEASE.md) — the step-4 gates, the cadence axis
(`--phase mid|pre`), and the liveness census are documented there beside the checklist they
extend; this README stays as the closure record (the sub-arc table below carries the
verification receipts).  The standing carriers of the plan's obligations: the `M-liveness`
checklist item re-reads the census every release, and the null-hypothesis re-measure of phase 5
(did the "never been true" class stop producing?) is `make bug-review`'s ordinary cadence —
no open work remains here.  Findings surfaced to the owner at close: today's `ci.yml` /
`miri.yml` scheduled runs red on main; `release-gate.yml`'s last run (09-04) red.

## Goal

Every release gate produces evidence of the form *"this check RAN on this commit and answered
X"* — never *"nothing was red"* — and the three properties assumed true and found false in
2026.8.0 (the acquisition chain, the registry validator's acceptance, step 4's same-cycle
effect) become automated per-release gates, each shown able to go red.

## Effort + design

- **Effort:** M — five phases, none straddling a PR; phase 2 is the largest single item.
- **Design:** ~ — the mechanisms are demonstrated by hand; the design calls are where each
  gate runs (tag pipeline vs checklist vs monthly report) and the evidence record's shape.
- **Value category:** S (silent failure).  The failing artefact is a procedure that reports
  done over things that never happened.
- **Last touched:** 2026-09-07.

## Composition matrix — Stage A

No new language composition surface (process/CI plan) — the matrix here is the **gate × verdict**
grid instead: for every gate this plan touches, the four verdicts { ran-and-passed ·
ran-and-failed · skipped/env-absent · never-ran } must each be REACHABLE and DISTINGUISHED in
the evidence record.  A gate for which the middle two collapse into green is the defect class
this plan exists for; each phase's Verify names how its gate is shown able to produce the
ran-and-failed cell.

## Sub-arcs

`Verify` names the comparison that would go RED if the phase were done wrong.

| Item | Source | Verify | Status |
|---|---|---|---|
| **1** — same-cycle effect proof: `self-update --dry-run --refresh` resolves the NEW version; the published bundle's `verify-self` anchors to the signed index — automatic, untickable checklist items | RELEASE.md step 4 postscript (today: advice) | the trap the postscript names — a cached index predating the splice answers `no releases published to compare against`, and the item must read that as FAIL, not quiet; plus `--refresh` absent must be detectable | Shipped — `A-registry-this` + `A-selfupdate-resolves`; both GREEN on live 2026.9.0; the unspliced-version red case measured FAIL |
| **2** — the acquisition chain end-to-end, automated per release: clean env → signed index → resolve → download → install → `verify-self` → run a program | the 2026-08-31 throwaway-clone run, made a post-publish job | a doctored index — wrong hash, missing triple, stale `manifest_sha256` — each goes red; the job's own green names the sha and the four triples it ran | Shipped — `scripts/acquisition-chain.sh` + `A-acquisition` + `post-publish-verify.yml`; full chain GREEN on 2026.9.0 (anchor line asserted); exit-3 red case measured |
| **3** — cross-repo validator dry-run pre-tag: registry's validator against the generated `loft-<v>-registry-entry.json` spliced into a copy of the live index | `scripts/gen-toolchain-entry.py --splice-into`; registry#22's gate set | the current validator against pre-#22 conditions reproduces the `loft package` rejection (demonstrated by hand 2026-08-31 — the phase automates it and pins the validator ref) | Shipped — `scripts/validator-dryrun.py` (full/structural/replay); replay of 2026.9.0 passed ALL registry gates; `--corrupt` went red at gate 2b; structural exits 3, never green |
| **4** — adversarial suite for untrusted install inputs: archive entry paths (`..`, absolute, symlink), manifests, sidecars, registry-index JSON | `src/self_update.rs` — the guard and its `../../etc/passwd` probe, generalised | each probe carries `@falsified-at` against a build with its guard removed (`make falsify`), per TESTING.md | Shipped — zip-slip cells + absolute-manifest cell; found+fixed the drifted second home (`verify_self::manifest_path_escapes`) |
| **5** — the liveness census, standing: dead tags, suppressions with stale justifications, skip/env-absent-green gates, steps never run — scheduled, in the monthly-review family | the 2026.8.0 rescue census, plus `release-checklist.py`'s existing UNKNOWN/STALE vocabulary | the per-release RULE it also lands: an UNKNOWN/skipped item never aggregates into a green verdict — checked by feeding a checklist state with one UNKNOWN blocker and requiring a non-green summary | Shipped — `make release-liveness` + `M-liveness` item + the UNKNOWN rule (exit 3, "not green" line, ran-on-sha stamp); the re-measure rides `make bug-review`'s ordinary cadence |

## Phase ordering

1 and 3 first — cheap, and they close the exact step-4-class hole in the same cycle.  2 next
(the single highest-value phase: the property assumed for every prior release that turned out
false) — it records into 1's items.  4 is independent and can interleave.  5 last: it
generalises what 1–4 instantiate, and its census vocabulary should be read off their evidence
records rather than invented first.

## Open design questions

1. **Where does phase 2 run?**  A post-publish workflow in this repo needs the published
   release and the signed index, which land in two repos on two clocks; the alternative is a
   checklist automatic item that shells the whole chain locally.  The job form is standing
   evidence, the item form is same-machine — the answer may be both, with the item as the
   pre-publish rehearsal against the draft's assets.
2. **Which validator ref does phase 3 pin?**  `origin/main` of the registry tracks the truth
   but makes the gate fail on THEIR regressions; the last-released validator misses exactly the
   next incompatibility.  Proposal: run both, gate on the released one, report the delta.
3. **What is the evidence record?**  `releases/<cycle>/checklist.json` already carries
   timestamps and notes; the ran-on-sha discipline wants each automatic verdict stamped with
   the commit it measured (the release-gate precedent — `release-checklist` reads it by HEAD's
   sha).  Whether manual (`M-*`) items also demand a sha is open; a human note without one is
   today's behaviour.
4. **Does phase 5 gate anything?**  No — the monthly-review family is REPORT, never gate
   (BUG_REVIEW.md's rule), and only per-release blockers gate.  The one rule it lands
   (UNKNOWN never aggregates green) lives in the checklist's summary, which already gates.

## Cross-arc dependencies

- **RELEASE.md / `make release-checklist`** — phases 1, 3 and 5 land as checklist items; the
  inventory design is not reopened.
- **`loft-lang/registry`** — phase 3 consumes its validator; any gate-set change there moves
  this plan's pin (question 2).
- **@PLN155 arc A** — none shared, but the same discipline: an instrument must be shown able
  to go red before its green is believed.

## See also

- [RELEASE.md](../../RELEASE.md) — step 4's measured block (the never-completed splice, the
  validator rejection, the end-to-end throwaway-clone verification this plan automates), the
  checklist section and its two corrections (`install.sh`, smoke-from-the-ZIP).
- [releases/2026-08/](../../releases/2026-08/) and [releases/2026-09/](../../releases/2026-09/)
  — the cycle records; `releases/<cycle>/checklist.json` is the evidence file phase 5 reads.
- `scripts/release-checklist.py` (UNKNOWN/STALE, the ran-vs-skipped smoke split),
  `scripts/check-release-published.py` (the next-release backstop this plan complements),
  `scripts/gen-toolchain-entry.py`, `tests/self_update_swap.rs`, `make falsify`.
- [CI_BUDGET.md](../../CI_BUDGET.md) — where the report/gate line stays.
- [@PLN156](https://github.com/loft-lang/plans/issues/156) — this plan's issue.
