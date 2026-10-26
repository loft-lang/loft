/* ── Files (src/state/io.rs, src/database/io.rs, codegen_runtime.rs's fs_*) ────────────────
 * A `File` record: its size at byte 0, the position of the last read or write at 8, the next one
 * at 16 (null: not set), its path at 24, its handle at 28 (an index into `loft_files`, INT32_MIN
 * for none) and its format at 32 (1 TextFile, 2 LittleEndian, 3 BigEndian, 4 Directory,
 * 5 NotExists).
 *
 * What a file needs beyond C99 — a path's kind and size, a directory listing, mkdir, rmdir,
 * unlink, truncate, fsync — comes from POSIX where loft_cc.h finds it (`LOFT_POSIX`).  Without
 * it a file is what `fopen` opens, a directory is never seen, a listing is empty and the
 * directory operations answer Other. */

#if defined(LOFT_POSIX)
#include <sys/types.h>
#include <sys/stat.h>
#include <dirent.h>
#include <unistd.h>
#if defined(LOFT_FOREIGN)
#include <fcntl.h>
#include <sys/mman.h>
#endif
#endif
#include <errno.h>

#define LOFT_FS_OK 0
#define LOFT_FS_NOT_FOUND 1
#define LOFT_FS_PERMISSION_DENIED 2
#define LOFT_FS_IS_DIRECTORY 3
#define LOFT_FS_NOT_EMPTY 4
#define LOFT_FS_OTHER 9

/* The program's own directory, which a relative path resolves against (Stores::resolve_path);
 * the generated main sets it, LOFT_SOURCE_DIR overrides it.  `#cwd` turns the anchoring off. */
static const char *loft_anchor = "";
static int loft_program_relative = 1;

static void loft_set_anchor(const char *dir, int program_relative) {
    const char *env = getenv("LOFT_SOURCE_DIR");
    loft_anchor = env && *env ? env : dir;
    loft_program_relative = program_relative;
    env = getenv("LOFT_PATHS");
    if (env) loft_program_relative = (env[0] == 'p' || env[0] == 'P');
#if defined(LOFT_POSIX)
    /* The interpreter runs a program-relative program in its own directory (main.rs, before
     * the run), so the working directory `directory()` answers is that one. */
    if (loft_program_relative && *loft_anchor && chdir(loft_anchor) != 0) {}
#endif
}

/* Rust's `{c:?}` of a character: quoted, the escapes Rust writes, `\u{…}` for a control. */
static void loft_char_debug(loft_text *t, uint32_t c) {
    char b[16];
    loft_push_char(t, '\'');
    switch (c) {
    case 0: loft_push_cstr(t, "\\0"); break;
    case '\t': loft_push_cstr(t, "\\t"); break;
    case '\r': loft_push_cstr(t, "\\r"); break;
    case '\n': loft_push_cstr(t, "\\n"); break;
    case '\'': loft_push_cstr(t, "\\'"); break;
    case '\\': loft_push_cstr(t, "\\\\"); break;
    default:
        if (c < 0x20 || (c >= 0x7F && c <= 0x9F)) {
            snprintf(b, sizeof b, "\\u{%x}", (unsigned) c);
            loft_push_cstr(t, b);
        } else {
            size_t n = loft_utf8(c, b);
            loft_push(t, b, n);
        }
    }
    loft_push_char(t, '\'');
}

/* file_access::name_refusal: why no platform takes the name, or 0 (the reason malloc'd). */
static char *loft_name_refusal(const char *p, size_t n) {
    if (n == 0 || (n == 1 && p[0] == '.') || (n == 2 && p[0] == '.' && p[1] == '.')) return 0;
    loft_text t = { 0, 0, 0 };
    for (size_t i = 0; i < n; i++) {
        unsigned char c = (unsigned char) p[i];
        uint32_t cp = c;
        /* A control character (U+0000–001F, U+007F–009F) or one of `<>:"\|?*`. */
        if (c == 0xC2 && i + 1 < n && (unsigned char) p[i + 1] >= 0x80 && (unsigned char) p[i + 1] <= 0x9F)
            cp = (unsigned char) p[i + 1];
        else if (!(c < 0x20 || c == 0x7F || strchr("<>:\"\\|?*", c)))
            continue;
        loft_push_cstr(&t, "the name `");
        loft_push(&t, p, n);
        loft_push_cstr(&t, "` holds ");
        loft_char_debug(&t, cp);
        loft_push_cstr(&t, ", which a file name cannot hold on every platform");
        break;
    }
    if (!t.len) {
        size_t stem = 0;
        while (stem < n && p[stem] != '.') stem++;
        char up[5] = { 0, 0, 0, 0, 0 };
        int device = 0;
        if (stem == 3 || stem == 4) {
            for (size_t i = 0; i < stem; i++) up[i] = (char) (p[i] >= 'a' && p[i] <= 'z' ? p[i] - 32 : p[i]);
            device = (stem == 3 && (!strcmp(up, "CON") || !strcmp(up, "PRN") || !strcmp(up, "AUX") || !strcmp(up, "NUL")))
                || (stem == 4 && (!strncmp(up, "COM", 3) || !strncmp(up, "LPT", 3)) && up[3] >= '1' && up[3] <= '9');
        }
        const char *why = device ? "` is a device name on Windows, so no platform takes it"
            : p[n - 1] == '.' || p[n - 1] == ' ' ? "` ends in a dot or a space, which Windows drops" : 0;
        if (!why) return 0;
        loft_push_cstr(&t, "the name `");
        loft_push(&t, p, n);
        loft_push_cstr(&t, why);
    }
    loft_push_char(&t, 0);
    return t.p;
}

/* `log_refusal_once`: one line per refused path per process. */
static void loft_log_refusal_once(loft_str raw, const char *why) {
    static loft_text *said = 0;
    static size_t nsaid = 0;
    for (size_t i = 0; i < nsaid; i++)
        if (said[i].len == raw.len && memcmp(said[i].p, raw.p, raw.len) == 0) return;
    said = (loft_text *) realloc(said, (nsaid + 1) * sizeof *said);
    if (!said) { fputs("loft: out of memory\n", stderr); abort(); }
    said[nsaid++] = loft_text_own(raw);
    fprintf(stderr, "loft: the path `%.*s` is refused: %s\n", (int) raw.len, raw.p, why);
}

