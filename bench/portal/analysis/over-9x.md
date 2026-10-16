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
from its price being each kept slot still going through the append.  A is built as
`(R-WorkBuffer)`'s result clause (worklist S1c), re-priced first on the emission with B in
it (−19 %, one static buffer in `token_width`): 14.35 → 11.26 ms on one core, 7.92× → 6.23×
in its lane, hash `eb888d85` unchanged.  It is wider than the price — `place_line`'s three
run buffers are held the same way.

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
(`ls | head -1`; rustc refuses with "colliding StableCrateId") — fixed to the newest.  A
bench whose library ships a cdylib (`server`, `pluginabi` via crypto, `web`, `ssh`) needs
`-L native=~/.loft/build-cache/<pkg>-<ver>/release -l dylib=loft_<pkg>` and an rpath; the
script takes them from the package's own compile (worklist.md § Tooling).

## The three rows still over 9× (2026-10-07, after `(R-Destination)`)

`map_json` 9.8×, `check_request` 9.2×, `mapfile_to_painted` 9.1× in their lanes.  Each row
was profiled alone on the shipped flags (`--native-release --names`, one core, the P-core
table of `perf`); each named lever below was priced by a temporary edit — of the runtime
(rlib A/B) or of the emitted Rust — with the hash unchanged, then reverted.

### moros `map_json` — the parser, not the formatter

The twin parses bytes straight into its structs; loft drives the general source lexer in
JSON mode, builds a `Parsed` tree, then walks the tree into the store.  Three runtime fixes,
all shared by both backends and by every `T.parse` of JSON:

| lever | what it removes | priced |
|---|---|--:|
| J1 | `JParser.path` — pushed and popped per array element (`idx.to_string()`) and per key (`name.clone()`), read only when an error names its path — so it is built while the error unwinds instead; and four token tests that build a `String` to compare (`LexItem::Token("}".to_string())`) where `Lexer::peek_token` compares in place | 15.0 → 13.7 ms (−9 %) |
| J2 | the lexer's two `HashSet<String>` lookups per punctuation character: an ASCII table for one-character tokens, the two-character lookup only where a two-character token starts with this character (the price form skipped the formatting-mode `}` resume — the real change must keep it) | → 12.7 ms (−8.5 %) |
| J3 | `walk_parsed_into`'s per-object `HashSet<&str>` of found fields under SipHash; `FxHashSet` (a bitset over field indices would be cheaper still) | → 11.8 ms (−7 %) |

