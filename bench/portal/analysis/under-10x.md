# Every routine under 10× — what has to be fixed

The portal run of 2026-10-02 on branch `157-native-4x` (macOS, arm64) left 15 routines at
10× their Rust twin or worse.  Each was profiled and its emitted Rust read beside the twin;
for every one, the work the twin does not do was named, the compiler site that declines to
remove it was found, and the removal was PRICED by hand-editing the emitted Rust
(`hand_price.sh`, in the lima arm64 VM, A/B runs alternated on one pinned core, the hash
column unchanged in every price).  A price is a CEILING for the form: the rewrite that
produces it still has to state the condition that makes it sound (`formal/rewrites.md`).

Read § The fixes for what to build and in what order, § Per routine for which fixes each
row needs, § Measurement for the corrections the work found in the numbers themselves.

## Status

Built on `157-native-4x`, each behind its own switch with a hand-computed guard; re-measured
with `bench/stats.py --routine …` on the same macOS host as the baseline row.

| routine | baseline | now | built |
|---|--:|--:|---|
| 18_consumer_crawler `build_vis` | 18.5× | **3.60×** | F4 (`LOFT_NO_DISTINCT_VERSION`) |
| fixstep `timer_spend` | 12.1× | **3.98×** | F3 (`LOFT_NO_PARAM_RECORD_PTR`) |
| hex_roof `roof_match` | 16.4× | **4.72×** | F1 |
| text2d `draw_quads` | 22.0× | **4.72×** | F2 |
| game_protocol `msg_ping` | 10.1× | **5.94×** | F6's struct-release half, F14 |
| 17_consumer `emit_to_material` | 12.7× | **6.06×** | F9's build-in-element half (`LOFT_NO_LITERAL_APPEND`) |
| gridmesh `build_index` | 13.4× | **6.34×** | F8 (`LOFT_NO_STORE_SWAP`) |
| mesh3d `mat4_mul` | 13.2× | **8.93×** | F11's first two shaves (`LOFT_NO_HEADER_DBREF`) |
| cbor `encode_bytes` | 14.3× | **9.49×** | F7 (`LOFT_NO_FORWARD_RESULT`), F14 |
| 15_stdlib_keyed `sorted_fill_walk` | 10.0× | **1.26×** | its twin made like-for-like (F13 declined) |
| pluginabi `check_request` | 12.8× | 12.6× | F14; F6's text half, F9's prefill clause not built |
| hex_field `doc_read` | 15.6× | 12.7× | F2, F12; the twin row is still suspect (§ Measurement) |
| cbor `decode` | 12.9× | 12.9× | F6's text half, F9's prefill clause not built |
| zttext `flow_layout_full` | 14.7× | 14.2× | F14; F10 not built |
| hex_field `edgeset_count` | 18.1× | 18.2× | — (F5 not built) |

Not built yet, and why each is more than a site edit:

- **F5** needs a SOUNDNESS condition before its price applies: the parameter and record reads
  it would range are unbounded statically, so the plain-arithmetic form is a guarded fast path
  (bounds checked at entry), the `(R-GuardedChain)` shape — not a rule change.
- **F6's text half** — `(R-RefillBuffer)` over a type with text fields needs a text write that
  reuses the old claim when it fits and releases it when it does not; `refillable()` refuses
  such types today because the old text would leak.
- **F9's prefill clause**, **F10**, **F11's float-sum clause**.

Declined: **F13** — `sorted` is not re-implemented for random inserts (§ F13).

Found on the way: F3 first regressed `edgeset_count` to 23.7×.  Bisected at the Rust level
(`hand_price.sh`, which now links on macOS), the cause was COLD setup functions calling
`eg_index`'s twin through a parameter's address — two more callers of the twin changed how
LLVM inlined it into the hot `edge_mat__inv`.  A parameter's address now serves field accesses
only, never a twin call.

## The fixes

Ordered by what each buys per unit of work.  "Size" is the build effort, XS to M.

### F1. The two-argument libm dispatchers are store-free — XS

`OpMathFunc2Float` and `OpMathFunc2Single` (`atan2`, `log(x, base)`) are missing from
`PURE_NULLARY_OPS` (`src/generation/hoist.rs`), which lists their one-argument siblings
`OpMathFuncFloat` / `OpMathFuncSingle`.  Their `const` function-selector parameter makes
`native_op_is_store_free` read them as writers, so ONE `atan2` anywhere in a callee — here
on a cold arc branch of hex_way's `seg_distance` — declines the loop hoist of every caller,
transitively: `track_distance`, both sweeps of hex_roof's `ridge_at`.