/* PathText's normal form of `raw`, malloc'd: '.' dropped, '..' folded, '/' between names — under
 * `base` when `raw` is relative and `base` is not empty.  With `check`, a name no platform
 * takes refuses the path (said on stderr, as `log_refusal_once` says it) and answers 0. */
static char *loft_normalize(loft_str raw, const char *base, int check) {
    size_t blen = strlen(base);
    char *out = (char *) malloc(blen + raw.len + 3);
    if (!out) { fputs("loft: out of memory\n", stderr); abort(); }
    if (raw.len == 0) {
        strcpy(out, base);
        return out;
    }
    int rooted = raw.p[0] == '/' || raw.p[0] == '\\';
    /* The names, each in `parts` from the offset in `at`. */
    char *parts = (char *) malloc(raw.len + 1);
    size_t *at = (size_t *) malloc((raw.len + 2) * sizeof *at);
    if (!parts || !at) { fputs("loft: out of memory\n", stderr); abort(); }
    size_t np = 0, used = 0, i = 0;
    at[0] = 0;
    while (i <= raw.len) {
        size_t s = i;
        while (i < raw.len && raw.p[i] != '/' && raw.p[i] != '\\') i++;
        const char *name = raw.p + s;
        size_t n = i - s;
        i++;
        if (n == 0 || (n == 1 && name[0] == '.')) continue;
        char *why = check ? loft_name_refusal(name, n) : 0;
        if (why) {
            loft_log_refusal_once(raw, why);
            free(why);
            free(parts);
            free(at);
            free(out);
            return 0;
        }
        if (n == 2 && name[0] == '.' && name[1] == '.') {
            if (np > 0 && !(at[np] - at[np - 1] == 3 && parts[at[np - 1]] == '.' && parts[at[np - 1] + 1] == '.')) {
                used = at[--np];
                continue;
            }
            if (rooted) continue;
        }
        memcpy(parts + used, name, n);
        used += n;
        parts[used++] = '/';
        at[++np] = used;
    }
    size_t o = 0;
    if (blen && !rooted) {
        memcpy(out, base, blen);
        o = blen;
        if (np > 0 && out[o - 1] != '/') out[o++] = '/';
    } else if (rooted) {
        out[o++] = '/';
    }
    if (np > 0) {
        memcpy(out + o, parts, used - 1);
        o += used - 1;
    }
    out[o] = 0;
    free(parts);
    free(at);
    return out;
}

/* Stores::resolve_path: the path the program's `raw` names — a relative one under the
 * program's directory unless the program said `#cwd` — or 0 for a refused name. */
#if defined(LOFT_POSIX)
/* ASCII case-insensitive equality of two names. */
static int loft_name_fold_eq(const char *a, size_t an, const char *b, size_t bn) {
    if (an != bn) return 0;
    for (size_t i = 0; i < an; i++) {
        char x = a[i] >= 'A' && a[i] <= 'Z' ? (char) (a[i] + 32) : a[i];
        char y = b[i] >= 'A' && b[i] <= 'Z' ? (char) (b[i] + 32) : b[i];
        if (x != y) return 0;
    }
    return 1;
}

/* `@FR-Path-Case` (file_access::case_clash): why a name on `full`'s way names another entry's
 * spelling, or 0 (malloc'd).  The longest existing prefix is compared with its on-disk
 * spelling where the file system folds case (macOS); the first missing name below it with
 * the entries of its directory. */
static char *loft_case_clash(const char *full) {
    size_t len = strlen(full);
    char *existing = (char *) malloc(len + 2);
    if (!existing) { fputs("loft: out of memory\n", stderr); abort(); }
    memcpy(existing, full, len + 1);
    char *missing = 0;
    struct stat st;
    while (stat(existing, &st) != 0) {
        char *slash = strrchr(existing, '/');
        if (!slash) { free(existing); free(missing); return 0; }
        free(missing);
        missing = (char *) malloc(strlen(slash + 1) + 1);
        if (!missing) { fputs("loft: out of memory\n", stderr); abort(); }
        strcpy(missing, slash + 1);
        if (slash == existing) slash[1] = 0; else *slash = 0;
    }
    loft_text why = { 0, 0, 0 };
#if defined(__APPLE__)
    char real[4096];
    if (realpath(existing, real)) {
        /* The trailing names, compared pairwise from the end. */
        const char *ae = existing + strlen(existing), *re = real + strlen(real);
        while (ae > existing && re > real) {
            const char *as = ae, *rs = re;
            while (as > existing && as[-1] != '/') as--;
            while (rs > real && rs[-1] != '/') rs--;
            size_t an = (size_t) (ae - as), rn = (size_t) (re - rs);
            if ((an != rn || memcmp(as, rs, an) != 0) && loft_name_fold_eq(as, an, rs, rn)) {
                loft_push_char(&why, '`');
                loft_push(&why, as, an);
                loft_push_cstr(&why, "` is spelled `");
                loft_push(&why, rs, rn);
                loft_push_cstr(&why, "` on disk");
                break;
            }
            ae = as > existing ? as - 1 : as;
            re = rs > real ? rs - 1 : rs;
            if (as == existing || rs == real) break;
        }
    }
#endif
    if (!why.len && missing) {
        DIR *d = opendir(existing);
        if (d) {
            char *best = 0;
            size_t mn = strlen(missing);
            struct dirent *e;
            while ((e = readdir(d)) != 0) {
                size_t en = strlen(e->d_name);
                if (!strcmp(e->d_name, ".") || !strcmp(e->d_name, "..")) continue;
                if ((en != mn || memcmp(e->d_name, missing, mn) != 0) && loft_name_fold_eq(e->d_name, en, missing, mn)
                    && (!best || strcmp(e->d_name, best) < 0)) {
                    free(best);
                    best = (char *) malloc(en + 1);
                    if (!best) { fputs("loft: out of memory\n", stderr); abort(); }
                    strcpy(best, e->d_name);
                }
            }
            closedir(d);
            if (best) {
                loft_push_char(&why, '`');
                loft_push_cstr(&why, missing);
                loft_push_cstr(&why, "` would be a second spelling of `");
                loft_push_cstr(&why, best);
                loft_push_char(&why, '`');
                free(best);
            }
        }
    }
    free(existing);
    free(missing);
    if (!why.len) return 0;
    loft_push_char(&why, 0);
    return why.p;
}
#endif

