// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// P137 regression harness: instantiate a loft `--html`-built WASM in
// Node with stub host imports, run `loft_start`, and report whether
// the module traps.
//
// Usage (release wasm):
//   node tools/wasm_repro.mjs <path/to/wasm_file> [--trace]
//
// Extracting the WASM from a `--html` HTML bundle:
//   python3 -c "import re,base64,sys;html=open(sys.argv[1]).read();\
//     m=re.search(r'wasmB64=\"([A-Za-z0-9+/=]+)\"',html);\
//     open(sys.argv[2],'wb').write(base64.b64decode(m.group(1)))" \
//     doc/brick-buster.html /tmp/bb.wasm
//
// Exit code 0 on clean run, 1 on trap.  The --trace flag enables a
// global buffer that records every host import call, printed on trap
// to narrow down which loft function last entered the host boundary
// before the fault.
//
// This harness found the P137 root cause: a `fn main() {}` HTML trap
// produced an empty trace (no host import reached), proving the bug
// was in WASM init, not in any host-call marshalling.

import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';
import zlib from 'node:zlib';
// loft#851 — the page's filesystem, imported rather than restubbed: this
// harness must exercise the SAME implementation a real page ships, or a
// round-trip proven here says nothing about the browser.  In node there is no
// localStorage, so the delta stays in memory for the run.
import { loftFS, loftFSImports } from '../doc/loft-fs.js';
// The page filesystem the WASI layer (`doc/loft-wasi.js`) reads a C-written module's files from.
globalThis.loftFS = loftFS;

const argv = process.argv.slice(2);
if (argv.length < 1) {
  console.error('usage: node wasm_repro.mjs <wasm_file> [--trace]');
  process.exit(2);
}

const wasmPath = argv[0];
const enableTrace = argv.includes('--trace');
const wasm = fs.readFileSync(wasmPath);
const trace = [];
let instance = null;

// @P321(c) Phase 3b — minimal PNG decoder so wasm_repro can stand in for
// the browser's createImageBitmap.  Supports 8-bit Grayscale / RGB / RGBA /
// Grayscale+Alpha non-interlaced PNGs (the only kind `lib/imaging` tests
// ship today).  Returns `{width, height, bytes}` with `bytes` being raw
// RGB triples (3 bytes/pixel) ready for the imaging bridge.
function paeth(a, b, c) {
  const p = a + b - c;
  const pa = Math.abs(p - a);
  const pb = Math.abs(p - b);
  const pc = Math.abs(p - c);
  if (pa <= pb && pa <= pc) return a;
  if (pb <= pc) return b;
  return c;
}
function decodePng(buf) {
  const sig = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
  for (let i = 0; i < 8; i++) if (buf[i] !== sig[i]) return null;
  let pos = 8;
  let width = 0,
    height = 0,
    bitDepth = 0,
    colorType = 0;
  const idat = [];
  while (pos + 8 <= buf.length) {
    const len = buf.readUInt32BE(pos);
    pos += 4;
    const type = buf.slice(pos, pos + 4).toString('ascii');
    pos += 4;
    const data = buf.slice(pos, pos + len);
    pos += len + 4; // skip CRC
    if (type === 'IHDR') {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      bitDepth = data[8];
      colorType = data[9];
    } else if (type === 'IDAT') {
      idat.push(data);
    } else if (type === 'IEND') {
      break;
    }
  }
  if (bitDepth !== 8) throw new Error(`unsupported bit depth ${bitDepth}`);
  const samplesByType = { 0: 1, 2: 3, 4: 2, 6: 4 };
  const samples = samplesByType[colorType];
  if (samples === undefined)
    throw new Error(`unsupported color type ${colorType} (palette PNG not decoded)`);
  const raw = zlib.inflateSync(Buffer.concat(idat));
  const stride = width * samples;
  const out = new Uint8Array(stride * height);
  const prev = new Uint8Array(stride);
  let ip = 0,
    op = 0;
  for (let y = 0; y < height; y++) {
    const filter = raw[ip++];
    for (let x = 0; x < stride; x++) {
      const left = x >= samples ? out[op + x - samples] : 0;
      const up = prev[x];
      const upLeft = x >= samples ? prev[x - samples] : 0;
      let v = raw[ip + x];
      switch (filter) {
        case 0:
          break;
        case 1:
          v = (v + left) & 0xff;
          break;
        case 2:
          v = (v + up) & 0xff;
          break;
        case 3:
          v = (v + ((left + up) >> 1)) & 0xff;
          break;
        case 4:
          v = (v + paeth(left, up, upLeft)) & 0xff;
          break;
        default:
          throw new Error(`unknown filter ${filter}`);
      }
      out[op + x] = v;
    }
    prev.set(out.subarray(op, op + stride));
    ip += stride;
    op += stride;
  }
  // Normalise to RGB.
  if (samples === 3) return { width, height, bytes: out };
  const rgb = new Uint8Array(width * height * 3);
  if (samples === 4) {
    for (let i = 0, j = 0; j < out.length; i += 3, j += 4) {
      rgb[i] = out[j];
      rgb[i + 1] = out[j + 1];
      rgb[i + 2] = out[j + 2];
    }
  } else if (samples === 1) {
    for (let i = 0, j = 0; j < out.length; i += 3, j += 1) {
      rgb[i] = rgb[i + 1] = rgb[i + 2] = out[j];
    }
  } else {
    // 2 = grayscale + alpha
    for (let i = 0, j = 0; j < out.length; i += 3, j += 2) {
      rgb[i] = rgb[i + 1] = rgb[i + 2] = out[j];
    }
  }
  return { width, height, bytes: rgb };
}

