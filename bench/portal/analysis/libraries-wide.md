# The libraries, measured wide — the first picture across eleven packages

Taken 2026-09-24 on x86-64 at `08204b85b` (`make perf-portal`, the packages alone). Each
library's bench lives in its own repository on branch `laptop-perf-bench`: `<pkg>/bench/bench.loft`
calls the real API, and `bench.rs` is a plain-Rust twin that prints the same hash. This was the
owner's WIDE direction (PERFORMANCE.md § Wide before deep). Every class now has a library row, so
the portal says where native stands for the code consumers actually call. Before this, the only
library measured was drawing, which had been optimised row by row.

This is a MEASUREMENT, not an analysis. Nothing below was hand-priced. The "visible reason" column
is what the bench's author saw in the library source, and it is a hypothesis to price. It is not
an attribution.

**32 rows, median 3.80× of Rust** (drawing alone: 1.48×). 3 rows are within 2×, 28 are over, and 1 is unclear.

| class | rows (ratio) | visible reason |
|---|---|---|
| call | text2d `write_text` **81×**, tween `value` 5.4×, fixstep `clock_pump` 4.2× | `write_text`: `face_codes()` / `face_rows()` return vector LITERALS, rebuilt (56 + 392 elements) for every glyph. That is a constant table built per call, not the call frame. The other two are chains of 3 small calls around a record, and rustc inlines the twin's into one. |
| text-build | html `escape_html` 10.3× / blob 11.6×, markdown `html_escape` 8.6×, `render_inline` 6.7×, `render` 4.1×, time `format_iso` 3.5× | the per-character append: `for c in s { out += "{c}" }` builds a 1-character text per character. The stdlib text-build rows are 1.2×, so the gap is in this SHAPE. |
| keyed | gridmesh `build_index` **16.3×**, `field_add_cell` 10.0×, `collect_dirty_inputs` 3.4×, text2d `atlas_cell` 3.3× (a linear scan) | a record inserted into `hash<CellRef[ck]>`. Library keyed rows run 3–5× worse than the stdlib keyed rows (3.5×). |
| vector-read | text2d `draw_quads` 11.6× | `get_pixel` + `set_pixel` method calls per texel |
| record-field | fixstep `timer_spend` 10.5× | a call per element of `vector<Timer>`, with field writes behind branches |
| vector-build | gridmesh `emit_segment` 7.8× | nine narrow-vector appends per segment, each with a cast |
| record-build | markdown `extract_headings` 6.4×, text2d `layout` 2.5× | a record with built texts appended per item |
| vector-write | random `indices` 6.6× | Fisher-Yates: two nullable reads and two writes per step |
| alloc-temp | markdown `slugify` 4.3× | a 1-character text and its lowercased copy per uppercase byte |
| int-loop | random `get` 3.3×, fixstep `clock_advance` 3.1×, time `day` 1.31×, `iso_week` 1.35× | checked `/` and `%` with record field writes |
| native-boundary | web `pack_u8` 3.1×, `byte_at` 3.0×, ssh `byte_at` 3.0×, random `rand` 2.7×, `rand_indices` 3.5× | about 1.7 ns per crossing on top of the callee. The twin calls the callee body directly. |
| float-kernel | fixstep `approach` 2.2×, tween `ease` **0.94×** | `exp` / `pow` dominate, and the float lane is at parity |
| text-scan | time `parse` 2.0× | `digits_at` walks the whole text per field |

## What it says, class by class

- **text-build is the widest gap the stdlib rows did not show.** The stdlib rows run
  `split`, `join`, `replace` and `format`. The libraries run a per-character `out += "{c}"`
  instead. Six library rows sit at 3.5–11.6×. It is the natural spelling (it is how html,
  markdown, zttext and text2d are all written), so it is the compiler's case to take.
  See feedback: the natural form is canonical.
- **A constant vector literal returned from a function is rebuilt per call.** text2d
  `write_text` at 81× is almost entirely this. The fix belongs to the compiler for the same
  reason as above, but it needs the callers' use proven read-only, so it is not free.
- **Library keyed rows are 3–5× worse than the stdlib keyed rows.** A RECORD value inserted
  under a key costs more than the integer maps `15_stdlib_keyed` measures, and `keyed.md`
  priced only the latter.
