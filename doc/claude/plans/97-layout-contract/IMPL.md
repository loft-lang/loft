<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# @PLN97 — Implementation steps

Build-level detail for [README.md](README.md). Each step names **what**, **where** (the file it
touches), and **how it's verified**. Ordering + critical path first; then per-phase steps.

## Ordering & critical path

```
A (census) ─▶ B (golden test + layout hash)  ◀── critical path
                     ├─▶ C (formal/layout.md)
                     └─▶ D (sidecar + in-memory identity)
                              ├─▶ E1 (add/drop, automatic)
                              ├─▶ F  (compiler aid; needs the schema diff)
                              └─▶ E2 (reshape migration)      [deferred]
```

**B is the critical path** — it defines the one artifact everything else consumes (the
`layout_algo_hash`) and is the instrument that turns "noticed by breaking" into "red test at
commit." Ship A→B before anything else. C and D parallelise after B; E1 and F ride on D; E2 stays
deferred behind a real-data-reshape trigger.

**Both backends throughout.** Every layout assertion runs on `--interpret` (the store) *and*
`--native` (the `DbRef` ABI); their agreement is itself a contract (shrinks the @PLN89 D-op-1 heap
gap). Use `--interpret` for the seeing loop; verify native at the end of each step.

---

## Phase A — Structure census + `layout(τ)` map

Goal: the exhaustive table of *what lays out how*, and every hidden input, before any test exists.

- **A1 — Enumerate the layout function.** In `src/data.rs`, list the `Type` variants and the
  functions that turn each into bytes: `size(nullable)`, `element_align`, `element_size`,
  `element_offsets`, `stored_tuple_offsets`; the store record/header in `src/store.rs`
  (`[0=SIG,4=free_idx,8=rec_size,12=content]`); the `DbRef` encoding in `src/keys.rs`. Produce a
  row per `Type` variant → (size, align, offset rule, encoding).