static char *loft_resolve(loft_str raw) {
    char *full = loft_normalize(raw, loft_program_relative ? loft_anchor : "", 1);
#if defined(LOFT_POSIX)
    if (full && raw.len) {
        char *why = loft_case_clash(full);
        if (why) {
            loft_log_refusal_once(raw, why);
            free(why);
            free(full);
            return 0;
        }
    }
#endif
    return full;
}

/* file_access::given: a directory the OS hands the program, in the portable form. */
static loft_text loft_given(const char *path) {
    char *p = loft_normalize(loft_lit(path, strlen(path)), "", 0);
    loft_text t = loft_text_own(loft_lit(p, strlen(p)));
    free(p);
    return t;
}

static char *loft_file_path(loft_ref f) {
    return loft_resolve(loft_get_text(f, 24));
}

/* The open handles a File record's byte 28 indexes (Stores::files), never reused, and which of
 * them were opened for reading only (LoftFile::reader). */
static FILE **loft_files = 0;
static uint8_t *loft_files_ro = 0;
static int32_t loft_nfiles = 0;

static int32_t loft_file_handle_add(FILE *h, int ro) {
    FILE **n = (FILE **) realloc(loft_files, (size_t) (loft_nfiles + 1) * sizeof *n);
    uint8_t *r = (uint8_t *) realloc(loft_files_ro, (size_t) (loft_nfiles + 1));
    if (!n || !r) { fputs("loft: out of memory\n", stderr); abort(); }
    loft_files = n;
    loft_files_ro = r;
    loft_files[loft_nfiles] = h;
    loft_files_ro[loft_nfiles] = (uint8_t) ro;
    return loft_nfiles++;
}

static FILE *loft_file_handle(loft_ref f) {
    int32_t h;
    memcpy(&h, loft_at(loft_store_of(f), f.rec, f.pos + 28), 4);
    return h >= 0 && h < loft_nfiles ? loft_files[h] : 0;
}

static char *loft_file_path(loft_ref f);

/* LoftFile::writable: before a write, a handle opened for reading becomes a read-write one on
 * the same path; one that cannot stays a reader, and the write fails. */
static FILE *loft_file_writable(loft_ref f, FILE *h) {
    int32_t nr;
    memcpy(&nr, loft_at(loft_store_of(f), f.rec, f.pos + 28), 4);
    if (!h || nr < 0 || nr >= loft_nfiles || !loft_files_ro[nr]) return h;
    char *path = loft_file_path(f);
    FILE *w = path ? fopen(path, "r+b") : 0;
    free(path);
    if (!w) return 0;
    fclose(h);
    loft_files[nr] = w;
    loft_files_ro[nr] = 0;
    return w;
}

static void loft_file_set_long(loft_ref f, uint32_t fld, int64_t v) {
    memcpy(loft_at(loft_store_of(f), f.rec, f.pos + fld), &v, 8);
}

static int64_t loft_file_long(loft_ref f, uint32_t fld) {
    int64_t v;
    memcpy(&v, loft_at(loft_store_of(f), f.rec, f.pos + fld), 8);
    return v;
}

static uint8_t loft_file_format(loft_ref f) {
    return *loft_at(loft_store_of(f), f.rec, f.pos + 32);
}

/* The path's kind (1 file, 4 directory, 5 neither, 0 absent) and its size. */
static int loft_stat(const char *path, int64_t *size) {
#if defined(LOFT_POSIX)
    struct stat st;
    if (stat(path, &st) != 0) return 0;
    *size = (int64_t) st.st_size;
    if (S_ISDIR(st.st_mode)) return 4;
    if (S_ISREG(st.st_mode)) return 1;
    return 5;
#else
    FILE *h = fopen(path, "rb");
    if (!h) return 0;
    long n = fseek(h, 0, SEEK_END) == 0 ? ftell(h) : -1;
    fclose(h);
    *size = n < 0 ? 0 : (int64_t) n;
    return 1;
#endif
}

/* fill_absent / fill_file: the File record describes the path. */
static void loft_fill_absent(loft_ref f) {
    loft_file_set_long(f, 0, 0);
    loft_file_set_long(f, 8, LOFT_NULL);
    loft_file_set_long(f, 16, LOFT_NULL);
    *loft_at(loft_store_of(f), f.rec, f.pos + 32) = 5;
}

static int loft_fill_file(loft_ref f, const char *path) {
    int64_t size = 0;
    int kind = loft_stat(path, &size);
    if (kind == 0) {
        loft_fill_absent(f);
        return 0;
    }
    loft_file_set_long(f, 8, LOFT_NULL);
    loft_file_set_long(f, 16, LOFT_NULL);
    loft_file_set_long(f, 0, kind == 1 ? size : LOFT_NULL);
    *loft_at(loft_store_of(f), f.rec, f.pos + 32) = (uint8_t) kind;
    return 1;
}

/* OpGetFile (Stores::get_file): what the File's path names. */
static uint8_t loft_get_file(loft_ref f) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return 0;
    char *path = loft_file_path(f);
    if (!path) {
        loft_fill_absent(f);
        return 0;
    }
    int r = loft_fill_file(f, path);
    free(path);
    return (uint8_t) r;
}

/* OpGetFileText (read_file_text_into): the whole file as text; nothing for a missing file, and
 * nothing — with a warning — for bytes that are not UTF-8. */
