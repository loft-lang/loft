<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The library benches on the C ship lane

Every library bench `bench/portal/libs.tsv` names, on the C99 ship lane (`cship`: the
release-pass probe, `gcc -std=c99 -O3`) against the library's own Rust twin.  Measured
2026-10-30 on branch `laptop-c-perf` (base `6659d06b4`), laptop, `python3 bench/stats.py
--no-suite --lanes rust,cship <every --package>`, checkouts in `../loft-bench-libs`.

## The table (before this file's changes)

Median ≈ 1.6×.  Over 3.5×:

| routine | ratio | what the time is | state |
|---|---|---|---|
| crypto `base64_to_bytes` | 14.9× | the decode loop pushed every byte through the store: a `return null` inside it freed the pushed vector, which blocked every holder | **BUILT** — `(R-InPlace)`'s exit clause, −71 % |
| cbor `encode_bytes` | 11.7× | `head` re-mints its result per call; each array item is encoded into its own buffer and appended generically (`loft_vec_add` classifies the type per call: `loft_tp_class` 7.7 %); the `CText { value }` binding copies the text | **BUILT** — `(R-AppendTwin)` on the C, −40 % |
| cbor `decode` | 9.0× | the recursive `read_value`: a ~100-local frame set up per call; per element the sub-result buffer is reset, its field moved into the element and the rest released (`loft_remove_claims_mode` 16 %) | **BUILT** — `(R-Destination)` on the C, −56 % (13.1 → 5.7 ms); the ~100-local frame remains |
| assets `keys_near` | 8.8× | the walk over `index<Placed[…]>` descended the tree by key compare on every step | **BUILT** — the walk hint, −78 % |
| hex_field `edgeset_count` | 7.7× | small helpers called per cell (`eg_index`, `eg_dir_from`), and `+ 1` after a subtraction is `OpAddIntNullable`, which the probe keeps checked by contract | the integer-null cost, set aside for now; the call overhead of a nine-argument twin (see `roof_match`) |
| server `header` | 7.2× | the LIBRARY's algorithm: `h.to_lowercase().starts_with(prefix)` lowercases and allocates every whole header line per lookup (`loft_text_case` 43 %, already ASCII-fast); the twin compares only the key with `eq_ignore_ascii_case` | library-side (loft-libs-net, the net stream's); told them 2026-10-30 |
| zttext `flow_layout_full` | 7.0× | diffuse: per-run allocation in `slice_runs` (45 %), a fresh `Style` record per resolver call (13 %), store mints and claim bits | open, no single lever |
| hex_recover `forms_upto` | 7.0× | not attributed (99 samples) | open |
| markdown `render_inline` | 5.5× | the per-character path `cs = "{c}"; out += cs` copied the format's work text into a fresh text and freed it per character | **BUILT** — the format clause on C, −28 % |
| zttext `insert_text` | 5.2× | the whole document buffer copied twice: element by element into `nb`, then `return Doc { buf: nb, … }` copies `nb` again into the caller's store; Rust moves it | a NEW mechanism (a returned literal's vector field built in the caller's buffer); rustc copies too — design first |
| cbor `encode` | 4.5× | as `encode_bytes` | **BUILT** — `(R-AppendTwin)` on the C, −27 % |
| hex_roof `roof_match` | 4.2× | call overhead: 26 % in `hexset_get__inv`'s wrapper, 14 % in `seg_distance__inv`'s — twins of 7-statement accessors left out of line, eight arguments with a 24-byte header crossing on the stack.  `(R-Inline)` counts brace-only lines as statements, so `hexset_get`'s twin (≈ 7 statements) counts 14 and misses the ≤ 12 cut | **BUILT** — `(R-Inline)` counts no brace-only lines: −37 % (gcc A/B; `side_edges` −67 %, `combine_cut` −51 %, `trace` −40 % with it) |
| graphics `fill_polygon` | 3.6× | `polygon_crossings`: four element views a pass (`pc_a`, `pc_b`, and `pc_ty` / `pc_by`, which are one of the two) each resolve the store for their address (`loft_rec_ptr`: `loft_at`, `loft_vh_index`, `loft_store_of` ≈ 38 %); the loop is growth-free (the output is a push window in another store, versioned) | the element's address from the held header's base (`h.base + i·size`), and a chosen view's address copied from the one it names — the general form of the first was tried and dropped (`top5-memory.md`: +37 % on `slope_path_with_undo`); a growth-free-loop clause wants pricing |
| hex_form `side_edges` | 3.6× | the `hexset_get` twin left out of line | **BUILT** — `(R-Inline)`'s count, −67 % |

