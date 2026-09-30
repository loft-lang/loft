# No `exit(code)`
axis: behaviour
met-by: scripts/wasm_bundle_stamp (strand 3)
status: open
fix: `exit(code)` (README § Strand 1)
ref: 

no `exit(code)`: `set -e`'s "stop with status 1 on the first unreadable file" is an `assert`, whose stderr and status are loft's, not the script's
