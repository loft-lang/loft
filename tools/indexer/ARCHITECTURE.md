<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# tracker-index architecture

Phase 00 of [`plans/37-tracker-index/`](../../doc/claude/plans/42-tracker-index/README.md).

## Why bash + grep + jq

Smallest possible dependency footprint: bash + grep are POSIX,
jq is pre-installed in most VMs.  No build step, no runtime,
no custom parser.  Trade-off accepted: can't distinguish a
tag in a code literal from a tag in prose — every reference
counts equally.

## Tag families

| Family | Shape | Purpose |
|---|---|---|
| `@P\d+[a-z]?` | P-issue references | "where is P259 mentioned?" |
| `@PLAN\d+(-[\w.]+)*` | Plan + phase + sub-phase (LEGACY spelling) | "where is PLAN22-2d-iii.a referenced?" |
| `@PLN\d+` | A `loft-lang/plans` issue — the CANONICAL plan tag | "where is PLN157 referenced?" |
| `@GH\d+` | A `loft-lang/loft` issue or PR | "where is GH247 referenced?" |
| `@FR-<Rule>` | A formal-rule citation | "which sites enforce this rule?" |
| `@F\d+`, `@I\d+` | `loft-lang/features` feature / infrastructure | "where is F7 referenced?" |
| `@AAA-\d\d\d` | Worked-example tag | "which fn demonstrates STD-011?" |
| `legacy:P\d+`, `legacy:plan-NN` | Bare-name (no `@`) forms | Track adoption progress |

Only `@P` and `@PLAN` are VALIDATED against something local (PROBLEMS.md row
ids and plan directories); the rest name ids in other trackers, so they are
indexed and URL-resolved without a network call and never appear in `broken`.

⚠ **Keep this table complete.** It listed two families for long enough that
`@PLN` — by then the canonical plan tag — was neither in it nor in the scanner,
so `idx tag:@PLN157` answered `[]` at exit 0 for a tag with hundreds of
references (loft#1544).  An unindexed family is indistinguishable from
"referenced nowhere".

The `legacy:` prefix lets us measure the gap between
"adopted-the-convention" references and "still need
migrating" references.

## Output shape — `index/tags.json`

```json
{
  "@P259":            [ {file, line, context}, ... ],
  "@PLAN22":          [ ... ],
  "@PLAN22-2d-iii.a": [ ... ],
  "legacy:P259":      [ ... ],
  "legacy:plan-22":   [ ... ]
}
```

Within each array, entries are `(file, line)` sorted and
deduplicated.  `tags.json` is byte-identical across runs on
the same source tree.

## Phasing

| Phase | What ships |
|---|---|
| 00 (this file) | Scanner + Makefile + tag convention in CLAUDE.md |
| 01 | `scripts/idx` CLI query wrapper |
| 02 | git pre-commit hook auto-refresh |
| 03 | Broken-tag validator + CI hygiene test |
| 04 | Plan-35 viewer integration |
| 05 | Claude integration (CLAUDE.md instructions, optional MCP) |
| 06 | Retroactive tagging sweep + closeout |

## Performance target

≤ 2 seconds on the loft tree (~1100 .md/.rs/.loft files).
Idempotent — same input always produces byte-identical
output.
