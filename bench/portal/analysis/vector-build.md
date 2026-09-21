# What to optimise next — the vector-build class, and four priced levers beside it

Taken 2026-09-21 on x86-64, right after the record shapes (`records.md` § Built), from the
portal's 24 rows still over 3×.  An ANALYSIS: every figure below is a HAND-PRICE — the
emitted Rust of the routine edited to the form a rewrite would emit, compiled with the
`--native-release` flags, run in-process with the result hash unchanged.  **V1–V3 and F1 are BUILT
(§ Built, § F1 built); T1 is re-sized (it needs a refactor first — § T1 re-priced) and C1
is priced with its condition CORRECTED (§ C1's condition — the premise below is false);
neither is built.**  Two candidates priced NEGATIVE or not at all are listed as such; they are not
work.

| # | lever | rows it moves | hand-priced |
|---|---|---|---|
| **V1** | a reserved counted push loop holds the element BASE and writes the length ONCE | `push`, `comprehension`, `grid`, `f32_build`, every `[for …]` | **−78 %**, **−78 %**, **−76 %** on the three rows priced |
| **V2** | the comprehension's loop is the counted push loop it spells | `comprehension`, `grid`, `drain`'s build, every `[for …]` | −27 % (its share of V1's total) |
| **V3** | the reservation is emitted in the arm that RUNS | `push`, every counted push loop behind a chain guard | −8 % |
| **F1** | a field of a `?`-discharged element — `v[i]?.f` — is one bounds test and one load | `record_update`, the idiom everywhere (NOT `chunk_lookup` — § F1 built) | **−65 %** |
| **T1** | the loop variable of `for p in vector<text>` borrows the element | `join`, `word_count`, every walk of texts | **−47 %** |
| **C1** | a function no fn-ref can reach constructs no fn-ref buffer guard | `fibonacci`, every recursive function | **−27 %** |

## V1–V3: a push loop that cannot grow still pays for growing

`for i in 0..n { v += [i * 3 + salt] }` and `[for i in 0..n { i * i + salt }]` build the same
vector.  Both push through `Stores::push_hoisted`: a capacity test, a store resolved through
`allocations[store_nr]`, the element written, the length bumped in the header AND written
back to the record — per element (`(R-Refresh)`: a runtime reader inside the loop must see
every push).  Against the Rust twin's 0.3–0.75 ns an element, loft pays 2.3–3.1 ns.

Three things are true of these loops that the emission does not use:

- **V3.**  The up-front reservation (`(R-PushFill)`) is emitted only in the CHECKED arm of
  `(R-GuardedChain)`'s guard.  The plain arm — the one that runs — has none and climbs the
  growth ladder.  `Output::chain_fast_path` writes the whole plain loop before
  `push_fast_path` emits the reserve, so the reserve lands in the `else`.  Priced by adding
  it to the plain arm: `push` 48.0 → 45.7 µs.
- **V2.**  A comprehension's loop is named `For comprehension` and has THREE statements —
  the iterator, the element's value, the push — and `hoist::range_counters`, the one parser
  of a counted range, answers only a two-statement loop.  So it is not a counted range to
  any rewrite that reads that parser (no trace names it, because none gets as far as
  declining it): its step stays the null-aware `op_add_int`, its body the fully checked `op_mul_int` /
  `op_add_int`, and it reserves nothing.  Every one of the lane's five comprehensions shows
  it (`make_ints`, `v_comprehend`, `v_grid` ×2, `v_drain`).  The natural spelling is the one
  the compiler should optimise.  Priced by giving it what the explicit loop gets:
  `comprehension` 62.0 → 45.0 µs.
- **V1.**  Once the trip count is reserved, NOTHING in the loop can grow that vector — so it
  is growth-free for the pushed path itself, which is `(R-Base)`'s condition, and the push
  is one store through a held base and a local length.  And where the body holds no reader
  of the vector that goes through the runtime (no call that can reach it), the length need
  reach the record once, after the loop.  Priced with a per-push capacity test kept and the
  header path as the fallback: `push` **48.0 → 10.9 µs (0.73× the Rust twin)**,
  `comprehension` **62.0 → 13.4 µs (2.06×)**, and `f32_build` — whose inner loop already
  holds its reservation and makes eighteen header pushes a pass — **777 → 187 µs (~1.9×)**.

Conditions V1 must validate, and where they already live: the counted range and invariant
end are `hoist::push_loop`'s; "no `break`, `return` or nested loop" is its existing decline;
"no runtime reader of P in the body" is `(R-Alias)`'s exclusivity (an owned local or the
return buffer) plus a body with no call that takes the root; the length written on every
exit is free where the body cannot leave early.  `LOFT_HOIST_VERIFY=1` re-derives the push
header at the end and compares.  The class today: `comprehension` 9.53×, `grid` 8.10×,
`f32_build` 7.50×, `push` 3.03×, `copy` 2.50× — the portal's worst class (7.5×), four of five
rows over 3×.  `copy` is a different mechanism (`vector_add` of a whole vector) and is not
moved by these.

## F1: `v[i]?.field` resolves the store although the loop holds the base

`v[i]?.y` used inline lowers to a JOIN — the element, or the `?` discharge buffer — and then
a general field read of whichever it was: `stores.store(&db).get_float(…)`, in a loop that
holds `v`'s header and element base.  In range the join IS the element, so the field is one
bounds test and one load at `base + i·size + off`; out of range the existing path answers.
Priced on `record_update` (`v[i].x = v[i]?.y + …`, whose WRITE is already fused):
**35.6 → 12.5 µs, 4.63× → ~1.7×**.  It is `map_set`'s shape in `chunk_lookup` —
`m.chunks[i]?.cx == cx && m.chunks[i]?.cy == cy && m.chunks[i]?.cz == 0`, three joins a chunk
— and the spelling a consumer reaches for whenever it wants one field of one element.  The
existing fused element read (`hoist::fused_element_read`) serves a SCALAR vector's `v[i]?`;
this is its record-field twin.

### F1 priced to the form it will be EMITTED in (2026-09-22) — and BUILT, see below

The IR is `OpGetFloat( Block "ncc" { Set(t, OpGetVectorNullable(v, size, i)); If(OpConvBoolFromRef(t),
t, Object { OpDatabase(buf, tp); …the default record's fields…; buf }) }, fld )` — the scalar
getter's operand is the JOIN block, which is why `hoist::fused_element_read` (it wants the
element address directly, and already fuses `v[i].y` WITHOUT the `?`) answers `None`.  Four
forms, `record_update`, three runs each within ±1 %, hash `7537` throughout:

