# zttext `invert` — hand-priced (gcc ship, tip 3cfb11fb9)

Driver: `pkg/` (both mains cut to `bench_invert`), CPU 8, `m.sh`, per op = (N=200 − N=1) / 199.
Hash `896d8de` at every step.  Patches `step1.py`, `step2.py` (cumulative), `only2.c` (step 2 alone).
The box was loaded by other agents: the twin's own timer swung 1.0–2.5 ms, so CYCLES (and
instructions) are the evidence here, not the printed time.

## 1. Baseline

| | instr / op | cycles / op |
|---|--:|--:|
| Rust twin | 20.2 M | 5.50 M |
| C ship, gcc | 48.5 M (2.4x) | 14.2 M (2.57x; stats.py by time 3.05x) |

The routine: 300 ops (Ins / Del / Sty in turn) on a 1 000-piece, 20 000-character document;
per op the inverse group (`slice_runs`), a prepend, and `cur = apply_op(cur, op)`.

## 2. Profile (baseline)

| % | leaf | what |
|--:|---|---|
| 22.9 | `n_insert_text__fr` | the piece loop and the buffer copy |
| 21.6 | `memmove` | the 80 KB character buffer: copied into every Del/Sty result (`Doc { buf: d.buf, … }`), twice per Ins |
| 14.9 | `loft_copy_record` | `np += [p]` in `insert_text`: one out-of-line record copy per piece (1 000 per Ins) |
| 9.6 | `loft_copy_claims` | called from that copy; returns at once (Piece owns no heap) |
| 8.7 / 8.4 | `n_set_style__fr` / `n_delete_range__fr` | their piece loops (already a push window) |
| 7.9 | `n_slice_runs__fr` | the inverse group's runs |

## 3. Steps

| step | edit in the C | instr / op | cycles / op | x twin (cycles) | Δ cycles |
|---|---|--:|--:|--:|--:|
| 0 | as emitted | 48.5 M | 14.2 M | 2.57x | |
| 1 | Del / Sty answer IN the handed-over document: `d` dies at the call (`cur = apply_op(cur, op)`), so the pieces are replaced in `d`'s record and `d` is the result — the buffer is not copied; `apply_op` and `invert`'s rebind pass it through | 48.3 M | 12.8 M | 2.33x | −9.3 % |
| 2 | `np += [p]` of a heap-free record (`Piece`, 3 integers): its 24 bytes copied from the address the loop holds into the pushed slot, no `loft_copy_record` call | **28.7 M** | **6.66 M** | **1.21x** | −48 % |
| 2 alone (on step 0) | | 28.9 M | 8.01 M | 1.46x | −43 % |

Step 2 removes ≈196 instructions per piece: the call itself, its store resolutions and
`copy_claims`' early return, and the spills an opaque call forces on the push-window loop.
After step 2 the routine is at 1.21x the twin by cycles (1.42x by instructions) — under the 2x
target with room to spare; no further step was needed.

## 4. The mechanism behind each step

| step | general mechanism | rule / where | native? | safe when | shared shape |
|---|---|---|---|---|---|
| **2** | **a record copy of a heap-free type is its block copy** — emitted inline as a `memcpy` from the held source address, not a runtime call | **`(R-FlatCopy)`'s emitted clause** (`formal/rewrites.md` § R-FlatCopy): native emits it; the C still calls `loft_copy_record` (3 sites in this unit alone) | yes | the type owns no heap (`owns_heap == 0`, a compile-time fact) and the copy keeps its source | every `v += [rec]` / `v[i] = rec` of a plain record: zttext `insert_text`, `delete_range`, `set_style`, `apply`, `patch`; any piece-table or geometry vector |
| 1 | a function whose result is built from a field of a parameter (`Doc { buf: d.buf, … }`), called where the argument dies (`x = f(x, …)`), answers in the argument's record: the unchanged field stays, the rest is replaced — through a forwarding callee (`apply_op` → `delete_range`) | NEW (a callee half beside `(R-RebindBuffer)`, which only recycles the caller's store; `(R-ExitVector)` moves a LOCAL vector, not a parameter's field) | no | every call site of the twin passes a local that dies at the call and is not borrowed; the result's other fields are fresh | zttext `apply_op`/`apply`/`delete_range`/`set_style`/`insert_text` (the Ins half would also skip one of its two buffer copies); any value-semantic edit of a document |

Not taken: `insert_text`'s second buffer copy (nb into the return buffer) — `(R-PlaceReturn)`
was built and measured SLOWER in its general form (c-libraries.md § R-PlaceReturn), and the
`insert_text` row is another stream's.

## 5. What is left at 1.21x

`insert_text`'s two 80 KB copies per Ins (the twin clones once), and the per-op `slice_runs`
store work of the inverse group (the `(R-Transient)` shape priced in `../flow_layout_full`).
