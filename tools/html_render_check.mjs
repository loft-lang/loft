#!/usr/bin/env node
// tools/html_render_check.mjs — load an HTML page in headless Chrome
// + capture JS console events / exceptions via the Chrome DevTools
// Protocol.  Exit code 0 if no console errors / exceptions.  Exit
// code 1 with a JSON summary on stderr otherwise.
//
// Used by tests/html_render.rs to gate browser-WebGL rendering.  The
// recurring failure mode for `--html` artefacts (shader compile errors,
// canvas-init exceptions, missing WebGL2 contexts) is a JS-side
// console.error — invisible to the WASM-side success/failure signal
// that tests/html_wasm.rs uses.  This harness wires the CDP socket
// directly so those events become a hard fail.
//
// Usage:
//   node tools/html_render_check.mjs <url> [--wait-ms N] [--ready EXPR]
//                                          [--screenshot PATH] [--port N]
//                                          [--allow SUBSTRING]
//                                          [--canvas SELECTOR]
//                                          [--canvas-min-colors N]
//                                          [--device-scale-factor N]
//                                          [--assert EXPR]
//
// `--ready EXPR` and `--assert EXPR` are the page-state pair, and they do NOT
// coerce alike.  `--ready` evaluates `!!(EXPR)` and POLLS every 50 ms until it
// is true or `--wait-ms` runs out; `--assert` evaluates `JSON.stringify(EXPR)`
// ONCE after the wait and fails unless the result is the literal `'true'`.  So
// a truthy-but-not-`true` expression — an element, a non-empty string, a
// number — satisfies `--ready` and FAILS `--assert`.  The difference presents
// as a flaky gate rather than as a contract, which is why it is stated here.
//
// Write `--assert` as a boolean:
//
//   --assert "!['Failed','Error'].includes(document.getElementById('s').textContent)"
//
// never as the bare `….textContent`, which stringifies to `"\"Running\""` on a
// WORKING page and so fails everywhere — a gate that passes nothing and
// catches a real defect only by coincidence.
//
// Their division of labour, stated where the check is made: `ready` means the
// page did not get there, `assert` means it did and the claim is false.
// Without `--ready` the wait is a flat sleep, so a slow load yields a
// pass-shaped absence.
//
// `--allow SUBSTRING` suppresses a console error / exception when its
// text contains the given substring (repeatable).  Plain substring
// match — no regex — to avoid the regex-injection / ReDoS class flagged
// by CodeQL (this tool runs against trusted local URLs but the API
// stays defensible).
//
// `--canvas SELECTOR` enables Layer 2: capture a clipped screenshot of
// the canvas element and count distinct RGB triples in the result.  If
// fewer than `--canvas-min-colors` (default 20) distinct colors are
// present, fail.  Catches the "compiles clean, blank canvas" pattern
// that Layer 1 (console-error-only) can't see — a WebGL2 context that
// never gets drawn into stays in clearColor and the screenshot is one
// uniform color.
//
// Skip protocol — if google-chrome is not installed, exit 2 with a
// `SKIP: …` line on stderr (callers turn this into a test skip).

import http from 'node:http';
import net from 'node:net';
import crypto from 'node:crypto';
import zlib from 'node:zlib';
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import process from 'node:process';