| form | µs | |
|---|---|---|
| today | 34.9 | |
| the join kept, its RESULT identity-tested against the header (`rec`/`store_nr`), load through the base | 21.4 | −39 % |
| **A** — the index tested FIRST; in range one load, the join only in the fallback | 12.1 | −65 % |
| **B** — A, and the join's temp still assigned in range (`t = element`) | 12.2 | −65 % |
| **C** — B through an `inline(always)` helper answering `Option<(DbRef, T)>` | 12.0–12.4 | −65 % |

Read it as: **build C.**  The result-identity form looked better on paper — it re-evaluates
nothing and drops no side effect — and loses 40 % of the gain, because the join stays in
the loop and with it a cold `OpDatabaseNP(cell, …)` call that holds the whole body back.
B against A is the useful half: assigning the join's temp in range costs NOTHING (LLVM drops
the dead store), so the rewrite preserves the join's one side effect exactly and owes no
proof that the temp is dead outside its block — a walker that did not have to be written.
What C still owes: the index is evaluated once for the test and again inside the fallback's
join, so it must be free of side effects (find the existing purity predicate; a `Var` is
the clear case), and out of range the ORIGINAL expression answers — never a constant: the
absent arm is the default RECORD's field, which a declared field default can make non-zero.

**Where F1 has to be built — TWO sites, one recogniser.**  An emitter arm alone would
compile, pass every test and never fire: `pre_eval.rs` treats every `Value::Block` argument
as needing pre-evaluation (`needs_pre_eval`, the `Value::Block(_) … => true` arm), so by the
time `FusedElementReadEmitter` runs the join is already `let _pre_40 = { //ncc_6 … }` and the
getter's operand is a local, not the join block.  The existing fused read solves the same
problem the same way — `collect_pre_evals_inner` asks `fused_element_read` so the collector
and the emitter cannot disagree about what is folded — and F1 wants that shape: ONE
`hoist::fused_join_read` asked by both, the collector leaving the join in place where it
answers, the emitter folding it.  The arm belongs INSIDE `FusedElementReadEmitter` (it owns
every scalar getter; a second registration would silently replace it), beside the existing
fused read, with a re-entrancy flag for the fallback's own emission as `nest_raw_arm` has.
The index is a `Var` in the first build: a computed index would re-run CHECKED arithmetic
on the fallback path and could note one overflow twice.  Needs a held header AND base
(`active_vec_base`) — the priced form is the load through the base.

