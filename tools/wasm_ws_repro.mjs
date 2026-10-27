// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN84 ZT-C — asyncify-aware headless Node driver for the `web`
// library's WebSocket wasm bridge.
//
// `tools/wasm_repro.mjs` runs `loft_start()` SYNCHRONOUSLY then exits, so
// it can never drive an event-driven WebSocket client (the JS event loop
// never runs between poll iterations, so `onmessage` never fires — see
// plans/84-zt-c-web-ws-bridge.md §4.1).  This driver instead mirrors the
// browser `--html` preamble: it detects the `asyncify_start_unwind`
// export, builds an AsyncifyCtrl (ported from doc/loft-gl-wasm.js:44-84),
// runs `loft_start()` until the first `loft_web.ws_yield` suspend, then
// resumes on a `setImmediate` loop (Node's requestAnimationFrame
// equivalent).  Between unwind and the next resume the event loop runs —
// that is when `WebSocket.onmessage` delivers a frame and fills the host
// inbound queue.
//
// Usage:
//   node tools/wasm_ws_repro.mjs <wasm_file> [--trace] [--max-frames N]
//
// The driver loads loft's own `doc/loft-web.js` (the `loft_web` imports a page carries), then
// host.js bridges the same way wasm_repro.mjs does:
//   * every <repoRoot>/lib/<X>/wasm/host.js, plus
//   * colon-separated paths in $LOFT_WASM_HOST_JS (for libs outside lib/).
// Node 22 has a global WebSocket, so a library's host.js that creates
// `new WebSocket(url)` runs unchanged here.
//
// Exit code 0 on a clean `loft_start` completion, 1 on a trap, 2 on a
// usage error or if the program never finished within --max-frames.

import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';
// loft#851 — the page's filesystem, imported rather than restubbed so every
// harness answers what a real page answers.  A stub returning 0 would mean "an
// empty file that EXISTS" where the contract says absent, and a missing import
// is a LinkError the moment a program under test touches a file.
import { loftFSImports } from '../doc/loft-fs.js';

const argv = process.argv.slice(2);
if (argv.length < 1) {
  console.error('usage: node wasm_ws_repro.mjs <wasm_file> [--trace] [--max-frames N]');
  process.exit(2);
}
const wasmPath = argv[0];
const enableTrace = argv.includes('--trace');
let maxFrames = 100000;
const mfIdx = argv.indexOf('--max-frames');
if (mfIdx >= 0 && argv[mfIdx + 1]) maxFrames = parseInt(argv[mfIdx + 1], 10);

const wasm = fs.readFileSync(wasmPath);
const trace = [];
let instance = null;
const decoder = new TextDecoder();

// ── AsyncifyCtrl — ported from doc/loft-gl-wasm.js:44-84 ─────────────────
// Drives the suspend/rewind handshake against the wasm-opt --asyncify ABI.
// The wasm allocates the unwind buffer in linear memory just above
// __heap_base; `suspend()` (called from the ws_yield host import) starts
// the unwind, `resume()` rewinds and re-enters the entry fn.
// CORRECTED for the headless resume loop (the browser GL path tolerates the
// original port; a Node-driven poll loop needs the full ABI right).  Three
// rules this gets right that the GL port glosses — all proven by
// tools/zt-c-web-staging/asyncify_probe.rs:
//   1. The save-buffer pointer (`current`, struct field @0) starts at the
//      buffer base for an UNWIND (asyncify writes upward) and at the SAVED
//      TOP for a REWIND (asyncify reads downward).  Resetting `current` to
//      base before rewind loses the saved frames; the program never advances.
//   2. The suspend shim is STATE-AWARE: when called while REWINDING (state 2)
//      it must `stop_rewind` and RETURN so execution continues past the yield.
//      Re-unwinding there spins forever on one loop iteration.
//   3. `current` after an unwind is the saved stack top — keep it for the
//      matching rewind.
function AsyncifyCtrl(inst) {
  const DATA_ADDR = inst.exports.__heap_base?.value || 65536;
  const STACK_SIZE = 16384;
  const E = inst.exports;
  this.sleeping = false;
  this.exports = E;
  let savedTop = DATA_ADDR + 8;
  const STATE_REWINDING = 2;

  const setStruct = (cur, end) => {
    const mem = new Int32Array(E.memory.buffer);
    mem[DATA_ADDR >> 2] = cur;
    mem[(DATA_ADDR + 4) >> 2] = end;
  };
  const curPtr = () => new Int32Array(E.memory.buffer)[DATA_ADDR >> 2];

  this.start = function (fn) {
    this.sleeping = false;
    E[fn]();
    if (this.sleeping) {
      savedTop = curPtr();
      E.asyncify_stop_unwind();
    }
  };

  this.resume = function (fn) {
    if (!this.sleeping) return false;
    this.sleeping = false;
    // current = saved top (rewind reads downward); end = buffer top.
    setStruct(savedTop, DATA_ADDR + 8 + STACK_SIZE);
    E.asyncify_start_rewind(DATA_ADDR);
    E[fn]();
    if (this.sleeping) {
      savedTop = curPtr();
      E.asyncify_stop_unwind();
    }
    return true;
  };

  // Called from a host import (e.g. loft_web.ws_yield) to yield a frame.
  // While REWINDING this call is the replay reaching the original yield
  // point — stop the rewind and return so the loop continues; while NORMAL,
  // start an unwind so control returns to the JS event loop.
  this.suspend = function () {
    if (E.asyncify_get_state() === STATE_REWINDING) {
      E.asyncify_stop_rewind();
      return;
    }
    this.sleeping = true;
    setStruct(DATA_ADDR + 8, DATA_ADDR + 8 + STACK_SIZE);
    E.asyncify_start_unwind(DATA_ADDR);
  };
}

