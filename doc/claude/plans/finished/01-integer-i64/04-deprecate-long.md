# Phase 4 — C54.B: deprecate `long` + `l` literal suffix

Status: **not started** — blocked by Phase 2 (the widen makes `long`
redundant).

## Timeline (per QUALITY-history.md § 455-462)

- **0.9.0 — deprecate.**  Parser accepts `long` type and `l` suffix
  but emits a warning diagnostic pointing at the migrator.
- **1.0.0 — remove.**  Hard error on both.
- **Repo migration is immediate** on the Phase 4 branch.  Stdlib,
  lib, tests all migrate to `integer` / plain literals on the same
  commit as the deprecation-warning patch — no second sweep needed
  at 1.0.0.

## Critical files — deprecation warnings

### `src/lexer.rs`

The `l` suffix is tokenised at `src/lexer.rs:982-993`:

```rust
fn ret_number(&mut self, r: u64, p: Position, start_zero: bool) -> LexResult {
    let max = i32::MAX as usize;
    if let Some('l') = self.iter.peek() {         // line 984
        self.next_char();
        LexResult::new(LexItem::Long(r), p)        // line 986
    } ...
}
```

Change: after `self.next_char()` at line 985, emit a deprecation
warning:

```rust
self.diagnostic(
    Level::Warning,
    &format!(
        "'l' literal suffix is deprecated; use plain '{r}' (integer is i64 in 0.9.0+).  \
         Run `loft --migrate-long` to rewrite.",
    ),
);
```

Keep emitting `LexItem::Long(r)` — the parse path still works; only
the warning is new.

### `src/data.rs:175` — `Type::Long`

Add a comment marking the variant as deprecated.  Keep the variant
itself until 1.0.0.  The type-keyword-recognition path where `long`
is parsed as `Type::Long` adds a warning at the declaration site.

### `src/parser/definitions.rs`

Find where `long` keyword is recognised and Type::Long is assigned.
Add parallel warning: "'long' type is deprecated; use 'integer' (now
i64 in 0.9.0+)."

## Critical files — migration tool

`loft --migrate-long <path>`:

```
1. Walk the path.  For every `.loft` file:
   a. Parse with the existing parser (fatal-error-tolerant mode).
   b. If parse fails, skip the file with a diagnostic.
   c. Walk the AST:
      - For every `Type::Long` occurrence in a type position,
        replace with `Type::Integer` (the default).
      - For every `LexItem::Long(n)` in a literal position, replace
        with `LexItem::Integer(n)` if `n <= i32::MAX`, else leave
        as-is and emit a diagnostic (literal is out of i32 range
        but plain `integer` literals in 0.9.0 are i64 so this is
        actually safe — the migrator can still rewrite; verify).
   d. Re-emit the source via the formatter.
   e. Atomic-rename.
2. Print a summary: files scanned, files rewritten, files with
   unresolvable references.
```

Dry-run flag: `--dry-run` prints the diff without rewriting.

Critical files for the migrator:

| File | Purpose |
|---|---|
| `src/migrate_long.rs` (new) | Migration logic |
| `src/main.rs` | CLI `--migrate-long <path>` handler |
| `src/formatter.rs` | Re-emit AST as source (already exists; verify it round-trips cleanly for this subset) |

## Stdlib / lib / tests sweep

Per Part B inventory, the volume is:

| Area | `long` type refs | `l` literal refs | Test files |
|---|---|---|---|
| `default/*.loft` | 64 matches (1 type def + 38 opcode stubs + usages) | — | — |
| `lib/*.loft` | 3 matches | — | — |
| `tests/scripts/*.loft` | 25 files affected | — | largest: `01-integers.loft` (26 lines) |
| `tests/docs/*.loft` | ~35 lines across `03-integer`, `13-file`, `15-lexer` | — | — |

**Default (`default/01_code.loft`)** is the tricky one — it defines
the `Op*Long` opcode stubs.  The approach:

