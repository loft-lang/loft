---
render_with_liquid: false
---
# @PLN10 — Retiring `stores.scratch`

**Status — DONE 2026-06-06.**

The `stores.scratch` global text buffer has been deleted in full.  Everything shipped
on branch `strings` via PR #277.

## What shipped

- **`Stores::scratch` field deleted** (`src/database/mod.rs`) — and the `Scratch`
  newtype + `LOFT_SCRATCH_TRIP` sentinel with it.
- **`OpClearScratch` opcode deleted** — the `clear_scratch` body (`fill.rs`), the
  loft declaration in `default/02_files.loft`, the per-`Value::Line` emission in
  `state/codegen.rs`.
- **18 dead non-dest stub natives retired** (graceful empty stubs first via G3,
  then deleted with the F-cleanup) — `n_kind`, `n_to_json`, `n_to_json_pretty`,
  `n_as_text`, `i_parse_errors`, `n_json_errors`, `struct_to_json_dispatch`,
  `t_4text_replace`, `t_4text_to_lowercase`, `t_4text_to_uppercase`, `n_source_dir`,
  `n_store_memory`, `n_ymd_days_ago`, `n_env_variable`, `n_parallel_buf_get_text`,
  and the wasm-tail producers.
- **Every text producer now dest-passes**: interpreter synth-temp chokepoint
  (`Parser::wrap_value_text_dest`), native codegen (`codegen_runtime.rs` owned
  `String` returns, `native_returns_owned_string` gate), cdylib bridge
  (`Stores::bridge_text_dest` + `n_set_bridge_dest`), wasm tail (`put_owned_text_or_dest`).
- **Goal E for strings reached**: a produced `text` lives only in its destination;
  the runtime holds nothing the source doesn't imply.

## Phase summary

| Phase | Description | Status |
|---|---|---|
| W (keystone) | `native_returns_owned_string` wrapper-return gate | done |
| N1 | Internal text stubs → owned `String` | done |
| N2a | cdylib FFI text wrap → owned `String` (codegen half) | done |
| N2b | cdylib FFI text wrap, interpreter bridge | done |
| wasm tail | `pack_take` + `ws_client_message` wasm-only producers | done |
| A | `as_text` null-carrying return (native) | done |
| B | Central `Value::Return`/`wrap_result` text wraps | done — Direction A |
| C | Chokepoint coverage proof | done (empirically) |
| I1 | Remaining interpreter producers → dest-passing | done |
| D/G1–G5 | Delete fallbacks + field in small verifiable steps | done |
| F-cleanup | Deferred graceful-stubs → real deletion | done |

## Key design insight

The chokepoint that made every producer shape dest-pass with zero per-shape code:
`Parser::wrap_value_text_dest` — walks the body IR at parse time, wraps every
value-position `Call(text-dest-native, args)` with `Block([Set(w, native()), Var(w)])`,
reusing the `set_var` dest-pass for the inner `Set`.  One mechanism covers all 12
matrix shapes (assignment, append, field, format, compare, arg, chain, cond, vpush,
concat, return, loop-body).

The D-execution steps (G1–G5) removed dead code in build-verified increments: remove
all *uses* of `scratch` first (each independently verifiable with the field still
present), so deleting the field became a trivial unused-declaration removal — and the
build was the completeness proof.

## Regression tests

- `tests/scripts/192-text-lifetime.loft` — cross-statement text-lifetime correctness
- `tests/scripts/193-text-dest-synth-temp.loft` — synth-temp chokepoint (Build 2)
- `tests/scripts/194-text-producer-dest.loft` — multi-producer dest-passing (Build 3)
- `tests/scripts/195-*.loft` — null-text / `as_text` regression
- `tests/scripts/196–198` — coverage proof + null-output fix

## See also

- [`D-execution.md`](D-execution.md) — the small-step G1–G5 delete checklist (historical)
- [`ROADMAP.md`](ROADMAP.md) — the dependency-ordered issues (historical record)
- [`01-destination-passing-design.md`](01-destination-passing-design.md) — Design
  Protocol 1 artifact: the matrix that corrected scope + the chokepoint design
  (historical record)
- [LIFETIME.md](../../../LIFETIME.md) — text dep / always-free model
- [PERFORMANCE.md § Open performance work](../../../PERFORMANCE.md#open-performance-work) — N1 (direct-emit
  local collections) cooperated with this plan; now independent
- [QUALITY.md](../../../QUALITY.md) — Dep-inference item cooperated with this plan
