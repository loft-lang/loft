<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 08-server — CLOSED (design declined) 2026-07-02

**Status: CLOSED.** The large HTTP-framework design this doc once held was
**declined, not built.** The `server` library shipped instead as a **minimal
single-file TCP + WebSocket server**, and its canonical documentation now lives
with the library itself.

- **Shipped library + its docs:** `loft-lang/loft-libs-net` →
  [`server/README.md`](https://github.com/loft-lang/loft-libs-net/tree/main/server)
  (real public API: `listen` / `next` / `respond*`, single-client
  `ws_upgrade` / `next` / `send`, multi-client `run(on_event)` / `poll_event` /
  `broadcast` / `send_to`). Catalogue entry: [LIBRARIES.md](../../../LIBRARIES.md). <!--noindex-->
- **Why the framework design was declined:**
  [DESIGN_DECISIONS.md § C84](../../../DESIGN_DECISIONS_PLATFORM.md#c84--server-ships-as-minimal-tcpws-primitives-not-a-fully-featured-http-framework).

## What this doc used to claim vs. what shipped

The original design (preserved in git history; see also the superseded
[`ARCHITECTURE.md`](ARCHITECTURE.md)) specified "a fully featured HTTP server
library" across **12 loft source files** — `App` + `route`/`get`/`post`
routing, a `Middleware` enum, `AuthConfig` (JWT / session / API-key / Basic),
`TlsConfig` + ACME / Let's Encrypt, `serve_dir`, `parse_json`, sessions, CORS,
and rate-limiting.

**None of that was built.** The shipped `server` is one file
(`server/src/server.loft`, ~366 lines) over a thin native socket +
`tungstenite` layer: blocking/non-blocking HTTP accept with typed `respond*`
helpers, single-client WebSocket, and a Rust-driven multi-client event pump
(`run(on_event: fn(WsEvent))`). A loft program does its own routing with a
`match` on `req.path`; there is no framework layer.

If a framework surface is ever wanted, treat it as new work with fresh evidence
(see the C84 "Revisit when") — do not resurrect this doc as the plan of record.
