# pluginabi `request` — hand-priced to ~2x (gcc, C ship lane)

Tip 3cfb11fb9.  Driver: `pkg/` (main cut to `bench_request`), `--n 200`, pinned CPU 4,
`perf stat -r 5`.  Hash `16387f10` in every build.  Other agents were running on other cores,
so cycles swing ±10 %; instructions carry the argument.

## 1. Baseline

| build | ns/op | instructions | cycles | × twin (cycles) |
|---|--:|--:|--:|--:|
| Rust twin | 0.78 ms | 3.17 G | 0.715 G | 1.00 |
| C ship | 4.07 ms | 12.15 G | 3.73 G | 5.2 |

## 2. Profile (cycles, base)

| % | function | what |
|--:|---|---|
| 72.0 | `n_base64_to_bytes__fr` | crypto's `b64_decode` inlined: loop 1 (per character: CR/LF/`=` tests, `b64_value`, push into `vals`) ≈ 85 % of it, loop 2 (quads → bytes) the rest |
| 4.3 | `loft_copy_claims` | the decoded bytes copied record-to-record |
| 2.8 | `n_encode__ap__fr` | cbor encode |
| 2.2 / 2.0 / 1.4 | `loft_st_claim_inner` / `loft_store_make` / `loft_bits_insert` | store mints and claims per call |
| 1.8 / 1.4 / 1.1 | `loft_vec_add` / `loft_win_grow` / `loft_vec_append_slow` | the result copies and growth |

Branch misses: C **36 M**, twin **1 M**, almost all on `b64_value`'s range chain and the CR/LF
test.  The twin runs the same range compares; its predictor learns the 16 repeating payloads,
the C's (twice as many dynamic branches, see step 5) does not.

The library decodes each argument TWICE per request: `pa_is_base64(s)` decodes to validate
(`len(base64_to_bytes(s)) > 0`), then `pa_cbytes(s)` decodes again.  The twin decodes once.

## 3. Steps (cumulative)

| step | edit | instr | cycles | ns/op | × twin (cycles) | Δ cycles |
|---|---|--:|--:|--:|--:|--:|
| 0 | as emitted | 12.15 G | 3.73 G | 4.07 ms | 5.2 | |
| 1 | `b64_value` answered from a 256-entry table | 11.31 G | 2.52 G | 2.67 ms | 3.5 | −32 % |
| 2 | the decode loop's push keeps its length in the header; written once at the loop's exits | 11.16 G | 2.47 G | 2.47 ms | 3.45 | −2 % |
| 3 | `(R-LitDiv)`: `v / 65536`, `v / 256` plain (dividend non-null) | 11.12 G | 2.46 G | 2.54 ms | 3.4 | ≈ 0 |
| 4 | results built in their destination: `b64_decode`'s `out` IS the caller's buffer, `base64_to_bytes` hands its buffer down; two `loft_vec_add` copies gone | 10.44 G | 2.28 G | 2.37 ms | 3.2 | −7 % |
| 5 (library) | pluginabi `request` decodes each argument once and validates the result (`sb = base64_to_bytes(s); if len(sb) == 0 && s != "" …`) | **6.60 G** | **1.53 G** | **1.52 ms** | **2.1** | −33 % |
| 5 alone | the library change on the unedited emission | 7.45 G | 2.22 G | 2.23 ms | 3.1 | −41 % from base |

Step 1 also failed in another form: writing the range tests as plain `&&`/`||` expressions
(so gcc can fold `65 <= c && c <= 90` into one unsigned compare) made it worse (50 M misses,
4.10 G cycles).  The emitted nested ifs are not the cost; the data-dependent branches are.

## 4. The mechanism behind each step

| step | mechanism | rule | native? | safe when | shares the shape |
|---|---|---|---|---|---|
| 1 | **NEW — a classifier answered from a table:** a pure function of one integer whose body is an `if` chain of constant range/equality tests returning constants or `c ± k`, called with a byte-domain argument (`byte_at`, a `u8` element) → a 256-entry table built at emit time, the out-of-domain case kept as the general path | none (`(R-Table)` would be new, `formal/rewrites.md`) | no | the function is pure, total over 0..255, no side effects; the argument's domain is known (else the table sits behind a `< 256` test, as priced) | crypto `b64_value` (also in `base64_to_bytes`, `check_request`'s decode paths), any hex/char-class decoder (cbor's text classes, markdown, html escape tests) |
| 2 | `(R-Push)` sole-growth clause, extended: the header keeps the LENGTH as well as the address while the loop's only growth is its own push; written back at each exit and before a growth | `@FR-R-Push` (`loft_ph_putb*`, `hoist::LoopHoist::sole_push`) | native's push window does it for bytes | as the sole-growth clause, plus a write-back on every loop exit (`return`, `break`) and before the slow path | every `vals += [v]` loop with an 8-byte element; the byte form already exists as `loft_win` |
| 3 | `(R-LitDiv)` on the C | `(R-LitDiv)` — census: 546 native sites, 0 on C | yes | divisor a non-zero literal, dividend proven non-null | broad but priced ≈ 0 here (gcc already shortcuts the test) |
| 4 | **a result vector built in its destination across the return chain** — the callee's local that every non-null exit returns IS the return buffer (`(R-Destination)` / RetAdopt for vectors), and a wrapper that returns `call(...) ?? []` passes its own buffer down | `(R-Destination)` (built on C for scalars as a tuple, `c/body.rs destination_site`); C RetAdopt (99b797739) covers the local's adoption but not this two-level chain | partly (rustc copies here too) | the local is returned on every non-null exit and not read after the return-buffer is cleared; the null exits return before the local exists | `base64_to_bytes` everywhere (pluginabi `check_request`, crypto `aes256gcm_*`), every `fn f() -> vector<…>?` wrapped by `f() ?? []` |
| 5 | library algorithm: validate by decoding, then decode again | — (loft-libs-plugins `pluginabi`) | n/a | `base64_to_bytes` answers `[]` exactly for non-base64 text or `""`, so the one decode answers both questions | pluginabi `request`; the compiler-side alternative is a pure-call reuse across two inlined wrappers (`(R-PureReuse)` extended past a function boundary), not priced |

## 5. What is left at 2.1×

- The decoder is still 46 % of the C's cycles (the twin's whole decode is 81 % of its time,
  but it does a quarter of the instructions per character).  Loop 2 still carries an element
  bounds test, a `?? 0` null test per element and a `% 256` → `u8` range test per byte (value
  range would prove `0..255`).  Not priced.
- **Store copies of the payload while building the request** (not priced): `CBytes { value: sb }`
  copies `sb` into the record, `pa_te` copies the `CborValue` into the entry
  (`loft_copy_record`), `es += [entry]` copies the entry into the vector (`loft_vec_add`) —
  `loft_copy_claims` 11.7 % + claims/mints ≈ 9 % of the final build.  The twin moves the
  `Vec`s.  Lever: a last-use MOVE into a record field / vector element (the text form exists:
  `LOFT_NO_C_TEXT_MOVE`; the vector/record form does not).  Estimated ≈ 1.8× after.

## `req_state_b64`

Not the same code: a cut-down driver (`pkgread/`) is 2.5× the twin, with **78 %** in
`n_bytes_to_base64` (the ENCODER — the short appends' territory), not the decoder.  Not priced
here.