- Priced: `roof_match` 23.2 → 6.5 ms/op (−72 %), by removing the branch in a library copy;
  the trace then reports no declined loop.  Reaches every `hex_*` library built on hex_way;
  `atan2` is also in hex_edge and hex_shape.
- Guard: a loop calling a helper that calls `atan2` hoists (`LOFT_TRACE_HOIST_DECLINE`
  silent).  The class behind it: a native op is store-free by NAME LIST or by a signature
  rule, and an op missing from both is silently a writer.  A sentinel that lists every
  native op with only scalar operands that neither path admits would catch the next one.

### F2. A twin call takes scalar inputs through a field path — S

`twin_call_inputs` (`src/generation/mod.rs`) requires each scalar input's argument to be a
plain `Value::Var`; a header input already accepts a pure path (`@PLN157 § V-ac`).  A call
on a field — `atlas.a_cv.get_pixel(x, y)`, `hexset_set(d.doc_cells, …)` — therefore takes the
plain callee, which re-reads the record's scalars through the store and rebuilds the vector
header per call, although the loop already holds the header for that path.

- Priced: `draw_quads` 2.64 → 0.645 ms (−75 %); `doc_read` −31 % (−55 % with F12 and F13).
- Needs: a scalar hoist keyed on a pure path, the same key the header hoist uses.

### F3. A record parameter's field accesses take its address once — M

`(R-RecPtr)` (`record_view_ptr`, `src/generation/hoist.rs`) only considers `Value::Set`
bindings; a PARAMETER is never a candidate, and the decline ("not a binding") is not traced.
`fixstep::timer_spend(t: Timer)` makes 7 field accesses, each `stores.store(&db).get_int(…)`
with its strict-store and bounds checks — which also bloats the function past what
`#[inline]` inlines.

- Priced: `timer_spend` 28.6 → 11.1 ms (−61 %); with the callee then small enough to inline,
  7.0 ms (−75 %).
- Needs: the parameter case of `(R-RecPtr)`, with the same lock and growth conditions as a
  binding; a trace line for the decline.

### F4. A loop versioned on a store-distinct check keeps its parameter headers — M

`@FR-R-Alias` in `hoistable` drops a header rooted at a parameter when the loop pushes into
the return buffer (`any_retbuf`), because `StoreFacts::distinct(param, retbuf)` cannot be
proved statically.  18_consumer_crawler's `build_vis` therefore calls the plain
`sim_hex_state` per pixel although its `__inv` twin is emitted.

- Priced: `build_vis` 3.06 → 0.62 ms (−79 %), headers built at the loop prelude.
- Needs: one runtime test, `retbuf.store_nr != param.store_nr`, choosing between the hoisted
  loop and the plain one.  The same versioning would answer other `distinct` refusals.

### F5. Byte elements fuse, and helper parameters get a range — M (one unit, two halves)

hex_field's `edgeset_count` needs both or neither:

- `FUSABLE_GETTERS` / `FUSABLE_SETTERS` (`hoist.rs`) have no `OpGetByte` / `OpSetByte`, so a
  `vector<u8>` or `boolean` element read goes through `get_vector_hoisted` and a store
  lookup although the base `__ib_0` is passed in, unused.  The getter's bias is the literal
  `0` here.
- `@FR-R-Range` never ranges a parameter or a record read, so `nb_q`, `nb_r`, `eg_index`
  keep `op_add_int` / `op_mul_int`; the overflow note is a side effect, and it stops LLVM
  folding the six neighbour evaluations per slot.
- Priced: byte read alone −13 %, plain arithmetic alone −27 %, both **9.3 → 0.6 ms
  (−93.5 %, about 1.15× Rust)**.  The loop collapses only when every op in the per-slot
  chain is side-effect free.

### F6. A reused buffer is cleared by type, and its text fields refill in place — M

A pooled call buffer is emptied by `OpClear` (`src/scopes/buffers.rs`) → `remove_claims`
→ `owned_walk`, a generic recursive walk that allocates a `Vec` per struct; the callee then
re-claims every text field through `set_str`.  `(R-RefillBuffer)` would avoid it, but
`refillable()` (`hoist.rs`) refuses a type with a text field.

- Priced: `msg_ping` typed clear alone −56 %, with text fields rewritten in place when they
  fit **207 → 35 ns (−83 %, 1.7×)**; `decode` −20 % (the five per-iteration `remove_claims`
  of its pooled temporaries, whose only heap field was already moved out);
  `check_request` −5 %.

