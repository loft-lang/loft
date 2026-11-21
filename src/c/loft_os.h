/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * The OS a unit of loft's C runtime asks its system headers for.  A feature request counts only
 * before the FIRST system header — glibc fixes what it declares when the first one includes
 * <features.h> — so this is the first text of every unit that carries the runtime, ahead of
 * `loft_width.h` and its <stdint.h>.  Asked later, `pread`, `pwrite`, `truncate`, `lstat` and
 * `readlink` are undeclared under `-std=c11`.  It is not given to a library's shim: a shim asks
 * its own system headers for what it uses. */
#ifndef LOFT_OS_H
#define LOFT_OS_H

/* The OS beyond ISO C — a monotonic clock, a path's kind, directories, truncate — is POSIX where
 * the platform is one (`LOFT_POSIX`); `-DLOFT_NO_POSIX` builds against ISO C alone, where the
 * clock is `clock()` and the directory operations answer Other.  WASI (wasm32-wasi) is POSIX for
 * files, directories and clocks, and has no sockets, `mmap` or `realpath`: those stand under
 * `LOFT_POSIX_HOST`, a POSIX host OS. */
#if !defined(LOFT_NO_POSIX) && (defined(__unix__) || defined(__unix) || defined(__APPLE__) || defined(__wasi__) || defined(ESP_PLATFORM))
#define LOFT_POSIX 1
/* ESP-IDF (newlib with its VFS) is POSIX for files, directories and clocks, as WASI is, and
 * has no `mmap`, processes or `realpath` either: it is no POSIX host.  Its headers declare
 * what it has without the request, which would hide newlib's extensions the drivers use. */
#if !defined(ESP_PLATFORM)
#define _POSIX_C_SOURCE 200809L
#endif
#if !defined(__wasi__) && !defined(ESP_PLATFORM)
#define LOFT_POSIX_HOST 1
#endif
/* `realpath` is XSI, not base POSIX: glibc declares it only under _XOPEN_SOURCE (or by accident
 * through _FORTIFY_SOURCE, which Ubuntu turns on from -O1 — so an -O0 build failed). */
#if defined(LOFT_POSIX_HOST) && !defined(__APPLE__)
#define _XOPEN_SOURCE 700
#endif
/* Darwin's headers, asked for base POSIX, hide everything outside it — `INADDR_LOOPBACK` and
 * `MSG_DONTWAIT` the sockets use among them; `_DARWIN_C_SOURCE` is Apple's request for the
 * rest of its API beside POSIX. */
#if defined(__APPLE__)
#define _DARWIN_C_SOURCE 1
#endif
#endif

#endif
