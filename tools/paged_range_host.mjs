// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// loft#678 — the browser working-set loader harness. Instantiates a `loft --html`-built
// WASM and serves `loft_host_http_range` REAL byte ranges out of a store file, so a
// `store_load_key` / `store_load_range` call in the browser target is exercised end to
// end: net::fetch_range → HttpRangeProvider → PagedReader traversal → relocating copy.
//
//   LOFT_PAGED_FILE=<store> node tools/paged_range_host.mjs <path/to/wasm_file>
//
// It answers ranges SYNCHRONOUSLY (the bytes are already on disk), which is legal for an
// asyncify import: instrumentation only has to tolerate a suspend, never require one. The
// suspend path itself is the same one `loft_host_http_get` already proves in
// tools/deliver_crossframe.mjs — what is unproven and tested here is the RANGE plumbing.
//
// On exit it prints `PAGED bytes_fetched=<n> file=<total> requests=<k>`. That line is the
// point of the whole feature: loading one entry must read a small fraction of the file,
// so the Rust test asserts the fraction, not merely that the value arrived.

import fs from "node:fs";
import process from "node:process";
// loft#851 — the page's filesystem, imported rather than restubbed so every
// harness answers what a real page answers.  A stub returning 0 would mean "an
// empty file that EXISTS" where the contract says absent, and a missing import
// is a LinkError the moment a program under test touches a file.
import { loftFSImports } from '../doc/loft-fs.js';

const wasmPath = process.argv[2];
const storePath = process.env.LOFT_PAGED_FILE;
if (!wasmPath || !storePath) {
  console.error("usage: LOFT_PAGED_FILE=<store> node paged_range_host.mjs <wasm_file>");
  process.exit(2);
}

const store = fs.readFileSync(storePath);
let mem = null;
const dec = new TextDecoder();

// The response stash, mirroring the browser shell's `ctrl`: the import returns a LENGTH
// and the bytes are copied out separately by loft_host_http_get_copy.
let httpBytes = null;
let bytesFetched = 0;
let requests = 0;

const loft_io = {
  ...loftFSImports(() => mem),
  loft_host_print(ptr, len) {
    process.stdout.write(dec.decode(new Uint8Array(mem.buffer, ptr, len)));
  },
  loft_host_input_len: () => 0,
  loft_host_input_copy: () => {},
  loft_host_output(ptr, len) {
    process.stderr.write("[out] " + dec.decode(new Uint8Array(mem.buffer, ptr, len)));
  },
  // Whole-file GET is refused BY DEFAULT: the paged test must not pass over a regression
  // that silently falls back to a whole-image load. LOFT_WHOLE_GET=1 enables it for the
  // `store_load_url` tests, which are about that path on purpose.
  //
  // Those loaders ask for the layout sidecar first (`<url>.dschema`), and it is answered as
  // a server answers it: the `.dschema` beside the store when there is one, a 404 when there
  // is not — never the store's own bytes, which would read as an unreadable sidecar.
  loft_host_http_get: (ptr, len) => {
    if (!process.env.LOFT_WHOLE_GET) return 0xffffffff;
    if (dec.decode(new Uint8Array(mem.buffer, ptr, len)).endsWith(".dschema")) {
      if (!fs.existsSync(storePath + ".dschema")) return 0xffffffff;
      httpBytes = fs.readFileSync(storePath + ".dschema");
      requests += 1;
      return httpBytes.length;
    }
    httpBytes = store;
    bytesFetched += store.length;
    requests += 1;
    return store.length;
  },
  loft_host_http_get_copy: (ptr) => {
    if (httpBytes) new Uint8Array(mem.buffer, ptr, httpBytes.length).set(httpBytes);
  },
  loft_host_http_range: (_ptr, _len, off, n) => {
    // Clamp to EOF exactly as a Range server does — a request overhanging the end yields
    // the short tail, and the loft side zero-pads it.
    const start = Math.min(off, store.length);
    const end = Math.min(off + n, store.length);
    httpBytes = store.subarray(start, end);
    bytesFetched += httpBytes.length;
    requests += 1;
    return httpBytes.length;
  },
  loft_host_http_range_total: () => store.length,
  loft_host_deliver() {},
  loft_host_expose() {},
  loft_host_release() {},
};

try {
  const { instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { loft_io });
  mem = instance.exports.memory;
  if (typeof instance.exports.loft_start === "function") instance.exports.loft_start();
  else if (typeof instance.exports.main === "function") instance.exports.main();
  process.stdout.write(
    `PAGED bytes_fetched=${bytesFetched} file=${store.length} requests=${requests}\n`,
  );
  process.exit(0);
} catch (e) {
  console.error("paged_range_host: " + (e && e.stack ? e.stack : e));
  process.exit(1);
}
