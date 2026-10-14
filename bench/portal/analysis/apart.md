# Apart values, priced — instances 2 and 3 on the two rows the design named

`doc/claude/APART_VALUES.md` is the rule; this file is its measurement: the two instances
the design said to price before anything is built, priced by hand on the emitted Rust
(`bench/portal/hand_price.sh`; the edit scripts are in `apart/` beside this file, one per row,
so a price can be re-taken after an emitter change), and a triage of every other row in the
systemic classes that LOOKS like a throwaway.  An ANALYSIS: numbers are from one box, none is
a promise.

Taken 2026-10-04 on arm64 macOS (`firewall02`), branch `157-native-4x` at `5737f6413`,
`--native-release` flags against `rustc -O`, `--n 20`, three alternated rounds per form, every
hash equal to the committed row's.  The library checkouts are the portal's
(`loft-libs-assets` `4ff6203`, `loft-libs-world` `a8e48d0`).

## Instance 2 — `mat4_mul`: a 16-float vector inside a value record, across calls

The shape: `Mat4 { m: vector<float> }` built by `mat4_identity` / `mat4_rotate_*`, multiplied
by `mat4_mul(const Mat4, const Mat4) -> Mat4`, rebound 100 000 times in `mul_op`'s loop, and
read through the store only at `mul_op`'s return (`.m[0]?`, `fnv_floats(mul_op(0).m)`) — ONE
boundary per 100 000 products.

What the emission pays today, per product: the refill of the result buffer with sixteen
zeros (`fill_exact`), three vector headers hoisted per call, 128 element reads through
`get_elem_at` (a bounds test and a NaN default each), 16 element writes through the store,
and the caller's exchange of the two buffers.  The 2026-10-02 double-buffer form
(`records.md` § `mat4_mul` re-profiled) IS built (`__rbb_mo_c` in `n_mul_op`): no store is
minted or freed per round any more, and the row is still 4.8× here.

The hand form (`apart/price_mat4_mul.py`): `[f64; 16]` for every `Mat4` inside `mul_op` —
`a_mat4_identity`, `a_mat4_rotate_x/y`, `a_mat4_mul` with the loft body's own loops, the same
wrapping index arithmetic, a read helper that keeps `@FR-H-Index` (negative from the end, out
of range answers NaN) and a write helper that keeps `@FR-H-WriteOOB` — and at the boundary the
ordinary constructor into the return buffer (`OpDatabaseRefill` guard, `clear_vector`, sixteen
`append_f64`), exactly the sequence `n_mat4_identity` emits.

| form | ns per product | × Rust | hash |
|---|--:|--:|---|
| as emitted (`5737f6413`) | 37.9 – 38.5 | 4.8× | `50409c98` |
| **apart: `[f64; 16]` across the four admitted calls** | **4.93** | **0.63×** | `50409c98` |
| the Rust twin (`Mat4 { m: [f64; 16] }`) | 7.87 | 1.0× | `50409c98` |

**−87 %, and under the twin** — the twin returns its `Mat4` by value through memory, the hand
form inlines the whole chain.  The whole price is the store protocol around a value that never
needed one; the arithmetic was never the cost.  (The committed row read 13.2× on 2026-10-02;
`9bcbd152b` and `0390ca612` took it to 4.8× before this measurement.)

What the gate must prove for this shape, read off the hand form: the field's length is the
same constant for the value's whole life (a 16-element literal in every constructor, and
`mat4_mul` only writes elements in range); every use of `.m` between construction and the
boundary is an element read or write or a `len`; the four functions are admitted on both sides
(`const Mat4` parameters, a `Mat4` result bound into a value local or forwarded); and the
boundary is `mul_op`'s return.  `mat4_translate`, `mat4_scale`, `mat4_perspective`,
`mat4_look_at` and `mat4_trs` are the same constructor shape, so a consumer composing a view
matrix per frame takes the same road.

## Instance 3 — `rig_world_frame3`: twelve variable-length vectors, consumed where made