// ── ctrl wrapper handed to each host.js extension (mirrors --html) ───────
// `ctrl.ac` is the AsyncifyCtrl once instantiated; a host.js suspend shim
// (loft_web.ws_yield) calls `ctrl.ac.suspend()`.  `getMem()` re-fetches
// the live memory (it can grow/detach across a resume).
const ctrl = { ac: null, assets: {} };
const getMem = () => (instance ? instance.exports.memory : { buffer: new ArrayBuffer(0) });

// Base import stubs: loft_io.loft_host_print routes program output to
// stdout; loft_gl is a permissive proxy (a headless WS client touches no
// GL, but the import section may still declare loft_gl entries).
const imports = {
  loft_io: {
    ...loftFSImports(getMem),
    // @PLN97: store_load_url_trusted's fetch import is now in every --html
    // wasm; a WS repro never fetches, so stub to the error sentinel so the
    // module links (an absent import would LinkError at instantiate).
    loft_host_http_get: () => 0xffffffff,
    loft_host_http_get_copy: () => {},
    // loft#678 working-set loaders: refused here like the whole-file GET above — these
    // harnesses never serve a store, and a MISSING import is a LinkError, not a skip.
    loft_host_http_range: () => 0xffffffff,
    loft_host_http_range_total: () => -1,
    loft_host_print: (ptr, len) => {
      if (instance) {
        const bytes = new Uint8Array(getMem().buffer, ptr, len);
        process.stdout.write(decoder.decode(bytes));
      }
    },
  },
  loft_gl: new Proxy(
    {},
    {
      get(_t, name) {
        return (...args) => {
          if (enableTrace) trace.push(`loft_gl.${String(name)}(${args.length} args)`);
          return 0;
        };
      },
    },
  ),
};

// ── load library host.js bridges (same discovery as wasm_repro.mjs) ──────
const harnessDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.dirname(harnessDir);
globalThis.LOFT_WASM_EXTENSIONS = [];
const libDir = path.join(repoRoot, 'lib');
try {
  for (const pkg of fs.readdirSync(libDir).sort()) {
    const hostJs = path.join(libDir, pkg, 'wasm', 'host.js');
    if (!fs.existsSync(hostJs)) continue;
    try {
      (0, eval)(fs.readFileSync(hostJs, 'utf8'));
    } catch (e) {
      if (enableTrace) console.error(`host.js load failed for ${hostJs}: ${e.message}`);
    }
  }
} catch (_e) {
  /* no lib/ dir */
}
// loft's own `loft_web` imports (the web library's natives, `src/web_browser.rs`), which the
// `--html` page carries whenever the module imports them.
try {
  (0, eval)(fs.readFileSync(path.join(repoRoot, 'doc', 'loft-web.js'), 'utf8'));
} catch (e) {
  console.error(`doc/loft-web.js load failed: ${e.message}`);
}
for (const hostJs of (process.env.LOFT_WASM_HOST_JS || '').split(':').filter(Boolean)) {
  try {
    (0, eval)(fs.readFileSync(hostJs, 'utf8'));
  } catch (e) {
    console.error(`host.js load failed for ${hostJs}: ${e.message}`);
  }
}
for (const reg of globalThis.LOFT_WASM_EXTENSIONS) {
  try {
    reg(imports, ctrl, getMem);
  } catch (e) {
    if (enableTrace) console.error(`host.js extension dispatch failed: ${e.message}`);
  }
}

// ── instantiate ──────────────────────────────────────────────────────────
const mod = new WebAssembly.Module(wasm);
const importedModules = new Set(WebAssembly.Module.imports(mod).map((i) => i.module));
if (enableTrace) console.error(`import modules: ${[...importedModules].join(',')}`);
try {
  instance = new WebAssembly.Instance(mod, imports);
} catch (e) {
  console.error(`instantiate failed: ${e.message}`);
  process.exit(1);
}
ctrl.ac = instance.exports.asyncify_start_unwind ? new AsyncifyCtrl(instance) : null;
if (enableTrace) console.error(`exports: ${Object.keys(instance.exports).join(',')}`);

// ── run: synchronous if no asyncify, else the suspend/resume loop ────────
function finishOk() {
  console.error('loft_start: OK');
  process.exit(0);
}
function finishTrap(e) {
  console.error(`TRAP: ${e.message}`);
  if (enableTrace) {
    console.error('last trace entries:');
    for (const entry of trace.slice(-20)) console.error(`  ${entry}`);
  }
  console.error(e.stack);
  process.exit(1);
}

if (!ctrl.ac) {
  // No asyncify export — compute-only bundle; run straight through.
  try {
    instance.exports.loft_start();
    finishOk();
  } catch (e) {
    finishTrap(e);
  }
} else {
  const ac = ctrl.ac;
  let frames = 0;
  try {
    ac.start('loft_start');
  } catch (e) {
    finishTrap(e);
  }
  if (!ac.sleeping) {
    // Ran to completion without ever yielding.
    finishOk();
  } else {
    const pump = () => {
      frames += 1;
      if (enableTrace) console.error(`[resume ${frames}]`);
      if (frames > maxFrames) {
        console.error(`did not finish within --max-frames ${maxFrames}`);
        process.exit(2);
      }
      let again;
      try {
        again = ac.resume('loft_start');
      } catch (e) {
        finishTrap(e);
        return;
      }
      if (again && ac.sleeping) {
        // Still yielding — schedule the next resume AFTER the event loop
        // drains (WebSocket onmessage/onopen run here).
        setImmediate(pump);
      } else {
        // loft_start returned for real — program done.
        finishOk();
      }
    };
    setImmediate(pump);
  }
}
