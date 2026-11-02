# pluginabi `check_request` — hand-priced to 2x (C ship lane, gcc)

Tip `3cfb11fb9`, unit emitted with `LOFT_RELEASE_PASS_PROBE=1`, `gcc -std=c99 -O3`, driver =
the bench with `main` cut to `bench_check` (both sides), `--n 200`, pinned to CPU 6, `perf stat
-r 5`.  Every step prints hash `ac0a4ec5`.  Cycles were noisy from neighbours: each row's cycles
come from a batch with its predecessor; instructions are exact.  Scripts: `step1.py` … `step7.py`
(cumulative unless marked), units `s1.c` … `s7a.c`, `m.sh` the measurement.

## 1. Baseline

| | instructions | cycles | ns / op (2048 checks) | ratio (cycles) |
|---|--:|--:|--:|--:|
| C ship | 3.757 G | 1.123 G | 1,128 µs | 4.45x |
| Rust twin | 0.969 G | 0.252 G | 252 µs | 1 |

(The C already decodes ONCE — `(R-PureReuse)` folds `pa_decode_ok(frame)` and `pa_decode(frame)`;
the twin decodes twice.)

## 2. Profile (base, self time)

| % | where | what |
|--:|---|---|
| 19.6 | `read_value__d__fr` | the per-entry decode; ~100 locals, plus a store-forwarding stall on its `loft_ref` arguments (§ step 3) |
| 11.0 | `read_value__fr` | the top map; the call into the `__d` twin per key/value |
| 7.6 | `check_request__fr` | with `decode`, `pa_get`, `pa_text` inlined |
| 6.3 | `loft_st_claim_inner` | store claims: a text record per key/text, a vector per byte string |
| 5.8 | `loft_vec_slice` | the 64 + 16 byte strings copied into their fields |
| 5.2 | `loft_remove_claims_mode` | releasing the decoded tree |
| 4.5 | `loft_bits_insert` | claim bits |
| 3.5 | `loft_copy_claims` | `pa_get` copying the matched record into its buffer |
| 3.0 | `loft_text_from_bytes` | each text copied out of its slice |
| 3.0 | `loft_free_ref` | stores freed per check |

## 3. Steps (cumulative)

| step | edit in the C | instr | cycles | x twin | Δ cycles |
|---|---|--:|--:|--:|--:|
| 0 | as emitted | 3.757 G | 1.123 G | 4.45 | |
| 1 | `(R-PathRelease)`: 200 releases of locals null on their path removed (100 per `read_value` twin) | 3.758 G | 1.124 G | 4.45 | 0 |
| 2 | `(R-DecodeView)` instance 1: CText/CBytes hold a SPAN of the frame in the record's spare word (`sign · lo<<32 · n`), no slice store, no `text_from_bytes`, no store text; text validated once; `pa_get`/`pa_text` read the span | 2.350 G | 0.786 G | 3.11 | −30 % |
| 3 | `loft_ref` padded to 16 bytes (`{u16 store_nr; u32 rec; u32 pos; u32 pad}`) | 2.483 G | 0.590 G | 2.33 | −25 % |
| 4 | match bindings as views: `pa_get` compares the key in place, `pa_text` formats from the field (no `loft_text_own`) | 2.409 G | 0.560 G | 2.20 | −5 % |
| 5 | `pa_get` answers `e.value` as a view of its argument: `pa_text` mints no store per call, no `loft_copy_record` | **1.954 G** | **0.432 G** | **1.70** | −23 % |

Alternatives, each applied to the UNTOUCHED base (they replace the decode, so steps 2–5 are moot):

| step | edit | instr | cycles | x twin |
|---|---|--:|--:|--:|
| 6 | `(R-DecodeView)` STRUCTURE clause, as it would emit: `ok` from a shape twin of `read_value` (every build statement dropped), `op` from a walk twin over the top map re-reading each entry at its position; nothing built | **0.383 G** | **0.073 G** | **0.29** |
| 7m | `(R-Transient)` T2: the tree outside the store (`lt_read`, views of the frame), one decode, arrays by `malloc` | 0.508 G | 0.129 G | 0.51 |
| 7a | the same, arrays in the site arena (`loft_ta_mark`/`loft_ta_back`) | 0.459 G | 0.104 G | 0.41 |

Step 3 alone on the base: 1.123 → 0.911 G cycles (−19 %, instructions +3.5 %).  Under clang the
same padding moves nothing beyond the ±10 % noise: the stall is gcc's.

## 4. Mechanisms

