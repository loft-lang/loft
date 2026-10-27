<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# tools/ir_schema — store-schema generator for the compiler IR (@PLN11 arc A)

The hybrid pipeline that turns the compiler IR into a store-schema
registration, so `Data`/`Value`/`Type`/… can live as `Stores` records (the
@PLN11 mmap end-goal) and be walked by the schema-driven inspection layer
(`Stores::show_json`).

## Pipeline

```
ir.loft  ──(loft --native --show-rust)──▶  generated.rs  ──(extract.py)──▶  schema block
                                                                  │
                              hand-written typed API (src/data_store.rs) ◀──┘
```

1. **`ir.loft`** (`selfhost/src/ir.loft`, its one home since @PLN91 S0.3: the loft-written
   interpreter reads program images as these same records) — the COMPLETE IR transcribed as loft `struct`/`enum`
   declarations (source of truth, hand-maintained to mirror `src/data.rs`).
   Verified to parse + lay out under `--interpret`.
2. **`generated.rs`** — `loft --native --show-rust` output (gitignored,
   ~139 KB; regenerate, never edit):
   ```
   loft --introspect --show-rust --rust-out tools/ir_schema/generated.rs selfhost/src/ir.loft
   ```
   Its `init(db)` body holds the authoritative schema: `db.structure` /
   `db.enumerate` / `db.field` / `db.value` / `db.vector` with every field
   offset / width / discriminant resolved by the compiler.
3. **`extract.py`** — pulls ONLY the IR-type registrations out of `init`.
4. **hand layer** (`src/data_store.rs`) — the match-able typed API
   (`store_type(&Type) -> DbRef`, a `StoreType` reader) layered on the
   extracted schema; the bit `--native` does not generate.

Why hybrid: generation owns the schema (mechanical, regenerate when the IR
changes — no hand-sync of ~940 sites); the ergonomic typed API is hand-coded
(codegen never emits an accessor layer — see @PLN11 § "What the generated
Rust gives us").

## The drift guard

`make ir-schema-check` (or `scripts/ir_schema_check.sh`) runs the pipeline for real and
byte-compares the result against the committed `src/ir_schema_gen.rs`. It costs ~0.1 s and
is gated by `doc_hygiene::ir_schema_gen_matches_its_loft_source`, which shells out to the
same script — so the gate and the tool cannot drift apart.

**Why it exists.** `ir_schema_gen.rs` says `DO NOT EDIT — regenerate`, and nothing enforced
that. It was hand-edited anyway: `src/keys.rs::Key` gained a third field, `start`, in
loft#812; the generated file was updated to match and **`ir.loft` — the source — was not.
Nobody regenerated for months, so the two stayed out of step invisibly, and the first
regeneration after that silently DROPPED `Key.start`, taking `KEY_STRIDE` from 24 to 16.
A wrong record layout is not a build error; it is a wrong byte offset.

When it fires, decide which side is right. If `ir.loft` is, `make ir-schema-regen` rewrites
the generated file — then run `cargo test --lib baked_layout_mirrors_loft_schema`, which is
what proves `data_store.rs`'s baked `DISC_*` / `ND*` constants still match the new layout.

The check SKIPS rather than fails when there is no built `loft` to regenerate with (a fresh
clone) or no `python3`. `make ci` builds a binary, so there it always runs.

## Extraction findings (2026-06-01)

`extract.py` (probe mode) established:

- **The IR block is name-selectable, not line-sliceable.**  In `init`, our
  type registrations are interleaved with stdlib `db.value(...)` lines (e.g.
  `FieldValue`, `JsonValue` variants) that share the line range but are NOT
  ours.  The extractor selects by type name (`TypeT` / `Node` / the listed
  structs / `Ty*` / `Nd*` variants), not by line span.
- **Our IR types depend only on BASE types** (`t0..t6`: integer 0, single 2,
  float 3, boolean 4, text 5, character 6) — confirmed by scanning every
  `field`/`value` line targeting an IR type.  No stdlib-type dependency.  So
  the extracted block is self-contained: emit a base-type prelude, then our
  registrations with `tN` ids rebased.
- **Variant names are global** and must be unique + CamelCase, hence the
  `Ty`/`Nd` prefixes in `ir.loft` (@PLN11 § Arc A reference, finding 1).
- **`--native` needs dependency-respecting definition order** (`Block` before
  `Node`, etc.) or it emits a forward-reference `E0425`; `ir.loft` is ordered
  accordingly.  Moot under `--interpret`.

## Maintaining

When the IR (`src/data.rs`) changes, update `ir.loft` to match, regenerate
`generated.rs`, re-run `extract.py`, and refresh the hand layer's field
accessors.  The round-trip test (native `Data` → store → read back) is the
guard that the transcription stayed faithful.

```
loft --introspect --show-rust --rust-out tools/ir_schema/generated.rs selfhost/src/ir.loft
python3 tools/ir_schema/extract.py tools/ir_schema/generated.rs > src/ir_schema_gen.rs
```

A regen is **byte-identical** to the committed file when `ir.loft` has not
changed — including from a binary with a different stdlib.  If it is not, one of
the two invariants below has been broken; do not hand-edit `src/ir_schema_gen.rs`
to paper over it, because that is how the file went stale before.

### What keeps regeneration reproducible (2026-08-04)

Regeneration had been unusable, so schema edits were hand-added to the generated
file instead — which let it drift out of sync with `ir.loft` unnoticed.  Two
defects, and one rule:

1. **`tN` labels were absolute.**  `generated.rs` numbers types after the whole
   stdlib, and the extractor copied those names verbatim, so adding ONE stdlib
   type renumbered every label and a regen differed in ~1300 lines.  The
   extractor now relabels our types in declaration order starting at `t7`, after
   the `t0..t6` base prelude, so the output depends on `ir.loft` alone.
2. **Named locals were dropped.**  Only `byte_enum` and `vec_*` were kept, so a
   field whose storage local was `dbref_*` referenced a name nothing bound and
   the regenerated file did not compile.  Every `let <name> = db.…` is kept now;
   an unused one is harmless (the file head allows it and the database dedupes).

**The rule: `ir.loft` describes the STORE, not `src/data.rs`.**  It had drifted
to `NdBlock { block: reference<Block> }` because `data.rs` boxes it, while the
store still INLINED the block and the hand layer read it that way
(`NDBLOCK_BLOCK + BLOCK_SCOPE`).  Regenerating from that declaration produced a
schema nothing could read: SIGSEGV in every IR round-trip test.  Changing how a
field is STORED is a real migration — schema, `ir_store`, `ir_read`, the baked
offsets in `data_store.rs`, and `CACHE_FORMAT_VERSION` — not a transcription
change, so make it deliberately.

That migration has since been done: `NdBlock` / `NdLoop` hold their
sub-record as a **box-of-one vector**, the idiom this schema already uses for
`Block.result` and `DbField.default`.  A box is a 4-byte handle where
`reference<T>` would be a 12-byte `Parts::DbRef` — same indirection, a third of
the width, and every helper (`field_recvec` / `push` / `get`) already exists.
`Node`'s stride went from 48 to 28, which every node in the image pays.