### F7. A returned local is built in the return buffer — M

A local bound from a call and returned bare gets its own store, then is copied into
`__retbuf`: cbor's `encode` arms do `OpDatabase` → `n_head(…)` → `vector_add(__retbuf, buf)`
→ `OpFreeRef`.  `(R-ExitVector)` (`src/exit_vector.rs`) declines a bare return, and a
call-bound local is never a candidate.  The rewrite is `(R-Rebind)`'s shape: hand the
function's own `__retbuf` to the call whose result is returned.

- Priced: −35 % on `encode_bytes`; with the callee then refilling the vector it is given
  (`n_head` releases and re-claims it, `OpDatabaseRefill` keeps only a NULL buffer's store)
  **1.5 → 0.47 ms (−68 %)**.  The refill half is worth nothing without the forwarding.

### F8. A keyed result is moved, not copied — S

`h = build_index(…)` emits `OpDatabase(h)` then `OpReplaceKeyed(src, h, 0x8000 | tp)`: a
deep copy whose source is freed by the same op.  The keyed-local `=` branch
(`src/parser/expressions.rs`) has no move form for a fresh-store result into a plain owned
local.  Beside it, `mint_target` (`hoist.rs`) keeps every keyed insert blocking the header
hoist, even into a frame-minted store distinct from the vectors the loop reads.

- Priced: `build_index` 30.9 → 15.7 ms (−49 %); the hoist clause −8 % more.

### F9. An appended record literal is built in its element — M

`tri = Triangle{…}; m.triangles += [tri]` mints a store for `tri`, sets three fields, then
`OpNewRecord` + `OpCopyRecord` + `OpFreeRef` — six times per hex in 17_consumer's
`emit_to_material`.  `v += [vertex(…)]` already builds in place; a literal local whose last
use is the append does not (`LOOP_RECORD` trace: "a native op takes it otherwise").
Beside it, `(R-CompleteWrite)` (`group_covers_type`) stops at the first statement between a
mint and its sets, and ignores a field MOVE, so fully written elements keep the default
prefill.

- Priced: `emit_to_material` build-in-element −40 %, no-prefill −17 % more (813 → 412 µs);
  `decode` no-prefill on move-written elements −12 %.

### F10. Return buffers for fn-ref calls, and no second clear — M

zttext's `flow_layout_full`:

- a fn-ref call answering a record gets a NULL buffer (`@FR-O-Unknown`) and mints and frees
  a store per call — 4 sites, about 3 calls per token;
- `slice_runs` clears its result buffer twice on entry (`clear_vector_release`, then
  `OpDatabaseNP` clears again);
- `(R-WorkBuffer)` declines a result buffer whose element holds a record or a text.

- Priced: −19 %, −12 %, −9 % respectively; with F14 the row goes 19.1 → 10.9 ms (−43 %).

### F11. mat4_mul's three shaves — S each

- The refilled `[0.0; 16]` result literal is rebuilt per call (clear, pre-alloc, append, 15
  template copies) although the buffer already holds 16 elements: write in place.  −30 %.
- Each hoisted element read rebuilds a `DbRef` temporary on the stack for the cold path's
  `&db` (`src/generation/ops/vector_ops.rs`): bind it once at the prelude.  −23 %.
- `bounded_nest` (`hoist.rs`) admits only an `OpAddInt` accumulate; a float sum needs the
  in-range proof and no overflow guard.  −9 %.
- Together 117 → 54 ns.  Not priced: the result rebound through another store
  (`OpRebindRecord`, ~15 %; `(R-ValueRecord)` declines a record with a vector field).

### F12. A counted fill of two vectors is reserved and filled — S

`for _ in 0..n { m += [0 as u8]; sf += [0 as i32]; }` (hex_field `edgeset_new`) pushes one
element at a time: `push_loop` declines "the pushes reach two paths", and a byte push is
kept out of the fill form.  `doc_read` −9 %.

### F13. DECLINED — `sorted` is not re-implemented for random inserts

Every insert into `sorted<…>` binary-searches and memmoves the tail (`sorted_finish`,
`src/vector.rs`): 98.7 MB moved per op in `15_stdlib_keyed`'s `sorted_fill_walk`, where the
twin's `BTreeMap` insert is O(log n).  A deferred sort (append, one stable sort at loop exit)
priced −87 %, and a gap-buffer or chunked layout was the structural alternative.

Neither is built, by the owner's decision: `sorted` keeps its contiguous layout, which is
what makes it the right collection for in-order reads and appends, and slow random inserts
are its known cost.  Making random inserts fast would morph `sorted` into an `index` and cost
it exactly the efficiency it exists for.  A program that needs random inserts to be fast uses `index` — the
keyed collection built for that.  So the 10× measured a documented trade-off, not a
defect: the twin was a `BTreeMap`, which is `index`'s counterpart.  It is now a `Vec` kept
sorted by binary-search insert, an equal key overwriting in place (bench README rule 1), and
the row reads 1.26× with the hash unchanged (`4173` in both lanes).

### F14. Text reaches a store field without intermediate `String`s — S

Every text local, match bindings (`_mv_*`) included, is an owned `String`; a slice goes
through `Display` and a second `to_string()` before `set_str`.  −4 % to −10 % on
`encode_bytes`, `check_request`, `flow_layout_full`.

### Not needed for 10×, priced or estimated

- **Destination-passing for recursive builders** (records.md lever 1): cbor `read_value`
  builds each child in a temporary, moves it and frees it.  Estimated 20–30 % of `decode`
  and `check_request`, not priced.  `check_request` needs it if its twin is corrected
  (§ Measurement).
- **A fixed-size scalar `f#read` without a full runtime crossing**: about 30 % of `doc_read`.
- **A view return for a match the caller only reads** (`pa_get`'s deep copy): −11 % on
  `check_request`; whether a view is allowed is an ownership decision for the owner.

## Per routine

Ratio on the macOS run; priced ratio from the hand prices' fraction applied to it (the VM's
own ratio where the two hosts disagree, § Measurement).

| routine | ratio | needs | priced to |
|---|--:|---|--:|
| text2d `draw_quads` | 22.0× | F2 | ~5.5× |
| 18_consumer_crawler `build_vis` | 18.5× | F4 | ~4× |
| hex_field `edgeset_count` | 18.1× | F5 | ~1.2× |
| hex_roof `roof_match` | 16.4× | F1 | ~4.6× |
| hex_field `doc_read` | 15.6× | F2 + F12 | ~9× (6× on the VM before any fix) |
| zttext `flow_layout_full` | 14.7× | F10 + F14 | ~8.6× |
| cbor `encode_bytes` | 14.3× | F7 | ~4.6× |
| gridmesh `build_index` | 13.4× | F8 | ~6.8× |
| mesh3d `mat4_mul` | 13.2× | F11 (first two) | ~6.8× |
| cbor `decode` | 12.9× | F6 + F9's prefill clause | ~9.5× |
| pluginabi `check_request` | 12.8× | F6 + F9 + F14 | ~10.7× here, 7.5× on the VM |
| 17_consumer `emit_to_material` | 12.7× | F9 | ~6.4× (3.8× on the VM) |
| fixstep `timer_spend` | 12.1× | F3 | ~4.7× (2.5× inlined) |
| game_protocol `msg_ping` | 10.1× | F6 | ~1.7× |
| 15_stdlib_keyed `sorted_fill_walk` | 10.0× | — (twin corrected) | 1.26× |

`decode` and `check_request` are the two that F1–F14 leave near the bar; destination-passing
is their next lever.

## Measurement

- **The host moves the twin, not loft.**  On the macOS run the Rust twins are 30–75 % faster
  than in the lima VM on the same machine, while loft native is about level.  So a ratio
  moved between the two results files is not a regression: `emit_to_material` read 7.7× →
  12.7× across them and is 7.5× on the VM today.  Compare a routine on one host only.
- **`doc_read`'s twin is suspect**: 0.90–0.94 ms standalone on the VM against 0.36 ms in the
  portal row.  Re-measure before ranking the row.
- **`check_request`'s twin does MORE than the library**: it decodes twice, as the library
  source reads, while loft decodes once since `(R-PureReuse)`.  Aligning the twin (bench
  README rule 1) roughly halves its time and doubles the ratio, so this row's real distance
  from the bar is larger than shown.
- **Four benches are not in the run**: `drawing` and `stage` fail to build on the pinned
  `laptop-perf-bench` branch of loft-libs-graphics (items its `main` already makes `pub`;
  `bench/portal/libs.tsv`), and `web` and `server` fail to link on macOS (`ld: mis-aligned
  LINKEDIT string pool` in the library's native dylib).  Their routines may hold more rows
  over the bar.
- **Re-measuring a row** after a fix: `python3 bench/stats.py --routine <bench/routine,…>`
  builds only the programs that hold them; `make perf-check ARGS="--routine …"` compares
  with the machine's committed row.
