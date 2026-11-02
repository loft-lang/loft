# cbor `encode_bytes` (+ `encode`, `decode`) hand-priced to 2x — C ship lane

Tip 3cfb11fb9, `target/release/loft`, unit emitted with `LOFT_RELEASE_PASS_PROBE=1`, one driver per
routine (`pkg/bench/b_{bytes,encode,decode}.loft` / `.rs`, main cut to one row, hashes unchanged).
Counts: `pi.sh` = (n=80 − n=40)/40 per op, pinned to CPU 10, `cpu_core` PMU; cycles are the
minimum of 3 (`final_*.txt`). Other agents ran on other cores: instructions are exact, cycles ±10 %.

## Baseline (cycles / op, ratio to the twin)

| routine | twin | gcc | × | clang | × |
|---|--:|--:|--:|--:|--:|
| encode_bytes | 283 K | 1 236 K | 4.37 | 434 K | 1.54 |
| encode | 5.71 M | 15.95 M | 2.79 | 11.12 M | 1.95 |
| decode | 6.94 M | 19.86 M | 2.86 | 9.27 M | 1.34 |

Instructions are only 2.0× the twin's on `encode_bytes` (2.54 M vs 1.28 M) — gcc's gap is
CYCLES: IPC 2.0 against the twin's 4.5, top-down 65 % backend-bound with ~no cache misses.

## Profile (gcc, `encode_bytes`)

`n_encode__ap__fr` 36 %, `n_head__ap__fr` 22 %, `n_encode__fr` 18 %, `make_texts` 6 %, memmove 4.5 %,
`loft_win_of` 2.3 %, the store mint+free of an unused buffer (`loft_database`, `st_init`,
`claim`, `bits_insert`) ≈ 4.4 %.  The hottest instruction in each of the three top functions is
one load: `movq -0x3c(%rbp)` (40 % of `head__ap__fr`), `mov -0x1ac(%rbp)`, `mov -0x1f0(%rbp)` —
an 8-byte load at an ODD offset, 4 bytes into a spilled `loft_ref`.

