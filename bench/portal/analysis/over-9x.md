# The five rows over 9× — what brings each clearly under 6×

The 2026-10-05 portal run on arm64 macOS (`doc/claude/PERF_PORTAL.md`, results
`bench/portal/results/firewall02.lan.betterbe.com.tsv`) leaves five routines at 9× or worse.
This file names the MECHANISM behind each (never the routine), prices the form a rewrite
would emit by hand on the emitted Rust (`bench/portal/hand_price.sh`; every edit script is
under `over-9x/<routine>/` beside this file, so a price can be re-taken after an emitter
change), and says which rules the fix lives in: an existing rule that DECLINES and the
condition to extend, or a NEW rule, drafted in `doc/claude/formal/rewrites.md` § Proposed.
An ANALYSIS: one box, provisional numbers, none a promise.

Taken 2026-10-05 on arm64 macOS (`firewall02`), branch `157-native-4x` at `29d68fde2`,
`--native-release` flags against `rustc -O`, three alternated rounds per form on an idle
box after the five attributions ran; every hash equals its committed row's.  "6×" is the
row's Rust time × 6; "clearly" means with the usual one-third loss between a hand price and
the built rewrite (loft-optimize § 4) the row still lands under it.

| routine | now | 6× line | hand-priced path | lands at | needs |
|---|--:|--:|---|--:|---|
| server `header` | 13.3× | 4.26 M | borrow the walked line + fold compared without a text | **4.3×** (3.0 M) | EXTEND (R-TextBorrow); NEW (R-FoldCompare) |
| cbor `decode` | 11.2× | 6.45 M | D2 D3 D5 D6 + D7 destination passing + the function-clause base | **4.8×** (5.2 M) | EXTEND (R-CompleteWrite), (R-Base); NEW (R-Destination) |
| zttext `flow_layout_full` | 11.1× | 9.54 M | the fn-ref answers its tuple + one text copy + pooled `runs` + text refilled in its slot | **3.6×** (5.8 M) | NEW (R-FnRefValue); EXTEND (R-RefillBuffer), (R-WorkBuffer) |
| graphics `draw_bezier` | 9.8× | 8.13 M | the library's own 0.9.8 form + the stacks' headers held across the keep-range | **3.7×** (5.0 M) | join main into the bench branch; EXTEND (R-Refresh) |
| pluginabi `check_request` | 12.1× | 1.49 M | a view returned from the lookup + match bindings borrowed, then `decode`'s ladder | **≈ 5.5×** by the decode ladder; clearly under only with texts kept in their slots | NEW (R-ViewReturn); EXTEND (R-TextBorrow); then cbor's steps |

Every row is like-for-like under `(Perf-Like)` (same hash, same algorithm), with two notes:
`check_request`'s twin still decodes twice where `(R-PureReuse)` folds the library's two calls
into one, so its aligned ratio is ≈ 17×, and the bench twin owes a re-alignment (bench README
rule 1, a bench-branch edit); `draw_bezier`'s bench branch carries a workaround graphics
`main` has already replaced (lever 0 below).  Where the twin wins by a representation — a
`&str` into the frame, a by-value struct, a `&[char]` slice — that is a `(Perf-Gap)` of
loft's, and the levers below are the abstractions it is missing.

## The mechanisms, across the five

Four mechanisms carry the five rows; each lever below is an instance of one.

1. **A temporary text or record passes through the store as if it were permanent.**  A
   case fold built to answer one `starts_with` (`header`, 40 %); a run's text claimed and
   released per `slice_runs` call (`flow_layout_full`, 50 %); a `Style` minted per `resolve`
   call for a record its callee already answers in registers (16 %); every decoded text
   claimed in a store where the twin keeps a slice (`check_request`, half of its `decode`).
   The rules that remove it: `(R-Apart)` for a value no store observes, `(R-RefillBuffer)`'s
   text clause for a buffer that survives the call, `(R-ValueRecord)` lifted through a fn-ref.
