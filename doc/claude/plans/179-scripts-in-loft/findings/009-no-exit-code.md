# No `exit(code)`
axis: behaviour
met-by: scripts/wasm_bundle_stamp (strand 3)
status: fixed
fix: `exit(code)` in the stdlib — a primitive only the runtime can answer, which is the one kind that belongs there (README § Strand 8)
ref: macos-scripting (2026-10-08) — `exit(code)` in `default/01_code.loft`, guarded by tests/panic_halts_both_backends.rs (`exit_ends_the_program_with_its_code_silently_on_both_backends`: three codes, both backends, from inside a method in a loop); the first port over it is scripts/check_bundle_fresh
probe: 009.probe.loft
expect: refused
checked: 6a26d8ca9
holds: no

no `exit(code)`: `set -e`'s "stop with status 1 on the first unreadable file" is an `assert`, whose stderr and status are loft's, not the script's
