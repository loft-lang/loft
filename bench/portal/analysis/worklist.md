# The worklist to 3× — every routine over the bar, by what is known about it

The bar is `(Perf-Weight)`: every shipped routine within 3× its Rust twin (C136).  This file
orders the WORK between the tree and that bar.  It holds no price of its own: each unit points
at the analysis file beside it that priced it, and at its rule in
`doc/claude/formal/rewrites.md` (§ Proposed for a rule no site implements).

**The release cut.**  The stable release waits only on rows over **5×** (C136's release clause);
those go first, by whatever cure removes the mechanism (no restriction to low-risk changes),
and the 3–5× rows follow after stability.  § 0 is that cut: the rows over 5×, by mechanism,
with what takes each under the line.

**Position.**  `python3 bench/portal/outliers.py bench/portal/results/<host>.tsv` reads the
last full run; it does not start one.  The rows
named here are read off `results/firewall02.lan.betterbe.com.tsv` (arm64 macOS, commit
`0d40d53d0`): 185 routines, 74 over the bar, three classes with a median over it (alloc-temp,
record-field, keyed).  `results/laptop.tsv` (x86-64) is measured whole at `cbe54eaa2`: 160
routines, 81 over the bar, 46 over 3×, 11 over 5× — every lane that builds on that box (drawing,
graphics, text2d, stage and assets do not: X11); § 0 ends with that host's cut.  A row leaves this file when the latest run of EVERY host puts it under the bar; a
unit leaves when it is built, and its record goes to its analysis file's § Built.

| what is known about the row | rows | next action |
|---|--:|---|
| a priced path that lands under the bar | 7 | build the unit (§ 3) |
| a priced path that ends above the bar, or a named step with no price | 16 | build, then re-profile (§ 3, § 4) |
| no price for what the row pays today | 51 | price it (§ 5) |

**The known work does not reach the bar.**  Every unit below, built, clears seven rows and
moves sixteen.  Of the five worst rows only `draw_bezier` has a priced form under 3×; the
other four land at 3.6–5.5× by hand.  Two thirds of the list has never been priced, so the
pricing pass (§ 5) is the larger half of the work.

## 0. The release cut — every row over 5×

Read off `bench/stats.py` runs of every lane that held a row over 5× (arm64 macOS, commit
`801d0137d`, the library checkouts synced as step 0 below): **21 rows over 5×**, plus
`panel_build` (5.03×) and `slope_path_with_undo` (5.09×, noisy) on the line; `truncate_to`
(4.81×) and `emit_to_material` (4.85×) sit just under and stay in view.  `clock_pump`,
`locate` and `mesh_to_floats` have left since (A below), `doc_read` is down to 5.27×: **18
remain**.  The levers already priced take about four more under;
the rest need a lever, a decision (§ 6) or a price.

**Step 0 — the bench checkouts, DONE.**  The checkouts were on `main` while four libraries
keep their benches on `laptop-perf-bench` (`libs.tsv` names the branch per repository); each
bench branch now has `main` merged in, as M2 asks.  `surface_fitted_spread` left the list
with its realigned twin (6.62× → 2.18×); `arguments/parse` (5.69×) and `index_fill_find`
(5.49×) moved with the library.

**A. Record-field rows — two prices, two different mechanisms.**

| row | × | what the price shows |
|---|--:|---|
| `clock_pump` | 6.82 | BUILT: a twin runs `(R-RecPtr)`'s parameter clause over the accesses its inputs leave (`bind_param_record_ptrs`); before, every left-over field access in `n_clock_pump__inv` and its callees re-resolved the store.  `bench/stats.py`: 6.82× → **2.33×** (16.66 ms, the hand price), `clock_advance` 24 → 10.7 ms (1.38×), hashes unchanged.  Leaves the cut |
| `locate` | 6.19 → 4.29 | BUILT: `(R-SameRead)`'s guard clause — the checked sum `cum + p.count` spelled in `if a < cum + p.count { return … }` and again in `cum += p.count` is bound once before the `if` (LLVM cannot merge two checked adds whose overflow path is an opaque call).  8.8 → 6.11 ms, the hand price, hash unchanged; plain adds would be 3.44 ms.  Leaves the cut |
| `mesh_to_floats` | 5.53 → 1.74 | BUILT: `(R-PushFill)`'s deep window — its six float pushes stand under `if mf_v != null` in a literal loop nested in the triangle walk, so no counted clause took them and each went through the header push.  A window opened on the outer loop's push header with no reservation (the growth arm answers a fresh one): 1.11 → 0.35 ms, the hand price, every row's hash unchanged.  Leaves the cut, and the bar |
| `fill_polygon` | 5.28 | 18 store accesses per edge in `polygon_crossings` |
| `stencil_rotate` | 5.09 | accessor helpers (`nb_q`, `nb_r`, `eg_index`) and out-of-line `length_vector` / `get_vector`; the `--names` profile disables inlining, so the helper shares are a lead, not a price.  NOT `clock_pump`'s lever: its helper twins (`eg_index__inv`, `hs_index__inv`, `ly_slot__inv`) read every field from their inputs and touch no store, so no address has anything to serve |

**B. Keyed collections** — the largest bloc; most of it clears 5× only with the keyed store
format decision (§ 6).

| row | × | known lever | lands at |
|---|--:|---|---|
| `field_add_cell` | 7.09 | the library's double bucket lookup (−8 %) | ≈ 5.1× |
| `build_index` | 6.35 | C7 `mint_target` (−8 %) | ≈ 5.8× |
| `index_fill_find` | 3.14 | DONE: `index` is a B+tree of key parts and record numbers (@C147, @PLN190); 5.93× → 3.14× on one host | off the cut |
| `reload_and_record` | 5.76 | P3 (≤ 12 %), P4 (≤ 15 %, needs a range proof) — keyed.md | ≈ 4.1× if both pay |
| `mapfile_to_painted` | 5.44 | the same P3 / P4 | under if both pay |
| `anim_of` | 5.38 | ~45 % is the returned text copied (`malloc`, `memmove`, `free`); the twin returns a borrow: S8 `(R-ViewReturn)` | under, after its decision |

**C. A temporary built, then copied or moved.**

| row | × | known lever | lands at |
|---|--:|---|---|
| `check_request` | 7.95 | `(R-DecodeView)` (decided, C141) | ≈ 5.5× — still over |
| `flow_layout_full` | 7.00 | S1b and S1c are built (S1c after this host's run: −21 % on x86-64); a fused `character` element read | re-measure, then re-price |
| `invert` | 5.82 | `(R-Compact)`'s prepend form, `(R-Rebind)` for a chain exit | unpriced |
| `sphere` | 5.72 | none named (vector growth, the two `add_triangle` writes); 5.97 → 5.72 with the deep window | unpriced |
| `decode` | 5.61 | C1 D4′ RE-PRICED: the resolve clause already serves its byte reads, and a base held from entry (the reads all precede the first growth) is −5.5 % on `read_value__d` and `__rg`, the same with all 12 sites held; the −21 % priced before predates the resolve clause.  And `(R-Base)` never proves a parameter apart from the return buffer, so the held form needs a prefix clause (the base valid up to the first statement that may grow).  Then C2 | ≈ 5.3×, then C2 |

**D. A cause of their own.**

| row | × | known lever | lands at |
|---|--:|---|---|
| `doc_read` | 6.56 → 5.27 | C8 is BUILT (`OpReadFileInt`: one store resolve, a buffered take, two cursor writes).  Profiled at the current tree: `edgeset_new` 22 % — two vectors built side by side, which `(R-PushFill)`'s several-paths clause reserved but pushed element by element.  BUILT: a window per path (`push_windows_ok`), 2.20 → 1.84 ms, hash unchanged.  Left: `hz_index__inv` 13 %, the u8 and i32 reads 10 % and 9 %, `store_mut` 9 %, `hexset_set__inv` 8 %, the float reads through the generic `OpReadFile` 8 % | ≈ 5.3× |
| `map_json` | 6.64 → 2.68 | BUILT (X6, § 0): `T.parse` writes the store as it reads, on the one lexer made cheaper | over-9x.md § moros `map_json` has what is left |
| `arguments/parse` | 5.69 | none named: the option walk reads its vector out of line (16 %), `len(name) > 0` counts characters (4 %), text copies (~10 %) | unpriced |
| `fill_rect` | 5.50 | C9, no bounds test where the index is proved in range | the range-facts decision |
| `rand_indices` | 5.35 | none — unprofiled: `hand_price.sh` cannot link a package with a cdylib (§ Tooling) | fix the tooling, then price |

**Order for the cut.**
1. Step 0 (done).
2. A: `(R-RecPtr)` in the `(R-Inputs)` twins (`clock_pump`) and `(R-SameRead)`'s guard
   clause (`locate`) are built, and the deep window took `mesh_to_floats` under the bar;
   re-measure `fill_polygon`.
3. The priced units: C1 + C2 (`decode`), C8 (`doc_read`), `(R-DecodeView)`
   (`check_request`); `flow_layout_full` re-measured with S1c built.
4. The three decisions, the keyed store format first (most of B), then `(R-ViewReturn)`
   (`anim_of`, `check_request`) and relational range facts (`fill_rect`).
5. Price the rest: `invert`, `sphere`, `map_json`, `arguments/parse`, `rand_indices`.

### The same cut on the x86-64 laptop — the steps, slowest row first

Eleven rows read over 5× in `results/laptop.tsv` (the full run at `cbe54eaa2`, every hash
equal to its twin's); `map_json` (2.85×), `mat4_mul` (0.57×, S2), `rig_world_frame3` (2.24×,
S3), `field_add_cell` (4.63×, X5 a and c), `arguments/parse` (3.76×, X7) and `binary_read`
(4.23×, X8) have left the cut.  The table below is the cut as it was ordered; the re-profile
under it (`1ed832afe`) is where each remaining row's time goes now, and supersedes
[rustc-vs-twin.md](rustc-vs-twin.md) § Re-profiled; "lands at" applies every step named
for the row at its figure, and a figure marked *est.* was read off a profile and never
hand-priced (such a figure has paid about two thirds, same section).

| × | row | steps | lands at |
|--:|---|---|---|
| 4.97 | `check_request` (7.55 before `(R-DecodeView)`) | the decode's result store minted and freed per call; the key bindings copied (S6) | on the line |
| 6.01 | `panel_build` (7.44 before the release plan) | the claims and frees themselves (the store allocator ~26 % of the row), then the text formatting | no figure |
| 7.34 | `arguments/parse` | X7 | the cursor match BUILT — `(R-SameRead)`'s match clause: the subject's length and each integer computation of one element two arm tests make, bound once before the first test (`same_read::bind_match_reads`, `LOFT_NO_MATCH_READ`).  Then counted by callgrind (per parse, loft ~200 k instructions, the twin ~23.7 k; IPC 4.4 against 3.7, so the gap is work, not stalls): `find_long_opt` 51 k (25 %: each `fo.long_name` resolved through the store, the record and the text — a TEXT FIELD of a vector's record element, which `(R-Base)`'s text clause does not serve — and `len(name) > 0` counted by `char_count`, 6 k); `classify`'s own 56 k (the tokens' texts claimed and copied where the twin borrows, `opt_kind`'s scans); `cur = Cur { src: toks, pos: 0 }` 18 k (9 %) — the token vector DEEP-COPIED into the cursor record, though `toks` is dead after it: `(R-MoveLast)` relocates only within one store and `classify`'s result lives in its own; `parse`'s loops over `results` 31 k (a checked element read and a length per iteration of the reset `while`, and of the required-option walk); each matched text copied twice into its result (a `String`, then `set_str`: ~97 `malloc`/`free` pairs, 7 k) | `arguments/parse` | the match clause measured: 25.23 → 24.01 ms native (−4.9 %, 86 % of its −5.7 % hand price), interpreted −0.8 %, hash equal; the dead token vector moved into the cursor record PRICED: −9.1 % (24.14 → 21.94 ms, six interleaved rounds, hash equal) with `Cur`'s record claimed in the token vector's store, its `src` slot taking the vector's handle (no element copied) and `Cur`'s own free dropped (its store is the tokens', freed once); then BUILT as `(R-MoveLast)`'s host clause (`literal_host::rewrite`, `LOFT_NO_LITERAL_HOST`): 24.04 → 22.08 ms native (−8.2 %, about 90 % of the hand price), interpreted −0.9 %, hash equal; `find_long_opt`'s text field PRICED (five interleaved rounds, hash equal): `fo.long_name` read through the held base and the store's text span — the field's record number one load at `base + i × 20 + 4`, the text sliced off the span (`vector::text_at`) instead of resolving the store, the record and the text per element — 22.09 → 19.61 ms (−11.2 %); `len(fo2.long_name) > 0` as a byte test (not null, not empty) instead of `t_4text_len`'s character count, a further −6.3 % (→ 18.32 ms); reading the field once where the prefix loop reads it twice, nothing more; then BUILT: `(R-RecPtr)`'s text clause (`vector::rec_text`, `LOFT_NO_TEXT_FIELD`, native) 22.09 → 18.64 ms alone (−15.6 %), `(R-ZeroCount)` (`src/zero_count.rs`, `LOFT_NO_ZERO_COUNT`, both backends) → 20.50 ms alone (−7.2 %), together → 16.92 ms (−23.4 %, past the −17 % hand price: the fold and the clause serve the library's other walks too); interpreted −1.6 % on layout-pinned builds, hash equal | RE-PROFILED at `69085c245` (callgrind, per parse: loft 133 k instructions, the twin 23.4 k; 16.9 against 3.47 ms, 4.9×): the CHECKED ELEMENT READ leads — `vec_get_or_raise_runtime` 21.6 k (16 %, 120 calls at 180 instructions each: it reads the length, then `get_vector` reads the header and the length again), and with `get_vector` and `length_vector` called from other sites the three helpers are 26 % of the row; `parse`'s reset loop 15.8 k (12 %: 40 checked reads per parse, and `self.results[ri] = null` allocates a `String` for the null text before `refill_str`); `find_long_opt` 18.6 k (14 %, its compares); `classify`'s `argv` walk 8 k (6 %: a parameter's vector, not held, each word copied into a `String`); `opt_kind`'s full scan 5.3 k (4 %, the library's own code).  The checked read as one header read PRICED (a runtime A/B, the same emitted Rust against two rlibs, five interleaved rounds, hash equal): `vec_get_or_raise_runtime` resolving the store, the element record and the length once (`get_vector` too), the in-range path inline and the raise outlined (`@FR-R-Cold`) — 16.89 → 15.60 ms (−7.6 %, instructions 280 → 248 M); with `length_vector` forced inline as well (LLVM declined its `#[inline]`), 14.96 ms (−11.4 %), the binary 0.13 % larger.  Then BUILT — `(R-Cold)`'s one-read clause (`vector::rec_and_len`, the runtime, both backends): 16.92 → 14.67 ms (−13.3 %, past the price, once `get_vector_hoisted`'s end-of-walk half was outlined too — inline, it made `mesh_aabb` 68 % slower); on six other lanes, against the same emitted Rust, `chunk_lookup` −14.6 %, `slope_path_with_undo` −15.6 %, `resolve_move` −11.1 %, `replace` −18 %, and nothing slower beyond 5 % at equal or fewer instructions (layout); the interpreter −5.6 % instructions on this row.  The reset loop (`while ri < self.results.len() { self.results[ri] = null; ri += 1 }`) PRICED (six interleaved rounds, hash equal), 14.64 ms at the tip: the null text written as the constant it is, not copied into a `String` first (`text_value_unborrowed` does not name the null constant) −2.0 %; the walk counted over a header held across the text sets — the vector's record and length, which a text set into an element cannot change, its store resolved once — −3.7 %; both −8.7 % (13.37 ms), the allocation costing more once the walk is cheap.  `(R-Header)` declines this loop over its text set (X2), which can grow the store and so move the BASE, never the header.  Then BUILT: `(R-Header)`'s text clause (`LOFT_NO_TEXT_SET_HEADER`, native) and the null text borrowed (`text_value_unborrowed`) — 14.70 → 13.72 ms (−6.7 %, three quarters of the price: each pass still resolves its element's store and checks the write's index); the clause fires in 3 loops of `arguments` and nowhere else the census reaches.  `classify`'s `argv` walk PRICED (six interleaved rounds, hash equal, 13.68 ms at the tip): each word borrowed from the store instead of copied into a `String` −2.9 %; `argv`'s header held −2.3 %; both −6.1 % (12.85 ms).  Neither is admitted now for one reason: `argv` is a parameter and the loop pushes into the return buffer, which a caller may hand in sharing `argv`'s store — then a push could move the borrowed bytes and the held elements.  Both hold exactly when `argv`'s store is none the loop writes, which one run-time compare of store numbers decides (`(R-Alias)`'s versioned clause, X2).  Then BUILT in `(R-Alias)`'s versioned copy: the copy now counts a walked path for the parameter its dep chain ends at (`argv`'s header and base kept there), and `(R-TextBorrow)`'s versioned clause borrows the word — 13.68 → 12.94 ms (−5.4 %), hash equal; `tween`'s `value`, never targeted, −9.0 % from the same counting.  RE-PROFILED at `19cb78861` (callgrind, per parse: loft 106.5 k instructions, the twin 23.4 k — was 140 k; 12.92 against 3.47 ms, 3.7×; the twin runs the same algorithms and the same `memcmp` total): `find_long_opt` 17.9 k (17 %, near its floor in this layout — a held address, the text's record number, a bounds-checked slice and the compare per option, where the twin's `String` is inline); store claims ~11 k (10 %: `claim` and `claim_best_fit` for each token's record and text, which the twin keeps in a `Vec` and borrows); `opt_kind`'s full scan 6.6 k (6 %, the library's algorithm, the twin's too); `find_short_opt` 6.2 k (6 %: `"{fch}" == fso.short_name` formats the loop-invariant character into a `String` per option, ~3 % of the row); the cursor match's text bindings copied into `String`s ~4 k (4 %); the reset loop 4.9 k (4.6 %: `refill_str` of the null text per element); the required-option walk 3.7 k (3.5 %, unheld: its error exit declines the gate); each held walk's read one past its end through `get_vector_cold` 1.9 k (1.75 %: 27 walk ends per parse at 69 instructions, an answer `elem_index` gives inline).  The store claims PRICED and DROPPED (a runtime A/B, six interleaved rounds, hash equal): per parse 15 claims at ~430 instructions each — the free-tree descent and the wilderness test ~1.5 k, `claim_block`'s split and the remainder's `fl_insert` ~2.3 k, `claim`'s own bookkeeping ~1.1 k, `finish_claim`'s switch reads and zero fill ~0.4 k — so even a free claim is under 6 % of the row's instructions.  A wilderness bump in `claim` (no tracked free block: the claim is the wilderness's front, the writes `claim_best_fit` and `claim_block` make) took 55 % of the claims, claim instructions 12.75 → 9.88 M, and the row 12.99 → 12.90 ms (−0.7 %).  The rest is `clear_vector_release` resetting the token store and claiming its vector record again per parse (3.5 k, 3.3 %).  `find_short_opt`'s `"{fch}"` PRICED and DROPPED (six interleaved rounds, hash equal, 13.05 ms at the tip): the format already reuses a work `String` (cleared, one character pushed per option, one heap allocation per call for the first push); built once before the loop −1.4 %, and encoded into a four-byte stack buffer instead of the `String` −3.6 % (12.58 ms) — the lone-character format's heap buffer is the larger half.  Under the bar, as are what is left of the profile's named rows; the row stands at 3.7× |
| 6.86 | `mapfile_to_painted` | X5 d | ≈ 5.2× if P3 and P4 both pay (*est.*) |
| 6.64 | `reload_and_record` | X5 d | ≈ 3.6× with the walk's decision (*est.*) |
| 6.44 | `invert` | X10 | ≈ 4× (*est.*) |
| 6.27 | `truncate_to` (8.17 before X1) | § 4's row names what is left | no figure |
| 6.13 | `fill_polygon` (measured at `0c4f9fd33`) | X11 | not measurable on this box |
| 5.74 | cbor `encode_bytes` (noisy: its twin reads 76–96 µs between runs, the native time holds at 0.43–0.44 ms) | X9 | no figure |
| 5.73 | `slope_path_with_undo` | § 5's record-build row names what is left | no figure |
| 5.66 | cbor `encode` (6.70 before X2's owned-store half) | X9 | ≈ 5× (*est.*) |
| 5.63 | `flow_layout_full` (7.92 before S1c, 6.14 before the text view's own `to_string`) | X3 | ≈ 4.5× (*est.*) |
| 5.52 | `insert_text` | X10 | ≈ 2.2× (*est.*) |
| 5.09 | `forms_upto` | X12 | no figure |

**Re-profiled at `1ed832afe`** (the portal run of that commit; callgrind, each routine counted
alone with `--toggle-collect`).  Eleven rows over 5×, with the share of the row each lever
addresses and the cut it needs to reach 5×:

| × | row | where the time goes | lever | cut to 5× |
|--:|---|---|---|--:|
| 6.86 | `reload_and_record` | the key-ordered walk of four hashes ~33 % (sort 14 %, `step_ordered` 8 %, `entries`, `OpStep`); the compound-key probe ~32 % (`hash::find` 15 %, `key_compare` 7 %, `int_key_part`, `Stores::find`) — the twin walks its `HashMap` unordered | the walk's decision (§ 6) | −27 % |
| 6.53 | `mapfile_to_painted` | keyed INSERTS: `keys::hash` 13 %, `hash::find` 6 %, `arena::alloc` 4 %, `probe_for_insert`, `rehash_into`; the ordered walk ~6 % | none priced that reaches the line | −23 % |
| 6.27 | `invert` | copies ~48 % (`memcpy` 24 %, `copy_claims`, `copy_block`, `OpCopyRecord`) | X10 | −20 % |
| 6.15 | `truncate_to` | each history push copies and releases a record of plain vectors (`copy_plain_vectors`, `copy_block`, `release_plain_vectors` ~15 %); the allocator ~15 % | the allocator, then the move of the dead entry | −19 % |
| 5.94 | `panel_build` | the allocator ~35 %: ~160 k claims and ~115 k frees per run at ~250 instructions each, where the twin's `malloc`/`free` are ~3 % | the allocator | −16 % |
| 5.63 | cbor `encode` | `key_lt` 20 % — the canonical key order compares keys byte by byte through checked reads, where the twin compares two slices | a slice compare | −11 % |
| 5.61 | `flow_layout_full` | generated code: `slice_runs` 26 %, `place_line` 10 %, `next_line` 6 %, `seg` 5 % | X3 (unpriced) | −11 % |
| 5.17 | `insert_text` | copies ~70 % (`memcpy` 27 %, `copy_block` 20 %, `OpCopyRecord` 18 %): `return Doc { buf: nb, pieces: np }` deep-copies two dead locals into the caller's store | X10 | −3 % |
| 5.17 | cbor `encode_bytes` | the byte runs appended (`encode__ap` 22 %, `head__ap` 9 %, `append_bytes` 8 %, `vector_replace` 7 %) | X9 | −3 % |
| 5.09 | `forms_upto` | generated 54 % (`forms_upto__rg` 36 %, `form_closes` 18 %); the allocator ~9 % | the allocator | −2 % |
| 5.06 | `check_request` | `read_value__dv__d` 17 %, `store_mut` 7 %, the allocator ~8 % | the allocator | −1 % |

The allocator is the one lever five rows share: `Store::claim` and `delete` at ~250 instructions
each (the free tree's descent, a split and the remainder's insert, the claim set) on stores
that have freed blocks; `arguments/parse`'s claims (wilderness, few) priced at −0.7 % and say
nothing of these.

**The allocator's small-block free list PRICED** (a runtime A/B: each bench's emitted source
compiled against the tip's `libloft.rlib` and the probe's, six interleaved rounds, every hash
equal).  The probe parks a freed block of at most 16 words on a list for its exact size, its
header left positive so no neighbour coalesces into it, and a claim of that size pops one
before the tree is asked.  `panel_build` 4.45 → 3.78 ms (−15.2 %, ≈ 5.04× — on the line),
`truncate_to` −12.8 %, `map_json` −4 %; `encode_bytes` −2.9 %, `flow_layout_full` −2.4 %,
`forms_upto` −1.8 %, `arguments/parse` −1.2 %; `check_request`, `invert`, `insert_text`, cbor
`encode`, `resolve_move` and `slope_path_with_undo` within ±1 %.  A 64-word bound priced the
same; peak memory level (10.8 → 10.9 MB on lane 17).  The build is more than the probe: a
parked block is a third block state (neither claimed nor in the tree), which every walk of a
store's blocks must know — the coalescing sweep, `claims_rebuild`, `usage`, an image's save,
`LOFT_POISON`'s scribble and `LOFT_STRICT_STORES` — and the lists must be flushed into the tree
before a claim grows the store, or freed space is never merged.

**cbor `encode`'s slice compare PRICED and DROPPED** (six interleaved rounds, hashes equal): `key_lt`
answering two in-range key spans with one slice compare (`a < b` over the held buffer, the
loop kept for a span past the end, where it reads a missing byte as 0) — `key_lt` 13.56 → 8.84 M
instructions (`memcmp` included), the map encoder 51.7 → 47.0 M, the row's TIME level (8.46 →
8.47 ms): the keys are a few bytes, so the call costs what the loop did.  What is left of the
row is the rank loops themselves (`encode_map__ap` 22 % of the map encoder: `koff[j] ?? 0`,
`klen[j] ?? 0` per pair) and the twin's O(n²) is the same algorithm.

**X9 on cbor `encode_bytes`, PRICED: the bytes are already one block copy** (`(R-ByteCopy)`: the
per-byte loop's in-range fast path is `Stores::append_text_bytes`); what each `CText` arm pays
is the match BINDING — `CText { value }` copies the record's 256-byte text into a `String`
(`get_str(…).to_string()`, a `malloc`, a copy, a `free`) before the block copy reads it.  The
binding borrowed instead (`&str` off the store, as `(R-TextBorrow)` binds a walk's element):
403.5 → 349 µs (−13.5 %, ≈ 4.5× — under the line), `encode` −1.4 %, six interleaved rounds,
hashes equal.  This is S6's match-binding clause; its soundness condition is the borrow's: no
statement of the arm may grow or rewrite the store the text lives in.  Here the arm writes the
return buffer only, and the matched value is a parameter — the versioned clause's test
(`(R-Alias)`: the parameter's store is not the buffer's), or a proof the arm writes none of the
value's store.

The steps these rows name; **Order**, under the table, is the sequence they are worked in.
No row reaches 3× with everything here built; the rows land
between 3.5× and 6.9×.

| | step | rows it moves (share of the row) | state | first action |
|---|---|---|---|---|
| X1 | BUILT, in the runtime and for one family of records: a record that owns plain vectors and nothing else is copied and released by the list of those fields (`Stores::plain_vectors_of`), where `copy_claims`, `owned_walk`, `remove_struct_claims`, `holds_no_heap` and `remove_claims_mode` asked each field's type what it is.  Both backends; a record with any other owner (a text, a vector of records that own heap) still takes the walk | `truncate_to` (8.17× → 6.03×, −26 %); `panel_build` is level — its records own texts | measured | nothing more of this kind on `panel_build`: its release driven by the type's flattened heap slots (texts deleted in a loop, a collection's own walk called per slot) priced at −3 % of the row and was dropped — the walk's descent is not where that row's release spends |
| S1c | BUILT — `(R-WorkBuffer)`'s result clause: a call's vector result that a function only reads is filled into the caller's buffer, so `token_width`'s runs buffer (and `place_line`'s three) outlive the call and `slice_runs` refills kept elements | `flow_layout_full` (7.92× → 6.23×, −21 %; priced −19 % on the emission before it); a switch A/B of lanes 16 and 18, where the clause also fires, is level | measured | X2 next on this row |
| X2 | BUILT for a store the frame owns — `(R-Header)`'s apart clause: a vector the loop only reads, in a store this activation minted or a work buffer the caller lends, and which nothing in the loop is handed, keeps its header and base whatever the loop calls (no callee verdict is asked).  NOT built: the same for a PARAMETER's vectors, which a callee can be handed by another route — `(R-Alias)`'s versioned clause with a run-time test | cbor `encode` (−20 %, the hand price: `ranks`, `koff`, `klen`, `keybuf`); `flow_layout_full` level (its headers were held already or root in a parameter); `arguments/parse` and `rig_world_frame3` unmoved — their unheld vectors are fields of a parameter record | measured | nothing more on `arguments/parse`: PRICED and dropped.  Its declines are a text set through an existing address (`self.results[i] = word`, `OpSetTextReplace`) and a free on an exit path; with the gate admitting both — a ceiling, since neither is sound as the gate's other readers stand — the row moves −5.3 % (25.45 → 24.10 ms, 21 → 15 unheld reads), the text set alone −4.5 %.  What is left of its vector runtime is single reads outside any loop |
| X3 | flow's runs as views of the buffer they are cut from | `flow_layout_full` (claim and release 25 % before S1c) | *est.* only | after X2: re-profile, then price |
| X4 | BUILT — `hand_price.sh` links a package's native library (§ Tooling) | `check_request`, `pluginabi/request`, `rand_indices` | tooling | `check_request` is profiled (rustc-vs-twin.md § Re-profiled: claims 32 %, spread); `(R-DecodeView)` (decided, C141) next, S8 after its decision |
| X5 | keyed inserts and walks: **a** BUILT — the second lookup of a get-or-insert (`if !h[k] { h += […] }; b = h[k]` looked the key up twice on a hit) is `(R-SameRead)`'s get-or-insert clause, an ENGINE cure rather than the library edit first planned: a loft rewrite of a library for speed is the last resort after code generation (LIBRARY_AUTHORING.md § The library contract, rule 7), and the idiom is everyone's; **b** C7; **c** BUILT for an integer-keyed hash of a heap-free element — `(R-KeyedEntry)`: one probe answers the entry the key names, a repeated key is overwritten in place (no claim, no release) and a new one is filed under the index its claim answered (no `arena::index_of`); an element that owns heap, a text key and the other keyed kinds keep the claim per entry; **d** P3, the palette scan as a slice compare, and P4, an order-free walk that need not sort its scratch (over-9x.md § dryopea) | `field_add_cell` (c: 22.95 → 15.08 ms, −34 %; a: 15.04 → 13.63 ms, −9.4 % of the −12.7 % priced), `build_index` (c: 10.11 → 7.50 ms, −26 %; b 8 %), `collect_dirty_inputs` (c: −10 %), `mapfile_to_painted` (P3 ≤ 12 %, P4 ≤ 15 %; the sort is 9 % of the row), `reload_and_record` (the sort 23 %) | a, c built; d unpriced, and P4 needs a range proof | `field_add_cell` is left with the table's growth (~15 %, a re-hash of every entry from its record) and the new keys' arena claims; d next |
| X6 | BUILT — `map_json`: `T.parse` writes the store as it reads, with no `Parsed` tree between (J9), read by the one lexer made cheaper — a cursor over the text's own bytes (J4), token offsets from the lexer (J5), a number and a string scanned once (J6, J7), no text allocated per punctuation token (J8), each token written where it is kept (J10); over-9x.md § moros `map_json` | `map_json` | measured: 7.44× → 2.68× (J4 −2 %, J8 −12.6 %, J6 and J7 −17 %, J9 −33 %, J5 and J10 −22 %) | the row is under 5× and has left this list; over-9x.md § What is left |
| S9 | sized, and its build half BUILT in the runtime: the buttons and the list already build straight into the return buffer, and each frame's buffer is reused, so what was left was the RELEASE of the last frame's tree — 36 % of the row's instructions, two thirds of that the type walk (`remove_claims_mode`, `remove_struct_claims`, `holds_no_heap`, `owned_walk`'s child list), not the frees.  A record whose heap is texts and vectors of text-owning elements is now released by its type's plan (`Stores::release_by_plan`, `LOFT_NO_RELEASE_PLAN`, `@FR-H-ClearRelease`'s record clause) | `panel_build` | 7.44× → 6.00× (−19.4 %, hand-priced at −18 % by an rlib A/B first); `emit_to_material` −2 % | the frees and claims themselves (~26 % of the row, `delete`, `claim_block`, `claim_best_fit`, `set_free_header`): not an allocator change (two were measured on `truncate_to` and dropped, § 4) — the form @C125 asks for is a claim removed, here a text refilled in its own block |
| X7 | read by line.  `find_long_opt__inv` (17 %): the options vector's header and base are held, but each `fo.long_name` is a TEXT FIELD of a record element, which `(R-Base)`'s text clause (text ELEMENTS only) does not serve — every read resolves the store, the record and the text; the prefix loop reads the field twice and counts its characters for `len > 0` (`char_count` 4.7 %).  `parse` (10 %, and most of `length_vector`'s 10.6 % and `vec_get_or_raise_runtime`'s 3.5 %): the cursor `match` re-tests the subject's length, re-reads element 0 and re-decodes its variant byte in EACH arm's guard, up to six arms per token, where the twin reads the tag once.  `classify` (15 %): `opt_kind`'s full scan, already a counting loop (~3 % of the row), and each token's text copied into its `Tok` (the twin borrows: `copy_claims` 3.9 %, `malloc`/`free` ~5 %) | `arguments/parse` | the cursor match PRICED: −7.4 % (25.52 → 23.63 ms, four interleaved rounds, hash equal) with the subject's length, elements 0 and 1 and their variant bytes read once per `match` instead of in each arm's guard and binding (6 length reads, 13 + 2 element reads, 7 variant decodes per token); the length and elements alone −5.2 % | build it in `parse_vector_match` (`src/parser/control.rs`), so both backends get it: the guards are pure reads and only the arm that matches runs its body, so a read the arms share is bound once before the first.  Then price a text field of a held vector's element read through the held base |
| X8 | BUILT — `(R-FileHold)`: a loop that names one `File` and nothing else that can hold one, and names it only to read scalars, moves the cursor in a guard local and writes the record when the loop is left; `(R-InlineLeaf)`'s reader clause puts a one-statement reader (`read_one_i16(bf)`) in the loop | `binary_read` | 1.47 → 0.84 ms, the hand price ([x8/price_binary_read.py](x8/price_binary_read.py)), hash unchanged | [formal/rewrites.md](../../../doc/claude/formal/rewrites.md) |
| X13 | the cursor kept in the HANDLE while one is open | — | DECLINED: each `File` record keeps its own position (`formal/heap.md` § The built-in `File`, loft#2005), so a read must ask its record where to start; the hold (X8) is what removes that per value inside a loop | — |
| X9 | cbor's byte runs copied as a block; `read_value__d` (17 % of `encode`'s row) | cbor `encode` | *est.* ~10 % | after X2 |
| X10 | a record whose source is dead is moved, not deep-copied: `(R-Compact)`'s prepend and identity forms, `(R-Rebind)` for a chain exit and for the return buffer passed as a parameter | `insert_text` (**60 %**; 4.84× now, under the cut), `invert` (36 %) | named in § 4, no price | hand-price on `insert_text`.  The largest single share on this list; it stands here by its row's ratio, as the cut is ordered |
| X11 | `graphics`, `drawing`, `text2d`, `stage` and `assets` build on this box again: they resolve dependencies (`mesh3d` 0.1.2 through `drawing`'s lock file, `shapes`) that lack the `pub` fields @C140 requires, and `main`'s own compiler refuses them the same way | `fill_polygon`, and those five libraries' census rows | library work | bump the pins, then re-measure the lane |
| S3 | § 3 | `rig_world_frame3` 2.59× | BUILT (and S2: `mat4_mul` 0.57×) | — |
| X12 | `forms_upto`'s remaining claims attributed (`(R-CtorLiteral)` and the refill clause are built) | `forms_upto` (claim and release 24 %) | no step | read `forms_upto__rg` and `form_closes` by line |

**Order.**  Three rules, applied in this order: a step another step or a row WAITS on goes
before it; the slowest row goes first; a step with no price is priced before it is built.

1. **Unblock** — X4 is built (`check_request` can be profiled).  X11 is open: `fill_polygon`
   cannot be measured on this box until the five libraries' pins are bumped.
2. **`truncate_to`** — X1 is built for a record of plain vectors (8.17× → 6.03×).  It did
   not move `panel_build`, whose records own texts.
3. **`flow_layout_full` (5.57×)** — S1c and X2's owned-store half are built; X2 took cbor
   `encode` and left this row level.  Re-profiled: generated 43 % (`slice_runs` 22 %), claim
   and release 26 %, vector runtime 18 %; the 14 % that was text formatting is gone
   (`Str::to_string`, § 3 S13).  The run's text borrowed from its call's buffer, never copied
   to a local, prices at −3.4 % and is dropped.  Next: price X3.
4. **`check_request` (5.07×)** — `(R-DecodeView)` instance 1 is BUILT: 2.55 → 1.67 ms
   (−34 %, 80 % of the −46 % hand price).  The row is on the line; what is left is flat —
   the store `decode` mints and frees per call (~10 %), the entries vector's build, and the
   key bindings copied (S6's match-binding clause, priced −1.5 %).
5. **`field_add_cell`** — X5 a and c are built: `(R-KeyedEntry)` and `(R-SameRead)`'s
   get-or-insert clause, 7.68× → 4.54× (−41 %); `build_index` 4.16× → 3.11×, and the keyed
   lane's `hash_fill`, never targeted, 3.83× → 2.98×.  The row has left the cut; what is left
   on it is the table's growth (~15 %, every entry re-hashed from its record) and the new
   keys' arena claims.
6. **`panel_build`** — S9 is sized and its release half built: the release plan,
   7.44× → 6.00×.  What is left is the frees and claims themselves.
7. **`arguments/parse` — LEFT the cut (3.76× at `cbe54eaa2`)** — X7: the match clause is built (−4.9 %), and so is the
   dead token vector moved into the cursor record (`(R-MoveLast)`'s host clause, −8.2 %);
   the text field read through the held base and `len(t) > 0` as a byte test are built too
   (−23.4 % together).  Re-profiled: the checked element read (`vec_get_or_raise_runtime`,
   `get_vector`, `length_vector`) is 26 % of the row, the reset loop 12 %, the token copies
   of `argv` 6 %.  The checked read as one header read, `length_vector` inlined, PRICED at
   −11.4 %, and BUILT at −13.3 % (16.92 → 14.67 ms).  The reset loop PRICED at −8.7 % (the null
   text uncopied, the walk counted over a held header), and BUILT at −6.7 % (14.70 → 13.72 ms);
   `classify`'s `argv` walk PRICED at −6.1 % and BUILT at −5.4 % (13.68 → 12.94 ms) in the
   versioned copy.  Re-profiled at 106.5 k instructions per parse (the twin 23.4 k):
   `find_long_opt` 17 % near its floor, store claims 10 %, `opt_kind` 6 %, `find_short_opt`'s
   per-option format 3 %.  The claims PRICED and dropped (a wilderness bump −0.7 %), and the
   invariant format too (−3.6 % at best, a stack buffer); the row stands at 3.7×.
8. **`mapfile_to_painted` (6.86×), `reload_and_record` (6.64×)** — X5 d: P3 first, P4 with
   its range proof.  Re-profiled: the key-ordered walk's sort is 22 % of `reload_and_record`
   and 9 % of `mapfile_to_painted`; the generic compound-key probe (`find`, `key_hash`,
   `key_compare`, `int_key_part`, `arena::slot`) ~28 % of the first and `keys::hash` 12 % of
   the second.  A typed probe for an all-integer compound key (`@FR-R-TypedKeyed` taken to two
   to four integer fields, the values handed over in key order) PRICED and dropped: the five
   lookups of `reload_and_record` through it read 1.415 → 1.385 ms (−2 %), `mapfile_to_painted`
   level, hashes equal.  The `Content` was never the cost — the typed probe itself is 15 % of
   the row: the seeded SipHash (kept: integer keys can come from outside, @P253) and the walk's
   three dependent loads (bucket, arena chunk, entry) where the twin's table has one.  What is
   left of these two rows is the ordered walk (the sort, `step_ordered`, `entries`, `OpStep`:
   ~45 % of `reload_and_record`), which waits on the walk's decision.
9. **`binary_read`** — X8 is built (1.47 → 0.84 ms); X13 is declined, so what is left is
   the push and the buffered take per value.
10. **cbor `encode` (5.57×)** — X9, on what X2 left of the row.
11. **`invert` (6.44×)** — price X10 on `insert_text` (4.84×, the row with the largest
    share of it), build.
12. **`truncate_to` (6.03×), `slope_path_with_undo` (5.97×)** — re-profile each; `truncate_to`'s
    row is now the pushes themselves (the generated loop 15 %, two claims and two frees per
    push) and § 5's record-build row names what is left of the other.
13. **`rig_world_frame3`** — S3 is built (2.59×); the row has left the cut.
14. **`forms_upto` (5.15×)** — X12.

After each item, measure the rows it names (M5) and read this order again: a row under 5×
leaves it, a step that prices under 5 % of its row is dropped for the row's next one, and a
row a step did not move as priced is re-profiled before the next step is built on it.  One
full run (`make perf-portal`) after item 6 and one after item 15.

## 1. The axis the order follows

What a routine costs is the data that leaves the cache more than the instructions it runs
(PERFORMANCE.md § How to optimise, step 3).  A temporary store is the worst case of it: every
byte written into one is claimed fresh and then copied to the store where it lives, and two
stores are two regions of memory.  So the units are split in two kinds, and the first kind
goes first:

- **stream** — removes a temporary object, a copy between two stores, or a fresh claim;
- **cycles** — the same traffic in fewer instructions (a bounds test, a store lookup, a call).

None of the rows priced so far was slow on arithmetic.  The portal reads nanoseconds only, so
a stream fault is found by profiling one row at a time; M4 below is the instrument that ranks
it directly.

## 2. Before building — what changes the list itself

| | item | why it comes first |
|---|---|---|
| M1 | the host rule: each agent reads its own host's results; a bar is met when every host's latest run agrees (bench/README.md § Which host's numbers count) | the bar is read per host — `tween/ease` is over it on arm64 macOS and far under it on the x86-64 laptop — so a row leaves this list only when no host has it over |
| M2 | the bench branches carry the libraries as they ship: `laptop-perf-bench` of `loft-libs-graphics` and `loft-libs-core` equal their `main` (@PLN185 A3a) | every library lane builds; after a library release, merge `main` into the bench branch again and measure its lanes only |
| M3 | twins follow the library as written (bench/README.md rule 1): `surface_fitted_spread`'s twin now sweeps three times as the library composes it — 6.7× was the twin doing a third of the work, the row reads 2.15× on x86-64; `check_request`'s twin already decodes twice as the library does, and loft folding that is loft's win.  pluginabi 0.1.6 retires its loft#425 and loft#651 workarounds (loft-libs-plugins#7).  Its natural `decode(bytes).value` first measured 2× slower: `(R-PureReuse)` and `(R-ReturnField)` knew only the two-statement spelling; both now take the natural one, and 0.1.6 measures level with 0.1.5 | the ratio means nothing while the lanes run different algorithms |
| M4 | the portal's bytes-moved columns from `LOFT_STORE_CENSUS` on the native lane: bytes moved within a store, between two stores, and claimed fresh, with stores touched and store switches per op | ranks every row on § 1's axis with no profile; filled by the next full run, and § 5's pricing pass reads it.  A report, never a gate |
| M5 | measure per unit only the rows that unit names (`python3 bench/stats.py --routine <bench/routine,…>`); run the full list (`make perf-portal`) after a group of units, not after each | a full run occupies the box for one unit's sake; the group run is what catches a row nobody targeted, and each price describes the program as emitted before the unit ahead of it |

### Regressions that stand

A row that reads slower than its previous measurement is re-measured alone before anything is
built on it (`make perf-trend` lists them; PERFORMANCE.md step 7).  These reproduce that way,
with the Rust lane steady, and are fixed ahead of the units.  The x86-64 figures are native
time against the row measured at `7df5a4f4b`:

| host | rows | cause |
|---|---|---|
| x86-64 laptop | `index_write` (+61 % with the lock test gone), `time/parse` +28 %, `keys_near` +16 %, `collect_dirty_inputs` +15 %, `remove_front` +13 %, `mapfile_to_painted` +13 %, `hash_text_keys`, `hash_find` and `hash_update` +10–12 % | not attributed; the lock test is ruled out for `index_write`'s remainder, `grid`, `remove_front`, `indices`, `hash_update` and `mesh_aabb` |
| arm64 macOS | `clock_pump` 4.44× → 5.95×, `boundary_loops` 1.94× → 2.40× | not attributed |
| x86-64 laptop, native time against the row at `0c4f9fd33`, Rust lanes level | `html/escape_html_blob` +43 % (2.03 → 2.90 ms, 1.58× → 2.40×), `14_stdlib_vector/comprehension` +24 %, `13_stdlib_text/join` +21 %, `13_stdlib_text/parse_num` and `15_stdlib_keyed/hash_fill` +10 % — each re-measured alone | not attributed.  Not `(R-WorkBuffer)`'s result or function-value clause: each lane's emission is byte for byte the same with both off.  `make perf-trend ARGS="--routine <name>"` gives the commits each point sits between |
| x86-64 laptop | `06_newton_sqrt` +14 % (28.2 → 32.8 ms, Rust lane level) | ATTRIBUTED: `(R-RangedCall)`'s twin `n_roots__rg` is lean enough that LLVM inlines the LOOPING callee `n_newton_sqrt` into the caller's loop; out of line (`#[inline(never)]` on the emitted Rust) the row is 28.0 ms, better than before.  The same effect as the blanket `#[inline]` experiment on this row.  A general rule — no inlining of a looping callee into a loop — changes inlining everywhere and is judged on a full run, not this row.  `mesh_emit` is not a regression (native 125.5 → 94.8 µs since 09-29, its Rust lane moved); `byte_walk` swings 3.6–6.0 µs between runs on this box |
| x86-64 laptop | random `indices` 7.61× | FIXED: the shuffle loop calls `get`, whose one inferred-range local inserts `OpRangeDefault`; its `const` bounds read as a store write, so every loop calling `get` declined the hoist.  Named store-free beside `atan2`'s dispatchers: `indices` 19.0 → 6.6 ms (2.63×), `get` 20.2 → 14.5 ms (2.57×) |
| x86-64 laptop | `mesh_aabb` +34 % (21.0 → 28.3 µs) | not in its emission or runtime (byte-identical twin since `70079f138`, `rec_get` unchanged), most likely layout — and the row is now 17.0 µs (2.23×) through `(R-RecPtr)`'s non-null address, which also took `entity_tick` −19 % |
| x86-64 laptop, with the library merge | `arguments/parse` +24 %, `stage/render_stage` +19 %, `zttext/insert_text` +9 % | the library source changed under them (arguments 0.2.4–0.2.5, stage 0.18.6–0.18.7, zttext 0.1.3): a library change or loft's is not yet told apart |
| x86-64 laptop | `gridmesh/build_index` +14 %, `cbor/encode_bytes` +16 % | FIXED: @C138's per-access alignment test in `Store::read` / `write` kept them from inlining; the check is now the compiler's (`Stores::validate_all_layouts`).  build_index 20.3 → 18.4 ms, encode_bytes 880 → 771 µs |
| x86-64 laptop | `hex_form/form_write` +31 % (2.82× → 3.69×), `hex_recover/index_build` +13 % | FIXED: the H-Elide gate took a promoted return buffer (renamed `outk`, `s`) for a caller's parameter and copied every `u = s.v` beside it; it now reads the `hidden` flag.  form_write 2.81×, index_build 3.84×.  Its alias matrix found loft#1895 (`acc = g(acc)` on a returned local answered `[]`), fixed first |
| — | a `for x in u` walk over a view `u = s.v` of a parameter | NOT a regression, a lever: the copy stays, before the H-Elide gate and after it, while an index walk over the same view elides.  The walk lowers to `Op…` calls only and binds `_vector_1 = u`; which elision condition declines is not attributed.  Unpriced |

## 3. Units with a price

Sizes are the analysis files' own (XS–M); "→" is the hand price, a ceiling for the built form.

### Stream units

| | unit | size | rows and priced landing | priced in |
|---|---|---|---|---|
| S1 | BUILT — `(R-RefillText)`: a pooled call buffer's texts refilled in their slots through the callee's refill twin (`formal/rewrites.md` § A pooled buffer's texts are refilled in their slots) | M | `msg_ping` 7.55× → 2.05× on x86-64 (the price), hash unchanged; the collection clause after it carries `flow_layout_full`, `check_request`, `decode` | under-10x.md § Status |
| S1b | BUILT — `(R-RefillText)`'s collection clause: a refilling callee keeps its vector's elements and refills their texts | M | `flow_layout_full` 14.15× → 12.35× on x86-64 (−12.7 %), hash unchanged; priced −20 %, the rest is each kept slot still taking the append | over-9x.md § zttext |
| S1c | BUILT — `(R-WorkBuffer)`'s result clause: the buffer of a call's vector result that its function only reads is the caller's, so a return buffer whose elements are kept survives its caller's activation (`token_width` per token) | M | `flow_layout_full` 7.92× → 6.23× on x86-64 (−21 %), hash unchanged | over-9x.md § zttext |
| S2 | BUILT — `(R-Apart)` instance 2: a record of fixed-length `vector<float>` fields is a `[f64; N]` per field between the functions that build it, read and write its elements and hand it on (twins `n_<f>__apv`), written into the store where it is returned to a caller that keeps it (`formal/rewrites.md` § A record of fixed-length vectors travels as a value) | L | `mat4_mul` 5.26× → 0.57× on x86-64 (62.0 → 7.74 ns a product, the hand price), every hash unchanged | apart.md |
| S3 | BUILT — `(R-Apart)` instance 3: a `vector<float>` local or work buffer that never leaves its function is an inline buffer with a heap spill (`codegen_runtime::ApVec`), on every native build | M | `rig_world_frame3` 5.33× → 2.59× on x86-64 (1540 → 747 ns a call; priced 692 without a spill test); untargeted `bone_shape_has` 2.49× → 1.40×, `mesh_emit` 2.54× → 2.01×; every hash unchanged.  `draw_bezier`'s two stacks wait on the library's slice copy | apart.md |
| S4 | BUILT — `(R-Destination)` with its chain clause: a child built in its element, a map entry's key and value in its two halves (D7).  The pooled buffer's release (D6's target) and the move (D2) went with it | M | `decode` 13.5 → 7.89 ms on one core (−41 %), hash unchanged, the hand price exactly; D5 (no one-element reserve) measured +3 % and dropped.  In its lane: `decode` 8.6× → 4.0×, `check_request` 12.0× → 9.4× (3.87 → 3.11 ms on one core, −20 %).  What is left on `decode`: the twin's body 35 %, the element append machinery ~40 % (`vector_append`, `vector_finish`, the element prefill, `record_new`, the reservation) — the push window held across the recursive call is the next step | over-9x.md, under-10x.md § cbor |
| S5 | BUILT — `(R-FnRefValue)`: a fn-ref whose every target answers a value record answers the tuple, and a record parameter every arm takes as a tuple is passed as one | M | `flow_layout_full` −34.5 % on the flow-only driver (the price −33 %), hash unchanged | over-9x.md § zttext |
| S6 | `(R-TextBorrow)`: the split-table verdict (BUILT for the walk) and the match-binding clause | XS/S + S/M | `check_request` L2; `encode_bytes`' owned match bindings; every `match`-based finder | over-9x.md |
| S7 | `(R-FoldCompare)` — BUILT for a synthesised fold; its operand clause is next | S + XS | `header` 7.4× → 3.87× with S6's split-table verdict (x86-64); the operand clause priced −14 % more | over-9x.md |
| S8 | `(R-ViewReturn)` — a lookup's found entry read in place | M | `check_request` −10 %; `pa_get` is the shape of the hex_* and markdown finders | over-9x.md |
| S9 | destination-directed builds — `(R-Place)` widened to a local with ONE owning sink, the return buffer a host | sized on `panel_build`: its locals already build in the return buffer; the release plan took −19 % (the S9 row above) | `panel_build` (most of its 55 % lifecycle share), `emit_to_material` (`tri`, `up`, `centre`; no-prefill −17 % more), `encode_bytes` (`buf += encode(x)`) | records.md § Order item 2 |

### Cycle units

| | unit | size | rows and priced landing | priced in |
|---|---|---|---|---|
| C1 | cbor's small steps: NULL frees inline (D3), no one-element reserve (D5), `(R-Base)` on the function clause (D4′) | XS, XS, S | D3 and D5 are part of S4's 6.0×; D4′ takes `decode` 6.0× → 4.8× and moves every function-clause byte read | over-9x.md |
| C2 | a push window across an admitted recursive call — one fixpoint over the callee's own write set | M | `decode` → 3.8× (ceiling 3.6×) | over-9x.md |
| C3 | field facts carried into `call_range` | S | `surface_fitted_spread` −37 % (under the bar with M3) | under-10x.md |
| C4 | a base under the return buffer's growth | M | `save_glb` → ≈ 2.7× | under-10x.md |
| C5 | `timer_spend`'s callee inlined after the parameter-address rewrite | not sized | `timer_spend` → 2.5× | under-10x.md F3 |
| C6 | `build_walls`: borrow the work buffer; admit a call writing only distinct stores | S/M, M | `build_walls` → ≈ 3.3× | under-10x.md |
| C7 | a keyed insert into a frame-minted store does not block the header hoist (`mint_target`) | not sized | `build_index` −8 % | under-10x.md F8 |
| C8 | a fixed-size scalar `f#read` without the runtime crossing; a leaner `OpReadFile` | not sized | `doc_read` ~−30 %, −15 % | under-10x.md |
| C9 | no bounds test where the index is proved in range (the write's lock test is BUILT) | a design | `10_sort` 2.77× → ~1.3× | vector-write.md |

| S11 | BUILT — an emptied text keeps its buffer: `s = ""` on a declared text local or a `&text` parameter emits `s.clear()`, not a fresh `String` | XS | `materialise` 500 → 206 µs (−59 %), `seg` 950 → 665 µs (−30 %) on arm64 macOS, hashes unchanged | this file |
| S12 | BUILT — `(R-TextBorrow)`'s format clause: a local bound to a format string borrows the format's own work text | S | `render_inline` 7.5 → 3.7–3.9 ms (7.45× → ≈ 3.8×) on arm64 macOS, hash unchanged | formal/rewrites.md § The format clause |
| S13 | BUILT, in the runtime — a text view (`keys::Str`) becomes an owned text by one allocation and one copy: `x.to_string()` in emitted code was the blanket `ToString` over `Display`, a `Formatter` and a `String` grown as it is written | XS | zttext `flow_layout_full` 6.12× → 5.57×, `time/format_iso` 3.31× → 2.88×, `markdown/html_escape` −10 %, `time/parse` −9 %; instructions −1.9 % over the whole zttext and html lanes | this file |
| S10 | BUILT — `(R-ElemFirst)`'s comprehension clause: a row built by a comprehension and appended as a whole element is built in that element | M | `grid` 58.8 → 31.3 µs on one core (−47 %, the price), hash unchanged | this file |

### Where the priced path stops

| row | lands at | what is left has |
|---|--:|---|
| `header` | 3.6–4.3× | "twin-shaped" remainder, no further lever named |
| `decode` | 3.6–3.8× | no further lever named |
| `flow_layout_full` | 6.23× on x86-64 with S1b, S5 and S1c built; the 3.6× priced earlier counted all three as still to come | X2 and X3 (§ 0, the x86-64 cut), neither priced |
| `draw_bezier` | 3.66× measured on x86-64 (graphics 0.9.8 with the keep-range clause) | under the bar only through S3 (2.3×) |
| `check_request` | ≈ 5.5×, ≈ 17× aligned | decided (C141): `(R-DecodeView)` — the copy loops are already one copy each (`(R-ByteCopy)`'s vector clause, `(R-TextRun)`, built); a slice spelled as a field value (`CBytes { value: bytes[a..b] }`) mints a store and copies twice (+94 % hand-measured), a gap of its own — ceiling with no text or byte string built: 3.95 → 2.54 ms (−36 %), formal/rewrites.md |
| `build_walls` | ≈ 3.3× | no further lever named |

## 4. Named steps with no price

Each is named in an analysis as the row's next step; none has a hand price for the row.

| row | step | named in |
|---|---|---|
| `field_add_cell` | re-analyse: the built bucket replace bought −27 % of the −42 % priced; `rehash_into` re-hashes from records; the library looks the bucket up twice | under-10x.md |
| `binary_read` | NOT the counted `while` (`while` and `for _ in 0..n` emit the same work).  The read specialised to the site's static type is BUILT (`(R-TypedRead)`, with the File record's store resolved once per side): the row is 99 % in the emitted loop, and the handle and cursor are held across it (`(R-FileHold)`, X8): 4.14×.  What is left is the push and the buffered take per value | rustc-vs-twin.md § Re-profiled |
| `insert_text`, `invert` | `(R-Compact)`'s filter, prepend and identity forms; `(R-Rebind)` for a chain exit and for the return buffer passed as a parameter | rules.md, libraries-wide.md |
| `mesh_to_floats` | BUILT (bf4cd8088): `(R-Alias)`'s versioned clause now treats the result local that adopts `__retbuf` as the buffer, and `(R-RecPtr)` reads a view's null test as a read rather than a rebind, so the copy that runs when the buffer's store is not the mesh's holds `mf_tri` and `mf_v`.  `perf_check`: 8.23× → 5.53× (−33 %); `pack_instances`, holding `n` the same way, 7.55× → 4.84× (−36 %).  Next: re-analyse both rows from the current code | records.md |
| `map_json` | PRICED: 86 % is `Map.parse` — the source lexer stepping through JSON 67 %, the `Parsed` tree written into the store 19 % — and `"{m:j}"` 6 %.  The lexer's per-token `Arc<str>` reference count (`Position.file`) was ~21 % of cycles: BUILT as `FileName` (an interned name, identity-first equality), −15 % cycles, same hash.  Next: the `Parsed` → store walk (two passes where the twin parses straight into structs) and the lexer's per-character stepping; a separate JSON byte scanner is ruled out (@PLN109: one lexer).  The parser's own allocations (string compares, key clones) priced at −0.8 % — not worth a change | profiled 2026-10-06 |
| `flow_layout_full` | levers B (S5) and A (S1c) are built; X2 and X3 are next (§ 0).  The bench itself works around fn-ref records not freed until a plain-value frame ends (`zttext/bench/bench.loft` `flow_op`) — a leak to file or fix | over-9x.md |
| `truncate_to` | NOT the move through a by-value parameter: hand-priced (`e` placed in the timeline's store, `history_push`'s copy a shallow move) it costs +16 %, because the pooled `e` already keeps its vectors across pushes and the placed one claims them anew.  `drop_oldest` is already in place (`keep_vector_range`).  The row is claim-for-claim the twin's malloc-for-malloc (two each per push); what differs is each one's price: `OpCopyRecord` 36 % and the release walk 27 % go through `copy_claims` / `owned_walk` / `remove_struct_claims` for a `Stroke` whose layout the emitter knows.  Next: a copy and a release specialised to the static type (unpriced).  Profiled alone: `Store::claim` is 18–20 %, and it is `claim_scan` — the timeline's store stays in `@FR-H-LazyFree`'s phase for good (each push frees what the next claims, so the untracked words never reach the bound), the freed vectors merge with their neighbours (8 + 13 → 21 words), and every miss walks the chain from the store's first block (480 walks per call).  Measured and dropped: ending the phase once the walks cost a sweep (+3 %: the free tree then costs what the walks did) and exact-size lists of lazily freed blocks (+5 %: 55 hits in 1866 claims, the merged sizes never match).  The walk itself is gone: `(H-ScanIndex)` reads the first fit off the lazily freed blocks' index (8.58× → 8.17× on x86-64).  X1 is built for it (§ 0): 8.17× → 6.03×.  What is left is spread: the generated loop 15 %, the copy 11 %, the claims and frees themselves ~25 %, vector appends ~19 % | records.md § Order item 4 |
| `panel_build` | the "bound null" window of `(R-RecPtr)`; a heap-owning tree's stores reused across a rebind (a rule that does not exist) | rules.md, records.md § Order item 5 |

## 5. Rows with no price — the pricing pass

For each: profile the row alone (`--names`, PERFORMANCE.md § Attributing a bench ROW), name
the work the twin does not do, hand-price its removal (`bench/portal/hand_price.sh`).  Work a
CLASS, not a row; the visible reason is a hypothesis until priced.

| class | rows | where to start |
|---|---|---|
| keyed (10) | `reload_and_record`, `index_fill_find`, `anim_of`, `index_build`, `composite_hash`, `hash_find`, `collect_dirty_inputs`, `hash_text_keys`, `word_count`, `blob_put` | six sit within 16 % of the bar: price keyed.md § What is left on them (the typed append, the arena's insert bookkeeping, the rebuild's re-hash, text keys copied into the record).  Both format decisions are made and built: `index` is a B+tree (@C147), a hash's bucket count a power of two (@C146).  @PLN185 D2 owes this class a named root mechanism.  `reload_and_record` 10.88× → 7.06× and `mapfile_to_painted` −14 % through a compound key's decorated walk sort (keyed.md L8b); its remainder is unpriced |
| text-build (5) | `map_json`, `render_inline`, `materialise`, `format_iso`, `char_roundtrip` | the per-character append (`out += "{c}"`) is the class-wide hypothesis (libraries-wide.md), and `slugify` is the same shape; `map_json` is `Map.parse` running the source lexer over JSON |
| call (5) | `clock_pump`, `forms_upto`, `combine_cut`, `edges_cut`, `resolve_move` | @PLN185 D2: one hand-priced edit naming the mechanism; `resolve_move` is five hash lookups a call.  `forms_upto` PRICED: the caller releases its pooled `Form` buffer before every `form_new` call (`remove_claims`), and `form_new` is a `(R-RefillBuffer)` callee that empties each vector field in place — the release frees the two vectors the refill would keep, and `vector_add` claims two new ones.  BUILT as `(R-RefillText)`'s vector clause: 12.96× → 6.62× (2.00 → 1.02 ms), hash unchanged |
| float-kernel (5) | `ease`, `terrain_fbm`, `terrain_surface_at`, `roof_match`, `roof_cone` | @PLN185 D2; `ease` is over the bar on arm64 macOS only — it counts as over until both agree |
| record-field (5) | `fill_polygon`, `stencil_rotate`, `locate`, `terrain_relief_pass`, cbor `encode` | a class with a median over the bar; `terrain_relief_pass` reads a type through a nullable record per cell |
| text-scan (4) | `arguments/parse`, `mapfile_to_painted`, `seg`, `time/parse` | — |
| record-build (3) | `slope_path_with_undo`, `sphere`, `pluginabi/request` | `slope_path_with_undo` PRICED and BUILT: each copy of the seven-field `Hex` tuple into or out of a store record resolved the store per field (`(R-RecPtr)`'s tuple clause, −32 %), and `s.us_redo = []` searched the record's fields at run time for a no-heap element type (−7 %): 440 → 267 µs, same hash.  What is left: the chunk walks the twin shares (~37 %), `length_vector` / `get_vector` out of line for `k.items[idx]` in a matched chunk (~23 %, both already `#[inline]`), the undo entry's append (~10 %) |
| vector-write (2) | `canvas`, `fill_rect` | the lock-test fix moved `fill_rect` 9 % |
| alloc-temp (3) | `draft_fit_p`, `catalog_churn`, `slugify` | `draft_fit_p` hands its temporary to callees in another library (apart.md triage) |
| vector-build (2) | `emit_segment`, `field_union` | `emit_segment` PRICED and BUILT: its nine appends sat behind a call per segment, and each re-resolved the record's field and its vector; written in the loop they took push windows (probe 15 → 7 ms hand-priced).  Built as `(R-InlineLeaf)`'s appender clause: 17.5 → 7.2 ms, 8.25× → 3.40× on arm64 macOS, rewrites in the lane `R-Push` 6 → 15 and `R-Header` 17 → 26.  What is left on it is unpriced; `field_union` too |
| vector-read (2) | `draw_quads`, `wall_chain_walk` | — |
| int-loop (2), parallel (1), native-boundary (1) | `clock_advance`, `fill_triangle`; `par_text`; `rand_indices` | all but `rand_indices` within 4 % of the bar: re-read after M1 |

## 6. Decisions the work waits on

- **`(R-ViewReturn)`** (S8) is an ownership decision: a result that is a view of the caller's
  own argument, consumed inside the binding statement.
- **`(R-Destination)`** (S4): DECIDED — an element minted and not finished is no member, and
  the CALLER releases what the failed child placed (the release walk the plain form's
  discarded result runs).  Identical in values, members and live records; spare capacity is
  not part of the contract (owner).
- **A decoded value whose texts and byte strings are views into the input frame**: DECIDED
  (C139) — a view with per-character operations on it is allowed wherever the results are the
  owned form's; the compiler proves the buffer outlives every read and is not written meanwhile.
  The only form the analysis names that reaches even 6× on `check_request` against an aligned
  twin.
- **A move through a by-value parameter**: it changes what a callee may assume of its
  parameter's store after an append.
- **Relational range facts** (`index < len(v)`) in `generation::range` (C9's second half).
- **A checked expression computed once where the program spells it twice**: DECIDED (owner,
  2026-10-11) — an overflow must be reported, not reported as often as it was spelled.
  `(R-SameRead)`'s guard clause (`locate`) rests on it.
- **`map_json`'s parse** (X6, § 0): DECIDED and built — `T.parse` needs no intermediate
  format: it writes the store as it reads (`Stores::read_into`).  The one lexer stays the
  reader (@PLN109) and is made cheaper rather than joined by a second.  What a program can
  observe stays: the fields, the defaults of absent ones, the last of a repeated key, and
  `x#errors` (@C148).  A text that is not well-formed answers the type's defaults however
  much of it was read: its half-written store is released (`Stores::record_from_text`).  A
  value under an enum, and a value whose shape its type does not have, is still read as a
  `Parsed` and written by the walk that says what each type accepts.
- **Keyed store format**: DECIDED and built — a hash's bucket count is a power of two (@C146);
  `index` is a B+tree of key parts and record numbers (@C147).  `sorted`'s random insert stays
  as it is (declined).

## 7. Order

The release cut's order (§ 0) comes first; this is the order toward 3× after it.

1. M1–M3 and the regressions that stand: they change which rows are on the list.
2. S2 and S3, then `(R-RefillText)`'s collection clause: the units that take a row under
   the bar by removing a temporary (S1, the text clause, is built).
3. M4 and one full run (the first since step 1), then § 5's pricing pass over the classes
   with a median over the bar (keyed, record-field, alloc-temp), read on both axes.
4. S4 with C1 (the XS steps first), then C2; S6 and S7; S5; S8 and S9 after their decisions.
5. The remaining cycle units, each on the row that carries it.

Each unit lands with a guard of hand-computed cells on both backends, its switch A/B,
`make rewrite-census` and a timing of the rows it names (M5; PERFORMANCE.md steps 6–7).

## Tooling the pass keeps tripping over

- A row's TIME moves up to 10 % between two builds whose code for that row is the same: a
  runtime rebuilt with one function changed shifted `mesh_emit`, which converts no text, by
  +9.9 %, and the same emitted source linked by `hand_price.sh` and by `loft` ran `mat4_mul`
  18 % apart.  Under 10 %, compare INSTRUCTIONS (`perf stat -e instructions:u`, the
  `cpu_core` line), which do not depend on where the code landed, and A/B a runtime change
  on one emitted source against both runtimes (a second worktree holds the old one).
- A rewrite the census counts inside a LIBRARY function is not in that library's bench run:
  the bench loads the library's prebuilt native code, and the census emits its loft source
  (`LOFT_NO_NATIVE_LIBS=1`).  To check such a site, run the bench — or the library's own
  `loft test --native` — under `LOFT_NO_NATIVE_LIBS=1`, and name the function that holds it
  from an emission made the same way; an on/off comparison of a run that never reaches the
  site agrees for no reason.
- Two binaries are never timed side by side: `paste <(a) <(b)` runs both at once on the
  pinned core, and read a row +15 % slower whose code was unchanged.  Run one, then the other.

- `bench/portal/hand_price.sh` links a package's native libraries by replaying the
  arguments of the package's own compile (`LOFT_NATIVE_RUSTC_ARGS`; the script's header has
  the commands).  Needed by every bench whose library ships a cdylib (`server`, `pluginabi`
  through crypto, `web`, `ssh`).
- No script attributes a profile's samples to one routine of a lane (rules.md § Change 6);
  callgrind does, by instructions: `valgrind --tool=callgrind --toggle-collect='*<symbol>'
  <bin> --n 1` counts only what runs under that routine's function, so the function list of
  the run IS the routine's profile.  Toggle on the routine's own symbol (`nm -C <bin>`): the
  bench's `bench_*` drivers are usually inlined into `main` and collect nothing.
- A profile of the RUNTIME by line needs an rlib with line tables:
  `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only cargo build --release --lib --target-dir
  <dir outside /tmp>`, then the package's rustc arguments file with `--extern loft=`,
  `dependency=`, `loft_ffi=` and the `ring` path pointed at that directory, and
  `hand_price.sh <rs> <bin> <args>` — the same code (equal instruction counts), now
  attributed to `store.rs` lines.  A RUNTIME change is priced the same way: one emitted
  source compiled against a saved copy of the tip's `libloft.rlib` and against the probe's.
  On Linux the method is `perf` samples windowed by time (rustc-vs-twin.md § Method notes).
  On macOS, `sample <pid>` over a `--native-release --names` build gives the call tree;
  a routine's share is its `n_bench_<row>` (or op helper) subtree.  `--names` disables
  inlining, so a small helper's share is inflated.
- `make rewrite-census` read 1–3 low on `R-ExitVector` and `R-Header` on the aarch64 box with
  the libraries at the baseline's commits (records.md); unexplained.  The same box reads
  `R-RefillBuffer` 1 → 0 in `web` and `server` with the checkout at the baseline's commit — and
  so does the loft commit that blessed the baseline (`9c69e6d8e`, built here), so it is the
  box, not a change.  The baseline is one host's; the census has no per-host row.
