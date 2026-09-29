# WASM filesystem — the JSON virtual filesystem and the layered base tree

Split out of [WASM.md](WASM.md): the JSON virtual filesystem (VirtFS) and the layered base tree + delta overlay.

## JSON Virtual Filesystem

### Design goals

- Support the full loft `File` API: `file()`, `content()`, `lines()`, `write()`,
  `files()`, `exists()`, `delete()`, `move()`, `mkdir()`, `mkdir_all()`, binary
  read/write, `seek`, `f#size`, `f#exists`, `f#next`.
- Represent the entire filesystem as a single JSON tree — serialisable, inspectable,
  diffable.
- Allow snapshot/restore for test isolation.
- Run identically in browser (IndexedDB-backed) and Node.js (in-memory).

### Tree structure

```jsonc
{
  "/": {                          // root directory
    "home": {                     // directory node: value is an object
      "user": {
        "project": {
          "main.loft": {          // text file node
            "$type": "text",
            "$content": "fn main() { println(\"hello\") }"
          },
          "data.bin": {           // binary file node
            "$type": "binary",
            "$content": "AQID/w=="  // base64-encoded bytes
          },
          "src": {                // subdirectory — nested object
            "lib.loft": {
              "$type": "text",
              "$content": "pub fn add(a: integer, b: integer) -> integer { a + b }"
            }
          }
        }
      }
    },
    "tmp": {}                     // empty directory
  }
}
```

**Conventions:**
- A key whose value is a plain `{}` or contains nested keys (without `$type`) is a
  **directory**.
- A key whose value is `{ "$type": "text", "$content": "..." }` is a **text file**.
- A key whose value is `{ "$type": "binary", "$content": "<base64>" }` is a **binary
  file**.  Content is base64-encoded.
- Special keys always start with `$` — no loft filename may start with `$`.

### VirtFS class

```js
export class VirtFS {
  // --- construction ---
  constructor(tree = { "/": {} })     // initialise from a JSON tree
  static fromJSON(json)               // parse a JSON string into a VirtFS
  toJSON()                            // serialise the current state

  // --- snapshot / restore (test isolation) ---
  snapshot()                          // returns a deep-cloned tree
  restore(snapshot)                   // replaces the tree with a prior snapshot

  // --- path resolution ---
  // All paths are absolute. Relative paths are resolved against cwd.
  resolve(path)                       // normalise: remove //, resolve . and ..
  private _navigate(path)             // returns { parent, name, node } or null

  // --- read operations ---
  exists(path) -> boolean
  isFile(path) -> boolean
  isDirectory(path) -> boolean
  stat(path) -> { type, size } | null
  readText(path) -> string | null
  readBinary(path) -> Uint8Array | null
  readdir(path) -> string[]           // entry names (not full paths)

  // --- write operations ---
  writeText(path, content)            // creates parent dirs as needed
  writeBinary(path, bytes)            // bytes: Uint8Array; stored as base64
  mkdir(path)                         // single level, error if parent missing
  mkdirAll(path)                      // recursive
  delete(path)                        // file only
  deleteDir(path)                     // directory (must be empty)
  move(from, to)                      // rename/relocate

  // --- binary cursor (per-file state for seek/read) ---
  // Maintains a Map<path, { cursor: number }> for binary file positions.
  seek(path, pos)
  getCursor(path) -> number
  readBytes(path, n) -> Uint8Array    // reads n bytes from cursor, advances
  writeBytes(path, bytes)             // writes at cursor, advances

  // --- working directory ---
  cwd                                 // current working directory (string)
  chdir(path)
}
```

### Path resolution rules

1. Paths use forward slashes. Backslashes are converted on input.
2. `resolve("/home/user/../user/./project")` → `"/home/user/project"`.
3. Trailing slashes are stripped (except for `"/"`).
4. `resolve("relative/path")` prepends `this.cwd`.

### Internal storage

The tree is a plain JS object. File content is stored as a JS string (text) or
base64 string (binary).  Binary cursors are kept in a separate `Map<string, number>`
keyed by absolute path — cursors reset when the file is written or deleted.

### Example: test setup

