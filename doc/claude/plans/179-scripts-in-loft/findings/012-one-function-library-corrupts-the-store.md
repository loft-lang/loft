# A library cdylib built against a stale `libloft.rlib` loads into a newer binary and corrupts the store
axis: behaviour
met-by: the `script` library (strand 8), the first time `tests/dump_ignored_tests` loaded it
status: fixed
fix: loft#1776 — a build identity the generated cdylib carries and the loader compares with the running binary (mismatch rebuilds or refuses, never loads); until then, rebuild the rlib and reinstall as one unit after every join (`make check-rlib` is the by-hand pre-flight)
ref: loft#1776 — fixed in 0c1e2c840 (#1831), guarded by tests/n3_use_native.rs (the loft#1776 test: two copies of one build adopt no artifact of each other's); a `.loft` probe cannot rebuild a stale rlib, so the Rust test is the evidence
probe: 
expect: 
checked: 
holds: unprobed

First read as "a one-function package corrupts the store on its first call" (`Store access out of bounds … the reference is corrupt`, or `realloc(): invalid next size`), with the crash flipping on what else the file held. The real boundary is the binary: `cargo build --bin loft` after a join left the installed `libloft.rlib` and prefix behind, every pure-loft library under `~/.loft/lib` was auto-compiled against that stale rlib, and the cdylib was dlopen'ed into the newer interpreter. Reinstalling binary and rlib as one unit and clearing `native-auto/` makes the exact repro pass on every backend, `LOFT_REQUIRE_NATIVE=1` included. What the language lacks is the check: nothing compares the pair at build or at load, and the failure is silent corruption rather than a refusal. A probe would need two builds on disk, so this one is re-measured by hand against the issue.
