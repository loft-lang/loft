# The rewrite rules audited — what to change, what to optimise

The register is `doc/claude/formal/rewrites.md`: 51 rewrites and five framework rules
(`(R-Switch)`, `(R-Escape)`, `(R-State)`, `(R-Refresh)`, `(R-Alias)`), every rewrite with a
switch, and its deviation list at `OPEN: 0`.  This file records the audit of 2026-09-28 on
branch `157-native-4x`: which cases each rule still declines on the benches, which shapes
have no rule, and where the instruments that keep the rules honest are thin.  The per-class
pricing stays in the other files here; this one is read from the RULES' side.

## Method

- **Declines, measured.**  Every `LOFT_TRACE_*` switch the compiler reads (56; see the
  first change below) set at once over `--native-emit` of the 19 bench lanes, the lines
  filtered to the benches' own functions (`n_*`, not the standard library's `t_*`/`i_*`),
  normalised and counted.  The count says how often a rule LOOKED at a site and refused.
- **Per-routine profiles.**  `perf record --call-graph dwarf` over a lane's
  `stats_native` binary; each sample attributed to the routine called from the bench's
  `n_main` and its leaf symbol tallied within that routine.  The whole-bench profile mixes
  routines; the attribution is what names a routine's own cost.
- **The register read whole**, rule by rule, for the gaps each rule's own text names.

## Change — the instruments and the register

1. **The census counted 24 of the 51 rewrites — DONE 2026-09-28: it counts 49.**  A row
   now exists for every rewrite but `(R-Prefill)` (admits per TYPE at its first mint, a
   run-time fact) and `(R-Cold)` (the runtime's own shape); the generator's rules count
   where they emit, the parser's and the scope pass's at their admission (the standard
   library's included, one constant per program).  Over the 57 programs every new row is
   non-zero somewhere — `(R-Place)`, `(R-MoveLast)` and `(R-Compact)` on ONE program each,
   which is how thin the body is for the ownership family.  A lost admission in any of
   them was invisible until a consumer bench moved — the loft#1647 shape, six days on
   main; now it is a named drop in `make rewrite-census`.
2. **The trace switches — MEASURED, smaller than it read.**  `src/` reads 56
   `LOFT_TRACE_*` switches and `NATIVE_SWITCHES.md` named 17 of them; but 28 of the rest
   live in their family's home (`BOTH_BACKEND_SWITCHES.md` for the parser and scope-pass
   rewrites, `DEBUG.md`'s table for the runtime and lexer traces, `PLACEMENT.md`,
   `PERFORMANCE.md`).  What was missing, now added: two native REWRITES with no entry at all
   (`(R-SplitTable)` and `(R-GuardFree)`, each with its `LOFT_NO_*` switch), the traces of
   `(R-LitHoist)` and the element fuse, and seven switches no document named
   (`LOFT_TRACE_KEYS`, `_VADD`, `_RETFRESH`, `_PREAMBLE`, `_INSTANCE_KEY`, `_CLOSURE_KEEP`,
   `_PAR_WORKERS` — `DEBUG_STORES.md` § Debugging store-ownership bugs).  A trace run should read
   the rule's home doc, not this one file.
3. **One known defect sat outside the deviation list — FILED loft#1729, deviation D-rw-6
   (`rewrites.md` OPEN: 1).**  `(R-Const)`'s bind of a top-level constant that is then
   written (`c = NAMES; c[0] = 5`) panics with "write to read-only store" on both backends;
   the fix is the copy road the rule already names.
4. **`(R-State)` is held by collector order alone** (`hoist::hoistable`: read candidates,
   then twin inputs, then push paths).  The one-holder-per-path rule has no construction
   enforcing it; the state-builder refactor ("one map, one insert path") is queued in
   @PLN157 and is the right closure.
5. **`README.md`'s status table lagged its files** (alloc-temp read "designed, not built"
   after `(R-WorkBuffer)` landed; round-3 read "nothing built" after W1, W2, C1 and P1).
   Fixed with this audit; the table is the resume point, so it has to be re-read when a
   file's § Built grows.
6. **A per-routine profile attributor is missing from the tools.**  `perf report --parent`
   does not filter these binaries' chains; the audit attributed samples by hand from
   `perf script`.  Worth a script beside `hand_price.sh`, since every "why is this row
   slow" question starts there.

## Optimise — the cases the rules decline on real programs

The declines below fired inside the benches' own functions (lanes 15–18 unless noted);
each is a case the rule's text names as its next clause or its known limit.

