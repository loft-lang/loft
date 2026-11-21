// loft-fs.js — the filesystem a `loft --html` page stores in (loft#851).
//
// `--html` used to bind no filesystem at all.  The file calls still compiled,
// each took an inert branch and answered "absent", and nothing warned — so a
// page that draws could not save, and a program had to discover that by
// grepping the emitted bundle.  This is the host half of the binding: it
// implements the `loft_io.loft_host_fs_*` imports declared in `src/lib.rs`.
//
// The shape is the one `tests/wasm/layered-fs.mjs` proved for the IDE, because
// it is the shape a page actually needs:
//
//   base tree   immutable, bundled with the page (`globalThis.loftBaseFS`)
//   delta       every write, persisted to localStorage
//
// Reads consult the delta first and fall back to the base; writes only ever
// land in the delta, so reloading the page keeps the user's work and
// `resetToBase()` throws it away.  A page that wants no persistence sets
// `globalThis.loftFSPersist = false` and gets the same filesystem for the
// lifetime of the tab.
//
// Page-tunable globals, all optional:
//   loftBaseFS     {"/abs/path": string | Uint8Array}  the read-only base tree
//   loftFSKey      localStorage key for the delta   (default 'loft-fs-delta:' + the page's
//                  path, so two games served from one origin keep two saves)
//   loftFSPersist  false to keep the delta in memory only
//   loftFSCwd      what a relative path resolves against  (default '/')

'use strict';

// ── Base64, for binary content in a JSON delta ────────────────────────────────

function _b64enc(bytes) {
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) {
    s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  }
  return btoa(s);
}