2. **A result is built in a temporary and moved, where the twin writes the destination.**
   `decode`'s child built in a pooled `Decoded`, cleared, moved out, read back (D1–D7);
   `pa_get`'s found entry deep-copied into a minted result store (`check_request`, 10 %).
   The rule: `(R-Place)` one call deeper — `(R-Destination)` — and a view where the caller
   only reads — `(R-ViewReturn)`.
3. **A held header is dropped by one op the refresh rule has no clause for.**  `OpKeepRange`
   declines `draw_bezier`'s whole loop (three rewrites lost at once); a split bind that
   `(R-SplitTable)` already lowered still reads as a store write to `(R-TextBorrow)`
   (`header`, 23 %); a recursive edge answers `None` to every callee question, so a recursive
   builder holds nothing across its own call (`decode`'s push window, `(R-Inputs)`).
4. **A text crosses a boundary through an owned `String` it never needed** (F14): the
   `Str` → `String` → `set_str` triple copy (`flow_layout_full` 15 %), four `String`s per
   lookup in `pa_get`/`pa_text`, the fold copied a second time in `header`.  The home is the
   "a text local is a `&str`" notion vector-build.md T1 asks for; the match-binding clause of
   `(R-TextBorrow)` below is its first instance.

## server `header` — 13.3× → 4.3×

Per lookup ≈ 7.6 header lines are scanned and one hit.  Shares from `sample`: the walked
line `h` copied out of the store per iteration and freed (23 %) — `(R-TextBorrow)` declines
with "the body may write a store" because `borrowed_text_walks`
(`src/generation/hoist.rs:3961`) judges the raw body, where `parts = h.split(':')` is a
vector bind, although `(R-SplitTable)` has already lowered that split and only the WALKED
vector's table reaches the check (`src/generation/mod.rs:2778`); `h.to_lowercase()` built as
a `String` per line and copied again, to answer one `starts_with` (40 %); the per-call
`prefix` the same way (9 %); no header held for the loop, by the same verdict (8 %); the hit
path's copies (10 %).

| level | lever | rule home | size | ns/op | ratio |
|---|---|---|---|--:|--:|
| base | — | — | — | 9.43 M | 13.3× |
| A | `h` is a borrow of its element | EXTEND (R-TextBorrow): a split bind the split table lowered is not a store write — pass `split_tables.keys()` into `borrowed_text_walks` | XS/S | 7.30 M | 10.3× |
| A+B | the fold is compared, never built | NEW (R-FoldCompare) | S | 3.52 M | **5.0×** |
| A+B+D | the `prefix` operand folded in the same test | (R-FoldCompare)'s operand clause | XS | 3.02 M | **4.3×** |
| +C | the split table slices the borrowed `h`; the value returned once | (R-SplitTable) with A; F14 | S | 2.52 M | 3.6× |

A and B are both needed: A alone is 10.3×, and B needs A or the fold re-reads a copied
`String`.  **A and B built** on 157-native-4x: 5.97 → 3.12 ms in the lane (7.4× → 3.87× on
x86-64, `bench/stats.py`, hash unchanged).  B first measured only −12 %: `pre_eval` lifted the
fold's block into a `let _pre_N` the new emitter never read, so the fold was still built —
both sides now ask one predicate.  D (the `prefix` operand) is next.  After A+B+D what remains is twin-shaped.  `(R-FoldCompare)` reaches every
case-insensitive lookup in the text-scan class (header names, keys, extensions, commands).

## cbor `decode` — 11.2× → 4.8×

181 480 items per op: loft spends 66 ns per item, the twin 5.9.  Self time now: `read_value`'s
own body 31 % (30 NULL `DbRef` inits, the call-line frame, `vec_header` of `bytes` per call,
five field writes each resolving the store, eight exit frees); the append machinery 11 %;
store lookups per byte read and field write 10 %; the 16-byte move 10 %; prefill and the
buffer clear 8 % (D1 is built; the call remains); `OpFreeRef` on NULL buffers 3.5 %.

| level | lever | rule home | size | ns/op | ratio |
|---|---|---|---|--:|--:|
| base | — | — | — | 12.04 M | 11.2× |
| D2 D3 D5 D6 D7 | fixed-width move; NULL frees inline; no one-element reserve; no prefill under a complete move; **the child built in its element** | D2 runtime (`Stores::move_field_out`, `src/database/mod.rs:2975`); D3 `OpFreeRefIfDistinct` (`src/generation/ops/ref_ops.rs`); D5 the parser's one-element `OpPreAllocVector` (`src/parser/vectors.rs:4382`); D6 EXTEND (R-CompleteWrite): `group_covers_type` (`hoist.rs:8827`) counts a field MOVE; D7 NEW (R-Destination) | S XS XS S M | 6.49 M | **6.0×** |
| + D4′ | every `bytes[i] ?? 0` reads through the function-clause header's BASE (36 sites) | EXTEND (R-Base) to the function clause: `vec_header` is emitted there (`src/generation/mod.rs:4564`) with no `vec_base` beside it; nothing appends to `bytes` | S | 5.15 M | **4.8×** |
| + push window | `items += [sub.value]` keeps its push header across the recursive call | EXTEND (R-Push)/(R-PushRec): the loop is declined by the call — `(R-Callee)` answers `None` on a recursive edge (`hoist.rs:1941`) | M | 4.06 M | 3.8× |
| + one resolved store, no call-line frame | the result record written through one store; the recursive body carries no frame | (R-RecPtr) for the return buffer; (R-CallLine) — a ceiling | S | 3.83 M | 3.6× |

D7 is REQUIRED (without it the ladder stops at 8.3×) and not sufficient alone; D4′ makes it
clear.  The recursive edge is one decline that denies every call-crossing hoist at once
(`(R-Callee)`, `(R-Inputs)`, the push window): one fixpoint over the callee's own write set
would admit them together.  The ranged top-level twin `n_read_value__rg` carries the same
body, so every lever must reach both emissions.  Build order: D3, D5 (XS) → D2 → D4′ (moves
every function-clause byte read, `check_request` included) → D6 → D7 (the rule first) → the
push window.

## zttext `flow_layout_full` — 11.1× → 3.6×

Priced on a flow-only driver (`over-9x/zttext_flow_layout_full/flow_only.loft`) whose
emitted bodies are byte-identical to the bench's.  Shares: `slice_runs` (three calls per
token) 50 % — a Run record minted per run, its text claimed, the buffer's previous texts
released at entry; stores minted per call 20 % (`token_width`'s result buffer, the `Style`
per `resolve`); `default_resolver` 16 % — the fn-ref answers a DbRef, so the tuple
`default_style` already returns is written into a minted store, adopted, read back, freed;
`Str` → `String` → `set_str` 15 %; the work `String`'s capacity dropped per call 14 %.