Together −21 % (≈ 7.7×).  **Built** (6534ee4a9): 15.3 → 12.0 ms alone on one core; the walker's
path is built on unwind the same way.  What is left: the lexer's core loop (`cont` / `string` / `next`,
≈ 23 %), one `String` allocated per token (`LexItem::Token(String)` — the representation the
compiler's parser shares, so an interned `&'static str` token is a front-end change with its
own compile-time win), `walk_parsed_into` cloning the type's `Parts` (the field list with its
names) per value walked to escape a borrow, and the two passes themselves: a schema-directed
parse straight into the store layout (one lexer still, @PLN109) is the structural lever that
removes the `Parsed` tree.

**What the lexer and the parse still pay** (the row alone at `f7f23c5b5`, 7.44×; self time,
rustc-vs-twin.md § Method notes).  The lexer's own functions are 31 % of the row before
their allocations, the JSON parser's 17 %, the tree and its walk about 10 %, and `malloc` /
`free` 24 % shared between them.  None of these is priced.

| lever | what it removes | share of the row |
|---|---|---|
| J4 | BUILT (`lexer::LineCursor`) — a line was decoded into a `Vec<char>` the scan walked through a `Peekable`; the cursor reads the line's own text at a byte offset.  The `String` per line that `Lexer::from_str_with` cuts is still made | measured: the row −2 % (instructions −1.0 %, cycles −1.2 % on the lane), the `Vec<char>` share exactly; a cold compile −0.6 % instructions, cycles level; the cold stdlib parse 293,580 → 267,472 allocations |
| J5 | BUILT (`Lexer::peek_offset`) — the JSON reader mapped each token's line and character column back to a byte offset, through a table it built by reading the whole text once more before the parse.  The lexer notes the byte it reads each token at; the reader asks it, and the table (`ByteMapper`) is gone | measured: the row −10 % on an efficiency core (`byte_of` 4 %, the table's pass over the text 6 %) |
| J6 | BUILT (`Lexer::decimal`) — a number was scanned twice: the lexer's `get_number` built a `String` of the digits loft's grammar accepts (and a `Vec` of group sizes), `JParser::scan_number` read the raw bytes again by JSON's grammar, and `advance_past` stepped the lexer until it had caught up.  Decimal digits alone are folded straight off the line with nothing allocated, and the JSON reader takes the token's value where JSON reads the same digits as one integer — no leading zero, directly after its sign; every other spelling is read from the raw text as it was | measured with J7 |
| J7 | BUILT (`LineCursor::plain_run`, `Lexer::peek_plain`) — a string was scanned twice and copied once more: `Lexer::string` pushed it a character at a time, `json_string_validate` walked the raw bytes again, and `take_string` cloned the token's text.  A literal's plain text is copied as one slice into one allocation, the lexer says when a token is its source text (`peek_plain`) so a plain string is not validated again, and its text is taken from the token | measured, J6 and J7 together: the row 9.54 → 7.89 ms (−17 %, 6.18× → 5.11×) on the fastest core and 17.60 → 13.38 ms (−24 %) on an efficiency core; the lane's instructions −11.5 %; the cold stdlib parse 231,077 → 228,369 allocations, the medium corpus 664,019 → 660,253 |
| J8 | BUILT — `LexItem::Token` names the lexicon's own text (`&'static str`) where it owned a `String` built per token: a one-character ASCII token is read off a table (`Lexer::tok1`), a longer one and a keyword off the set, and nothing is allocated, copied or freed for it | measured: the row 11.06 → 9.67 ms (−12.6 %; the lane's instructions −4.7 %, cycles −5.9 %); a cold compile −0.7 % instructions and cycles; the cold stdlib parse 267,472 → 231,077 allocations, the medium corpus 699,885 → 664,019 |
| J9 | BUILT (`Stores::read_into`, `Stores::record_from_text`) — the `Parsed` tree between the text and the store (decided: no intermediate format — worklist.md § 6).  A collection is filled from an array element by element and a record from an object member by member, on the same reader steps `json::parse` builds its tree with; the fields a member named are a bit each where they were a hashed set of names, and the type's field list is read in place where it was cloned per value.  A value under an enum, and one whose shape its type does not have, is still read as a `Parsed` | measured: the row 7.89 → 5.30 ms (−33 %, 5.11× → 3.41×) on the fastest core, 13.7 → 9.95 ms (−28 %) on an efficiency core; `malloc` / `free` 20 % → 5 % of the row |
| J10 | BUILT (`Lexer::put`, `Lexer::scan`) — what a token cost whatever it was.  A scanner handed its `LexResult` back by value: written to one stack slot, copied to a second in `cont`, copied into `peek`, each copy read before its own writes had settled (half of `cont`; the same at the end of `string`, and `string` re-read the position a step had just written).  Every scanner writes `peek` itself, releasing the token before first; the comment test runs only where a token is produced; a word is looked up as a keyword while it is a slice of the line | measured: the row −8 % on an efficiency core; with J5 4.15 ms, 3.40× → 2.68× on the fastest core (−22 %); the cold stdlib parse 228,369 → 223,493 allocations (a keyword no longer copies its text) |

J6 and J7 rest on J4: a plain run is a slice of the line the cursor stands in.  Neither moved
a scan into the JSON reader — the lexer still reads every string and every number, and
JSON's grammar still owns every refusal; the reader only skips its second look where the
lexer's token is the source text.  J4, J6, J7 and J8 change the lexer every compile runs, so
they are measured on the front-end allocation pins as well as on this row.

**What is left** (the row alone at 2.68×, self time).  The lexer is 36 %: `scan_once` 15 %,
`string` 10 %, `cont` 6 %, `put` 5 % — no one instruction stands out in any of them now.
The store side is `read_struct` 6 %, `read_into` 6 %, the leaf write 3 %; `"{m:j}"`, the
other half of the row, about 12 %.  Forcing `put` inline was measured and is slower.

