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
record-field, keyed).  It is the only file measured whole at a recent tree: `results/laptop.tsv`
(x86-64) holds fresh rows (`f7f23c5b5`) for every row it had at 4.3× or worse and for the
regressions below, and older ones for everything else; § 0 ends with that host's cut.  A row leaves this file when the latest run of EVERY host puts it under the bar; a
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
| `flow_layout_full` | 7.00 | S1c (−8 % alone; S1b is built) and a fused `character` element read | re-price on today's code |
| `invert` | 5.82 | `(R-Compact)`'s prepend form, `(R-Rebind)` for a chain exit | unpriced |
| `sphere` | 5.72 | none named (vector growth, the two `add_triangle` writes); 5.97 → 5.72 with the deep window | unpriced |
| `decode` | 5.61 | C1 D4′ RE-PRICED: the resolve clause already serves its byte reads, and a base held from entry (the reads all precede the first growth) is −5.5 % on `read_value__d` and `__rg`, the same with all 12 sites held; the −21 % priced before predates the resolve clause.  And `(R-Base)` never proves a parameter apart from the return buffer, so the held form needs a prefix clause (the base valid up to the first statement that may grow).  Then C2 | ≈ 5.3×, then C2 |

**D. A cause of their own.**

| row | × | known lever | lands at |
|---|--:|---|---|
| `doc_read` | 6.56 → 5.27 | C8 is BUILT (`OpReadFileInt`: one store resolve, a buffered take, two cursor writes).  Profiled at the current tree: `edgeset_new` 22 % — two vectors built side by side, which `(R-PushFill)`'s several-paths clause reserved but pushed element by element.  BUILT: a window per path (`push_windows_ok`), 2.20 → 1.84 ms, hash unchanged.  Left: `hz_index__inv` 13 %, the u8 and i32 reads 10 % and 9 %, `store_mut` 9 %, `hexset_set__inv` 8 %, the float reads through the generic `OpReadFile` 8 % | ≈ 5.3× |
| `map_json` | 6.64 → 3.41 | BUILT (X6, § 0): `T.parse` writes the store as it reads, on the one lexer made cheaper | over-9x.md § moros `map_json` has what is left |
| `arguments/parse` | 5.69 | none named: the option walk reads its vector out of line (16 %), `len(name) > 0` counts characters (4 %), text copies (~10 %) | unpriced |
| `fill_rect` | 5.50 | C9, no bounds test where the index is proved in range | the range-facts decision |
| `rand_indices` | 5.35 | none — unprofiled: `hand_price.sh` cannot link a package with a cdylib (§ Tooling) | fix the tooling, then price |

**Order for the cut.**
1. Step 0 (done).
2. A: `(R-RecPtr)` in the `(R-Inputs)` twins (`clock_pump`) and `(R-SameRead)`'s guard
   clause (`locate`) are built, and the deep window took `mesh_to_floats` under the bar;
   re-measure `fill_polygon`.
3. The priced units: C1 + C2 (`decode`), C8 (`doc_read`), `(R-DecodeView)`
   (`check_request`), S1c re-priced (`flow_layout_full`).
4. The three decisions, the keyed store format first (most of B), then `(R-ViewReturn)`
   (`anim_of`, `check_request`) and relational range facts (`fill_rect`).
5. Price the rest: `invert`, `sphere`, `map_json`, `arguments/parse`, `rand_indices`.

### The same cut on the x86-64 laptop — the steps, slowest row first

Sixteen rows read over 5× in `results/laptop.tsv` (seventeen at `f7f23c5b5`; `map_json` left the cut at 3.41× with X6 built).  Where each one's time
goes is [rustc-vs-twin.md](rustc-vs-twin.md) § Re-profiled; "lands at" applies every step named
for the row at its figure, and a figure marked *est.* was read off a profile and never
hand-priced (such a figure has paid about two thirds, same section).