The shape: twelve `vector<float>` locals pushed once per bone (`n = i + 1`, `i < 24` in the
bench) and read by index at the parent bone and at the exit; `(R-WorkBuffer)` already makes
them the caller's pooled buffers (cleared per call, never freed), and the row is 5.3× here.

The hand form (`apart/price_rig_world_frame3.py`) leaves the function as emitted and swaps
only the twelve locals: their prologue, push headers, `push_hoisted`s, `get_elem_hoisted`s
and the exit reads become a Rust value with the `@FR-H-Index` read helper.  Three
representations priced, since the design left the choice open ("an inline buffer, or a Rust
`Vec`"):

| form | ns per call | × Rust | hash |
|---|--:|--:|---|
| as emitted — twelve pooled work buffers | 901 – 911 | 5.3× | `572398f7` |
| apart: `Vec::new()` + push | 1234 – 1445 | **7.2× — SLOWER** | `572398f7` |
| apart: `Vec::with_capacity(n)` + push | 643 – 657 | 3.8× | `572398f7` |
| **apart: an inline stack buffer (`[f64; 64]` + a length, no spill)** | **470 – 474** | **2.75×** | `572398f7` |
| the Rust twin (`[[f64; 12]; MAXB]` on the stack) | 169 – 172 | 1.0× | `572398f7` |

**Two findings.**  A bare `Vec` is not an improvement on the pool: twelve small vectors
grown by push cost twelve `malloc`s and their reallocations per call, more than twelve clears
of buffers that already exist (+37 %).  Reserving the length the compiler can see (`n`, the
loop's trip count, and one push per trip) recovers it (−29 %), and only a stack buffer wins
clearly (−48 %, under the 3× bar).  So **instance 3's representation is an inline buffer with
a spill to the heap past its capacity, never a bare `Vec`** — and the spill must keep the same
answers, which is the whole of its correctness surface.  The remaining 300 ns are not
temporaries: seven reads per bone from the Rig's own vectors (a bounds test and a NaN default
each), `cos`/`sin`, and the twelve `set_float`s of the `Frame` result (past the eight-scalar
tuple width) — the `(Perf-Gap)` against a twin that indexes a fixed stack array.

## Triage — the other rows that look like a throwaway

Every row in the seven systemic classes whose portal note names a per-call or per-iteration
allocation, read at its source for WHERE the value crosses:

| row | the throwaway | crosses | apart? | the nearer lever |
|---|---|---|---|---|
| `mat4_mul` 4.8× | `Mat4 { m: [16] }` per product | `mul_op`'s return only | **instance 2 — priced above** | — |
| `rig_world_frame3` 5.3× | twelve `vector<float>` per call | never | **instance 3 — priced above** | an inline buffer, as measured |
| `terrain_surface_at` 4.2× | `wm`, a weight vector per sample | never | already `(R-WorkBuffer)`'s | the hashed-noise evaluations, not a temporary |
| `draw_bezier` 10× | `bz_tx = bz_sx[0..bz_top]; bz_sx = bz_tx` per subdivision | never | instance 3 would take the stack (one call, ~1000 iterations), but | **a library fix first**: the slice copy is the workaround for the self-slice-assign bug the source names, long closed — a truncate in place removes two stores per subdivision with no rule |
| `surface_fitted_spread` 9.9× | `SideRun` (five vectors in a record) returned by `side_edges`, three times per call | a RETURN, then field reads | instances 2+3 together — a variable-length vector field inside a value record across admitted calls; not priced | **a library fix first**: `surface_of` and `side_edges` are computed twice and thrice for one answer |
| `draft_fit_p` 7.4× | `edgeset_new` per trial inside `run_is_inside` | into `wall_write` and `edge_mat`, callees in another library | not admitted: the value is handed to calls that take a store value | the transitive `(R-WorkBuffer)` — or those callees admitted to take the apart form (instance 2's "across calls", later) |
| `msg_ping` 10× | a nested record pair with text fields, returned | the return, at once | no — made to be stored; its heap fields are texts | destination passing / `(R-ValueRecord)` over text fields (`records.md`) |
| `fill_polygon` 7.9× | a crossings vector per scanline | never | already `(R-WorkBuffer)`'s after `(R-CopyView)` | the per-crossing record copies, `records.md` |
| `flow_layout_full` 14.7× | `vector<Run>` per token, a text per run | a return | no — elements own heap (text) | `alloc-temp.md` § priced: result buffers, the fn-ref ABI, the allocator shave |

The pattern the design predicted holds: the rows apart REMOVES are the ones whose throwaway is
consumed inside one frame or across a short chain of admitted calls; the rest cross a return
into a parent that keeps them, and those are destination passing's.

## Order

1. **Instance 2 — BUILT** (§ Built below).  Its gate (`generation::apart_value::plan`) is
   the one every later instance reuses.
2. **Instance 3 — BUILT** (§ Built below); `draw_bezier`'s stack is its next row once the
   library's slice copy is gone.
3. The two library fixes in the triage (`draw_bezier`, `surface_fitted_spread`) are a
   library branch each — this stream's to make, no consumer in the loop.

## Built — instance 2, 2026-10-14

`(R-Apart)` instance 2 (`formal/rewrites.md` § A record of fixed-length vectors travels as a
value), x86-64 laptop, `--native-release`, branch `157-native-4x` at `f82d8df0e`.  Not the
tuple the order above proposed: each apart type is a value struct of its own (`Apart<n>`, a
`[f64; N]` per field) and each admitted function a twin (`n_<f>__apv`), because
`(R-ValueRecord)`'s return-buffer protocol is the part an array field does not need.

| | before | after |
|---|--:|--:|
| `mat4_mul`, ns a product, one core, three alternated runs | 62.0 | **7.74** (the hand form: 7.75) |
| `mat4_mul` against its twin (`perf_check`, recorded) | 5.26× | **0.57×** |
| float-kernel median, x86-64 | 1.75× | 1.69× (13 of 18 under the bar, 12 before) |

Every row's hash and the run's sink are unchanged; `mesh_to_floats`' native time −1.1 % (its
ratio moved with its twin).  The census: `R-Apart` fires only in mesh3d (15 twin calls).  A
first build rewrote every body holding an apart value, `mat4_mul`'s own included; such a body is
emitted with the vector hoists off, and its store reads lost their headers and bases for every
caller handing it store matrices — the census showed it as drops in mesh3d and glb.  Only a body
that calls a twin is rewritten now, and the only drops left are `mul_op`'s buffer exchange
(`H-SwapRebind`, `R-RebindBuffer`), which a value assignment does not need.

Left on this instance's reach: `return g(…)` forwarded through a hidden buffer (the cells'
first `e2` was declined on it), `len(v)` through the stdlib wrapper, and fields of another type.

