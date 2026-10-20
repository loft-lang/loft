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
| `check_request` | 5.04 | DONE: `(R-DecodeView)`'s structure clause, instance 1 BUILT (a value read once held as its position) | **0.94×** (measured) |
| `flow_layout_full` | 7.00 | S1b and S1c are built (S1c after this host's run: −21 % on x86-64); a fused `character` element read | re-measure, then re-price |
| `invert` | 5.82 | `(R-Compact)`'s prepend form, `(R-Rebind)` for a chain exit | unpriced |
| `sphere` | 5.72 | none named (vector growth, the two `add_triangle` writes); 5.97 → 5.72 with the deep window | unpriced |
| `decode` | 2.62 | DONE: `(R-FieldAppend)`'s bare clause, then `(R-PushRec)`'s apart-call clause (the decoder loops' push header held across the child's call); the held window priced at a further −8 % | off the cut |

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

### The x86 rows over 4×, toward 3× (measured at `27ea270e8`)

`scripts/perf_check.py --routine …` on this laptop, each row interleaved with its Rust twin,
hashes equal.  Nine rows stand over 4×; `fill_polygon` 3.81×, `draft_fit_p` 3.31×, `sphere`
2.45×, `combine_cut` 1.98× and `timer_spend` 1.94× have left it since `7d30105c8`.  Each
profile is callgrind over one routine: the routine's own driver toggled (lane 17), or the
library function counted at `--n 2` and `--n 6` and differenced so the setup cancels (the
packages, whose drivers are inlined).  A share is of loft's instructions in the routine.
"Lands at" multiplies the row by each lever's saving; *est.* marks a saving read off a
profile share and never hand-priced (such a figure has paid about two thirds, § 0).

| × | row | where loft's excess goes | levers, in order | lands at |
|--:|---|---|---|---|
| 4.75 | `flow_layout_full` | `slice_runs` 59 % inclusive (`buf_slice_text` 16 %): each run a vector, a `Run` record and a 1–2 character text the caller walks once and then drops | F1 (−10.6 %, priced) and F2 (−32.5 % on top, priced) below | **≈ 2.9×** |
| 4.67 | cbor `encode` | store creation 40 % (`OpDatabase`: 13 stores an encode, one per local vector of each `encode_map` call, ~3.3 k instructions each with the claim, the prefill and the free); `head__ap` 21 % | E1 (−15.5 %, priced); L2, the local vectors' stores pooled per frame (−30 % *est.*) | ≈ 2.8× (*est.*) |
| 4.48 | `slope_path_with_undo` | `map_set_hex` 35 % and `map_get_hex` 28 % (the chunk walk set up per call, three field compares per chunk, `rec_and_len` out of line, 8.5 %); the undo vector's growth 10 % with the store's `resize` 6 % | L7, the chunk vector's header and base handed into the callees as an `__inv` twin from the caller's loop (−30 % *est.*); the undo vector reserved once (−5 % *est.*) | ≈ 3.0× (*est.*) |
| 4.41 | `pluginabi/request` | base64 59 %: each payload decoded TWICE (`pa_is_base64` decodes to validate, `pa_cbytes` decodes to use; the twin decodes once) by a decoder that makes three passes (a stripped copy, a value vector, the output; the twin's two); the decoded bytes deep-copied into their entry 27 % (`pa_te(k, pa_cbytes(…))`: `OpCopyRecord` of a fresh value); the encoder 15 % | L5, crypto's decoder in one pass (a library native, −20 % *est.*) and the validation answered without decoding twice (−25 % *est.*); L3, a fresh argument built in the record that takes it (−20 % *est.*) | ≈ 2.0× (*est.*) |
| 4.39 | `truncate_to` | `history_push` 67 %: the entry deep-copied into the timeline's store (`OpCopyRecord` 28 %, `copy_plain_vectors` 18 %) and the dropped entry's release (`release_plain_vectors` 13 %, `remove_claims` 14 %); `keep_vector_range` 20 % (the truncated tail's release, the twin's `Vec::truncate` drops the same) | L3, the entry built in the store that keeps it (`(R-Place)`'s foreign-store clause: no copy, no release of a temporary; a move across stores is ruled out by @C125) (−40 % *est.*) | ≈ 2.6× (*est.*) |
| 4.31 | `field_add_cell` | the keyed get-or-insert 56 % (`entry_ints` 48 %: `add_at` 15 %, `grow_if_full` + `rehash_into` 22 %, `ints_hash` 11 %); the lookup `OpGetHashLong` 14 %.  The twin is std's `HashMap` (SipHash) and grows too, so the excess is per insert, not the hash | L6, the keyed insert path line by line (unpriced: next to profile) | needs pricing |
| 4.27 | `panel_build` | the release of last frame's panel 26 % (`release_by_plan`, `remove_claims`), claims 11 %, the prefill 11 % (`set_default_value`), number formatting ~20 % (`format_long` and `format_float` through `core::fmt`) | L4, the panel tree refilled in place of release-and-claim (−25 % *est.*); L1, numbers formatted without `core::fmt` (−12 % *est.*) | ≈ 2.8× (*est.*) |
| 4.23 | cbor `encode_bytes` | `encode__ap` 78 %: `head__ap` 35 % (one to nine bytes, each run through `push_header` 9 % and `reserve_more` 11 %), `append_bytes` 17 %.  The twin reads 76–96 µs between runs, so this row is noisy | L8, the head written through the caller's held window with the payload reserved with it (−25 % *est.*); X9 −3 % (priced) | ≈ 3.1× (*est.*) |
| 4.14 | `forms_upto` | spread over the library: `form_admissible` 25 %, `form_closes` 21 %, `form_canon` 15 %, `form_write` 8 %; the key text built by formatting integers ~12 % (`format_long`, `core::fmt`), stores ~8 %, `malloc`/`free` ~6 % | L1 (−10 % *est.*); L2 (−6 % *est.*); the helpers line by line (unpriced) | ≈ 3.5× (*est.*) |

**Built since** (A/B on this laptop, each side on its own rlib): **L1** — numbers formatted
in place (`ops/number_text.rs`): `panel_build` 4.23× → 3.72×, `forms_upto` 4.10× → 3.80×.
**F2** — `(R-StreamWalk)`, the walk twin as a general rule: `flow_layout_full` 4.60× → **2.98×**
(the height loop's twin reads only the style, so F1 comes with it).  A text set reads its
value in place (`Store::holds_bytes`): −1.8 % of `panel_build`'s instructions, no wall move.
**`(R-CompleteWrite)` buffer clauses** — a record built into the caller's slot skips the
prefill on both sides of the call: `panel_build` 3.67× → 3.44×.
**Standing at `666268397`** (`bench/stats.py`, this laptop): `flow_layout_full` 2.98×,
`panel_build` 3.44×, `forms_upto` 3.83×, cbor `encode_bytes` 3.92×, `truncate_to` 4.16×,
`pluginabi/request` 4.42×, `field_add_cell` 4.44×, `slope_path_with_undo` 4.53×, cbor `encode`
4.66×.  Store creation is ~950 instructions (12 % of `encode`) spread over slot bookkeeping,
`Store::init` and a fresh store's first claim through the tree path; no single fold moves it
more than ~2 %.
#### The five rows over 4× at `4cc416af4`, analysed (2026-10-20)

Callgrind per op against the twin, compiled from the release emission with line tables; a
price is an edit of the emitted Rust or of the library source, timed interleaved on one core
(`--n` 200–300, uncached), hash unchanged.

| × | row | loft / twin | where the excess goes | priced | lands at |
|--:|---|---|---|---|---|
| 4.53 | `slope_path_with_undo` | 2275 / 415 instr per step | `map_get_hex` 337 and `map_set_hex` 408 instr per call (twin ~100): the chunk walk rebuilds a `DbRef` per pass, each field read is a null-and-bounds `rec_get`, the nested `ck_hexes` is resolved twice (`length_vector`, then `get_vector`), the `Hex` read and written field by field through the store; the undo vector's growth (`resize`, `resize_store`, `memset`) ~11 % | both helpers in the tight form (held base, raw field compares, one `rec_and_len`, one element address) **−27.5 %** | **≈ 3.3×**; the undo vector reserved without the zero fill *est.* −8 % → ≈ 3.0× |
| 4.42 | `pluginabi/request` | 151 k / 31 k instr per request | base64 decoded TWICE per payload (`pa_is_base64` then `pa_cbytes`, 31 % each pair); the decoded bytes copied ~3 times into the entry (`copy_claims` 40 k, `OpCopyRecord` 13 k); the encoder | one decode per payload, validated by its result (library form) **−31 %** | **≈ 3.05×**; R2, the bytes built where the entry keeps them, *est.* −15 % → ≈ 2.6× |
| 4.66 | cbor `encode` | 113 k / 26.5 k instr per encode | the rank loop's `key_lt` (per byte a checked `kb[i] ?? 0`, ~112 instr a compare, ~40 k); a key's bytes appended through `vector_slice` (~450 instr for 1–8 bytes: the headers resolved four times) and `vector_add` (~650); 14 stores (~14 k with their release) | E1 (earlier, library form) −15.5 % | E1 + `vector_slice`/`vector_add` resolving each header once *est.* −12 % → ≈ 3.5×; 3× needs the rank loop's reads held (`key_lt` is called with the buffer as a parameter: a twin clause) |
| 4.16 | `truncate_to` | 3.6 k / 755 instr per push | the entry deep-copied into the timeline (~1035: two claims ~143 each, `copy_plain_vectors` ~372 of its own); the dropped entry released (~730: two deletes ~165 each, `release_plain_vectors` ~234); the entry's own build ~1130 (a push header and a reservation per push) | L3, the entry built in the timeline's element: **+30 % — REFUTED** (its vectors then grow element by element in the large, fragmented store; the reused loop record copies once at the exact size) | runtime tightening of the copy, the release and claim/delete *est.* −25 % → ≈ 3.2×; no structural lever found |
| 4.44 | `field_add_cell` | 545 M / 123 M instr | keyed operations 55 %: ~440 instr each against hashbrown's ~200–300 — per call the entries header, the seed, the record words and the sub-record type re-read (~60), SipHash 52, the arena claim 52; rehash 54 M against 22 M (191 per moved entry); `mark_borders`' dirty inserts | the second bucket lookup removed (library form) −0.9 % — DROPPED | the keyed header held across a loop or call *est.* −8 %, a cheaper rehash *est.* −6 % → ≈ 3.9×; needs a line-level pass of the keyed runtime |

**BUILT (2026-10-20)** — `pluginabi/request`'s decode once, as `(R-PureReuse)`'s SHARED-CALL
clause (`formal/rewrites.md`): a total call two wrappers make is made once by the caller and
handed to each wrapper's variant.  Native `request` 6.88 → 4.77 ms (−30.6 %; the hand-written
form 4.72), hash 16387f10, interpreter −25 %; `check_request` unchanged.  It fires once crypto
declares its decoder `#pure` (the lane's crypto 0.3.11 does not): a crypto release with the
annotation and pluginabi's lock moved to it are what put it in the lane.  Found beside it and
fixed: a decode root calling another root did not compile on `--native-release`
(`LOFT_NO_PURE_REUSE=1` broke the bench), loft#2091.

**`slope_path_with_undo`, the lookups, RE-PRICED and part BUILT (2026-10-20).**  The −27.5 % above
was a hand form that also replaced `chunk_idx_32` / `hex_idx_32`'s checked arithmetic with
plain `div_euclid` / `rem_euclid` — a change `@C120` rules out.  With the arithmetic kept, the
two helpers price at −15.2 % (200.9 → 170.3 µs), split as: the WRITE `sh_c.ck_hexes[idx] = h`
−7.1 % and the READ `return gh_c.ck_hexes[idx]?` −5.8 %.  Not the scan: its compares read
through `rec_get` on the held address, and raw loads there measured slower.  Not the `?`
discharge (inert).  Not a missed inline (`#[inline(always)]` on `vec_get_or_raise_runtime`,
`get_vector`, `rec_ptr`: each inert).  The write's cost is the element's reference coming back
from `vec_get_or_raise_runtime` and its address read back out of it (time, not instructions:
−25 M instructions over the program against the read's −339 M).
BUILT: `(R-RecPtr)`'s ELEMENT clause (`LOFT_NO_ELEMENT_ADDRESS`) — the element derived where the
value record is written, one header read, its reference and address from the record and the
index: 4.52× → 4.26× in the lane (−5.7 %, hash 3f61de).  PRICED, NOT BUILT: the read's two
header reads shared (−4.8 %, the guard's `len` and the arm's element read): an `if` statement
clause on the loop gate fired at 6 other sites and moved nothing, and here the gate declines
the arm (its `?` fallback mints and frees a temporary), so it needs a write analysis that
proves the arm's writes land only in fresh temporaries.  Dropped.  What is left is the chunk
walk the twin shares and the checked arithmetic.

The common thread of the last three is the runtime's unit cost: a claim, a delete, a vector
header resolution and a keyed probe each cost several times their `malloc` / `Vec` /
hashbrown counterpart, and these rows are made of them.

**PRICED and DROPPED** — `@FR-R-Apart` instance 3 for full-width `vector<integer>` locals:
cbor `encode` 4.66× → 4.26×, but drawing's image routines 2–3.5× slower (`render_marks` +255 %,
`resize` +220 %): their whole-image integer buffers lost the store's reservation, held base
and fill, and the IR cannot tell a 20-element table from a pixel buffer.  Kept from it: the
inline slots are no longer zero-filled per call.

The levers, by reach — each a compiler or runtime rule, never a library rewrite for speed
(`formal/performance.md` `(Perf-Cure)`):

- **F1 + F2 — `(R-DeadResult)` per field, and a vector a callee builds and its caller walks once
  streamed.**  F2 is the structure clause's WALK TWIN (`decode_structure_emit`, built) moved
  off decoders: the callee's loop runs with the caller's loop body as its closure at each
  append, so neither the vector nor its records are made.  F1 is the shape twin's dropped
  statements for one unread field.  `flow_layout_full`, and every builder-then-walker pair.
- **L1 — numbers formatted without `core::fmt`.**  `format_long` / `format_float` go through
  `fmt::write` and `pad_integral`; a direct integer writer and a shortest-float writer in the
  runtime.  `panel_build`, `forms_upto`, every format of a number.  Runtime only, both backends.
- **L2 — a function's local vectors pooled per frame.**  `(R-WorkBuffer)`'s proposed clause
  for vectors: a local vector's store claimed once per frame and cleared, not created and freed
  per call (~3.3 k instructions).  cbor `encode`, `forms_upto`, `request`'s encoder.
- **L3 — a fresh value built where it is kept.**  A call result handed as an argument the
  callee stores into a record (`pa_te(k, pa_cbytes(…))`), or a record pushed into another
  store (`history_push`), built in the destination instead of built, deep-copied and released.
  `request`, `truncate_to`.
- **L4 — a record tree rebuilt every frame refilled in place.**  `(R-RefillBuffer)` for a tree:
  the previous frame's records reused field by field.  `panel_build`.
- **L5 — crypto's base64 decoder in one pass, and its validation without a second decode.**  A
  library native; the validation half needs a call shared across natives or a validating
  native — a decision (@C entry) before it is built.  `request`.
- **L6 — the keyed insert path.**  Unpriced; `field_add_cell` is the row to profile line by line.
- **L7 — a callee's chunk walk set up once per caller loop.**  `(R-Inputs)` for the chunk
  vector `map_get_hex` / `map_set_hex` walk.  `slope_path_with_undo`.
- **L8 — a small append run through the caller's window.**  `(R-PushFill)`'s run clause across a
  call.  `encode_bytes`, cbor `encode`.

Order by reach and price: F1 + F2 (priced, one row under 3× alone, the machinery built), L3
(two rows), L1 and L2 (three rows each, both general), then L5, L4, L7, L8; L6 first needs its
profile.

### The laptop rows at 4.5× and over, toward 3× (measured at `2cf6c15ca`)

`bench/stats.py`, calibrated: `check_request` 5.05×, `flow_layout_full` 4.83×, cbor `encode`
4.69×; `pluginabi/request` 4.42× shares the decoder and the encoder.  `stage/pack_instances`
measures 2.08× now; its committed 4.97× predates the `mesh_to_floats` fix.  The profiles are
callgrind runs of single-row copies of each bench, loft's `--native-release` emission against
its twin, with the hashes equal.  A price is cycles per op, three interleaved pinned rounds,
of a LIBRARY-level form of the change in a scratch copy of the package.  It is the ceiling for
the compiler rule that would make the natural spelling do the same.

| row | loft / twin per op | where loft's excess goes | priced | after |
|---|---|---|---|---|
| `flow_layout_full` | 154 M / 22.5 M instr | `slice_runs` 59 % inclusive: 64 k calls per layout, each a vector, a `Run` record and a 1–2 character text, where the twin pushes a `&[char]` slice | F1 −10.6 %, F1+F2 −39.7 % | **≈ 2.9×** |
| cbor `encode` | 113.5 M / 26.3 M instr | each of 20 keys per encode built in a vector of its own and copied into `keybuf` (`vector_add`); 13 stores created per encode, one per local vector of each `encode_map` call; the rank loop | E1 −15.5 % | ≈ 3.96× |
| `check_request` | 11.6 k / 2.3 k instr per check | one decode per check (loft already shares it between the two questions, the twin decodes twice); per check: leaf reads 2.0 k, four span texts 1.4 k, three entry records 1.0 k, the decode's store 1.0 k, frees 0.8 k, record placement 0.6 k | the structure clause, BUILT: 1.58 → 0.32 ms | **0.94×** |

- **F1 — an unread field is not built.**  `place_line`'s height loop asks `slice_runs` for runs
  and reads only `.style`; every run's text is built and dropped.  The rule: a callee whose
  result field no caller reads is specialised without that field's construction, as the
  `__dv` / `__rg` variants already specialise by caller demand.  −10.6 % cycles (−18 % instr).
- **F2 — a vector a callee builds and the caller walks once is streamed.**  `token_width` and
  `place_line`'s box loop walk `slice_runs`' result once, straight away.  Inlined and fused, no
  vector or `Run` record is made, and the run's text goes straight to `measure`.  −32.5 %
  cycles on top of F1.  The text itself stays: `measure` is `fn(text, Style)`.  X3 (run views)
  would remove it too, but 3× does not need it.
- **E1 — a bound call result appended whole is appended in place.**  `k = encode(key);
  koff += [len(keybuf)]; klen += [len(k)]; keybuf += k` → the key encoded by `encode__ap`
  straight into `keybuf`, its length taken as the growth.  −15.5 % cycles (−14.3 % instr).
  Next on the row, unpriced: the 13 stores per encode (~10 % by the profile: creating and
  freeing a store is ~600–800 instructions), then the rank and emit loops (key-pair compare
  ~90 instructions against the twin's ~36; the slice compare was priced and dropped).  E1 and
  the stores reach ≈ 3.5×.  3× needs a third lever not yet found.
- **`check_request` — DONE at 0.94× (0.32 ms against the twin's 0.34 ms): `(R-DecodeView)`'s
  structure clause, instance 1, holds the decoded value as its position and builds nothing of
  it (formal/rewrites.md).  The analysis that led there:** it was 5.04×, 1.70 ms, and its cost
  was the decoded TREE.  Its
  two questions already share one decode (`(R-PureReuse)`), and that decode's `value` is read,
  so neither `(R-SharedCall)` nor `(R-DeadResult)` moves the row alone.  What is left is
  building a map, its entries and an enum value per key and value to read one key — which
  only `(R-DecodeView)`'s structure clause removes: DECIDED (C141, read-once only), PROPOSED in
  formal/rewrites.md, its validation step `(R-DeadResult)`'s `{ok, next}` twin.  PRICED as
  library forms of the rules' output (`bench/stats.py`, one core, hash unchanged): the shared
  decode by hand 1.72 ms (5.15×, nothing); a validation walk then the decode 1.61 ms (4.82×,
  `(R-DeadResult)` alone); a validation walk then a key search in the bytes 0.60 ms (1.79×,
  with the structure clause) — the clause is the whole cut.  Its size is the obstacle: it
  derives a skip twin, a tag reader and an in-order entry walk from `read_value`'s eager
  recursion, not a local rewrite.  Cheaper and built since: `(R-FieldAppend)`'s bare clause
  appends each decoded child through `vector_append` directly (cbor `decode` −20 %, the same
  decoder `check_request` runs; the switch A/B moves the row 5.00× → 4.81×).  BUILT since:
  `(R-PushRec)`'s apart-call clause holds the decoder loops' push header across the child's
  call (cbor `decode` 3.42× → 2.62×, `check_request` 4.74× → 4.60×).  PRICED, NOT BUILT: the
  held window on top of it — the length in a local, written at the loop's end, growth by
  doubling — `decode` 5.15 → 4.73 ms by hand, hash equal (formal/rewrites.md's open list).  PRICED and DROPPED: lending the decode's store across calls (the caller-lent
  buffer, cleared before each call) — +29 % instructions, +25 % cycles: clearing the previous
  tree claim by claim and claiming again from a reused store costs more than a fresh store's
  bump claims and one whole-store free.  The same holds for `encode`'s 13 stores per call.
- **Library rewrites are not the cure here** (`formal/performance.md` `(Perf-Teach)`): E1, F1
  and F2 were priced as library forms to measure their ceilings; they reach the rows as
  compiler rules, or not at all.
- **`15_stdlib_keyed/hash_text_keys` 4.46× → 3.17×, BUILT** — `(R-TypedKeyed)`'s and
  `(R-KeyedEntry)`'s text clauses: a lookup in a hash keyed by one `text` hashes the text it
  is handed (`OpGetHashText`, −4 % of the row), and an insert asks the key first and writes
  the entry it answers (`keyed_entry_text`, −21 %).  `word_count` 3.09× → 2.68× with it.
- **`hex_edge/edges_cut` 4.42× → 1.76×, BUILT** — a narrow-integer vector element write no
  longer declines the loop's scalar hoists, so `hexset_get` is called as its `__inv` twin with
  the window held; `combine_cut` 4.13× → ~2.2× with it.  Loop-free twins are forced inline.
- **`16_consumer_shapes/composite_hash` 4.18× → 3.27×, BUILT** — the compound clauses of the
  typed lookup and the one-probe insert (two to four integer key parts).


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
| 4.97 | `check_request` (7.55 before `(R-DecodeView)`) | DONE: `(R-DecodeView)`'s structure clause builds nothing of the decoded tree (§ 0's bullet) | 0.94× |
| 4.42 | `panel_build` (7.44 before the release plan, 5.24 before the record enum's literal exits) | BUILT: `(R-Place)`'s callee clause for a record enum — `panel_hit_test`'s four variant exits build into the caller's buffer (−5.0 %); `(R-Place)`'s foreign-exit clause — the palette built in its field though two exits answer a constant (−11.3 %, the hand price).  What is left: the release of last frame's tree (~19 %), the claims (~25 %), the status format (~15 %) | out of the cut |
| 7.34 | `arguments/parse` | X7 | the cursor match BUILT — `(R-SameRead)`'s match clause: the subject's length and each integer computation of one element two arm tests make, bound once before the first test (`same_read::bind_match_reads`, `LOFT_NO_MATCH_READ`).  Then counted by callgrind (per parse, loft ~200 k instructions, the twin ~23.7 k; IPC 4.4 against 3.7, so the gap is work, not stalls): `find_long_opt` 51 k (25 %: each `fo.long_name` resolved through the store, the record and the text — a TEXT FIELD of a vector's record element, which `(R-Base)`'s text clause does not serve — and `len(name) > 0` counted by `char_count`, 6 k); `classify`'s own 56 k (the tokens' texts claimed and copied where the twin borrows, `opt_kind`'s scans); `cur = Cur { src: toks, pos: 0 }` 18 k (9 %) — the token vector DEEP-COPIED into the cursor record, though `toks` is dead after it: `(R-MoveLast)` relocates only within one store and `classify`'s result lives in its own; `parse`'s loops over `results` 31 k (a checked element read and a length per iteration of the reset `while`, and of the required-option walk); each matched text copied twice into its result (a `String`, then `set_str`: ~97 `malloc`/`free` pairs, 7 k) | `arguments/parse` | the match clause measured: 25.23 → 24.01 ms native (−4.9 %, 86 % of its −5.7 % hand price), interpreted −0.8 %, hash equal; the dead token vector moved into the cursor record PRICED: −9.1 % (24.14 → 21.94 ms, six interleaved rounds, hash equal) with `Cur`'s record claimed in the token vector's store, its `src` slot taking the vector's handle (no element copied) and `Cur`'s own free dropped (its store is the tokens', freed once); then BUILT as `(R-MoveLast)`'s host clause (`literal_host::rewrite`, `LOFT_NO_LITERAL_HOST`): 24.04 → 22.08 ms native (−8.2 %, about 90 % of the hand price), interpreted −0.9 %, hash equal; `find_long_opt`'s text field PRICED (five interleaved rounds, hash equal): `fo.long_name` read through the held base and the store's text span — the field's record number one load at `base + i × 20 + 4`, the text sliced off the span (`vector::text_at`) instead of resolving the store, the record and the text per element — 22.09 → 19.61 ms (−11.2 %); `len(fo2.long_name) > 0` as a byte test (not null, not empty) instead of `t_4text_len`'s character count, a further −6.3 % (→ 18.32 ms); reading the field once where the prefix loop reads it twice, nothing more; then BUILT: `(R-RecPtr)`'s text clause (`vector::rec_text`, `LOFT_NO_TEXT_FIELD`, native) 22.09 → 18.64 ms alone (−15.6 %), `(R-ZeroCount)` (`src/zero_count.rs`, `LOFT_NO_ZERO_COUNT`, both backends) → 20.50 ms alone (−7.2 %), together → 16.92 ms (−23.4 %, past the −17 % hand price: the fold and the clause serve the library's other walks too); interpreted −1.6 % on layout-pinned builds, hash equal | RE-PROFILED at `69085c245` (callgrind, per parse: loft 133 k instructions, the twin 23.4 k; 16.9 against 3.47 ms, 4.9×): the CHECKED ELEMENT READ leads — `vec_get_or_raise_runtime` 21.6 k (16 %, 120 calls at 180 instructions each: it reads the length, then `get_vector` reads the header and the length again), and with `get_vector` and `length_vector` called from other sites the three helpers are 26 % of the row; `parse`'s reset loop 15.8 k (12 %: 40 checked reads per parse, and `self.results[ri] = null` allocates a `String` for the null text before `refill_str`); `find_long_opt` 18.6 k (14 %, its compares); `classify`'s `argv` walk 8 k (6 %: a parameter's vector, not held, each word copied into a `String`); `opt_kind`'s full scan 5.3 k (4 %, the library's own code).  The checked read as one header read PRICED (a runtime A/B, the same emitted Rust against two rlibs, five interleaved rounds, hash equal): `vec_get_or_raise_runtime` resolving the store, the element record and the length once (`get_vector` too), the in-range path inline and the raise outlined (`@FR-R-Cold`) — 16.89 → 15.60 ms (−7.6 %, instructions 280 → 248 M); with `length_vector` forced inline as well (LLVM declined its `#[inline]`), 14.96 ms (−11.4 %), the binary 0.13 % larger.  Then BUILT — `(R-Cold)`'s one-read clause (`vector::rec_and_len`, the runtime, both backends): 16.92 → 14.67 ms (−13.3 %, past the price, once `get_vector_hoisted`'s end-of-walk half was outlined too — inline, it made `mesh_aabb` 68 % slower); on six other lanes, against the same emitted Rust, `chunk_lookup` −14.6 %, `slope_path_with_undo` −15.6 %, `resolve_move` −11.1 %, `replace` −18 %, and nothing slower beyond 5 % at equal or fewer instructions (layout); the interpreter −5.6 % instructions on this row.  The reset loop (`while ri < self.results.len() { self.results[ri] = null; ri += 1 }`) PRICED (six interleaved rounds, hash equal), 14.64 ms at the tip: the null text written as the constant it is, not copied into a `String` first (`text_value_unborrowed` does not name the null constant) −2.0 %; the walk counted over a header held across the text sets — the vector's record and length, which a text set into an element cannot change, its store resolved once — −3.7 %; both −8.7 % (13.37 ms), the allocation costing more once the walk is cheap.  `(R-Header)` declines this loop over its text set (X2), which can grow the store and so move the BASE, never the header.  Then BUILT: `(R-Header)`'s text clause (`LOFT_NO_TEXT_SET_HEADER`, native) and the null text borrowed (`text_value_unborrowed`) — 14.70 → 13.72 ms (−6.7 %, three quarters of the price: each pass still resolves its element's store and checks the write's index); the clause fires in 3 loops of `arguments` and nowhere else the census reaches.  `classify`'s `argv` walk PRICED (six interleaved rounds, hash equal, 13.68 ms at the tip): each word borrowed from the store instead of copied into a `String` −2.9 %; `argv`'s header held −2.3 %; both −6.1 % (12.85 ms).  Neither is admitted now for one reason: `argv` is a parameter and the loop pushes into the return buffer, which a caller may hand in sharing `argv`'s store — then a push could move the borrowed bytes and the held elements.  Both hold exactly when `argv`'s store is none the loop writes, which one run-time compare of store numbers decides (`(R-Alias)`'s versioned clause, X2).  Then BUILT in `(R-Alias)`'s versioned copy: the copy now counts a walked path for the parameter its dep chain ends at (`argv`'s header and base kept there), and `(R-TextBorrow)`'s versioned clause borrows the word — 13.68 → 12.94 ms (−5.4 %), hash equal; `tween`'s `value`, never targeted, −9.0 % from the same counting.  RE-PROFILED at `19cb78861` (callgrind, per parse: loft 106.5 k instructions, the twin 23.4 k — was 140 k; 12.92 against 3.47 ms, 3.7×; the twin runs the same algorithms and the same `memcmp` total): `find_long_opt` 17.9 k (17 %, near its floor in this layout — a held address, the text's record number, a bounds-checked slice and the compare per option, where the twin's `String` is inline); store claims ~11 k (10 %: `claim` and `claim_best_fit` for each token's record and text, which the twin keeps in a `Vec` and borrows); `opt_kind`'s full scan 6.6 k (6 %, the library's algorithm, the twin's too); `find_short_opt` 6.2 k (6 %: `"{fch}" == fso.short_name` formats the loop-invariant character into a `String` per option, ~3 % of the row); the cursor match's text bindings copied into `String`s ~4 k (4 %); the reset loop 4.9 k (4.6 %: `refill_str` of the null text per element); the required-option walk 3.7 k (3.5 %, unheld: its error exit declines the gate); each held walk's read one past its end through `get_vector_cold` 1.9 k (1.75 %: 27 walk ends per parse at 69 instructions, an answer `elem_index` gives inline).  The store claims PRICED and DROPPED (a runtime A/B, six interleaved rounds, hash equal): per parse 15 claims at ~430 instructions each — the free-tree descent and the wilderness test ~1.5 k, `claim_block`'s split and the remainder's `fl_insert` ~2.3 k, `claim`'s own bookkeeping ~1.1 k, `finish_claim`'s switch reads and zero fill ~0.4 k — so even a free claim is under 6 % of the row's instructions.  A wilderness bump in `claim` (no tracked free block: the claim is the wilderness's front, the writes `claim_best_fit` and `claim_block` make) took 55 % of the claims, claim instructions 12.75 → 9.88 M, and the row 12.99 → 12.90 ms (−0.7 %).  The rest is `clear_vector_release` resetting the token store and claiming its vector record again per parse (3.5 k, 3.3 %).  `find_short_opt`'s `"{fch}"` PRICED and DROPPED (six interleaved rounds, hash equal, 13.05 ms at the tip): the format already reuses a work `String` (cleared, one character pushed per option, one heap allocation per call for the first push); built once before the loop −1.4 %, and encoded into a four-byte stack buffer instead of the `String` −3.6 % (12.58 ms) — the lone-character format's heap buffer is the larger half.  Under the bar, as are what is left of the profile's named rows; the row stands at 3.7× |
| 6.86 | `mapfile_to_painted` | X5 d | ≈ 5.2× if P3 and P4 both pay (*est.*) |
| 6.64 | `reload_and_record` | X5 d | ≈ 3.6× with the walk's decision (*est.*) |
| 6.44 | `invert` | X10 | ≈ 4× (*est.*) |
| 4.37 | `truncate_to` (8.17 before X1, 5.04 before the one-resolution copy) | BUILT: the copy of a record of plain vectors between two stores resolves both stores once and reads the cached plan in place, and `remove_claims` tries the plan before the heap-slot test (222 → 202.6 ns a push, −8.6 %, instructions 37.4 → 33.7 M); a reservation the block already holds answered by its claim header (`reserve_more`, no `resize`: → 192.3 ns, −5.3 %).  Both the runtime, both backends.  What is left (callgrind, per push ~3.6 k instructions against the twin's ~750): the deep copy into the timeline's store and the drop's release — two claims and two frees a push, ~170 instructions each where `malloc`/`free` take ~60 — which only a move of the dead entry removes, and it lives in another store (@C125); building the entry ~15 %; `h.entries += [e]`'s append ~8 % | on the line below 4.4× |
| 6.13 | `fill_polygon` (measured at `0c4f9fd33`) | X11 | not measurable on this box |
| 5.74 | cbor `encode_bytes` (noisy: its twin reads 76–96 µs between runs, the native time holds at 0.43–0.44 ms) | X9 | no figure |
| ≈ 4.4 | `slope_path_with_undo` (4.76 calibrated before the field's null arm) | BUILT: a record copy into a minted element's field carries a null arm writing the field's default, so `(R-CompleteWrite)` counts it as covering the field and `undo_push`'s append drops its prefill (100.4 → 92.2 ns, −8.1 %, the hand price; the null arm closed loft#2044 too).  What is left (callgrind, 5.7× the twin's instructions): `map_set_hex` ~397 instructions a call against the twin's 103 and `map_get_hex` ~280 — the chunk walk's held header and base set up per call, three field compares per chunk, `rec_and_len` out of line for the matched chunk's `length_vector` / `get_vector` (8 %: forced inline it made six of eight lane-17 rows 2–6 % slower, dropped); the undo entry's append ~6 % | re-measure |
| 5.66 | cbor `encode` (6.70 before X2's owned-store half) | X9 | ≈ 5× (*est.*) |
| 5.63 | `flow_layout_full` (7.92 before S1c, 6.14 before the text view's own `to_string`) | X3 | ≈ 4.5× (*est.*) |
| 5.52 | `insert_text` | X10 | ≈ 2.2× (*est.*) |
| 4.2 | `forms_upto` (5.02 before the nest clause) | BUILT: `(R-LoopRecord)`'s nest clause — `ef = form_new(…)` kept across the whole loop nest, where the record was freed and minted per pass of the loop around its innermost loop (761 → 638 µs, −16 %, the −17 % hand price).  What is left (callgrind): `form_admissible` ~22 %, the canonical form and its key text ~20 % (576 calls a run), the six appends and two clears of the refill | out of the cut |

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

**The allocator's small-block free list PRICED, then BUILT** (`@FR-H-Park`, `Store::claim`'s parked-block pop) (a runtime A/B: each bench's emitted source
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
value's store.  BUILT as `(R-TextBorrow)`'s tested clause (`LOFT_NO_TESTED_BORROW`, native): the bind
borrows under the run-time store compare and copies otherwise — 396.5 → 356.9 µs (−10 %,
≈ 4.65×), `encode` −2.6 %; the row has left the cut.

**X9 on cbor `encode`, BUILT** (interleaved, pinned, hashes equal): the map encoder's emit loop
copies each key's bytes out of `keybuf` one at a time (`out += [keybuf[koff + i] ?? 0]`), and
`(R-ByteCopy)` declined it only for an exclusive-destination test the clause does not need.
With the test dropped, `Stores::vector_slice` made a block copy without its per-call
temporary, `(R-AppendTwin)` admitting the slice append (without it `encode__ap` stopped being
generated) and `key_lt` taking the header its twin caller already holds: 8.04 → 7.26 ms
(−9.7 %); `decode` and `encode_bytes` level (by instruction count — their time moves ±2 % with
code layout alone).  What is left of the row is the O(n²) rank loop, the twin's algorithm too.

**graphics `fill_polygon` (6.45×, re-measured at `46d145499`), PRICED** (hand forms of the
emitted Rust, three interleaved rounds each, hashes equal; 1.83 ms against Rust's 0.29 ms).
By loft line: the two point reads per edge ~30 %, the per-scanline `polygon_crossings` call
~22 %, `hline`'s per-pixel store ~21 %, the crossing push ~12 %.  Three steps, each on the last:
(a) the return buffer's entry clear — emitted twice, each a `clear_vector_release` that walks
the element type — as one plain length reset for a scalar element: −4 %; (b) `pc_ty = if
pc_a.py < pc_b.py { pc_a } else { pc_b }` read through the chosen held address instead of
`stores.store(&db).get_int` (`(R-RecPtr)` following a conditional alias of two record views):
−13 %; (c) `pc_pts[i]?` taken as `base + i·16` off the held element base when `i` is in range,
with no `DbRef` and no second `rec_ptr` (the out-of-range arm must read the `?` fallback's
zero record, which the hand form did not): −29 %.  Together 1.83 → 1.10 ms (−40 %, ≈ 3.9×).
`fill_triangle` (2.87×) and `fill_rect` (2.52×) share `hline` and are already under 3×.
BUILT: (c) as `(R-RecPtr)`'s index clause in its sound form — the index named once, the
binding as written past the end (−12 % where the unchecked hand form priced −29 %: the
fallback stays in the loop); (b) as the choice clause (−10 %); (a) as the scalar clear (−7 %).
1.84 → 1.35 ms (≈ 4.7×).  Then `hline`'s per-pixel store BUILT as `(R-Fill)`'s guarded clause
(the fill of `lo..min(hi, N - base)`, spans of eight or more), the fill now tried before the
chain guard whose plain copy of the loop had run in its place: 1.36 → 1.02 ms.  `bench/stats.py`
at `0ba7bce2b`: `fill_polygon` 3.53× (6.45× before the arc), `fill_triangle` 2.23×, `fill_rect`
1.91× — the row has left the cut.

**X3 on `flow_layout_full`, PRICED in part** (six interleaved rounds, hashes equal, 10.92 ms at
`c7f19c25a`).  Each run of `slice_runs` pays for its text three times: `buf_slice_text` pushes
the characters into the caller's work `String`, the bind `s = buf_slice_text(…)` copies that
into a local `String` (`.to_string()`), and the run's field set copies it into the runs store.
The local borrowed from the work buffer instead (`Str::str()`; the work text is the call's own
and only `set_str` reads it): 10.92 → 10.00 ms (−8.4 %, ≈ 5.14× — just over the line),
`insert_text` and `invert` level.  The mechanism is general: a call's text result in a work
buffer, bound to a local read only as a text value, needs no copy — the format clause of
`(R-TextBorrow)` for a call.  The rest of X3 — a run that IS a view of the character buffer, as
the twin's `&[char]` is — needs a type change (a `text` over a `vector<character>` slice, or a
measure over characters) and has no hand form.  The borrow BUILT as `(R-TextBorrow)`'s call
clause (`LOFT_NO_CALL_TEXT_BORROW`, native), the run's field set reading the work text in
place too: 10.90 → 10.25 ms (−5.9 %, ≈ 5.28× — still over the line); the hand form and the
build run the same 659 M instructions, the gap between their times is layout.

**`place_line` PRICED** (six interleaved rounds, hashes equal, 10.26 ms at `0bc13b3c4`).  Two
thirds of what runs under it is its two `slice_runs` calls per token (126 M instructions
each of 374 M), and those are mostly the result buffer's bookkeeping, not the cut: the refill
protocol (`refill_keep_open`/`close`, `OpDatabaseRefill`, `clear_vector`) ~50 M, each run's
record mint and finish ~40 M, and `return runs` calling `vector_replace` on the buffer's own
vector — a no-op that pays `deliver_vector_parse_errors` and two slot reads before its
identity test — 17 M.  P1, the self-replace dropped (`runs` is the buffer's own field):
9.86 ms (−3.9 %).  P2, `len(r.str)` counted once per run where `place_line` counts the same
unchanged text three times: 10.13 ms (−1.3 %).  Both: 9.55 ms (−6.9 %, ≈ 4.92× — under the
line).  P1 is general (any result built in its buffer and returned); P2 is a pure count of a
text no statement between writes, which `(R-SameRead)` binds only within one statement.
BUILT, both backends, as `(R-SelfReplace)` (`LOFT_NO_SELF_REPLACE`) and `(R-SameRead)`'s
sequence clause (`LOFT_NO_SEQUENCE_READ`): 9.70 → 9.06 ms (−6.6 %, ≈ 4.93×; 985 → 923 M
instructions), zttext `materialise` −23.9 %, `insert_text` level in instructions (its +8 % in
time is layout); the row has left the cut.

The steps these rows name; **Order**, under the table, is the sequence they are worked in.
No row reaches 3× with everything here built; the rows land
between 3.5× and 6.9×.

| | step | rows it moves (share of the row) | state | first action |
|---|---|---|---|---|
| X1 | BUILT, in the runtime and for one family of records: a record that owns plain vectors and nothing else is copied and released by the list of those fields (`Stores::plain_vectors_of`), where `copy_claims`, `owned_walk`, `remove_struct_claims`, `holds_no_heap` and `remove_claims_mode` asked each field's type what it is.  Both backends; a record with any other owner (a text, a vector of records that own heap) still takes the walk | `truncate_to` (8.17× → 6.03×, −26 %); `panel_build` is level — its records own texts | measured | nothing more of this kind on `panel_build`: its release driven by the type's flattened heap slots (texts deleted in a loop, a collection's own walk called per slot) priced at −3 % of the row and was dropped — the walk's descent is not where that row's release spends |
| S1c | BUILT — `(R-WorkBuffer)`'s result clause: a call's vector result that a function only reads is filled into the caller's buffer, so `token_width`'s runs buffer (and `place_line`'s three) outlive the call and `slice_runs` refills kept elements | `flow_layout_full` (7.92× → 6.23×, −21 %; priced −19 % on the emission before it); a switch A/B of lanes 16 and 18, where the clause also fires, is level | measured | X2 next on this row |
| X2 | BUILT for a store the frame owns — `(R-Header)`'s apart clause: a vector the loop only reads, in a store this activation minted or a work buffer the caller lends, and which nothing in the loop is handed, keeps its header and base whatever the loop calls (no callee verdict is asked).  BUILT for a PARAMETER's vectors too — the clause's parameter half: every writer of the loop proven apart from the parameter, the return buffer by `(R-Alias)`'s versioned run-time test (`invert` −17 %, twelve programs in the census) | cbor `encode` (−20 %, the hand price: `ranks`, `koff`, `klen`, `keybuf`); `flow_layout_full` level (its headers were held already or root in a parameter); `arguments/parse` and `rig_world_frame3` unmoved — their unheld vectors are fields of a parameter record | measured | nothing more on `arguments/parse`: PRICED and dropped.  Its declines are a text set through an existing address (`self.results[i] = word`, `OpSetTextReplace`) and a free on an exit path; with the gate admitting both — a ceiling, since neither is sound as the gate's other readers stand — the row moves −5.3 % (25.45 → 24.10 ms, 21 → 15 unheld reads), the text set alone −4.5 %.  What is left of its vector runtime is single reads outside any loop |
| X3 | flow's runs as views of the buffer they are cut from | `flow_layout_full` (claim and release 25 % before S1c) | *est.* only | after X2: re-profile, then price |
| X4 | BUILT — `hand_price.sh` links a package's native library (§ Tooling) | `check_request`, `pluginabi/request`, `rand_indices` | tooling | `check_request` is profiled (rustc-vs-twin.md § Re-profiled: claims 32 %, spread); `(R-DecodeView)` (decided, C141) next, S8 after its decision |
| X5 | keyed inserts and walks: **a** BUILT — the second lookup of a get-or-insert (`if !h[k] { h += […] }; b = h[k]` looked the key up twice on a hit) is `(R-SameRead)`'s get-or-insert clause, an ENGINE cure rather than the library edit first planned: a loft rewrite of a library for speed is the last resort after code generation (LIBRARY_AUTHORING.md § The library contract, rule 7), and the idiom is everyone's; **b** C7; **c** BUILT for an integer-keyed hash of a heap-free element — `(R-KeyedEntry)`: one probe answers the entry the key names, a repeated key is overwritten in place (no claim, no release) and a new one is filed under the index its claim answered (no `arena::index_of`); an element that owns heap, a text key and the other keyed kinds keep the claim per entry; **d** P3, the palette scan as a slice compare, and P4, an order-free walk that need not sort its scratch (over-9x.md § dryopea) | `field_add_cell` (c: 22.95 → 15.08 ms, −34 %; a: 15.04 → 13.63 ms, −9.4 % of the −12.7 % priced), `build_index` (c: 10.11 → 7.50 ms, −26 %; b 8 %), `collect_dirty_inputs` (c: −10 %), `mapfile_to_painted` (P3 ≤ 12 %, P4 ≤ 15 %; the sort is 9 % of the row), `reload_and_record` (the sort 23 %) | a, c built; d unpriced, and P4 needs a range proof | `field_add_cell` is left with the table's growth (~15 %, a re-hash of every entry from its record) and the new keys' arena claims; d next |
| X6 | BUILT — `map_json`: `T.parse` writes the store as it reads, with no `Parsed` tree between (J9), read by the one lexer made cheaper — a cursor over the text's own bytes (J4), token offsets from the lexer (J5), a number and a string scanned once (J6, J7), no text allocated per punctuation token (J8), each token written where it is kept (J10); over-9x.md § moros `map_json` | `map_json` | measured: 7.44× → 2.68× (J4 −2 %, J8 −12.6 %, J6 and J7 −17 %, J9 −33 %, J5 and J10 −22 %) | the row is under 5× and has left this list; over-9x.md § What is left |
| S9 | sized, and its build half BUILT in the runtime: the buttons and the list already build straight into the return buffer, and each frame's buffer is reused, so what was left was the RELEASE of the last frame's tree — 36 % of the row's instructions, two thirds of that the type walk (`remove_claims_mode`, `remove_struct_claims`, `holds_no_heap`, `owned_walk`'s child list), not the frees.  A record whose heap is texts and vectors of text-owning elements is now released by its type's plan (`Stores::release_by_plan`, `LOFT_NO_RELEASE_PLAN`, `@FR-H-ClearRelease`'s record clause) | `panel_build` | 7.44× → 6.00× (−19.4 %, hand-priced at −18 % by an rlib A/B first); `emit_to_material` −2 % | the frees and claims themselves (~26 % of the row, `delete`, `claim_block`, `claim_best_fit`, `set_free_header`): not an allocator change (two were measured on `truncate_to` and dropped, § 4) — the form @C125 asks for is a claim removed, here a text refilled in its own block |
| X7 | read by line.  `find_long_opt__inv` (17 %): the options vector's header and base are held, but each `fo.long_name` is a TEXT FIELD of a record element, which `(R-Base)`'s text clause (text ELEMENTS only) does not serve — every read resolves the store, the record and the text; the prefix loop reads the field twice and counts its characters for `len > 0` (`char_count` 4.7 %).  `parse` (10 %, and most of `length_vector`'s 10.6 % and `vec_get_or_raise_runtime`'s 3.5 %): the cursor `match` re-tests the subject's length, re-reads element 0 and re-decodes its variant byte in EACH arm's guard, up to six arms per token, where the twin reads the tag once.  `classify` (15 %): `opt_kind`'s full scan, already a counting loop (~3 % of the row), and each token's text copied into its `Tok` (the twin borrows: `copy_claims` 3.9 %, `malloc`/`free` ~5 %) | `arguments/parse` | the cursor match PRICED: −7.4 % (25.52 → 23.63 ms, four interleaved rounds, hash equal) with the subject's length, elements 0 and 1 and their variant bytes read once per `match` instead of in each arm's guard and binding (6 length reads, 13 + 2 element reads, 7 variant decodes per token); the length and elements alone −5.2 % | build it in `parse_vector_match` (`src/parser/control.rs`), so both backends get it: the guards are pure reads and only the arm that matches runs its body, so a read the arms share is bound once before the first.  Then price a text field of a held vector's element read through the held base |
| X8 | BUILT — `(R-FileHold)`: a loop that names one `File` and nothing else that can hold one, and names it only to read scalars, moves the cursor in a guard local and writes the record when the loop is left; `(R-InlineLeaf)`'s reader clause puts a one-statement reader (`read_one_i16(bf)`) in the loop | `binary_read` | 1.47 → 0.84 ms, the hand price ([x8/price_binary_read.py](x8/price_binary_read.py)), hash unchanged | [formal/rewrites.md](../../../doc/claude/formal/rewrites.md) |
| X13 | the cursor kept in the HANDLE while one is open | — | DECLINED: each `File` record keeps its own position (`formal/heap.md` § The built-in `File`, loft#2005), so a read must ask its record where to start; the hold (X8) is what removes that per value inside a loop | — |
| X9 | cbor's byte runs copied as a block; `read_value__d` (17 % of `encode`'s row) | cbor `encode` | *est.* ~10 % | after X2 |
| X10 | a record whose source is dead is moved, not deep-copied: `(R-Compact)`'s prepend and identity forms, `(R-Rebind)` for a chain exit and for the return buffer passed as a parameter | `insert_text` (**60 %**; 4.84× now, under the cut), `invert` | PRICED on `invert` (over-9x.md § zttext `invert`): the prepend in place −24 %, the buffer handed through a chain exit −6 % (the profile's copy share overstated it: callgrind counts `rep movsb` per byte run) | the prepend form is BUILT (`OpPrependVector`, `invert` −31 %), and the per-piece record copy beside it (`(R-FlatCopy)`'s emitted clause, −17.5 %); the chain exit is left |
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
   to a local, prices at −3.4 % and is dropped.  X3's call-text borrow and `place_line`'s two
   levers are built since (≈ 4.93×): the row has left the cut.
4. **`check_request` (5.04×)** — `(R-DecodeView)` instance 1 is BUILT: 2.55 → 1.67 ms
   (−34 %, 80 % of the −46 % hand price).  What is left is the decoded tree itself: the
   structure clause is priced at 1.79× and is the next lever (§ 0's bullet); the flat rest —
   the decode's store per call (~10 %), the key bindings copied (S6, −1.5 %) — does not
   reach the line on its own.
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
10. **cbor `encode`** — X9 BUILT (−9.7 %); re-measure the row in the next portal run.
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
| `check_request` | 0.94× (was 5.04×, ≈ 17× aligned) | DONE (C141): `(R-DecodeView)` instance 1 and its structure clause (read-once) BUILT; before it — the copy loops are already one copy each (`(R-ByteCopy)`'s vector clause, `(R-TextRun)`, built); a slice spelled as a field value (`CBytes { value: bytes[a..b] }`) mints a store and copies twice (+94 % hand-measured), a gap of its own — ceiling with no text or byte string built: 3.95 → 2.54 ms (−36 %), formal/rewrites.md |
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
- **A hash's walk order**: DECIDED (@C151) — `for e in h` keeps key order and no rewrite walks
  a hash in another order behind the program's back; a program that needs no order writes
  `for e in unordered(h)` (slot order, fixed from run to run, no sort).  The twins walk a hash
  in key order too (`by_key`), so `reload_and_record`'s and `mapfile_to_painted`'s walks
  compare like with like.
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
