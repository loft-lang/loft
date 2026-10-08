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
| check_request | 8.0× | 8.0× | 3.45 / 3.43 | claims 22 %; each decoded text copied three times | decoded texts as views, @C139 (−36 % ceiling, measured earlier) |
| dryopea `panel_build` | 7.9× | 8.3× | 2.82 / 2.96 | claim + release 45 % | a small-size free list in the store allocator (~20 %); straight-line release of a known type (~13 %) |
| dryopea `reload_and_record` | 7.9× | 7.3× | 2.94 / 2.72 | keyed 68 %: the key-ordered walk sorts (19 %), generic multi-key lookup | a typed lookup for an all-integer compound key (~25 %); an order-free walk (~30 %, needs a decision) |
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
   strings), `flow_layout_full` (runs), `arguments parse` (token texts).  @C139 already permits
   the decoded-value form; `(R-DecodeView)` is the proposed rule.
5. **Keyed specifics (2 rows).** A double lookup (`if !h[k] { insert }; b = h[k]`), a replace of
   an identical record, a compound integer key taking the generic `Content` path, and a
   key-ordered walk that sorts a scratch per walk where the twin's `HashMap` iterates unordered.
6. **One-off architecture (2 rows).** JSON parsing through the general lexer and a `Parsed` tree
   (`map_json`); a fixed-width file read per value (`binary_read`).

## Small, pure fixes the comparison surfaced

* **A redundant text clone before a store write** — `let s_val = (x).to_string();
  store.set_str(&s_val)` where `x` is already an owned `String` (`check_request`'s decode): one
  malloc, copy and free per text field written.  An emitter fix; every text field write.
* **`len(t) > 0` counts characters** (`t_4text_len`) where the question is emptiness
  (`arguments`).
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

## Built

* **A copy where Rust moves — the constructor case.**  `(R-CtorLiteral)` (a call of a
  function whose body is one record literal is that literal, a vector-literal argument built
  in its field) and `(R-LoopRecord)`'s refill clause (a record literal in a loop keeps its
  vectors across passes).  `forms_upto` −24.5 % cycles native (priced −31 %; the rest is the
  inlined call's own prologue the hand form also dropped), −54 % interpreted; hash unchanged.
  Receipt: `formal/rewrites-history.md` § 2026-10-08.  A runtime fast path in
  `Stores::vector_add` was priced first and bought −4 % here: the copy routine was not the
  cost, the temporary was.