### F1 built (2026-09-22)

`(R-Base)`'s join clause (`formal/rewrites.md`), switch `LOFT_NO_JOIN_READ`, cells
`tests/scripts/158-join-read.loft` j1–j11, pins `tests/join_read.rs`.  `record_update`
34.9 → 11.6–12.3 µs, hash unchanged — the hand-price, to the microsecond.  Pinned (`bench/stats.py`,
±0.8 %): **4.62× → 1.60×** of Rust (range 1.59–1.61); lane 14 now 8 of 12 within 2×, median
1.86×.

⚠ **`chunk_lookup` does NOT move, and the analysis above was wrong to name it** — measured,
pinned, through the switch: 1 097 524 → 1 101 111 ns (±0.2 %).  The shape is `map_set`'s, three
times over, exactly as cell j6 folds it; but `map_set`'s loop HOISTS NOTHING
(`LOFT_TRACE_HOIST_DECLINE=1`: declined by the block holding
`m.chunks[i].hexes[k].h_material = mat; return`), so there is no base to read through.
`map_get` walks `for c in m.chunks` and has no such join; its one `?` is over `c.hexes`, a
field of the loop ELEMENT, which is no loop-invariant path.  What would move the row is that
loop's admission — an in-place scalar set through a vector INSIDE an element grows nothing —
which is a lever of its own, unpriced.  Right about the shape, wrong about the row: a
prediction about a ROW is a claim about every gate between the shape and the emission.

What the building found:

- **Pricing the forms BEFORE building chose the design.**  The form that looked best on paper
  lost 40 % of the gain, and the one question that would have cost a walker and a proof —
  *is the join's temp read anywhere else?* — was answered by a measurement instead: assigning
  it in range is free, so keep the assignment and owe no proof.
- **The absent arm is never a constant.**  j3 declares field defaults of 7, 2.5 and 1.5; the
  sabotage that answers the getter's null off the fast path reads `null null null` for
  `80 57.5 52.5`.  `LOFT_HOIST_VERIFY=1` is blind to it (no held fact is wrong), so the
  interpreter is the falsifier.  A negative index addresses from the end (j4) and lands in the
  same arm.
- **A pin can be wrong where the emission is right.**  The first "no join is lifted" check
  matched ` = { //ncc_`, which is also the text of the fallback arm this rewrite emits — it
  could never pass.  A lifted join is exactly `let _pre_<digits> = { //ncc_`.
