/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later */

/* ── Stores bound to a file (`@FR-S-Mapped`) ─────────────────────────────────────────────────
 * The interpreter's `Store::open_mapped` (src/store.rs), ported: a store whose buffer IS a file,
 * mapped shared, holding the S-Frame chain over the file's whole length with the allocator's
 * free-block links and footers in its free blocks.  The file grows by the same steps a heap
 * store's buffer grows by (`loft_resize_store`), is cut by the same `loft_st_shrink_to` with a
 * floor of `LOFT_MIN_BOUND_WORDS`, and is unmapped and closed when the store is released — the
 * bytes stay in the file.  A file one runtime wrote, the other opens and continues: the two
 * allocators place every record alike (`tests/c_emit.rs`
 * `a_store_bound_to_a_file_holds_the_same_bytes_on_both_runtimes`).
 *
 * Included after `windows.h` (loft_process.h) on Windows; on a POSIX host it brings its own
 * headers.  Where there is no mapping (WASI, `-DLOFT_NO_POSIX`) nothing here is defined and no
 * store is ever bound to a file. */
#if defined(LOFT_POSIX_HOST) || defined(_WIN32)
#define LOFT_MAPPED_STORE 1

#if !defined(_WIN32)
#include <fcntl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>
#endif

/* The file's length in bytes; -1 when it cannot be read. */
static int64_t loft_map_file_len(intptr_t f) {
#if defined(_WIN32)
    LARGE_INTEGER n;
    return GetFileSizeEx((HANDLE) f, &n) ? (int64_t) n.QuadPart : -1;
#else
    struct stat st;
    return fstat((int) f, &st) == 0 ? (int64_t) st.st_size : -1;
#endif
}

/* The file cut or extended to `bytes`; an extension reads zero.  0 when the OS refused. */
static int loft_map_file_set_len(intptr_t f, uint64_t bytes) {
#if defined(_WIN32)
    LARGE_INTEGER at;
    at.QuadPart = (LONGLONG) bytes;
    return SetFilePointerEx((HANDLE) f, at, 0, FILE_BEGIN) && SetEndOfFile((HANDLE) f);
#else
    return ftruncate((int) f, (off_t) bytes) == 0;
#endif
}

/* The file's first `bytes` mapped shared, read and write; null when the OS refused. */
static uint8_t *loft_map_view(intptr_t f, size_t bytes) {
#if defined(_WIN32)
    HANDLE m = CreateFileMappingA((HANDLE) f, 0, PAGE_READWRITE, (DWORD) ((uint64_t) bytes >> 32), (DWORD) bytes, 0);
    if (!m) return 0;
    void *v = MapViewOfFile(m, FILE_MAP_WRITE, 0, 0, bytes);
    /* The view keeps the mapping alive; the handle is not needed past it. */
    CloseHandle(m);
    return (uint8_t *) v;
#else
    void *v = mmap(0, bytes, PROT_READ | PROT_WRITE, MAP_SHARED, (int) f, 0);
    return v == MAP_FAILED ? 0 : (uint8_t *) v;
#endif
}

static void loft_map_unview(uint8_t *b, size_t bytes) {
#if defined(_WIN32)
    (void) bytes;
    UnmapViewOfFile(b);
#else
    munmap(b, bytes);
#endif
}

static void loft_map_close_file(intptr_t f) {
#if defined(_WIN32)
    CloseHandle((HANDLE) f);
#else
    close((int) f);
#endif
}

/* `Store::resize_store` / `shrink_to` on a mapped store (`MmapStorage::resize`): the mapping
 * dropped, the file set to `words` words and mapped again.  When the OS refuses the length the
 * old length is mapped again and 0 answered, the store as it was; a mapping that cannot be made
 * again leaves no store to continue with. */
static int loft_map_resize(loft_store_t *s, uint32_t words) {
    uint64_t bytes = (uint64_t) words * 8;
    loft_map_unview(s->buf, s->mapped);
    int ok = loft_map_file_set_len(s->map_file, bytes);
    size_t len = ok ? (size_t) bytes : s->mapped;
    uint8_t *b = loft_map_view(s->map_file, len);
    if (!b) { fputs("loft: a store bound to a file lost its mapping\n", stderr); abort(); }
    s->buf = b;
    s->mapped = len;
    if (ok) s->size = words;
    return ok;
}

/* The store's file unmapped and closed (`Store`'s drop of its `MmapStorage`); the store keeps no
 * buffer, so the slot's next store is made in a fresh one (`loft_store_make`). */
