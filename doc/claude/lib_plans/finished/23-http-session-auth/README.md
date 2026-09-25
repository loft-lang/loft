<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 23 — HTTP response headers + cookie-jar session

**Status — DONE 2026-05-27.**  All three sub-arcs (P1 + P2 + P3) shipped on `lib/web`.

Reference for the shipped feature lives in the library source itself at
`lib/web/src/web.loft` (the `HttpResponse.header` / `headers_for` / `headers`
field, `HttpSession` struct, `http_session(...)` / `get` / `post` / `put` /
`delete`, `base64_encode` / `base64_decode`).  This file is a closure record
only.

## What shipped

| Phase | What | Source |
|---|---|---|
| **P1** — response headers | `headers: vector<text>` field on `HttpResponse` + `header(name)` (first-match, case-insensitive) + `headers_for(name)` (all values, preserves duplicates).  Wire format mirrors existing `LAST_BODY` thread-local pattern: a `LAST_HEADERS` thread-local + `n_http_headers_raw() -> text` native that newline-joins `"Name: Value"`, split on the loft side. | `lib/web/src/web.loft` lines 22-32 + `lib/web/native/src/lib.rs` |
| **P2** — cookie-jar `HttpSession` | `http_session(follow_redirects: boolean) -> HttpSession` + `get` / `post` / `put` / `delete(self, url, [body,] headers)`.  Mirrors the `ws_client` slot pattern (registry keyed by an `i32` handle).  `ureq = { version = "2", features = ["cookies"] }` enables the cookie store; `follow_redirects=false` lets the caller read `Location` on 302s while keeping the jar. | `lib/web/src/web.loft` lines 140-157 + native handle registry |
| **P3** — `base64_encode`/`decode` | Standard Base64 alphabet; RFC 4648 test vector passes both backends.  `base64` crate already vendored for `lib/web` / `lib/server`. | `lib/web/src/web.loft` |

## Verification

- 10/10 `tests/http.loft` pass on `--interpret`.
- Minimal session program (cookie round-trip + header read) verified on `--native`.
- Base64 RFC vector passes both backends.
- Consumer-driven: `~/workspace/personal/training/loft/requests/E2-lib-web-http-session.md` (consume-only) — verified end-to-end native Garmin login on training-port A2 once P1+P2 landed.

## P-issue closed during this plan

- **@P365** — early `return []` miscompiled on both backends.  Surfaced building P1's `headers_for` (an early-return empty-vector idiom).  Fixed in `src/parser/control.rs` + `src/parser/vectors.rs`.  Regression: included in `tests/scripts/` parser suite.

## Deferred follow-ups (not part of this plan)

- **TLS / JA3 impersonation (E1)** — conditional; only if Garmin's WAF later blocks the plain-TLS credential POST.  Untested today; "don't build yet."
- **`exec()` subprocess primitive** — superseded by this workstream for the native-Garmin use case; covered by [`../15-process/`](../../67-process) for other consumers.
- **`HttpSession.close()`** — login is short-lived (a few requests, one process), so process-lifetime jars suffice.  Add only if a leak shows up.
- **WASM target** — out of scope: native login runs natively (browser can't call Garmin directly via CORS).

## Historical: investigation archaeology

`IMPL.md` retains the step-by-step implementation designs (P1.1 / P1.2 / P2.1 / P2.2 / P3.1) that drove the landed commits.  Kept as the per-step verification trail.

## See also

- [`../../06-web-services/HTTP_CLIENT.md`](../../06-web-services/HTTP_CLIENT.md) — the broader HTTP-client design this plan implemented one slice of.
- [`../../future/15-process/`](../../67-process) — `exec()` subprocess primitive (for non-HTTP shell-out use cases).
- [`IMPL.md`](IMPL.md) — implementation steps (closure archaeology).
