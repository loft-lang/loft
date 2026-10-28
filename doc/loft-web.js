// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// The `loft_web` host imports: the web library's natives in a page `loft --html` builds
// (`src/web_browser.rs` declares them).  The `--html` driver adds this file to the page when the
// module imports `loft_web`.  It was the web library's `wasm/host.js` until web 0.5.0, whose
// natives live in loft (`[native] in_binary = true`).
//
// A live `WebSocket` per handle with an inbound frame queue; the wasm side polls (`ws_poll`) and
// reads the latched current frame (`ws_msg_len` / `ws_msg_copy` / `ws_opcode`).  `fetch()` runs
// asynchronously while the wasm side polls `http_poll` and yields.  `ws_yield` is the asyncify
// suspend: between an unwind and the next resume the event loop runs, which is when
// `WebSocket.onmessage` and the fetch promise fire.
//
// Every import re-reads `getMem().buffer` — wasm memory can grow and detach — and payloads are
// `.slice()`d before a send, so the socket never holds a view into memory that could move.

(globalThis.LOFT_WASM_EXTENSIONS = globalThis.LOFT_WASM_EXTENSIONS || []).push(
  function loftWebHostImports(imports, ctrl, getMem) {
    const ns = (imports.loft_web = imports.loft_web || {});
    const enc = new TextEncoder();
    const dec = new TextDecoder('utf-8', { fatal: false });

    // handle -> { socket, inbound: [{op, bytes}], current: {op, bytes} | null }
    const conns = new Map();
    let nextHandle = 0;

    function bytesAt(ptr, len) {
      return new Uint8Array(getMem().buffer, ptr, len);
    }

    ns.ws_connect = function (urlPtr, urlLen) {
      let url;
      try {
        url = dec.decode(bytesAt(urlPtr, urlLen));
      } catch (_e) {
        return -1;
      }
      let sock;
      try {
        sock = new WebSocket(url);
      } catch (_e) {
        return -1;
      }
      sock.binaryType = 'arraybuffer';
      const h = nextHandle++;
      const conn = { socket: sock, inbound: [], current: null, open: false, closed: false };
      conns.set(h, conn);
      sock.onopen = () => {
        conn.open = true;
      };
      sock.onmessage = (ev) => {
        if (typeof ev.data === 'string') {
          conn.inbound.push({ op: 1, bytes: enc.encode(ev.data) });
        } else {
          conn.inbound.push({ op: 2, bytes: new Uint8Array(ev.data) });
        }
      };
      sock.onclose = () => {
        conn.closed = true;
      };
      sock.onerror = () => {
        conn.closed = true;
      };
      return h;
    };

    ns.ws_send = function (h, ptr, len) {
      const conn = conns.get(h);
      if (!conn || conn.socket.readyState !== 1 /* OPEN */) return 0;
      // Send a STRING -> TEXT frame.  Decode the wasm bytes as UTF-8.
      const text = dec.decode(bytesAt(ptr, len).slice());
      try {
        conn.socket.send(text);
        return 1;
      } catch (_e) {
        return 0;
      }
    };

    ns.ws_send_binary = function (h, ptr, len) {
      const conn = conns.get(h);
      if (!conn || conn.socket.readyState !== 1) return 0;
      // Send a Uint8Array -> BINARY frame.  `.slice()` detaches from wasm
      // memory.  Sending binary as a binary frame is load-bearing for C3 — a
      // CBOR payload sent as TEXT would mangle its zero bytes.
      const buf = bytesAt(ptr, len).slice();
      try {
        conn.socket.send(buf);
        return 1;
      } catch (_e) {
        return 0;
      }
    };

    ns.ws_poll = function (h) {
      const conn = conns.get(h);
      if (!conn) return 0;
      const next = conn.inbound.shift();
      if (!next) {
        conn.current = null;
        return 0;
      }
      conn.current = next;
      return 1;
    };

    ns.ws_msg_len = function (h) {
      const conn = conns.get(h);
      return conn && conn.current ? conn.current.bytes.length : 0;
    };

    ns.ws_msg_copy = function (h, ptr) {
      const conn = conns.get(h);
      if (!conn || !conn.current) return;
      const src = conn.current.bytes;
      // Re-fetch the buffer (it may have grown since ws_msg_len).
      new Uint8Array(getMem().buffer, ptr, src.length).set(src);
    };

    ns.ws_opcode = function (h) {
      const conn = conns.get(h);
      return conn && conn.current ? conn.current.op : 1;
    };

    ns.ws_close = function (h) {
      const conn = conns.get(h);
      if (!conn) return;
      try {
        conn.socket.close();
      } catch (_e) {
        /* already closing */
      }
      conns.delete(h);
    };

    // ── HTTP fetch (async; the wasm side polls http_poll + yields until done) ──
    // handle -> { done, error, status, body: Uint8Array, headers: string }
    const fetches = new Map();
    let nextFetch = 0;

    // Parse the wasm "Key: Value\n…" request-header text into a fetch headers obj.
    function parseReqHeaders(text) {
      const h = {};
      for (const line of text.split('\n')) {
        const i = line.indexOf(':');
        if (i > 0) h[line.slice(0, i).trim()] = line.slice(i + 1).trim();
      }
      return h;
    }

    ns.http_start = function (mPtr, mLen, uPtr, uLen, bPtr, bLen, hPtr, hLen) {
      const method = dec.decode(bytesAt(mPtr, mLen).slice());
      const url = dec.decode(bytesAt(uPtr, uLen).slice());
      const body = bLen > 0 ? bytesAt(bPtr, bLen).slice() : undefined;
      const headers = hLen > 0 ? parseReqHeaders(dec.decode(bytesAt(hPtr, hLen).slice())) : undefined;
      const id = nextFetch++;
      const entry = { done: false, error: false, status: 0, body: new Uint8Array(0), headers: '' };
      fetches.set(id, entry);
      const init = { method, headers };
      if (body !== undefined && method !== 'GET' && method !== 'HEAD') init.body = body;
      fetch(url, init)
        .then(async (resp) => {
          entry.status = resp.status;
          // Only CORS-exposed headers are visible cross-origin; the server must
          // send Access-Control-Expose-Headers for Content-Range/Content-Length.
          const hdrs = [];
          resp.headers.forEach((v, k) => hdrs.push(k + ': ' + v));
          entry.headers = hdrs.join('\n');
          entry.body = new Uint8Array(await resp.arrayBuffer());
          entry.done = true;
        })
        .catch(() => {
          entry.error = true;
          entry.done = true;
        });
      return id;
    };

    ns.http_poll = function (h) {
      const e = fetches.get(h);
      if (!e) return -1;
      if (!e.done) return 0;
      return e.error ? -1 : 1;
    };
    ns.http_status = function (h) {
      const e = fetches.get(h);
      return e ? e.status : 0;
    };
    ns.http_body_len = function (h) {
      const e = fetches.get(h);
      return e ? e.body.length : 0;
    };
    ns.http_body_copy = function (h, ptr) {
      const e = fetches.get(h);
      if (!e) return;
      new Uint8Array(getMem().buffer, ptr, e.body.length).set(e.body);
    };
    ns.http_headers_len = function (h) {
      const e = fetches.get(h);
      return e ? enc.encode(e.headers).length : 0;
    };
    ns.http_headers_copy = function (h, ptr) {
      const e = fetches.get(h);
      if (!e) return;
      const bytes = enc.encode(e.headers);
      new Uint8Array(getMem().buffer, ptr, bytes.length).set(bytes);
    };
    ns.http_free = function (h) {
      fetches.delete(h);
    };

    // Asyncify suspend: hand control back to the JS event loop for one frame.
    // `ctrl.ac` is the AsyncifyCtrl (the --html preamble / wasm_ws_repro.mjs
    // sets it after instantiate).  No-op if asyncify is absent (compute-only
    // bundle).
    ns.ws_yield = function () {
      if (ctrl && ctrl.ac) ctrl.ac.suspend();
    };
  },
);