**Cause: a store-forwarding stall on every `loft_ref` that goes through memory.**  `loft_ref` is
`{uint16 store_nr; uint32 rec; uint32 pos}` (12 bytes); the SysV ABI passes it as two eightbytes,
`store_nr|rec` and `pos`, and gcc spills it that way (an 8-byte and a 4-byte store).  gcc then
merges the adjacent `rec` and `pos` reads into ONE 8-byte load at offset 4, which spans both
stores: the forward fails and the load waits for the stores to retire (~15 cycles).  It happens
wherever a ref is spilled: a parameter whose address is taken (`&v24___vdb_1` in the cold
`loft_database_room` arm of `head`), a ref returned from a call, a ref built field by field on a
merged path (`loft_vec_get`'s null arm).  clang reads the fields separately and does not stall,
which is the whole of the gcc/clang difference on these rows.

## Steps (cumulative, `encode_bytes`, cycles / op, × twin 283 K)

| step | edit | gcc instr | gcc cycles | gcc × | clang cycles | clang × |
|---|---|--:|--:|--:|--:|--:|
| 0 | as emitted | 2.54 M | 1 236 K | 4.37 | 434 K | 1.54 |
| 1 | `loft_ref` fields reordered `{rec, pos, store_nr}`: `rec|pos` is one eightbyte, so the merged load matches one store (`reorder.py`: 209 positional initialisers made designated first) | 2.55 M | 671 K | 2.37 | 471 K | 1.67 |
| 2 | the outer `encode`'s array arm opens the append window ONCE before the item loop and syncs it after, instead of `loft_win_of` … `loft_win_sync` around every item call (`step2.py`) | 2.13 M | 449 K | 1.59 | — | — |
| 3 | the array arm's dead call buffer dropped: `v23___ref_8` is a store minted (`loft_database_typed`) and freed per arm for the plain call the window twin replaced (`step3.py`) | 2.09 M | 446 K | **1.58** | 421 K | 1.49 |
| 2+3 on the ORIGINAL layout | (control: without step 1) | 2.09 M | 1 071 K | 3.79 | **392 K** | **1.39** |
| ✗ 4 | `head__ap` forced inline into `encode__ap__fr` (the "call overhead" lead) | 1.87 M | 540 K | 1.91 | | |

Step 4 is a LOSS on gcc: −10 % instructions, +15–20 % cycles — the inlined body spills the item's
`loft_ref` (built in two 4-byte stores, read as one 8-byte argument) and stalls again.  The earlier
"call overhead" reading of this row was the store-forwarding stall seen through the call boundary.

## `encode` and `decode` with step 1 alone (cycles / op)

| routine | gcc before | gcc after | × | clang before | clang after | × |
|---|--:|--:|--:|--:|--:|--:|
| encode | 15.95 M (2.79) | 11.23 M | **1.97** | 11.12 M (1.95) | 11.46 M | 2.01 |
| decode | 19.86 M (2.86) | 9.35 M | **1.35** | 9.27 M (1.34) | 11.01 M | 1.59 |

With step 1 gcc lands where clang already is.  clang loses with the reorder (`decode` +18 %
instructions, all inside `read_value__d__fr` — its inlining shifts), so the layout is a
per-compiler choice.  Tried: `{rec, store_nr, pos}` (rec and pos not adjacent, nothing to merge):
gcc only −18 % on `encode_bytes` / −10 % `decode`, clang `decode` still +22 % — rejected.

## The mechanisms (general, ranked)

1. **`loft_ref` layout per compiler** — `src/c/loft_rt.h:1097`.  NEW (not a rewrite rule: the C
   runtime's representation, so no `(R-…)`; C122 — representation is free).  Under gcc
   `{uint32 rec; uint32 pos; uint16 store_nr}`, under clang the current one (`#if
   defined(__clang__)`).  Needs every positional `loft_ref` initialiser in the runtime headers and
   the emitter made designated first (`{ 0xFFFF, 0, 0 }` alone is 104 sites in this unit; also the
   `dst = {…}, src = {…}` multi-declarator lines in `loft_rt.h` and the `loft_ph` initialiser),
   then a layout-flip test (the C test binaries + `scripts/c_sweep.py` under both layouts — a
   missed positional site answers wrong silently, so the sweep is the falsifier).  Safe because no
   `loft_ref` is ever stored as raw bytes (all three hashes identical once the initialisers were
   designated).  Native has no counterpart (rustc lays out `DbRef` itself).  Shares the shape:
   every C row that passes refs through calls — expect it in all the worst gcc rows (`check_request`,
   `request`, `insert_text`, `flow_layout_full`, cbor `decode` above), and it explains why gcc
   trails clang there.  The alternative that serves both compilers is to stop spilling refs
   (no address-taken ref locals in hot paths; `loft_database_room` answering by value) — not priced.
2. **(R-AppendTwin) window clause, loop form** — `src/c/body.rs` `windowed` (line ~6035 emits the
   per-call `loft_win_of … loft_win_sync`).  A loop whose body's only writes to buffer B are
   window-twin calls (`X += f(…)` with f's twin writing through the window) opens the window before
   the loop and syncs after (`open_window` already does this for push loops).  Native: the window
   clause exists for the twin chain; the loop-hoisted open is the C's missing piece.  Condition:
   nothing in the loop reads B's length or elements outside the window (`encode_map`'s value loop
   interleaves plain pushes to B, so it needs those pushes through the window too).  −33 % cycles.
3. **Dead call buffer of a call the window twin replaced** — the buffer `v23___ref_8` minted for
   `encode(items[i])` survives the switch to `encode__ap`; the window path should not reserve it.
   `src/c/body.rs` (the call-buffer reservation beside `append_site`).  −1 to −2 %.

## What is left at 1.58× (gcc) / 1.39× (clang)

`encode__ap__fr` + `head__ap__fr` per item ≈ 55 % (the enum `if`-chain re-reading the tag per arm,
`loft_get_text` + the text view choice, the frame's ~13 ref initialisations and 8 `loft_free_var`
on exit for refs most arms never set), `make_texts` (the setup) ≈ 18 %, the 256-byte copies ≈ 10 %
(the twin copies the same bytes).  Both compilers are under 2× after steps 1–3; the twin's
remaining lead is the per-item call with its prologue, which pays only once the stalls are gone.