**How a change to the lexer or the JSON reader is checked.**  Two byte-for-byte comparisons
of the build before against the build after, neither of which a unit test replaces.  For the
compiler: `loft introspect` (IR, bytecode, diagnostics) of every file under `tests/scripts`,
`tests/docs`, `tests/comparisons` and every `bench/*/bench.loft`, both binaries run from one
directory — the dump prints the stdlib's path.  For `T.parse`: a Rust program over
`loft::json::parse_with` that prints the whole result (`{:?}`) for each of a set of seed
documents and a few thousand mutations of each, drawn from a fixed generator, in both
dialects; it is compiled with `hand_price.sh` against each build's `libloft.rlib`.  Most of
the mutations are refused, which is the point: the messages and byte offsets `x#errors`
reports are the part of the contract no bench row reads.

`T.parse` itself is compared one level up, as a program sees it: a loft program reads each
of some tens of thousands of generated documents as each of nine types — records, keyed
collections, enums with a payload, nullable elements, narrow integers — and prints the
value both ways with its `#errors`; the documents are shaped after the types with a share
of wrong shapes, unknown and repeated keys, and a fifth of them damaged.  Both builds run
it under `LOFT_POISON_CLAIM=1`, so that a byte nothing wrote reads the same in both builds
instead of whatever an earlier parse left there.

