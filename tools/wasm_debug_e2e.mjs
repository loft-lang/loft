// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN98 P3.4 — the FULL browser debug relay end-to-end: an AGENT debugs a
// browser wasm CLIENT THROUGH a running game server, over the WebSockets both
// hold. Args: <client.wasm> <server-port>. The client half is the `--html
// --debug` wasm run in Node with a JS driver bridging the server WebSocket to its
// host_input (forwarded `D!:` frames) and host output (its `D:` replies, wrapped
// as `D!:reply`). The agent connects to the same server and drives bp/run/eval/
// resume addressed to the client by name. Prints `AGENT_GOT=<json>` of the `D:`
// frames the agent received (the client's relayed replies + the server's forward
// acks).
import fs from 'node:fs';
// loft#851 — the page's filesystem, imported rather than restubbed so every
// harness answers what a real page answers.  A stub returning 0 would mean "an
// empty file that EXISTS" where the contract says absent, and a missing import
// is a LinkError the moment a program under test touches a file.
import { loftFSImports } from '../doc/loft-fs.js';
const arg = process.argv[2];
let wasm;
if (arg.endsWith('.html')) {
  const html = fs.readFileSync(arg, 'utf8');
  const m = html.match(/const wasmB64="([^"]*)"/);
  wasm = Buffer.from(m[1], 'base64');
} else {
  wasm = fs.readFileSync(arg);
}
const URL = 'ws://127.0.0.1:' + process.argv[3] + '/ws';
const enc = new TextEncoder(), dec = new TextDecoder();
const sleep = ms => new Promise(r => setTimeout(r, ms));

let mem = null; const inQ = []; let registered = false;
const client = new WebSocket(URL);
await new Promise((res, rej) => { client.onopen = res; client.onerror = rej; });
client.binaryType = 'arraybuffer';
client.onmessage = (e) => {
  const msg = typeof e.data === 'string' ? e.data : dec.decode(new Uint8Array(e.data));
  if (msg === 'D:registered alice') registered = true;
  if (msg.startsWith('D!:')) inQ.push(enc.encode(msg));
};
const io = {
  ...loftFSImports(() => mem),
  loft_host_print: (p, l) => {
    for (const line of dec.decode(new Uint8Array(mem.buffer, p, l)).split('\n'))
      if (line.startsWith('D:')) client.send('D!:reply ' + line);
  },
  loft_host_input_len: () => (inQ.length ? inQ[0].length : 0),
  loft_host_input_copy: (p) => { const b = inQ.shift(); if (b) new Uint8Array(mem.buffer, p, b.length).set(b); },
  loft_host_output: () => {},
};
// loft_io gets a per-function callable fallback too, so a newly-added import
// (e.g. loft_host_http_get) never LinkErrors a stub harness that never calls it.
const stubs = new Proxy({ loft_io: new Proxy(io, { get: (t, k) => (k in t ? t[k] : () => 0) }) }, { get: (t, k) => (k in t ? t[k] : new Proxy({}, { get: () => () => 0 })) });
const inst = new WebAssembly.Instance(new WebAssembly.Module(wasm), stubs);
mem = inst.exports.memory;
const dbg = inst.exports.loft_debug_start();
client.send('D!:iam alice');
const pump = setInterval(() => inst.exports.loft_debug_pump(dbg), 25);

const got = [];
const agent = new WebSocket(URL);
await new Promise((res, rej) => { agent.onopen = res; agent.onerror = rej; });
agent.binaryType = 'arraybuffer';
agent.onmessage = (e) => { got.push(typeof e.data === 'string' ? e.data : dec.decode(new Uint8Array(e.data))); };
// Event-driven, not a fixed sleep: under the full suite's parallel load the 4MB wasm
// instantiate + stdlib parse + `iam` round-trip can exceed any fixed delay, and the
// agent would then address an unregistered name ("D:err no debug client alice").
// Wait for the server's registration ack (the pump keeps running between awaits).
for (let t = 0; !registered && t < 15000; t += 20) await sleep(20);
for (const c of ['bp compute', 'run', 'eval n', 'eval n + 2', 'resume']) { agent.send('D!:@alice:' + c); await sleep(300); }
// Drain: keep pumping until the replies stop arriving, so a slow last reply under
// load isn't cut off by clearInterval (the fixed post-loop tail used to race).
for (let stable = 0, last = -1, t = 0; stable < 300 && t < 5000; t += 50) {
  if (got.length !== last) { last = got.length; stable = 0; } else stable += 50;
  await sleep(50);
}
clearInterval(pump);
console.log('AGENT_GOT=' + JSON.stringify(got.filter(m => m.startsWith('D:'))));
process.exit(0);
