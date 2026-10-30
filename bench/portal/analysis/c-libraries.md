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
| cbor `decode` | 9.0× | the recursive `read_value`: a ~100-local frame set up per call; per element the sub-result buffer is reset, its field moved into the element and the rest released (`loft_remove_claims_mode` 16 %) | native has `(R-Destination)` here (15 sites); the shared analysis is `src/lower/destination.rs`, the twin emission is native-only — a port |
| assets `keys_near` | 8.8× | the walk over `index<Placed[…]>` descended the tree by key compare on every step | **BUILT** — the walk hint, −78 % |
| hex_field `edgeset_count` | 7.7× | small helpers called per cell (`eg_index`, `eg_dir_from`), and `+ 1` after a subtraction is `OpAddIntNullable`, which the probe keeps checked by contract | the integer-null cost, set aside for now; the call overhead of a nine-argument twin (see `roof_match`) |
| server `header` | 7.2× | the LIBRARY's algorithm: `h.to_lowercase().starts_with(prefix)` lowercases and allocates every whole header line per lookup (`loft_text_case` 43 %, already ASCII-fast); the twin compares only the key with `eq_ignore_ascii_case` | library-side (loft-libs-net, the net stream's); told them 2026-10-30 |
| zttext `flow_layout_full` | 7.0× | diffuse: per-run allocation in `slice_runs` (45 %), a fresh `Style` record per resolver call (13 %), store mints and claim bits | open, no single lever |
| hex_recover `forms_upto` | 7.0× | not attributed (99 samples) | open |
| markdown `render_inline` | 5.5× | the per-character path `cs = "{c}"; out += cs` copied the format's work text into a fresh text and freed it per character | **BUILT** — the format clause on C, −28 % |
| zttext `insert_text` | 5.2× | the whole document buffer copied twice: element by element into `nb`, then `return Doc { buf: nb, … }` copies `nb` again into the caller's store; Rust moves it | a NEW mechanism (a returned literal's vector field built in the caller's buffer); rustc copies too — design first |
| cbor `encode` | 4.5× | as `encode_bytes` | **BUILT** — `(R-AppendTwin)` on the C, −27 % |
| hex_roof `roof_match` | 4.2× | call overhead: 26 % in `hexset_get__inv`'s wrapper, 14 % in `seg_distance__inv`'s — twins of 7-statement accessors left out of line, eight arguments with a 24-byte header crossing on the stack.  `(R-Inline)` counts brace-only lines as statements, so `hexset_get`'s twin (≈ 7 statements) counts 14 and misses the ≤ 12 cut | hand-priced: forced inline −59 % (15.0 → 6.2 ms; ±91 % spread on a loaded box) — RE-MEASURE QUIET, then decide whether the count drops brace lines (re-calibrate the 12 over every row) |
| graphics `fill_polygon`, hex_form `side_edges` | 3.6× | not profiled | open |

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

1. `(R-Inline)`'s count, on a QUIET box: re-measure the `roof_match` hand price, then price
   counting statements without brace-only lines over every row (it moves the calibrated 12).
2. `(R-Destination)` on C — cbor `decode` (9.0×); the analysis is `src/lower/destination.rs`.
3. Profile `forms_upto` (longer run), `fill_polygon`, `side_edges`.
4. `insert_text`: design the move of a returned literal's vector field (both writers copy).
5. The full table again on gcc AND clang (`--cc clang`), with pluginabi's checkout moved to
   crypto 0.4.0 (`loft update`) and web's net component built (`make components`).
