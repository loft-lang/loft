# zttext `insert_text` — hand-priced (C ship, gcc, CPU 2, tip 3cfb11fb9)

Driver: `pkg/` with both mains cut to `bench_insert` only.  Same hash `368acdef` at every step.
Files: `base.c` (as emitted), `step1.py` → `s1.c`, `step2.py` → `s2.c`, `s2only.c` (step 2 on base).
`m.sh` measures (ns/op best of 3 at --n 200; perf stat -r 3 at --n 100, whole process).

## Baseline

| build | ns/op | instructions | cycles | × twin (ns/op) |
|---|--:|--:|--:|--:|
| C ship (gcc) | 3,372,900 | 5.24 G | 1.65 G | 4.55× |
| Rust twin | 740,390 | 0.94 G | 0.34 G | 1× |

## Profile (baseline)

| % | leaf | what |
|--:|---|---|
| 33.7 | `n_insert_op__fr` (insert_text inlined) | the piece loop, window slots |
| 29.4 | `loft_copy_record` | `np += [p]` — generic record copy of a 24-byte `Piece` (no heap), ~250 k a run |
| 16.7 | `loft_copy_claims` | the same copy's claims walk, out of line, answers "owns no heap" |
| 13.7 | `memmove` | the 24-byte copies + the return's two `loft_vec_replace` copies |

The buffer copy `for c0 in d.buf { nb += [c0] }` is ALREADY one bulk `loft_vec_add` in the
emitted C — nothing to price there.  The twin also copies the buffer once (`d.buf.clone()`).

## Steps (cumulative)

| step | edit in the C | ns/op | instr | cycles | × twin | Δ time |
|---|---|--:|--:|--:|--:|--:|
| 0 | as emitted | 3,372,900 | 5.24 G | 1.65 G | 4.55× | |
| 1 | `loft_copy_record(…, 103, src, fresh_slot, 6)` of a heap-free `Piece` → `memcpy(loft_rec_ptr(slot), src_addr, 24)` (3 sites) | 816,980 | 1.25 G | 0.40 G | 1.10× | −76 % |
| 2 | `nb`/`np` built in the RETURN BUFFER's store (holder claimed there, not a minted store); at return the blocks are swapped into the record's fields and the old blocks freed, in place of `loft_vec_replace` (a copy) | 668,645 | 1.36 G | 0.32 G | **0.91×** | −18 % |
| 2 alone | step 2 on the baseline | 3,198,960 | 5.29 G | 1.55 G | 4.32× | −7 % |

## Mechanisms

1. **`(R-FlatCopy)`'s emitted clause, ported to the C** — EXISTS for native
   (`hoist::flat_copy`, `src/generation/ops/ref_ops.rs:799`, `Stores::copy_flat_record`); the C
   writer still emits `loft_copy_record`.  Safe: the type owns no heap (schema), the source is
   kept, the destination is a fresh push slot (`COPY_FRESH_DEST`).  The whole gap of this row.
   Shares the shape: every `v += [rec]` / `x = rec` of a flat struct on the C — zttext `invert`,
   `delete_range`, `patch` (rewrites.md cites `invert` −20–30 % for the runtime half); grep
   `loft_copy_record(loft_types, N, …, 6)` with a flat N in any unit.
2. **NEW: a returned literal's vector field built in the caller's buffer** (a local vector whose
   only end is `return T { f: local, … }` is claimed in the return buffer's store and MOVED into
   the field — a block swap inside one store — where today it lives in its own store and is
   copied).  Native copies too (c-libraries.md).  Conditions: the return buffer is ensured
   before the local is minted; the local is not read after the literal nor stored elsewhere; the
   field's old block is released as `loft_vec_replace` releases it; and the destination store now
   GROWS in the function, so any header/record address held into that store (here `h3` over
   `d.pieces`, the same store when `d` is the buffer) must be derived again after a growth — it
   was safe here only because `np` is reserved before the loop.  Worth −18 % after step 1;
   −7 % alone (hidden behind the copies).

## What is left

Nothing over the twin: step 2 is 0.91× (the twin pays malloc/realloc per insert).  Step 1 alone
reaches 1.10× — the row is a (R-FlatCopy) port, the move is a bonus.
