// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN105 §5 — the CROSS-FRAME expose falsifier. The single-shot harness (deliver_repro.mjs) reads
// an exposed value DURING the expose host call, while `Stores` is unambiguously live. The real
// use case is different: a page reads the exposed value on a LATER frame — after loft has run on
// past `expose`, yielded control back to JS (an asyncify unwind), and the JS event loop has
// regained control. This harness reproduces that: it drives asyncify, and at the yield point
// (`loft_host_http_get`, the store_load_url_trusted fetch — the only headless suspend) it
// reconstructs every exposed value from the CURRENT linear memory and prints a `CROSSFRAME` line,
// then resumes loft (which releases and returns). Proving the value survives the yield validates
// that `lock_store` keeps the store pinned across real execution + the unwind/rewind round-trip,
// and that the reader re-derives its view (the fetch may have grown memory).
//
//   node tools/deliver_crossframe.mjs <path/to/wasm_file>
//
// Exit 0 on a clean run, 1 on a trap/error.

import fs from "node:fs";
import process from "node:process";
import { readLoftValue } from "../doc/loft-deliver.js";
import { AsyncifyCtrl } from "../doc/loft-asyncify.js";
// loft#851 — the page's filesystem, imported rather than restubbed so every
// harness answers what a real page answers.  A stub returning 0 would mean "an
// empty file that EXISTS" where the contract says absent, and a missing import
// is a LinkError the moment a program under test touches a file.
import { loftFSImports } from '../doc/loft-fs.js';

const wasmPath = process.argv[2];
if (!wasmPath) {
  console.error("usage: node deliver_crossframe.mjs <wasm_file>");
  process.exit(2);
}

let mem = null;
let ac = null;
const dec = new TextDecoder();
const exposed = new Map();
const fakeBytes = new Uint8Array(0); // empty store image → store_load rejects it (false), never crashes

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
  // The headless asyncify yield. Mirrors the minimal-shell shim in src/main.rs: on the REWIND
  // replay it stop_rewinds and returns the byte count; on the NORMAL call it is the FRAME BOUNDARY
  // — loft has unwound and JS has control, exactly when a page would read globalThis.loftExposed.
  loft_host_http_get: (ptr, len) => {
    if (ac && ac.exports.asyncify_get_state() === 2 /* REWINDING */) {
      ac.suspend(); // stop_rewind, continue past the yield
      return fakeBytes.length;
    }
    // The layout sidecar a whole-image load asks for first (`<url>.dschema`) is answered as a
    // server without one answers it — a 404, without suspending — so the frame boundary stays
    // the image fetch itself.
    if (dec.decode(new Uint8Array(mem.buffer, ptr, len)).endsWith(".dschema")) return 0xffffffff;
    // Cross-frame read: reconstruct every exposed value from the memory as it stands AFTER the
    // yield (re-deriving mem.buffer inside readLoftValue — the fetch may have grown it).
    for (const [tag, h] of exposed) {
      const value = readLoftValue(mem, h.store_base, h.desc, h.type_id, h.rec, h.pos);
      const line = { tag: Number(tag), type: h.type_id, value: jsonSafe(value) };
      process.stdout.write("CROSSFRAME " + JSON.stringify(line) + "\n");
    }
    ac.suspend(); // start_unwind — hand control back to the driver below
    return 0;
  },
  loft_host_http_get_copy: (ptr) => {
    if (fakeBytes.length) new Uint8Array(mem.buffer, ptr, fakeBytes.length).set(fakeBytes);
  },
  // loft#678 working-set loaders. Refused, and deliberately NOT wired to the yield
  // above: this harness uses loft_host_http_get as its frame boundary, so making a
  // range fetch suspend too would emit a second CROSSFRAME batch per frame and the
  // expected output would drift. Declared because a missing import is a LinkError.
  loft_host_http_range: () => 0xffffffff,
  loft_host_http_range_total: () => -1,
  loft_host_deliver() {},
  loft_host_expose(tag, store_base, rec, pos, type_id, dptr, dlen) {
    const desc = JSON.parse(dec.decode(new Uint8Array(mem.buffer, dptr, dlen)));
    exposed.set(String(tag), { store_base, rec, pos, type_id, desc });
    process.stdout.write("EXPOSE " + JSON.stringify({ tag: Number(tag), type: type_id }) + "\n");
  },
  loft_host_release(tag) {
    exposed.delete(String(tag));
    process.stdout.write("RELEASE " + Number(tag) + "\n");
  },
};

try {
  const { instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { loft_io });
  mem = instance.exports.memory;
  // The cross-frame yield needs the asyncify runtime, which `wasm-opt --asyncify` adds at build
  // time. If wasm-opt/binaryen was absent when `loft --html` ran (some CI jobs), the bundle has NO
  // asyncify instrumentation and there is no yield to exercise — self-skip rather than trap (mirrors
  // the node/wasm32/release skips in tests/deliver_wasm.rs).
  if (typeof instance.exports.asyncify_get_state !== "function") {
    console.log("SKIP: wasm lacks asyncify instrumentation (wasm-opt/binaryen not installed at build)");
    process.exit(0);
  }
  ac = new AsyncifyCtrl(instance);
  ac.start("loft_start"); // runs to the fetch yield (expose has already fired + been stashed)
  if (ac.sleeping) ac.resume("loft_start"); // rewind past the yield → release → return
  process.exit(0);
} catch (e) {
  console.error("deliver_crossframe: " + (e && e.stack ? e.stack : e));
  process.exit(1);
}
