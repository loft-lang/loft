// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Browser runtime host for the loft WASM build.
//
// Combines the canonical VirtFS implementation
// (tests/wasm/virt-fs.mjs, ~366 lines) and the createHost factory
// (tests/wasm/host.mjs, ~130 lines) into a single self-contained
// module that the playground can `import`.  Two files are merged
// here because GitHub Pages doesn't follow symlinks reliably and
// the deployed `doc/` folder needs to be self-contained.
//
// Drift guard: tests/loft_rt_in_sync.rs (or `make doc-rt-check`)
// asserts this file matches the upstream sources concatenated.
// Any change to virt-fs.mjs or host.mjs must regenerate this file.


/**
 * W1.10 — In-memory virtual filesystem for WASM tests.
 *
 * The filesystem is represented as a JSON tree:
 *   - A key whose value is a plain `{}` or contains nested keys (without `$type`)
 *     is a **directory**.
 *   - `{ "$type": "text", "$content": "..." }` is a **text file**.
 *   - `{ "$type": "binary", "$content": "<base64>" }` is a **binary file**.
 *
 * Special keys always start with `$`.  No loft filename may start with `$`.
 */

'use strict';

// ── Base64 helpers ─────────────────────────────────────────────────────────────

/**
 * Encode a Uint8Array to a base64 string.
 * Works in both Node.js (Buffer.from) and browser (btoa + typed arrays).
 */
function _b64encode(bytes) {
  if (typeof Buffer !== 'undefined') {
    return Buffer.from(bytes).toString('base64');
  }
  let binary = '';
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary);
}

/**
 * Decode a base64 string to a Uint8Array.
 */
function _b64decode(str) {
  if (typeof Buffer !== 'undefined') {
    const buf = Buffer.from(str, 'base64');
    return new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength);
  }
  const binary = atob(str);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

// ── Deep clone ─────────────────────────────────────────────────────────────────

function _deepClone(obj) {
  return JSON.parse(JSON.stringify(obj));
}

// ── VirtFS ─────────────────────────────────────────────────────────────────────

export class VirtFS {
  /**
   * @param {object} tree  The initial JSON filesystem tree.  Defaults to an empty root.
   */
  constructor(tree = { '/': {} }) {
    this._tree = tree;
    // Binary cursors: Map<absolutePath, number>
    this._cursors = new Map();
    // Current working directory
    this._cwd = '/';
  }

  /** Parse a JSON string into a VirtFS. */
  static fromJSON(json) {
    return new VirtFS(JSON.parse(json));
  }

  /** Serialise the current state as a plain tree object (not a string). */
  toJSON() {
    return _deepClone(this._tree);
  }

  // ── Snapshot / restore ──────────────────────────────────────────────────────

  /** Return a deep clone of the tree for later restoration. */
  snapshot() {
    return _deepClone(this._tree);
  }

  /** Replace the current tree with a prior snapshot and clear all cursors. */
  restore(snapshot) {
    this._tree = _deepClone(snapshot);
    this._cursors.clear();
  }

  // ── Working directory ───────────────────────────────────────────────────────

  get cwd() { return this._cwd; }
  set cwd(path) { this._cwd = this.resolve(path); }

  chdir(path) { this._cwd = this.resolve(path); }

  // ── Path resolution ─────────────────────────────────────────────────────────

  /**
   * Normalise a path:
   *   - Backslashes → forward slashes
   *   - Relative → prepend cwd
   *   - Collapse `//`, resolve `.` and `..`
   *   - Strip trailing slash (except for '/')
   */
  resolve(path) {
    path = path.replace(/\\/g, '/');
    if (!path.startsWith('/')) {
      path = this._cwd.replace(/\/$/, '') + '/' + path;
    }
    const parts = path.split('/').filter(Boolean);
    const resolved = [];
    for (const p of parts) {
      if (p === '.') continue;
      if (p === '..') { resolved.pop(); }
      else { resolved.push(p); }
    }
    return '/' + resolved.join('/');
  }