// Discover *.png siblings of the wasm file (the harness for the lib
// suite copies imaging test assets into the same per-test temp dir as
// the .wasm — see tests/html_wasm.rs).
const assets = {};
const wasmDir = path.dirname(path.resolve(wasmPath));
try {
  for (const entry of fs.readdirSync(wasmDir)) {
    if (entry.toLowerCase().endsWith('.png')) {
      try {
        const decoded = decodePng(fs.readFileSync(path.join(wasmDir, entry)));
        if (decoded) assets[entry] = decoded;
      } catch (e) {
        if (enableTrace) console.error(`png decode failed for ${entry}: ${e.message}`);
      }
    }
  }
} catch (e) {
  // No sibling dir — fine, just no assets.
}

const decoder = new TextDecoder();
function basename(p) {
  return p.split(/[\\/]/).pop();
}

// JS->loft input queue for the loft_io bridge below (seed via $LOFT_INPUT).
const inQ = [];
if (process.env.LOFT_INPUT)
  inQ.push(new TextEncoder().encode(process.env.LOFT_INPUT));

// Expected host imports for a `loft --html` bundle.  Loose stubs —
// we don't care what they return; we only care whether a trap fires
// during loft_start.  The proxy answers anything unrecognised with 0.
//
// `host_asset_exists` is library-agnostic (any wasm-asset library
// uses it), so it stays inline here.  Library-specific imports
// (`imaging_query`, `imaging_copy_rgb`, `imaging_save` for the
// imaging library) come from `lib/<X>/wasm/host.js` files — same
// mechanism the production `--html` HTML preamble uses.  Discovered
// + evaluated below so adding a new wasm-bridge library doesn't
// require editing the harness.
const loftGlExplicit = {
  host_asset_exists(pp, pl) {
    if (!instance) return 0;
    const mem = instance.exports.memory;
    const name = basename(decoder.decode(new Uint8Array(mem.buffer, pp, pl)));
    return assets[name] ? 1 : 0;
  },
};

const stubs = {
  loft_io: {
    // loft#851: the real page filesystem, not a stub.  It has to come first —
    // a read answering the proxy's blanket 0 would mean "an empty file that
    // exists" rather than "absent", which is the one distinction every file
    // test in this harness turns on.
    ...loftFSImports(() => instance.exports.memory),
    // @PLN97: store_load_url_trusted's fetch import is now in every --html
    // wasm; this repro never fetches, so stub to the error sentinel so the
    // module still links (an absent import would LinkError at instantiate).
    loft_host_http_get: () => 0xffffffff,
    loft_host_http_get_copy: () => {},
    // loft#678 working-set loaders: refused here like the whole-file GET above — these
    // harnesses never serve a store, and a MISSING import is a LinkError, not a skip.
    loft_host_http_range: () => 0xffffffff,
    loft_host_http_range_total: () => -1,
    // @PLN105: deliver/expose/release imports are linked by the FULL-interpreter
    // `--debug` client (its op dispatch includes OpDeliver/OpExpose/OpRelease),
    // so they must exist or the module LinkErrors — this repro never delivers, so
    // no-ops suffice.
    loft_host_deliver: () => {},
    loft_host_expose: () => {},
    loft_host_release: () => {},
    // #620: the browser clock bridge.  REAL values, not the usual loose stub —
    // the point of the guard that uses them is that a browser `now()`/`ticks()`
    // advances, so a 0 stub here would make that test vacuously pass.
    loft_host_time_now_ms: () => Date.now(),
    loft_host_time_ticks_us: () => performance.now() * 1000,
    loft_host_print: (ptr, len) => {
      if (enableTrace) trace.push(`loft_host_print(${ptr}, ${len})`);
      if (instance) {
        const mem = instance.exports.memory;
        const bytes = new Uint8Array(mem.buffer, ptr, len);
        const s = new TextDecoder().decode(bytes);
        if (!enableTrace) process.stdout.write(s);
      }
    },
    // The JS->loft input QUEUE (host_input pops one message per call):
    // seeded from $LOFT_INPUT, extended by the loft_host_output loopback
    // below — the headless stand-in for a page's loftPush().
    loft_host_input_len: () => (inQ.length ? inQ[0].length : 0),
    loft_host_input_copy: (ptr) => {
      const b = inQ.shift();
      if (b && instance)
        new Uint8Array(instance.exports.memory.buffer, ptr, b.length).set(b);
    },
    // loft->JS structured messages (host_output): the harness LOOPBACK —
    // every message is echoed straight back into the input queue as
    // "echo:<msg>", a deterministic stand-in for "the page acts on the
    // request (fetch etc.) and loftPush'es the completion".
    loft_host_output: (ptr, len) => {
      if (!instance) return;
      const s = new TextDecoder().decode(
        new Uint8Array(instance.exports.memory.buffer, ptr, len));
      if (enableTrace) trace.push(`loft_host_output(${JSON.stringify(s)})`);
      inQ.push(new TextEncoder().encode("echo:" + s));
    },
  },
  loft_gl: new Proxy(loftGlExplicit, {
    get(target, name) {
      if (name in target) return target[name];
      return (...args) => {
        if (enableTrace) trace.push(`loft_gl.${String(name)}(${args.length} args)`);
        return 0;
      };
    },
  }),
};