| level | lever | rule home | size | ns/op | ratio |
|---|---|---|---|--:|--:|
| base | — | — | — | 17.72 M | 11.1× |
| B | `resolve` answers its tuple | NEW (R-FnRefValue): the dispatch is a closed `match` over the program's functions of that signature (`src/generation/emit.rs:1766`), so `(R-ValueRecord)`'s admission lifts to the fn-ref type | M | −27 % alone |
| C+D | one text copy; the work `String` keeps its capacity | EXTEND (R-WorkBuffer): a `__work_*` text local is a buffer of the frame; F14 | S | −8 % alone |
| A | `runs` pooled per frame | EXTEND (R-WorkBuffer): record elements whose heap is text (`src/scopes/buffers.rs:244` admits scalars only) — pays only with F | S | −8 % alone |
| A+B+C+D | | | | 10.36 M | 6.5× |
| + F | the kept buffer is REFILLED: a run's text overwritten in its own slot when it fits the slot's claim, replaced otherwise; a shorter result truncates | EXTEND (R-RefillBuffer): THE TEXT CLAUSE — `refillable_plain` (`hoist.rs:8976`) refuses any heap-owning field; F6's second half | M | 5.77 M | **3.6×** |

**Re-priced and built.**  On the emission after `(R-RefillBuffer)`'s heap-element form (its
entry clear a store reset), F alone is −20 %, A alone −8 % and A+F −31 % (26.74 → 18.52 M,
three interleaved runs, hash unchanged; edits `over-9x/zttext_flow_layout_full/price_keep.py`).
F is built as `(R-RefillText)`'s collection clause: 26.0 → 22.7 M (12.35×), the difference
from its price being each kept slot still going through the append.  A is the next unit
(worklist S1c).