static void loft_st_close_mapped(loft_store_t *s) {
    if (!s->mapped) return;
    loft_map_unview(s->buf, s->mapped);
    loft_map_close_file(s->map_file);
    s->buf = 0;
    s->size = 0;
    s->mapped = 0;
    s->map_file = 0;
}

/* The OS half of `Store::try_open_mapped`: `path` opened read and write, created when missing,
 * and mapped whole — a file under `LOFT_MIN_BOUND_WORDS` words set to 8192 bytes first and
 * `*fresh` set, a store to initialise.  1 when mapped; 0 when the OS refused (the caller's miss:
 * a read-only cache directory); -1 when the file holds no store (`Unknown file format`, which the
 * interpreter asserts and its call arena reports — a refusal here too, nothing left open). */
static int loft_map_open_file(const char *path, intptr_t *file, uint8_t **buf, size_t *len, int *fresh) {
#if defined(_WIN32)
    HANDLE h = CreateFileA(path, GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, 0, OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0);
    if (h == INVALID_HANDLE_VALUE) return 0;
    intptr_t f = (intptr_t) h;
#else
    int fd = open(path, O_RDWR | O_CREAT, 0666);
    if (fd < 0) return 0;
    intptr_t f = (intptr_t) fd;
#endif
    int64_t n = loft_map_file_len(f);
    if (n < 0 || (uint64_t) n / 8 > (uint64_t) INT32_MAX) { loft_map_close_file(f); return 0; }
    *fresh = (uint64_t) n / 8 < LOFT_MIN_BOUND_WORDS;
    if (*fresh) {
        if (!loft_map_file_set_len(f, 8192)) { loft_map_close_file(f); return 0; }
        n = 8192;
    }
    uint8_t *b = loft_map_view(f, (size_t) n);
    if (!b) { loft_map_close_file(f); return 0; }
    if (!*fresh) {
        uint32_t sig;
        memcpy(&sig, b, 4);
        if (sig != LOFT_ST_SIGNATURE) {
            loft_map_unview(b, (size_t) n);
            loft_map_close_file(f);
            return -1;
        }
    }
    *file = f;
    *buf = b;
    *len = (size_t) n;
    return 1;
}

/* The store half: `s` takes the mapping, its side state that of a store just opened (no lazy
 * phase, no parking, defined bytes), then a fresh file is initialised, a read surface left
 * exactly as it is (`@FR-H-ReadSurface`: locked, nothing rebuilt into a file other processes map),
 * and any other file's recorded length cleared and its free tree and claims rebuilt from the
 * chain (`(S-Side)`, `loft_st_resync`).  The heap buffer `s` held is the caller's to release. */
static void loft_st_bind_mapped(loft_store_t *s, intptr_t f, uint8_t *b, size_t len, int fresh, int read_surface) {
    loft_map_resize_hook = loft_map_resize;
    loft_map_release_hook = loft_st_close_mapped;
    s->buf = b;
    s->size = (uint32_t) (len / 8);
    s->mapped = len;
    s->map_file = f;
    s->live = 1;
    s->read_only = 0;
    s->known = 0xFFFF;
    s->defined = 1;
    s->no_park = 1;
    s->lazy = 0;
    s->nplaced = 0;
    s->needs_coalesce = 0;
    s->claimed_end = 0;
    s->free_root = 0;
    s->wild = 0;
    s->dead_words = 0;
    loft_bits_clear(&s->claims);
    loft_bits_clear(&s->referenced);
    loft_bits_clear(&s->removed);
    loft_bits_clear(&s->lazy_free);
    loft_parked_clear(s);
    if (fresh) {
        loft_st_init(s);
    } else if (read_surface) {
        s->read_only = 1;
    } else {
        /* Bound, the file is a live store that grows: a written image's length no longer
         * holds for it (`(S-Frame)`). */
        uint32_t one = 1;
        memcpy(s->buf + 4, &one, 4);
        loft_st_resync(s);
    }
}

/* `Store::open` / `open_read_surface`: 1 when `s` is bound to the file, else 0 or -1 as
 * `loft_map_open_file` answers, `s` then as it was.  A store in the table that takes a file
 * releases its heap buffer first; `loft_free_ref` unmaps it.  (Binding one into the table is the
 * hosted unit's, @PLN206 P4: the call arena a C worker maps.) */
static int loft_st_open_mapped(loft_store_t *s, const char *path, int read_surface) {
    intptr_t f;
    uint8_t *b;
    size_t len;
    int fresh, rc = loft_map_open_file(path, &f, &b, &len, &fresh);
    if (rc == 1) loft_st_bind_mapped(s, f, b, len, fresh, read_surface);
    return rc;
}

#endif