function _b64dec(str) {
  const bin = atob(str);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

// ── LoftPageFS ────────────────────────────────────────────────────────────────

class LoftPageFS {
  constructor(base, delta, cwd) {
    // Every entry is normalised to bytes on the way in, so a read never has to
    // ask which of two representations a path happens to hold.
    this._base = new Map();
    for (const [p, v] of Object.entries(base || {})) {
      this._base.set(this.resolve(p), typeof v === 'string' ? new TextEncoder().encode(v) : v);
    }
    this._cwd = cwd || '/';
    this._files = new Map();     // path -> Uint8Array   (the delta's writes)
    this._dirs = new Set(['/']); // paths created with mkdir
    this._deleted = new Set();   // base paths the delta has removed
    this._cursors = new Map();   // path -> byte position
    if (delta) this.setDelta(delta);
    // Every directory implied by a base path exists without being created.
    for (const p of this._base.keys()) this._addParents(p);
  }

  // ── Paths ───────────────────────────────────────────────────────────────────

  /** Absolute, `.`/`..`-free, no trailing slash (except the root itself). */
  resolve(path) {
    let p = String(path == null ? '' : path);
    if (p[0] !== '/') p = `${this._cwd || '/'}/${p}`;
    const out = [];
    for (const part of p.split('/')) {
      if (part === '' || part === '.') continue;
      if (part === '..') { out.pop(); continue; }
      out.push(part);
    }
    return `/${out.join('/')}`;
  }

  _parent(path) {
    const i = path.lastIndexOf('/');
    return i <= 0 ? '/' : path.substring(0, i);
  }

  _addParents(path) {
    let d = this._parent(path);
    while (d !== '/' && !this._dirs.has(d)) { this._dirs.add(d); d = this._parent(d); }
  }

  // ── Reads: delta wins, base is the fallback ─────────────────────────────────

  read(path) {
    const p = this.resolve(path);
    if (this._files.has(p)) return this._files.get(p);
    if (this._deleted.has(p)) return null;
    return this._base.has(p) ? this._base.get(p) : null;
  }

  size(path) {
    const b = this.read(path);
    return b ? b.length : -1;
  }

  isFile(path) { return this.read(path) !== null; }

  isDirectory(path) {
    const p = this.resolve(path);
    if (this._deleted.has(p)) return false;
    return this._dirs.has(p);
  }

  exists(path) { return this.isFile(path) || this.isDirectory(path); }

  /** Immediate child names of `path` — names, not paths, matching `list_dir`. */
  readdir(path) {
    const dir = this.resolve(path);
    const prefix = dir === '/' ? '/' : `${dir}/`;
    const names = new Set();
    const consider = (p) => {
      if (!p.startsWith(prefix) || p === dir) return;
      const rest = p.substring(prefix.length);
      if (rest.indexOf('/') !== -1) return;   // a grandchild, not a child
      names.add(rest);
    };
    for (const p of this._base.keys()) if (!this._deleted.has(p) && !this._files.has(p)) consider(p);
    for (const p of this._files.keys()) consider(p);
    for (const p of this._dirs) consider(p);
    for (const p of this._deleted) {
      if (!this._files.has(p) && p.startsWith(prefix)) names.delete(p.substring(prefix.length));
    }
    return [...names];
  }

  // ── Writes: always into the delta ───────────────────────────────────────────

  write(path, bytes) {
    const p = this.resolve(path);
    if (this.isDirectory(p)) throw new Error(`is a directory: ${p}`);
    this._files.set(p, bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes));
    this._deleted.delete(p);
    this._addParents(p);
    this.persist();
  }

  mkdir(path) {
    const p = this.resolve(path);
    if (p === '/') return;
    if (!this.isDirectory(this._parent(p))) throw new Error(`no parent directory: ${p}`);
    if (this.isFile(p)) throw new Error(`exists as a file: ${p}`);
    this._dirs.add(p);
    this._deleted.delete(p);
    this.persist();
  }

  mkdirAll(path) {
    const p = this.resolve(path);
    const parts = p.split('/').filter(Boolean);
    let cur = '';
    for (const part of parts) {
      cur = `${cur}/${part}`;
      if (this.isFile(cur)) throw new Error(`path component is a file: ${cur}`);
      this._dirs.add(cur);
      this._deleted.delete(cur);
    }
    this.persist();
  }

  delete(path) {
    const p = this.resolve(path);
    if (this.isDirectory(p)) throw new Error(`is a directory: ${p}`);
    if (!this.isFile(p)) throw new Error(`not found: ${p}`);
    this._files.delete(p);
    this._deleted.add(p);
    this._cursors.delete(p);
    this.persist();
  }

  move(from, to) {
    const bytes = this.read(from);
    if (bytes === null) throw new Error(`not found: ${from}`);
    this.write(to, bytes);
    this.delete(from);
  }

  // ── The cursor.  A page has no OS file handle, so the host keeps the read
  //    and write position per path — `f#read(n)` and `f += x` seek here first.

  seek(path, pos) { this._cursors.set(this.resolve(path), Math.max(0, pos)); }

  cursor(path) { return this._cursors.get(this.resolve(path)) || 0; }

  /** Read up to `n` bytes at the cursor; a short read at EOF is not an error. */
  readBytes(path, n) {
    const bytes = this.read(path);
    if (bytes === null) return null;
    const at = this.cursor(path);
    const slice = bytes.subarray(Math.min(at, bytes.length), Math.min(at + n, bytes.length));
    this._cursors.set(this.resolve(path), at + slice.length);
    return slice;
  }

  /**
   * Write at the cursor, zero-filling any gap past the current end.
   *
   * This rebuilds the file's buffer, so building a file by many small appends
   * costs O(size²) in copying — about 1.1s for a 1.28 MB file written 64 bytes
   * at a time, against 45 ms for 256 KB.  One `write_bytes` of a finished blob,
   * or a few large `f += …`, does not pay it.  Worth a chunk list if a consumer
   * ever appends at that scale; measure before assuming they do.
   */
  writeBytes(path, data) {
    const at = this.cursor(path);
    const old = this.read(path) || new Uint8Array(0);
    const end = Math.max(old.length, at + data.length);
    const out = new Uint8Array(end);
    out.set(old, 0);
    out.set(data, at);
    this.write(path, out);
    this._cursors.set(this.resolve(path), at + data.length);
  }

  // ── Delta persistence ───────────────────────────────────────────────────────

  getDelta() {
    const files = {};
    for (const [p, b] of this._files) files[p] = _b64enc(b);
    return { files, dirs: [...this._dirs], deleted: [...this._deleted] };
  }

  setDelta(delta) {
    for (const [p, b64] of Object.entries(delta.files || {})) this._files.set(p, _b64dec(b64));
    for (const d of delta.dirs || []) this._dirs.add(d);
    for (const d of delta.deleted || []) this._deleted.add(d);
    for (const p of this._files.keys()) this._addParents(p);
  }

  /**
   * Mark the delta dirty and save it once the stack unwinds.
   *
   * Saving on every write is quadratic and not by a little: serialising the
   * WHOLE delta per write made 4000 appends to a 256 KB file cost 2.5 seconds,
   * against 11 ms for 200 — and building a file by `f += chunk` is exactly what
   * a page that saves a world does.  A loft run is synchronous, so a
   * zero-delay timer cannot fire until it finishes: a burst of writes
   * collapses into ONE save, and an idle page still saves promptly.
   */
  persist() {
    if (globalThis.loftFSPersist === false) return;
    if (typeof localStorage === 'undefined') return;
    this._dirty = true;
    if (this._scheduled) return;
    this._scheduled = true;
    // A page can be closed or hidden before the timer fires, so flush on the
    // way out too.  `pagehide` rather than `beforeunload`: it is the one a
    // mobile browser actually fires when the tab is backgrounded and then
    // discarded, which is when a page loses work.
    if (!this._unloadBound && typeof addEventListener === 'function') {
      this._unloadBound = true;
      addEventListener('pagehide', () => this.flush());
    }
    setTimeout(() => {
      this._scheduled = false;
      this.flush();
    }, 0);
  }

  /**
   * Write the delta out now.
   *
   * A quota failure is reported once and then tolerated: the page keeps working
   * against the in-memory delta, because losing the run is strictly worse than
   * losing the persistence, and a write that half-succeeded would be the one
   * outcome a consumer cannot reason about.
   */
  flush() {
    if (!this._dirty) return;
    if (globalThis.loftFSPersist === false) return;
    if (typeof localStorage === 'undefined') return;
    this._dirty = false;
    try {
      localStorage.setItem(_loftFSKey(), JSON.stringify(this.getDelta()));
    } catch (e) {
      if (!this._quotaWarned) {
        this._quotaWarned = true;
        console.warn('[loft:fs] cannot persist to localStorage — this session\'s writes stay in memory:', e);
      }
    }
  }

  /** Discard every write and go back to the bundled base tree. */
  resetToBase() {
    this._files.clear();
    this._deleted.clear();
    this._cursors.clear();
    this._dirs = new Set(['/']);
    for (const p of this._base.keys()) this._addParents(p);
    this.persist();
  }
}

