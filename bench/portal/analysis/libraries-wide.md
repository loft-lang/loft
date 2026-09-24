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

The census (`bench/portal/census.tsv`) still lists crypto, regex, arguments, cbor, zttext,
imaging, server, game_protocol, pluginabi, graphics, stage, shapes, mesh3d, glb, assets and
the hex_* family. The next wide wave takes one rank-1 row from each of these.
