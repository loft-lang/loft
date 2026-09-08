# The Tier-2 `wasm.bridge` recipe

Read this when a `#native` library must run in the **browser** (`--html`) — the one matrix
cell with no automatic path. The bridge lets the library's wasm call a **host capability**
(crypto, sockets, the DOM, files) that wasm itself can't reach. Full reference:
[PACKAGES.md § library-owned wasm bridges](../../../../doc/claude/PACKAGES.md),
[WASM.md](../../../../doc/claude/WASM.md), [HTML_EXPORT.md](../../../../doc/claude/HTML_EXPORT.md).

## The shape (four parts)

```
my_lib/
├── loft.toml            # [wasm.bridge] block: crate, host_js, [wasm.bridge.routes]
├── src/my_lib.loft      # the loft API; #native fns map to bridge routes
└── wasm/                # the bridge crate
    ├── Cargo.toml
    ├── host.js           # what [wasm.bridge] host_js names
    └── src/lib.rs        # bridge fns the routes point at
```

1. **`[wasm.bridge]` in `loft.toml`** — its real fields are `crate` (the bridge crate),
   `host_js` (the JS shim file), and a `[wasm.bridge.routes]` table mapping each native
   symbol (`n_<sym>`) to a bridge function (PACKAGES.md § wasm.bridge). Host-import
   module names carry the `loft_` prefix (e.g. `loft_web`), which is what the runtime
   recognizes as a permitted host-import module.
2. **The bridge crate (`wasm/`)** — a small Rust crate compiled to wasm alongside the
   library. The routed bridge functions receive the loft store + argument references
   and marshal per function (raw memory, not a serialized envelope).
3. **The host shim** — the JS/WASI side that *implements* those imports:
   - **Browser (`--html`):** a `host.js` providing `loft_<lib>.<fn>` against the real
     capability (WebCrypto, WebSocket, the DOM). It's wired into the page's import object next
     to the loft runtime imports.
   - **Headless (no browser):** the `--html`-built wasm driven in Node
     (`LOFT_WASM_HOST_JS=… node tools/wasm_ws_repro.mjs` is the @PLN84 model) — this is
     how the bridge is tested without a browser. ⚠ `--native-wasm` (wasip2) is a
     DIFFERENT path: it links the wasm rlib (`wasm_impl`) and never sees `host.js`.
4. **The marshalling** — values cross the boundary as store/memory references the bridge
   fn reads and writes per its route; there is no serialized envelope (the "CBOR" in
   @PLN84 was one WebSocket regression's payload, `ws_cbor.loft`, not the bridge ABI).
   This is the silent-corruption surface; see the traps.

## Asyncify — the suspend trap (this one cost real time)

If a bridge function **yields or awaits** (a socket read, a frame yield, anything async on the
host), the wasm must be asyncify-transformed so it can suspend and resume. ⚠ You do NOT run
`wasm-opt` yourself: `loft --html` runs it unconditionally with a **hardcoded** asyncify
import allowlist (`src/main.rs`, currently `loft_gl.loft_gl_swap_buffers`,
`loft_web.ws_yield`, `loft_io.loft_host_http_get`, `loft_io.loft_host_http_range`) — an
import left OFF that list corrupts the stack, so a new library's suspending import means
**editing that list in loft** and rebuilding, not an author-side wasm-opt pass.

The trap that bites: **`yield_frame()` only sets a flag — it does NOT itself suspend.** Only an
import listed in `--pass-arg=asyncify-imports@…` actually unwinds the stack. So a suspending
call needs a **dedicated suspend import** (the @PLN84 pattern: `loft_web.ws_yield`), added to
the asyncify import list — not a reuse of a flag-only yield. If your "await" returns instantly
or hangs, this is almost always why.

## The boundary-marshalling traps

- **Validate with a round-trip *value* check, not "it didn't crash."** A mis-sized or mis-
  ordered field decodes to garbage that often *looks* plausible — assert the decoded value
  equals the sent value, on a distinctive payload.
- **The control struct has its own ABI.** The asyncify control block (the AsyncifyCtrl
  layout — stack ptr / data ptrs) is part of the contract; a wrong offset there corrupts the
  suspend/resume, not the payload, so it presents as a hang or a wild pointer, not a decode
  error. Two such bugs hid in the @PLN84 WS bridge — suspect the control ABI when the *value*
  round-trips but the *suspend* misbehaves.

## The gate for a bridge

The bridge is done only when the library passes the **parity gate** on `--native-wasm`
(the wasip2 / wasm-rlib path) **and** `--html` (via `host.js`), with results equal to
`--interpret`. For the `--html` half without a browser, drive the built wasm in Node —
the @PLN84 `tools/wasm_ws_repro.mjs` is the model; keep such a driver so the bridge is
provable in CI.