static void loft_get_file_text(loft_ref f, loft_text *out) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return;
    out->len = 0;
    char *path = loft_file_path(f);
    if (!path) return;
    FILE *h = fopen(path, "rb");
    if (!h) { free(path); return; }
    char buf[4096];
    size_t n;
    while ((n = fread(buf, 1, sizeof buf, h)) > 0) loft_push(out, buf, n);
    fclose(h);
    if (!loft_utf8_valid((const unsigned char *) out->p, out->len)) {
        fprintf(stderr, "warning: file(\"%s\").content() got non-UTF-8 bytes (%lu bytes in file) — returning null. "
                "Read the bytes exactly with `read_bytes(path)`, or a field at a time with `f#format = LittleEndian; "
                "f#read(n)` (or `BigEndian`); see the loft-write skill § File I/O.\n", path, (unsigned long) out->len);
        out->len = 0;
    }
    free(path);
}

/* Is a write or read through this File in little-endian order (its format LittleEndian)? */
static int loft_file_le(loft_ref f) {
    return f.store_nr != 0xFFFF && f.rec != 0 && loft_file_format(f) == 2;
}

/* OpWriteFile (State::write_file): `bytes` at the File's next position — the end of the file
 * when none was set, so `f += x` appends — through a handle opened on first use. */
static void loft_write_file(loft_ref f, const loft_text *bytes) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return;
    uint8_t format = loft_file_format(f);
    if (format != 1 && format != 2 && format != 3 && format != 5) return;
    int64_t next = loft_file_long(f, 16);
    if (next == LOFT_NULL) {
        char *path = loft_file_path(f);
        int64_t size = 0;
        next = path && loft_stat(path, &size) != 0 ? size : 0;
        free(path);
    }
    loft_file_set_long(f, 8, next);
    FILE *h = loft_file_handle(f);
    if (!h) {
        char *path = loft_file_path(f);
        if (!path) { fputs("file open error: the path is refused\n", stderr); return; }
        h = fopen(path, "r+b");
        if (!h) h = fopen(path, "w+b");
        if (!h) {
            fprintf(stderr, "file open error: %s\n", strerror(errno));
            free(path);
            return;
        }
        free(path);
        int32_t nr = loft_file_handle_add(h, 0);
        memcpy(loft_at(loft_store_of(f), f.rec, f.pos + 28), &nr, 4);
        if (format == 5) *loft_at(loft_store_of(f), f.rec, f.pos + 32) = 1;
    } else if (!(h = loft_file_writable(f, h))) {
        fputs("file write error\n", stderr);
        loft_file_set_long(f, 16, next + (int64_t) bytes->len);
        return;
    }
    fseek(h, (long) next, SEEK_SET);
    /* Unbuffered, as the interpreter's `write_all` is: another handle on the path reads it. */
    if (bytes->len && (fwrite(bytes->p, 1, bytes->len, h) != bytes->len || fflush(h) != 0)) fputs("file write error\n", stderr);
    loft_file_set_long(f, 16, next + (int64_t) bytes->len);
}

/* OpReadFile (State::read_file): up to `n` bytes from the File's next position into `*out`
 * (malloc'd), and how many came: a text takes what arrived, a binary value all `n` or none, and
 * a count below the value's width (`width`, 0 for a text) none at all.  -1 when the File
 * cannot be read, which reads nothing into the value. */
static int64_t loft_read_file(loft_ref f, int64_t n, int text, int64_t width, uint8_t **out) {
    *out = 0;
    if (f.store_nr == 0xFFFF || f.rec == 0) return -1;
    uint8_t format = loft_file_format(f);
    if (format != 1 && format != 2 && format != 3 && format != 5) return -1;
    int64_t next = loft_file_long(f, 16);
    if (next == LOFT_NULL) next = 0;
    loft_file_set_long(f, 8, next);
    FILE *h = loft_file_handle(f);
    if (!h) {
        char *path = loft_file_path(f);
        h = path ? fopen(path, "rb") : 0;
        free(path);
        if (!h) {
            fprintf(stderr, "file open error: %s\n", strerror(errno));
            return -1;
        }
        int32_t nr = loft_file_handle_add(h, 1);
        memcpy(loft_at(loft_store_of(f), f.rec, f.pos + 28), &nr, 4);
    }
    fseek(h, (long) next, SEEK_SET);
    size_t want = n < 0 ? 0 : (size_t) n;
    *out = (uint8_t *) malloc(want + 1);
    if (!*out) { fputs("loft: out of memory\n", stderr); abort(); }
    size_t got = 0;
    if (text || n >= width) {
        got = fread(*out, 1, want, h);
        if (!text && got < want) got = 0;
    }
    loft_file_set_long(f, 16, next + (int64_t) got);
    return (int64_t) got;
}

/* OpSeekFile: the next read or write starts at `pos` — through the open handle, which the next
 * read or write moves to its own position again, or as the position the first one opens at. */
static void loft_seek_file(loft_ref f, int64_t pos) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return;
    FILE *h = loft_file_handle(f);
    if (!h) loft_file_set_long(f, 16, pos);
    else if (fseek(h, (long) pos, SEEK_SET) != 0) fprintf(stderr, "file seek error: %s\n", strerror(errno));
}

/* OpSizeFile: the file's size now, or null. */
static int64_t loft_size_file(loft_ref f) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return LOFT_NULL;
    char *path = loft_file_path(f);
    int64_t size = 0;
    int kind = path ? loft_stat(path, &size) : 0;
    free(path);
    return kind == 0 ? LOFT_NULL : size;
}

/* OpSyncFile: the written bytes reach the device. */
static uint8_t loft_sync_file(loft_ref f) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return 0;
    FILE *h = loft_file_handle(f);
    if (!h) return 1;
    if (fflush(h) != 0) return 0;
#if defined(LOFT_POSIX)
    return (uint8_t) (fsync(fileno(h)) == 0);
#else
    return 1;
#endif
}

/* OpTruncateFile: the file is `size` bytes long; an open handle is closed first. */
static uint8_t loft_truncate_file(loft_ref f, int64_t size) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return 0;
    int32_t h;
    memcpy(&h, loft_at(loft_store_of(f), f.rec, f.pos + 28), 4);
    if (h >= 0 && h < loft_nfiles) {
        if (loft_files[h]) fclose(loft_files[h]);
        loft_files[h] = 0;
        int32_t none = INT32_MIN;
        memcpy(loft_at(loft_store_of(f), f.rec, f.pos + 28), &none, 4);
        loft_file_set_long(f, 8, LOFT_NULL);
        loft_file_set_long(f, 16, LOFT_NULL);
    }
    char *path = loft_file_path(f);
    if (!path) return 0;
    int ok;