  /**
   * Walk the tree to `path` and return `{ parent, name, node }`, or `null` if not found.
   * `parent` is the directory object containing `name`.
   * For the root `/`, returns `{ parent: null, name: null, node: tree['/'] }`.
   */
  _navigate(path) {
    path = this.resolve(path);
    const root = this._tree['/'];
    if (root === undefined) return null;
    if (path === '/') return { parent: null, name: null, node: root };

    const parts = path.slice(1).split('/');
    let current = root;
    for (let i = 0; i < parts.length - 1; i++) {
      const part = parts[i];
      if (!Object.hasOwn(current, part) || current[part]?.$type !== undefined) return null;
      current = current[part];
    }
    const last = parts[parts.length - 1];
    if (!Object.hasOwn(current, last)) {
      return { parent: current, name: last, node: undefined };
    }
    return { parent: current, name: last, node: current[last] };
  }

  // ── Read operations ─────────────────────────────────────────────────────────

  exists(path) {
    const nav = this._navigate(path);
    return nav !== null && nav.node !== undefined;
  }

  isFile(path) {
    const nav = this._navigate(path);
    if (!nav || nav.node === undefined) return false;
    return nav.node?.$type === 'text' || nav.node?.$type === 'binary';
  }

  isDirectory(path) {
    const nav = this._navigate(path);
    if (!nav || nav.node === undefined) return false;
    return nav.node?.$type === undefined;
  }

  /**
   * @returns {{ type: string, size: number } | null}
   */
  stat(path) {
    const nav = this._navigate(path);
    if (!nav || nav.node === undefined) return null;
    const node = nav.node;
    if (node.$type === 'text') {
      // Size in UTF-8 bytes
      const size = new TextEncoder().encode(node.$content).length;
      return { type: 'text', size };
    }
    if (node.$type === 'binary') {
      const bytes = _b64decode(node.$content);
      return { type: 'binary', size: bytes.length };
    }
    // directory
    return { type: 'directory', size: 0 };
  }

  /** @returns {string | null} */
  readText(path) {
    const nav = this._navigate(path);
    if (!nav || nav.node === undefined) return null;
    if (nav.node.$type !== 'text') return null;
    return nav.node.$content;
  }

  /** @returns {Uint8Array | null} */
  readBinary(path) {
    const nav = this._navigate(path);
    if (!nav || nav.node === undefined) return null;
    if (nav.node.$type !== 'binary') return null;
    return _b64decode(nav.node.$content);
  }

  /** @returns {string[]} Entry names (not full paths), or [] for missing/non-directory. */
  readdir(path) {
    const nav = this._navigate(path);
    if (!nav || nav.node === undefined) return [];
    if (nav.node.$type !== undefined) return [];  // not a directory
    return Object.keys(nav.node).filter(k => !k.startsWith('$'));
  }

  // ── Write operations ────────────────────────────────────────────────────────

  /**
   * Write a text file, creating all parent directories as needed.
   */
  writeText(path, content) {
    path = this.resolve(path);
    this._mkdirAllInternal(path.substring(0, path.lastIndexOf('/')) || '/');
    const nav = this._navigate(path);
    if (!nav) throw new Error(`Cannot create file at ${path}: path error`);
    nav.parent[nav.name] = { $type: 'text', $content: content };
    this._cursors.delete(path);
  }

  /**
   * Write a binary file (Uint8Array), creating all parent directories as needed.
   */
  writeBinary(path, bytes) {
    path = this.resolve(path);
    this._mkdirAllInternal(path.substring(0, path.lastIndexOf('/')) || '/');
    const nav = this._navigate(path);
    if (!nav) throw new Error(`Cannot create binary file at ${path}: path error`);
    nav.parent[nav.name] = { $type: 'binary', $content: _b64encode(bytes) };
    this._cursors.delete(path);
  }