```js
const fs = new VirtFS({
  "/": {
    "project": {
      "main.loft": { "$type": "text", "$content": "fn main() { println(\"hi\") }" },
      "data": {
        "names.txt": { "$type": "text", "$content": "alice\nbob\ncharlie" }
      }
    }
  }
});

fs.cwd = "/project";

assert(fs.exists("/project/main.loft"));
assert(fs.isDirectory("/project/data"));
assert.deepEqual(fs.readdir("/project/data"), ["names.txt"]);
assert(fs.readText("/project/data/names.txt") === "alice\nbob\ncharlie");
```

### Example: snapshot isolation in tests

```js
test('write does not leak between tests', () => {
  const snap = fs.snapshot();

  fs.writeText("/project/temp.loft", "fn temp() {}");
  assert(fs.exists("/project/temp.loft"));

  fs.restore(snap);
  assert(!fs.exists("/project/temp.loft"));
});
```

---

## Layered Filesystem — Base Tree + Delta Overlay

### Problem

The Web IDE ships with example programs and user documentation (from `tests/docs/`
and `doc/*.html`). These are **read-only defaults** that every user should see. When
a user edits an example or creates a new file, only the **changes** should be persisted
to localStorage/IndexedDB — not a full copy of the entire default tree.

### Design: two-layer VirtFS

```
┌─────────────────────────────────────────────┐
│              LayeredFS (read path)           │
│                                             │
│  read(path):                                │
│    if delta.exists(path)  → return delta    │
│    if delta.deleted(path) → return null     │
│    if base.exists(path)   → return base     │
│    → return null                            │
│                                             │
│  write(path, content):                      │
│    delta.write(path, content)               │
│    (base is never mutated)                  │
│                                             │
│  delete(path):                              │
│    delta.markDeleted(path)                  │
│    (base entry remains but is shadowed)     │
│                                             │
│  readdir(path):                             │
│    merge base entries + delta entries        │
│    minus delta-deleted entries              │
├─────────────────────────────────────────────┤
│  base (immutable)   │  delta (persisted)    │
│  ─────────────────  │  ──────────────────   │
│  Bundled at build   │  localStorage or      │
│  time from:         │  IndexedDB. Starts    │
│  • tests/docs/*.loft│  empty. Only grows    │
│  • doc/*.html       │  when the user edits  │
│  • default/*.loft   │  or creates files.    │
│  Shipped as a       │                       │
│  static JSON file   │  Stored as JSON:      │
│  in ide/assets/     │  { files: {...},      │
│    base-fs.json     │    deleted: [...] }   │
└─────────────────────┴───────────────────────┘
```

### Base tree — `base-fs.json`

Generated at build time by `ide/scripts/build-base-fs.js`:

```jsonc
{
  "/": {
    "examples": {
      "01-hello.loft":    { "$type": "text", "$content": "fn main() {\n  println(\"Hello, world!\")\n}" },
      "06-function.loft": { "$type": "text", "$content": "// Functions example\n..." },
      "07-vector.loft":   { "$type": "text", "$content": "// Vector example\n..." },
      "10-structs.loft":  { "$type": "text", "$content": "..." }
      // ... all tests/docs/*.loft files
    },
    "docs": {
      "language.html":    { "$type": "text", "$content": "<!DOCTYPE html>..." },
      "stdlib.html":      { "$type": "text", "$content": "..." }
      // ... generated doc/*.html files
    },
    "lib": {
      "01_code.loft":     { "$type": "text", "$content": "..." },
      "02_files.loft":   { "$type": "text", "$content": "..." },
      "03_text.loft":     { "$type": "text", "$content": "..." }
    }
  }
}
```

This file is loaded once on startup and never written to.

### Delta format

The delta is a small JSON object persisted to localStorage (or IndexedDB for large
projects):

```jsonc
{
  "files": {
    "/examples/06-function.loft": {        // modified base file
      "$type": "text",
      "$content": "// My edited version\nfn main() { ... }"
    },
    "/my-projects/game/main.loft": {       // new user file
      "$type": "text",
      "$content": "fn main() { ... }"
    }
  },
  "deleted": [
    "/examples/01-hello.loft"              // user deleted this example
  ],
  "dirs": [
    "/my-projects",
    "/my-projects/game"
  ]
}
```

**Storage key**: `loft-ide-delta` (single key for simple projects), or per-project
keys `loft-ide-delta:<project-id>` when multi-project support (M4) is active.

