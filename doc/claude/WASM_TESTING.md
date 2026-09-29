# WASM testing — the Node.js harness and the test compatibility matrix

Split out of [WASM.md](WASM.md): the Node.js test harness and which tests run under WASM.

## Node.js Test Harness

### Architecture

```
tests/wasm/
├── harness.mjs            — test runner, VirtFS, host setup
├── virt-fs.mjs            — VirtFS class implementation
├── virt-fs.test.mjs       — unit tests for VirtFS itself
├── host.mjs               — loftHost factory for Node.js
├── bridge.test.mjs        — WASM bridge integration tests
├── file-io.test.mjs       — loft file I/O through the virtual FS
├── random.test.mjs        — rand / rand_seed determinism
└── fixtures/
    ├── hello.json          — minimal VirtFS tree
    ├── multi-file.json     — multi-file project tree
    └── binary-data.json    — tree with binary file entries
```

### Running

```sh
# Build WASM for Node.js target
wasm-pack build --target nodejs --out-dir tests/wasm/pkg -- --features wasm

# Run all WASM tests
node --experimental-vm-modules tests/wasm/harness.mjs

# Run a single test file
node --experimental-vm-modules tests/wasm/virt-fs.test.mjs
```

### Host factory (`host.mjs`)

Creates a `loftHost` object wired to a `VirtFS` instance:

```js
import { VirtFS } from './virt-fs.mjs';

export function createHost(tree, options = {}) {
  const fs = new VirtFS(tree);

  // deterministic PRNG (xoshiro128**)
  let rng_state = [1, 2, 3, 4];
  function nextRandom() { /* xoshiro128** body */ }

  const storage = new Map();

  const host = {
    // filesystem — delegates to VirtFS
    fs_exists:       (p) => fs.exists(p),
    fs_read_text:    (p) => fs.readText(p),
    fs_read_binary:  (p, o, n) => fs.readBinary(p)?.slice(o, o + n) ?? null,
    fs_write_text:   (p, c) => { try { fs.writeText(p, c); return 0; } catch { return 5; } },
    fs_write_binary: (p, b) => { try { fs.writeBinary(p, b); return 0; } catch { return 5; } },
    fs_file_size:    (p) => fs.stat(p)?.size ?? -1,
    fs_delete:       (p) => { try { fs.delete(p); return 0; } catch { return 1; } },
    fs_move:         (f, t) => { try { fs.move(f, t); return 0; } catch { return 5; } },
    fs_mkdir:        (p) => { try { fs.mkdir(p); return 0; } catch { return 5; } },
    fs_mkdir_all:    (p) => { try { fs.mkdirAll(p); return 0; } catch { return 5; } },
    fs_list_dir:     (p) => fs.readdir(p) ?? [],
    fs_is_dir:       (p) => fs.isDirectory(p),
    fs_is_file:      (p) => fs.isFile(p),
    fs_seek:         (p, pos) => fs.seek(p, pos),
    fs_read_bytes:   (p, n) => fs.readBytes(p, n),
    fs_write_bytes:  (p, b) => { try { fs.writeBytes(p, b); return 0; } catch { return 5; } },
    fs_get_cursor:   (p) => fs.getCursor(p),
    fs_cwd:          () => fs.cwd,
    fs_user_dir:     () => '/home/test',
    fs_program_dir:  () => '/usr/local/bin',

    // random — deterministic by default
    random_int: (lo, hi) => lo + (nextRandom() % (hi - lo + 1)),
    random_seed: (hi, lo) => { rng_state = [lo, hi, lo ^ hi, lo + hi]; },

    // time
    time_now:   () => options.fakeTime ?? Date.now(),
    time_ticks: () => options.fakeTicks ?? 0,

    // environment
    env_variable: (name) => options.env?.[name] ?? null,

    // arguments
    arguments: () => options.args ?? [],

    // logging — goes to console
    log_write: (level, msg) => {
      const fn_ = level === 'fatal' ? 'error' : level;
      console[fn_](`[loft] ${msg}`);
    },

    // storage
    storage_get:    (k) => storage.get(k) ?? null,
    storage_set:    (k, v) => storage.set(k, v),
    storage_remove: (k) => storage.delete(k),
  };

  return { host, fs, storage };
}
```

### VirtFS unit tests (`virt-fs.test.mjs`)