## Built — instance 3, 2026-10-14

`(R-Apart)` instance 3, x86-64 laptop, `--native-release`, at `dab609a29`: a `vector<float>`
local or work buffer as `codegen_runtime::ApVec<64>` (64 elements inline, a heap spill past
them).  The hand form re-taken on this box first: 1545 → 692 ns a call (the arm64 price was
−48 %).

| | before | after |
|---|--:|--:|
| `rig_world_frame3`, ns a call, one core, three alternated runs | 1540 | **747** (hand form, no spill test: 692) |
| `rig_world_frame3` against its twin (`perf_check`, recorded) | 5.33× | **2.59×** |
| `bone_shape_has` (hex_body, not targeted) | 2.49× | 1.40× |
| `mesh_emit` (16_consumer_shapes, not targeted) | 2.54× | 2.01× |
| `lock` (12_drawing) | 1.31× | 1.09× (native −3 %) |

Every hash unchanged; no routine of the four lanes got slower.  The gap to the hand form is the
spill test per push and the length test per read, which a vector past 64 elements needs.  The
census: `R-Apart/vector` in hex_body (30), hex_terrain (11), 16_consumer_shapes (4) and
12_drawing (1), and the header, push, `len` and base admissions that drop with it are those
vectors' own.  12_drawing's lane found the one mention the hooks missed — a free on a `return`
path, written through an `Insert` — as a `rustc` error, which is the design's backstop.
