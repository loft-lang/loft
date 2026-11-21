// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// loft#851 — unit tests for the page filesystem (`doc/loft-fs.js`).
//
// The end-to-end guards in `tests/html_wasm.rs` drive a real `--html` page and
// prove the loft-side contract. They cannot reach the two properties that make
// this a PAGE filesystem rather than a scratch buffer, because the node harness
// has no localStorage and no bundled base tree:
//
//   * a base tree is read-only, and a write to a base path shadows it
//   * the delta survives a reload, and `resetToBase()` throws it away
//
// Those are what the consumer asked for ("data/parts/ becomes the base tree,
// the edited world becomes the delta"), so they are tested here against the
// SAME module the page ships — not a restatement of it.
//
// Run: node tools/loft_fs_unit.mjs      (exit 0 = pass)

import process from 'node:process';

// A localStorage stand-in, installed before loft-fs.js is imported so its
// `typeof localStorage` guard sees one. Deliberately a plain object: the point
// is what the delta round-trips through it, not the browser's quota behaviour.
const store = new Map();
globalThis.localStorage = {
  getItem: (k) => (store.has(k) ? store.get(k) : null),
  setItem: (k, v) => store.set(k, String(v)),
  removeItem: (k) => store.delete(k),
};

const { LoftPageFS, loftFS } = await import('../doc/loft-fs.js');

let failures = 0;
function check(name, got, want) {
  const g = JSON.stringify(got);
  const w = JSON.stringify(want);
  if (g === w) {
    console.log(`  ok   ${name}`);
  } else {
    failures++;
    console.log(`  FAIL ${name}\n         got  ${g}\n         want ${w}`);
  }
}

const dec = new TextDecoder();
const text = (bytes) => (bytes === null ? null : dec.decode(bytes));

// ── The base tree is read-only, and the delta shadows it ──────────────────────

console.log('base tree + delta');
{
  const base = { '/data/parts/tree.txt': 'a pine', '/data/parts/rock.txt': 'granite' };
  const fs = new LoftPageFS(base, null, '/');

  check('base file reads through', text(fs.read('/data/parts/tree.txt')), 'a pine');
  check('base dirs exist without mkdir', fs.isDirectory('/data/parts'), true);
  check('base listing', fs.readdir('/data/parts').sort(), ['rock.txt', 'tree.txt']);

  fs.write('/data/parts/tree.txt', new TextEncoder().encode('an oak'));
  check('a write shadows the base', text(fs.read('/data/parts/tree.txt')), 'an oak');
  check('the base itself is untouched', text(fs._base.get('/data/parts/tree.txt')), 'a pine');

  fs.delete('/data/parts/rock.txt');
  check('deleting a base file hides it', fs.exists('/data/parts/rock.txt'), false);
  check('and drops it from the listing', fs.readdir('/data/parts'), ['tree.txt']);

  fs.resetToBase();
  check('resetToBase restores the edit', text(fs.read('/data/parts/tree.txt')), 'a pine');
  check('resetToBase restores the deletion', fs.exists('/data/parts/rock.txt'), true);
}

// ── The delta survives a reload ───────────────────────────────────────────────

console.log('persistence');
{
  const base = { '/base.txt': 'bundled' };
  globalThis.loftFSKey = 'loft-fs-unit';

  const first = new LoftPageFS(base, null, '/');
  first.write('/work/world.hxw', new Uint8Array([1, 2, 250]));
  first.mkdirAll('/work/saves');
  first.delete('/base.txt');
  // `persist()` only SCHEDULES (writes coalesce into one save once the run
  // finishes); `flush()` is the synchronous force, and what `pagehide` calls.
  first.flush();

  const raw = globalThis.localStorage.getItem('loft-fs-unit');
  check('the delta reached localStorage', typeof raw === 'string' && raw.length > 0, true);

  // A burst of writes must cost ONE save, not one per write — serialising the
  // whole delta each time is quadratic, and building a file by `f += chunk` is
  // what a page saving a world does.
  let saves = 0;
  const realSet = globalThis.localStorage.setItem;
  globalThis.localStorage.setItem = function (k, v) { saves++; return realSet.call(this, k, v); };
  const bursty = new LoftPageFS({}, null, '/');
  const chunk = new TextEncoder().encode('x'.repeat(64));
  for (let i = 0; i < 500; i++) { bursty.seek('/log.txt', i * 64); bursty.writeBytes('/log.txt', chunk); }
  check('500 writes schedule, none save yet', saves, 0);
  bursty.flush();
  check('and they collapse into one save', saves, 1);
  check('with every byte present', bursty.size('/log.txt'), 500 * 64);
  globalThis.localStorage.setItem = realSet;

  // What a reload does: same page, same base tree, delta read back off the key.
  const reloaded = new LoftPageFS(base, JSON.parse(raw), '/');
  check('a written file survives', [...reloaded.read('/work/world.hxw')], [1, 2, 250]);
  check('a created directory survives', reloaded.isDirectory('/work/saves'), true);
  check('a deletion survives', reloaded.exists('/base.txt'), false);
  check('the base tree is still there', reloaded.isDirectory('/work'), true);
}