Not measured: **web, ssh, pluginabi** — a C build never builds a Rust cdylib, and each reaches
a Rust native.  The net stream owns their C story: web runs on C once its manifest declares
`[native] component = "net"` and the net component is built (`make components`); pluginabi
runs once its checkout moves to crypto 0.4.0 (`loft update`; 0.4.0 is a C99 `[c]` shim); ssh
has no C path yet.  **arguments `parse`** printed no C lane row.

## Built (2026-10-30)

| change | where | effect (C ship, gcc, same hashes) |
|---|---|---|
| `(R-InPlace)`'s exit clause: the frees a `return` runs on its way out (a frame-owned store, after the answer) no longer decline the loop's hoist | `opt::writes::exit_tail_frees`, `LOFT_NO_EXIT_RELEASE` | crypto `base64_to_bytes` 76.2 → 21.8 ms (−71 %); shared with rustc's writer (census: 36 rises, no drop) |
| `(R-TextBorrow)`'s format clause on C: a local bound only to its format's own work text is a `loft_str` view of it | `c/body.rs` `work_view_bind`, `LOFT_NO_C_WORK_BORROW` | markdown `render_inline` −28 %, `slugify` −18 %, `render` −17 %, `extract_headings` −5 % |
| the index walk hint: a step tries the slot its last step found, validated against the live tree, before descending by key | `c/loft_btree.h` `loft_bt_hinted` (the twin of `btree::hinted`), `-DLOFT_NO_WALK_HINT` | assets `keys_near` 3.04 → 0.67 ms (−78 %) |
| `(R-AppendTwin)` on C: `X += f(args)` calls `f__ap` with X as the buffer (one more twin bit, `AP_BIT`); the analysis moved to `src/opt/append_twin.rs` | `c/body.rs` `append_site` / `append_call` / `append_twin_buffer_op` | cbor `encode` −27 %, `encode_bytes` −40 % |
| `(R-Destination)` on C: the `__d` twin answers its scalars as a tuple struct; the caller's window mints the element above the call | `c/body.rs` `destination_site` / `dest_scalar_set` / `dest_return` | cbor `decode` −56 % |
| `(R-TextBorrow)`'s tested clause on C: a text bound out of a parameter's record is a view while the parameter's store is not the buffer's | `c/body.rs` `borrowed_assign`, `LOFT_NO_C_TESTED_BORROW` | cbor `encode_bytes` −16 %, `encode` −4 % |
| `(R-Inline)`'s count: lines that only close a block or open its `else` are no statements | `c/body.rs` `statement_count`, `LOFT_INLINE_COUNT_BRACES` | gcc, 119 routines: 6 faster (−5 … −67 %), 112 the same, hex_world `get_cell` +2.8 %; clang: 1 faster (`passable` −15 %), 118 the same |
| `(R-TextBorrow)`'s discharge clause on C: a text `if` choosing between slices is a view, each arm in its branch | `c/body.rs` `view_choice` / `viewable_arm`, `LOFT_NO_VIEW_CHOICE_SLICE` | pluginabi `req_state_b64` −36 % |
| `(R-SliceInto)`: a slice appended to a place is sliced into it, no store of its own | `c/body.rs` `slice_fill` / `slice_into`, `LOFT_NO_SLICE_INTO` | pluginabi `check_request` −10 %, `req_state_b64` −4 % |
| the scope-exit release of a variable inlined (`loft_free_var`, `loft_free_var_distinct`) | `c/loft_rt.h` | cbor `decode` −20 %, pluginabi `check_request` −10 % |
| a text's set and append inlined (`loft_text_set`, `loft_text_concat`) | `c/loft_text.h`, `-DLOFT_NO_TEXT_INLINE` | 119 routines: 13 faster (up to −16 %), 100 the same, five 2–5 % slower, `hex_neighbor` +22 % from code placement only |
| `stats.py` measures every program after one whose lane fails (a worker's exit used to end the run) | `bench/stats.py` `MEASURE_FAILURES` | a 38-library run measured 18 before |

## The census against rustc

`LOFT_REWRITE_CENSUS_FN` per program, the C's rows (phase `c`) beside rustc's (`native`),
over the functions both emit.  Rules the C applies nowhere and rustc applies on these benches:
`R-RangedCall` (384), `R-LitDiv` (546), `R-TextClear` (326), `R-ValueLocal` (262),
`R-GuardFree` (188), `R-RefillBuffer` (142), `R-KeptTwin` (133), `R-LoopBuffer` (78),
`R-Destination` (45), `R-AppendTwin` (31).  Some are counting differences, not missing work:
`R-Leaf`/`R-LeafChain` are frame elisions the C gets by compiling frames away, `R-Wrapper` is
the C compiler's inlining, and `R-Header`/`R-Base`/`R-Scalar` are counted per loop frame on C
and per path on native.  A gap counts only once a profile puts time on it — the four above with
a row are `R-Destination`, `R-AppendTwin`, `R-RefillBuffer` and (already built) the format clause.

## Order from here

The goal stays the worst rows, fixed by general optimisations with the memory model first
([`README.md`](README.md)).  pluginabi's three rows (measurable since crypto 0.4.0), profiled
2026-10-30:

| row | ratio | where the time is | state |
|---|---|---|---|
| `req_state_b64` | 15.8× | crypto's `b64_encode`: each `if … { chars[k..k + 1] } else { "=" }` piece copied into a temporary text; the result copied out of the work buffer at the return (5.4 KB a call) | **BUILT** — the slice view choice, −36 %; the return copy is the C's missing `(R-TextBorrow)` CALL clause (the caller would read the callee's work buffer as a view) |
| `check_request` | 12.1× | cbor's `read_value` (85 %); per bytes value `CBytes { value: bytes[a..b] }` mints a STORE for the slice temporary (`loft_store_make` + `loft_bits_insert` ≈ 13 %), copies it into the field and frees it — rustc does the same | **BUILT** — `(R-SliceInto)` −10 %, and the scope-exit release inlined −10 % (cbor `decode` −20 % with it); what is left is `read_value`'s own frame |
| `request` | 6.4× | crypto's `b64_decode` (70 %), per input byte: the push header's slot and length writes resolve the store (`loft_ph_new`, `loft_at`, the `memcpy`s), `byte_at`, `b64_value` | the push header holds no data pointer by design (a sibling's growth in the same store moves the memory); a base held where the loop's only growth is that header's own wants pricing |

Hand-priced 2026-10-30 (the emitted C with the two slice temporaries sliced straight into their
fields, `loft_vec_slice(field, src, lo, hi, 1)`): `check_request` −8.3 % (6.31 → 5.78 ms), the
other rows unchanged, same hashes.  Next: write it as a rule (with the foreign-view case) before
building.

**Tried and dropped (2026-10-30):** one push header held across `head`'s straight-line pushes
(`buf += [b]`, one to three a call), hand-priced on cbor: `encode` −1 %, `encode_bytes` within
noise — deriving the header costs what the pushes save.  `head`'s cost is its call and branches.

### Design: `(R-PlaceReturn)` — a local built for the returned literal is built in its buffer's store

**The cost.**  A function that builds a vector in a local and answers it as a field of a
returned literal pays a store of its own for the local and a deep copy into the caller's
buffer at the exit, then the store's free: zttext `insert_text` (`return Doc { buf: nb,
pieces: np }`: the whole document buffer copied at every insert, 5.2×), cbor `read_value`
(`CArray { items: items }`, `CMap { entries: es }`: every nested array and map, pluginabi
`check_request` ~9.8×).  rustc copies the same way.

**Why not build in the field.**  Binding the local to the buffer's field from the start fails
under a rebind: `d = insert_text(d, …)` hands the callee `d`'s own record as its buffer
(`(R-Rebind)`), and `nb` is filled from `d.buf` — building in place clears the field it reads.
A hand edit of exactly that changed the answer (hash 368acdef → b704c50b).  The field must not
be touched before the exit; only its STORE can be shared.

**The rule — a clause of `(R-Place)`.**

```
  (R-PlaceReturn) a plain vector local v — declared `v: vector<T> = []` (its witness
                 `__vdb_N` minted by `OpDatabase`), never rebound, captured, linked (`&v`),
                 yielded, stored into another place or handed to a loft call that may keep
                 it — whose ONE owning destination on every path that keeps it is a field f
                 of a literal written into the frame's own return buffer B (an exit
                 `return R { f: v, … }`, a struct's or a record enum variant's field), has
                 its witness CLAIMED IN B's store: `__vdb_N = OpPlaceRecord(B, wrapper)` in
                 place of the mint (B minted first where the caller offered none, as O-Buffer
                 mints it, the entry snapshot `__rbw_B` recording that it did).  At that exit
                 f takes v by RELOCATION (`OpMoveField`: v's data record becomes f's, f's old
                 heap released first — H-ClearRelease — the wrapper record freed record-
                 level), and every other exit releases the wrapper and v's records alone
                 (`OpFreeRecordIn`), plus B where this frame minted it and answers something
                 else.  The state at each exit is a compile-time fact written into the IR,
                 as `(R-Place)` writes it.
```

**Why it is sound.**  f itself is untouched until the exit, so a rebind is as safe as before:
the callee still reads `d.buf` through the argument while `nb` grows elsewhere in the same
store.  Records keep their numbers when a store grows, so every `loft_ref`/`DbRef` stays valid.
What a growth of B's store does invalidate is a RAW address into it — and B's store may be any
argument's store (a rebind, a destination twin's element in the caller's collection):
- **The hoist and apart facts must count v's growth as a growth of B's store.**  Today both
  writers treat v's witness as a store of its own, so `opt::apart`'s "a plain argument's tree
  is apart from this activation's records" holds and a loop may hold a header, a base or a
  record address over an argument while v grows (zttext's `for i in 0..len(d.pieces) { …
  np += [p] }`).  With v placed, v's records are NOT this activation's own: a holder over any
  argument (or B) must not live across a growth of v, exactly as for a sibling vector of one
  store.  This is the obligation the rule carries; the cells below pin it.
- A text argument is passed as a view only to a callee that runs no store write
  (`@FR-F-ParamText`); a callee with a placed local writes B's store, so it already receives
  copies.

**Cells to write first** (one axis each, hand-computed, value AND length, both backends,
`LOFT_STRICT_STORES=1` / `LOFT_NATIVE_LEAK_CHECK=1` for the leak and double-free channels):
1. plain call, a pooled buffer apart from the arguments — the local built, returned;
2. a rebind `d = f(d, …)` where f reads `d`'s vector while building the local (insert_text's
   shape) — the answer, and `d`'s old content intact until the exit;
3. a loop that holds a header over an argument's vector while the placed local grows in the
   same store (insert_text's pieces loop under the rebind) — the header must not go stale;
4. the caller offers no buffer — v placed in a buffer the callee mints; an early exit that
   answers another record frees it (leak check);
5. two locals into two fields (`nb`, `np`);
6. an exit that answers `R { f: [] }` on one path and `R { f: v }` on another — v released
   record-level on the first;
7. a heap-owning element type (`vector<text>`) — relocation keeps the claims; poison mode;
8. a struct-enum variant's field reached through `(R-Destination)` (cbor's `CArray { items }`
   inside `read_value__d`);
9. the local read for its length and indexed before the exit (allowed), and a local also
   handed to a call that stores it (declines).

**Falsifiers to plant:** a relocation that does not clear f's old heap (leak), one that leaves
the source handle (double free under strict stores), and a holder kept across v's growth
(cell 3 reads a stale length or element).

**Price before building:** hand-place `nb`/`np` in `insert_text`'s emitted C (the wrapper
claimed in the buffer's store, `loft_place_record`; the exit's `loft_vec_replace` replaced by
the handle move), with cell 2's rebind shape checked for the same hash, then `read_value`'s
`items`/`es` the same way.  Expected: `insert_text` loses the whole-document copy per insert
(the `memmove`/`copy_claims` share), `check_request` / cbor `decode` the per-array copy.

**Where it lives:** `src/place_result.rs` (the admission and the three IR rewrites, beside
`(R-Place)`'s), the apart/hoist facts in `src/opt/apart.rs` and `src/opt/header.rs` (a placed
local's growth is B's), both writers' `OpPlaceRecord` / `OpMoveField` / `OpFreeRecordIn`
(present already); switch `LOFT_NO_PLACE_RETURN`; the rule text beside `(R-Place)` in
`formal/rewrites.md` once built.

### Design: a slice that initialises a field is sliced into it

**Shape.**  A record literal whose vector field is initialised by a value slice,
`R { f: v[a..b] }` — cbor's `CBytes { value: bytes[argpos..argpos + arg] }`.  Today the slice
is a value of its own: `materialize_iterator` (`parser/convert.rs`) mints a `__vdb_N` store,
binds `__iter_vec_N` to it, clamps the bounds, fills it with one `OpSliceView`, and the literal's
field set then copies it in (`OpAppendVector` → `loft_vec_add`) and the scope frees the store.

**Rule.**  Lower it as `slice_copy_form` (`parser/expressions.rs`) already lowers a slice bound
to a local, with the FIELD as the destination: the clamped-bounds prelude, then one
`OpSliceVector(field, v, lo, hi, row)` — no store, no `__iter_vec`, no copy, no free.  The field
of a literal under construction is empty, so append and replace answer the same.

**Why it is sound.**  The slice's value is fixed by its source and its clamped bounds, both read
before the copy; the destination is a fresh field of a record no other variable names yet, so
the source cannot be the destination (a literal cannot alias its own fields).  It takes the COPY
op, never `OpSliceView`: a field's store holds more than this vector, which is the reason
`slice_copy_form` gives for keeping the copy for a field, so `@FR-Const-Foreign`'s view does not
apply and nothing locks a foreign store.  Both backends run the new IR; the interpreter's
answer is the reference, as for every lowering.

**Cells to write first** (one axis each, hand-computed, value AND length):
in range; `a > b` (empty); bounds past the end (clamped); a negative bound (counts from the
end); a slice of a field of the same record type built in a loop; a heap-owning element type
(`vector<text>`: the copy claims); a slice of a foreign (file-mapped) vector into a field (stays a
copy — the guard that the view was not taken).

**Price.**  `check_request` −8.3 % by hand; cbor `decode`'s bytes items take the same path.



1. server `header`: re-measure after server 0.7.6 (key compared in place, loft-libs-net #33; ≈ 3.7× left by its author's count — the per-call `name.to_lowercase()` and the trimmed value's copy).
2. Profile `forms_upto` (longer run), `fill_polygon`; `edgeset_count`'s remaining cost is the
   integer-null `+ 1` (set aside) and `eg_dir_from`'s six-way neighbour loop.
3. `insert_text`: design the move of a returned literal's vector field (both writers copy).
4. `flow_layout_full`: per-run allocation in `slice_runs` and a `Style` record per resolver call.
5. The full table again on gcc AND clang (`--cc clang`), with pluginabi's checkout moved to
   crypto 0.4.0 (`loft update`) and web's net component built (`make components`).
