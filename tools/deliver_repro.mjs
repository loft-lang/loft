// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN105 Phase 2d — the deliver node harness (the whole-slice falsifier for Phase 2).
// Instantiates a `loft --html`-built WASM with loft_io host stubs, wires `loft_host_deliver` to the
// generic descriptor-driven reader (doc/loft-deliver.js), runs `loft_start`, and prints every
// delivered value as JSON on stdout. The Rust test (tests/deliver_wasm.rs) asserts that JSON equals
// the value the loft program delivered — proving JS reconstructs a value from wasm memory driven
// only by the descriptor, and (via the same program on --interpret) that --html == interpreter.
//
//   node tools/deliver_repro.mjs <path/to/wasm_file>
//
// Exit 0 on a clean run (deliveries printed as `DELIVER <json>` lines), 1 on a trap/error.

import fs from "node:fs";
import process from "node:process";
import { readLoftValue } from "../doc/loft-deliver.js";
// loft#851 — the page's filesystem, imported rather than restubbed so every
// harness answers what a real page answers.  A stub returning 0 would mean "an
// empty file that EXISTS" where the contract says absent, and a missing import
// is a LinkError the moment a program under test touches a file.
import { loftFSImports } from '../doc/loft-fs.js';

// A module the C backend wrote imports the WASI its libc runs on: the layer a page carries
// (`doc/loft-wasi.js`), loaded as `wasm_repro.mjs` loads it.  A Rust page imports none of it.
(0, eval)(fs.readFileSync(new URL('../doc/loft-wasi.js', import.meta.url), 'utf8'));
async function instantiateWithWasi(bytes, imports) {
  let inst = null;
  for (const reg of globalThis.LOFT_WASM_EXTENSIONS || []) reg(imports, {}, () => inst.exports.memory);
  const out = await WebAssembly.instantiate(bytes, imports);
  inst = out.instance;
  return out;
}

const wasmPath = process.argv[2];
if (!wasmPath) {
  console.error("usage: node deliver_repro.mjs <wasm_file>");
  process.exit(2);
}

let mem = null;
const dec = new TextDecoder();
const exposed = new Map();

// JSON-safe: BigInt → Number when exact, else a "…n" string; typed arrays → plain arrays.
function jsonSafe(v) {
  if (typeof v === "bigint")
    return v >= BigInt(Number.MIN_SAFE_INTEGER) && v <= BigInt(Number.MAX_SAFE_INTEGER)
      ? Number(v)
      : `${v}n`;
  if (ArrayBuffer.isView(v)) return Array.from(v, jsonSafe);
  if (Array.isArray(v)) return v.map(jsonSafe);
  if (v && typeof v === "object") {
    const o = {};
    for (const k of Object.keys(v)) o[k] = jsonSafe(v[k]);
    return o;
  }
  return v;
}

const loft_io = {
  ...loftFSImports(() => mem),
  loft_host_print(ptr, len) {
    process.stderr.write(dec.decode(new Uint8Array(mem.buffer, ptr, len)));
  },
  loft_host_input_len: () => 0,
  loft_host_input_copy: () => {},
  loft_host_output(ptr, len) {
    process.stderr.write("[out] " + dec.decode(new Uint8Array(mem.buffer, ptr, len)));
  },
  loft_host_http_get: () => 0xffffffff,
  loft_host_http_get_copy: () => {},
  // loft#678 working-set loaders: refused here like the whole-file GET above — these
  // harnesses never serve a store, and a MISSING import is a LinkError, not a skip.
  loft_host_http_range: () => 0xffffffff,
  loft_host_http_range_total: () => -1,
  // @PLN105 Phase 2 — the value is handed over as (store_base, rec, pos, type_id) + the descriptor
  // JSON blob (dptr,dlen). Read it synchronously here (the borrow ends when this returns), copying
  // out of the zero-copy typed-array views so the captured value survives past the call.
  loft_host_deliver(tag, store_base, rec, pos, type_id, dptr, dlen) {
    const desc = JSON.parse(dec.decode(new Uint8Array(mem.buffer, dptr, dlen)));
    const value = readLoftValue(mem, store_base, desc, type_id, rec, pos);
    const line = { tag: typeof tag === "bigint" ? Number(tag) : tag, type: type_id, value: jsonSafe(value) };
    process.stdout.write("DELIVER " + JSON.stringify(line) + "\n");
  },
  // @PLN105 Phase 3 — expose(tag,value): a long-lived deliver. The value's store is pinned loft-side
  // so its addresses stay valid across frames — a real page reads globalThis.loftExposed each frame.
  // This single-shot harness reads it once here (in the expose call, while `Stores` is alive); the
  // full cross-frame path (reading between yields) needs the asyncify game-loop harness.
  loft_host_expose(tag, store_base, rec, pos, type_id, dptr, dlen) {
    const desc = JSON.parse(dec.decode(new Uint8Array(mem.buffer, dptr, dlen)));
    exposed.set(String(tag), { store_base, rec, pos, type_id, desc });
    const value = readLoftValue(mem, store_base, desc, type_id, rec, pos);
    const line = { tag: typeof tag === "bigint" ? Number(tag) : tag, type: type_id, value: jsonSafe(value) };
    process.stdout.write("EXPOSE " + JSON.stringify(line) + "\n");

    // @PLN105 §5 memory.grow-safety cell (opt-in via LOFT_DELIVER_GROW). A page reads an exposed
    // value every FRAME; between frames wasm can `memory.grow`, which DETACHES the old ArrayBuffer.
    // readLoftValue re-derives `mem.buffer` on every call, so a re-read after a grow must still
    // succeed and reproduce the SAME value (grow preserves existing bytes; the store is pinned).
    // Anti-vacuity: a DataView captured BEFORE the grow must throw afterwards — proving the harness
    // can actually observe the detach (a passing re-read is only meaningful if the buffer really
    // changed underneath it).
    if (process.env.LOFT_DELIVER_GROW) {
      const staleView = new DataView(mem.buffer); // bound to the pre-grow ArrayBuffer
      const prevPages = mem.grow(1); // +1 page (64 KiB) — detaches staleView's buffer
      let staleDetached = false;
      try {
        staleView.getUint8(0);
      } catch {
        staleDetached = true;
      }
      const after = readLoftValue(mem, store_base, desc, type_id, rec, pos); // re-derives mem.buffer
      const rereadMatches = JSON.stringify(jsonSafe(after)) === JSON.stringify(jsonSafe(value));
      process.stdout.write(
        "GROW-SAFE " +
          JSON.stringify({
            grew: prevPages >= 0,
            stale_detached: staleDetached,
            reread_matches: rereadMatches,
            value: jsonSafe(after),
          }) +
          "\n",
      );
    }
  },
  loft_host_release(tag) {
    exposed.delete(String(tag));
    process.stdout.write("RELEASE " + (typeof tag === "bigint" ? Number(tag) : tag) + "\n");
  },
};

try {
  const bytes = fs.readFileSync(wasmPath);
  const { instance } = await instantiateWithWasi(bytes, { loft_io });
  mem = instance.exports.memory;
  if (typeof instance.exports.loft_start === "function") instance.exports.loft_start();
  else if (typeof instance.exports.main === "function") instance.exports.main();
  process.exit(0);
} catch (e) {
  console.error("deliver_repro: " + (e && e.stack ? e.stack : e));
  process.exit(1);
}
