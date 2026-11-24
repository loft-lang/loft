// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Bundle-integrity gate for `loft --html` output (e.g. doc/brick-buster.html).
//
// Usage:  node tools/check_html_bundle.mjs <path/to/bundle.html>
//
// Exit 0 = the embedded WASM is shippable; non-zero with a diagnostic = it is
// not.  This catches the two toolchain footguns that produced a broken
// doc/brick-buster.html in the past (see @P337):
//
//   1. rlib STOMP — `make wasm` (wasm-pack, `feature=wasm`) and `loft --html`
//      write the SAME target/wasm32-unknown-unknown/release/libloft.rlib with
//      incompatible feature sets.  If `--html` links the wasm-bindgen variant,
//      the bundle imports `__wbindgen_placeholder__` (35+ entries) which the
//      embedded loft-gl-wasm.js glue does not provide → the page fails to
//      instantiate ("Import ... __wbindgen_placeholder__: module is not an
//      object").  A correct --html bundle imports ONLY `loft_gl` + `loft_io`, and the
//      WASI host its page provides.
//
//   2. MISSING asyncify — if `wasm-opt` (binaryen) was not on PATH at build
//      time, the `--asyncify` pass never ran, so there is no frame-yield.  The
//      HTML driver then calls loft_start() synchronously and a render loop
//      (`for _ in 0..N`) never returns control to the browser → the tab hangs
//      ("page times out").  A correct bundle exports `asyncify_start_unwind`.
//
// The check is static (parse imports/exports) plus a stub instantiate; it needs
// no GL context, so it runs anywhere Node runs.  It does NOT execute the game.

import fs from 'node:fs';

const path = process.argv[2];
if (!path) {
  console.error('usage: node tools/check_html_bundle.mjs <bundle.html>');
  process.exit(2);
}

const html = fs.readFileSync(path, 'utf8');
const m = html.match(/wasmB64="([A-Za-z0-9+/=]+)"/);
if (!m) {
  console.error(`FAIL: no embedded wasmB64 found in ${path}`);
  process.exit(1);
}
const wasm = Buffer.from(m[1], 'base64');

let mod;
try {
  mod = new WebAssembly.Module(wasm);
} catch (e) {
  console.error(`FAIL: embedded WASM does not compile: ${e.message}`);
  process.exit(1);
}

// (1) Import modules must be exactly the raw extern bridges the HTML provides: `loft_gl` and
// `loft_io`, and `wasi_snapshot_preview1` where the page carries its WASI host (a module the C
// backend wrote, through clang's WASI libc).  A page whose module runs `par` on Web Workers
// (plans/227) also imports its shared memory (`env`), the libc's thread start (`wasi`) and the
// pool's idle call (`loft_page`), which the page's thread script answers.
const importMods = [...new Set(WebAssembly.Module.imports(mod).map((i) => i.module))];
const pageWasi = html.includes('imports.wasi_snapshot_preview1');
const pagePool = html.includes("'thread-spawn'");
const badMods = importMods.filter(
  (mm) =>
    mm !== 'loft_gl' &&
    mm !== 'loft_io' &&
    !(mm === 'wasi_snapshot_preview1' && pageWasi) &&
    !(['env', 'wasi', 'loft_page'].includes(mm) && pagePool),
);
if (badMods.length) {
  console.error(
    `FAIL: unexpected import module(s) ${JSON.stringify(badMods)}.\n` +
      '       The wasm32-unknown-unknown libloft.rlib was built with the `wasm`\n' +
      '       (wasm-bindgen) feature — most likely `make wasm` stomped it.\n' +
      '       Rebuild the rlib in the --html shape, then re-run --html:\n' +
      '         cargo build --release --target wasm32-unknown-unknown --lib \\\n' +
      '             --no-default-features --features random',
  );
  process.exit(1);
}

// (2) asyncify must be present, or the render loop will hang the browser tab.
const exps = WebAssembly.Module.exports(mod).map((e) => e.name);
if (!exps.includes('asyncify_start_unwind')) {
  console.error(
    'FAIL: no asyncify exports — `wasm-opt` (binaryen) did not run, so the\n' +
      '       bundle cannot frame-yield and the page will hang.  Install\n' +
      '       wasm-opt (binaryen) and rebuild.',
  );
  process.exit(1);
}

// (3) Sanity: instantiate with no-op stub imports (no GL/host work runs here).
try {
  const stub = new Proxy({}, { get: () => new Proxy({}, { get: () => () => 0 }) });
  // An imported memory is a memory (a threaded page's), at least as large as a page declares.
  const memImport = WebAssembly.Module.imports(mod).find((i) => i.kind === 'memory');
  const imports = memImport
    ? new Proxy({}, {
      get: (_, m) => (m === memImport.module
        ? { [memImport.name]: new WebAssembly.Memory({ initial: 4096, maximum: 16384, shared: true }) }
        : stub[m]),
    })
    : stub;
  // eslint-disable-next-line no-new
  new WebAssembly.Instance(mod, imports);
} catch (e) {
  console.error(`FAIL: bundle does not instantiate even with stub imports: ${e.message}`);
  process.exit(1);
}

console.log(
  `OK: ${path} — imports ${JSON.stringify(importMods)} only, asyncify present, instantiates.`,
);
