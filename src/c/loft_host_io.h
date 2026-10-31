/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * ── host_input / host_output (Stores::host_input_native / host_output_native, format.rs) ──
 * The program's headless channel to its host.  Input is stdin — on a page, the JS queue through
 * the `loft_io` imports, one message a call.  Output is a line on stderr — on a page, the
 * `loft_host_output` import the page routes to `globalThis.loftOutput`.
 *
 * `host_input(wait_ms)`: a negative wait reads the whole stream; 0 or more waits at most that
 * long for the first byte and answers what has arrived.  A read that stops inside a character
 * keeps its bytes for the next call; malformed bytes are answered as U+FFFD (the Rust runtime's
 * `takeable_len` / `take_utf8_prefix`).  A target that cannot wait for stdin (WASI) reads the
 * whole stream for every wait, so the VALUE stays right. */

#if defined(LOFT_BROWSER)
__attribute__((import_module("loft_io"), import_name("loft_host_input_len"))) size_t
loft_io_input_len(void);
__attribute__((import_module("loft_io"), import_name("loft_host_input_copy"))) void
loft_io_input_copy(unsigned char *p);
__attribute__((import_module("loft_io"), import_name("loft_host_output"))) void
loft_io_output(const char *p, size_t n);
#elif defined(LOFT_POSIX_HOST)
#include <poll.h>
#include <unistd.h>
#elif defined(_WIN32)
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#endif

/* `n` bytes as text, an invalid byte as U+FFFD (`String::from_utf8_lossy`). */
static void loft_hi_push_lossy(loft_text *t, const unsigned char *p, size_t n) {
    if (loft_utf8_valid(p, n)) {
        loft_push(t, (const char *) p, n);
        return;
    }
    for (size_t i = 0; i < n;) {
        size_t len = p[i] < 0x80 ? 1 : (p[i] >= 0xC2 && p[i] <= 0xDF) ? 2
                   : (p[i] >= 0xE0 && p[i] <= 0xEF) ? 3 : (p[i] >= 0xF0 && p[i] <= 0xF4) ? 4 : 0;
        if (len && i + len <= n && loft_utf8_valid(p + i, len)) {
            loft_push(t, (const char *) p + i, len);
            i += len;
        } else {
            loft_push(t, "\xEF\xBF\xBD", 3);
            i++;
        }
    }
}

static void loft_host_output(loft_str msg) {
#if defined(LOFT_BROWSER)
    loft_io_output(msg.p, msg.len);
#else
    fwrite(msg.p, 1, msg.len, stderr);
    fputc('\n', stderr);
    fflush(stderr);
#endif
}

#if defined(LOFT_BROWSER)
static loft_text loft_host_input(int64_t wait_ms) {
    (void) wait_ms; /* a page reads its queue without blocking: every read is a poll */
    loft_text out = { 0, 0, 0 };
    size_t n = loft_io_input_len();
    if (n == 0) return out;
    unsigned char *b = (unsigned char *) malloc(n);
    if (!b) return out;
    loft_io_input_copy(b);
    loft_hi_push_lossy(&out, b, n);
    free(b);
    return out;
}
#else
/* Bytes read and not yet answered — the tail of a character a read stopped inside — and
 * whether stdin has ended.  The OS's stream, buffered: no program-visible state. */
static unsigned char *loft_hi_buf;
static size_t loft_hi_len, loft_hi_cap;
static int loft_hi_eof;

static void loft_hi_add(const unsigned char *p, size_t n) {
    if (loft_hi_len + n > loft_hi_cap) {
        size_t cap = loft_hi_cap ? loft_hi_cap : 4096;
        while (cap < loft_hi_len + n) cap *= 2;
        unsigned char *b = (unsigned char *) realloc(loft_hi_buf, cap);
        if (!b) return;
        loft_hi_buf = b;
        loft_hi_cap = cap;
    }
    memcpy(loft_hi_buf + loft_hi_len, p, n);
    loft_hi_len += n;
}

/* Read stdin to its end. */
static void loft_hi_drain(void) {
    unsigned char chunk[65536];
    while (!loft_hi_eof) {
        size_t r = fread(chunk, 1, sizeof chunk, stdin);
        if (r > 0) loft_hi_add(chunk, r);
        if (r < sizeof chunk) loft_hi_eof = 1;
    }
}

