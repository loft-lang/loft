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
| hex_recover `forms_upto` | 7.0× | profiled 2026-10-30: per candidate a `Form` built from two vector literals (a store minted and freed, six pushes growing twice each), then admissibility, canon, write, and a linear search that copies every stored text out to compare it | **BUILT** — the run reservation, −7.3 %.  Priced and not built: the search reading in place −4 %, keeping `ef`'s store across passes −3.4 % (`(R-LoopRecord)` declines a record owning heap) |
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
| the short appends: a piece of up to four bytes copied inline, a one-byte ASCII slice `s[k..k + 1]` answered as that byte before any snapping, a one-byte append and an ASCII character append into a buffer with room a store and a length bump | `c/loft_text.h` `loft_push` / `loft_text_concat`, `c/loft_rt.h` `loft_text_sub` / `loft_push_code`, `-DLOFT_NO_SHORT_APPEND` | gcc, 122 routines: pluginabi `req_state_b64` −66 % (7.9× → ≈ 2.7×), `escape_html` −26 %, `escape_html_blob` −30 %, markdown `render_inline` −31 %, `render` −20 %, `opaque_texels` −13 %, `sha256_small` −10 %, `request` −6 %, nine more −3 … −9 %; 100 the same; slower: `slugify` +6.5 %, `terrain_hydrology` +7 %, `hex_neighbor` +7 %, `save_glb` +5 % (see § The short appends) |
| `(R-Push)`'s sole-growth clause: a loop whose one store growth is its one pushed path's pushes writes each element and the length through the record address the header holds | `hoist::LoopHoist::sole_push`, `c/body.rs` `hold_pushes` / `pushed_mint`, `c/loft_rt.h` `loft_ph_putb*`, `LOFT_NO_PUSH_BASE` | gcc, 122 routines (the C changes in 11 libraries): pluginabi `request` −3 %, `compose` −3 %, the rest the same.  `request`'s decode loop is now the hand-priced loop instruction for instruction, but that row swings with placement (−3 … −11.5 % across builds): its character-class compares on random base64 are data-dependent branches |
| a call's text result moves into its local; a value block whose value is a work text is its view | `c/body.rs` `pending_text_temp` / `text_tail_view`, `loft_text_move`, `LOFT_NO_C_TEXT_MOVE`, `LOFT_NO_C_TAIL_VIEW` | pluginabi `req_state_b64` −5 %, `check_request` −3 % |
| `(R-PushFill)`'s straight-line run clause on the C: a run of pushes onto one path reserves its count first | `c/body.rs` `reserve_push_run`, `loft_vec_reserve_run`, `LOFT_NO_PUSH_RUN_RESERVE` | hex_recover `forms_upto` −7.3 % |
| `(R-SliceArg)`: a slice handed to a `const` parameter is copied into a pooled carrier store, not a store minted for it | `c/body.rs` `slice_carrier`, `loft_slice_take` / `loft_slice_give`, `LOFT_NO_SLICE_ARG_POOL` | pluginabi `check_request` −19.5 %, `req_state_b64` −3.4 % |

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
| `req_state_b64` | 15.8× | crypto's `b64_encode`: each `if … { chars[k..k + 1] } else { "=" }` piece copied into a temporary text; then the per-character append itself (75 % of the row: a library `memcpy` per byte, the slice's boundary snapping, the general append) | **BUILT** — the slice view choice, −36 %; then the short appends, −66 %.  The return copies down the four `return <call>` levels are ≈ 2 % (profiled), not the remaining cost an earlier note guessed |
| `check_request` | 12.1× | cbor's `read_value` (85 %); per bytes value `CBytes { value: bytes[a..b] }` mints a STORE for the slice temporary (`loft_store_make` + `loft_bits_insert` ≈ 13 %), copies it into the field and frees it — rustc does the same | **BUILT** — `(R-SliceInto)` −10 %, and the scope-exit release inlined −10 % (cbor `decode` −20 % with it); what is left is `read_value`'s own frame |
| `request` | 6.4× | crypto's `b64_decode` (70 %), per input byte: the push header's slot and length writes resolve the store (`loft_ph_new`, `loft_at`, the `memcpy`s), `byte_at`, `b64_value` | **BUILT** — `(R-Push)`'s sole-growth clause (the header holds its record's address where the loop grows nothing else), −3 %; what is left is `b64_value`'s compare chain, whose branches the input decides |