// ── CLI ────────────────────────────────────────────────────────────
// `--device-scale-factor N` runs Chrome at that ratio; paired with an
// `--assert` about the result, the two gate DPI behaviour, which no pixel
// check can see.  The flag list and the `--ready`/`--assert` coercion
// difference live in the header — this note is only why these two pair up.
const args = process.argv.slice(2);
if (args.length < 1) {
  console.error('usage: html_render_check.mjs <url> [--wait-ms N] [--ready EXPR] [--screenshot PATH] [--port N] [--allow SUBSTRING] [--canvas SELECTOR] [--canvas-min-colors N] [--device-scale-factor N] [--assert EXPR]');
  process.exit(64);
}
const URL_ARG = args[0];
const opts = {
  waitMs: 8000, screenshot: null, port: 9222, allow: [],
  canvasSelector: null, canvasMinColors: 20,
  deviceScaleFactor: null, assertExpr: null, readyExpr: null,
};
for (let i = 1; i < args.length; i++) {
  if (args[i] === '--wait-ms') opts.waitMs = parseInt(args[++i], 10);
  else if (args[i] === '--screenshot') opts.screenshot = args[++i];
  else if (args[i] === '--port') opts.port = parseInt(args[++i], 10);
  else if (args[i] === '--allow') opts.allow.push(args[++i]);
  else if (args[i] === '--canvas') opts.canvasSelector = args[++i];
  else if (args[i] === '--canvas-min-colors') opts.canvasMinColors = parseInt(args[++i], 10);
  // Layer 3: run the page at a forced device pixel ratio and assert a
  // property of the result.  A DPI bug is invisible to Layers 1 and 2 —
  // an upscaled canvas logs no error and has plenty of distinct colours.
  else if (args[i] === '--device-scale-factor') opts.deviceScaleFactor = parseFloat(args[++i]);
  else if (args[i] === '--assert') opts.assertExpr = args[++i];
  else if (args[i] === '--ready') opts.readyExpr = args[++i];
  else { console.error('unknown flag: ' + args[i]); process.exit(64); }
}

// ── Chrome availability ────────────────────────────────────────────
const CHROME_NAMES = ['google-chrome', 'chromium', 'chromium-browser', 'chrome'];
let CHROME = null;
for (const name of CHROME_NAMES) {
  const r = spawnSync('sh', ['-c', `command -v ${name}`], { encoding: 'utf8' });
  if (r.stdout && r.stdout.trim()) { CHROME = r.stdout.trim(); break; }
}
if (!CHROME) {
  console.error('SKIP: no chrome binary in PATH (tried: ' + CHROME_NAMES.join(', ') + ')');
  process.exit(2);
}

// ── Spawn Chrome ───────────────────────────────────────────────────
// Headless WebGL2 requires SwiftShader (no GPU in CI) — the three
// `--*-angle*` / `--enable-unsafe-swiftshader` flags switch the GL
// backend to the software path so the page gets a real WebGL2 context
// regardless of GPU availability.
const chrome = spawn(CHROME, [
  '--headless=new',
  '--no-sandbox',
  '--remote-debugging-port=' + opts.port,
  '--window-size=1024,768',
  ...(opts.deviceScaleFactor
    ? ['--force-device-scale-factor=' + opts.deviceScaleFactor]
    : []),
  '--enable-unsafe-swiftshader',
  '--use-gl=angle',
  '--use-angle=swiftshader',
  '--mute-audio',
  '--hide-scrollbars',
  'about:blank',
], { stdio: ['ignore', 'pipe', 'pipe'] });

let chromeErr = '';
chrome.stderr.on('data', d => { chromeErr += d.toString(); });
chrome.on('exit', code => {
  if (code !== null && code !== 0 && !chrome._intentionalExit) {
    console.error('chrome exited unexpectedly: code=' + code + '\n' + chromeErr.slice(-500));
  }
});

// ── Helpers ────────────────────────────────────────────────────────
function getJson(path) {
  return new Promise((resolve, reject) => {
    http.get('http://localhost:' + opts.port + path, r => {
      let body = '';
      r.on('data', c => body += c);
      r.on('end', () => { try { resolve(JSON.parse(body)); } catch (e) { reject(e); } });
    }).on('error', reject);
  });
}
function putJson(path) {
  return new Promise((resolve, reject) => {
    const req = http.request({ host: 'localhost', port: opts.port, method: 'PUT', path }, r => {
      let body = '';
      r.on('data', c => body += c);
      r.on('end', () => { try { resolve(JSON.parse(body)); } catch (e) { reject(e); } });
    });
    req.on('error', reject); req.end();
  });
}
async function waitFor(check, maxTries = 50) {
  for (let i = 0; i < maxTries; i++) {
    try { const r = await check(); if (r) return r; } catch (e) {}
    await new Promise(r => setTimeout(r, 100));
  }
  throw new Error('timeout waiting for ' + check.toString().slice(0, 80));
}

