# No `exit(code)`
axis: behaviour
met-by: scripts/wasm_bundle_stamp (strand 3)
status: open
fix: `exit(code)` in the stdlib — a primitive only the runtime can answer, which is the one kind that belongs there (README § Strand 8)
ref: 
probe: 009.probe.loft
expect: refused
checked: ff407fedf
holds: yes

no `exit(code)`: `set -e`'s "stop with status 1 on the first unreadable file" is an `assert`, whose stderr and status are loft's, not the script's
