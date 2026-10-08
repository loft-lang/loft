# `x ?? exit(1)` is refused — a call that never returns is not a `??` default
axis: clarity
met-by: scripts/check_contract_goldens (strand 4d) — `contract=$(…) || exit 1` in the original, one line
status: fixed
fix: let a `??` default be a call that never returns — `exit(code)`, `panic(…)` — the way `?? return` already is (@F2): the value's type is then the left side's, and the author writes the absent case where it is met; needs `exit` / `panic` to be typed as never returning, which `-> !` or an attribute the stdlib declares would give
ref: macos-scripting (2026-10-08) — `build_null_coalesce_default_inner` routes a default that is a call of `exit` or `panic` through `null_coalesce_exit`, the lowering `?? return` / `?? break` take; `panic` is emitted on the first pass too (it left `Null` there, and a coalesce typed from a void default crashed on a `text?` subject).  Guarded by tests/panic_halts_both_backends.rs (`a_halting_call_is_a_coalesce_default`, both backends) and tests/scripts/019-a-halting-call-is-a-coalesce-default.loft
probe: 019.probe.loft
expect: refused
checked: 6a26d8ca9
holds: no

The original says `contract=$(grep …) || exit 1`.  The port wants `contract = version(manifest) ?? exit(1)` and gets `coalesce-default-type-mismatch`: `exit` is a `void` call, and a `??` default must have the left side's type.  `?? return` is special-cased for exactly this shape; `exit(1)`, `panic("…")` and any `fn … -> !` of the program's own are not, so the port spends a sentinel and an `if` where the shell spent two words.  The same shape recurs in every gate script: read a value, or stop with a code.
