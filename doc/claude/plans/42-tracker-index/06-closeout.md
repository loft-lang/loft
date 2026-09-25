<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 06 — Retroactive tagging

**Status:** Migration shipped 2026-05-14; the plan closed 2026-09-25.  The threshold test
below was retired rather than built: the bare `P<n>` series is the closed archive, so no new
bare ids are minted — see [README.md § Status](README.md).

## What shipped

- `tools/indexer/migrate.py` — Python rewriter (sed proved
  too risky; awk too clumsy for backtick-span detection)
- 1500+ refs migrated across ~150 `.md` files
- `make index` legacy bucket dropped from ~2643 → ~1900
  refs (the residue is single-digit `P1`-`P9` skipped on
  purpose, refs to closed P-issues no longer in
  PROBLEMS.md, and code-file refs which don't migrate)
- `tests/index_hygiene.rs::no_broken_tracker_tags` still
  green; no broken tags introduced

The script's safeguards (kept conservative on purpose):

- Skip `P\d+-R\d+` (phase-N risk-M notation in COROUTINE.md
  / SAFE.md / CHANGELOG_TECHNICAL.md)
- Skip `P[0-9]` followed by single-digit only (P1-P9 are
  heavily overloaded with PERFORMANCE.md design IDs and
  plan-N phase-M shorthand)
- Validate the numeric part against PROBLEMS.md row IDs
  before rewriting (closed-and-removed P-issues don't get
  `@`-prefixed)
- Skip lines inside fenced code blocks
- Skip lines containing `<!--noindex-->`
- Skip occurrences inside same-line backtick spans (so
  `\`P259\`` examples explaining the convention survive)
- Skip refs preceded by `/` (URL paths like `/tag/P259`
  shouldn't break)

## Goal

One-shot migration: convert most existing bare-name `P\d+`
references to `@P\d+` form so the indexer's `legacy:`
buckets shrink toward zero.  Then close the plan.

## What ships

### Retroactive sed pass

A scripted migration of the obvious mass-rewrites.  Three
classes:

1. **Trivially safe** — refs in PROBLEMS.md row IDs and the
   row's own narrative.  The leading `| 259 |` table cell
   stays bare (it's the row ID, not a reference); body text
   gets `P259` → `@P259` rewritten.
2. **Plan READMEs + phase docs** — references in prose.
   `\bP\d+\b` → `@P\d+`, `\bplan-\d+\b` → `@PLAN\d+`.
3. **Code comments + commit messages going forward** —
   adopted by convention; no sed pass for code (would
   produce too much churn vs benefit).

A migration script `tools/indexer/migrate.sh` does the
rewrite + diff:

```bash
# Only `.md` under doc/claude/, not source code.
git ls-files 'doc/claude/**/*.md' | while read -r f; do
  # @P-id rewrite — match P\d+ NOT preceded by @ or alphanum.
  sed -E -i \
    -e 's/(^|[^@a-zA-Z0-9])(P[0-9]+[a-z]?)(\b)/\1@\2\3/g' \
    "$f"
  # @PLAN-id rewrite — match plan-\d+ NOT preceded by @
  sed -E -i \
    -e 's/(^|[^@a-zA-Z0-9])plan-([0-9]+)(\b)/\1@PLAN\2/g' \
    "$f"
done
```

The script is **idempotent** (the `[^@a-zA-Z0-9]` lookbehind
prevents double-prefixing).

Run + review the diff before committing — not all
occurrences should migrate (e.g., `P-issue` shouldn't
become `@P-issue`).  Phase 06 reviews the diff and accepts
or rejects per file.

### Hygiene test enforcement

After the migration, `tests/index_hygiene.rs` (phase 03)
gains a SECOND test:

```rust
#[test]
fn legacy_tag_count_under_threshold() {
    // After phase 06 migration, expect < 50 legacy refs.
    // Acts as a regression guard against accidental
    // bare-name re-introductions in new docs.
    let json = std::fs::read_to_string("index/tags.json").unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let legacy_count: usize = v.as_object().unwrap()
        .iter()
        .filter(|(k, _)| k.starts_with("legacy:"))
        .map(|(_, v)| v.as_array().map(|a| a.len()).unwrap_or(0))
        .sum();
    assert!(legacy_count < 50,
        "legacy tag refs ({legacy_count}) exceeded threshold (50). \
         New docs should use @P-id / @PLAN-id forms.  Run: \
         tools/indexer/migrate.sh");
}
```

Threshold (50) is tunable; the goal is to catch
accidental regressions, not enforce zero.

### Doc closeout

- `CHANGELOG_TECHNICAL.md` — @PLN42 retrospective entry
  (per-phase summary + bug yield + adoption stats:
  "before/after legacy ref count").
- `ROADMAP.md` — remove @PLN42 row from active section.
- Close the issue: `gh issue edit <N> --repo loft-lang/plans
  --remove-label status:active --add-label status:finished`,
  then `gh issue close <N>`.  The plan dir stays in place; its
  README becomes the closure record (see `_LIFECYCLE.md`).

### Optional: rewrite scanner in loft

If the bash + grep pipeline has felt fragile, phase 06 can
optionally rewrite `tools/indexer/scan.sh` as a loft
program: `tools/indexer/scan.loft` compiled to a binary
similar to `loft-view`.  Drives loft's text-handling +
file-walking + JSON-emitting capabilities.

This is **optional** — bash version is shipping and works.
Promote to "required" only if maintenance burden surfaces.

## Acceptance

- `tools/indexer/migrate.sh` reviewed + landed on a focused
  branch.
- `make index` after migration shows ≥ 80% reduction in
  `legacy:` reference count.
- `tests/index_hygiene.rs::legacy_tag_count_under_threshold`
  passes.
- ROADMAP.md, CHANGELOG_TECHNICAL.md, plan README updated.
- Plan dir stays at `plans/42-tracker-index/` (top-level, as its own closure record — the
  current convention; the legacy `finished/37-…` move is retired).
- `bash scripts/check_doc_drift.sh` (or `cargo test
  --test doc_hygiene`) clean.

## Risks

| Risk | Mitigation |
|---|---|
| Sed pass false-positives (e.g., `P` in chemistry context) | Manual diff review per file; accept individual non-migrations |
| Some legacy refs are intentional (test fixtures, prose examples) | Add `# noindex` opt-out comment recognised by phase 00's scanner; document in this phase |
| Migration churn dominates a release | Land in chunks (PROBLEMS.md alone first; plan READMEs after) |
| Threshold of 50 too aggressive | Tune the constant; the test exists to catch accidental large regressions, not micromanage every reference |

## Cross-references

- [Phase 00 — scanner](00-convention-and-scanner.md) — defines the legacy form
- [Phase 03 — broken validator](03-broken-validator.md) — tests gain the legacy-count check here
- [README § Acceptance](README.md#acceptance--full-plan)