// @lib_plan-29 W3 — discover + load every `lib/<X>/wasm/host.js` in
// the workspace.  Each file pushes a registration callback onto
// `globalThis.LOFT_WASM_EXTENSIONS`; the dispatch loop below applies
// each to the stubs object so library-specific imports
// (`imaging_query` etc.) become available without the harness
// hard-coding them.  Mirrors the production `--html` driver's
// concatenation step.
const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.dirname(harnessDir);
const libDir = path.join(repoRoot, 'lib');
globalThis.LOFT_WASM_EXTENSIONS = [];
try {
  for (const pkg of fs.readdirSync(libDir).sort()) {
    const hostJs = path.join(libDir, pkg, 'wasm', 'host.js');
    if (!fs.existsSync(hostJs)) continue;
    try {
      const code = fs.readFileSync(hostJs, 'utf8');
      // Use indirect eval (`(0, eval)`) so the host.js code runs in
      // the global scope (where `globalThis.LOFT_WASM_EXTENSIONS` is
      // the one we just set).
      (0, eval)(code);
    } catch (e) {
      if (enableTrace) console.error(`host.js load failed for ${hostJs}: ${e.message}`);
    }
  }
} catch (_e) {
  // No lib/ dir — fine, no extensions to load.
}

// Also load explicit host.js paths from $LOFT_WASM_HOST_JS (colon-separated) —
// for library bridges that live outside <repoRoot>/lib (e.g. loft-libs-core
// packages built with `--lib`), so their host imports can be tested headlessly.
for (const hostJs of (process.env.LOFT_WASM_HOST_JS || '').split(':').filter(Boolean)) {
  try {
    (0, eval)(fs.readFileSync(hostJs, 'utf8'));
  } catch (e) {
    if (enableTrace) console.error(`host.js load failed for ${hostJs}: ${e.message}`);
  }
}

// The WASI layer a page carries for a module the C backend wrote (@PLN203 C6): the same file
// the `--html` driver adds, so a run here exercises what a page ships.
try {
  (0, eval)(fs.readFileSync(path.join(repoRoot, 'doc', 'loft-wasi.js'), 'utf8'));
} catch (e) {
  if (enableTrace) console.error(`loft-wasi.js load failed: ${e.message}`);
}

// Apply each registered extension to the stubs object BEFORE
// instantiate, so the imports the wasm sees include everything.
// `ctrl` wraps the asset table the same way the production preamble
// does; `getMem` returns the instance's memory once instantiated.
const reproCtrl = { assets };
const reproGetMem = () => (instance ? instance.exports.memory : { buffer: new ArrayBuffer(0) });
for (const reg of globalThis.LOFT_WASM_EXTENSIONS) {
  try {
    reg(stubs, reproCtrl, reproGetMem);
  } catch (e) {
    if (enableTrace) console.error(`host.js extension dispatch failed: ${e.message}`);
  }
}

const mod = new WebAssembly.Module(wasm);
try {
  instance = new WebAssembly.Instance(mod, stubs);
} catch (e) {
  console.error(`instantiate failed: ${e.message}`);
  process.exit(1);
}

console.error(`exports: ${Object.keys(instance.exports).join(',')}`);

try {
  instance.exports.loft_start();
  console.error('loft_start: OK');
  process.exit(0);
} catch (e) {
  console.error(`TRAP: ${e.message}`);
  if (enableTrace) {
    console.error('last trace entries:');
    for (const entry of trace.slice(-20)) console.error(`  ${entry}`);
  }
  console.error('stack:');
  console.error(e.stack);
  process.exit(1);
}
