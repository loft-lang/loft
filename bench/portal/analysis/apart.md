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

1. **Build instance 2 first** — the gate over the IR (construction, every use, every
   assignment, the boundary), the `[T; N]` field inside the value record's tuple, the
   boundary constructor, `LOFT_NO_APART=1`, cells per allowed use × both switch states.  It
   is the one unit priced at an order of magnitude, it closes the worst float-kernel row, and
   its gate is the same one every later instance reuses.  `make perf-portal` after it: the
   float-kernel median should move, no long-lived class may.
2. Instance 3 as an **inline buffer with spill**, priced here at −48 % on `rig_world_frame3`;
   `draw_bezier`'s stack is its second row once the library's slice copy is gone.
3. The two library fixes in the triage (`draw_bezier`, `surface_fitted_spread`) are a
   library branch each — this stream's to make, no consumer in the loop.