#if defined(LOFT_POSIX)
    int64_t old = 0;
    ok = loft_stat(path, &old) == 1 && truncate(path, (off_t) (size < 0 ? 0 : size)) == 0;
#else
    FILE *r = fopen(path, "rb");
    ok = r != 0;
    if (ok) {
        size_t keep = size < 0 ? 0 : (size_t) size;
        char *buf = (char *) calloc(keep + 1, 1);
        if (!buf) { fputs("loft: out of memory\n", stderr); abort(); }
        size_t got = fread(buf, 1, keep, r);
        (void) got;
        fclose(r);
        FILE *w = fopen(path, "wb");
        ok = w && fwrite(buf, 1, keep, w) == keep;
        if (w) fclose(w);
        free(buf);
    }
#endif
    free(path);
    return (uint8_t) ok;
}

/* fs_classify: an errno as a FileResult code. */
static int64_t loft_fs_code(int err) {
    if (err == ENOENT) return LOFT_FS_NOT_FOUND;
#if defined(LOFT_POSIX)
    if (err == EACCES || err == EPERM) return LOFT_FS_PERMISSION_DENIED;
#endif
    return LOFT_FS_OTHER;
}

static int loft_is_dir(const char *path) {
    int64_t size;
    return loft_stat(path, &size) == 4;
}

/* OpDelete: the file goes; a directory is refused as one. */
static int64_t loft_fs_delete(loft_str raw) {
    char *path = loft_resolve(raw);
    if (!path) return LOFT_FS_OTHER;
    int64_t code = LOFT_FS_OK;
#if defined(LOFT_POSIX)
    if (unlink(path) != 0) code = loft_is_dir(path) ? LOFT_FS_IS_DIRECTORY : loft_fs_code(errno);
#else
    if (remove(path) != 0) code = loft_fs_code(errno);
#endif
    free(path);
    return code;
}

/* OpMoveFile: `from` is renamed to `to`. */
static int64_t loft_fs_move(loft_str from, loft_str to) {
    char *a = loft_resolve(from), *b = loft_resolve(to);
    int64_t code = !a || !b ? LOFT_FS_OTHER : rename(a, b) == 0 ? LOFT_FS_OK : loft_fs_code(errno);
    free(a);
    free(b);
    return code;
}

/* OpMkdir / OpMkdirAll: the directory (and with `all` its parents; one already there is fine). */
static int64_t loft_fs_mkdir(loft_str raw, int all) {
    char *path = loft_resolve(raw);
    if (!path) return LOFT_FS_OTHER;
    int64_t code = LOFT_FS_OTHER;
#if defined(LOFT_POSIX)
    if (all) {
        for (char *p = path + 1; *p; p++) {
            if (*p != '/') continue;
            *p = 0;
            if (mkdir(path, 0777) != 0 && errno != EEXIST) {
                *p = '/';
                code = loft_fs_code(errno);
                free(path);
                return code;
            }
            *p = '/';
        }
        code = mkdir(path, 0777) == 0 || (errno == EEXIST && loft_is_dir(path)) ? LOFT_FS_OK : loft_fs_code(errno);
    } else {
        code = mkdir(path, 0777) == 0 ? LOFT_FS_OK : loft_fs_code(errno);
    }
#else
    (void) all;
#endif
    free(path);
    return code;
}

/* OpRmdir: the EMPTY directory goes. */
static int64_t loft_fs_rmdir(loft_str raw) {
    char *path = loft_resolve(raw);
    if (!path) return LOFT_FS_OTHER;
    int64_t code = LOFT_FS_OTHER;
#if defined(LOFT_POSIX)
    if (rmdir(path) == 0) code = LOFT_FS_OK;
    else if (errno == ENOENT) code = LOFT_FS_NOT_FOUND;
    else if (errno == EACCES || errno == EPERM) code = LOFT_FS_PERMISSION_DENIED;
    else if (errno == ENOTEMPTY || errno == EEXIST) code = LOFT_FS_NOT_EMPTY;
#endif
    free(path);
    return code;
}

static int loft_cmp_cstr(const void *a, const void *b) {
    return strcmp(*(char *const *) a, *(char *const *) b);
}

/* OpGetDir (Stores::get_dir): a File record per entry of the directory, in name order, each
 * filled as `file()` fills one; `size` is a File record's.  True whenever the path is not
 * refused, whether or not it could be listed. */
static uint8_t loft_get_dir(loft_str raw, loft_ref v, uint32_t size) {
    char *dir = loft_resolve(raw);
    if (!dir) return 0;
#if defined(LOFT_POSIX)
    DIR *d = opendir(dir);
    if (d) {
        char **names = 0;
        size_t n = 0, cap = 0;
        struct dirent *e;
        while ((e = readdir(d)) != 0) {
            if (!strcmp(e->d_name, ".") || !strcmp(e->d_name, "..")) continue;
            if (n == cap) {
                cap = cap ? cap * 2 : 16;
                names = (char **) realloc(names, cap * sizeof *names);
                if (!names) { fputs("loft: out of memory\n", stderr); abort(); }
            }
            size_t dl = strlen(dir), nl = strlen(e->d_name);
            names[n] = (char *) malloc(dl + nl + 2);
            if (!names[n]) { fputs("loft: out of memory\n", stderr); abort(); }
            memcpy(names[n], dir, dl);
            names[n][dl] = '/';
            memcpy(names[n] + dl + 1, e->d_name, nl + 1);
            n++;
        }
        closedir(d);
        if (n > 1) qsort(names, n, sizeof *names, loft_cmp_cstr);
        for (size_t i = 0; i < n; i++) {
            loft_ref el = loft_vec_append(v, size);
            if (el.rec != 0) {
                loft_set_text(el, 24, loft_lit(names[i], strlen(names[i])));
                int32_t none = INT32_MIN;
                memcpy(loft_at(loft_store_of(el), el.rec, el.pos + 28), &none, 4);
                loft_file_set_long(el, 8, LOFT_NULL);
                loft_file_set_long(el, 16, LOFT_NULL);
                loft_vec_finish(v);
                loft_fill_file(el, names[i]);
            }
            free(names[i]);
        }
        free(names);
    }
#else
    (void) v;
    (void) size;
#endif
    free(dir);
    return 1;
}