| × | row | steps | lands at |
|--:|---|---|---|
| 8.17 | `truncate_to` | X1 | ≈ 6× with X1's whole share gone (*est.*) |
| 7.92 | `flow_layout_full` | S1c, X2, X3 | ≈ 5× (*est.*) |
| 7.85 | `check_request` | X4, then `(R-DecodeView)` and S8 | ≈ 5.5× |
| 7.54 | `field_add_cell` | X5 a, c | ≈ 6.9× with the priced part |
| 7.43 | `panel_build` | S9, X1 | ≈ 5× (*est.*) |
| 7.18 | `arguments/parse` | X2, X7 | ≈ 6× (*est.*) |
| 6.93 | `mapfile_to_painted` | X5 d | ≈ 5.2× if P3 and P4 both pay (*est.*) |
| 6.83 | `binary_read` (7.72 measured alone; the Rust lane is noisy beside others) | X8 | no figure |
| 6.81 | cbor `encode` | X2, X9 | ≈ 5× (*est.*) |
| 6.75 | `reload_and_record` | X5 d | ≈ 3.6× with the walk's decision (*est.*) |
| 6.35 | `invert` | X10 | ≈ 4× (*est.*) |
| 6.13 | `fill_polygon` (measured at `0c4f9fd33`) | X11 | not measurable on this box |
| 5.83 | `slope_path_with_undo` (noisy) | § 5's record-build row names what is left | no figure |
| 5.51 | `rig_world_frame3` | S2, S3 | **2.75×**, priced |
| 5.40 | `forms_upto` | X12 | no figure |
| 5.32 | `insert_text` | X10 | ≈ 2.2× (*est.*) |

The steps these rows name; **Order**, under the table, is the sequence they are worked in.
Two rows reach 3× with everything here built (`rig_world_frame3`, `insert_text`); the
others land between 3.5× and 6.9×.

