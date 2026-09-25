# What to optimise next — the temporary minted per call, and the two ways to take it

Taken 2026-09-25 on x86-64, after the copy-class rules, `(R-FormatAppend)` and
`(R-ByteCopy)`.  An ANALYSIS: every number is a measurement on this box (`--native-release`,
a standalone binary of the emitted Rust under `perf`), none is a promise.  Nothing in it
is built; the second lever is a design that needs a plan number once this box can
authenticate to GitHub again.

## The finding in one paragraph

The performance overview's largest language-side bucket is a value Rust keeps on the stack
and loft mints as a STORE: a vector or record made fresh per call or per iteration and freed
at the end.  Sixteen of the 96 rows over 3× are that shape, and the two worst classes
(vector-build 11.8×, alloc-temp 11.5×) are mostly it.  The floor was measured on the
smallest such function — `fn f(salt) { v: vector<integer> = []; …; len(v) }` — as
**67 ns per call on an idle box** (110 ns beside a running build) for the mint and the
free alone, against ~20 ns for a Rust `Vec` that is created and dropped; the whole cbor
bench binary spends **~45 %** of its time in that bookkeeping.  It is not one slow
routine: it is a dozen correct steps, each a few nanoseconds, that a stack value never pays.

## Where the 67 ns go

The flat profile of the probe (`mint_only`, 2 M calls), per call:

| step | share | what it is |
|---|---:|---|
| `Stores::database_named` | 12 % | the free slot (a bit scan), `reinit_reused_slot` → `Store::init` (header, free root, the wilderness re-inserted), the serial, the peak |
| `Stores::free_named` | 12 % | the guards (strict, audit, pinned, already free), the cascade lookup, `unlock` (a `Cow` assignment), the poison test, the free bit, `trim_free_top` |
| `op_database_inner` | 11 % | `enum_parent_size` (a type-table lookup per mint), `clear`, the claim, `zero_fill`, the type tag, `set_known_type` |
| `claim_block` + `finish_claim` + `claim` + `claim_best_fit` + `fl_insert` + `set_free_header` | ~29 % | ONE claim on a pristine store: the wilderness taken as the tree node it would have been (`@FR-H-Wilderness`), the remainder carved in place (`@FR-H-Carve`), the free header and footer of what is left |
| `reserve_vector` + `push_header` | 7 % | the vector's record claimed inside the store (a second claim) |
| `store_mut`, `enum_parent_size`, `unlock`, `close_file_handle`, `memset` | ~14 % | one lookup, one flag test, one assignment each — per call |

Nothing here is wrong and nothing is large.  The cheap lever — a shave — would take the
items that are pure re-derivation: `enum_parent_size` per mint (cache the parent size on
the type row), `close_file_handle` per free (a flag the store already carries), the
second claim for the vector's record (a store minted FOR a vector local could claim both
records in one step), the `Cow` write in `unlock` (only when locked).  Together perhaps
20–30 % of the 67 ns, on every temporary in every program, at the cost of touching five
routines each with an `@FR-H-` rule and a pin.  It does not change the shape of the row: a
routine at 25× goes to 18×.

## The structural lever: the temporary is allocated once, by the caller

What removes the mint is not a cheaper mint but no mint: the store lives across calls.
The language already does exactly this for two kinds of temporary, and the third is the
same rule read once more:

* a function's TEXT locals that flow into its result are promoted to a hidden `&text`
  work buffer the caller supplies (`Parser::text_return`, the `__work_ret` / `__tret`
  attributes) — allocated once per caller activation, cleared per call;
* a function's RESULT record or vector is built into a hidden return buffer the caller
  supplies (`__retbuf`, `__ref_N`), minted at first use (`@FR-O-LazyBuffer`) and, where the
  result local is witnessed, allocated once per call SITE and reused across the calls
  (`@FR-R-Reuse`, `scopes::reuse_record_buffers`; `@FR-O-Buffer` is the witness).

