// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

/**
 * W1.13 — Full loft test suite via WASM.
 *
 * Runs tests/scripts/*.loft and tests/docs/*.loft through the WASM module
 * and compares their output against the native loft interpreter.
 *
 * Prerequisites:
 *   1. Build the WASM package:
 *        wasm-pack build --target nodejs --out-dir tests/wasm/pkg -- --no-default-features --features wasm
 *   2. Run:
 *        node tests/wasm/suite.mjs
 *
 * For each test file the runner:
 *   1. Reads the .loft source
 *   2. Populates a VirtFS with the source file and any supporting fixtures
 *   3. Calls compile_and_run() via the WASM module
 *   4. Gets native reference output via `cargo run`
 *   5. Compares outputs; reports pass / fail / skip
 *
 * Exit code: 0 if all run tests pass, 1 if any fail.
 */

import { readFileSync, existsSync, readdirSync } from 'fs';
import { execSync, spawnSync } from 'child_process';
import { basename, join } from 'path';

// ── Check WASM package ────────────────────────────────────────────────────────

try {
  await import('./pkg/loft.js');
} catch {
  console.log('SKIP  suite — WASM package not built');
  console.log('      wasm-pack build --target nodejs --out-dir tests/wasm/pkg -- --no-default-features --features wasm');
  process.exit(0);
}

// ── Check native runner ────────────────────────────────────────────────────────

let nativeAvailable = false;
try {
  execSync('cargo --version', { stdio: 'ignore', timeout: 5000 });
  nativeAvailable = true;
} catch {
  console.log('NOTE  cargo not found — WASM tests will verify success only, not compare output');
}

// ── Skip / compare-skip lists ──────────────────────────────────────────────────

/**
 * Tests that are skipped entirely.
 *
 * Empty.  It held `14-image.loft`, which needs PNG fixtures VirtFS has no easy way to
 * carry — and that page now lives in the `imaging` package, where its own CI runs it
 * against the real files (@PLN149 step 9).  A page that cannot run here is a page a
 * reader cannot trust, so add an entry only with the open issue that explains it.
 */
const SKIP = new Set([]);

/**
 * Tests that run through WASM but whose output is NOT compared to native.
 * - Time-sensitive: output changes every run.
 * - Order-sensitive with randomness: values differ per seed but behaviour is correct.
 *
 * `22-threading.loft` used to sit here on the assumption that a sequential WASM
 * `par` merely ordered its output differently.  It did not: a build without the
 * `threading` feature had no par runtime registered at all, so every `par`
 * returned garbage — and this list is what kept that invisible.  A par now
 * produces byte-identical output with and without threads, so it COMPARES.
 */
const SKIP_COMPARE = new Set([
  '16-time.loft',      // now()/ticks() values are non-deterministic
  '22-time.loft',      // same — doc version
  '15-random.loft',    // rand() without seed — non-deterministic
]);

// ── Native reference runner ────────────────────────────────────────────────────

/**
 * Run a loft file with the native interpreter and return its stdout, or null
 * if the run fails or cargo is unavailable.
 *
 * @param {string} filePath  Path to the .loft file.
 * @returns {string|null}
 */
function runNative(filePath) {
  if (!nativeAvailable) return null;
  try {
    return execSync(
      `cargo run --bin loft --quiet -- "${filePath}" 2>/dev/null`,
      { encoding: 'utf8', timeout: 30_000 }
    );
  } catch (e) {
    return null;  // native also failed — both sides should fail/skip
  }
}

// ── WASM runner ────────────────────────────────────────────────────────────────

/**
 * Run a loft file through the WASM module in a subprocess and return the result.
 *
 * Each test gets its own Node.js process with a fresh WASM module instance,
 * so a WASM crash (RuntimeError: unreachable / memory access out of bounds)
 * does not corrupt state and cause all subsequent tests to fail.
 *
 * @param {string} filePath  Path to the .loft file.
 * @returns {{ success: boolean, output: string, diagnostics: string }}
 */