### LayeredFS class

```js
export class LayeredFS extends VirtFS {
  constructor(baseTree, delta = null) {
    super(baseTree);                    // base becomes the read-only layer
    this._base = baseTree;              // keep reference
    this._delta = delta ?? { files: {}, deleted: [], dirs: [] };
  }

  // --- read: delta wins, then base ---
  exists(path) {
    path = this.resolve(path);
    if (this._delta.deleted.includes(path)) return false;
    if (path in this._delta.files) return true;
    if (this._delta.dirs.includes(path)) return true;
    return super.exists(path);           // check base
  }

  readText(path) {
    path = this.resolve(path);
    if (this._delta.deleted.includes(path)) return null;
    const df = this._delta.files[path];
    if (df) return df.$content;
    return super.readText(path);         // fall through to base
  }

  readdir(path) {
    path = this.resolve(path);
    const baseEntries = new Set(super.readdir(path) ?? []);
    // add delta files in this directory
    for (const p of Object.keys(this._delta.files)) {
      const dir = p.substring(0, p.lastIndexOf('/')) || '/';
      if (dir === path) baseEntries.add(p.substring(p.lastIndexOf('/') + 1));
    }
    // add delta dirs
    for (const d of this._delta.dirs) {
      const parent = d.substring(0, d.lastIndexOf('/')) || '/';
      if (parent === path) baseEntries.add(d.substring(d.lastIndexOf('/') + 1));
    }
    // remove deleted entries
    for (const del of this._delta.deleted) {
      const dir = del.substring(0, del.lastIndexOf('/')) || '/';
      if (dir === path) baseEntries.delete(del.substring(del.lastIndexOf('/') + 1));
    }
    return [...baseEntries];
  }

  // --- write: always goes to delta ---
  writeText(path, content) {
    path = this.resolve(path);
    this._ensureParentDirs(path);
    this._delta.files[path] = { $type: 'text', $content: content };
    // remove from deleted if it was there
    this._delta.deleted = this._delta.deleted.filter(p => p !== path);
  }

  delete(path) {
    path = this.resolve(path);
    delete this._delta.files[path];
    if (!this._delta.deleted.includes(path)) {
      this._delta.deleted.push(path);
    }
  }

  // --- persistence ---
  getDelta()          { return this._delta; }
  setDelta(delta)     { this._delta = delta; }

  saveDelta(key = 'loft-ide-delta') {
    localStorage.setItem(key, JSON.stringify(this._delta));
  }

  static loadDelta(key = 'loft-ide-delta') {
    const raw = localStorage.getItem(key);
    return raw ? JSON.parse(raw) : null;
  }

  // --- reset: discard all user changes ---
  resetToBase() {
    this._delta = { files: {}, deleted: [], dirs: [] };
  }

  // --- check if a file has been modified from base ---
  isModified(path) {
    path = this.resolve(path);
    return path in this._delta.files;
  }

  isDeleted(path) {
    return this._delta.deleted.includes(this.resolve(path));
  }

  // --- list all user-modified paths ---
  modifiedPaths() {
    return Object.keys(this._delta.files);
  }
}
```

### Size budget

| Component | Estimated size |
|---|---|
| `base-fs.json` (gzipped) | ~50-100 KB (loft examples + docs) |
| Delta in localStorage | Typically < 50 KB (just edited files) |
| localStorage limit | 5-10 MB — plenty for delta |
| IndexedDB fallback | Needed only if binary data or > 100 files |

Since the base tree is static and cacheable, the service worker (M6) caches it
alongside the WASM module. The delta is tiny and persists in localStorage — no
IndexedDB needed for typical usage.

### Build script — `ide/scripts/build-base-fs.js`