- **A callee that holds a `?` is never twinned.**  j12 was written to reach the join read
  through a callee twin and measured not to: the join's absent arm mints a discharge buffer,
  the CALLEE admission reads that as a store write, and the calling loop hoists nothing.  The
  loop admission already knows better (§ V-ad: that mint moves no held vector).  Pinned at zero
  join reads on purpose, so the row moves when that admission widens.
- NOT built: a COMPUTED index (`v[i + 1]?.f`, j11) — evaluated twice on the fallback path, and
  a checked operator's second evaluation can note one overflow twice; binding it once needs
  the fallback's join to read the bound local instead of the expression.  And a loop that
  GROWS a store holds no base, so `out += [v[i]?.id]` (j7) keeps its join — the same loop the
  push window declines, because the join's absent arm mints a store.

## T1: a walk of texts allocates a `String` per element

`for p in words` binds `p` as `{ … store.get_str(…) }.to_string()` — a heap allocation and a
copy per element — and the walk itself is not hoisted (`vector::get_vector` and
`length_vector` per pass, a checked index step: a text element's head is
`OpGetText(OpGetVectorNullable(…), 0)`, which `hoist::iteration_head` does not read).  The
copy exists because a `&str` borrowed from `stores` would conflict with a later `&mut
stores` in the body.  Where the body writes no store the borrow is sound, which is
`(R-Header)`'s own condition and what `(R-LazySplit)` already does for a parameter nothing
writes.  Priced on the stdlib's `join`: **31.4 → 16.7 µs, 5.36× → ~2.9×**, before hoisting
the walk.

### T1 re-priced in its emitted form, and re-SIZED (2026-09-22, not built)

The price holds: `let var_p: &str = { …get_str… };` for `let mut var_p = { … }.to_string();`
and `&*(var_p)` for `&*(&var_p)` at its one use — `join` 30.8 → 16.8–17.6 µs (−45 %), hash
`2df9` unchanged, three runs.  It compiles because `join`'s body touches only its `&mut String`
result, never `stores`.