- **A2 — Find the hidden inputs.** For each variant, list every input to the layout *beyond the
  type tag*: `nullable` (widening + null-sentinel), keyed-dense (@PLN25), narrow-int (#399),
  nesting depth, inline-vs-boxed, backend ABI (interp store vs native `DbRef`, incl. the synthetic
  `__tuple<…>` struct). This is the axis list the corpus (B) must span.
- **A3 — Falsify "layout is a pure function of `Type`."** Throwaway `/tmp` probes on `--interpret`:
  construct one value per cell, dump its bytes, hand-compute the expected layout. Any cell whose
  bytes depend on something **not** in A2's input list is a finding — record it (that discovery is
  the point; it is the class #477 lived in).
- **A4 — Output:** the census table + the confirmed hidden-input list, written into this file's
  § Census (below). Gate: every `Type` variant appears with its inputs.

## Phase B — Golden layout-conformance test *(the instrument)*

Goal: a test that pins the exact bytes of every structure, fails red on any layout change, and
defines `layout_algo_hash`.

- **B1 — Layout-dump helper.** New test-only utility (`tests/support/layout_dump.rs` or a
  `#[cfg(test)]` module): given a constructed value + its `DbRef`, emit a canonical, human-readable
  descriptor — `size`, `align`, per-field/element offsets + strides, the store header bytes, and
  the raw record bytes (hex). Reads via the store's byte access + `data.rs` functions; no runtime
  cost. **Both a structured table (readable diff) and raw hex (catches encoding changes the table
  misses).**
- **B2 — The corpus.** One fixture per A cell: scalars (base types 0..=6), struct, enum, vector,
  **nested vector** (#477), hash, index, sorted, tuple (+ native synthetic), reference, closure
  record — each × nullable / keyed-dense / narrow-int / nesting. Small `.loft` constructors +
  a Rust driver (`tests/layout_golden.rs`).
- **B3 — The golden assertion.** For each corpus entry, dump (B1) and compare against a checked-in
  golden (`tests/golden/layout/<name>.txt`). Run on **both backends** and assert they agree. A
  layout change → red diff naming exactly what moved. Regen command documented (like the
  `ir_schema` golden pattern).
- **B4 — Coverage self-audit.** A test that is a compile error (or a hard runtime fail) when a new
  `Type` variant is added without a corpus entry: an **exhaustive `match` on the `Type` enum** in
  the coverage map (a new variant → non-exhaustive-match compile error) mapping each to its corpus
  key. Keeps "every structure" true as the language grows.
- **B5 — Derive `layout_algo_hash()`.** A stable hash over the full canonical corpus dump; expose
  `pub fn layout_algo_hash() -> u64` (or a short digest). A test asserts `hash(golden) ==
  layout_algo_hash()` so the hash can never drift from the actual layout. **This is the one artifact
  D and F consume.**
- **B6 — CI.** Add `layout_golden` to the suite (both backends). Gate: green on both, self-audit
  covers every variant, hash test passes.

## Phase C — `formal/layout.md`

Goal: the written contract + its first deviation.

- **C1 — Write the `layout(τ)` rules.** New `doc/claude/formal/layout.md` in the formal/ house
  style (formal rule + "In words" + falsifying examples): per-type size/align/offset, the store
  header format, the `DbRef` encoding, null-sentinel representation, the nullable/keyed-dense/
  narrow-int axes. Rules are the *target*; cite B's golden as the conformance check.
- **C2 — The invariant.** State **"no silent cross-version misread"**: a reader whose
  `layout_algo_hash` differs from a store's recorded one **rejects or migrates, never misreads**.
- **C3 — `D-layout-1`.** Deviation entry: today no layout version exists → a layout change (cite
  #477) is silently misread; closed by D.
- **C4 — Wire in.** Add `layout.md` to `formal/README.md`'s area list + a `formal/ROADMAP.md` row;
  cross-link `heap.md` (semantics) ↔ `layout.md` (format). Gate: `scripts/check_doc_drift.sh` clean.

## Phase D — Schema-description sidecar *(self-describing store)*

Goal: the store carries its schema + layout hash beside it; load classifies the drift.

- **D1 — In-memory identity.** Expose the running program's `(schema, layout_algo_hash)` from the
  compiled `Data`/type table: reuse `ir_schema::data_to_json` (already golden-pinned) restricted to
  the `known_type`s the store references, plus `layout_algo_hash()` (B5). Source of truth lives in
  memory (the constraint).
- **D2 — Sidecar format + placement.** Content: `{ schema: data_to_json, layout_hash, version }`.
  Placement decision (Open Q4): a **second** sidecar (`.dschema`) beside `.dmeta` (which is
  fixed-size integrity), reusing the `store.rs` sidecar machinery. Payload file untouched.
- **D3 — Write on persist.** When a durable store is written (`Store::open_durable` close path),
  emit the `.dschema` sidecar. Gate: round-trip test — write, read back, schema matches.
- **D4 — Compare on load.** Read `.dschema`; compare vs the in-memory identity (D1). Classify:
  identical (raw handoff) · add-only / drop-only (auto, E1) · **add∧drop (actionable, F)** ·
  unreadable/incompatible (reject). Return a typed verdict.
- **D5 — Reject path.** New `CorruptReason::SchemaMismatch` in `src/store.rs`; the incompatible
  verdict routes through the existing `on_corruption` rebuild callback (no new machinery). Gate: a
  store written under a mutated `layout_algo_hash` is *detected*, never misread.

## Phase E — Schema switch

- **E1 — Add/Drop is automatic (prove it).** No new code path: the handoff serialize→deserialize is
  `show_json` (out) / `populate_struct_from_jsonvalue` (in), which is already lenient (missing →
  default, extra → ignored). **Work = tests that prove it**: a store serialized under schema A and
  deserialized under `A+field` (defaulted) and `A−field` (ignored), on both backends. Wire D4's
  add-only/drop-only verdict to run this automatically. Gate: both directions green, no data loss
  beyond the intentionally-dropped part.
- **E2 — Reshape migration *(deferred)*.** Design only until a real reshape needs live data:
  **expand→contract** — an intermediate **superset** schema so each step is a pure Add or Drop (E1),
  with the transform reduced to an **additive backfill** (compute new from old) while both coexist.
  Backfill hook = a loft function the programmer fills (scaffolded by F). Trigger: a shipped
  reshape must preserve data.  Continued as [@PLN196](../196-store-upgrade.md): a store-to-store
  converter from one upgrade map, with a rename in two upgrades.

## Phase F — Compiler migration aid *(the developer surface)*

Goal: the programmer edits structs; the compiler notices, classifies, and scaffolds — the handoff
stays invisible.

- **F1 — The baseline.** A committed schema descriptor (Open Q6) — a project-level schema lockfile
  recording the last-accepted `(schema, layout_hash)` (same `data_to_json` serialization). Decide
  path + format; check it into the project alongside `loft.toml`.
- **F2 — Compile-time diff.** After the two-pass build produces the type table, diff current schema
  vs the baseline: compute the **add-set** and **drop-set** of structures/fields (a structural diff
  over `data_to_json`).
- **F3 — Classify** (mirrors D4): none (silent) · adds-only (note) · drops-only (note) ·
  **adds ∧ drops (actionable)**.
- **F4 — Diagnostic.** For the actionable state, emit via the @PLN28 diagnostic surface: *"schema
  changed in a way that may need migration — added {X}, dropped {Y}"* (a notice, not a hard error —
  the run still works via lenient add/drop; migration is offered, not forced).
- **F5 — Migration-script outline generator.** Emit a scaffolded `.loft` migration stub: the
  mechanical add (default) / drop (remove) lines pre-filled, and one `// TODO: backfill <new> from
  <old>` stub per ambiguous add∧drop pair. Written next to the source or to a `migrations/` dir.
- **F6 — Accept flow.** A CLI path (`loft migrate --accept` or similar) that, once the programmer
  fills + runs the backfill (or confirms the add/drop are independent), **updates the baseline** to
  the new schema. Gate: after accept, F2's diff is empty and no diagnostic fires.

---

## Census (filled by Phase A)

*(A4 writes the `Type`-variant × layout-input table here.)*

## Verification summary

| Phase | Done when |
|---|---|
| A | Every `Type` variant + its hidden inputs enumerated; falsification probes run on both backends |
| B | **v1 + B4 done** (`tests/layout_golden.rs`): `layout_golden` pins size + `Parts` layout over a representative corpus (transitive closure → collection strides), stable `LAYOUT_ALGO_HASH`, proven-fails on a #477-class perturbation. `layout_coverage_audit` (B4): exhaustive `Parts` match (new kind → compile error) + Gap-closed ratchet (proven) + exact-set completeness. **Phase B substantially complete.** Corpus = 11 covered kinds (+ tuple); dynamic both-backend parity (`tests/scripts/509-layout-parity.loft`, interp+native+wasm); **`pub Stores::layout_algo_hash()`/`layout_dump()` exposed** (`src/database/types.rs`) — storage-level (catches #477), consumed by the golden test and ready for D/F. **Residual (audit-tracked, non-blocking):** Sorted-local, Short, Spacial (1.1+), closure DbRef/ChildRec |
| C | `formal/layout.md` written (rules + `D-layout-1`); wired into formal/README + ROADMAP; drift-clean |
| D | **substantially complete** (`src/schema_sidecar.rs` + `CorruptReason::SchemaMismatch`): identity + `.dschema` round-trip + `classify`/`is_actionable` (slice 1); the `.dschema` file lifecycle `write_beside`/`check_beside`→`SchemaVerdict` + `as_corrupt_reason` routing a changed layout to `on_corruption` (slice 2). 7 tests. Residual: `data_to_json` for E2; consumer wiring when a loft builtin drives the durable store |
| E1 | Add-only + drop-only serialize→deserialize green both backends; auto-run on D4's verdict |
| F | Add∧drop drift emits the diagnostic + a fillable migration outline; accept clears the diff |
