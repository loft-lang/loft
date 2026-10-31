// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN105 probe #3 — the ZERO-COPY performance falsifier (the reason the whole feature exists).
// A delivered inline `vector<scalar>` must reach JS with BOTH sides O(1) in the element count `n`,
// not O(n)/copying. This harness proves that on two axes, structurally (airtight) plus a timing
// corroboration (a same-machine, huge-margin ratio — never an absolute wall-clock threshold, which
// would flake):
//
//   1. LOFT-SIDE O(1): the delivered top node is a plain `vector` (NOT `flatarray`). Only keyed
//      collections are pre-flattened (materialised → O(n)); an inline vector is handed over by
//      reference, so its descriptor is `vector<base>` and deliver did no per-element work.
//   2. JS-SIDE O(1): the reconstructed value is a typed-array VIEW whose `.buffer === mem.buffer`.
//      A copy cannot alias the source buffer; a view over an existing buffer is O(1) by JS
//      semantics (no element is touched to build it).
//   3. Corroboration: view construction (O(1)) vs an O(n) touch of the SAME data on the SAME
//      machine — the ratio is enormous (~500×), so `touchMs >= viewMs * 3` has a vast margin and
//      is dominated by the linear scan, not noise. Both times are the MIN over several reps, so a
//      transient GC/scheduler spike cannot flake it.
//
// (The O(1) guarantee is the inline fast-lane path only. Keyed collections — hash/index/sorted/
// radix — are pre-flattened at deliver time and are O(n) by design; this probe does not claim
// otherwise.)
//
//   node tools/deliver_perf.mjs <path/to/wasm_file>
//
// Emits one `PERF …` line per delivery; exit 0 clean, 1 on a trap/error.

import fs from "node:fs";
import process from "node:process";
import { performance } from "node:perf_hooks";
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
  console.error("usage: node deliver_perf.mjs <wasm_file>");
  process.exit(2);
}

let mem = null;
const dec = new TextDecoder();

const loft_io = {
  ...loftFSImports(() => mem),
  loft_host_print(ptr, len) { process.stderr.write(dec.decode(new Uint8Array(mem.buffer, ptr, len))); },
  loft_host_input_len: () => 0,
  loft_host_input_copy: () => {},
  loft_host_output() {},
  loft_host_http_get: () => 0xffffffff,
  loft_host_http_get_copy: () => {},
  // loft#678 working-set loaders: refused here like the whole-file GET above — these
  // harnesses never serve a store, and a MISSING import is a LinkError, not a skip.
  loft_host_http_range: () => 0xffffffff,
  loft_host_http_range_total: () => -1,
  loft_host_deliver(tag, store_base, rec, pos, type_id, dptr, dlen) {
    const desc = JSON.parse(dec.decode(new Uint8Array(mem.buffer, dptr, dlen)));
    const kind = desc.nodes[type_id]?.kind; // (1) loft-side: `vector` = no materialisation

    // (2) JS-side O(1): time view construction — MIN over reps discards GC/scheduler spikes.
    let viewMs = Infinity;
    let view;
    for (let r = 0; r < 8; r++) {
      const t0 = performance.now();
      view = readLoftValue(mem, store_base, desc, type_id, rec, pos);
      viewMs = Math.min(viewMs, performance.now() - t0);
    }

    // (3) O(n) baseline on the SAME data / SAME machine: a full linear scan.
    let touchMs = Infinity;
    let checksum = 0;
    for (let r = 0; r < 3; r++) {
      const t0 = performance.now();
      let s = 0;
      for (let i = 0; i < view.length; i++) s += view[i];
      touchMs = Math.min(touchMs, performance.now() - t0);
      checksum = s;
    }

    const shared = view instanceof Float32Array && view.buffer === mem.buffer;
    const o1 = touchMs >= viewMs * 3; // O(1) view vs O(n) scan — vast margin, min-based, robust
    const ratio = (touchMs / Math.max(viewMs, 1e-6)).toFixed(0);
    // Deterministic facts up front (the Rust test matches these); timing in the trailing paren.
    process.stdout.write(
      `PERF tag=${Number(tag)} n=${view.length} kind=${kind} shared=${shared} ` +
        `first=${view[0]} last=${view[view.length - 1]} o1=${o1} ` +
        `(viewMs=${viewMs.toFixed(4)} touchMs=${touchMs.toFixed(4)} ratio=${ratio}x checksum=${checksum})\n`,
    );
  },
  loft_host_expose() {},
  loft_host_release() {},
};

try {
  const { instance } = await instantiateWithWasi(fs.readFileSync(wasmPath), { loft_io });
  mem = instance.exports.memory;
  if (typeof instance.exports.loft_start === "function") instance.exports.loft_start();
  process.exit(0);
} catch (e) {
  console.error("deliver_perf: " + (e && e.stack ? e.stack : e));
  process.exit(1);
}