| | step | rows it moves (share of the row) | state | first action |
|---|---|---|---|---|
| X1 | a record's copy and its release written for the static type: `copy_claims`, `owned_walk`, `remove_struct_claims`, `holds_no_heap` and `remove_claims_mode` walk at run time a layout the emitter knows | `truncate_to` (walks ~15 %, copy 10 %), `panel_build` (~16 %) | named in § 4, no price | hand-price on `truncate_to`'s `history_push` |
| S1c | § 3 | `flow_layout_full` (−8 %) | priced | build |
| X2 | vector headers held across a write the loop makes to ANOTHER store.  `(R-Alias)`'s versioned clause runs a copy of the loop under a run-time test that the return buffer's store is not a parameter's; the same test over any pair of stores the loop names admits these | `rig_world_frame3` (26 %), cbor `encode` (23 %), `arguments/parse` (19 %), `flow_layout_full` (16 %) | a widening of a built rule, no price | hand-price on cbor `encode`'s emit loop (the callee writes `buf`; the loop reads `ranks`, `koff`, `klen` out of line) |
| X3 | flow's runs as views of the buffer they are cut from | `flow_layout_full` (claim and release 25 %) | *est.* only | after S1c and X2: re-profile, then price |
| X4 | `hand_price.sh` links a package's native library (§ Tooling) | `check_request`, `pluginabi/request`, `rand_indices` | tooling | first — `check_request` cannot be profiled with symbols without it; then `(R-DecodeView)` (decided, C141) and S8 after its decision |
| X5 | keyed inserts and walks: **a** the library's one bucket lookup where it does two (−8 %, priced); **b** C7; **c** the record claimed per entry — the arena's bookkeeping on insert (`arena::alloc`, `index_of`, the zeroing; `index_of` goes if the mint hands its index to the finish) and a typed keyed append (keyed.md § What is left); **d** P3, the palette scan as a slice compare, and P4, an order-free walk that need not sort its scratch (over-9x.md § dryopea) | `field_add_cell` (a 8 %, c 26 % of the row), `build_index` (b 8 %, c 30 %), `mapfile_to_painted` (P3 ≤ 12 %, P4 ≤ 15 %; the sort is 9 % of the row), `reload_and_record` (the sort 23 %) | a, b priced; c an instruction ledger, ~17 % of an insert; d unpriced, and P4 needs a range proof | price c on `build_index`; a is a library edit |
| X6 | BUILT — `map_json`: `T.parse` writes the store as it reads, with no `Parsed` tree between (J9), read by the one lexer made cheaper — a cursor over the text's own bytes (J4), a number and a string scanned once (J6, J7), no text allocated per punctuation token (J8); over-9x.md § moros `map_json` | `map_json` | measured: 7.44× → 3.41× (J4 −2 %, J8 −12.6 %, J6 and J7 −17 %, J9 −33 %) | the row is under 5× and leaves this list; what the lexer still pays per token (`cont` / `next` 31 % of the row) and J5 are in over-9x.md § What is left |
| S9 | § 3 — not sized | `panel_build` (claim and release 45 %) | not sized | size it on `panel_build`.  Not an allocator change: two were measured on `truncate_to` and dropped (§ 4), and a claim removed is the form @C125 asks for |
| X7 | `arguments`' own code: `find_long_opt`, `classify` and `parse` are 45 % of the row; `"{c}" == t` builds a text per comparison | `arguments/parse` | not read by line | read the emitted `find_long_opt__inv`, price what the twin does not do |
| X8 | a file's handle and cursor held in locals across a read loop | `binary_read` (the whole row is the emitted loop) | no price; the typed read is built | hand-price |
| X9 | cbor's byte runs copied as a block; `read_value__d` (17 % of `encode`'s row) | cbor `encode` | *est.* ~10 % | after X2 |
| X10 | a record whose source is dead is moved, not deep-copied: `(R-Compact)`'s prepend and identity forms, `(R-Rebind)` for a chain exit and for the return buffer passed as a parameter | `insert_text` (**60 %**), `invert` (36 %) | named in § 4, no price | hand-price on `insert_text`.  The largest single share on this list; it stands here by its row's ratio, as the cut is ordered |
| X11 | `graphics`, `drawing`, `text2d`, `stage` and `assets` build on this box again: they resolve dependencies (`mesh3d` 0.1.2 through `drawing`'s lock file, `shapes`) that lack the `pub` fields @C140 requires, and `main`'s own compiler refuses them the same way | `fill_polygon`, and those five libraries' census rows | library work | bump the pins, then re-measure the lane |
| S2, S3 | § 3 | `rig_world_frame3` → 2.75×, `mat4_mul` (4.54× here) → 0.63× | priced | build, S2 first |
| X12 | `forms_upto`'s remaining claims attributed (`(R-CtorLiteral)` and the refill clause are built) | `forms_upto` (claim and release 24 %) | no step | read `forms_upto__rg` and `form_closes` by line |

**Order.**  Three rules, applied in this order: a step another step or a row WAITS on goes
before it; the slowest row goes first; a step with no price is priced before it is built.

1. **Unblock** — small, and a row waits on each (X6's decision is taken, § 6).  X4:
   `check_request`, the third slowest row, has no profile without it.  X11: `fill_polygon`
   cannot be measured on this box until then.
2. **`truncate_to` (8.17×)** — price X1 on `history_push`, build it if the price holds.  It
   also moves `panel_build`, so item 6 is sized on the profile X1 leaves.
3. **`flow_layout_full` (7.92×)** — build S1c (priced).  Price X2, then build it: the same
   step moves `arguments/parse` (7.18×), cbor `encode` (6.81×) and `rig_world_frame3`
   (5.51×), which is why it stands here and not at their rows.  Then price X3 on what is left.
4. **`check_request` (7.85×)** — with X4 in: profile the row, build `(R-DecodeView)`; S8
   after its decision (§ 6).
5. **`field_add_cell` (7.54×)** — X5 a (a library edit) and X5 b, both priced; price X5 c
   on `build_index` and build it.  `build_index` (4.21×) and `collect_dirty_inputs` (4.09×)
   move with it.
6. **`panel_build` (7.43×)** — size S9 on the row as item 2 left it, then build.
7. **`arguments/parse` (7.18×)** — X7, on what X2 left of the row.
8. **`mapfile_to_painted` (6.93×), `reload_and_record` (6.75×)** — X5 d: P3 first, P4 with
   its range proof.
9. **`binary_read` (6.83×)** — price X8, build.
10. **cbor `encode` (6.81×)** — X9, on what X2 left of the row.
11. **`invert` (6.35×), `insert_text` (5.32×)** — price X10 on `insert_text`, build.
12. **`slope_path_with_undo` (5.83×)** — re-profile; § 5's record-build row names what is left.
13. **`rig_world_frame3` (5.51×)** — S2, then S3.
14. **`forms_upto` (5.40×)** — X12.

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
| S1c | the pooled clause — the return buffer whose elements are kept survives its caller's activation (`token_width` per token), lifted as `(R-WorkBuffer)` lifts a work buffer | M | `flow_layout_full` −8 % alone, −31 % with S1b (priced on today's emission) | over-9x.md § zttext |
| S2 | `(R-Apart)` instance 2 — a fixed-length vector field inside a value record, across admitted calls | new gate over the IR | `mat4_mul` → 0.63× | apart.md |
| S3 | `(R-Apart)` instance 3 — an inline buffer with a spill, never a bare `Vec` | after S2, same gate | `rig_world_frame3` → 2.75×; `draw_bezier`'s two stacks → 2.3× | apart.md |
| S4 | BUILT — `(R-Destination)` with its chain clause: a child built in its element, a map entry's key and value in its two halves (D7).  The pooled buffer's release (D6's target) and the move (D2) went with it | M | `decode` 13.5 → 7.89 ms on one core (−41 %), hash unchanged, the hand price exactly; D5 (no one-element reserve) measured +3 % and dropped.  In its lane: `decode` 8.6× → 4.0×, `check_request` 12.0× → 9.4× (3.87 → 3.11 ms on one core, −20 %).  What is left on `decode`: the twin's body 35 %, the element append machinery ~40 % (`vector_append`, `vector_finish`, the element prefill, `record_new`, the reservation) — the push window held across the recursive call is the next step | over-9x.md, under-10x.md § cbor |
| S5 | BUILT — `(R-FnRefValue)`: a fn-ref whose every target answers a value record answers the tuple, and a record parameter every arm takes as a tuple is passed as one | M | `flow_layout_full` −34.5 % on the flow-only driver (the price −33 %), hash unchanged | over-9x.md § zttext |
| S6 | `(R-TextBorrow)`: the split-table verdict (BUILT for the walk) and the match-binding clause | XS/S + S/M | `check_request` L2; `encode_bytes`' owned match bindings; every `match`-based finder | over-9x.md |
| S7 | `(R-FoldCompare)` — BUILT for a synthesised fold; its operand clause is next | S + XS | `header` 7.4× → 3.87× with S6's split-table verdict (x86-64); the operand clause priced −14 % more | over-9x.md |
| S8 | `(R-ViewReturn)` — a lookup's found entry read in place | M | `check_request` −10 %; `pa_get` is the shape of the hex_* and markdown finders | over-9x.md |
| S9 | destination-directed builds — `(R-Place)` widened to a local with ONE owning sink, the return buffer a host | not sized | `panel_build` (most of its 55 % lifecycle share), `emit_to_material` (`tri`, `up`, `centre`; no-prefill −17 % more), `encode_bytes` (`buf += encode(x)`) | records.md § Order item 2 |

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
| S10 | BUILT — `(R-ElemFirst)`'s comprehension clause: a row built by a comprehension and appended as a whole element is built in that element | M | `grid` 58.8 → 31.3 µs on one core (−47 %, the price), hash unchanged | this file |

### Where the priced path stops

| row | lands at | what is left has |
|---|--:|---|
| `header` | 3.6–4.3× | "twin-shaped" remainder, no further lever named |
| `decode` | 3.6–3.8× | no further lever named |
| `flow_layout_full` | 7.3× on x86-64 (7.92× with S1b and S5 built, S1c −8 %); the 3.6× priced earlier counted both as still to come | X2 and X3 (§ 0, the x86-64 cut), neither priced |
| `draw_bezier` | 3.66× measured on x86-64 (graphics 0.9.8 with the keep-range clause) | under the bar only through S3 (2.3×) |
| `check_request` | ≈ 5.5×, ≈ 17× aligned | decided (C141): `(R-DecodeView)` — the copy loops are already one copy each (`(R-ByteCopy)`'s vector clause, `(R-TextRun)`, built); a slice spelled as a field value (`CBytes { value: bytes[a..b] }`) mints a store and copies twice (+94 % hand-measured), a gap of its own — ceiling with no text or byte string built: 3.95 → 2.54 ms (−36 %), formal/rewrites.md |
| `build_walls` | ≈ 3.3× | no further lever named |

## 4. Named steps with no price

Each is named in an analysis as the row's next step; none has a hand price for the row.

| row | step | named in |
|---|---|---|
| `field_add_cell` | re-analyse: the built bucket replace bought −27 % of the −42 % priced; `rehash_into` re-hashes from records; the library looks the bucket up twice | under-10x.md |
| `binary_read` | NOT the counted `while` (`while` and `for _ in 0..n` emit the same work).  The read specialised to the site's static type is BUILT (`(R-TypedRead)`, with the File record's store resolved once per side): the row is 99 % in the emitted loop.  Next: the handle and cursor held across the loop (X8, § 0) | rustc-vs-twin.md § Re-profiled |
| `insert_text`, `invert` | `(R-Compact)`'s filter, prepend and identity forms; `(R-Rebind)` for a chain exit and for the return buffer passed as a parameter | rules.md, libraries-wide.md |
| `mesh_to_floats` | BUILT (bf4cd8088): `(R-Alias)`'s versioned clause now treats the result local that adopts `__retbuf` as the buffer, and `(R-RecPtr)` reads a view's null test as a read rather than a rebind, so the copy that runs when the buffer's store is not the mesh's holds `mf_tri` and `mf_v`.  `perf_check`: 8.23× → 5.53× (−33 %); `pack_instances`, holding `n` the same way, 7.55× → 4.84× (−36 %).  Next: re-analyse both rows from the current code | records.md |
| `map_json` | PRICED: 86 % is `Map.parse` — the source lexer stepping through JSON 67 %, the `Parsed` tree written into the store 19 % — and `"{m:j}"` 6 %.  The lexer's per-token `Arc<str>` reference count (`Position.file`) was ~21 % of cycles: BUILT as `FileName` (an interned name, identity-first equality), −15 % cycles, same hash.  Next: the `Parsed` → store walk (two passes where the twin parses straight into structs) and the lexer's per-character stepping; a separate JSON byte scanner is ruled out (@PLN109: one lexer).  The parser's own allocations (string compares, key clones) priced at −0.8 % — not worth a change | profiled 2026-10-06 |
| `flow_layout_full` | lever B is built (S5); S1c is next.  The bench itself works around fn-ref records not freed until a plain-value frame ends (`zttext/bench/bench.loft` `flow_op`) — a leak to file or fix | over-9x.md |
| `truncate_to` | NOT the move through a by-value parameter: hand-priced (`e` placed in the timeline's store, `history_push`'s copy a shallow move) it costs +16 %, because the pooled `e` already keeps its vectors across pushes and the placed one claims them anew.  `drop_oldest` is already in place (`keep_vector_range`).  The row is claim-for-claim the twin's malloc-for-malloc (two each per push); what differs is each one's price: `OpCopyRecord` 36 % and the release walk 27 % go through `copy_claims` / `owned_walk` / `remove_struct_claims` for a `Stroke` whose layout the emitter knows.  Next: a copy and a release specialised to the static type (unpriced).  Profiled alone: `Store::claim` is 18–20 %, and it is `claim_scan` — the timeline's store stays in `@FR-H-LazyFree`'s phase for good (each push frees what the next claims, so the untracked words never reach the bound), the freed vectors merge with their neighbours (8 + 13 → 21 words), and every miss walks the chain from the store's first block (480 walks per call).  Measured and dropped: ending the phase once the walks cost a sweep (+3 %: the free tree then costs what the walks did) and exact-size lists of lazily freed blocks (+5 %: 55 hits in 1866 claims, the merged sizes never match).  The walk itself is gone: `(H-ScanIndex)` reads the first fit off the lazily freed blocks' index (8.58× → 8.17× on x86-64).  Next: X1 (§ 0) | records.md § Order item 4 |
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

- `bench/portal/hand_price.sh` links no package native library: a bench whose library ships
  a cdylib (`server`, `pluginabi` through crypto, `web`, `ssh`) needs the `-L native=…` and
  rpath lines by hand (over-9x.md § Tooling).
- No script attributes a profile's samples to one routine of a lane (rules.md § Change 6).
  On Linux the method is `perf` samples windowed by time (rustc-vs-twin.md § Method notes).
  On macOS, `sample <pid>` over a `--native-release --names` build gives the call tree;
  a routine's share is its `n_bench_<row>` (or op helper) subtree.  `--names` disables
  inlining, so a small helper's share is inflated.
- `make rewrite-census` read 1–3 low on `R-ExitVector` and `R-Header` on the aarch64 box with
  the libraries at the baseline's commits (records.md); unexplained.  The same box reads
  `R-RefillBuffer` 1 → 0 in `web` and `server` with the checkout at the baseline's commit — and
  so does the loft commit that blessed the baseline (`9c69e6d8e`, built here), so it is the
  box, not a change.  The baseline is one host's; the census has no per-host row.