// ── Two games on one origin keep two saves ────────────────────────────────────

console.log('one save per page');
{
  // With no `loftFSKey` the key is the page's path: one key for every page made the games
  // on an origin read and overwrite each other's saves.
  delete globalThis.loftFSKey;
  globalThis.location = { pathname: '/games/moros/' };
  // A save from before saves were kept per page: carried over, not lost.
  store.set('loft-fs-delta', JSON.stringify(new LoftPageFS({}, null, '/').getDelta()));
  const legacy = new LoftPageFS({}, null, '/');
  legacy.write('/save.txt', new TextEncoder().encode('old save'));
  store.set('loft-fs-delta', JSON.stringify(legacy.getDelta()));
  delete globalThis.__loftFS;
  const moros = loftFS();
  check('an older shared save is carried over', text(moros.read('/save.txt')), 'old save');
  check('under the page\'s own key', store.has('loft-fs-delta:/games/moros/'), true);
  moros.write('/save.txt', new TextEncoder().encode('moros'));
  moros.flush();
  globalThis.location = { pathname: '/games/dryopea/' };
  store.delete('loft-fs-delta');
  delete globalThis.__loftFS;
  const dryopea = loftFS();
  check('another page does not see it', dryopea.exists('/save.txt'), false);
  dryopea.write('/save.txt', new TextEncoder().encode('dryopea'));
  dryopea.flush();
  check('nor overwrite it', JSON.parse(store.get('loft-fs-delta:/games/moros/')) !== null, true);
  globalThis.location = { pathname: '/games/moros/' };
  delete globalThis.__loftFS;
  check('the first page reads its own back', text(loftFS().read('/save.txt')), 'moros');
  delete globalThis.location;
  delete globalThis.__loftFS;
}

// ── Paths, and the cursor the loft side leans on ──────────────────────────────

console.log('paths and cursor');
{
  const fs = new LoftPageFS({}, null, '/home');
  fs.write('notes.txt', new TextEncoder().encode('abcdef'));
  check('a relative path resolves against the cwd', fs.exists('/home/notes.txt'), true);
  check('`..` is folded, not passed through', fs.resolve('/a/b/../c/./d'), '/a/c/d');

  fs.seek('/home/notes.txt', 2);
  check('a sized read starts at the cursor', text(fs.readBytes('/home/notes.txt', 3)), 'cde');
  check('and advances it', fs.cursor('/home/notes.txt'), 5);
  check('a read past the end is short', text(fs.readBytes('/home/notes.txt', 99)), 'f');

  fs.seek('/home/notes.txt', 1);
  fs.writeBytes('/home/notes.txt', new TextEncoder().encode('ZZ'));
  check('a cursor write patches in place', text(fs.read('/home/notes.txt')), 'aZZdef');

  // Writing past the end zero-fills rather than dropping the gap, so a program
  // that seeks forward and writes gets a file of the length it asked for.
  fs.seek('/home/notes.txt', 8);
  fs.writeBytes('/home/notes.txt', new TextEncoder().encode('!'));
  check('a write past the end zero-fills', [...fs.read('/home/notes.txt')].length, 9);
  check('the gap really is zero', fs.read('/home/notes.txt')[7], 0);

  check('an absent file reads as null, not empty', fs.read('/home/none.txt'), null);
  fs.write('/home/none.txt', new Uint8Array(0));
  check('an empty file that exists is not null', [...fs.read('/home/none.txt')], []);
}

console.log(failures === 0 ? '\nloft-fs: all checks passed' : `\nloft-fs: ${failures} FAILED`);
process.exit(failures === 0 ? 0 : 1);