F is load-bearing, and it is the mechanism `msg_ping`, `decode` and `check_request` were
priced against too: build it first, then B (reaches every fn-ref API), then A, C, D.  The
hand price of F used `Store::delete` on a text block for the replace path; the built form
uses the release walk's own text free, under the leak census and `LOFT_POISON=1`.

**B built** as `(R-FnRefValue)` with its parameter half, re-priced first on the emission after
`(R-RefillText)`'s collection clause: −33 % priced (`price_fnref_value.py` with the arm numbers
of that emission, 864 / 863), −34.5 % built — 22.2 → 14.5 ms on the flow-only driver, three
interleaved runs on one core, hash `eb888d85` unchanged; in its lane (`bench/stats.py`, 7
samples) 22.17 → 14.41 ms, 11.9× → 7.82× its Rust twin.  The result half alone bought nothing:
`token_width`'s `measure(r.str, resolve(r.style))` hands the resolved record to a second
dispatch whose arms kept their record parameters, which consumed it as a record and declined
the group.  And the dispatch carried an arm no value could select — the driver's own
`bench_flow(n) -> Row` matched `fn(integer) -> Style` by Rust ABI — which would have declined it
on the bench too; arms now match the declared record.

## graphics `draw_bezier` — 9.8× → 3.7×

Lever 0 is the library: the bench pins `laptop-perf-bench`, which still pops its stacks
through a temp vector (the @P390 workaround), while graphics `main` (0.9.8, `2d29620`)
writes `v = v[0..n]`, which `OpKeepRange` keeps in place; `origin/main ^laptop-perf-bench`
is twelve commits.  Joining main into the bench branch is the first action.

| form | ns/op | ratio |
|---|--:|--:|
| the pinned bench branch | 13.25 M | 9.8× |
| the 0.9.8 form | 9.62 M | 7.1× |
| + the stacks' headers held across the keep-range, the `draw_line__inv` call | 5.04 M | **3.7×** |
| **BUILT** — `(R-Refresh)`'s keep-range clause (`LOFT_NO_KEEP_RANGE_REFRESH` off → on, the 0.9.8 form, stats.py) | 10.12 → 5.32 M | 7.46× → **4.08×** |
| ceiling: the two stacks as an `(R-Apart)` instance (a `Vec<i64>` no store observes) | 3.16 M | 2.3× |

The mechanism: `LOFT_TRACE_HOIST_DECLINE=1` names `OpKeepRange` (def 353) at line 457 as
the decline of the whole subdivision loop — `(R-Refresh)` admits only ops with a refresh
clause and the op has none — so three rewrites are lost at once: both stacks' headers, the
sixteen push-window appends, and the canvas-held `draw_line__inv` call `(R-Inputs)` emits
elsewhere.  One clause (below) restores all three.  The `(R-Apart)` form is the owner's
instance list, not needed for the bar.

## pluginabi `check_request` — 12.1× → ≈ 5.5×, clearly under only with texts kept