Hand-priced 2026-10-30 (the emitted C with the two slice temporaries sliced straight into their
fields, `loft_vec_slice(field, src, lo, hi, 1)`): `check_request` −8.3 % (6.31 → 5.78 ms), the
other rows unchanged, same hashes.  Next: write it as a rule (with the foreign-view case) before
building.

**Tried and dropped (2026-10-30):** one push header held across `head`'s straight-line pushes
(`buf += [b]`, one to three a call), hand-priced on cbor: `encode` −1 %, `encode_bytes` within
noise — deriving the header costs what the pushes save.  `head`'s cost is its call and branches.

### `check_request`: the text arm of `read_value` (2026-10-30, evening)

**Profile** (gcc ship, after the day's builds, 10.4× the twin): 60 % in `read_value`'s
destination twin, diffuse — making stores (`loft_store_make`, `loft_bits_insert`, the claims,
`loft_database*`) ≈ 16 %, copies ≈ 14 %, vector appends ≈ 7 %.  `loft_store_make`'s scan for
the lowest free slot is not the cost: 148 k mints at an average of 7 slots.

**The text arm** — `CText { value: text_from_bytes(bytes[argpos..argpos + arg]) }`, every map
key — mints a store for the slice (for a non-file source the "view" is a copy), copies the bytes
in, converts them (`loft_text_from_bytes`, a fresh text), copies that into the work text, the
work text into a temporary, and the temporary into the field.  The twin borrows `&'a str`.
Hand-priced in the emitted C (same hashes):

| form | `check_request` |
|---|---|
| the arm read straight from the range (`loft_text_from_byte_range` into the field) | −33 % |
| only the slice's store gone (range read, the copies kept) | −27 % |
| only the copies gone (store kept) | −9 % |

**Built:** a call's text result held in a statement temporary MOVES into the text it is assigned
to (`loft_text_move`, `FnWriter::pending_text_temp`, `LOFT_NO_C_TEXT_MOVE`): pluginabi
`req_state_b64` −5 %, `check_request` −2 %.  And a value block whose value is a text local it
leaves alone after (`{ w = f(…); w }`, a call's or a format's work text) is that local's view
instead of a copy into a temporary (`FnWriter::text_tail_view`, `LOFT_NO_C_TAIL_VIEW`); the
operand-order guard (`arg_with`) copies such a view before a later operand that may write, as it
does for a variable: `check_request` −3 %.  The text arm is now conversion, move, field set.
Guard `tests/scripts/a-call-text-moves-into-its-local.loft` (8 cells; the move falsified by a
plant that leaves the source its buffer).

**The store half — built as `(R-SliceArg)` (owner, 2026-10-30: "text from bytes is known to be
read only for the slice, can we use that information here?").**  The information is the
parameter's `const`, so the rule is general: a slice VALUE handed to a `const` parameter is
copied into a pooled carrier store instead of a store minted for it, and the free that ended the
fresh store gives the carrier back.  A zero-copy foreign VIEW over the source's bytes was priced
too and lost: it needs the foreign intercept in every store read of the unit (`request` +2.2 %)
and measured −16.8 % against the copy's −20.7 %, since the keys copied are a few bytes and the
mint was the cost.  Built: pluginabi `check_request` −19.5 %, `req_state_b64` −3.4 %;
`request` +2.5 % is placement (its `base64_to_bytes` byte-identical at another address).

### The evening's regressions (2026-10-30, night)

The four evening changes together, against all four switched off (gcc, 36 libraries): 22 rows
faster, 93 the same, 7 slower.  Split by switch:

| row | cause | finding |
|---|---|---|
| cbor `encode_bytes` +7.4 % | the push-run reservation | REAL: `head` runs as an append twin onto the caller's buffer, which has the room; the reservation was a call for nothing.  FIXED: the clause now requires the vector KNOWN EMPTY at the run (`known_empty`); cbor's C is identical with and without it, `forms_upto` keeps −6.8 % |
| zttext `materialise` +25.6 %, `seg` +4.9 % | the text move | placement: `materialise` contains no move site, and its machine code is the same 513 instructions in both builds (register choices only) |
| html `escape_html_blob` +10.3 % | the text tail view | placement: the changed code is `main`'s row printer, and `t_4text_escape_html__fr` is byte-identical, 0x60 bytes away |
| hex_shape `wall_chain_walk` +3.4 %, hex_field `trace` +2.9 % | — | not split (under 4 %) |

Recorded, not fixed (owner: no layout fixes).

### `check_request`: what is left, and what the rules allow (2026-10-30, night)

After `(R-SliceArg)`: ≈ 1 000 ns a check against the twin's 124 (≈ 8×).  Per check: 7
`read_value` calls, 3 stores, 11 record claims, 39 slice bytes.  By kind: allocating the decoded
tree's records ≈ 31 % (a claim per node, per key text, per vector growth — the pool-reset path
is never reached here: whole-store frees), copies ≈ 21 %, store addressing ≈ 8 %, other
allocator pieces ≈ 13 %, the decoder's own code ≈ 27 % (one ~1 300-line C function, ~95 locals,
17 exits each releasing ~10 temporaries).

