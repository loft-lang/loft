# zttext `flow_layout_full` — hand-priced to 2x (gcc ship, tip 3cfb11fb9)

Driver: `pkg/` (bench main cut to `bench_flow`; the twin's main the same), CPU 8, `m.sh`
(`perf stat -r 5`, P-core counters).  Per op = (N=100 − N=1) / 99.  Hash `eb888d85` at every step.
Patches: `step1.py` … `step5.py` (cumulative, base.c → s1.c → … → s5.c), `loo2.py` (leave-one-out).

## 1. Baseline

| | instr / op | cycles / op | ns / op (row) |
|---|--:|--:|--:|
| Rust twin | 22.1 M | 5.57 M | 1.46–1.53 ms |
| C ship, gcc | 127.9 M (5.8x) | 31.9 M (5.7x) | 6.73 ms (4.4x; stats.py 4.32x) |

Document: one piece, 100 000 characters, ≈20 k tokens; `slice_runs` runs three times per token
(`token_width` in `next_line`, both passes of `place_line`).  Per token the twin spends ≈1 100
instructions, the C ≈6 400.

## 2. Profile (baseline)

| % | leaf | what |
|--:|---|---|
| 15.2 | `n_slice_runs__fr` | per run: the char loop into a work text, the returned copy, `ph_new` of a `Run` record |
| 14.9 | `n_place_line__fr` | two passes over the line's tokens, each calling `slice_runs`; resolver + measure per run |
| 13.1 | `n_layout_tokens_from__fr` | `next_line`/`token_width` inlined; box copies out of each placed line |
| 8.1 | `loft_st_claim_inner` | a store claim per run's text (`loft_set_text`) and per `Style` mint |
| 7.5 | `loft_database` | the `Style` store minted per `token_width` call (the lift is freed at return) |
| 5.6 | `n_seg__fr` | `materialise`, then text → `vector<character>`, then tokens (the twin does the same) |
| 5.4 | `loft_bits_insert` | the claims bitset for those claims |
| 3.5 | `loft_database_room` | the runs buffer reset per `slice_runs` call |
| 3.5 | `loft_vec_reached` | |
| 3.3 + 2.9 | `loft_reserve`, `loft_vec_prealloc` | text growth per char, the `Run` vector's room |

## 3. Steps (cumulative)

| step | edit in the C | instr / op | cycles / op | x twin (cycles) | Δ instr |
|---|---|--:|--:|--:|--:|
| 0 | as emitted | 127.9 M | 31.9 M | 5.7x | |
| 1 | `token_width`: `measure` gets the run record's text as a VIEW (no work-text concat, no lift copy) | 122.4 M | 29.7 M | 5.3x | −4.3 % |
| 2 | `token_width`: the resolver's `Style` record KEPT across calls (no mint + free per token) | 113.5 M | 28.3 M | 5.1x | −7.2 % |
| 3 | `slice_runs`: the run text stored from the builder's work buffer — `buf_slice_text`'s returned copy, the move into `s` and its frame wrapper gone | 108.3 M | 26.7 M | 4.8x | −4.6 % |
| 3b | steps 1 + 2 in `place_line` (pass 2's measure copy; its two `Style` lifts kept) | 106.4 M | 25.8 M | 4.6x | −1.8 % |
| **4** | **`slice_runs`' result built OUTSIDE the store**: a frame-kept C array of `{text, style}` whose slot texts keep their room; `token_width` and both `place_line` passes read it — no store vector, no `Run` record, no text claim per run | **49.0 M** | **11.9 M** | **2.14x** | **−54 %** |
| 5 | a `vector<character>` range → text in ONE pass: room reserved once, elements read from the held header's base, ASCII bytes stored directly | 44.1 M | 10.8 M | **1.94x** | −10 % |

Routine timer at step 5: 2.33–2.44 ms against the twin's 1.46–1.53 ms (≈1.6x).

Leave-one-out on step 5: the `Style` records minted + freed per call again (steps 2/3b undone) →
54.6 M instr, 13.8 M cycles (+24 % / +27 %).  Once the transient runs are in, the kept `Style` is
the SECOND lever, worth more than before them.

Steps 1, 3 and 3b are SUBSUMED by step 4 for this routine (their copies sit on the record path the
transient removes); they still price the general mechanisms where a result IS stored.

## 4. The mechanism behind each step

| step | general mechanism | rule / where | native? | safe when | shared shape |
|---|---|---|---|---|---|
| 1, 3b | a `text` argument of a fn-ref call read from a record field is passed as a view, not copied into a work text and lifted | `(R-TextBorrow)`, a fn-ref-argument clause — NEW (`c/body.rs` `borrowed_assign` covers a direct call's tested parameter only) | not checked | the field's store is frame-owned and no argument of the call reaches it (the callee cannot write it) | any `measure(r.str, …)`-style injected callback |
| 2, 3b | a fn-ref's record answer keeps its buffer across CALLS of the frame, not only across loop passes | `(R-FnRefBuffer)` extended across frames: the lift handed down as the runs buffer already is (`__ref_N` parameters), or `(H-KeepRoom)` on the lift | not checked | the target fills the handed store (the third bind case of R-FnRefBuffer) | every layout entry point taking `resolve` (`token_width`, `place_line`, `caret_at`, `sel_rects`, `anchor_at`) |
| 3 | a function whose result is the work text it built in the caller's buffer answers it as that buffer (no return copy) | `(R-TextBorrow)` / the format clause's return half — NEW for an out-param work text | not checked | the caller reads the result before the buffer is reused | `buf_slice_text`, `materialise`, any text builder called per item |
| **4** | **a vector of records built by a call, read by the caller's loop and dropped, is built outside the store** | **`(R-Transient)`, its PRODUCER half for a returned `vector<Run>`** (`formal/rewrites.md`; WIP `src/c/body/transient.rs`, opt-in `LOFT_TRANSIENT=1` — leaves this routine's emission unchanged today) | no | every use of the result reads it (index, field, `len`, a `text` argument by view); none stores it | cbor `read_value` (check_request, transient.md); zttext `token_width`/`place_line`/`char_styles`; any `for r in f(...)` over a fresh vector |
| 5 | a loop appending one character per element of a vector range (`out += "{ch}"`) reserves the range once and reads the elements from the held header's base | NEW: `(R-PushFill)`'s run clause for a text target (the short append is built; the per-element store read and capacity test remain) | not checked | the vector is not grown in the loop (its header is held) | `buf_slice_text`, `materialise`, `doc_chars`, `seg`'s text → `vector<character>` |

## 5. What is left at 1.94x

Profile at step 5: `place_line` 26 %, the transient slicing 23 % (the run text is still a COPY of the
characters; the twin passes `&[char]`), `layout_tokens_from` 18 % (each placed line's boxes copied
record by record into the layout — the twin `extend`s a moved `Vec`), `seg` 17 % (`materialise` +
text → `vector<character>`; the twin does the same work).  Next candidates: the run text as a view
of the buffer (needs a text view over `vector<character>` — `(O-SliceView)` cannot, the encodings
differ), and `PlacedLine.boxes` moved into the layout rather than copied (`(R-Push)`/append of a
whole moved vector).