```js
import { test, assert } from './harness.mjs';
import { VirtFS } from './virt-fs.mjs';

test('empty filesystem has root', () => {
  const fs = new VirtFS();
  assert(fs.isDirectory('/'));
  assert.deepEqual(fs.readdir('/'), []);
});

test('writeText creates file and parent dirs', () => {
  const fs = new VirtFS();
  fs.writeText('/a/b/c.txt', 'hello');
  assert(fs.isDirectory('/a'));
  assert(fs.isDirectory('/a/b'));
  assert(fs.isFile('/a/b/c.txt'));
  assert(fs.readText('/a/b/c.txt') === 'hello');
});

test('readdir lists entries', () => {
  const fs = new VirtFS({
    "/": {
      "x.txt": { "$type": "text", "$content": "x" },
      "y.txt": { "$type": "text", "$content": "y" },
      "sub": {}
    }
  });
  const entries = fs.readdir('/').sort();
  assert.deepEqual(entries, ['sub', 'x.txt', 'y.txt']);
});

test('delete removes file', () => {
  const fs = new VirtFS();
  fs.writeText('/f.txt', 'data');
  fs.delete('/f.txt');
  assert(!fs.exists('/f.txt'));
});

test('move renames file', () => {
  const fs = new VirtFS();
  fs.writeText('/old.txt', 'content');
  fs.move('/old.txt', '/new.txt');
  assert(!fs.exists('/old.txt'));
  assert(fs.readText('/new.txt') === 'content');
});

test('mkdir fails without parent', () => {
  const fs = new VirtFS();
  assert.throws(() => fs.mkdir('/a/b/c'));
});

test('mkdirAll creates full path', () => {
  const fs = new VirtFS();
  fs.mkdirAll('/a/b/c');
  assert(fs.isDirectory('/a/b/c'));
});

test('stat returns size for text files', () => {
  const fs = new VirtFS();
  fs.writeText('/f.txt', 'hello');     // 5 bytes UTF-8
  assert(fs.stat('/f.txt').size === 5);
  assert(fs.stat('/f.txt').type === 'text');
});

test('binary roundtrip', () => {
  const fs = new VirtFS();
  const data = new Uint8Array([1, 2, 3, 255]);
  fs.writeBinary('/b.bin', data);
  const out = fs.readBinary('/b.bin');
  assert.deepEqual(out, data);
  assert(fs.stat('/b.bin').type === 'binary');
});

test('binary cursor seek and read', () => {
  const fs = new VirtFS();
  fs.writeBinary('/b.bin', new Uint8Array([10, 20, 30, 40, 50]));
  fs.seek('/b.bin', 2);
  const chunk = fs.readBytes('/b.bin', 2);
  assert.deepEqual(chunk, new Uint8Array([30, 40]));
  assert(fs.getCursor('/b.bin') === 4);
});

test('snapshot and restore isolate mutations', () => {
  const fs = new VirtFS();
  fs.writeText('/keep.txt', 'original');
  const snap = fs.snapshot();

  fs.writeText('/keep.txt', 'modified');
  fs.writeText('/extra.txt', 'leaked');

  fs.restore(snap);
  assert(fs.readText('/keep.txt') === 'original');
  assert(!fs.exists('/extra.txt'));
});

test('resolve normalises paths', () => {
  const fs = new VirtFS();
  assert(fs.resolve('/a/b/../c/./d') === '/a/c/d');
  assert(fs.resolve('/a//b') === '/a/b');
  assert(fs.resolve('/a/b/') === '/a/b');
});

test('toJSON and fromJSON roundtrip', () => {
  const fs = new VirtFS();
  fs.writeText('/a.txt', 'hello');
  fs.mkdirAll('/sub/dir');
  fs.writeBinary('/sub/b.bin', new Uint8Array([42]));

  const json = JSON.stringify(fs.toJSON());
  const fs2 = VirtFS.fromJSON(json);

  assert(fs2.readText('/a.txt') === 'hello');
  assert(fs2.isDirectory('/sub/dir'));
  assert.deepEqual(fs2.readBinary('/sub/b.bin'), new Uint8Array([42]));
});
```

### WASM bridge integration tests (`bridge.test.mjs`)

These load the actual WASM module with a VirtFS-backed host:

```js
import { test, assert } from './harness.mjs';
import { createHost } from './host.mjs';
import { initWasm, compileAndRun } from './pkg/loft_wasm.js';

const tree = {
  "/": {
    "project": {
      "main.loft": { "$type": "text", "$content": "" }  // overwritten per test
    }
  }
};

let host, fs;

function run(code) {
  ({ host, fs } = createHost(structuredClone(tree)));
  globalThis.loftHost = host;
  fs.writeText('/project/main.loft', code);
  return compileAndRun([{ name: 'main.loft', content: code }]);
}

test('file write and read back', () => {
  const r = run(`
    fn main() {
      f = file("/project/out.txt")
      f.write("hello world")
      g = file("/project/out.txt")
      println(g.content())
    }
  `);
  assert(r.success);
  assert(r.output.trim() === 'hello world');
  assert(fs.readText('/project/out.txt') === 'hello world');
});

test('exists and delete', () => {
  const r = run(`
    fn main() {
      f = file("/project/tmp.txt")
      f.write("x")
      println(exists("/project/tmp.txt"))
      delete("/project/tmp.txt")
      println(exists("/project/tmp.txt"))
    }
  `);
  assert(r.success);
  assert(r.output.trim() === 'true\nfalse');
});

test('directory listing', () => {
  ({ host, fs } = createHost(structuredClone(tree)));
  globalThis.loftHost = host;
  fs.writeText('/project/a.loft', 'fn a() {}');
  fs.writeText('/project/b.loft', 'fn b() {}');

  const r = compileAndRun([{
    name: 'main.loft',
    content: `
      fn main() {
        d = file("/project")
        for f in d.files() { println(f.path) }
      }
    `
  }]);
  assert(r.success);
  assert(r.output.includes('a.loft'));
  assert(r.output.includes('b.loft'));
});

test('rand with seed is deterministic', () => {
  const r1 = run(`
    fn main() {
      rand_seed(42)
      println(rand(1, 1000))
      println(rand(1, 1000))
    }
  `);
  const r2 = run(`
    fn main() {
      rand_seed(42)
      println(rand(1, 1000))
      println(rand(1, 1000))
    }
  `);
  assert(r1.success && r2.success);
  assert(r1.output === r2.output);
});

test('mkdir_all and nested write', () => {
  const r = run(`
    fn main() {
      mkdir_all("/project/a/b/c")
      f = file("/project/a/b/c/deep.txt")
      f.write("nested")
      println(file("/project/a/b/c/deep.txt").content())
    }
  `);
  assert(r.success);
  assert(r.output.trim() === 'nested');
});

test('binary write and read', () => {
  const r = run(`
    fn main() {
      f = file("/project/data.bin")
      f.little_endian()
      f += 42
      f += 256
      g = file("/project/data.bin")
      g.little_endian()
      a = g#read(4) as integer
      b = g#read(4) as integer
      println(a)
      println(b)
    }
  `);
  assert(r.success);
  assert(r.output.trim() === '42\n256');
});
```