- **The native crossing is about 1.7 ns.** It holds near 3× across web, ssh and random, and
  that is the calibration constant for every mixed row.
- **Found on the way:** loft#1657 (`v += [f(v)]` lost what `f` appended; silent-wrong,
  fixed). It is why text2d `wrap` joined the lane late.

## Not yet benched

After wave 2 the census (`bench/portal/census.tsv`) keeps only what a std-only twin cannot
carry (regex, imaging's png, crypto's ed25519 and HPKE), the hex_* packages wave 2 did not
reach (body, draw, fit, form, place, recover, roof, shape), and the consumer rows, which
are modelled in `16_consumer_shapes` rather than benched in a library.

## Wave 2 — nineteen more libraries (2026-09-24, `35c3d39b2`)

55 rows, median **5.60×**.  The whole portal: 156 routines, median **2.75×**; libraries
**3.67×** (102 rows, 62 over 3×) against the stdlib's 1.72×.  Visible reasons, still
hypotheses, grouped by the MECHANISM they share. Each is a candidate lever, and the order
is how many rows it would move:

| mechanism | rows it explains | visible in |
|---|---|---|
| a `u8` / narrow element append takes the general element path (`FUSED_PUSH_KINDS` has no byte or short kind): ~22× a `vector<integer>` push | cbor `encode_bytes` 497×, `encode` 21×, `decode` 28×; pluginabi `check_request` 160×, `req_state_b64` 68× (they run cbor) | `LOFT_TRACE_PUSH_FILL`: "a mint's element does not qualify for the record push" |
| a record rebuilt around a large field, `Doc { buf: d.buf, … }`, deep-copies that field | zttext `delete_range` 138×, `invert` 118×, `insert_text` 95× | the emitted `vector_add` of `d.buf` per edit |
| a small record returned by value gets a store, a fill and a copy per call | mesh3d `mat4_mul` 37×, `mat4_transform` 22×, `sphere` 23×; game_protocol `msg_ping` 11× | `OpDatabase` + `OpCopyRecord` per call |
| a fn-ref's record result is held to frame exit (loft#1659) | zttext `flow_layout_full` 107× | 20,000 live Style stores on native |
| a loop whose body calls declines the push hoist | stage `pack_instances` 25×, mesh3d `mesh_to_floats` 40× | `LOFT_TRACE_HOIST_DECLINE` |
| a view-returning scan through `self` declines the record address | hex_world `get_cell` 13×, hex_field `edgeset_count` 37× | `LOFT_TRACE_RECPTR`: "the remainder may grow a store" |
| per-character text building | zttext `materialise` 23×, `seg` 8.6×, arguments `parse` 7×, server `header` 7× | `out += "{ch}"`, a `split` per lookup |

The native crossing holds at about 1.0–1.1× where the callee does real work (crypto's sha256
and base64). The ~3× of the web/ssh/random rows is a 1-byte callee against the crossing.