/* ── Binary forms (Stores::read_data / write_data) ──
 * A value's file form: an integer, long or float in 8 bytes, a single or character in 4, a
 * boolean or byte in 1, a text its bytes, a record its fields one after another, a vector its
 * elements — in the File's byte order. */

static void loft_put(loft_text *b, uint64_t v, int n, int le) {
    char c[8];
    for (int i = 0; i < n; i++) c[le ? i : n - 1 - i] = (char) (unsigned char) (v >> (8 * i));
    loft_push(b, c, (size_t) n);
}

static uint64_t loft_take(const uint8_t *d, int n, int le) {
    uint64_t v = 0;
    for (int i = 0; i < n; i++) v |= (uint64_t) d[le ? i : n - 1 - i] << (8 * i);
    return v;
}

static uint32_t loft_bits32(float g) { uint32_t u; memcpy(&u, &g, 4); return u; }
static uint64_t loft_bits64(double d) { uint64_t u; memcpy(&u, &d, 8); return u; }

static uint64_t loft_u64(loft_store_t *s, uint32_t rec, uint32_t byte) {
    uint64_t v;
    memcpy(&v, loft_at(s, rec, byte), 8);
    return v;
}

/* read_data of the builtin types, at a place in a store. */
static void loft_ser0(loft_text *b, loft_ref r, int le) { loft_put(b, loft_u64(loft_store_of(r), r.rec, r.pos), 8, le); }
static void loft_ser1(loft_text *b, loft_ref r, int le) { loft_ser0(b, r, le); }
static void loft_ser2(loft_text *b, loft_ref r, int le) { loft_put(b, loft_u32(loft_store_of(r), r.rec, r.pos), 4, le); }
static void loft_ser3(loft_text *b, loft_ref r, int le) { loft_ser0(b, r, le); }
static void loft_ser4(loft_text *b, loft_ref r, int le) { (void) le; loft_push_char(b, (char) *loft_at(loft_store_of(r), r.rec, r.pos)); }
static void loft_ser5(loft_text *b, loft_ref r, int le) { (void) le; loft_text_append(b, loft_get_text(r, 0)); }
static void loft_ser6(loft_text *b, loft_ref r, int le) { loft_ser2(b, r, le); }

/* write_data of the builtin types: `d` holds `n` bytes, at least the type's width. */
static void loft_de0(loft_ref r, int le, const uint8_t *d, size_t n) {
    (void) n;
    uint64_t v = loft_take(d, 8, le);
    memcpy(loft_at(loft_store_of(r), r.rec, r.pos), &v, 8);
}
static void loft_de1(loft_ref r, int le, const uint8_t *d, size_t n) { loft_de0(r, le, d, n); }
static void loft_de2(loft_ref r, int le, const uint8_t *d, size_t n) {
    (void) n;
    uint32_t v = (uint32_t) loft_take(d, 4, le);
    memcpy(loft_at(loft_store_of(r), r.rec, r.pos), &v, 4);
}
static void loft_de3(loft_ref r, int le, const uint8_t *d, size_t n) { loft_de0(r, le, d, n); }
static void loft_de4(loft_ref r, int le, const uint8_t *d, size_t n) { (void) le; (void) n; *loft_at(loft_store_of(r), r.rec, r.pos) = d[0]; }
static void loft_de5(loft_ref r, int le, const uint8_t *d, size_t n) { (void) le; loft_set_text(r, 0, loft_lit((const char *) d, n)); }
static void loft_de6(loft_ref r, int le, const uint8_t *d, size_t n) { loft_de2(r, le, d, n); }

/* ── file natives ── */

/* read_bytes (Stores::fs_read_bytes): the whole file as a `vector<u8>` in a store of its own,
 * or null for a missing or unreadable one. */
static loft_ref loft_read_bytes(loft_str raw) {
    char *path = loft_resolve(raw);
    FILE *h = path ? fopen(path, "rb") : 0;
    free(path);
    if (!h) return loft_ref_null();
    loft_ref r = loft_ref_null();
    loft_database(&r, 8);
    char buf[4096];
    size_t n;
    while ((n = fread(buf, 1, sizeof buf, h)) > 0) {
        for (size_t i = 0; i < n; i++) {
            loft_ref e = loft_vec_append(r, 1);
            *loft_at(loft_store_of(e), e.rec, e.pos) = (uint8_t) buf[i];
            loft_vec_finish(r);
        }
    }
    fclose(h);
    return r;
}

#if defined(LOFT_FOREIGN)
#if defined(LOFT_POSIX)
static void loft_unmap(void *bytes, size_t n) { munmap(bytes, n); }
#endif

/* file_map (Stores::fs_file_map): the whole file mapped read-only as a FOREIGN `vector<u8>` —
 * a store of its own whose slot names `LOFT_FOREIGN_REC`, the mapping its owner — or null for a
 * missing or unreadable one.  An empty file, or one that is not a regular file, is read as
 * read_bytes reads it; so is every file where there is no mmap. */
