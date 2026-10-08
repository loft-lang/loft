# `x ?? exit(1)` is refused — a call that never returns is not a `??` default
axis: clarity
met-by: scripts/check_contract_goldens (strand 4d) — `contract=$(…) || exit 1` in the original, one line
status: open
fix: let a `??` default be a call that never returns — `exit(code)`, `panic(…)` — the way `?? return` already is (@F2): the value's type is then the left side's, and the author writes the absent case where it is met; needs `exit` / `panic` to be typed as never returning, which `-> !` or an attribute the stdlib declares would give
ref: 
probe: 019.probe.loft
expect: refused
checked: 8c8a7ed43
holds: yes

The original says `contract=$(grep …) || exit 1`.  The port wants `contract = version(manifest) ?? exit(1)` and gets `coalesce-default-type-mismatch`: `exit` is a `void` call, and a `??` default must have the left side's type.  `?? return` is special-cased for exactly this shape; `exit(1)`, `panic("…")` and any `fn … -> !` of the program's own are not, so the port spends a sentinel and an `if` where the shell spent two words.  The same shape recurs in every gate script: read a value, or stop with a code.