**Found on the way, and fixed:**
- a method's inline record result leaked one store per call;
- `@FR-N-Store` checked an element store only at a literal index (loft#1660, owner-ruled);
- `t += t` through a `&text` did not compile on native;
- `+=` on a `&text?` was refused;
- `now()` re-read its environment variable per call.

**Filed:** loft#1659.  **Open:** a fn-ref into an auto-built native library answering null
from a `return <call>;` body (zttext's `token_width`, interpreter only).

**The rules for the copy class** (the 100×+ rows) are written, not built: `(R-Rebind)`,
`(R-Compact)`, `(R-Const)`, `(R-ValueLocal)` in `doc/claude/formal/rewrites.md` § *A value
already home is not copied to a second one*, on the floor `(H-CopySelf)` (heap.md).  Each
names its headline row and the hand-price to take first.

## Wave 3 — the consumers modelled, the rest of hex_*, and par (2026-09-24)

53 rows, median **5.51×**; the whole portal is now 203 routines (median 3.13×, libraries 4.25×).  The consumer lanes
(`17_consumer_moros_dryopea`, `18_consumer_crawler`) model the games' hot loops at their own
layouts; every row but `binary_read` was written with the copy class already named above,
and the numbers say the same thing the rules do: `truncate_to` **207×** (`(R-Compact)`),
`slope_path_with_undo` 20× and `panel_build` 19× (records copied into stacks and panels),
`resolve_move` 10× (`(R-ValueLocal)`), `map_json` 17× (`Map.parse` runs the source lexer).
`binary_read` fell 224× → **33×** with the buffered `LoftFile` (each `f#read(2)` was a
system call); the rest of its 33× is the per-read allocation and the crossing.  `par` holds
0.7–2.4×: the pool is not the problem.  The hex_* rows are the same mechanisms again —
`field_union` 173× and `bone_shape_has` 100× (a vector filled one push at a time, then read
through nullable accessors; three records per query), `edgeset_count` 40×, `doc_read` 17×
(one file-read crossing per cell), `stencil_rotate` 10× — with three twins that run a
CHEAPER algorithm than the library (`bone_shape_has`, `rig_read`, `form_read`, and
pluginabi's decoders from wave 2): those rows overstate loft's share and are to be
re-aligned to the library's algorithm (bench/README.md rule 1), the library's own extra
work listed beside them for its author.

**`(R-Rebind)` hand-priced** on `delete_range`'s emitted Rust (2026-09-24): 61–64 → 4.2–4.6 ms/op
(−93 %), 138× → ~6.5×, hash unchanged — the two whole-buffer copies were the row.  Probe:
the session scratch `rb/dr.loft` (the bench row alone), `dr.rs` → `dr2.rs` the hand edit.

**Twins re-aligned to the library's algorithm (2026-09-24, bench rule 1).**  Four twins had
been written with the census's IDIOMATIC algorithm rather than the library's, so their rows
charged loft for work the library itself repeats: hex_body `bone_shape_has` re-poses the rig
per query (97× → ~4.9×), `rig_read` and hex_form `form_read` re-split each line per field read
(13.8× → ~1.8×, 5.2× → ~2.7×), pluginabi `check_request` decodes its frame twice and
`req_state_b64` three times.  Each `bench.rs` now names that extra work for the library's
author; the re-measured rows are in the results file.  **A twin's own noise floor, measured on
the way:** the pluginabi `request` twin moved ±40 % between two builds of UNCHANGED source —
`b64_decode` byte-identical, at a different address (function alignment 16 vs 32 mod 64);
`codegen-units=1` inverts the order and `-C llvm-args=-align-all-functions=6` equalises them.
`stats.py`'s seven interleaved samples resample one binary and cannot see it, so a ratio near
the bar on a small twin can move a class with no loft cause.  The cure, if wanted, is an
alignment flag in the protocol's `rustc -O` line, not a source edit chasing a layout.

**`(R-Const)` hand-priced** on `write_text`'s emitted Rust (2026-09-24): 117 → 4.3 ms/op (−96 %),
86× → ~3.3×, hash unchanged — the two face tables rebuilt per glyph were the row; the rule's
second half strikes `(O-LazyBuffer)`'s per-call buffer, not a copy.  Emitter waste found: the
fused element write derives `text_span_of` per call for an integer element (9–13 % of what
remains).  Probe: session scratch `rk/`.

**`(R-ValueLocal)` hand-priced** on `mat4_transform`'s emitted Rust (2026-09-24): 45 → 3.1–4.0 ms/op
(−92 %), 22× → ~1.7–2.2×, hash unchanged.  Today's site mints a store and frees the previous one
per call (`(R-ValueRecord)` declines it: the local is also an argument).  The remainder is twelve
loop-invariant element reads of the unwritten matrix per call (an instrument hoisting them:
~1.2–1.5×) — a rule to write.  Probe: session scratch `rv/`.

**`(R-Compact)` hand-priced** on `truncate_to`'s emitted Rust (2026-09-24): 3.2–3.8 ms → 414–435 µs/op
(−87 %), 207× → ~30×, hash unchanged — the rebuild was three deep copies of every kept entry; the
rest is the per-push record construction (`(R-Place)`'s class).  Found on the way and fixed: a
vector FIELD rebind on a multi-field record stranded the old elements' owned heap inside the
store.  Probe: session scratch `rc/` (`compact_vector_in_place` there is the primitive the
runtime lacks).