1. Keep `Op*Long` opcode definitions in place.  Phase 5 deletes them.
2. Remove the `pub type long size(8);` alias at line 10 (or leave it
   as a deprecated alias pointing to `integer`).
3. Change `long` usages in function signatures throughout the file
   to `integer`.

**Library files** (3 refs total):
- `lib/code.loft:43` — `struct variant Long { v: long }` — rewrite to
  `integer`.  Variant name `Long` stays for now (identifier, not
  type).  May revisit later.
- `lib/lexer.loft:333` — `fn long_int() -> long` — rename + retype to
  `integer_int() -> integer` or leave identifier as-is, just swap
  return type.
- `lib/parser.loft:558` — comment `<long>` → update comment.

**Test sweep**: run the migrator on `tests/scripts/` and
`tests/docs/`.  Verify each rewritten file still parses and tests
still pass.

## Deprecation-warning suppression for the transition

Users with existing `long` / `l` code can't fix it instantly.  Give
them a pragma: `#[allow_deprecated_long]` at file-scope or
function-scope suppresses the warning for that scope.  Document in
LOFT.md.

Alternative: a CLI flag `--suppress-deprecation-warnings` for
build-integration.  Keep both.

## Test plan (from QUALITY-history.md § 460-462)

Un-ignore:

| Test | Purpose |
|---|---|
| `c54b_long_type_deprecated` | Compiler emits warning on `long` type position |
| `c54b_l_literal_deprecated` | Compiler emits warning on `10l` literal |
| `c54b_long_migration_tool_rewrites_type` | Migrator rewrites `long` → `integer` in type positions |
| `c54b_long_migration_tool_rewrites_literal` | Migrator rewrites `10l` → `10` |
| `c54b_long_migration_tool_preserves_identifiers` | Migrator does NOT rewrite `fn long_int(...)` identifier (only types + literals) |
| `c54b_stdlib_no_long` | `grep -rn "\bllong\b\|[0-9]l\b" default/ lib/` returns empty post-sweep |
| `c54b_allow_deprecated_pragma_suppresses` | `#[allow_deprecated_long]` suppresses the warning for its scope |

## Risks

1. **Migrator AST-rewriting correctness.**  Rewriting `.loft` source
   while preserving formatting is non-trivial.  If the formatter
   doesn't round-trip cleanly, open `04a-migration-tool-design.md`.
2. **User impact.**  Every existing loft program with `long` /
   `l` now sees a warning.  Expected; document in CHANGELOG 0.9.0.
3. **Stdlib sweep correctness.**  One missed `long` in
   `default/01_code.loft` = subtle type-error on build.  Run the
   full workspace suite after each file group.
4. **Migration-tool identifier collision.**  A function named
   `long_something` must not be touched.  The migrator walks the
   AST (not a grep) and touches only type positions + literal
   positions.  Add regression tests.

## Budget

**360-480 minutes** — the sweep can be hundreds of files.

Sub-phases that may open:
- `04a-migration-tool-design.md` if the AST-rewriter exceeds ~300 LoC.
- `04b-docs-migration-sweep.md` if the `tests/docs/` sweep surfaces
  semantic regressions (unlikely — just documentation).

## Deliverables

- Deprecation warnings at lexer + parser (both `long` keyword and
  `l` suffix).
- Migration tool (`loft --migrate-long`) with round-trip fixtures.
- Stdlib / lib / tests sweep committed.
- 7 un-ignored tests passing.
- `ROADMAP.md` 0.9.0 entry: `long` deprecated.
- `ROADMAP.md` 1.0.0 entry: `long` removed (future phase).
- `CHANGELOG.md` 0.9.0 entry: migration note + `--migrate-long` CLI.
- QUALITY.md C54.B entry: Closed (for 0.9.0 deprecation; 1.0.0
  removal tracked separately).