// ── WebSocket (minimal RFC6455 client) ─────────────────────────────
function parseWsUrl(url) {
  const m = url.match(/^ws:\/\/([^:]+):(\d+)(\/.*)$/);
  if (!m) throw new Error('bad ws url: ' + url);
  return { host: m[1], port: parseInt(m[2], 10), path: m[3] };
}
async function openWebSocket(url) {
  const u = parseWsUrl(url);
  const sock = net.createConnection(u.port, u.host);
  await new Promise((resolve, reject) => {
    sock.once('connect', resolve);
    sock.once('error', reject);
  });
  const key = crypto.randomBytes(16).toString('base64');
  sock.write(
    `GET ${u.path} HTTP/1.1\r\nHost: ${u.host}:${u.port}\r\n` +
    `Upgrade: websocket\r\nConnection: Upgrade\r\n` +
    `Sec-WebSocket-Key: ${key}\r\nSec-WebSocket-Version: 13\r\n\r\n`
  );
  let buf = Buffer.alloc(0);
  await new Promise(resolve => sock.once('data', d => { buf = d; resolve(); }));
  const headerEnd = buf.indexOf('\r\n\r\n');
  buf = buf.slice(headerEnd + 4);
  return { sock, buf };
}
function encodeFrame(text) {
  const payload = Buffer.from(text, 'utf8');
  const mask = crypto.randomBytes(4);
  const len = payload.length;
  let header;
  if (len < 126) {
    header = Buffer.from([0x81, 0x80 | len]);
  } else if (len < 65536) {
    header = Buffer.from([0x81, 0xfe, (len >> 8) & 0xff, len & 0xff]);
  } else {
    header = Buffer.alloc(10);
    header[0] = 0x81; header[1] = 0xff;
    header.writeBigUInt64BE(BigInt(len), 2);
  }
  const masked = Buffer.alloc(payload.length);
  for (let i = 0; i < payload.length; i++) masked[i] = payload[i] ^ mask[i % 4];
  return Buffer.concat([header, mask, masked]);
}