The COST is not what this page said.  "A text variable is a `&str`" is a notion with NO HOME:
it is spelled inline as `vars.is_argument(v) && matches!(vars.tp(v).base(), Type::Text(_))` at
six sites and more (`dispatch.rs` ×2, `emit.rs` ×3, `mod.rs`; a ±3-line grep, so a lower
bound), it decides the use-site spelling (`&*(var_sep)` for a parameter, `&*(&var_p)` for a
local), and the sites have already disagreed three times in shipped code (loft#1004, loft#1006,
loft#1278 — *"the two spellings of the same write disagreed about the same source"*).  T1 adds
a THIRD class, a LOCAL that is a `&str`, to every one of them.  Patching the sites that can be
found is how loft#1278 happened; the failure is loud (rustc refuses the emitted program) but
it is a program that compiled yesterday refusing today, on one backend.

So T1 is two units, in this order: **(1)** one predicate for "this text variable is borrowed"
asked by every site — a behaviour-preserving refactor whose proof is an EMPTY
`scripts/introspect_diff.sh` over the corpus (the loft-codegen skill's Mode B); **(2)** the
loop variable of `for p in vector<text>` joins that class where the body writes no store.
The second condition is the emitter's to PROVE and not rustc's to catch: a user call is handed
the raw `cell` and takes its own `&mut Stores` from it, so a callee reallocating the store
under a live `&str` is invisible to the borrow checker (`hoist::may_write_store`, as
`(R-Header)` and the push window use it).  The walk itself is a third, separate piece: a text
element's head is `OpGetText(OpGetVectorNullable(…), 0)`, which `hoist::iteration_head` does
not read, so the loop holds no header either.

## C1: every call constructs a guard for fn-ref buffers it cannot have

A `--native-release` function enters with `cr_call_push_lean`, a `CallGuard` and
`FnRefBufGuard::new(cell, false)` — the last one two `Cell` reads on every exit, for return
buffers a fn-ref dispatch arm allocated.  `(R-LeafChain)` drops the frame for a call tree
off any cycle and free of fn-refs; a RECURSIVE function keeps all three.  The depth count is
owed (it is the stack-overflow guard); the fn-ref guard is owed only where a fn-ref dispatch
can reach the function.  Priced by removing it from `fib`: **11.8 → 8.67 ms (−27 %),
4.36× → ~3.2×**.  A further −3 % from plain `n - 1` / `n - 2` needs a fact the range proof
does not carry today — a branch condition (`n > 1` in the arm that subtracts) — and is not
proposed on this row's evidence alone.

### C1's condition CORRECTED before it was built (2026-09-22)

The paragraph above says the fn-ref guard is *"owed only where a fn-ref dispatch can reach the
function"*.  **That premise is false**, and building from it would leak.  The guard is not only
the owner of a fn-ref dispatch arm's return buffer: `OpFreeRefOrHandUp`
(`ops/ref_ops.rs`, `OpFreeRefOrHandUpEmitter`) — an ordinary op, emitted where a callee
returns the store it minted and the caller reads the result as a BORROW — registers that store
against the running frame through the same `cr_fnref_buf`, and `FnRefBufGuard` is what frees
it.  A function with no `CallRef` anywhere in its call tree can therefore still own a store
through its guard.  Found by asking the second-spelling question of the PUSHERS rather than of
the call nodes: three emission sites register a buffer — the fn-ref dispatch
(`emit.rs`, `cr_fnref_buf` and `cr_fnref_minted`) and this op — and only two of them sit behind
a `CallRef`.

The condition C1 can be built on: NOTHING THAT REGISTERS is reachable from the function —
no `CallRef`, no `parallel`, no `yield`, no native (not loft-bodied) user function, and no
`OpFreeRefOrHandUp` — over the whole reachable call tree, cycles INCLUDED (the closure of the
call graph from the function, not `frameless_chain_from`, which answers `false` on a cycle
because the DEPTH count needs the frame there; the guard does not).  Then no entry can stand
above the mark the guard would take, and its drop is empty every time — the reasoning
`is_frameless_chain`'s own doc gives, which never depended on acyclicity.  `fib` satisfies it
(it returns an integer and calls itself), so the −27 % stands for that row.  Its falsifiers are
`LOFT_NATIVE_LEAK_CHECK=1` and `LOFT_STRICT_STORES=1` over a cell that hands a store up through
a RECURSIVE function with no fn-ref — the shape that would leak under the false premise, and
the cell that must exist before the rewrite does.

## Priced negative, or not a clear case

- **`sum` (6.62×) — NEGATIVE.**  Its loop is already a plain counter and a base read; what
  is left is the null-aware checked add, which cannot vectorise.  The admissible route — one
  pass for the element bound (`vector::abs_bound_i64`), then plain adds — measured
  **13.4 → 21.2 µs, slower**: for a single reduction the bound pass IS the cost.  The gap is
  the language's semantics (an overflow's null propagates, C85) against a twin that wraps.
- **`split` (12.27×)** collects `vector<text>`: one owned text per piece against the twin's
  `Vec<&str>`, which allocates nothing.  A representation question (text slices as views),
  one row, and the walked form is already `(R-LazySplit)`'s — a design item, not a lever.
- **keyed (3.5×, ten rows)** is analysed in `keyed.md` § What is left: each remaining item
  is a store-format or data-structure change (the bucket division, a B-tree `index`, a
  chunked `sorted`), priced there at 7–17 % apiece.
- **`mesh_aabb` (4.84×)**: the null-aware float comparison on fields no proof calls non-null
  (`records.md` § 8).  **`par` (5.16×)**: one row, another subsystem; not looked at.

## Built — V1–V3 (2026-09-21)

One clause of `(R-PushFill)` (`formal/rewrites.md`), switch `LOFT_NO_PUSH_WINDOW`, cells
`tests/scripts/158-push-window.loft`, pins `tests/push_window.rs`.  Pinned figures
(`bench/stats.py`, ±0.5 % or tighter), hashes unchanged:

| row | before | after | of Rust |
|---|---|---|---|
| `push` | 48.8 µs | 8.0 µs | 3.03× → **0.47×** |
| `comprehension` | 62.1 µs | 11.0 µs | 9.53× → **1.70×** |
| `grid` | 69.6 µs | 23.9 µs | 8.10× → **2.81×** |
| `f32_build` | 739 µs | 148 µs | 7.50× → **1.49×** (range 1.22–1.50: the Rust lane is noisy, ±22.8 %; native ±0.1 %) |

Lane 14 median 1.90×, 7 of 12 within 2×.  `grid` stays over the bar: what is left is the
outer loop — a vector-of-vectors append, whose hoist is declined whole.

What the building found, that the next lever should start from:

- **The hand-price has to be of the form you will EMIT, not of the idea.**  The first price
  of V1 kept the header's own `len` as the counter and handed the header to the growth arm by
  reference: 22.8 µs, half the analysis's figure.  Its address escaped, so LLVM kept it on the
  stack and every push loaded and stored the length through memory.  Three scalars, the
  growth arm taking the length by value and answering a fresh window by value: 10–13 µs.  A
  helper's signature decides whether the rewrite works.
- **A side-finding, fixed: `(R-Base)` bound a base over a store that grows.**  `growth_free`
  counted the mints that earned a push header, so a mint left on its templates read as no
  growth.  Reachable by default: `o.rows += [seed]` (`rows: vector<vector<integer>>`) beside
  `o.vals[i % 4]?` answered `null` on --native for 1 425 000.  Cell r1.
- **A prediction that failed is a measurement owed.**  d8 was predicted to decline and took a
  window: a result vector's root depends on a `__vdb` witness that is the return buffer the
  CALLER handed in, and `hoist::owned_local` reads any `__vdb*` dep as the store it owns.
  Built both ways a caller could make it and the parameter one store (cells a1–b1): all hold.
- **A sabotage that changes nothing is a finding too.**  Of the admission's three clauses two
  have a cell that fails without them; the third (`may_write_store`) has none, because every
  second grower that could be built declines the loop's hoist upstream.  Recorded as such in
  the cell file rather than claimed.
- **Re-derive a moved pin by what its column MEANS.**  `tests/push_hoist.rs` went red on c7,
  c10, c17: its "hoisted pushes" column counted `push_hoisted::<` lines.  A windowed push is
  still a push through the held header, so the counter took both routes and the table needed
  no number changed.
- **An unpinned bench loop can invent a regression.**  `record_append` read bimodal
  (69k / 76–79k) in an ad-hoc loop while its emission was byte-identical; pinned it reads
  68.4–69.5k.  Not reproduced, cause not established — `bench/stats.py` is the instrument.
- NOT built, and the natural next widening: a body that READS the vector it pushes to
  (`v += [v[i-1]? + x]`, d1) keeps the header push — the reads already serve from the header,
  so it needs the window's length kept current in the header as well.  And `len(v)` in a push
  loop's body (d2) is read by the counted-push recogniser as another write to the path, so
  that loop is not even reserved.

## Order

V1–V3 first: the worst class, four routines priced or sharing the priced loop, every
comprehension in the language, the largest hand-price here, and conditions that three
existing rules already validate.  F1
second — one idiom, −65 %, two lanes.  T1 and C1 are each one clear mechanism worth a
quarter to a half of their rows.  Together they take eight of the 24 rows over 3× to about
or under it.

## Method

`--native-release --native-emit` for the routine's Rust; the edit a rewrite would make,
applied by script to that function alone; `bench/portal/hand_price.sh`, which is `rustc`
with loft's own release line (captured once with a logging `rustc` shim on `PATH`); the lane's own binary run with `--n`, the hash
column compared.  A figure is quoted only where the hash is unchanged and two runs agree.