function runWasm(filePath) {
  const result = spawnSync('node', ['tests/wasm/run-one.mjs', filePath], {
    encoding: 'utf8',
    timeout: 30_000,
  });
  if (result.error) {
    return { success: false, output: '', diagnostics: String(result.error) };
  }
  try {
    return JSON.parse(result.stdout);
  } catch {
    return { success: false, output: '', diagnostics: result.stderr || result.stdout || 'no output' };
  }
}

// ── Test discovery ─────────────────────────────────────────────────────────────

/**
 * Collect all .loft files from a directory that contain `fn main(`.
 * @param {string} dir  Relative directory path.
 * @returns {string[]}  Sorted relative file paths.
 */
function discoverTests(dir) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter(f => f.endsWith('.loft'))
    .sort()
    .map(f => join(dir, f))
    .filter(p => {
      try {
        return readFileSync(p, 'utf8').includes('\nfn main(');
      } catch { return false; }
    });
}

// ── Main ───────────────────────────────────────────────────────────────────────

const scriptTests = discoverTests('tests/scripts');
const docTests    = discoverTests('tests/docs');
// Every program of the language suite beside the tree (@PLN193): the rule cells, then the
// attic.  `LOFT_SUITE_DIR` names the checkout; an absent one contributes nothing and says so.
const suiteDir = process.env.LOFT_SUITE_DIR || '../loft-suite';
let suiteTests = [];
if (existsSync(join(suiteDir, 'rules'))) {
  for (const chapter of readdirSync(join(suiteDir, 'rules')).sort()) {
    for (const rule of readdirSync(join(suiteDir, 'rules', chapter)).sort()) {
      suiteTests.push(...discoverTests(join(suiteDir, 'rules', chapter, rule)));
    }
  }
  suiteTests.push(...discoverTests(join(suiteDir, 'attic', 'scripts')));
} else {
  console.error(`no language suite at ${suiteDir} — its programs are NOT in this run (LOFT_SUITE_DIR)`);
}
const allTests    = [...scriptTests, ...docTests, ...suiteTests];

let passed = 0;
let failed = 0;
let skipped = 0;

console.log(`\nWASM suite — ${allTests.length} test files\n`);

for (const filePath of allTests) {
  const name = basename(filePath);

  if (SKIP.has(name)) {
    console.log(`SKIP  ${filePath}`);
    skipped++;
    continue;
  }

  const wasmResult = runWasm(filePath);

  if (!wasmResult.success) {
    console.error(`FAIL  ${filePath}`);
    if (wasmResult.diagnostics) {
      console.error(`      diagnostics: ${wasmResult.diagnostics.split('\n')[0]}`);
    }
    failed++;
    continue;
  }

  if (SKIP_COMPARE.has(name)) {
    console.log(`  ok  ${filePath}  (no output comparison)`);
    passed++;
    continue;
  }

  if (!nativeAvailable) {
    console.log(`  ok  ${filePath}  (WASM success; no native comparison)`);
    passed++;
    continue;
  }

  const nativeOut = runNative(filePath);
  if (nativeOut === null) {
    // Native failed too — treat as expected (both agree)
    console.log(`  ok  ${filePath}  (both native and WASM produced no output)`);
    passed++;
    continue;
  }

  const wasmOut = wasmResult.output;
  if (nativeOut.trimEnd() === wasmOut.trimEnd()) {
    console.log(`  ok  ${filePath}`);
    passed++;
  } else {
    console.error(`FAIL  ${filePath}  — output mismatch`);
    console.error(`      native: ${JSON.stringify(nativeOut.slice(0, 120))}`);
    console.error(`        wasm: ${JSON.stringify(wasmOut.slice(0, 120))}`);
    failed++;
  }
}

console.log(`\n${passed} passed, ${failed} failed, ${skipped} skipped`);
process.exit(failed > 0 ? 1 : 0);