| decline, as traced | count | rule | the case |
|---|---:|---|---|
| "the remainder may grow a store" — `panel_build`, `make_map`, `reload_and_record`, `make_names` | 56 | `(R-RecPtr)`, `(R-Base)` | **BUILT 2026-09-28** (`(R-Base)`'s growth clause, `LOFT_NO_DISTINCT_GROWTH`): a growth is judged per STORE, so `out += [v[i]?.id]` keeps `v`'s base and address.  Six bench functions gained bases (`c_flow`, `emit_mesh`, `truncate_to`, `drop_oldest`, `c_talus`, `row_crossings`); measured on lanes 16–18, old compiler against new on one box (`bench/stats.py`, hashes identical): `bfs_flow` 1.65× → **1.20×** (−27 %), `mesh_emit` 3.30× → **2.73×** (−17 %), `sort_floats` 1.76× → **1.52×** (−14 %), `binary_map` −5 %, the other 26 routines within ±4 %; the four named here did NOT, each for a reason the trace names — `reload_and_record` walks HASHES (no vector header at all), `make_names` pushes TEXTS (not a fusable push, the loop never reaches the growth question), `make_map` re-mints a heap-owning record local per pass (`OpDatabase(ck, …)` blocks the hoist: the rebound-mover clause's next case), `panel_build` is the "bound null" row below |
| "bound null" — `panel_build`, `emit_hex_surface`, `enum_match` | 55 | `(R-RecPtr)` | a record minted into a pre-initialised buffer (`__ref_p2_N = null`, then the mint) never gets its address; only the mint clause's own window does |
| "no push at the top level of the body", "the body can leave early" — `fov_rays` | 61 | `(R-PushFill)` | a push under a branch, after an early exit, or beside an inner loop keeps the header push |
| "iterator block is `iter next`/`for text next`, not a range" (suite-wide) | 299 | `(R-Fill)`, `(R-BoundedNest)`, `(R-PushFill)`, `(R-GuardedChain)`, `(R-Counter)` | none of the counted-loop rules see a `while` or a `for e in v` walk; `binary_read`'s loop is a `while` |
| "rebound in the body", "handed to a call outside a text-value position" — `slice_shrink`, `format_specs`, `make_names` | 20 | `(R-TextBorrow)` | a `&p` link, a rebind, or a text-building op with `p` as destination needs a `String` slot |
| "Grown through parameter (whole)" | 50 | `(R-Base)` | growth through a parameter blocks the base for the whole loop |
| a nullable view tested against null (`if v != null`) — `mesh_to_floats`, `mesh_to_floats_uv` | 4 loops | `(R-Header)`'s store-free list | **BUILT 2026-09-29**: `OpEqRef` / `OpNeRef` compare two `DbRef`s and touch no store, now listed beside `OpRefIsNull`; `mesh_to_floats` 36.9× → 15.8× here |
| a scalar vector literal walked by `for` (`for i in [t.a, t.b, t.c]`) — `mesh_to_floats`, `mesh_to_floats_uv`, hex_fit's bench | 7 loops | `(R-LiteralWalk)` (new) | **BUILT 2026-09-29**: the items are scalar temps and the walk a counted select, no vector on either backend; `mesh_to_floats` 15.8× → 10.5× here |
| a callee whose `??` fallback carries a TEXT field — `frame_of`, `lit_colour` under `pack_instances` | 3 callees, 1 loop | `(R-Callee)`'s discharge allowance | **BUILT 2026-09-29**: the set into the discharge buffer is admitted as its mint is; `pack_instances` 24.3× → 8.4× here |

And the clauses the rule texts name, in the order the rows pay for them:

- **`(R-ValueRecord)` / `(R-ValueLocal)`** — a vector field at any depth is declined (the
  persisted file grew 25 %); a bind from a sub-record keeps the local a record; `q = p`
  declines as an untyped mint.  `resolve_move` (13.5×) calls the accessor
  `slope_path_with_undo` was fixed through and is the row to re-read.
- **`(R-PushRec)`** — the tuple delivered into an appended slot is the written next clause;
  `add_vertex` still writes eight fields through `store_mut` (`sphere` 14.5×,
  `mesh_to_floats` 21×, `msg_ping` 11× wait on it).
- **`(R-TypedKeyed)`** — the lookup half only.  The append walks the type table per element
  (`nullable_field_parent`, `sub_record_type`, `field_ref`, twice): ~300 of an insert's
  2 100 instructions, priced −8 % on `hash_fill`.
- **`(R-WorkBuffer)`** — scalar elements only.  Record and text elements (252 in the
  library census), result buffers, and hand-offs to a `&` parameter, a native or a
  view-answering callee (28 declines) keep the 67 ns mint-and-free.
- **`(R-Compact)`** — the filter, prepend and identity forms are not built (`invert` 99×,
  `insert_text` 63×).
- **`(R-Rebind)`** — chain exits (`return delete_range(d, …)`) are the next admission.
- **`(R-BoundedNest)`** — the dot product (`acc += a[i]*b[i]`, bound 2^20) and any float
  accumulate.
- **`(R-Base)`** — a computed join index `v[i+1]?.f` is evaluated twice on the fallback
  path, so a checked operator can note one overflow twice; binding it once needs the
  fallback's join to read the bound local.
- **`(R-Range)` / `(R-GuardedChain)`** — parameters and record reads are never ranged by
  shape (12 of `composite_layer`'s 36 operators stay checked); the chain guard is an
  under-approximation that costs `hair_brush` 8 % on a trip count no static rule sees.
- **`(R-Mint)` / `(R-GroupPush)`** — a field-form mint group outside a held header keeps its
  templates, because the parser reserves only for a local vector.
- **`(R-FormatAppend)`** — a field or element destination, and BUILT text rather than
  appended text (`render_inline` 5.95×).
- **`(R-Place)`** — the implementation admits less than the rule: the relocation only for an
  element appended to a PARAMETER's collection, outside a loop, with no second destination.

## Optimise — shapes with no rule

- **A record tree rebuilt per frame.**  `panel_build` (10.4×) spends its samples freeing the
  previous `Panel` (`remove_claims_mode`, `owned_walk`, `copy_claims`) and claiming text
  stores (`claim_block`, `claim_best_fit`, `set_free_header`).  `(R-LoopRecord)` and
  `(R-LoopBuffer)` keep a store across passes only for a no-heap record or ONE scalar
  vector.  A rule letting a heap-owning local rebound to a fresh literal reuse its stores
  is the missing sibling, and it is the one genuinely new rule this audit found.
- **A loop-invariant element read of a held, unwritten vector.**  Named in
  `(R-ValueLocal)`'s text as "the next one to write"; `(R-Invariant)` covers integer chains
  only.
- **A callee's result placed inside the caller's store.**  `check_request` (51×) and the
  fn-ref result buffer are both called "a design, not a rewrite" (`alloc-temp.md`).
- **The `while` loop as a counted loop.**  Every counted-loop rule asks for a `for` over a
  range; a `while i < n { …; i += 1 }` with a provable step is the same shape and gets
  nothing (`binary_read` 8.4×).

## Not rewrites — structural, priced elsewhere

The keyed class (3.5×) is store-format work: the division in `hash::home_bucket` (part of
the placement contract, kept for a format revision), the `sorted` insert's memmove (50 % of
`sorted_fill_walk`), the `index` red-black tree of store records, and text keys copied
into the record where the twin borrows `&str` — 7–17 % apiece, no single lever to 2×
(`keyed.md` § What is left).  The mint-and-free floor's allocator shave (20–30 % of 67 ns,
five rule-pinned routines) does not change any row's shape (`alloc-temp.md`).  `map_json`
(7.3×) is `Map.parse` running the source lexer over JSON and interning every string — a
library routine, not the emitter.

## Measured on the way: the branch against main

`bench/stats.py` over the 19 lanes, branch `3b79ae0a` against `origin/main` `d7daefcd`,
same box, the Rust lane shared as the drift control: 71 of 79 routines within noise, the
median ratio 2.24× → 2.16×.  Three movers repeat across three runs — `hash_walk` +5–7 %,
`parse_num` +4–9 %, `index_fill_find` −11 %.  The emitted Rust of both lanes is
byte-identical between the binaries, so only the runtime rlib differs; isolated in a
routine-only program on one pinned core the walk is 6 % FASTER on the branch and the parse
equal.  The suite's +5 % is code placement in a shared binary after a 460-line runtime
addition, the class README § What this arc learned calls "an identical function at a new
address" — not a regression to chase.

## Order

1. ~~Census rows for every rewrite; the trace switches documented; the `(R-Const)` panic
   filed and its deviation opened.~~  Done 2026-09-28 (§ Change 1–3).
2. ~~`(R-RecPtr)` and `(R-Base)` under growth~~ — built 2026-09-28 (per-store growth); what
   the four named functions still need is the heap-record rebind mint (`make_map`) and the
   "bound null" window (`panel_build`), both rows of the table above.
3. `(R-PushRec)`'s appended tuple; `(R-TypedKeyed)`'s append half; `(R-WorkBuffer)` for
   record elements and results — each priced in its class file.
4. The two new rules: the record tree reused across a rebind, and the `while` loop as a
   counted loop.