  /**
   * Create a single directory level.  Throws if the parent does not exist.
   */
  mkdir(path) {
    path = this.resolve(path);
    if (path === '/') return;  // root always exists
    const parentPath = path.substring(0, path.lastIndexOf('/')) || '/';
    const parentNav = this._navigate(parentPath);
    if (!parentNav || parentNav.node === undefined || parentNav.node.$type !== undefined) {
      throw new Error(`mkdir: parent directory does not exist: ${parentPath}`);
    }
    const name = path.substring(path.lastIndexOf('/') + 1);
    if (!Object.hasOwn(parentNav.node, name)) {
      parentNav.node[name] = {};
    }
  }

  /**
   * Create a directory and all its ancestors.
   */
  mkdirAll(path) {
    this._mkdirAllInternal(this.resolve(path));
  }

  _mkdirAllInternal(path) {
    if (path === '/') return;
    const parts = path.slice(1).split('/').filter(Boolean);
    let current = this._tree['/'];
    for (const part of parts) {
      if (!Object.hasOwn(current, part)) {
        current[part] = {};
      } else if (current[part]?.$type !== undefined) {
        throw new Error(`mkdirAll: path component is a file: ${part}`);
      }
      current = current[part];
    }
  }

  /**
   * Delete a file.  Throws if path does not exist or is a directory.
   */
  delete(path) {
    path = this.resolve(path);
    const nav = this._navigate(path);
    if (!nav || nav.node === undefined) throw new Error(`delete: not found: ${path}`);
    if (nav.node.$type === undefined) throw new Error(`delete: is a directory: ${path}`);
    delete nav.parent[nav.name];
    this._cursors.delete(path);
  }

  /**
   * Delete an empty directory.  Throws if not empty or not a directory.
   */
  deleteDir(path) {
    path = this.resolve(path);
    if (path === '/') throw new Error('deleteDir: cannot delete root');
    const nav = this._navigate(path);
    if (!nav || nav.node === undefined) throw new Error(`deleteDir: not found: ${path}`);
    if (nav.node.$type !== undefined) throw new Error(`deleteDir: not a directory: ${path}`);
    const entries = Object.keys(nav.node).filter(k => !k.startsWith('$'));
    if (entries.length > 0) throw new Error(`deleteDir: directory not empty: ${path}`);
    delete nav.parent[nav.name];
  }

  /**
   * Move / rename a file or directory.
   */
  move(from, to) {
    from = this.resolve(from);
    to = this.resolve(to);
    const srcNav = this._navigate(from);
    if (!srcNav || srcNav.node === undefined) throw new Error(`move: source not found: ${from}`);

    const dstParent = to.substring(0, to.lastIndexOf('/')) || '/';
    this._mkdirAllInternal(dstParent);
    const dstNav = this._navigate(to);
    if (!dstNav) throw new Error(`move: destination path error: ${to}`);

    dstNav.parent[dstNav.name] = srcNav.node;
    delete srcNav.parent[srcNav.name];

    // Update cursors: move any cursor keyed by `from` to `to`
    if (this._cursors.has(from)) {
      this._cursors.set(to, this._cursors.get(from));
      this._cursors.delete(from);
    }
  }

  // ── Binary cursor ───────────────────────────────────────────────────────────

  seek(path, pos) {
    path = this.resolve(path);
    this._cursors.set(path, pos);
  }

  getCursor(path) {
    path = this.resolve(path);
    return this._cursors.get(path) ?? 0;
  }

  /** Read `n` bytes from the cursor position; advance the cursor. */
  readBytes(path, n) {
    path = this.resolve(path);
    const all = this.readBinary(path);
    if (!all) return null;
    const cursor = this._cursors.get(path) ?? 0;
    const slice = all.slice(cursor, cursor + n);
    this._cursors.set(path, cursor + slice.length);
    return slice;
  }

