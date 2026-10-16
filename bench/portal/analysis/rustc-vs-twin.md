# The slow routines: loft's emitted Rust against the hand-written twin

Twelve of the slowest routines on 2026-10-08 (tree `085826175`), each cut to one routine and
built twice: loft's `--native-release` emission through `bench/portal/hand_price.sh`, and the
`bench.rs` twin with `rustc -O`.  Every pair's output hash is equal.  Counts are user-mode
`perf stat` instructions and cycles per op; the profiles are self time.  The runs were pinned to
efficiency cores (12–15, `cpu_atom`): instruction counts are core-independent, absolute cycles
run higher than the portal's, and every RATIO compares like with like.

## The one finding

**loft does not run its code slower than the twin — it runs 6–24× more of it.**  IPC is equal
or higher on the loft side in every routine (3.1–3.8 against 2.0–3.6), so the gap is
instructions executed, not stalls.  And the generated code is rarely where they go (0.3–38 %
of cycles, 60 % only in cbor `encode`): the extra instructions are the STORE MACHINERY around
values — claiming, copying, releasing, re-reading — that the twin moves, borrows or keeps in a
register.

| routine | instr loft/twin | cycles loft/twin | IPC loft / twin | where loft's time goes | largest lever (share) |
|---|--:|--:|--:|---|---|
| crawler `binary_read` | **24×** | 13.6× | 3.79 / 2.15 | a file op per value: `read_one_i16` 51 %, push loop 25 % | one bulk read, or the file cursor in locals across the loop (~55 %) |
| hex_recover `forms_upto` | 14.1× | 10.8× | 3.73 / 2.84 | vector runtime 38 % — each candidate's vectors built, then deep-copied into the `Form` | build the literal arguments in the callee's result fields (**−35 %, priced**) |
| dryopea `truncate_to` | 10.8× | 10.2× | 3.75 / 3.53 | claims 24 %, deep copy 19 % — the entry copied into the timeline | move the dead entry instead of copying it (~25–30 %) |
| gridmesh `field_add_cell` | 10.4× | 6.8× | 3.14 / 2.04 | keyed 45 %, a record claimed per entry 22 % | one bucket lookup not two (**−6.5 %, priced**); no claim/release on an identical replace (~5–8 %) |
| zttext `flow_layout_full` | 10.4× | 10.4× | 3.27 / 3.25 | runs built as texts (encode, claim, copy) | runs as views of the buffer (~25–30 %) |
| arguments `parse` | 9.7× | 8.5× | 3.40 / 2.99 | generated 38 %, claims 20 %, unheld vector reads 19 % | hold the headers across a write to another vector (~16 %); `len(t) > 0` as emptiness, no `"{c}"` per compare (~9 %) |
| check_request | 8.0× | 8.0× | 3.45 / 3.43 | claims 22 %; each decoded text copied three times | decoded texts as views, @C141 (−36 % ceiling, measured earlier) |
| dryopea `panel_build` | 7.9× | 8.3× | 2.82 / 2.96 | claim + release 45 % | a small-size free list in the store allocator (~20 %); straight-line release of a known type (~13 %) |
| dryopea `reload_and_record` | 7.9× | 7.3× | 2.94 / 2.72 | keyed 68 %: the key-ordered walk sorts (19 %), generic multi-key lookup | a typed lookup for an all-integer compound key (priced −2 % and dropped, worklist.md § 0 item 8); an order-free walk (~30 %, needs a decision) |
| zttext `invert` | 7.9× | 6.1× | 3.58 / 2.77 | deep copy 46 % — the document buffer per op, the prepend | move the dead document buffer into the callee's result (~25–30 %); prepend in place (~15–20 %) |
| cbor `encode` | 6.7× | 6.4× | 3.79 / 3.59 | generated 60 %, vector runtime 17 % | hold the emit loop's headers through a callee writing another store (~15–20 %); byte runs copied as a block (~10 %) |
| moros `map_json` | 6.3× | 6.4× | 3.28 / 3.35 | JSON lexer/parser/walk 55 % | a schema-directed parse straight into the store (@PLN109, ~60–70 %) |

## The mechanisms, by reach

The twelve rows reduce to a handful of mechanisms.  Ordered by how many rows each moves:

1. **A copy where Rust moves (6 rows).** A value built in one place and then deep-copied, whole,
   into where it lives: `forms_upto` (vector arguments stored whole into the callee's result
   literal), `truncate_to` (the entry appended to the timeline), `invert` (`Doc { buf: d.buf }`
   per op; the prepend), cbor `encode` (each key's vector copied into `keybuf`),
   `check_request` (a decoded text copied three times — `text_from_bytes_range().to_string()`,
   a second `.to_string()`, `set_str`), `flow_layout_full` (run texts).  In every case the
   source is DEAD after the copy — an argument the caller never reads again, a temporary.  The
   general rule is `(R-MoveLast)` extended ACROSS A CALL: an argument whose last use is the call,
   stored whole by the callee into a field or collection of its result, is built in (or
   relocated to) that destination.  One instance is priced: **−35 %** on `forms_upto`.
2. **Loop state re-read because something else in the loop writes (4 rows).** `arguments parse`
   (15 unheld accesses: the loop writes `self.results`, which blocks the headers of the vectors it
   only reads), cbor `encode` (the emit loop calls `encode__ap`, which writes `buf`, so
   `ranks`/`koff`/`klen` are fetched generically), `flow_layout_full` (the piece walk), and in its
   own form `binary_read` (a File record's store resolved twice per two-byte read).  The holds
   decline on a WRITE they cannot prove touches a different store; the writes here provably do
   (another local's own store, a callee's buffer).
3. **The store allocator's per-claim cost (3 rows, ~20–25 % each).** `panel_build`,
   `truncate_to`, `check_request`: a claim is a best-fit search of a free-list tree plus a header
   write, a release a red-black insert plus a type walk — roughly 3–5× `malloc`/`free`'s thread
   cache.  A small-size free list in front of the tree is a runtime change, both backends,
   reaching every program.
4. **Materialising what the twin borrows (3 rows).** `check_request` (decoded texts and byte
   strings), `flow_layout_full` (runs), `arguments parse` (token texts).  @C141 already permits
   the decoded-value form; `(R-DecodeView)` is the proposed rule.
5. **Keyed specifics (2 rows).** A double lookup (`if !h[k] { insert }; b = h[k]`), a replace of
   an identical record, a compound integer key taking the generic `Content` path, and a
   key-ordered walk that sorts a scratch per walk where the twin's `HashMap` iterates unordered.
6. **One-off architecture (2 rows).** JSON parsing through the general lexer and a `Parsed` tree
   (`map_json`); a fixed-width file read per value (`binary_read`).

## Re-profiled at `f7f23c5b5` — where each row's time goes

Superseded for the x86-64 cut by [worklist.md](worklist.md) § 0's re-profile at `1ed832afe`
(each routine counted alone under callgrind); this section is the record of the tree before it.

The same rows and the others over 5× on the x86-64 laptop, on the tree rebased onto `main`
(`f0b49ec8f`).  The ratios are `bench/portal/results/laptop.tsv`'s; the shares are user-mode
self time of the `--native-release` binary (§ Method notes, the last item).  The steps these
shares lead to are ordered in [worklist.md](worklist.md) § 0, the x86-64 list.

| routine | × | self time by kind | what it says about the step |
|---|--:|---|---|
| dryopea `truncate_to` | 8.17 | claim and release 22 %, the ownership walks (`owned_walk`, `holds_no_heap`, `remove_claims_mode`) ~15 %, vector runtime 18 %, copy 10 %, generated 16 % | NOT the move of the dead entry the table above names: worklist.md § 4 hand-priced it at +16 %.  `(H-ScanIndex)` took the chain walk; what is left is a copy and a release through the generic type walk |
| zttext `flow_layout_full` | 7.92 | generated 35 % (`slice_runs` 17.5 %, `place_line` 7 %, `next_line` 5 %), claim and release 25 %, vector runtime 16 % | three causes of similar size; no one step takes the row |
| pluginabi `check_request` | 7.85 | claim and release 32 % (spread: `claim_block` 4 %, `claim_best_fit` 3 %, `malloc` / `free` 5 %, `delete` 2.5 %, a store minted and freed per call 4.5 %), the generated cbor reader 16 % (`read_value__d` 12 %, `read_value__rg` 4 %), `store_mut` 4 %, vector appends and slices 9 %, text built from bytes 4 % | flat — no one function over 12 %.  The decoded values are each claimed, copied and released: `(R-DecodeView)` (decided, C141) is the step that removes claims rather than making one cheaper |
| gridmesh `field_add_cell` | 7.54 | keyed 44 % (`find_long` 10 %, `keys::hash` 8 %, `probe_for_insert` 8 %), a record claimed per entry 26 % (`record_new`, `arena::alloc`, `record_finish`, `insert_record`), generated 12 % | the double lookup is real and small; the record per entry has no step |
| moros `map_json` | 7.44 | the lexer and the JSON parser 46 %, `malloc` / `free` 25 %, text 9 % | as above: the parse, and the `Parsed` tree it allocates |
| dryopea `panel_build` | 7.43 | claim and release 45 % (`remove_claims_mode` 8 %, `delete` 6 %, `claim_block` 5 %, `claim_best_fit` 4 %), the release's type walk 8 %, generated 13 % | as above |
| arguments `parse` | 7.18 | the library's own code 45 % (`find_long_opt` 17 %, `classify` 15 %, `parse` 10 %), vector runtime 19 % (`length_vector` 11 %, `get_vector` 6.5 %), claim and release 12 %, `char_count` 5 % | the unheld reads are the 19 %; the 45 % has no named step |
| dryopea `mapfile_to_painted` | 6.93 | keyed 44 % (`keys::hash` 10 %, `find` 5 %, `rehash_into` 4 %), a sort 9 %, generated 18 % (`palette_index_of` 12 %) | as above |
| crawler `binary_read` | 6.8–7.7 | 99 % in the emitted loop (`read_one_i16` inlined into it) | the per-value file operation is typed and inline now (`(R-TypedRead)`); what repeats per value is the handle and the cursor |
| cbor `encode` | 6.81 | generated 50 % (`read_value__d` 17 %, `encode` 11 %, `key_lt` 8 %), vector runtime 23 % | the unheld reads are the 23 % |
| dryopea `reload_and_record` | 6.75 | a sort per ordered walk 23 %, keyed 35 % (`entries`, `find`, `key_compare`, `key_hash`), the ordered step 10 % | as above |
| zttext `invert` | 6.35 | copy 36 % (`copy_claims` 11.5 %, `memmove` 10.5 %, `copy_block` 7 %, `OpCopyRecord` 6 %), generated 31 %, `get_vector` 9 % | as above |
| hex_body `rig_world_frame3` | 5.51 | generated 61 %, vector runtime 26 % (`clear_vector_release`, `length_vector`, `get_vector`, `push_header`), `sincos` 9 % | the cleared and refilled work vectors are apart.md's instance 3 |
| hex_recover `forms_upto` | 5.40 | generated 47 % (`forms_upto` 26 %, `form_closes` 16 %), claim and release 24 %, vector runtime 12 % | `(R-CtorLiteral)` is in; the claims that are left are not attributed |
| zttext `insert_text` | 5.32 | **copy 60 %** (`OpCopyRecord` 26.5 %, `copy_block` 22 %, `memmove` 11 %), generated 20 % | the largest single share on the list: the document copied per operation |
| gridmesh `build_index` | 4.21 | keyed 36 % (`probe_for_insert` 14 %), a record claimed per entry 30 % | the reservation halved the row (8.34×); the record per entry is what is left |

**Built steps deliver less than their price on the portal's core.**  `forms_upto` was priced
−31 % and moved −20 % (6.72× → 5.40×); `(H-ScanIndex)` read −12 % cycles on an efficiency core
and −4 % on `truncate_to`'s row; the bucket replace bought −27 % of a priced −42 %.  An estimate
read off a profile and never hand-priced is worth about two thirds of its figure.

## Small, pure fixes the comparison surfaced

* **A redundant text clone before a store write** — `let s_val = (x).to_string();
  store.set_str(&s_val)` where `x` is already an owned `String` (`check_request`'s decode): one
  malloc, copy and free per text field written.  An emitter fix; every text field write.
* **`len(t) > 0` counts characters** (`t_4text_len`) where the question is emptiness
  (`arguments`).  **Measured and declined:** an IR rewrite to `size(t)` (exact for every
  comparison with 0, and `< 1` / `>= 1`) cut `parse`'s instructions 4.6 % and its native
  cycles not at all (the count runs at high IPC), the interpreter's by 1.8 %.
* **`"{c}" == t` builds a `String` per comparison** (`arguments`' option scan).
* **A per-byte copy loop** (`buf += [src[i] ?? 0]`, `for c in d.buf { nb += [c] }`) where the
  twin copies a block (cbor, zttext).
* **Generated code in a tight scalar loop** is rarely the cost: cbor's `key_lt` rewritten as a
  slice compare saved 3 %.

## Method notes

* **Copying a library copies its `bench/.loft` cache.**  A `--native` run whose cache key
  matches then executes the copied binary, not one the current compiler built; one first
  measurement here came from such a binary.  Clear `bench/.loft` as well as `.loft`.
* **A bench's own µs column can disagree with its cycle count** (zttext `invert` implied
  ~13 GHz): per-op counts here are the difference of two run lengths, which removes setup.
* Pricing a lever by a hand edit is only faithful within one store; `invert`'s two levers cross
  a store boundary and are estimated from the profile, not priced.
* **One row's profile out of a lane that runs many.**  A lane runs its routines one after
  another, so `perf record -F 4000` over the whole binary (no call graph: it unwinds nothing
  for the thread a loft program runs on) and `perf script -F time,ip,sym,dso` give each
  sample a time; a routine's samples are those between the first and the last one in a
  function that carries its name.  Give the routine at least a second of the run.  A library
  routine a bench calls under another name is windowed by that name (`field_add_cell` runs
  in `n_paint_field`), and a neighbour that calls the same function is cut by its own
  (`collect_dirty_inputs` calls `build_index`).

## Built

* **A copy where Rust moves — the constructor case.**  `(R-CtorLiteral)` (a call of a
  function whose body is one record literal is that literal, a vector-literal argument built
  in its field) and `(R-LoopRecord)`'s refill clause (a record literal in a loop keeps its
  vectors across passes).  `forms_upto` −24.5 % cycles native (priced −31 %; the gap to the
  price is not yet attributed), −54 % interpreted; hash unchanged.
  Receipt: `formal/rewrites-history.md` § 2026-10-08.  A runtime fast path in
  `Stores::vector_add` was priced first and bought −4 % here: the copy routine was not the
  cost, the temporary was.