/* Read what stdin holds, waiting up to `wait_ms` for the first byte. */
static void loft_hi_pull(int64_t wait_ms) {
#if defined(LOFT_POSIX_HOST)
    unsigned char chunk[65536];
    int timeout = wait_ms > 2147483647 ? 2147483647 : (int) wait_ms;
    while (!loft_hi_eof) {
        struct pollfd pfd = { 0, POLLIN, 0 };
        int ready = poll(&pfd, 1, timeout);
        if (ready <= 0) return;
        ssize_t r = read(0, chunk, sizeof chunk);
        if (r <= 0) {
            loft_hi_eof = 1;
            return;
        }
        loft_hi_add(chunk, (size_t) r);
        timeout = 0; /* what else is there now, without waiting again */
    }
#elif defined(_WIN32)
    HANDLE in = GetStdHandle(STD_INPUT_HANDLE);
    if (GetFileType(in) != FILE_TYPE_PIPE) {
        loft_hi_drain(); /* a file has all its bytes now */
        return;
    }
    int64_t until = loft_now_us() + wait_ms * 1000;
    unsigned char chunk[65536];
    while (!loft_hi_eof) {
        DWORD avail = 0, got = 0;
        if (!PeekNamedPipe(in, NULL, 0, NULL, &avail, NULL)) {
            loft_hi_eof = 1; /* the writer closed its end */
            return;
        }
        if (avail == 0) {
            if (loft_now_us() >= until) return;
            Sleep(1);
            continue;
        }
        if (!ReadFile(in, chunk, avail < sizeof chunk ? avail : (DWORD) sizeof chunk, &got, NULL)
            || got == 0) {
            loft_hi_eof = 1;
            return;
        }
        loft_hi_add(chunk, got);
        until = 0; /* what else is there now, without waiting again */
    }
#else
    (void) wait_ms;
    loft_hi_drain(); /* no way to wait for stdin here: the whole stream keeps the value right */
#endif
}

/* How many buffered bytes can be answered now: all of them at the end of the stream or when a
 * byte is malformed (waiting cannot repair it), else up to a character that is still arriving. */
static size_t loft_hi_takeable(void) {
    const unsigned char *p = loft_hi_buf;
    size_t n = loft_hi_len;
    if (loft_hi_eof) return n;
    for (size_t i = 0; i < n;) {
        unsigned char c = p[i];
        size_t len = c < 0x80 ? 1 : (c >= 0xC2 && c <= 0xDF) ? 2
                   : (c >= 0xE0 && c <= 0xEF) ? 3 : (c >= 0xF0 && c <= 0xF4) ? 4 : 0;
        if (len == 0) return n;
        if (i + len > n) {
            /* Cut off at the end: incomplete if every byte present could start the character. */
            for (size_t k = i + 1; k < n; k++) {
                unsigned char b = p[k];
                int ok = (b & 0xC0) == 0x80;
                if (k == i + 1) {
                    if (c == 0xE0) ok = b >= 0xA0 && b <= 0xBF;
                    else if (c == 0xED) ok = b >= 0x80 && b <= 0x9F;
                    else if (c == 0xF0) ok = b >= 0x90 && b <= 0xBF;
                    else if (c == 0xF4) ok = b >= 0x80 && b <= 0x8F;
                }
                if (!ok) return n;
            }
            return i;
        }
        if (!loft_utf8_valid(p + i, len)) return n;
        i += len;
    }
    return n;
}

static loft_text loft_host_input(int64_t wait_ms) {
    loft_text out = { 0, 0, 0 };
    if (wait_ms < 0) {
        loft_hi_drain();
    } else {
        loft_hi_pull(0);
        if (loft_hi_takeable() == 0 && !loft_hi_eof) loft_hi_pull(wait_ms);
    }
    size_t take = loft_hi_takeable();
    if (take == 0) return out;
    loft_hi_push_lossy(&out, loft_hi_buf, take);
    memmove(loft_hi_buf, loft_hi_buf + take, loft_hi_len - take);
    loft_hi_len -= take;
    return out;
}
#endif