  /** Write bytes at the cursor position, extending/overwriting as needed; advance the cursor. */
  writeBytes(path, bytes) {
    path = this.resolve(path);
    const existing = this.readBinary(path) ?? new Uint8Array(0);
    const cursor = this._cursors.get(path) ?? 0;
    const end = cursor + bytes.length;
    const newLen = Math.max(existing.length, end);
    const updated = new Uint8Array(newLen);
    updated.set(existing);
    updated.set(bytes, cursor);
    // Store back (bypass writeText parent-dir logic — file already exists or we just read it)
    const nav = this._navigate(path);
    if (!nav || nav.node === undefined) {
      // File doesn't exist yet — create it
      this.writeBinary(path, updated);
    } else {
      nav.parent[nav.name] = { $type: 'binary', $content: _b64encode(updated) };
    }
    this._cursors.set(path, end);
  }
}

/**
 * W1.11 — loftHost factory for Node.js WASM tests.
 *
 * Creates a `loftHost` object wired to a `VirtFS` instance, ready to be
 * assigned to `globalThis.loftHost` before calling `compileAndRun()`.
 *
 * Usage:
 *   const { host, fs, storage } = createHost(tree, options);
 *   globalThis.loftHost = host;
 *
 * Options:
 *   fakeTime   — override `time_now()` return value (ms since epoch)
 *   fakeTicks  — override `time_ticks()` return value (µs)
 *   env        — plain object used for `env_variable(name)` lookups
 *   args       — string[] returned by `arguments()`
 */

// ── xoshiro128** PRNG ──────────────────────────────────────────────────────────
// Reference: https://prng.di.unimi.it/xoshiro128starstar.c
// All arithmetic is unsigned 32-bit.

function _rotl(x, k) {
  return ((x << k) | (x >>> (32 - k))) >>> 0;
}

function _makeRng(seed = [1, 2, 3, 4]) {
  // State: four uint32 values (stored as regular JS numbers, masked to 32-bit).
  let [s0, s1, s2, s3] = seed.map(v => v >>> 0);

  return {
    next() {
      const result = Math.imul(_rotl(Math.imul(s1, 5) >>> 0, 7), 9) >>> 0;
      const t = (s1 << 9) >>> 0;
      s2 = (s2 ^ s0) >>> 0;
      s3 = (s3 ^ s1) >>> 0;
      s1 = (s1 ^ s2) >>> 0;
      s0 = (s0 ^ s3) >>> 0;
      s2 = (s2 ^ t) >>> 0;
      s3 = _rotl(s3, 11);
      return result;
    },
    seed(hi, lo) {
      s0 = lo >>> 0;
      s1 = hi >>> 0;
      s2 = (lo ^ hi) >>> 0;
      s3 = (lo + hi) >>> 0;
    },
  };
}

// ── createHost ─────────────────────────────────────────────────────────────────

/**
 * Create a loftHost + VirtFS pair for use in Node.js WASM tests.
 *
 * @param {object} tree      Initial VirtFS tree (default: empty root).
 * @param {object} options   Optional overrides (fakeTime, fakeTicks, env, args).
 * @returns {{ host: object, fs: VirtFS, storage: Map }}
 */
