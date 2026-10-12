# Plan 44 — Hash / keyed-collection semantics: complete the contract

**Status:** ✅ COMPLETE (2026-05-21). Opened to close the keyed-collection
contract surfaced by `gridmesh` lib-plan 19 Phase B (a deduplicating,
clearable dirty-chunk index). All three holes resolved: @P305 + @P307
fixed in code, @P306 closed by decision @C68.

**Goal:** make `hash` / `sorted` / `index` (the keyed collections) behave
the way a keyed map/set should — *every* operation, on *both* backends —
so consumers (gridmesh's dirty index, the tracker indexer, any future map
use) can rely on dedup-by-key, indexed insert/replace, and field-level
clear. Pin the whole contract with one cross-mode spec.

## The spec — what we want from a hash

`hash_spec.loft` (in this dir) is the executable behavior spec: ~28
self-checking cases, runs on `--interpret` AND `--native`, prints
`PASS` / `FAIL want=… got=…` per case. It is the acceptance gate — the
plan is done when every case is `PASS` on both backends. Run:

```bash
./target/release/loft --interpret --no-warnings --lib lib doc/claude/plans/44-hash-semantics/hash_spec.loft
./target/release/loft --native    --no-warnings --lib lib doc/claude/plans/44-hash-semantics/hash_spec.loft
```

### Behavior matrix (measured 2026-05-21, identical on both backends)

| Behavior | Want | Status |
|---|---|---|
| empty literal `h: hash<T[k]> = []` + `len` | 0 | ✅ |
| `h += [T{…}]` append + `len` | grows | ✅ |
| lookup present `h[key]` | the entry | ✅ |
| lookup absent `h[key]` | `null` | ✅ |
| membership `if h[key] { … }` | present→true, absent→false | ✅ |
| `h[key] = null` removal (present + absent) | removed / no-op | ✅ |
| iteration `for e in h` (count, sum) | all entries | ✅ |
| `sorted<T[k]>` ordered iteration | ascending by key | ✅ |
| clear LOCAL `h = []` + reuse | empty, reusable | ✅ (@P302) |
| negative keys | work | ✅ |
| struct-FIELD append `s.h += […]` + lookup | grows / found | ✅ |
| `h[key] = value` **update** an EXISTING key | replaces in place | ✅ |
| struct return with hash field (`build_index`) | works | ✅ (@P300/@P301) |
| **`h[key] = value` INSERT a NEW key** | inserts | ✅ **@P305 fixed** 2026-05-21 (`OpSetKeyed`) |
| **`keyed += [dup-key]` dedup** | replace, `len` stays | ✅ **@P306 fixed** 2026-05-21 (`dedup_keyed` + `sorted_finish`) |
| **clear struct-FIELD `s.h = []`** | empty, no leak | ✅ **@P307 fixed** 2026-05-21 (`OpClearKeyed`) |

**All three fixed in code 2026-05-21** — @P305 (`OpSetKeyed` upsert), @P306
(dedup in the shared insert path: `dedup_keyed` for hash/index, overwrite-on-
found for sorted), @P307 (`OpClearKeyed`).  Both `coll += [entry]` and
`coll[key] = value` now dedup by key (latest wins).  @C68 records the @P306
reverse (briefly closed-by-decision, then implemented once the world-chunk
index made dedup-on-insert a correctness requirement).  **Plan 44 complete.**

## The three bugs

### @P305 — `keyed[key] = value` never INSERTS a new key (silent no-op)
Indexed-assignment **updates** an existing key in place (works — spec C28),
but when the key is **absent** it is a silent no-op: nothing is inserted
(`len` unchanged, lookup stays `null`), on both backends. So there is no
working dedup-insert primitive — the natural `h[k] = v` map idiom can only
mutate keys that already exist. Repro: `/tmp/p_followups/p305_hash_idxset_no_insert.loft`.
*This is the one gridmesh's dirty index wanted (`f.dirty[ck] = ChunkKey{…}`).*

### @P306 — `keyed += [entry]` does not dedup on key collision
`h += [T{ck:10,…}]` twice yields **two** entries with key 10 (`len` grows);
`h[10]` returns the **first**. Same for `sorted`. A keyed collection should
treat the key as an identity — re-inserting a key should replace, not
duplicate. Repro: `/tmp/p_followups/p306_keyed_append_no_dedup.loft`.

### @P307 — keyed struct-FIELD clear `s.h = []` is broken three ways
`fn clr(b: &Bag) { b.d = []; }` (where `b.d: hash<…>`): (1) **compile
error** — `check_ref_mutations` (`src/parser/mod.rs:4768`) doesn't count the
keyed-field assign as a write through `b`, so it rejects the `&` as unused;
(2) **runtime no-op** — the assign emits no write op (native shows a dead
`var_fresh;`); (3) **store leak** on interp. The @P302 fix covered keyed
*locals* (`s = []`); the struct-*field* path was never wired. Repro:
`/tmp/p_followups/p307_keyed_field_clear.loft`.

## Likely fix sites (to confirm during impl)
- **@P305**: the indexed-assignment lowering for keyed LHS (`towards_set` /
  the `h[key] = …` path in `src/parser/collections.rs` — note the existing
  `h[key] = null` removal intercept at `collections.rs:443` is the sibling
  to extend) must emit an insert-or-replace, not an update-only op.
- **@P306**: the keyed `+=` / `new_record` add path
  (`src/parser/vectors.rs` + the per-kind `*::add` in `default/01_code.loft`
  / `src/database/`) should replace on existing key instead of appending.
  *(Decision needed: does `+=` dedup, or is `h[k]=v` the canonical insert
  and `+=` left as-is? Lean: `+=` should dedup — a keyed collection has no
  meaning for duplicate keys, and lookup-returns-first hides the dup.)*
- **@P307**: mirror the @P302 four-layer keyed-LOCAL clear onto the FIELD
  path — (a) recognise the keyed-field assign as a write in
  `find_written_vars` / `find_field_written_vars` (`src/parser/mod.rs`),
  (b) emit the in-place `OpReplaceKeyed`/`OpDatabase` clear against the field
  ref on interp + native, (c) free the prior store (no leak).

## Implementation order
1. **@P307** first — it is the closest sibling of the already-fixed @P302
   (keyed-local clear) and unblocks gridmesh B2's `clear_dirty`.
2. **@P305** — the indexed-insert; unblocks the `f.dirty[ck] = …` dedup
   idiom gridmesh wants.
3. **@P306** — the `+=` dedup decision; lower priority if @P305 lands (a
   guarded `if !h[k] { h += [..] }` is a clean workaround for append-dedup
   once insert works, but `+=` dedup is the more principled fix).
4. Promote `hash_spec.loft` to `tests/scripts/126-hash-semantics.loft`
   (cross-mode wrap + native) once all green; add leak guards in
   `tests/leak_cases/clean/` for the field-clear + insert paths.

## Acceptance
- `hash_spec.loft` all-`PASS` on `--interpret` and `--native`.
- No store leaks under the field-clear + insert + dedup churn
  (`store_memory()` / leak harness).
- gridmesh B2 (`field.dirty[ck] = …` + `clear_dirty`) lands on the clean
  primitives — no workaround in shipped lib code.
- @P305/@P306/@P307 rows in PROBLEMS.md move to **Closed** with guards.
