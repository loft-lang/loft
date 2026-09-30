# The registry is unreachable behind a TLS-intercepting proxy
axis: environment
met-by: scripts/wasm_bundle_stamp.sh (strand 3, not yet ported)
status: open
fix: honour `SSL_CERT_FILE` / the system root store in the registry fetch.  Interim that works: `git clone` (which the proxy allows) `loft-libs-core` and copy the package under `~/.loft/lib/<name>`; its native crate builds on first use and both backends resolve it
ref: 

`use crypto;` cannot auto-install behind this box's TLS-intercepting proxy — loft's HTTP client trusts only its bundled roots and ignores `SSL_CERT_FILE`, where `curl` on the same box succeeds

Status note: open — the fetch itself.