The rule to write, `(R-WorkBuffer)`: **a local vector or record of a callee that does not
escape the call — not returned, not stored into a place the caller can reach, not captured,
not linked, not handed to a call that keeps it — is a hidden WORK parameter the caller
allocates once per call site and the callee clears at entry** (`OpClearVector` for a
vector, the record's default prefill for a record) **and never frees.**  The callee's
store for it is then minted once per caller activation, lazily, like `__ref_N`; a callee
called in a loop pays the mint once instead of per turn; a callee called once pays it once,
as now.  Recursion is per activation per site, exactly as the return buffers are.  A
function value (`CallRef`) carries the hidden parameter in its signature the way it carries
the return buffer today, and the live-reload arm mints one per call as it does for a tuple
parameter.  Both backends: the promotion is a parse-time signature change plus a scope-pass
allocation, the interpreter being the values oracle.

Conditions and where they already live: the escape question is `(R-Const)`'s
`use_analysis::read_only_uses` widened to "reaches no place the caller can reach" — the
same walk `(R-Escape)` reads; the per-site allocation and the witness are
`reuse_record_buffers`' own; the lazy mint is `Variable::lazy_buffer`.  What is new is only
the promotion of a LOCAL (today the promotions are of a result) and the clear at entry.

What it moves, from the overview: `fill_polygon` 25× (a crossings vector per scanline),
`flow_layout_full` 15× (a `vector<Run>` per token), `mat4_mul` 32× (a 16-float vector
inside a record per call), `rig_world_frame3` 9.5× (twelve vectors per call),
`delete_range` 9×, `sort_floats` 4×, `terrain_surface_at` 3.2×, `draft_fit_p` 12×,
`combine_cut` 8.7×, `canvas` 9.9×, and the ~45 % of the cbor bench.  Expected: the mint
and free gone from each, the rows to 2–4× where the remaining work is the loop itself.

## Order

The structural lever first: it is the one that changes the shape of the rows, and it needs
no touch on the allocator.  The shave after it, if the remaining temporaries — the ones that
do escape — still show it.  Neither is a rewrite of anybody's loft code: the spelling
`v: vector<integer> = []` inside a function is the natural one, and it stays.

## Built

`(R-WorkBuffer)` landed on 2026-09-25 (`src/parser/work_buffer.rs`, `LOFT_NO_WORK_BUFFER`):
the structural lever above, for a vector local with a scalar element whose every mention is
an in-place vector operator.  The probe measured here went 78–91 → 29–36 ns a call on the
release tier (the hand-written caller-buffer form 26), and the walk's trace over six
libraries promoted 98 locals; the by-value clause built the same day admits a hand-off to a
loft-bodied callee whose answer carries no dep on the parameter (82 promoted over eight
libraries' own code, 28 hand-offs still declined, 37 locals kept for the emitter's
element-first build).  The shave was not built; the temporaries that still
mint are the record and text elements (252 in that census), and the results, which are
another rule's.  One lesson cost a re-measure: the native emitter's ownership test read the
promoted parameter as a possibly aliased view and declined the push window it had for the
local, which made push-heavy rows 1.5× SLOWER; `hoist::work_buffer_arg` now admits the
parameter as exclusive at the push and mint windows.

Measured clean on the final build (2026-09-26): `rig_world_frame3` 9.57× → 5.26×, cbor
`encode` 18.1× → 11.5× and `encode_bytes` 43.3× → 27.1×, `fill_rect` 4.08× → 3.10×,
`draw_line` 2.70× → 2.00×, `blend_pixel` 2.11× → 1.29×, `composite` 1.58× → 1.28×.  The one
row that lost, `bone_shape_has` 4.84× → 5.76×, names the next step: a wrapper called per
element mints its callee's buffers per call one level up and pays the clear and witness for
nothing — the TRANSITIVE form promotes the wrapper's own work-refs onward, so a buffer
climbs to the outermost looping frame (Phase A and B to a fixpoint over the call graph).
