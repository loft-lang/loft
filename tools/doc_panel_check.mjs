#!/usr/bin/env node
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN149 step 8 — drive a documentation page's Run / REPL / Debug panel through the REAL
// wasm exports, in node, with no browser.
//
// `tests/wasm_entry.rs` already drives `debug_start` / `debug_command` natively, which
// checks the grammar and the values. It cannot check the one thing that only exists in a
// wasm build: `output`. Print is captured by `crate::wasm::output_push`, which is compiled
// in only under the `wasm` feature — so natively the field is present and always empty, and
// a regression that stopped the program's output reaching the page would pass every native
// test. This runs the same session against the wasm module and asserts the output arrives.
//
// Prerequisite (the same one tests/wasm/suite.mjs has):
//   wasm-pack build --target nodejs --out-dir tests/wasm/pkg -- --no-default-features --features wasm
//
// Exit code: 0 all checks passed · 1 a check failed · 2 the wasm package is not built
// (a SKIP — not having built it is not a wrong answer).

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const PAGE = join(ROOT, 'tests/docs/38-call-it-yourself.loft');

let wasm;
try {
  wasm = await import(join(ROOT, 'tests/wasm/pkg/loft.js'));
} catch {
  console.error('SKIP: tests/wasm/pkg not built — run `wasm-pack build --target nodejs '
    + '--out-dir tests/wasm/pkg -- --no-default-features --features wasm`');
  process.exit(2);
}
if (typeof wasm.debug_start !== 'function' || typeof wasm.debug_command !== 'function') {
  console.error('SKIP: tests/wasm/pkg predates debug_start/debug_command — rebuild it');
  process.exit(2);
}

let failures = 0;
function check(label, got, want) {
  const ok = typeof want === 'function' ? want(got) : got === want;
  if (!ok) {
    failures++;
    console.error(`FAIL ${label}\n  got: ${JSON.stringify(got)}`);
  } else {
    console.log(`ok   ${label}`);
  }
}

const cmd = (c) => JSON.parse(wasm.debug_command(c));
const source = readFileSync(PAGE, 'utf8');

// ── The session the panel opens on load ─────────────────────────────────────
check('the page compiles in wasm', wasm.debug_start(source), '{"ok":true}');

// ── What the panel offers to call ───────────────────────────────────────────
const fns = cmd('fns').replies[0];
check('fns names the page functions with signatures', fns,
  (r) => r.includes('fib(n: integer) -> integer')
      && r.includes('stats(a: integer, b: integer) -> Span'));
check('fns does not offer main', fns, (r) => !r.includes('main('));

// ── Run, ending paused so the prompt has a frame ────────────────────────────
check('bp end is accepted', cmd('bp end').replies[0], 'D:ok bp end');
const run = cmd('run');
check('the run pauses in main', run.replies[0], (r) => r.startsWith('D:hit main'));
check('main’s locals are live at the pause', run.replies[0], (r) => r.includes('s=Span'));

// THE reason this harness exists: natively `output` is always empty, so only here can the
// program's own print be observed reaching the page.
check('the program’s output rides back with the run', run.output,
  (o) => o.includes('fib(10)=55') && o.includes('nth_prime(10)=29'));

// ── The prompt ──────────────────────────────────────────────────────────────
// Every value hand-computed, not read off a previous run of this harness.
const evals = [
  ['fib(10)', '55'],
  ['fib(30)', '832040'],
  ['nth_prime(10)', '29'],
  ['is_prime(97)', 'true'],
  ['vowels("your name here")', '6'],
  ['stats(3, 17)', '{"lo":3,"hi":17,"span":14}'],
  ['s.span', '14'],
];
for (const [expr, want] of evals) {
  check(`eval ${expr}`, cmd(`eval ${expr}`).replies[0], `D:eval ${expr}=${want}`);
}

// A text result is boxed onto the one value path and answers its VALUE (loft#1187 — it
// used to answer `<unavailable>`), and the session goes on afterwards.
check('a text result answers its value',
  cmd('eval "a" + "b"').replies[0], 'D:eval "a" + "b"="ab"');
check('and the session survives it', cmd('eval fib(10)').replies[0], 'D:eval fib(10)=55');

// ── A line breakpoint, which is what a gutter click sends ───────────────────
check('a fresh session for the line breakpoint', wasm.debug_start(source), '{"ok":true}');
// `total = 0;` inside walk_to — the line after its `fn` header.
const walkLine = source.split('\n').findIndex((l) => l.includes('total = 0;')) + 1;
check('bp on a line is accepted', cmd(`bp ${walkLine}`).replies[0], `D:ok bp ${walkLine}`);
const hit = cmd('run');
check('the run pauses in walk_to', hit.replies[0], (r) => r.startsWith('D:hit walk_to'));
check('the caller’s argument is live there', hit.replies[0], (r) => r.includes('limit=4'));
check('a call using the paused frame’s local', cmd('eval fib(limit)').replies[0],
  'D:eval fib(limit)=3');
check('resume runs to the end', cmd('resume').replies[0], 'D:terminated');

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log('\nall checks passed');