/** The page's filesystem, created on first use from the page-tunable globals. */
// The key this page's delta is kept under: the page's own `loftFSKey`, else one per page path.
// One key for every page made all games on an origin read and overwrite each other's saves.
function _loftFSKey() {
  if (globalThis.loftFSKey) return globalThis.loftFSKey;
  const path = typeof location !== 'undefined' && location.pathname ? location.pathname : '/';
  return 'loft-fs-delta:' + path;
}

function loftFS() {
  if (!globalThis.__loftFS) {
    let delta = null;
    if (globalThis.loftFSPersist !== false && typeof localStorage !== 'undefined') {
      try {
        let raw = localStorage.getItem(_loftFSKey());
        // A save made before saves were kept per page is still this player's: carried over
        // once, under the page's own key, and never lost.
        if (!raw && !globalThis.loftFSKey) {
          raw = localStorage.getItem('loft-fs-delta');
          if (raw) localStorage.setItem(_loftFSKey(), raw);
        }
        if (raw) delta = JSON.parse(raw);
      } catch (e) { delta = null; }
    }
    globalThis.__loftFS = new LoftPageFS(globalThis.loftBaseFS, delta, globalThis.loftFSCwd);
  }
  return globalThis.__loftFS;
}

/**
 * The `loft_io.loft_host_fs_*` handlers, as a plain object to merge into a
 * page's imports.  `getMem()` answers the instance's current `WebAssembly.Memory`
 * (it is re-read on every call because growing the heap detaches the old buffer).
 *
 * Reads answer a LENGTH and stash the bytes for `loft_host_fs_copy` — a raw wasm
 * import cannot return a string.  `0xFFFFFFFF` (`usize::MAX`) means absent, which
 * is deliberately distinct from a length of 0: an empty file that exists.
 */