**A read writes every byte it makes reachable** (`@FR-H-Claim`, loft#1995) — the defect
that comparison found, on both forms of the parse.  Two records are written by no prefill,
only by the members that name their fields: an enum's payload and a nullable record.  A
variant named by its bare tag (`Shape.parse("Circle")`) answered the previous parse's
payload with no error, and either record left by a member that did not fit kept the
earlier value's fields, a collection's handle among them.  Both take their fields'
defaults before their members are read now (`Stores::write_absent_fields`).  The count is
exact and cheap to take again: the results that differ between a `LOFT_POISON_CLAIM=1` and
a `LOFT_ZERO_CLAIM=1` run of the program above — none of 270,000, where 360 did.

**What J4 taught.**  `Lexer::next` and `cont` were read as the cost of the character iterator;
they are the cost per TOKEN.  With the cursor in, their shares are what they were, and their
hottest instructions are the token's `String` being built and the result moved — J8.  The
first form of the cursor was 4.5 % SLOWER on the row with fewer instructions: its step was
not inlined (a call per character), because the decode of a wide character sat in it.  That
path is out of line now (`decode_wide`), and the step inlines at every scan site.

### dryopea `mapfile_to_painted` — a store per insert

Same algorithm as the twin: per ground entry a palette scan (≤ 16 text compares) and a hash
insert keyed on `(q, r)`.  Profile: the keyed insert 53 %, the row's own sorted walk of the
result 15 %, the palette scan 12 %.

| lever | what it removes | priced |
|---|---|--:|
| P1 | `paint`'s `w.painted[q, r] = PaintedHex { … }` mints a STORE per call, writes the record, copies it into the hash (`set_keyed` → `insert_keyed_copy_at`) and frees the store.  Priced with the scratch store minted once (the copy kept): | 2.26 → 1.84 ms (−18 %) |
| P2 | the same insert built IN its hash slot (`(R-InPlaceLiteral)`'s keyed clause): the store and the copy go; the find-then-insert pair of probes stays (the removal and the link) | **built**: 2.26 → 1.69 ms (−25 %) |
| P3 | the palette scan reads each name through the generic `store.get_str` and `op_eq_text`; through the record address (`(R-RecPtr)`'s text read) it is a slice compare | unpriced, ≤ 12 % |
| P4 | the row's `for ph in pw.painted` sorts the hash into a scratch first — loft's iteration order is by key, the twin's `HashMap` order is arbitrary.  A walk whose body only accumulates is order-free and need not sort, but integer overflow makes `+` order-sensitive, so the rewrite needs a range proof | unpriced, ≤ 15 % |

**Re-profiled at `e218a2d48`** (6.58×; callgrind with `--toggle-collect` on `c_mapfile_to_painted`,
21 calls, hashes equal).  Per call, loft 25.6 M instructions against the twin's 3.95 M — the
same 6.5× as the time, so the gap is work, not stalls.  Each call reads 5 000 entries and
inserts 4 615 new keys (all distinct, the table grown from empty).  The twin column is the
twin before @C151, walking its `HashMap` in table order; the key-order twin is the next
section's baseline:

| part | loft | twin | what loft does that the twin does not |
|---|--:|--:|---|
| the keyed insert (`paint`) | 17.5 M (68 %) | 3.13 M (79 %) | |
| — lookup before the insert (`keyed_place_begin` → `Stores::find`) | 2.84 M | — | a miss answered, then forgotten: the probe below repeats it |
| — record claimed in the arena (`record_new`, `arena::alloc`) | 2.68 M | — | the twin's table holds the value inline |
| — second probe (`probe_for_insert`: hash again, compare through `Entries::at`) | 3.52 M | ~1.3 M | |
| — growth (`grow_if_full` → `rehash_into`) | 4.52 M | 1.81 M | 8 975 entries rehashed (twin ~7 200), ~470 instructions each against ~250: every key is re-read from its record and rehashed through the key description |
| — the rest (field writes, `set_key` writing `q, r` again, `record_finish`, the `keys.clone()` per insert: one `malloc`/`free`) | 2.9 M | — | |
| the walk `for ph in pw.painted` | 4.55 M (18 %) | in 0.42 M | key order: `n_hash_sorted` sorts the entries (`sort_records` 9 %), then `OpStep` / `step_ordered` |
| `palette_index_of` | 1.73 M (7 %) | 0.40 M | per element a `get_vector_hoisted` it does not need, `rec_text`, `op_eq_text` |
| the two loops' own code (`mapfile_to_painted` is inlined) | 1.77 M (7 %) | in 0.42 M | the loop over `m.ground` is not header-held (the body calls `paint`, which writes stores): `get_vector`, `length_vector` and a store resolve per field read |

The hash function is not the gap: `keys::hash` is ~170 instructions a call against the twin's
SipHash at ~235.  The gap is how many times the work is done: each new key is hashed and
probed twice and claimed as a record, and each rehash goes back to that record.  What the row
needs to change is the next section.

### What `mapfile_to_painted` needs to be under 3×

The twin walks its hash in key order since @C151 (`by_key`: a sorted snapshot, as loft's), so
it runs 5.46 M instructions per call (0.36 ms) where it ran 3.95 M, and the row reads **4.76×**
(`portal.py measure --routine`, hashes equal), out of the release cut.  3× is **16.4 M
instructions per call** (1.08 ms), from 25.6 M: the row must drop **36 %**.  The time follows
the instructions here (the same IPC on both sides), so the budget is kept in instructions.  Every figure below is read off the profile above (*est.*), never
hand-priced; a profile estimate has paid about two thirds once built (worklist.md § 0), so
the list is ordered to price the two levers that decide the outcome first.

**The frame: C105 holds.**  The twin's `HashMap` keeps each value inside its table and
probes by a stored hash fragment.  Both are declined for loft's hash (@C105: two random
reads per lookup, no hash in the slot, no entries in the bucket table, because entries that
move on growth break the `DbRef`s a program holds into the collection).  Nothing below needs
C105 reopened.

| # | change | what it removes | est. saving per call | needs |
|---|---|---|--:|---|
| K1 | `(R-KeyedEntry)` extended to the subscript-assign spelling `h[k₁, k₂] = T{…}` and to a compound key of two to four integer parts (the key handed over as an array, as L8b's compound clause does) | `keyed_place_begin`'s lookup (2.84 M), the second probe's re-hash and record compare, `arena::index_of`, `record_finish`'s bookkeeping, `set_key` writing `q, r` a second time, the `keys.clone()` per insert | 6.5 M | a both-shapes admission in `generation::keyed_entry`, and a compound `hash::entry_*` |
| W | the key-order snapshot made as the twin's is: each entry's key parts read once into the sort array, without the key description per entry, and the walk stepped over the array without `step_ordered`'s per-step dispatch | loft's walk 4.55 M against the twin's `by_key` 1.55 M | ~2 M | runtime only (`records_sorted`, `build_ref_scratch`, `OpStep` on a hash scratch) |
| K3 | the table reserved before the loop: at most one insert per pass into a hash that is empty where the loop starts (`mtp_out = painted_empty()` just before), so `hash::reserve(len(m.ground))` | every growth: 238 rebuilds over 21 calls, 4.52 M | 4.5 M | the insert seen through the call to `paint` (a callee summary *inserts at most one entry into `w.painted`*, or `paint` inlined in the IR) |
| K2 | (instead of K3, where no reserve is provable) a rebuild typed by the key's shape: integer parts read straight from the entry and hashed without the key description | about half of each rebuild's ~470 instructions per entry | 2.5 M | runtime only, both backends |
| L | the loop over `m.ground` header-held across the call to `paint` | `get_vector` + `length_vector` per pass and a store resolve per field | 1.0 M | the same callee summary as K3, answering that `paint` writes no store `m` lives in, or `(R-Alias)`'s versioned run-time store compare |
| P | the palette scan (P3): no `get_vector_hoisted` for an element whose only use is its name, the length tested before the text is resolved | about half of the scan | 0.9 M | `(R-RecPtr)`'s text read, without the unused element reference |

**Where it lands.**  K1 + K3 + W + L + P remove 14.9 M: **10.7 M per call, 2.0×** if every
change pays in full, and 2.6× if each pays two thirds.  With K2 in place of K3, 12.9 M, 2.4×
in full and 2.9× at two thirds.  K1 alone is 6.5 M of the 9.2 M the row needs, so it is the
first thing to hand-price: the one-probe insert hand-written into the emitted `n_paint`.  The
margin past the list is the arena claim on a new key (`arena::alloc` and its zero fill,
1.4 M; the allocator's free list) and the division in `home_bucket` (keyed.md § What is
left, 7–10 % of a lookup).

**Built: 2.81×** (`portal.py measure --only 08,15,16,17,18`, hashes equal; 1.712 → 1.015 ms
per call, −41 %).  Each step was priced pinned on one core against the build before it:

| step | per call | what it bought against the estimate |
|---|--:|---|
| — the row before | 1.681 ms | |
| K1, hand-priced: `n_paint` calling `Stores::keyed_entry_ints` (one probe for both parts, the parts written on a miss) in place of `keyed_place_begin` / `keyed_place_finish` | 1.123 ms (−33 %) | more than the 25 % read off the profile |
| K2: `rehash_into` filing an integer-part key from the parts read off each entry (`rehash_ints`) | 1.097 ms (−2.3 %) | a quarter of the estimate: what a rebuild costs is the walk of the new table and the store's checked reads, not the key description |
| the arena: `arena::alloc_at` answering the address with the index, a slot at the append cursor not zeroed again | 1.056 ms (−3.7 %) | the margin item named below the table |
| K1 built (`(R-KeyedEntry)`'s subscript clause, which also drops the literal's key-field writes) | 1.029 ms (−2.6 %) | the built form beats the price: it writes one field where the price wrote three |

W, K3, L and P are not built; the row is under the bar without them.  The runtime half moved
the keyed class with it — `hash_fill` 3.02 → 2.58×, `composite_hash` 4.43 → 4.06×,
`hash_text_keys` 4.63 → 4.34×, `hash_remove` 3.34 → 3.07×, `grouped_fill_find` 3.92 → 3.72×
— and no routine of the five lanes slowed beyond its noise (`emit_to_material`, which makes
no hash operation, +1.5 % pinned: the relinked runtime's layout).

**The walk keeps key order (@C151).**  `for ph in pw.painted` is the dryopea source's own
loop, so the bench keeps it, and W makes the key-order walk cheaper rather than giving the
order up.  A program that has no use for the order writes `for ph in unordered(pw.painted)`,
which skips the sort; that reaches this row only if dryopea's `save.loft` writes it, and then
the twin walks `.values()` for that loop.

**Both backends.**  K3, L and P are IR facts (a reservation, a callee summary), so the
interpreter gets them too; W and K2 are runtime.  K1 is a native emission, as
`(R-KeyedEntry)` is today.

**Who else moves.**  K1 reaches every `h[a, b] = T{…}` of a heap-free element:
`reload_and_record` (its compound-key probe is ~32 % of that row, now 3.65× against the
key-order twin), `truncate_to`'s history writes, `field_add_cell`.  W reaches every key-order
hash walk, `reload_and_record`'s four among them.

### zttext `invert` — what it needs to be under 3×

The row read 6.28×: 8.00 ms per call against the twin's 1.27 ms, so 3× is 3.82 ms, a 53 %
cut.  Per op (300 a call: an insert, a delete or a restyle in turn, over a 1 000-piece,
20 000-character document) `invert` computes the op's inverse group against the current
document, prepends it to the result, and applies the op.  The twin MOVES the buffer on a
delete and a restyle (`Doc { buf: d.buf, pieces }`), prepends by `splice` (no copy of an
earlier op's text), and pushes a `Piece` as 24 bytes.

**Priced, not profiled.**  The callgrind split put `memcpy` at 23 % of the row and the
buffer copies first; in time the buffer move is the SMALLEST lever.  Callgrind counts every
iteration of `rep movsb` as an instruction, so a block copy reads far above its time — this
row's levers are ordered by hand prices only (`perf` with LBR call chains, restricted to
`n_invert`, for the time split; each price pinned on one core, three interleaved runs,
hashes equal):

| # | lever | alone | in sequence | what building it takes |
|---|---|--:|--:|---|
| L1 | `cur = apply_op(cur, op)` hands `cur` as the callee's buffer, through `apply_op`'s chain exits (`return delete_range(d, …)`): the literal's `buf: d.buf` is a self-replace | −6 % | 8.00 → 7.53 ms | `(R-Rebind)` across a chain exit; the chain's delivery must take a result in the parameter's store as the buffer it is — without that, `OpRebindRecord` deep-copies it and the row is **8 % slower** |
| L2 | the prepend `tmp += g; tmp += result; result = tmp` done in `result`'s own record | −24 % | 7.53 → 5.53 ms | **BUILT**: `(R-Compact)`'s prepend form, `OpPrependVector` (both backends): 6.48 → 4.48 ms on top of L3 (−31 %, more than the price: the build appends then rotates once, where the price inserted per element) |
| L3 | `np += [p]` of a 24-byte `Piece` view as its bytes | −19 % | 5.53 → 4.10 ms | **BUILT**: `(R-FlatCopy)`'s emitted clause, 7.99 → 6.59 ms on the row as it was (−17.5 %, the hand price) |
| L4 | `slice_runs` holds `d.pieces`' header across its loop | — | 4.10 → 3.43 ms (−16 %) | **BUILT**: `(R-Header)`'s apart clause, its parameter half, in `(R-Alias)`'s versioned copy (the return buffer tested apart from `d` at run time; `buf_slice_text` a callee that writes no store): 4.51 → 3.75 ms on top of L3 and L2 (−17 %) |

**Where it lands.**  All four, priced: **3.43 ms, 2.70×**.  Without L1, about 3.65 ms (≈ 2.87×).
With L3 and L2 built: **3.60×** in the portal (4.45 ms against 1.24 ms; `make perf-portal` at
164252e78), and `insert_text` 5.09 → 1.93× beside it from L3 alone.  With L4: **2.97×** in the portal (3.72 ms; `make perf-portal` at 1d7ec5904) — under the bar, with
L1 (about −6 %) the margin left to take.  The same run read
the text lane (13) 10–23 % slower; the emitted Rust there is unchanged, and with
`-align-all-functions=6` on both runtimes the two builds are level — the relinked runtime's layout.
L3 and L4 are exactly the emitted forms, so they land at their price, as L3 did.  The
hand-priced files are the emitted `invert` with each edit applied by script
(`hand_price.sh` with the package's `LOFT_NATIVE_RUSTC_ARGS`); the edits are the four
`python3` replacements recorded in the commit that built L3.

### dryopea `truncate_to` — what it needs to be under 3× (analysis, not priced in full)

6.38× at 1d7ec5904: 86.9 µs per call against the twin's 13.6 µs, so 3× is 40.9 µs (−53 %).
300 `history_push`es on a 50-deep timeline; `(R-Compact)` already admits both rebuilds
(`truncate_to` and `drop_oldest` are one in-place keep each), and the driver's `e` is a loop
record refilled in place, so neither the rebuild nor the entry's build is the cost.  Callgrind
(no large block copies here, so the instruction split stands), per call 1.47 M: `history_push`
73 % — the entry copied into the timeline (`OpCopyRecord` → `copy_plain_vectors`, two vector
blocks claimed and copied) 31 %, the dropped entries released (`keep_vector_range` →
`remove_claims` → `delete`) 21 %, the append slot (`OpNewRecord` with its default prefill,
`OpFinishRecord`) ~13 %, its own code 9.5 %; the allocator inside all of them (`claim_best_fit`,
`delete`, `set_free_header`, `fl_insert`) ~25 % of the row.  Per push that is about 3 600
instructions inside `history_push`; the twin's whole iteration is ~45 ns.

**The arithmetic.**  Outside `history_push` (the entry's build, the final walk) is 27 % of the
row, ~23 µs.  Under 3× leaves `history_push` ~18 µs for 300 pushes — ~60 ns a push against
~210 ns now (−72 %).  No one lever comes near that:

| lever | price | status |
|---|--:|---|
| the append slot minted without its default prefill (`OpNewRecordNP`: the whole-record copy overwrites every byte) | **−6 %** (86.9 → 81.6 µs, hash unchanged) | priced by hand on 1d7ec5904's emission; `(R-CompleteWrite)` does not count a whole-record copy as covering the slot |
| the allocator's small-block free list (worklist.md § 4) | −12.8 % | priced at d88f8556d, probe undone: needs a parked-block state every store walk knows |
| the dead entry MOVED instead of copied (X10) | +16 % | priced and declined (worklist.md § 4) |
| a typed copy and release of a record of plain vectors (no generic dispatch per entry) | *est.* ~10 % | not priced |

Together ~−30 %, about 4.4×.  What the twin does that loft does not is pay a `malloc`/`free`
(~30–50 instructions) where loft pays a best-fit claim (~280) and a delete with coalescing
(~170), twice each per push, plus a generic copy and release walk around them.  The row
reaches 3× only when a claim and a delete of a small block cost what the twin's do — the
free list built lean (a size-class pop and push, ~50 instructions) — AND the copy and release
are typed per entry.  Next: build the free list (it also moves `panel_build` −15 %, `map_json`
−4 %), take the prefill, then re-profile; the shares above assume none of them.

### pluginabi `check_request` — the decode it shares, and the texts

`(R-Destination)` took it 3.87 → 3.11 ms (12.0× → 9.4×).  **`(R-ViewReturn)` built** (211d5055e):
the routine's total 62.6 → 53.7 ms alone on one core (−14 %, against −10 % priced).  What was
left before it is the § above: `pa_get` answered a deep copy of the entry it found, its key
compares copy each key, `pa_text` copies the value twice more, and `decode` still claims,
fills and frees four texts and two byte strings per frame the twin never materialises —
`(R-DecodeView)` (@C141).

**`(R-DecodeView)` priced on today's tree** (`pluginabi_check_request/price_decode_view.py`):
every decoded text and byte string held as a span — offset and length — of the frame, in the
spare word of its 16-byte `CborValue` record; `decode`'s twins write the span where the owned
form claims, and `pa_text` / `pa_get` read the frame in place.  1255 → **680 ns per check
(−46 %)** on one core, every row's hash unchanged: the byte strings alone −26 % (no reader
needs them), the texts alone −20 %.  **Built** (instance 1, `formal/rewrites.md` § A decoded text or byte
string is a view into the frame): 2.55 → 1.67 ms per op, 7.55× → 5.07× recorded.  Of the 20 %
of the price it leaves, the parts measured: the key bindings copied into a `String` (−1.5 %
borrowed, S6) and the span helpers' frame lookups (−1 % with one lookup per call, taken).

### Order

J1–J3 first: runtime-only, both backends, every JSON parse, −21 % priced, and no rule.  Then
P1/P2 — the temporary-record mechanism C125 names, at a keyed destination (it reaches every
`h[k] = Record { … }`).  Then `(R-ViewReturn)` and `(R-DecodeView)` on `check_request`.