static loft_ref loft_file_map(loft_str raw) {
#if defined(LOFT_POSIX)
    char *path = loft_resolve(raw);
    if (!path) return loft_ref_null();
    int fd = open(path, O_RDONLY);
    free(path);
    if (fd < 0) return loft_ref_null();
    struct stat st;
    if (fstat(fd, &st) != 0 || (uint64_t) st.st_size > UINT32_MAX) {
        close(fd);
        return loft_ref_null();
    }
    if (st.st_size == 0 || !S_ISREG(st.st_mode)) {
        close(fd);
        return loft_read_bytes(raw);
    }
    size_t n = (size_t) st.st_size;
    void *map = mmap(0, n, PROT_READ, MAP_PRIVATE, fd, 0);
    close(fd);
    if (map == MAP_FAILED) return loft_ref_null();
    loft_fowner *o = (loft_fowner *) malloc(sizeof *o);
    if (!o) { fputs("loft: out of memory\n", stderr); abort(); }
    o->refs = 1;
    o->bytes = map;
    o->n = n;
    o->release = loft_unmap;
    loft_fspan span;
    span.base = (const uint8_t *) map;
    span.len = (uint32_t) n;
    span.elem = 1;
    span.owner = o;
    /* Stores::vector_buffer(4): a fresh store whose root slot names the empty vector. */
    loft_ref r = loft_ref_null();
    loft_database(&r, 4);
    loft_set_u32(loft_store_of(r), r.rec, r.pos, 0);
    loft_st_make_foreign(loft_store_of(r), r.rec, r.pos, span);
    return r;
#else
    return loft_read_bytes(raw);
#endif
}
#endif

/* write_bytes (Stores::fs_write_bytes): the file is the vector's bytes, replacing what it held. */
static uint8_t loft_write_bytes(loft_str raw, loft_ref v) {
    char *path = loft_resolve(raw);
    if (!path) return 0;
    FILE *h = fopen(path, "wb");
    free(path);
    if (!h) return 0;
    int64_t n = loft_vec_len(v);
    int ok = 1;
    if (n > 0) {
        uint32_t r = loft_collection_rec(loft_store_of(v), v.rec, v.pos);
        ok = fwrite(loft_at(loft_store_of(v), r, 8), 1, (size_t) n, h) == (size_t) n;
    }
    return (uint8_t) (fclose(h) == 0 && ok);
}

/* write_text_raw (Stores::write_file): `v` at the File's position — its first write creates
 * the file empty — leaving the position around what it wrote. */
static uint8_t loft_write_text_raw(loft_ref f, loft_str v) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return 0;
    FILE *h = loft_file_handle(f);
    if (!h) {
        char *path = loft_file_path(f);
        if (!path) return 0;
        h = fopen(path, "w+b");
        free(path);
        if (!h) {
            fprintf(stderr, "loft: cannot create a file for writing: %s — write skipped\n", strerror(errno));
            return 0;
        }
        int32_t nr = loft_file_handle_add(h, 0);
        memcpy(loft_at(loft_store_of(f), f.rec, f.pos + 28), &nr, 4);
    } else if (!(h = loft_file_writable(f, h))) {
        return 0;
    }
    int64_t next = loft_file_long(f, 16);
    if (next != LOFT_NULL && fseek(h, (long) next, SEEK_SET) != 0) return 0;
    long start = ftell(h);
    if (start < 0) return 0;
    if (v.len && (fwrite(v.p, 1, v.len, h) != v.len || fflush(h) != 0)) return 0;
    loft_file_set_long(f, 8, start);
    loft_file_set_long(f, 16, start + (int64_t) v.len);
    return 1;
}

/* list_dir (Stores::fs_list_dir): the entries' names in order, or null for a path that is not a
 * readable directory. */
static loft_ref loft_list_dir(loft_str raw) {
    char *dir = loft_resolve(raw);
    if (!dir) return loft_ref_null();
    loft_ref r = loft_ref_null();
#if defined(LOFT_POSIX)
    DIR *d = opendir(dir);
    if (d) {
        char **names = 0;
        size_t n = 0, cap = 0;
        struct dirent *e;
        while ((e = readdir(d)) != 0) {
            if (!strcmp(e->d_name, ".") || !strcmp(e->d_name, "..")) continue;
            if (n == cap) {
                cap = cap ? cap * 2 : 16;
                names = (char **) realloc(names, cap * sizeof *names);
                if (!names) { fputs("loft: out of memory\n", stderr); abort(); }
            }
            size_t len = strlen(e->d_name);
            names[n] = (char *) malloc(len + 1);
            if (!names[n]) { fputs("loft: out of memory\n", stderr); abort(); }
            memcpy(names[n++], e->d_name, len + 1);
        }
        closedir(d);
        if (n > 1) qsort(names, n, sizeof *names, loft_cmp_cstr);
        loft_database(&r, 8);
        for (size_t i = 0; i < n; i++) {
            loft_ref el = loft_vec_append(r, 4);
            loft_set_text(el, 0, loft_lit(names[i], strlen(names[i])));
            loft_vec_finish(r);
            free(names[i]);
        }
        free(names);
    }
#endif
    free(dir);
    return r;
}

/* mtime: the path's modification time in Unix seconds, 0 when unknown. */
static int64_t loft_mtime(loft_str raw) {
    char *path = loft_resolve(raw);
    int64_t t = 0;
#if defined(LOFT_POSIX)
    struct stat st;
    if (path && stat(path, &st) == 0 && st.st_mtime > 0) t = (int64_t) st.st_mtime;
#endif
    free(path);
    return t;
}

static uint8_t loft_is_dir_at(loft_str raw) {
    char *path = loft_resolve(raw);
    uint8_t r = (uint8_t) (path && loft_is_dir(path));
    free(path);
    return r;
}

static uint8_t loft_is_file_at(loft_str raw) {
    char *path = loft_resolve(raw);
    int64_t size;
    uint8_t r = (uint8_t) (path && loft_stat(path, &size) == 1);
    free(path);
    return r;
}

/* os_temp_dir (std::env::temp_dir): TMPDIR, else /tmp. */
static loft_text loft_os_temp_dir(void) {
    const char *t = getenv("TMPDIR");
    return loft_given(t && *t ? t : "/tmp");
}

/* `dir_with_sub`: `base` in the portable form, `sub` joined to it as `Path::join` does. */
static loft_text loft_dir_with_sub(const char *base, loft_str sub) {
    if (!sub.len || loft_str_is_null(sub) || !*base) return loft_given(base);
    loft_text t = { 0, 0, 0 };
    if (sub.p[0] != '/') {
        loft_push_cstr(&t, base);
        if (t.len && t.p[t.len - 1] != '/') loft_push_char(&t, '/');
    }
    loft_text_append(&t, sub);
    loft_push_char(&t, 0);
    loft_text r = loft_given(t.p);
    loft_text_free(&t);
    return r;
}