function loftFSImports(getMem) {
  const dec = typeof loftTextDecoder === 'function' ? loftTextDecoder() : new TextDecoder();
  const enc = new TextEncoder();
  const ABSENT = 0xFFFFFFFF;
  let stash = null;

  const str = (ptr, len) => dec.decode(new Uint8Array(getMem().buffer, ptr, len));
  const bytesAt = (ptr, len) => new Uint8Array(getMem().buffer, ptr, len).slice();
  const put = (bytes) => { stash = bytes; return bytes === null ? ABSENT : bytes.length; };
  const putText = (s) => put(s === null ? null : enc.encode(s));
  // A thrown host error must not become a wasm trap: loft's contract is that a
  // failed file operation answers a code, never a crash (C80 — no runtime
  // errors, ever).  9 is FS_OTHER.
  const code = (f) => { try { return f() === false ? 9 : 0; } catch (e) { return 9; } };

  return {
    loft_host_fs_read_text(p, pl) {
      const b = loftFS().read(str(p, pl));
      if (b === null) return put(null);
      // Bytes that are not text read as absent, which is what the native reader
      // answers for the same file — `content()` must not differ per target.
      try { return putText(new TextDecoder('utf-8', { fatal: true }).decode(b)); }
      catch (e) { return put(null); }
    },
    loft_host_fs_read_binary(p, pl) { return put(loftFS().read(str(p, pl))); },
    loft_host_fs_read_bytes(p, pl, want) { return put(loftFS().readBytes(str(p, pl), want)); },
    loft_host_fs_list_dir(p, pl) {
      const path = str(p, pl);
      if (!loftFS().isDirectory(path)) return put(null);
      return putText(loftFS().readdir(path).join('\n'));
    },
    loft_host_fs_cwd() { return putText(loftFS()._cwd); },
    loft_host_fs_user_dir() { return putText('/'); },
    loft_host_fs_program_dir() { return putText('/'); },
    loft_host_fs_copy(ptr) {
      if (stash && stash.length) new Uint8Array(getMem().buffer, ptr, stash.length).set(stash);
    },

    loft_host_fs_write_text(p, pl, d, dl) {
      return code(() => loftFS().write(str(p, pl), bytesAt(d, dl)));
    },
    loft_host_fs_write_binary(p, pl, d, dl) {
      return code(() => loftFS().write(str(p, pl), bytesAt(d, dl)));
    },
    loft_host_fs_write_bytes(p, pl, d, dl) {
      return code(() => loftFS().writeBytes(str(p, pl), bytesAt(d, dl)));
    },
    loft_host_fs_delete(p, pl) { return code(() => loftFS().delete(str(p, pl))); },
    loft_host_fs_move(f, fl, t, tl) { return code(() => loftFS().move(str(f, fl), str(t, tl))); },
    loft_host_fs_mkdir(p, pl) { return code(() => loftFS().mkdir(str(p, pl))); },
    loft_host_fs_mkdir_all(p, pl) { return code(() => loftFS().mkdirAll(str(p, pl))); },
    loft_host_fs_exists(p, pl) { return loftFS().exists(str(p, pl)) ? 1 : 0; },
    loft_host_fs_is_dir(p, pl) { return loftFS().isDirectory(str(p, pl)) ? 1 : 0; },
    loft_host_fs_is_file(p, pl) { return loftFS().isFile(str(p, pl)) ? 1 : 0; },
    loft_host_fs_file_size(p, pl) { return loftFS().size(str(p, pl)); },
    loft_host_fs_seek(p, pl, pos) { loftFS().seek(str(p, pl), pos); },
    loft_host_fs_get_cursor(p, pl) { return loftFS().cursor(str(p, pl)); },
  };
}

export { LoftPageFS, loftFS, loftFSImports };
