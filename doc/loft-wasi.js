// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// The `wasi_snapshot_preview1` host imports for a page whose module the C backend wrote
// (`--native-c --html`, @PLN203 C6): its runtime is the WASI libc, so a page answers what that
// libc asks — standard output and error into the page, the clocks, randomness, an empty
// environment, one argument, and files: the page filesystem (`loft-fs.js`, the `[[embed]]`
// files and what the program writes), preopened as `/`, against which `loft-fs.js` resolves a
// program's relative path too.  A page has no sockets; anything else answers "not supported".
// The `--html` driver adds this file when the module imports WASI.
//
// Every import re-reads `getMem().buffer`: wasm memory can grow and detach.

(globalThis.LOFT_WASM_EXTENSIONS = globalThis.LOFT_WASM_EXTENSIONS || []).push(
  function loftWasiHostImports(imports, ctrl, getMem) {
    const view = () => new DataView(getMem().buffer);
    const bytes = (ptr, len) => new Uint8Array(getMem().buffer, ptr, len);
    const decs = { 1: new TextDecoder(), 2: new TextDecoder() };
    const enc = new TextEncoder();
    const dec = new TextDecoder();
    const args = ['program'];
    // Into the page's output, or — under node, which test harnesses run a module in — the
    // process's own streams, byte for byte.
    const write = (fd, text) => {
      if (!text) return;
      if (typeof document === 'undefined') {
        if (typeof process !== 'undefined') (fd === 2 ? process.stderr : process.stdout).write(text);
        return;
      }
      const out = document.getElementById('out');
      if (out) {
        out.style.display = '';
        out.textContent += text;
      }
      (fd === 2 ? console.error : console.log)(text.replace(/\n$/, ''));
    };
    // WASI errnos.
    const BADF = 8, EXIST = 20, INVAL = 28, ISDIR = 31, NOENT = 44, NOSYS = 52, NOTDIR = 54,
      NOTEMPTY = 55, SPIPE = 70, NOTCAPABLE = 76;
    // WASI file types.
    const DIR = 3, FILE = 4, CHAR = 2;

    // The page filesystem, when the page carries one; without it files are refused.
    const pageFS = typeof loftFS === 'function' ? loftFS() : null;
    const ROOT = 3; // the one preopened directory, `/`
    const fds = new Map(); // fd -> { path, dir, pos, write }
    let nextFd = ROOT + 1;
    const pathAt = (ptr, len) => dec.decode(bytes(ptr, len));
    const under = (dirfd, ptr, len) => {
      const base = dirfd === ROOT ? '/' : fds.get(dirfd)?.path;
      if (base === undefined) return null;
      return pageFS.resolve(`${base}/${pathAt(ptr, len)}`);
    };
    const statInto = (buf, isDir, size) => {
      const v = view();
      for (let i = 0; i < 64; i += 8) v.setBigUint64(buf + i, 0n, true);
      v.setUint8(buf + 16, isDir ? DIR : FILE);
      v.setBigUint64(buf + 24, 1n, true);
      v.setBigUint64(buf + 32, BigInt(size), true);
      return 0;
    };
    const persist = () => {
      try { pageFS.persist(); } catch (_e) { /* a page without storage keeps its delta in memory */ }
    };

    const wasi = {
      fd_write(fd, iovs, n, written) {
        const v = view();
        let total = 0;
        const f = fds.get(fd);
        for (let i = 0; i < n; i++) {
          const p = v.getUint32(iovs + 8 * i, true);
          const l = v.getUint32(iovs + 8 * i + 4, true);
          if (fd === 1 || fd === 2) write(fd, decs[fd].decode(bytes(p, l), { stream: true }));
          else if (f && !f.dir && f.write) {
            const old = pageFS.read(f.path) || new Uint8Array(0);
            const out = new Uint8Array(Math.max(old.length, f.pos + l));
            out.set(old);
            out.set(bytes(p, l), f.pos);
            pageFS.write(f.path, out);
            f.pos += l;
          } else return BADF;
          total += l;
        }
        v.setUint32(written, total, true);
        return 0;
      },
      fd_read(fd, iovs, n, read) {
        const v = view();
        const f = fds.get(fd);
        if (fd === 0) { v.setUint32(read, 0, true); return 0; }
        if (!f || f.dir) return f ? ISDIR : BADF;
        const data = pageFS.read(f.path) || new Uint8Array(0);
        let total = 0;
        for (let i = 0; i < n; i++) {
          const p = v.getUint32(iovs + 8 * i, true);
          const l = v.getUint32(iovs + 8 * i + 4, true);
          const take = Math.max(0, Math.min(l, data.length - f.pos));
          bytes(p, take).set(data.subarray(f.pos, f.pos + take));
          f.pos += take;
          total += take;
          if (take < l) break;
        }
        v.setUint32(read, total, true);
        return 0;
      },
      fd_seek(fd, offset, whence, newOffset) {
        const f = fds.get(fd);
        if (!f) return fd <= 2 ? SPIPE : BADF;
        const size = (pageFS.read(f.path) || new Uint8Array(0)).length;
        const base = whence === 0 ? 0 : whence === 1 ? f.pos : size;
        const at = base + Number(offset);
        if (at < 0) return INVAL;
        f.pos = at;
        view().setBigUint64(newOffset, BigInt(at), true);
        return 0;
      },
      fd_tell(fd, out) {
        const f = fds.get(fd);
        if (!f) return BADF;
        view().setBigUint64(out, BigInt(f.pos), true);
        return 0;
      },
      fd_close(fd) {
        const f = fds.get(fd);
        if (f && f.write) persist();
        fds.delete(fd);
        return 0;
      },
      fd_filestat_set_size(fd, size) {
        const f = fds.get(fd);
        if (!f || f.dir) return f ? ISDIR : BADF;
        const old = pageFS.read(f.path) || new Uint8Array(0);
        const out = new Uint8Array(Number(size));
        out.set(old.subarray(0, Math.min(old.length, out.length)));
        pageFS.write(f.path, out);
        persist();
        return 0;
      },
      fd_sync() { persist(); return 0; },
      fd_datasync() { persist(); return 0; },
      fd_fdstat_get(fd, st) {
        const v = view();
        let type;
        if (fd <= 2) type = CHAR;
        else if (fd === ROOT && pageFS) type = DIR;
        else if (fds.has(fd)) type = fds.get(fd).dir ? DIR : FILE;
        else return BADF;
        v.setUint8(st, type);
        v.setUint16(st + 2, 0, true);
        v.setBigUint64(st + 8, 0xffffffffffffffffn, true);
        v.setBigUint64(st + 16, 0xffffffffffffffffn, true);
        return 0;
      },
      fd_fdstat_set_flags() { return 0; },
      fd_prestat_get(fd, buf) {
        if (fd !== ROOT || !pageFS) return BADF;
        const v = view();
        v.setUint8(buf, 0);
        v.setUint32(buf + 4, 1, true);
        return 0;
      },
      fd_prestat_dir_name(fd, ptr, len) {
        if (fd !== ROOT || !pageFS) return BADF;
        if (len >= 1) bytes(ptr, 1)[0] = 0x2f; // `/`
        return 0;
      },
      fd_filestat_get(fd, buf) {
        if (fd <= 2) return statInto(buf, false, 0);
        const path = fd === ROOT ? '/' : fds.get(fd)?.path;
        if (path === undefined || !pageFS) return BADF;
        const isDir = pageFS.isDirectory(path);
        return statInto(buf, isDir, isDir ? 0 : pageFS.size(path));
      },
      path_filestat_get(dirfd, flags, ptr, len, buf) {
        if (!pageFS) return NOTCAPABLE;
        const path = under(dirfd, ptr, len);
        if (path === null) return BADF;
        if (!pageFS.exists(path)) return NOENT;
        const isDir = pageFS.isDirectory(path);
        return statInto(buf, isDir, isDir ? 0 : pageFS.size(path));
      },
      path_open(dirfd, dirflags, ptr, len, oflags, rightsBase, rightsInherit, fdflags, out) {
        if (!pageFS) return NOTCAPABLE;
        const path = under(dirfd, ptr, len);
        if (path === null) return BADF;
        const CREAT = 1, DIRECTORY = 2, EXCL = 4, TRUNC = 8;
        const wants = BigInt(rightsBase);
        const writing = (wants & (1n << 6n)) !== 0n || (oflags & (CREAT | TRUNC)) !== 0;
        const exists = pageFS.exists(path);
        const isDir = exists && pageFS.isDirectory(path);
        if ((oflags & DIRECTORY) && exists && !isDir) return NOTDIR;
        if (exists && (oflags & EXCL) && (oflags & CREAT)) return EXIST;
        if (!exists) {
          if (!(oflags & CREAT)) return NOENT;
          pageFS.write(path, new Uint8Array(0));
        } else if (isDir && writing && !(oflags & DIRECTORY)) {
          return ISDIR;
        } else if (oflags & TRUNC) {
          pageFS.write(path, new Uint8Array(0));
        }
        const fd = nextFd++;
        const append = (fdflags & 1) !== 0;
        fds.set(fd, {
          path,
          dir: isDir,
          pos: append ? (pageFS.read(path) || new Uint8Array(0)).length : 0,
          write: writing,
        });
        view().setUint32(out, fd, true);
        return 0;
      },
      fd_readdir(fd, buf, bufLen, cookie, used) {
        const f = fd === ROOT ? { path: '/', dir: true } : fds.get(fd);
        if (!f || !pageFS) return BADF;
        if (!f.dir) return NOTDIR;
        const names = ['.', '..', ...pageFS.readdir(f.path).sort()];
        const v = view();
        let at = 0;
        for (let i = Number(cookie); i < names.length && at < bufLen; i++) {
          const name = enc.encode(names[i]);
          const child = pageFS.resolve(`${f.path}/${names[i]}`);
          const head = new DataView(new ArrayBuffer(24));
          head.setBigUint64(0, BigInt(i + 1), true);
          head.setBigUint64(8, BigInt(i + 1), true);
          head.setUint32(16, name.length, true);
          head.setUint8(20, pageFS.isDirectory(child) ? DIR : FILE);
          const entry = new Uint8Array(24 + name.length);
          entry.set(new Uint8Array(head.buffer));
          entry.set(name, 24);
          const take = Math.min(entry.length, bufLen - at);
          bytes(buf + at, take).set(entry.subarray(0, take));
          at += take;
        }
        v.setUint32(used, at, true);
        return 0;
      },
      path_create_directory(dirfd, ptr, len) {
        if (!pageFS) return NOTCAPABLE;
        const path = under(dirfd, ptr, len);
        if (path === null) return BADF;
        if (pageFS.exists(path)) return EXIST;
        pageFS.mkdir(path);
        persist();
        return 0;
      },
      path_unlink_file(dirfd, ptr, len) {
        if (!pageFS) return NOTCAPABLE;
        const path = under(dirfd, ptr, len);
        if (path === null) return BADF;
        if (!pageFS.isFile(path)) return pageFS.isDirectory(path) ? ISDIR : NOENT;
        pageFS.delete(path);
        persist();
        return 0;
      },
      path_remove_directory(dirfd, ptr, len) {
        if (!pageFS) return NOTCAPABLE;
        const path = under(dirfd, ptr, len);
        if (path === null) return BADF;
        if (!pageFS.isDirectory(path)) return pageFS.exists(path) ? NOTDIR : NOENT;
        if (pageFS.readdir(path).length) return NOTEMPTY;
        pageFS.delete(path);
        persist();
        return 0;
      },
      path_rename(fromFd, fromPtr, fromLen, toFd, toPtr, toLen) {
        if (!pageFS) return NOTCAPABLE;
        const from = under(fromFd, fromPtr, fromLen);
        const to = under(toFd, toPtr, toLen);
        if (from === null || to === null) return BADF;
        if (!pageFS.exists(from)) return NOENT;
        pageFS.move(from, to);
        persist();
        return 0;
      },
      proc_exit(code) {
        for (const fd of [1, 2]) write(fd, decs[fd].decode());
        if (typeof window !== 'undefined') window.loftExit = code;
        throw new Error('loft: the program exited with status ' + code);
      },
      args_sizes_get(argc, size) {
        const v = view();
        v.setUint32(argc, args.length, true);
        v.setUint32(size, args.reduce((s, a) => s + enc.encode(a).length + 1, 0), true);
        return 0;
      },
      args_get(argv, buf) {
        const v = view();
        const m = new Uint8Array(getMem().buffer);
        let at = buf;
        args.forEach((a, i) => {
          v.setUint32(argv + 4 * i, at, true);
          const b = enc.encode(a);
          m.set(b, at);
          m[at + b.length] = 0;
          at += b.length + 1;
        });
        return 0;
      },
      environ_sizes_get(c, s) {
        const v = view();
        v.setUint32(c, 0, true);
        v.setUint32(s, 0, true);
        return 0;
      },
      environ_get() { return 0; },
      clock_time_get(id, precision, t) {
        const ns = id === 0
          ? BigInt(Date.now()) * 1000000n
          : BigInt(Math.round(performance.now() * 1e6));
        view().setBigUint64(t, ns, true);
        return 0;
      },
      clock_res_get(id, r) {
        view().setBigUint64(r, 1000n, true);
        return 0;
      },
      random_get(p, l) {
        crypto.getRandomValues(bytes(p, l));
        return 0;
      },
      sched_yield() { return 0; },
      poll_oneoff() { return NOSYS; },
    };
    // Any other WASI function the libc links answers "not supported", so the module
    // instantiates whatever its libc imports.
    const own = imports.wasi_snapshot_preview1 || {};
    imports.wasi_snapshot_preview1 = new Proxy(own, {
      get: (t, k) => t[k] || wasi[k] || (() => NOSYS),
    });
  },
);