/* directory (Stores::os_directory_native): the working directory, `sub` joined to it. */
static loft_text loft_directory(loft_str sub) {
    char buf[4096];
    const char *base = "";
#if defined(LOFT_POSIX)
    if (getcwd(buf, sizeof buf)) base = buf;
#else
    (void) buf;
#endif
    return loft_dir_with_sub(base, sub);
}

/* user_directory (Stores::os_home_native): the home directory, `sub` joined to it. */
static loft_text loft_user_directory(loft_str sub) {
    const char *h = getenv("HOME");
    return loft_dir_with_sub(h ? h : "", sub);
}

/* program_directory (Stores::os_executable_native): the directory holding the running
 * executable — `current_exe`'s answer: `/proc/self/exe` on Linux, `_NSGetExecutablePath` on
 * macOS, else `argv[0]` — `sub` joined to it. */
#if defined(__APPLE__)
extern int _NSGetExecutablePath(char *buf, uint32_t *size);
#endif
static loft_text loft_program_directory(loft_str sub) {
    char buf[4096];
    buf[0] = 0;
#if defined(__linux__) && defined(LOFT_POSIX)
    /* `readlink` is POSIX: a LOFT_NO_POSIX build has no <unistd.h> and answers no path. */
    ssize_t n = readlink("/proc/self/exe", buf, sizeof buf - 1);
    buf[n > 0 ? n : 0] = 0;
#elif defined(__APPLE__)
    uint32_t size = sizeof buf;
    if (_NSGetExecutablePath(buf, &size) != 0) buf[0] = 0;
#endif
#if defined(LOFT_POSIX)
    if (!buf[0] && loft_argv && loft_argv[0]) {
        char real[4096];
        if (realpath(loft_argv[0], real)) strcpy(buf, real);
    }
#endif
    char *slash = strrchr(buf, '/');
    if (slash) *(slash == buf ? slash + 1 : slash) = 0; else buf[0] = 0;
    return loft_dir_with_sub(buf, sub);
}

/* log_info / log_warn / log_error / log_fatal with no log configuration: nothing is written. */
static void loft_log_unconfigured(loft_str message, loft_str file, int64_t line) {
    (void) message;
    (void) file;
    (void) line;
}

/* os_cache_dir (cache::cache_base_dir): $XDG_CACHE_HOME/loft, else $HOME/.cache/loft, else the
 * temporary directory's. */
static loft_text loft_os_cache_dir(void) {
    loft_text t = { 0, 0, 0 };
    const char *x = getenv("XDG_CACHE_HOME"), *h = getenv("HOME"), *tmp = getenv("TMPDIR");
    if (x) {
        loft_push_cstr(&t, x);
    } else if (h) {
        loft_push_cstr(&t, h);
        loft_push_cstr(&t, "/.cache");
    } else {
        loft_push_cstr(&t, tmp && *tmp ? tmp : "/tmp");
    }
    if (t.len && t.p[t.len - 1] != '/') loft_push_char(&t, '/');
    loft_push_cstr(&t, "loft");
    loft_push_char(&t, 0);
    loft_text r = loft_given(t.p);
    loft_text_free(&t);
    return r;
}

/* ymd_days_ago (Stores::ymd_days_ago_native): the UTC date `days` before today, `YYYY-MM-DD`,
 * by logger::days_to_ymd's civil-from-days. */
static loft_text loft_ymd_days_ago(int64_t days) {
    uint64_t now = (uint64_t) time(0);
    uint64_t today = now / 86400, back = days > 0 ? (uint64_t) days : 0;
    int64_t z = (int64_t) (today > back ? today - back : 0) + 719468;
    int64_t era = (z >= 0 ? z : z - 146096) / 146097;
    uint64_t doe = (uint64_t) (z - era * 146097);
    uint64_t yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    int64_t y = (int64_t) yoe + era * 400;
    uint64_t doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    uint64_t mp = (5 * doy + 2) / 153;
    uint64_t d = doy - (153 * mp + 2) / 5 + 1;
    uint64_t m = mp < 10 ? mp + 3 : mp - 9;
    if (m <= 2) y++;
    char b[32];
    int n = snprintf(b, sizeof b, "%04u-%02u-%02u", (unsigned) (uint32_t) y, (unsigned) m, (unsigned) d);
    return loft_text_own(loft_lit(b, (size_t) n));
}

/* env_variables (Stores::os_variables): every environment variable as an EnvVariable (name at 0,
 * value at 4) of `size` bytes, in the environment's order, in a vector of its own store. */
#if defined(LOFT_POSIX)
extern char **environ;
#endif
static loft_ref loft_env_variables(uint32_t size) {
    loft_ref vec = loft_ref_null();
    loft_database(&vec, size * 8 - 8);
    loft_set_u32(loft_store_of(vec), vec.rec, vec.pos, 0);
#if defined(LOFT_POSIX)
    for (char **e = environ; e && *e; e++) {
        const char *eq = strchr(*e, '=');
        if (!eq) continue;
        loft_ref el = loft_vec_append(vec, size);
        loft_set_text(el, 0, loft_lit(*e, (size_t) (eq - *e)));
        loft_set_text(el, 4, loft_lit(eq + 1, strlen(eq + 1)));
        loft_vec_finish(vec);
    }
#endif
    return vec;
}

/* source_dir: the program's own directory. */
static loft_text loft_source_dir(void) {
    return loft_given(loft_anchor);
}

static uint32_t loft_path_sep(void) { return '/'; }

/* env_variable (Stores::os_variable): the variable's value, the null text when it is unset. */
static loft_text loft_env_variable(loft_str name) {
    char *n = (char *) malloc(name.len + 1);
    if (!n) { fputs("loft: out of memory\n", stderr); abort(); }
    memcpy(n, name.p, name.len);
    n[name.len] = 0;
    const char *v = getenv(n);
    free(n);
    return loft_text_own(v ? loft_lit(v, strlen(v)) : loft_null_text());
}