// ── PNG decode (just enough to count distinct RGB triples) ────────
// Layer 2 wants to know "is the canvas blank?".  We get a PNG
// screenshot from CDP, decompress the IDAT chunks via node:zlib, undo
// the per-row filters, and count distinct (R,G,B) tuples.  A canvas
// that never got drawn into is one uniform color (Chrome's default
// black, or the WebGL clearColor); a working frame has dozens to
// thousands of distinct colors.
//
// We don't need a general PNG decoder — Chrome's captureScreenshot
// always emits RGBA8 (`colorType=6`, `bitDepth=8`), no interlace.  A
// dozen lines of code cover that.
function decodePngRgb(pngBuf) {
  if (pngBuf.readUInt32BE(0) !== 0x89504e47 || pngBuf.readUInt32BE(4) !== 0x0d0a1a0a) {
    throw new Error('not a PNG');
  }
  let off = 8;
  let width = 0, height = 0, bitDepth = 0, colorType = 0;
  const idatChunks = [];
  while (off < pngBuf.length) {
    const len = pngBuf.readUInt32BE(off);
    const type = pngBuf.slice(off + 4, off + 8).toString('ascii');
    const data = pngBuf.slice(off + 8, off + 8 + len);
    if (type === 'IHDR') {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      bitDepth = data[8];
      colorType = data[9];
      if (data[10] !== 0 || data[11] !== 0 || data[12] !== 0) {
        throw new Error('unsupported PNG: compression/filter/interlace != 0');
      }
    } else if (type === 'IDAT') {
      idatChunks.push(data);
    } else if (type === 'IEND') {
      break;
    }
    off += 12 + len;
  }
  if (bitDepth !== 8) throw new Error('unsupported PNG bit depth: ' + bitDepth);
  // colorType 6 = RGBA, 2 = RGB, 0 = gray, 4 = gray+alpha
  let bpp;
  if (colorType === 6) bpp = 4;
  else if (colorType === 2) bpp = 3;
  else if (colorType === 4) bpp = 2;
  else if (colorType === 0) bpp = 1;
  else throw new Error('unsupported PNG color type: ' + colorType);
  const raw = zlib.inflateSync(Buffer.concat(idatChunks));
  const stride = width * bpp;
  const out = Buffer.alloc(height * stride);
  // Un-filter each scanline.  Filter byte precedes each row.
  for (let y = 0; y < height; y++) {
    const filt = raw[y * (stride + 1)];
    const src = raw.slice(y * (stride + 1) + 1, y * (stride + 1) + 1 + stride);
    const prev = y === 0 ? null : out.slice((y - 1) * stride, y * stride);
    const dst = out.slice(y * stride, (y + 1) * stride);
    for (let x = 0; x < stride; x++) {
      const a = x >= bpp ? dst[x - bpp] : 0;
      const b = prev ? prev[x] : 0;
      const c = (prev && x >= bpp) ? prev[x - bpp] : 0;
      let val;
      if (filt === 0) val = src[x];
      else if (filt === 1) val = (src[x] + a) & 0xff;
      else if (filt === 2) val = (src[x] + b) & 0xff;
      else if (filt === 3) val = (src[x] + ((a + b) >> 1)) & 0xff;
      else if (filt === 4) {
        // Paeth
        const p = a + b - c;
        const pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
        const pr = (pa <= pb && pa <= pc) ? a : (pb <= pc ? b : c);
        val = (src[x] + pr) & 0xff;
      } else throw new Error('unknown PNG filter: ' + filt);
      dst[x] = val;
    }
  }
  // Walk the de-filtered RGBA/RGB buffer, count distinct RGB triples.
  const seen = new Set();
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const off = y * stride + x * bpp;
      const r = out[off];
      const g = bpp >= 2 ? out[off + 1] : r;
      const b = bpp >= 3 ? out[off + 2] : r;
      seen.add((r << 16) | (g << 8) | b);
    }
  }
  return { width, height, distinctColors: seen.size };
}

