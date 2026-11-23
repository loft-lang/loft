<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 225 — The rust-free census (R0)

What fails when nothing may build through rustc.  Every later phase shrinks this list; R7
starts only when it is empty.

## How to measure it again

```sh
LOFT_NO_RUSTC=1 ./scripts/find_problems.sh --full --bg     # LOFT_NATIVE_BACKEND=c is the default
```

`index_hygiene_clean` fails with rustc on as well (decisions C162 and C168 keep their guards
in the language suite); it is not part of this census.

## The test suite — measured on `tuxedo-pr2617-next` @ b8451bba6

6766 tests: 6592 pass, 174 fail, 173 of them through rustc.  Every failure sampled stops at the same refusal — a
`--native-rust` (or `LOFT_NATIVE_BACKEND=rust`) leg the test runs beside its interpreter and C
legs, often through a shared helper (`windows_rules`, `runtime_logging`, `runtime_errors`).
None is a C or interpreter defect.  That makes R4's work list: each test keeps its
interpreter and C legs and loses its rust leg, or goes when its subject is the writer.

| Test binary | Failures |
|---|---|
| `windows_rules` | 31 |
| `runtime_logging` | 11 |
| `runtime_errors` | 10 |
| `engine_host_kernel` | 10 |
| `ffi_c_library` | 8 |
| `store_persist_loft` | 7 |
| `native` | 7 |
| `lazy_sql_source` | 7 |
| `p254_cache_poisoning` | 6 |
| `locked_writes` | 5 |
| `nullable_ret_buffer` | 4 |
| `multiplayer_v5` | 4 |
| `foreign_bridge` | 4 |
| `store_lifetime_1522` | 3 |
| `prefill_image` | 3 |
| `apart_value` | 3 |
| `adopt_buffer_reuse` | 3 |
| `store_swap` | 2 |
| `serve` | 2 |
| `native_yield_channel` | 2 |
| `native_loader` | 2 |
| `native_checkpoints` | 2 |
| `lib_process` | 2 |
| `ledger` | 2 |
| `leaf_chain` | 2 |
| `invariant_arith` | 2 |
| `html_wasm` | 2 |
| `exit_codes` | 2 |
| `dropped_writes` | 2 |
| `dev_soft_halt_surfaces_overflow` | 2 |
| `callee_disturb` | 2 |
| `web_natives` | 1 |
| `viewer_markdown` | 1 |
| `store_memory_limit` | 1 |
| `store_image_sound` | 1 |
| `ssh_natives` | 1 |
| `server_natives` | 1 |
| `return_field` | 1 |
| `repl_session` | 1 |
| `native_scratch_hygiene` | 1 |
| `n0_fingerprint` | 1 |
| `lift_adopt` | 1 |
| `html_fonts` | 1 |
| `html_embed` | 1 |
| `engine_host_audience` | 1 |
| `codegen_emitter` | 1 |
| `clear_release_field` | 1 |
| `c_emit` | 1 |
| `arc_e_program_cache` | 1 |
| `adopt_first_bind` | 1 |

The full list, one `binary test` per line: [census-tests.txt](census-tests.txt).

## Not yet measured

The language suite, `revalidate-libs` and the corpus under `LOFT_NO_RUSTC=1`; each is added
here when it is run.