### File I/O edge case tests (`file-io.test.mjs`)

```js
test('read nonexistent file returns null content', () => { /* ... */ });
test('write to nested path auto-creates dirs', () => { /* ... */ });
test('delete nonexistent returns NotFound code', () => { /* ... */ });
test('move across directories', () => { /* ... */ });
test('overwrite existing file', () => { /* ... */ });
test('f#size reflects write', () => { /* ... */ });
test('seek beyond end pads with zeros', () => { /* ... */ });
test('binary cursor resets on write', () => { /* ... */ });
```

---

## Test Compatibility Matrix

### Rust integration tests (`tests/*.rs`)

| Test file | Tier 1 (sequential) | Tier 2 (threaded) | Notes |
|---|---|---|---|
| `expressions.rs` | Yes | Yes | Pure computation, no OS deps |
| `enums.rs` | Yes | Yes | Pure computation |
| `strings.rs` | Yes | Yes | Pure computation |
| `objects.rs` | Yes | Yes | Pure computation |
| `vectors.rs` | Yes | Yes | Pure computation |
| `sizes.rs` | Yes | Yes | Pure computation |
| `data_structures.rs` | Yes | Yes | Pure computation |
| `parse_errors.rs` | Yes | Yes | Diagnostic checking, no runtime |
| `immutability.rs` | Yes | Yes | Diagnostic checking |
| `slot_assign.rs` | Yes | Yes | Compile-time analysis |
| `log_config.rs` | Yes | Yes | Unit tests for config parsing |
| `issues.rs` | Yes | Yes | Reproducers — most are pure computation |
| `expressions_auto_convert.rs` | Yes | Yes | Pure computation |
| `threading.rs` | **Skip** | **Skip** | Tests Rust `std::thread` APIs directly — not WASM-portable |
| `wrap.rs` | Partial | Partial | Runs `.loft` files — needs VirtFS for file-IO tests |

### Loft script tests (`tests/scripts/*.loft`)

| Test file | Tier 1 | Tier 2 | Bridge needed |
|---|---|---|---|
| `01-*` through `14-*` | Yes | Yes | Output capture only |
| `15-random.loft` | Yes | Yes | `random_int`, `random_seed` |
| `16-time.loft` | Yes | Yes | `time_now`, `time_ticks` |
| `19-files.loft` | Yes | Yes | Full VirtFS bridge |
| `22-threading.loft` | Yes (sequential) | Yes (parallel) | Sequential fallback or Web Workers |
| `42-file-result.loft` | Yes | Yes | VirtFS + `fs_delete`, `fs_move`, `fs_mkdir` |

### Loft doc tests (`tests/docs/*.loft`)

| Test file | Tier 1 | Tier 2 | Bridge needed |
|---|---|---|---|
| `13-file.loft` | Yes | Yes | Full VirtFS bridge |
| `21-random.loft` | Yes | Yes | `random_int`, `random_seed` |
| `22-time.loft` | Yes | Yes | `time_now`, `time_ticks` |
| All others | Yes | Yes | Output capture only |

### Summary

| Category | Total | Tier 1 | Tier 2 | Skip | Notes |
|---|---|---|---|---|---|
| Rust integration tests | ~15 | 14 | 14 | 1 | Only `threading.rs` skipped (both tiers) |
| Loft script tests | ~40+ | All | All | 0 | `22-threading` sequential in T1, parallel in T2 |
| Loft doc tests | ~25+ | All | All | 0 | |

The **only test file that must be skipped** is `tests/threading.rs`, which tests
Rust-level `std::thread` APIs directly. Every loft-level test — including those
using `par()` — runs under both tiers. Tier 2 gives real parallelism; Tier 1
gives identical results sequentially.