Per frame: `decode` 84 % (one call — `(R-PureReuse)` folds the library's two), `pa_text` →
`pa_get` 13 %, `Str` → `String` and frees 3 %.  On this map/text workload half of `decode` is
store claim and free (24 %) plus libc alloc, zero and copy (23 %) of four texts and two byte
vectors the twin never materialises — the (Perf-Gap) of the row.  What `pa_text("op")` does
per lookup: `pa_text` mints a `CborValue` store for `pa_get`'s result and frees it; `pa_get`
copies each scanned key into a `String` to compare it; the hit is `OpCopyRecord`, a deep copy
with a text claim (`materialized_view_return`, `src/parser/control.rs:15575`); `pa_text`
copies the value twice more and the caller once (`.to_string()` for `valid_op`).

| level | lever | rule home | size | ns/op | ratio |
|---|---|---|---|--:|--:|
| base | — | — | — | 3.02 M | 12.1× |
| L1 | `pa_get` answers a VIEW of the entry it found | NEW (R-ViewReturn): the gap between `(R-ReturnField)` ("never a parameter", rewrites.md) and `(R-CopyView)`; the emitter's borrowed-view bracket exists (`src/generation/dispatch.rs:1265`) | M | −10 % |
| L1+L2 | text match bindings read as values borrow the field | EXTEND (R-TextBorrow): the match-binding clause | S/M | 2.56 M | 10.3× |
| + decode's ladder | D2–D7, D4′, the push window on the 84 % | cbor above | | ≈ 1.1–1.4 M | ≈ 4.5–5.5× |

The decode ladder was priced on the integer-array workload; here the claim-and-copy family it
shaves but never removes is half the routine, so the ratio it reaches on this row is an
estimate, and "clearly under" needs the texts kept in their slots — the same
`(R-RefillBuffer)` text clause as `flow_layout_full` (the pooled `Decoded` buffer's text
field refilled, not claimed per node) — or a decoded value whose text and byte fields are
VIEWS into the input frame, which is a design (a borrowed field inside a store record does
not exist; `(R-TextBorrow)` is frame-scoped) and starts as a note in APART_VALUES.md.  Against
the aligned twin (decode once, 0.176 ms) the 6× line is 1.06 M, which only that design
reaches.  L1 and L2 are small and reach every `match`-based lookup in every library
(`pa_get` is the shape of hex_*'s and markdown's finders).

## Order across the five

By reach, each unit the mechanism rather than the row:

1. `(R-RefillBuffer)`'s text clause (flow F, check_request's texts, msg_ping's priced −83 %).
2. `(R-Refresh)`'s keep-range clause (draw_bezier, every self-slice pop) — S, three rewrites
   for one clause; and joining graphics main into the bench branch.
3. `(R-TextBorrow)`: the split-table verdict and the match-binding clause (header A,
   check_request L2, every finder).
4. `(R-FoldCompare)` (header B+D, every case-insensitive compare).
5. cbor's ladder: D3, D5, D2, D4′ (`(R-Base)` on the function clause), D6, then
   `(R-Destination)` with its design note.
6. `(R-FnRefValue)` (flow B, every fn-ref API answering a record).
7. `(R-ViewReturn)` (check_request L1), an ownership decision for the owner.

Each lands under its price with a guard of hand-computed cells on both backends, its switch
A/B, `make rewrite-census`, and `make perf-check` on its own row; after every two or three,
re-measure the band (PERFORMANCE.md checklist step 8), because the shares above describe
the program as emitted today.

## Tooling found on the way

`bench/portal/hand_price.sh` picked the OLDEST of several `libloft_ffi-*.rlib` generations
(`ls | head -1`; rustc refuses with "colliding StableCrateId") — fixed to the newest.  It
still links no package native library, so a bench whose library ships a cdylib (`server`,
`pluginabi` via crypto, `web`, `ssh`) needs `-L native=~/.loft/build-cache/<pkg>-<ver>/release
-l dylib=loft_<pkg>` and an rpath by hand; the `hp.sh` beside each of those two reports is the
fix shape, not yet in the script.