```js
// Reads tests/docs/*.loft, doc/*.html, default/*.loft
// Outputs ide/assets/base-fs.json
import { readFileSync, readdirSync, writeFileSync } from 'fs';

const tree = { "/": { examples: {}, docs: {}, lib: {} } };

// examples from tests/docs/
for (const f of readdirSync('tests/docs').filter(f => f.endsWith('.loft'))) {
  tree["/"].examples[f] = {
    $type: 'text',
    $content: readFileSync(`tests/docs/${f}`, 'utf8')
  };
}

// generated HTML docs
for (const f of readdirSync('doc').filter(f => f.endsWith('.html'))) {
  tree["/"].docs[f] = {
    $type: 'text',
    $content: readFileSync(`doc/${f}`, 'utf8')
  };
}

// default stdlib
for (const f of readdirSync('default').filter(f => f.endsWith('.loft'))) {
  tree["/"].lib[f] = {
    $type: 'text',
    $content: readFileSync(`default/${f}`, 'utf8')
  };
}

writeFileSync('ide/assets/base-fs.json', JSON.stringify(tree));
```

### Browser startup flow

```js
// app.js — startup
const baseTree = await fetch('assets/base-fs.json').then(r => r.json());
const delta = LayeredFS.loadDelta();  // from localStorage, may be null
const fs = new LayeredFS(baseTree, delta);

// Wire the host
globalThis.loftHost = createBrowserHost(fs);

// Auto-save delta on every write (debounced)
let saveTimer;
const originalWrite = fs.writeText.bind(fs);
fs.writeText = (path, content) => {
  originalWrite(path, content);
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => fs.saveDelta(), 2000);
};
```

### "Reset to default" button

```js
resetButton.onclick = () => {
  if (confirm('Discard all changes and restore examples to original?')) {
    fs.resetToBase();
    fs.saveDelta();    // clears localStorage
    reloadEditor();
  }
};
```

### Node.js testing with layers

```js
import { LayeredFS } from './layered-fs.mjs';

test('user edit shadows base file', () => {
  const base = { "/": { "examples": {
    "hello.loft": { "$type": "text", "$content": "fn main() { println(\"hi\") }" }
  }}};
  const fs = new LayeredFS(base);

  // unmodified — reads from base
  assert(fs.readText('/examples/hello.loft').includes('hi'));
  assert(!fs.isModified('/examples/hello.loft'));

  // user edits — goes to delta
  fs.writeText('/examples/hello.loft', 'fn main() { println("bye") }');
  assert(fs.readText('/examples/hello.loft').includes('bye'));
  assert(fs.isModified('/examples/hello.loft'));

  // delta is small
  const delta = fs.getDelta();
  assert(Object.keys(delta.files).length === 1);

  // reset brings back original
  fs.resetToBase();
  assert(fs.readText('/examples/hello.loft').includes('hi'));
});

test('delete base file is tracked in delta', () => {
  const base = { "/": { "examples": {
    "a.loft": { "$type": "text", "$content": "fn a() {}" },
    "b.loft": { "$type": "text", "$content": "fn b() {}" }
  }}};
  const fs = new LayeredFS(base);

  fs.delete('/examples/a.loft');
  assert(!fs.exists('/examples/a.loft'));
  assert(fs.exists('/examples/b.loft'));    // unaffected
  assert.deepEqual(fs.readdir('/examples'), ['b.loft']);

  // delta only stores the deletion marker, not a copy of b.loft
  const delta = fs.getDelta();
  assert(delta.deleted.includes('/examples/a.loft'));
  assert(Object.keys(delta.files).length === 0);
});

test('new user file coexists with base', () => {
  const base = { "/": { "examples": {
    "hello.loft": { "$type": "text", "$content": "fn main() {}" }
  }}};
  const fs = new LayeredFS(base);

  fs.writeText('/my-project/main.loft', 'fn main() { println("mine") }');
  assert(fs.exists('/my-project/main.loft'));
  assert(fs.exists('/examples/hello.loft'));  // base still visible
  assert.deepEqual(fs.readdir('/').sort(), ['examples', 'my-project']);
});

test('delta serialise and reload', () => {
  const base = { "/": { "examples": {
    "a.loft": { "$type": "text", "$content": "original" }
  }}};
  const fs = new LayeredFS(base);
  fs.writeText('/examples/a.loft', 'modified');
  fs.writeText('/new.loft', 'brand new');

  // simulate save/reload cycle
  const deltaJson = JSON.stringify(fs.getDelta());
  const fs2 = new LayeredFS(base, JSON.parse(deltaJson));

  assert(fs2.readText('/examples/a.loft') === 'modified');
  assert(fs2.readText('/new.loft') === 'brand new');
});
```