The twin borrows (`Cbor<'a>` holds `&str`/`&[u8]` into the frame).  Owner, 2026-10-30: *"We
cannot change the store itself, but we can introduce short lived objects as long as the end
result is the same."*  So the rules written tonight are: `(O-SliceView)` (a slice of a `const`
source may be a view, held outside the store, materialised where it reaches a store record),
`(R-Transient)` (a value built, read and dropped without being stored is built as a short-lived
object outside the store — the decoded tree with its keys and payloads viewing the frame), and
`(R-PathRelease)` (an exit releases only what can hold a store on some path to it).  `(R-Transient)`
is the one that reaches the twin's shape; none is built.

**`(R-Transient)` priced (2026-10-30, late night)**, by hand in the emitted C, same hashes:
both decodes as a tree outside the store, `malloc` per array — gcc **0.109×**, clang 0.129×;
with the arrays in a site-owned bump arena gcc **0.087×** (≈ 87 ns a check, under the twin's
124); the `.ok` decode alone building no tree at all 0.537×.  Design and steps (T1 the dead
tree, T2 the representation, T3 materialisation): [`transient.md`](transient.md).

### The top three after `check_request` (2026-10-31, gcc ship, profiled at `888133d95`)

| row | ratio | what the profile says | lever (general) | priced |
|---|--:|---|---|---|
| cbor `encode_bytes` | ≈ 6.9× | per ITEM, not per byte (64 texts × 256 bytes; the bytes are one `loft_append_text_bytes`): `head` 47 % — three one-byte pushes for a length of 256, each `loft_vec_prealloc(buf, 1, 1)` + `loft_vec_push_byte` resolving the store, behind a frame wrapper, the buffer re-resolved per call (`loft_field`); `encode`'s own arm 40 %.  ≈ 66 ns an item against the twin's ≈ 10 | a push RUN onto a vector that is not known empty: one capacity check for the run, byte stores through a held header (`(R-PushFill)`'s run clause asks known-empty since the `encode_bytes` regression); the per-push prealloc is redundant with the push's own growth | no |
| hex_field `edgeset_count` | 7.2× | the twin runs the SAME algorithm (a six-way `eg_dir_from` per edge); 3.1 ns an edge against 0.43.  `eg_index__inv` 60 %, `eg_dir_from` 17 % — gcc does not inline the helpers (an `__inv` twin behind a frame wrapper) | the C's inlining decision: gcc with its limits raised (`max-inline-insns-single=4000` …) on the same C is **−47 %** (≈ 3.8×); the checked `loft_add`s replaced by wrapping adds on top: −3.6 % only (the integer-null cost is small here) | BUILT 2026-10-31 as `(R-Inline)`'s twin clause, no flags: the `__inv` twin and what its loop calls hinted inline — `edgeset_count` 0.450× (gcc), hex_shape `wall_chain_walk` 0.780×; clang unchanged |
| server `header` | 7.0× | the LIBRARY's algorithm: `loft_text_case` 60 % — every header line lowercased whole per lookup; the twin compares the key with `eq_ignore_ascii_case` | library-side: loft-libs-net PR #33 (server 0.7.6, key compared in place) is OPEN, not merged; the bench checkout is 0.7.5 | by its author: ≈ 3.7× after |

### cbor `encode_bytes` to 2× — priced by hand (2026-10-31)

The emitted C at `06770627d` (`target-exp/prof/cbor/{a..g}.c`, local), `ab.py` 5 rounds, every
form the same hashes (`encode_bytes` 71560779, `encode` de1a932).  Rust twin 57.9 µs an op.

| form | µs an op | × twin | finding |
|---|--:|--:|---|
| today | 418 | 7.2× | |
| A: `head`'s 1–9 bytes as ONE bulk append | 418 | 7.2× | nothing: three pushes cost what one append does — the per-CALL store work is the cost, not the per-byte |
| **B: the output buffer as a push WINDOW kept across the append twins** (`encode__ap` and `head` take the caller's window by pointer; the text's bytes one `memcpy` into it; the array arm opens the window, syncs at its end; other arms sync around their calls) | **132** | **2.28×** | the lever: every append re-resolved the store, re-reserved and re-wrote the length (`loft_append_text_bytes`, `loft_vec_prealloc` + `loft_vec_push_byte`) |
| C: B + the window copied into a local per twin | 155 | 2.7× | worse: the struct copy in and out costs more than it saves — keep it by pointer |
| D: B + `items` held as a header in the outer loop | 129 | 2.2× | nothing |
| E: B + the text arm before the twin's 30-local prologue | 126 | 2.2× | −2 % |
| **F: B + E + the reused buffer KEEPS its capacity** (reserved once) | **116** | **1.94×** | `head` re-mints its holder (`loft_database_wrapper`) and zeroes the vector field (`loft_set_int4(vdb, 0, 0)`) per call, so the buffer reused across the 100 calls regrows from empty every call: `(R-ResetCapacity)` keeps a reused buffer's room only for a store-ROOT vector, not a holder's vector field |

**Built (2026-10-31)** — the window clause of `(R-AppendTwin)` and `(H-KeepRoom)`, on the C:

| form | µs an op | × twin | note |
|---|--:|--:|---|
| before | 409 | 7.1× | |
| `(H-KeepRoom)` alone | 402 | 6.9× | the regrowth hides behind the per-append store work |
| both rules | **255** | **4.4×** | cbor `encode` 0.891×, `decode` unchanged; hashes the same |
| both + gcc's inlining limits raised (no code change) | 166 | 2.9× | the rest of the hand price: it inlined `head` and called the twin bodies directly |

The gap between the built 4.4× and the priced 2.28× is CALL OVERHEAD — the twins' frame
wrappers (8 % and 7.5 % of the profile) and their prologues (`head__ap` initialises ~20 locals for
three byte pushes) — the same lever as `edgeset_count`'s −47 %.  Next for this row: that lever.

Split of F: `make_texts` (the setup, once an op) 19.4 µs against the twin's 8.7 (2.2×); one
`encode` ≈ 0.97 µs against 0.51.  The two general mechanisms that reach 2×: a push window kept
across an append-twin chain (`(R-Header)`'s kept-twin clause, `__kh`, extended from a loop's
held header to the push window an append twin writes through), and `(R-ResetCapacity)` for a
holder's vector field.  Neither is built.

### The short appends (2026-10-30)

**What the row paid.**  `req_state_b64` profiled at 75 % in `b64_encode`'s loop, four
`out += chars[k..k + 1]` a 3-byte group, and ≈ 2 % in the return copies.  Per character: the
slice's general path (a boundary snap back and forward), the append's null tests, and a
`memcpy` whose length the C compiler cannot see — a library call per byte.  The twin pushes a
byte into a `String` with room.

**What was built.**  Four fast paths in the runtime, each answering exactly what the general
path answers, behind one switch (`-DLOFT_NO_SHORT_APPEND`):
`loft_push` copies a piece of up to four bytes inline; `loft_text_sub` answers a one-byte slice
of an ASCII byte (the next byte tested, so even a text that is not valid UTF-8 answers as the
general path does) as that byte; `loft_text_concat` appends one byte into a buffer with room
with a store; `loft_push_code` does the same for an ASCII character (`out += "{c}"`).  The
slice's fast path alone priced −15 %: after it joins the general path the length is unknown
again.  With the concat's one-byte path behind it, the C compiler threads the constant length
through and the append is one store: −67 %, the same as rewriting the slice by hand.

**Measured** (gcc ship, `ab.py`, switch on against off, 36 libraries, 5 alternating rounds):
see the Built row.  The first run had `escape_html` +8 % slower: its common branch is
`loft_push_code`, where the inline byte copy cost more than the `memcpy` did; the character
path took it to −26 %.  The inline copy is still a net gain on its own (without it: `slugify`
+7 %, `opaque_texels` +12 %, `request` +6 %, `escape_html` +3.5 %; `terrain_hydrology` −6 %,
`base64_to_bytes` −3 %).  `terrain_hydrology`'s hot function appends no text — its code differs
only in register allocation between the builds — so its +7 % is placement, as `hex_neighbor`'s
is (owner: no layout fixes).

Guard `tests/scripts/a-one-byte-slice-appended-is-the-general-slice.loft` (11 cells, value and
size, falsified by three planted defects, one per fast path that has a condition).  The corpus
sweep found one more thing: with the inline copy, gcc's overread check sees `loft_push`'s
`memcpy` branch (pieces over four bytes) reached from a four-byte code-point buffer
(`loft_push_code`, the JSON writer's `loft_jv_push_cp`) and strict mode failed on 8 programs.
The branch cannot run for those callers; the check is silenced around that one copy, for gcc
only.  Sweep after: 2552 match, 0 differ, 0 cc-error.

**clang** (both of the day's last changes against both switches off, 36 libraries, 5 rounds):
24 faster — `escape_html` −52 %, `escape_html_blob` −57 %, `html_escape` −39 %, `render_inline`
−34 %, `extract_headings` −27 %, `sha256_small` −25 %, `base64_to_bytes` −25 %, `materialise` −25 %,
`slugify` −25 %, `form_write` −24 %, `render` −23 %, `wrap` −18 %, `req_state_b64` −16 % (clang
was already at 231 µs there, gcc at 908), `request` −10 %; 86 the same; 12 slower by 2–10 %.
Split: the address clause alone moves none of them on clang; six of the twelve have
byte-identical emitted C, and the two whose machine code was compared (time `parse` +5.6 %,
stage `blur_region` +2.1 %) are byte-identical functions at another address — placement;
`blob_put` and `doc_read` did not reproduce, `save_glb` was an outlier round (±455 %).

### `(R-PlaceReturn)` — built, measured slower, reverted (2026-10-30)

**The idea.**  A local vector answered inside the returned literal (`return Doc { buf: nb,
pieces: np }`) is built in the return buffer's store and moved into its field at the exit,
instead of in a store of its own and deep-copied there.

**What was found first.**  The rule exists: `(R-ExitVector)` (`src/exit_vector.rs`) claims the
wrapper in the buffer's store and hands the vector over by a handle move.  It already admits
cbor's `read_value` (`items`, `entries`) and most of zttext's layout functions.  It declined
`insert_text` for three reasons: the early `return d` frees the wrappers before they are minted;
the exit REPLACES the field (`OpReplaceVector`, since the buffer may be the caller's live record
under `d = insert_text(d, …)`), which the rule did not match; and an exit that stages a scalar
field (`n: d.n + 1`) before ensuring the buffer hid the ensure statement from the rule.

**What was built.**  The three clauses (a free that runs before the mint, a replace exit that
clears the field and then moves, the ensure found after staged statements), plus a runtime move
into an allocated empty slot on both runtimes.  Cells:
`tests/scripts/a-local-built-for-the-returned-literal-is-built-in-its-buffers-store.loft`, green on all three backends under strict
stores, poison, `LOFT_HOIST_VERIFY` and the native leak checks.

**What it measured** (gcc ship, `ab.py`, 7 alternating rounds, against the rule as it was):

| row | B/A | |
|---|---|---|
| zttext `insert_text` | 1.146 | slower |
| zttext `patch` | 1.056 | slower |
| zttext `invert` | 1.041 | slower |
| zttext `delete_range` | 1.654 | slower (before the cost decline below) |
| cbor `encode`/`decode` | identical C | already admitted |
| runtime move alone | same everywhere | |

A cost decline (no placement where a loop that grows the local reads an argument's collection)
fixed `delete_range` but cost `patch` its earlier gain, and `insert_text` stayed slower.

**Why.**  The wrapper's own store is the cheapest place a vector can grow.  A fresh store grows
at its tail, in place.  In the buffer's store, which may be an argument's (the rebind hands
`d`'s own record), a growth relocates the vector, and a loop that grows the local loses every
holder over an argument, because `opt::apart` can no longer prove the argument apart from it.
The exit copy the rule saves is one block copy: for `insert_text` about 10 KB against 7.7 µs
per insert.  The premise "the exit copy is what the row pays" was wrong.  `insert_text`'s cost
lies in rebuilding the whole document per insert, which is the library's algorithm.

**Kept:** the cells, as guards of the rule's reach and answers (falsified by a planted
handle-move defect), and this note in `(R-ExitVector)`'s rule text.  **Reverted:** all code.
The patch is in the session scratchpad as `placereturn.patch`; it is not needed again unless
the store model changes so that a shared store grows at its tail.

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