1. **`(R-PathRelease)`** (`formal/rewrites.md:4223`, not built) — **worth 0 on the C**: gcc
   already folds the release of a local initialised null and never assigned on that path.
   Drop it from the C plan (it may still matter for rustc/native).
2. **`(R-DecodeView)` instance 1** (@C141, `src/lower/decode_view.rs`, emitted by
   `src/generation/decode_view_emit.rs` — NATIVE ONLY).  C port = NEW in `src/c/`: the span word
   write at the slice-into-field sites, `OpGetText` on a view type reading the span, the frame
   pointer named at the root.  Safe under the rule's (1): the frame read-only in the region.
   Shares the shape: cbor `decode`, pluginabi `req_state_b64` / `request` (their byte field IS
   read — by `bytes_to_base64` — so instance 1's "never read" condition must admit a read-only
   native reader there).
3. **`loft_ref` passed by value: a store-forwarding stall under gcc** — NEW, runtime-wide.  The
   12-byte struct travels in two registers (`store_nr|rec`, `pos`); gcc's SLP vectoriser spills
   both and reloads `rec|pos` as ONE 8-byte load spanning the two stores, which cannot forward
   (≈ 12+ cycles per `loft_ref` argument per call: the two hottest instructions of
   `read_value__d__fr`).  `-fno-tree-slp-vectorize` on the whole unit: −15 %; the 16-byte pad:
   −25 %, at +3–6 % instructions.  NOT priced: the reorder `{u32 rec; u32 pos; u16 store_nr}`,
   whose first register IS `rec|pos` (no extra moves) — it needs every positional `loft_ref`
   initialiser in the runtime rewritten (ten runtime structs nest one).  Every C routine passing
   `loft_ref` by value is a candidate: measure the whole table A/B before choosing pad vs reorder.
4. **`(R-TextBorrow)` match-binding clause** — written as PROPOSED (`formal/rewrites.md:7579`);
   the C's tested clause (`borrowed_assign`) covers a bind out of a parameter's record but did
   not fire here (`pa_get` binds out of an ELEMENT of a parameter's vector, `pa_text` out of a
   call result).  Safe: the arm only compares / formats the binding and writes no store.
5. **A read-only lookup answers a field of its argument as a view** — NEW (no rule found;
   nearest: `(O-ViewField)`, and the structure clause's lookup rewrite).  `pa_get(m, key)`
   returns `e.value` where `m` outlives the call and nothing writes it; today the caller mints a
   16-byte store per call and the callee copies the record (with claims) into it.  Caveat in
   the hand edit: the miss path still mints `CNull` and is no longer freed — the built form
   answers a static null record.
6. **Structure clause on the C** (native only today, `decode_structure_emit.rs`) — the largest
   lever by far: 0.29x the twin, 15x faster than today.  Condition (6) READ ONCE holds here
   (no consumer recursive; the lookup reads each entry once).
7. **`(R-Transient)` T2** — 0.41x with the arena, 0.51x with `malloc`; the S0–S8 WIP
   (`LOFT_TRANSIENT=1`).  Covers sites the structure clause declines (a tree read more than
   once, a recursive reader), at ~1.4x the structure clause's cost here.

## 5. What is left at step 5 (1.70x)

The store-built tree itself: the `Decoded` store minted per check, the entries vector growing
(`loft_ph_grow`, `loft_vec_append_slow` — the count is known up front), the tree's release
(`loft_remove_claims_mode` ≈ 10 %), and `read_value`'s ~100-local frame per entry.  Steps 6/7
remove all of it; on the store form the next levers would be a reservation of the map's known
count and a kept `Decoded` buffer across calls.

## Step table (reply)

| step | mechanism | instr | cycles | x twin |
|---|---|--:|--:|--:|
| 0 | as emitted | 3.757 G | 1.123 G | 4.45 |
| 1 | (R-PathRelease) | 3.758 G | 1.124 G | 4.45 |
| 2 | (R-DecodeView) spans, ported to C | 2.350 G | 0.786 G | 3.11 |
| 3 | loft_ref 16 bytes (gcc stall) | 2.483 G | 0.590 G | 2.33 |
| 4 | match binding as view | 2.409 G | 0.560 G | 2.20 |
| 5 | lookup answers a view of its argument | 1.954 G | 0.432 G | 1.70 |
| 6 alt | (R-DecodeView) structure clause on C (base +6 only) | 0.383 G | 0.073 G | 0.29 |
| 7 alt | (R-Transient) T2, arena (base +7 only) | 0.459 G | 0.104 G | 0.41 |