export function createHost(tree = { '/': {} }, options = {}) {
  const fs = new VirtFS(tree);
  const rng = _makeRng();
  const storage = new Map();

  const host = {
    // ── filesystem — delegates to VirtFS ──────────────────────────────────────
    fs_exists:       (p) => fs.exists(p),
    fs_read_text:    (p) => fs.readText(p),
    fs_read_binary:  (p, o, n) => {
      const all = fs.readBinary(p);
      return all ? all.slice(o, o + n) : null;
    },
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
    fs_seek:         (p, pos) => { fs.seek(p, pos); },
    fs_read_bytes:   (p, n) => fs.readBytes(p, n),
    fs_write_bytes:  (p, b) => { try { fs.writeBytes(p, b); return 0; } catch { return 5; } },
    fs_get_cursor:   (p) => fs.getCursor(p),
    fs_cwd:          () => fs.cwd,
    fs_user_dir:     () => '/home/test',
    fs_program_dir:  () => '/usr/local/bin',

    // ── random — deterministic xoshiro128** ───────────────────────────────────
    // `random_int(lo, hi)` returns an integer in the closed interval [lo, hi].
    random_int:  (lo, hi) => {
      const range = (hi - lo + 1) >>> 0;
      return lo + (rng.next() % range);
    },
    // The Rust side passes a 64-bit seed split as (seed_hi: i32, seed_lo: i32).
    random_seed: (hi, lo) => rng.seed(hi, lo),

    // ── time ──────────────────────────────────────────────────────────────────
    time_now:   () => options.fakeTime  ?? Date.now(),
    // Real µs by default (the browser kernel's tick grid); `fakeTicks`
    // still overrides for deterministic harnesses.
    time_ticks: () => options.fakeTicks ?? Math.round(performance.now() * 1000),

    // ── environment ───────────────────────────────────────────────────────────
    env_variable: (name) => options.env?.[name] ?? null,

    // ── arguments ─────────────────────────────────────────────────────────────
    arguments: () => options.args ?? [],

    // ── logging — delegates to console ────────────────────────────────────────
    log_write: (level, msg) => {
      const fn_ = level === 'fatal' ? 'error' : level;
      (console[fn_] ?? console.log)(`[loft] ${msg}`);
    },

    // ── storage — in-memory Map ───────────────────────────────────────────────
    storage_get:    (k) => storage.get(k) ?? null,
    storage_set:    (k, v) => { storage.set(k, v); },
    storage_remove: (k) => { storage.delete(k); },
  };

  // ── TTT v3.5 — WebSocket bridge ─────────────────────────────────────────
  // Mirrors `lib/web/native/src/ws_client.rs` — the loft side calls
  // `web::ws_handler(url)` etc., the interpreter routes through
  // `host_ws_*`, and these handlers wrap browser's `WebSocket` (or
  // a `WebSocket`-shaped fallback under Node — see options.WebSocket).
  //
  // State per slot: `{ socket, inbox, lastMessage, lastOpcode, ready }`.
  // `inbox` queues raw frames; `lastMessage` / `lastOpcode` reflect the
  // most-recently-popped frame (read by host_ws_last_message /
  // host_ws_last_opcode after a positive host_ws_recv).
  const WS = options.WebSocket
    || (typeof WebSocket !== 'undefined' ? WebSocket : null);
  const wsConns = [];
  // @PLN18 08-S6 — the living-page swap hooks.  The PAGE is the persistent
  // host layer: it owns the sockets across loft instances.
  //  * onBuildPush: frames starting "B!:" are build pushes — consumed at
  //    the host (the script never sees them) and handed to the page.
  //  * ws_adopt_next(id): the NEXT ws_connect call ADOPTS the existing open
  //    socket `id` instead of dialing — the new build resumes on the SAME
  //    WebSocket object (no reconnect, the S6 identity clause).
  //  * ws_opens counts real socket constructions (the identity assert).
  host.onBuildPush = null;
  host.ws_opens = 0;
  host.onSend = null;
  let wsAdoptNext = -1;
  host.ws_adopt_next = (id) => { wsAdoptNext = id; };
  const wsOpen = (slot) => {
    slot.socket.binaryType = 'arraybuffer';
    slot.socket.addEventListener('open',  () => { slot.ready = true; });
    slot.socket.addEventListener('close', () => { slot.ready = false; });
    slot.socket.addEventListener('error', () => { slot.ready = false; });
    slot.socket.addEventListener('message', (ev) => {
      if (typeof ev.data === 'string' && ev.data.startsWith('B!:')) {
        // A build push — host-level, invisible to the running script.
        if (host.onBuildPush) host.onBuildPush(ev.data);
        return;
      }
      if (typeof ev.data === 'string') {
        slot.inbox.push({ data: ev.data, opcode: 1 });
      } else {
        // ArrayBuffer or Blob — for Node `ws` it's Buffer (Uint8Array-like).
        const u8 = ev.data instanceof ArrayBuffer
          ? new Uint8Array(ev.data)
          : new Uint8Array(ev.data);
        // Stash binary as a JS string of bytes (latin-1) so the loft side
        // can recover them via `byte_at(i, t)` regardless of UTF-8 validity.
        let s = '';
        for (let i = 0; i < u8.length; i++) s += String.fromCharCode(u8[i]);
        slot.inbox.push({ data: s, opcode: 2 });
      }
    });
  };
  host.ws_connect = (url) => {
    if (wsAdoptNext >= 0 && wsConns[wsAdoptNext]) {
      const id = wsAdoptNext;
      wsAdoptNext = -1;
      return id; // the swap target adopts the living socket
    }
    if (!WS) return -1;
    let socket;
    try { socket = new WS(url); } catch { return -1; }
    host.ws_opens++;
    const id = wsConns.length;
    const slot = {
      socket, inbox: [], lastMessage: '', lastOpcode: 0, ready: false,
    };
    wsConns.push(slot);
    wsOpen(slot);
    return id;
  };
  // Is the socket open?  The browser kernel buffers sends until this is 1
  // (a native connect blocks until upgraded; the same contract, async).
  host.ws_ready = (id) => {
    // (Was `sockets[id]` — an undefined table; ws_ready THREW on every
    // call.  Latent until the swap path needed it.)
    const slot = wsConns[id];
    return slot && slot.socket.readyState === 1 ? 1 : 0;
  };
  // Is the socket closing or closed?  Asked of a socket that never became ready, it
  // is how the browser kernel tells "still connecting" from "nothing answered".
  host.ws_closed = (id) => {
    const slot = wsConns[id];
    return slot && slot.socket.readyState >= 2 ? 1 : 0;
  };
  // The serving origin's host — engine_host::default_host() on a phone is
  // the cabinet that served the page.
  host.origin_host = () => (globalThis.location && location.hostname) || "127.0.0.1";
  host.ws_send = (id, msg, binary) => {
    const slot = wsConns[id];
    if (!slot || !slot.ready) return 0;
    if (host.onSend && !binary) host.onSend(msg); // page-side observability
    try {
      if (binary) {
        // Loft passed a JS string whose codepoints are the byte values
        // (latin-1).  Convert back to bytes for binary send.
        const u8 = new Uint8Array(msg.length);
        for (let i = 0; i < msg.length; i++) u8[i] = msg.charCodeAt(i) & 0xff;
        slot.socket.send(u8);
      } else {
        slot.socket.send(msg);
      }
      return 1;
    } catch { return 0; }
  };
  host.ws_recv = (id) => {
    const slot = wsConns[id];
    if (!slot) return 0;
    if (slot.inbox.length === 0) return 0;
    const next = slot.inbox.shift();
    slot.lastMessage = next.data;
    slot.lastOpcode  = next.opcode;
    return 1;
  };
  host.ws_last_message = () => {
    // Without a slot id, return the most recent across any slot.  The
    // loft side calls ws_recv(id) immediately before ws_last_message()
    // so the per-slot lastMessage/lastOpcode is current; we just return
    // whichever slot last popped.
    let s = '', best = -1;
    for (const slot of wsConns) {
      if (slot.lastOpcode && slot.lastMessage !== '') {
        s = slot.lastMessage; best = slot.lastOpcode;
      }
    }
    void best;
    return s;
  };
  host.ws_last_opcode = () => {
    let op = 0;
    for (const slot of wsConns) if (slot.lastOpcode) op = slot.lastOpcode;
    return op;
  };
  host.ws_close = (id) => {
    const slot = wsConns[id];
    if (!slot) return;
    try { slot.socket.close(); } catch {}
  };

  return { host, fs, storage };
}