// ── Main ───────────────────────────────────────────────────────────
(async () => {
  let exitCode = 0;
  // REACHING the browser is a precondition, not a measurement — so failing to reach it
  // is the same answer as not having one: SKIP (exit 2), never a failure.
  //
  // A missing chrome binary already skipped.  A chrome that is INSTALLED and never
  // answers its debugging port fell into the generic catch below and exited 3, which the
  // caller reads as "the page was wrong" — so a CI runner whose chrome could not start
  // reported a red probe that had rendered nothing and asserted nothing.  Observed on
  // `main` since at least 2026-08-06: `harness error: timeout waiting for () =>
  // getJson('/json/version')`, three retries, no pixels ever produced.
  //
  // Everything AFTER this point keeps its failure codes: once the browser answers, a bad
  // page is a real result and must stay red.
  try {
    await waitFor(() => getJson('/json/version'));
  } catch (e) {
    console.error('SKIP: chrome is installed but never answered its debugging port ('
      + e.message + ')');
    if (chromeErr) console.error('chrome stderr (last 500):\n' + chromeErr.slice(-500));
    chrome._intentionalExit = true;
    chrome.kill();
    process.exit(2);
  }
  try {
    const tab = await putJson('/json/new?' + encodeURIComponent(URL_ARG));
    const wsState = await openWebSocket(tab.webSocketDebuggerUrl);
    let { sock } = wsState;
    let buf = wsState.buf;
    let nextId = 1;
    const pending = new Map();
    const events = [];
    sock.on('data', d => {
      buf = Buffer.concat([buf, d]);
      while (buf.length >= 2) {
        let len = buf[1] & 0x7f;
        let off = 2;
        if (len === 126) { len = buf.readUInt16BE(2); off = 4; }
        else if (len === 127) { len = Number(buf.readBigUInt64BE(2)); off = 10; }
        if (buf.length < off + len) break;
        const text = buf.slice(off, off + len).toString('utf8');
        buf = buf.slice(off + len);
        try {
          const msg = JSON.parse(text);
          if (msg.id && pending.has(msg.id)) {
            const cb = pending.get(msg.id); pending.delete(msg.id); cb(msg);
          } else if (msg.method) {
            events.push(msg);
          }
        } catch (e) {}
      }
    });
    function send(method, params = {}) {
      const id = nextId++;
      return new Promise((resolve, reject) => {
        pending.set(id, msg => msg.error ? reject(new Error(JSON.stringify(msg.error))) : resolve(msg.result));
        sock.write(encodeFrame(JSON.stringify({ id, method, params })));
      });
    }

    await send('Runtime.enable');
    await send('Page.enable');
    await send('Log.enable');
    await send('Page.navigate', { url: URL_ARG });
    // `--wait-ms` alone is a fixed SLEEP, and a sleep answers a question the caller did not
    // ask: it says "how long to wait" where the caller means "wait until the page has got
    // there".  On a loaded runner the difference is a gate that reports the CLAIM as false
    // when nothing was ever observed — `globalThis.loftFonts` undefined read as "the fonts
    // resolved", with a message that sent the reader after a font bug.
    //
    // `--ready EXPR` polls for the observation to EXIST, up to the same budget, and reports
    // its own failure kind when it never does.  That keeps the two shapes apart: `ready.timeout`
    // means the page did not get there, `assert` means it did and the claim is false.
    let readyReport = null;
    if (opts.readyExpr) {
      const deadline = Date.now() + opts.waitMs;
      let last = null;
      for (;;) {
        const ev = await send('Runtime.evaluate', {
          expression: `(() => { try { return JSON.stringify(!!(${opts.readyExpr})); }
                                catch (e) { return 'ERR ' + e.message; } })()`,
          returnByValue: true,
        });
        last = ev.result.value;
        if (last === 'true') break;
        if (Date.now() >= deadline) {
          readyReport = { expr: opts.readyExpr, value: last, waited: opts.waitMs };
          break;
        }
        await new Promise(r => setTimeout(r, 50));
      }
    } else {
      await new Promise(r => setTimeout(r, opts.waitMs));
    }

    // Optional screenshot
    if (opts.screenshot) {
      try {
        const shot = await send('Page.captureScreenshot', { format: 'png' });
        fs.writeFileSync(opts.screenshot, Buffer.from(shot.data, 'base64'));
      } catch (e) {
        console.error('screenshot failed: ' + e.message);
      }
    }

    // Layer 2: canvas content check.  Capture a screenshot clipped to
    // the canvas element's bounding rect, decode it, count distinct
    // RGB triples, fail if below the threshold.  A canvas that was
    // cleared to a single color but never drawn into has 1-2 distinct
    // colors; a working game frame has hundreds-thousands.
    let canvasReport = null;
    if (opts.canvasSelector) {
      try {
        // dpr is needed to convert CSS px (returned by getBoundingClientRect)
        // to device px (what Page.captureScreenshot operates in).
        const rectEval = await send('Runtime.evaluate', {
          expression: `
            (function(){
              const el = document.querySelector(${JSON.stringify(opts.canvasSelector)});
              if (!el) return null;
              const r = el.getBoundingClientRect();
              if (r.width < 1 || r.height < 1) return null;
              return { x: r.x, y: r.y, width: r.width, height: r.height,
                       dpr: window.devicePixelRatio || 1 };
            })()
          `,
          returnByValue: true,
        });
        const rect = rectEval.result.value;
        if (!rect) {
          canvasReport = { error: 'canvas selector matched no visible element: ' + opts.canvasSelector };
        } else {
          const clip = {
            x: rect.x, y: rect.y, width: rect.width, height: rect.height,
            scale: 1,
          };
          const shot = await send('Page.captureScreenshot', { format: 'png', clip });
          const pngBuf = Buffer.from(shot.data, 'base64');
          const info = decodePngRgb(pngBuf);
          canvasReport = { ...info, threshold: opts.canvasMinColors };
        }
      } catch (e) {
        canvasReport = { error: 'canvas content check failed: ' + e.message };
      }
    }

    // Layer 3: the caller's own assertion about the loaded page.  Evaluated
    // after the wait, so it sees the state the program actually reached.
    let assertReport = null;
    if (opts.assertExpr) {
      try {
        const ev = await send('Runtime.evaluate', {
          expression: `(() => { try { return JSON.stringify(${opts.assertExpr}); }
                                catch (e) { return 'ERR ' + e.message; } })()`,
          returnByValue: true,
        });
        assertReport = { expr: opts.assertExpr, value: ev.result.value };
        if (assertReport.value !== 'true') {
          // A false assertion names the expression, not what the page showed; the page's own
          // words (a status line, a compile error, a program's refusal) are what say why.
          const page = await send('Runtime.evaluate', {
            expression: `(() => { const t = (document.body && document.body.innerText) || '';
                                  return t.length > 2000 ? '…' + t.slice(-2000) : t; })()`,
            returnByValue: true,
          });
          assertReport.page = page.result.value;
        }
      } catch (e) {
        assertReport = { expr: opts.assertExpr, error: e.message };
      }
    }

    // Collect failures
    const failures = [];
    if (readyReport) {
      failures.push({
        kind: 'ready.timeout',
        text: `--ready ${readyReport.expr} never became true within ${readyReport.waited}ms `
          + `(last: ${readyReport.value}) — the page did not reach the state the assert is `
          + `about, so nothing was observed`,
      });
    }
    if (assertReport) {
      if (assertReport.error || assertReport.value !== 'true') {
        failures.push({
          kind: 'assert',
          text: `--assert ${assertReport.expr} => ${assertReport.error || assertReport.value}`,
          page: assertReport.page,
        });
      }
    }
    if (canvasReport && canvasReport.error) {
      failures.push({ kind: 'canvas.error', text: canvasReport.error });
    } else if (canvasReport && canvasReport.distinctColors < opts.canvasMinColors) {
      failures.push({
        kind: 'canvas.blank',
        text: `canvas has only ${canvasReport.distinctColors} distinct colors (threshold ${opts.canvasMinColors})`,
      });
    }
    for (const e of events) {
      if (e.method === 'Runtime.consoleAPICalled' && e.params.type === 'error') {
        const text = e.params.args.map(a => a.value ?? a.description ?? '').join(' ');
        if (!opts.allow.some(s => text.includes(s))) {
          failures.push({ kind: 'console.error', text });
        }
      } else if (e.method === 'Runtime.exceptionThrown') {
        const text = e.params.exceptionDetails.exception?.description ||
                     e.params.exceptionDetails.text || 'unknown exception';
        if (!opts.allow.some(s => text.includes(s))) {
          failures.push({ kind: 'exception', text });
        }
      } else if (e.method === 'Log.entryAdded' && e.params.entry.level === 'error') {
        const text = e.params.entry.text;
        // Browser-side resource 404s (favicon, etc.) are noise unless
        // they hit a file we intentionally load.  Filter the generic
        // "Failed to load resource" pattern unless --allow excludes it.
        if (/Failed to load resource/.test(text) && !opts.allow.some(s => text.includes(s))) {
          continue;
        }
        if (!opts.allow.some(s => text.includes(s))) {
          failures.push({ kind: 'log.error', text });
        }
      }
    }

    if (failures.length > 0) {
      console.error(JSON.stringify({ ok: false, url: URL_ARG, failures, canvas: canvasReport }, null, 2));
      exitCode = 1;
    } else {
      console.log(JSON.stringify({ ok: true, url: URL_ARG, eventCount: events.length, canvas: canvasReport }));
    }
  } catch (e) {
    console.error('harness error: ' + e.message);
    if (chromeErr) console.error('chrome stderr (last 500):\n' + chromeErr.slice(-500));
    exitCode = 3;
  } finally {
    chrome._intentionalExit = true;
    chrome.kill();
  }
  process.exit(exitCode);
})();
