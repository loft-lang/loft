/* ── Files (src/state/io.rs, src/database/io.rs, src/fs_ops.rs) ────────────────────────────
 * A `File` record: its size at byte 0, the position of the last read or write at 8, the next one
 * at 16 (null: not set), its path at 24, its handle at 28 (an OS descriptor plus one; 0 or
 * INT32_MIN for none: `loft_file_get`) and its format at 32 (1 TextFile, 2 LittleEndian, 3 BigEndian, 4 Directory,
 * 5 NotExists).
 *
 * What a file needs beyond C99 — a path's kind and size, a directory listing, mkdir, rmdir,
 * unlink, truncate, fsync — comes from POSIX where loft_cc.h finds it (`LOFT_POSIX`).  Without
 * it a file is what `fopen` opens, a directory is never seen, a listing is empty and the
 * directory operations answer Other — except on Windows, whose C runtime makes and removes a
 * directory (`_mkdir`, `_rmdir`). */

#if defined(LOFT_POSIX)
#include <sys/types.h>
#include <sys/stat.h>
#include <dirent.h>
#include <unistd.h>
#if defined(LOFT_FOREIGN) && defined(LOFT_POSIX_HOST)
#include <fcntl.h>
#include <sys/mman.h>
#endif
#elif defined(_WIN32)
/* No POSIX, but the Windows C runtime makes and removes a directory and stats a path. */
#include <direct.h>
#include <sys/stat.h>
#endif
#include <errno.h>

#if defined(_WIN32)
#include <wchar.h>
#include <io.h>
/* `@FR-Path-Utf8`: a path goes to Windows in its wide form, `loft_wpath` (loft_rt.h). */

/* A name the OS gives in UTF-16 as loft shows it (malloc'd UTF-8): an unpaired surrogate — a
 * name that is not text — is U+FFFD, as Rust's `to_string_lossy` shows it, and sets `*lossy`. */
static char *loft_wname(const wchar_t *w, int *lossy) {
    size_t n = wcslen(w), o = 0;
    char *s = (char *) malloc(n * 3 + 1);
    if (!s) { fputs("loft: out of memory\n", stderr); abort(); }
    for (size_t i = 0; i < n; i++) {
        uint32_t c = (uint32_t) w[i];
        if (c >= 0xD800 && c < 0xDC00 && i + 1 < n && w[i + 1] >= 0xDC00 && w[i + 1] < 0xE000) {
            c = 0x10000 + ((c - 0xD800) << 10) + ((uint32_t) w[i + 1] - 0xDC00);
            i++;
        } else if (c >= 0xD800 && c < 0xE000) {
            c = 0xFFFD;
            *lossy = 1;
        }
        if (c < 0x80) s[o++] = (char) c;
        else if (c < 0x800) { s[o++] = (char) (0xC0 | (c >> 6)); s[o++] = (char) (0x80 | (c & 0x3F)); }
        else if (c < 0x10000) {
            s[o++] = (char) (0xE0 | (c >> 12));
            s[o++] = (char) (0x80 | ((c >> 6) & 0x3F));
            s[o++] = (char) (0x80 | (c & 0x3F));
        } else {
            s[o++] = (char) (0xF0 | (c >> 18));
            s[o++] = (char) (0x80 | ((c >> 12) & 0x3F));
            s[o++] = (char) (0x80 | ((c >> 6) & 0x3F));
            s[o++] = (char) (0x80 | (c & 0x3F));
        }
    }
    s[o] = 0;
    return s;
}

static FILE *loft_fopen(const char *path, const char *mode) {
    wchar_t *wp = loft_wpath(path), wm[8];
    size_t i = 0;
    for (; mode[i] && i < 7; i++) wm[i] = (wchar_t) (unsigned char) mode[i];
    wm[i] = 0;
    FILE *h = _wfopen(wp, wm);
    int e = errno;
    free(wp);
    errno = e;
    return h;
}

static int loft_wstat64(const char *path, struct _stat64 *st) {
    wchar_t *wp = loft_wpath(path);
    int r = _wstat64(wp, st), e = errno;
    free(wp);
    errno = e;
    return r;
}

/* One call of `fn` (`_wmkdir`, `_wrmdir`, `_wremove`) on the path's wide form; `errno` is the
 * call's. */
static int loft_wcall(const char *path, int (*fn)(const wchar_t *)) {
    wchar_t *wp = loft_wpath(path);
    int r = fn(wp), e = errno;
    free(wp);
    errno = e;
    return r;
}

/* The entries a `_wfindfirst64` pattern matches, each name as loft shows it (`loft_wname`). */
typedef struct {
    intptr_t h;
    struct _wfinddata64_t fd;
} loft_wfind;

/* The first entry: its name (malloc'd) or null when the pattern matches nothing. */
static char *loft_wfind_first(loft_wfind *f, const char *pattern, int *lossy) {
    wchar_t *wp = loft_wpath(pattern);
    f->h = _wfindfirst64(wp, &f->fd);
    free(wp);
    return f->h == -1 ? 0 : loft_wname(f->fd.name, lossy);
}

/* The next entry's name (malloc'd), or null at the end, which closes the search. */
static char *loft_wfind_next(loft_wfind *f, int *lossy) {
    if (f->h == -1) return 0;
    if (_wfindnext64(f->h, &f->fd) != 0) {
        _findclose(f->h);
        f->h = -1;
        return 0;
    }
    return loft_wname(f->fd.name, lossy);
}

static void loft_wfind_close(loft_wfind *f) {
    if (f->h != -1) _findclose(f->h);
    f->h = -1;
}
#else
#define loft_fopen(path, mode) fopen(path, mode)
#endif

#define LOFT_FS_OK 0
#define LOFT_FS_NOT_FOUND 1
#define LOFT_FS_PERMISSION_DENIED 2
#define LOFT_FS_IS_DIRECTORY 3
#define LOFT_FS_NOT_EMPTY 4
#define LOFT_FS_OTHER 9

/* The program's own directory, which a relative path resolves against (Stores::resolve_path):
 * `LOFT_SOURCE_DIR`, which the driver hands every run, else the executable's own directory — a
 * program written out with `--out` or run on its own anchors where it lies, as the Rust
 * writer's standalone binary does.  No unit bakes a directory of the machine that built it.
 * `#cwd` turns the anchoring off. */
static const char *loft_anchor = "";
static int loft_program_relative = 1;

static loft_text loft_program_directory(loft_cx *cx, loft_str sub);
static void loft_set_anchor(loft_cx *cx, int program_relative) {
    const char *env = getenv("LOFT_SOURCE_DIR");
    if (env && *env) loft_anchor = env;
    else {
        loft_text dir = loft_program_directory(cx, loft_lit(cx, "", 0));
        if (dir.len > 1 && (dir.p[dir.len - 1] == '/' || dir.p[dir.len - 1] == '\\')) dir.len--;
        /* The anchor is read as a C string (`chdir`, `strlen`): a text carries no terminator. */
        loft_push(cx, &dir, "", 1);
        loft_anchor = dir.p;
    }
    loft_program_relative = program_relative;
    env = getenv("LOFT_PATHS");
    if (env) loft_program_relative = (env[0] == 'p' || env[0] == 'P');
    /* The interpreter runs a program-relative program in its own directory (main.rs, before
     * the run), so the working directory `directory()` answers is that one — and a C library's
     * own relative `fopen` finds what lies beside the program, on every platform. */
#if defined(LOFT_POSIX)
    if (loft_program_relative && *loft_anchor && chdir(loft_anchor) != 0) {}
#elif defined(_WIN32)
    if (loft_program_relative && *loft_anchor && _chdir(loft_anchor) != 0) {}
#endif
}

/* Rust's `{c:?}` of a character: quoted, the escapes Rust writes, `\u{…}` for a control. */
static void loft_char_debug(loft_cx *cx, loft_text *t, uint32_t c) {
    char b[16];
    loft_push_char(cx, t, '\'');
    switch (c) {
    case 0: loft_push_cstr(cx, t, "\\0"); break;
    case '\t': loft_push_cstr(cx, t, "\\t"); break;
    case '\r': loft_push_cstr(cx, t, "\\r"); break;
    case '\n': loft_push_cstr(cx, t, "\\n"); break;
    case '\'': loft_push_cstr(cx, t, "\\'"); break;
    case '\\': loft_push_cstr(cx, t, "\\\\"); break;
    default:
        if (c < 0x20 || (c >= 0x7F && c <= 0x9F)) {
            snprintf(b, sizeof b, "\\u{%x}", (unsigned) c);
            loft_push_cstr(cx, t, b);
        } else {
            size_t n = loft_utf8(cx, c, b);
            loft_push(cx, t, b, n);
        }
    }
    loft_push_char(cx, t, '\'');
}

/* file_access::name_refusal: why no platform takes the name, or 0 (the reason malloc'd, its
 * length in `*len` — the name it quotes may hold a NUL). */
static char *loft_name_refusal(loft_cx *cx, const char *p, size_t n, size_t *len) {
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
        loft_push_cstr(cx, &t, "the name `");
        loft_push(cx, &t, p, n);
        loft_push_cstr(cx, &t, "` holds ");
        loft_char_debug(cx, &t, cp);
        loft_push_cstr(cx, &t, ", which a file name cannot hold on every platform");
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
        loft_push_cstr(cx, &t, "the name `");
        loft_push(cx, &t, p, n);
        loft_push_cstr(cx, &t, why);
    }
    *len = t.len;
    loft_push_char(cx, &t, 0);
    return t.p;
}

/* `log_refusal_once`: one line per refused path per process, written byte for byte — a path
 * and the reason quoting it may hold a NUL, which the interpreter writes as it is. */
static void loft_log_refusal_once(loft_cx *cx, loft_str raw, const char *why, size_t why_len) {
    static loft_text *said = 0;
    static size_t nsaid = 0;
    /* `@FR-C-Publish`: the paths said are the program's, under the lock. */
    loft_pub_lock();
    for (size_t i = 0; i < nsaid; i++)
        if (said[i].len == raw.len && memcmp(said[i].p, raw.p, raw.len) == 0) {
            loft_pub_unlock();
            return;
        }
    said = (loft_text *) realloc(said, (nsaid + 1) * sizeof *said);
    if (!said) { fputs("loft: out of memory\n", stderr); abort(); }
    said[nsaid++] = loft_text_own(cx, raw);
    loft_pub_unlock();
    fputs("loft: the path `", stderr);
    fwrite(raw.p, 1, raw.len, stderr);
    fputs("` is refused: ", stderr);
    fwrite(why, 1, why_len, stderr);
    fputc('\n', stderr);
}

#if defined(_WIN32)
#define LOFT_PATH_SEP(c) ((c) == '/' || (c) == '\\')
/* PathText::parse_with's Windows prefix: a drive (`C:`, its letter upper-cased) or a UNC share
 * (`\\srv\share`, written `//srv/share`), the `\\?\` verbatim spellings read as their plain
 * twins.  The prefix goes to `pfx` (never longer than what it consumed of `raw`); the answer is
 * where the names start, and `*rooted` says a separator follows the prefix. */
static size_t loft_win_prefix(loft_cx *cx, loft_str raw, char *pfx, size_t *plen, int *rooted) {
    const char *p = raw.p;
    size_t n = raw.len, i = 0;
    int unc = 0;
    *plen = 0;
    if (n >= 4 && LOFT_PATH_SEP(p[0]) && LOFT_PATH_SEP(p[1]) && p[2] == '?' && LOFT_PATH_SEP(p[3])) {
        i = 4;
        if (n - i >= 4 && memcmp(p + i, "UNC", 3) == 0 && LOFT_PATH_SEP(p[i + 3])) {
            i += 4;
            unc = 1;
        }
    } else if (n >= 2 && LOFT_PATH_SEP(p[0]) && LOFT_PATH_SEP(p[1])) {
        i = 2;
        unc = 1;
    }
    if (unc) {
        /* The server and the share are the prefix; the path is rooted under them. */
        pfx[(*plen)++] = '/';
        pfx[(*plen)++] = '/';
        for (int part = 0; part < 2; part++) {
            if (part) {
                pfx[(*plen)++] = '/';
                if (i < n) i++;
            }
            size_t s = i;
            while (i < n && !LOFT_PATH_SEP(p[i])) i++;
            memcpy(pfx + *plen, p + s, i - s);
            *plen += i - s;
        }
        *rooted = 1;
        return i;
    }
    if (n - i >= 2 && ((p[i] >= 'a' && p[i] <= 'z') || (p[i] >= 'A' && p[i] <= 'Z')) && p[i + 1] == ':') {
        pfx[0] = (char) (p[i] >= 'a' ? p[i] - 32 : p[i]);
        pfx[1] = ':';
        *plen = 2;
        i += 2;
    }
    *rooted = i < n && LOFT_PATH_SEP(p[i]);
    return i;
}
#endif

/* PathText's normal form of `raw`, malloc'd: '.' dropped, '..' folded, '/' between names — under
 * `base` when `raw` is relative and `base` is not empty.  With `check`, a name no platform
 * takes refuses the path (said on stderr, as `log_refusal_once` says it) and answers 0.  On
 * Windows a drive or UNC prefix leads the result, as `PathText` keeps it; with `check`, a
 * drive without a root (`C:x`) is refused, as `PathText::program` refuses it. */
static char *loft_normalize(loft_cx *cx, loft_str raw, const char *base, int check) {
    size_t blen = strlen(base);
    char *out = (char *) malloc(blen + raw.len + 3);
    if (!out) { fputs("loft: out of memory\n", stderr); abort(); }
    if (raw.len == 0) {
        strcpy(out, base);
        return out;
    }
    /* The names, each in `parts` from the offset in `at`; the Windows prefix in `pfx`. */
    char *parts = (char *) malloc(raw.len + 1);
    size_t *at = (size_t *) malloc((raw.len + 2) * sizeof *at);
    char *pfx = (char *) malloc(raw.len + 1);
    if (!parts || !at || !pfx) { fputs("loft: out of memory\n", stderr); abort(); }
    size_t plen = 0, i = 0;
    int rooted;
#if defined(_WIN32)
    i = loft_win_prefix(cx, raw, pfx, &plen, &rooted);
    if (check && plen && pfx[0] != '/' && !rooted) {
        char why[96];
        int wl = snprintf(why, sizeof why,
                          "it is relative to drive %c:'s current directory, which no other platform has",
                          pfx[0]);
        loft_log_refusal_once(cx, raw, why, (size_t) wl);
        free(parts);
        free(at);
        free(pfx);
        free(out);
        return 0;
    }
#else
    rooted = raw.p[0] == '/' || raw.p[0] == '\\';
#endif
    size_t np = 0, used = 0;
    at[0] = 0;
    while (i <= raw.len) {
        size_t s = i;
        while (i < raw.len && raw.p[i] != '/' && raw.p[i] != '\\') i++;
        const char *name = raw.p + s;
        size_t n = i - s;
        i++;
        if (n == 0 || (n == 1 && name[0] == '.')) continue;
        size_t why_len = 0;
        char *why = check ? loft_name_refusal(cx, name, n, &why_len) : 0;
        if (why) {
            loft_log_refusal_once(cx, raw, why, why_len);
            free(why);
            free(parts);
            free(at);
            free(pfx);
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
    if (plen) {
        memcpy(out, pfx, plen);
        o = plen;
        if (rooted) out[o++] = '/';
    } else if (blen && !rooted) {
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
    free(pfx);
    return out;
}

/* Stores::resolve_path: the path the program's `raw` names — a relative one under the
 * program's directory unless the program said `#cwd` — or 0 for a refused name. */
#if defined(LOFT_POSIX) || defined(_WIN32)
static int loft_path_exists(loft_cx *cx, const char *path) {
#if defined(_WIN32)
    struct _stat64 st;
    return loft_wstat64(path, &st) == 0;
#else
    struct stat st;
    return stat(path, &st) == 0;
#endif
}

#ifdef LOFT_PATH_FOLD
/* The full lowercase mapping, emitted with the unit's Unicode tables (`crate::c::unicode`). */
static loft_text loft_text_lower(loft_cx *cx, loft_str s);
#endif

/* Case-insensitive equality of two names, as `file_access::case_clash` compares them
 * (`str::to_lowercase` on both): the full mapping where the unit carries it, else ASCII. */
static int loft_name_fold_eq(loft_cx *cx, const char *a, size_t an, const char *b, size_t bn) {
#ifdef LOFT_PATH_FOLD
    loft_str as = { a, an }, bs = { b, bn };
    loft_text la = loft_text_lower(cx, as), lb = loft_text_lower(cx, bs);
    int eq = la.len == lb.len && memcmp(la.p, lb.p, la.len) == 0;
    loft_text_free(cx, &la);
    loft_text_free(cx, &lb);
    return eq;
#endif
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
 * spelling where the file system folds case (macOS, Windows); the first missing name below it
 * with the entries of its directory. */
static char *loft_case_clash(loft_cx *cx, const char *full) {
    size_t len = strlen(full);
    char *existing = (char *) malloc(len + 2);
    if (!existing) { fputs("loft: out of memory\n", stderr); abort(); }
    memcpy(existing, full, len + 1);
    char *missing = 0;
    while (!loft_path_exists(cx, existing)) {
        char *slash = strrchr(existing, '/');
        /* The walk ends at the root: where even `/` cannot be stat'ed (WASI reaches only its
         * pre-opened directories), no prefix exists to compare spellings against. */
        if (!slash || (slash == existing && existing[1] == 0)) { free(existing); free(missing); return 0; }
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
            if ((an != rn || memcmp(as, rs, an) != 0) && loft_name_fold_eq(cx, as, an, rs, rn)) {
                loft_push_char(cx, &why, '`');
                loft_push(cx, &why, as, an);
                loft_push_cstr(cx, &why, "` is spelled `");
                loft_push(cx, &why, rs, rn);
                loft_push_cstr(cx, &why, "` on disk");
                break;
            }
            ae = as > existing ? as - 1 : as;
            re = rs > real ? rs - 1 : rs;
            if (as == existing || rs == real) break;
        }
    }
#elif defined(_WIN32)
    /* Each trailing name of the existing prefix against its on-disk spelling, which the CRT's
     * `_findfirst` of the name itself answers; the walk stops at the drive. */
    {
        char *prefix = (char *) malloc(strlen(existing) + 1);
        if (!prefix) { fputs("loft: out of memory\n", stderr); abort(); }
        strcpy(prefix, existing);
        for (;;) {
            char *slash = strrchr(prefix, '/');
            const char *name = slash ? slash + 1 : prefix;
            size_t nn = strlen(name);
            if (!nn || (nn == 2 && name[1] == ':')) break;
            loft_wfind fd;
            int lossy = 0;
            char *on_disk = loft_wfind_first(&fd, prefix, &lossy);
            loft_wfind_close(&fd);
            if (on_disk) {
                size_t fn = strlen(on_disk);
                int clash = (fn != nn || memcmp(on_disk, name, nn) != 0) && loft_name_fold_eq(cx, name, nn, on_disk, fn);
                if (clash) {
                    loft_push_char(cx, &why, '`');
                    loft_push(cx, &why, name, nn);
                    loft_push_cstr(cx, &why, "` is spelled `");
                    loft_push(cx, &why, on_disk, fn);
                    loft_push_cstr(cx, &why, "` on disk");
                }
                free(on_disk);
                if (clash) break;
            }
            if (!slash || slash == prefix) break;
            *slash = 0;
        }
        free(prefix);
    }
#endif
    if (!why.len && missing) {
#if defined(_WIN32)
        size_t xl = strlen(existing);
        char *pattern = (char *) malloc(xl + 3);
        if (!pattern) { fputs("loft: out of memory\n", stderr); abort(); }
        memcpy(pattern, existing, xl);
        memcpy(pattern + xl, xl && existing[xl - 1] == '/' ? "*" : "/*", xl && existing[xl - 1] == '/' ? 2 : 3);
        loft_wfind fd;
        int lossy = 0;
        char *en_name = loft_wfind_first(&fd, pattern, &lossy);
        free(pattern);
        if (en_name) {
            char *best = 0;
            int unspellable = 0;
            size_t mn = strlen(missing);
            for (; en_name; free(en_name), lossy = 0, en_name = loft_wfind_next(&fd, &lossy)) {
                size_t en = strlen(en_name);
                if (!strcmp(en_name, ".") || !strcmp(en_name, "..")) continue;
                /* `@FR-Path-Utf8` — as the POSIX arm: the name does not exist as spelled, so an
                 * entry that SHOWS as it is one whose real name is not text. */
                if (lossy && en == mn && memcmp(en_name, missing, mn) == 0) unspellable = 1;
                if ((en != mn || memcmp(en_name, missing, mn) != 0) && loft_name_fold_eq(cx, en_name, en, missing, mn)
                    && (!best || strcmp(en_name, best) < 0)) {
                    free(best);
                    best = (char *) malloc(en + 1);
                    if (!best) { fputs("loft: out of memory\n", stderr); abort(); }
                    strcpy(best, en_name);
                }
            }
            loft_wfind_close(&fd);
            if (unspellable) {
                loft_push_char(cx, &why, '`');
                loft_push_cstr(cx, &why, missing);
                loft_push_cstr(cx, &why, "` is how loft shows a name that is not valid text, and names no file");
                free(best);
                best = 0;
            }
#else
        DIR *d = opendir(existing);
        if (d) {
            char *best = 0;
            int unspellable = 0;
            size_t mn = strlen(missing);
            struct dirent *e;
            while ((e = readdir(d)) != 0) {
                size_t en = strlen(e->d_name);
                if (!strcmp(e->d_name, ".") || !strcmp(e->d_name, "..")) continue;
                /* `@FR-Path-Utf8` — the name does not exist as spelled, so an entry that SHOWS as
                 * it is one whose real name is not text: the spelling reaches no file. */
                if (!unspellable && !loft_utf8_valid(cx, (const unsigned char *) e->d_name, en)) {
                    loft_text shown = { 0, 0, 0 };
                    loft_push_lossy(cx, &shown, e->d_name, en);
                    unspellable = shown.len == mn && memcmp(shown.p, missing, mn) == 0;
                    loft_text_free(cx, &shown);
                }
                if ((en != mn || memcmp(e->d_name, missing, mn) != 0) && loft_name_fold_eq(cx, e->d_name, en, missing, mn)
                    && (!best || strcmp(e->d_name, best) < 0)) {
                    free(best);
                    best = (char *) malloc(en + 1);
                    if (!best) { fputs("loft: out of memory\n", stderr); abort(); }
                    strcpy(best, e->d_name);
                }
            }
            closedir(d);
            if (unspellable) {
                loft_push_char(cx, &why, '`');
                loft_push_cstr(cx, &why, missing);
                loft_push_cstr(cx, &why, "` is how loft shows a name that is not valid text, and names no file");
                free(best);
                best = 0;
            }
#endif
            if (best) {
                loft_push_char(cx, &why, '`');
                loft_push_cstr(cx, &why, missing);
                loft_push_cstr(cx, &why, "` would be a second spelling of `");
                loft_push_cstr(cx, &why, best);
                loft_push_char(cx, &why, '`');
                free(best);
            }
        }
    }
    free(existing);
    free(missing);
    if (!why.len) return 0;
    loft_push_char(cx, &why, 0);
    return why.p;
}
#endif

static char *loft_resolve(loft_cx *cx, loft_str raw) {
    char *full = loft_normalize(cx, raw, loft_program_relative ? loft_anchor : "", 1);
#if defined(LOFT_POSIX) || defined(_WIN32)
    if (full && raw.len) {
        char *why = loft_case_clash(cx, full);
        if (why) {
            loft_log_refusal_once(cx, raw, why, strlen(why));
            free(why);
            free(full);
            return 0;
        }
    }
#endif
    return full;
}

/* file_access::given: a directory the OS hands the program, in the portable form. */
static loft_text loft_given(loft_cx *cx, const char *path) {
    char *p = loft_normalize(cx, loft_lit(cx, path, strlen(path)), "", 0);
    loft_text t = loft_text_own(cx, loft_lit(cx, p, strlen(p)));
    free(p);
    return t;
}

static char *loft_file_path(loft_cx *cx, loft_ref f) {
    return loft_resolve(cx, loft_get_text(cx, f, 24));
}

/* A File record's handle (byte 28) is the OS descriptor it reads and writes through, kept PLUS
 * ONE, so a record's zero bytes read as no descriptor: a `File` field of a record just allocated
 * and not yet written (`Holder { h: f }` releases the field's old value before the copy lands)
 * names none, where descriptor 0 closed the program's stdin.  INT32_MIN, the field's null, is
 * none as well.  It is none while no descriptor is open: opened on the record's first read or write, read and written at the
 * record's own position (`#next`, byte 16, which the record always holds) and closed when the
 * record goes.  No record shares it — a copy of a record opens its own (`loft_file_lease`) —
 * and nothing else holds it, so a `par` worker or a `parallel` arm opens and reads files as
 * the main thread does: there is no table for two threads to grow (loft#2568).  It is opened
 * read-write where it can be, so a read and a later write go through one descriptor.  On
 * Windows it is opened shared for deletion, so a `delete` of an open file goes through as on
 * the other hosts.  A build with no descriptors (`-DLOFT_NO_POSIX`: ISO C alone) keeps no
 * handle: each read or write opens the path and closes it again. */
#if defined(LOFT_POSIX) || defined(_WIN32)
#define LOFT_FD 1
#endif

#if defined(LOFT_FD) && defined(_WIN32)
#include <io.h>
#include <fcntl.h>
struct _SECURITY_ATTRIBUTES;
__declspec(dllimport) void *__stdcall CreateFileW(const wchar_t *name, unsigned long access, unsigned long share,
                                                  struct _SECURITY_ATTRIBUTES *security, unsigned long disposition,
                                                  unsigned long flags, void *templ);
__declspec(dllimport) int __stdcall CloseHandle(void *handle);
__declspec(dllimport) unsigned long __stdcall GetLastError(void);
#elif defined(LOFT_FD)
#include <fcntl.h>
#endif
#if defined(LOFT_FD) && !defined(O_CLOEXEC)
#define O_CLOEXEC 0
#endif

/* `file open error: {e}` (state/io.rs): the error as Rust's `io::Error` shows
 * a path's — `<path>: <strerror> (os error <n>)` — on stderr. */
static void loft_file_open_error(loft_cx *cx, const char *path) {
    int e = errno;
    if (path) fprintf(stderr, "file open error: %s: %s (os error %d)\n", path, strerror(e), e);
    else fprintf(stderr, "file open error: %s (os error %d)\n", strerror(e), e);
}

/* How a read or write opens the file: an existing one, one made when absent, or one made empty. */
#define LOFT_OPEN_EXISTING 0
#define LOFT_OPEN_CREATE 1
#define LOFT_OPEN_EMPTY 2

#if defined(LOFT_FD)
typedef int32_t loft_fh;

/* `path` opened as `how` says — read-write, or read-only when an existing file allows no more —
 * or -1 with errno set. */
static loft_fh loft_fh_open(loft_cx *cx, const char *path, int how) {
#if defined(_WIN32)
    /* GENERIC_READ | GENERIC_WRITE, shared for reading, writing and deletion. */
    unsigned long disp = how == LOFT_OPEN_EMPTY ? 2ul : how == LOFT_OPEN_CREATE ? 4ul : 3ul;
    int ro = 0;
    wchar_t *wp = loft_wpath(path);
    void *h = CreateFileW(wp, 0xC0000000ul, 7ul, 0, disp, 0x80ul, 0);
    if (h == (void *) (intptr_t) -1 && how == LOFT_OPEN_EXISTING) {
        h = CreateFileW(wp, 0x80000000ul, 7ul, 0, disp, 0x80ul, 0);
        ro = 1;
    }
    unsigned long e = h == (void *) (intptr_t) -1 ? GetLastError() : 0;
    free(wp);
    if (h == (void *) (intptr_t) -1) {
        errno = e == 2 || e == 3 ? ENOENT : e == 5 ? EACCES : EIO;
        return -1;
    }
    int fd = _open_osfhandle((intptr_t) h, _O_BINARY | (ro ? _O_RDONLY : 0));
    if (fd < 0) CloseHandle(h);
    return fd;
#else
    int flags = O_RDWR | O_CLOEXEC | (how != LOFT_OPEN_EXISTING ? O_CREAT : 0) | (how == LOFT_OPEN_EMPTY ? O_TRUNC : 0);
    int fd = open(path, flags, 0666);
    if (fd < 0 && how == LOFT_OPEN_EXISTING && errno != ENOENT) fd = open(path, O_RDONLY | O_CLOEXEC);
    return fd;
#endif
}

/* Up to `n` bytes at `at` into `buf`: how many came, fewer only at the end or on an error. */
LOFT_NOIPA static size_t loft_fh_read(loft_cx *cx, loft_fh fd, void *buf, size_t n, int64_t at) {
    size_t got = 0;
    while (got < n) {
#if defined(_WIN32)
        size_t want = n - got < ((size_t) 1 << 30) ? n - got : (size_t) 1 << 30;
        int r = _lseeki64(fd, at + (int64_t) got, SEEK_SET) < 0 ? -1 : _read(fd, (char *) buf + got, (unsigned) want);
#elif defined(ESP_PLATFORM) || defined(__wasi__)
        int r = lseek(fd, (off_t) (at + (int64_t) got), SEEK_SET) < 0 ? -1 : (int) read(fd, (char *) buf + got, n - got);
#else
        ssize_t r = pread(fd, (char *) buf + got, n - got, (off_t) (at + (int64_t) got));
#endif
        if (r < 0 && errno == EINTR) continue;
        if (r <= 0) break;
        got += (size_t) r;
    }
    return got;
}

/* The `n` bytes of `buf` at `at`: 1 when all were written. */
LOFT_NOIPA static int loft_fh_write(loft_cx *cx, loft_fh fd, const void *buf, size_t n, int64_t at) {
    size_t done = 0;
    while (done < n) {
#if defined(_WIN32)
        size_t want = n - done < ((size_t) 1 << 30) ? n - done : (size_t) 1 << 30;
        int r = _lseeki64(fd, at + (int64_t) done, SEEK_SET) < 0 ? -1 : _write(fd, (const char *) buf + done, (unsigned) want);
#elif defined(ESP_PLATFORM) || defined(__wasi__)
        int r = lseek(fd, (off_t) (at + (int64_t) done), SEEK_SET) < 0 ? -1 : (int) write(fd, (const char *) buf + done, n - done);
#else
        ssize_t r = pwrite(fd, (const char *) buf + done, n - done, (off_t) (at + (int64_t) done));
#endif
        if (r < 0 && errno == EINTR) continue;
        if (r <= 0) return 0;
        done += (size_t) r;
    }
    return 1;
}

static int loft_fh_sync(loft_cx *cx, loft_fh fd) {
#if defined(_WIN32)
    return _commit(fd) == 0;
#else
    return fsync(fd) == 0;
#endif
}

static void loft_fh_close(loft_cx *cx, loft_fh fd) {
#if defined(_WIN32)
    _close(fd);
#else
    close(fd);
#endif
}

static int32_t loft_file_fd(loft_cx *cx, loft_ref f) {
    int32_t h;
    memcpy(&h, loft_at(cx, loft_store_of(cx, f), f.rec, f.pos + 28), 4);
    return h > 0 ? h - 1 : INT32_MIN;
}
#else
typedef FILE *loft_fh;

static loft_fh loft_fh_open(loft_cx *cx, const char *path, int how) {
    FILE *h = how == LOFT_OPEN_EMPTY ? loft_fopen(path, "w+b") : loft_fopen(path, "r+b");
    if (!h && how == LOFT_OPEN_CREATE) h = loft_fopen(path, "w+b");
    if (!h && how == LOFT_OPEN_EXISTING) h = loft_fopen(path, "rb");
    return h;
}

static size_t loft_fh_read(loft_cx *cx, loft_fh h, void *buf, size_t n, int64_t at) {
    return fseek(h, (long) at, SEEK_SET) != 0 ? 0 : fread(buf, 1, n, h);
}

static int loft_fh_write(loft_cx *cx, loft_fh h, const void *buf, size_t n, int64_t at) {
    return fseek(h, (long) at, SEEK_SET) == 0 && fwrite(buf, 1, n, h) == n && fflush(h) == 0;
}

/* No handle is kept: none is open. */
static int32_t loft_file_fd(loft_cx *cx, loft_ref f) {
    (void) f;
    return INT32_MIN;
}
#endif

static char *loft_file_path(loft_cx *cx, loft_ref f);

/* `f`'s handle for a read or a write: its open descriptor, else its path opened as `how` says —
 * kept in the record where a build has descriptors.  0 when it does not open, with `*path` the
 * path it tried (null when the path is refused) for the caller's message, errno saying why;
 * the caller frees `*path` and gives the handle back (`loft_file_put`). */
static int loft_file_get(loft_cx *cx, loft_ref f, int how, loft_fh *h, char **path) {
    *path = 0;
#if defined(LOFT_FD)
    int32_t fd = loft_file_fd(cx, f);
    if (fd >= 0) {
        *h = fd;
        return 1;
    }
#endif
    *path = loft_file_path(cx, f);
    if (!*path) return 0;
    *h = loft_fh_open(cx, *path, how);
#if defined(LOFT_FD)
    if (*h < 0) return 0;
    int32_t kept = *h + 1;
    memcpy(loft_at(cx, loft_store_of(cx, f), f.rec, f.pos + 28), &kept, 4);
#else
    if (!*h) return 0;
#endif
    return 1;
}

/* The end of one read or write: a build without descriptors closes what it opened for it. */
static void loft_file_put(loft_cx *cx, loft_fh h) {
#if defined(LOFT_FD)
    (void) h;
#else
    fclose(h);
#endif
}

/* `f` lets go of its descriptor: closed, the record detached (its handle reads none), so a
 * second walk over the same bytes closes nothing twice. */
static void loft_file_close(loft_cx *cx, loft_ref f) {
    int32_t h = loft_file_fd(cx, f), none = INT32_MIN;
    if (h == INT32_MIN) return;
    memcpy(loft_at(cx, loft_store_of(cx, f), f.rec, f.pos + 28), &none, 4);
#if defined(LOFT_FD)
    if (h >= 0) loft_fh_close(cx, h);
#endif
}

/* `Stores::lease_if_file`: a copy of a `File` record — its bytes the original's, descriptor
 * included — opens its own on first use, at the position it copied. */
static void loft_file_lease(loft_cx *cx, loft_ref f) {
    int32_t none = INT32_MIN;
    memcpy(loft_at(cx, loft_store_of(cx, f), f.rec, f.pos + 28), &none, 4);
}

/* `Stores::release_file_handle`: a `File` record is gone — its descriptor closed. */
static void loft_file_release(loft_cx *cx, loft_ref f) {
    loft_file_close(cx, f);
}

/* `Stores::release_files_in`: every `File` reached from `r` (of type `tp`) through what it owns
 * closes its descriptor.  Nothing is deleted: the store is about to be freed whole.  A view
 * member is skipped, as the record that owns it releases it; a type that cannot hold a `File`
 * is not entered. */
static void loft_release_files_in(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref r) {
    if (r.store_nr == 0xFFFF || r.rec == 0) return;
    const loft_tdesc *d = &T[tp];
    if (!d->file) return;
    if (d->file == 2) { loft_file_release(cx, r); return; }
    loft_store_t *s = loft_store_of(cx, r);
    switch (d->kind) {
    case LOFT_T_STRUCT:
        for (uint16_t i = 0; i < d->n; i++)
            if (!d->fields[i].view) loft_release_files_in(cx, T, d->fields[i].type, loft_field(cx, r, d->fields[i].pos));
        return;
    case LOFT_T_ENUM: {
        int e = (int) *loft_at(cx, s, r.rec, r.pos) - 1;
        if (e >= 0 && e < d->n && d->variants[e] != 0xFFFF) loft_release_files_in(cx, T, d->variants[e], r);
        return;
    }
    default:
        break;
    }
    uint32_t cur = loft_u32(cx, s, r.rec, r.pos);
    if (cur == 0 || cur == 0xFFFFFFFFu) return;
    switch (d->kind) {
    case LOFT_T_VECTOR: {
        uint32_t len = loft_u32(cx, s, cur, 4), size = T[d->elem].size;
        for (uint32_t i = 0; i < len; i++) {
            loft_ref e = { r.store_nr, cur, 8 + i * size, 0 };
            loft_release_files_in(cx, T, d->elem, e);
        }
        return;
    }
    case LOFT_T_CHILD: {
        loft_ref c = { r.store_nr, cur, 8, 0 };
        loft_release_files_in(cx, T, d->elem, c);
        return;
    }
    case LOFT_T_ORDERED:
        for (uint32_t i = 0; i < loft_u32(cx, s, cur, 4); i++) {
            uint32_t elm = loft_u32(cx, s, cur, 8 + 4 * i);
            if (elm == 0) continue;
            loft_ref e = { r.store_nr, elm, 8, 0 };
            loft_release_files_in(cx, T, d->elem, e);
        }
        return;
    case LOFT_T_HASH: {
        uint32_t n;
        loft_hent *all = loft_h_entries_of(cx, r, &n);
        for (uint32_t i = 0; i < n; i++) loft_release_files_in(cx, T, d->elem, all[i].e);
        free(all);
        return;
    }
    case LOFT_T_INDEX:
    case LOFT_T_RADIX:
    case LOFT_T_TRIE: {
        uint32_t n;
        uint32_t *recs = d->kind == LOFT_T_INDEX ? loft_bt_records(cx, r, &n) : loft_rx_records(cx, r, &n);
        for (uint32_t i = 0; i < n; i++) {
            loft_ref e = { r.store_nr, recs[i], 8, 0 };
            loft_release_files_in(cx, T, d->elem, e);
        }
        free(recs);
        return;
    }
    default:
        return;
    }
}

/* `Stores::release_file_leases`: the store `nr` is being freed — every `File` its records hold,
 * reached from its root through the type the root names, closes its descriptor.  A build
 * without descriptors has none to close. */
static void loft_release_file_leases(loft_cx *cx, uint16_t nr) {
#if defined(LOFT_FD)
    if (!loft_T) return;
    loft_store_t *s = &cx->stores[nr];
    uint16_t tp = LOFT_KNOWN(*s);
    if (tp >= loft_T_n || !loft_T[tp].file || s->size < 3 || loft_u32(cx, s, 1, 4) != tp) return;
    loft_ref root = { nr, 1, 8, 0 };
    loft_release_files_in(cx, loft_T, tp, root);
#else
    (void) nr;
#endif
}

static void (*loft_file_lease_hook)(loft_cx *cx, loft_ref f) = loft_file_lease;
static void (*loft_file_release_hook)(loft_cx *cx, loft_ref f) = loft_file_release;
static void (*loft_release_file_leases_hook)(loft_cx *cx, uint16_t nr) = loft_release_file_leases;

static void loft_file_set_long(loft_cx *cx, loft_ref f, uint32_t fld, int64_t v) {
    memcpy(loft_at(cx, loft_store_of(cx, f), f.rec, f.pos + fld), &v, 8);
}

static int64_t loft_file_long(loft_cx *cx, loft_ref f, uint32_t fld) {
    int64_t v;
    memcpy(&v, loft_at(cx, loft_store_of(cx, f), f.rec, f.pos + fld), 8);
    return v;
}

static uint8_t loft_file_format(loft_cx *cx, loft_ref f) {
    return *loft_at(cx, loft_store_of(cx, f), f.rec, f.pos + 32);
}

/* The path's kind (1 file, 4 directory, 5 neither, 0 absent) and its size. */
static int loft_stat(loft_cx *cx, const char *path, int64_t *size) {
#if defined(LOFT_POSIX)
    struct stat st;
    if (stat(path, &st) != 0) return 0;
    *size = (int64_t) st.st_size;
    if (S_ISDIR(st.st_mode)) return 4;
    if (S_ISREG(st.st_mode)) return 1;
    return 5;
#elif defined(_WIN32)
    /* The CRT's own stat: a directory is one, where an `fopen` probe could only fail. */
    struct _stat64 st;
    if (loft_wstat64(path, &st) != 0) return 0;
    *size = (int64_t) st.st_size;
    if (st.st_mode & _S_IFDIR) return 4;
    if (st.st_mode & _S_IFREG) return 1;
    return 5;
#else
    FILE *h = loft_fopen(path, "rb");
    if (!h) return 0;
    long n = fseek(h, 0, SEEK_END) == 0 ? ftell(h) : -1;
    fclose(h);
    *size = n < 0 ? 0 : (int64_t) n;
    return 1;
#endif
}

/* fill_absent / fill_file: the File record describes the path. */
static void loft_fill_absent(loft_cx *cx, loft_ref f) {
    loft_file_set_long(cx, f, 0, 0);
    loft_file_set_long(cx, f, 8, LOFT_NULL);
    loft_file_set_long(cx, f, 16, LOFT_NULL);
    *loft_at(cx, loft_store_of(cx, f), f.rec, f.pos + 32) = 5;
}

static int loft_fill_file(loft_cx *cx, loft_ref f, const char *path) {
    int64_t size = 0;
    int kind = loft_stat(cx, path, &size);
    if (kind == 0) {
        loft_fill_absent(cx, f);
        return 0;
    }
    loft_file_set_long(cx, f, 8, LOFT_NULL);
    loft_file_set_long(cx, f, 16, LOFT_NULL);
    loft_file_set_long(cx, f, 0, kind == 1 ? size : LOFT_NULL);
    *loft_at(cx, loft_store_of(cx, f), f.rec, f.pos + 32) = (uint8_t) kind;
    return 1;
}

/* OpGetFile (Stores::get_file): what the File's path names. */
static uint8_t loft_get_file(loft_cx *cx, loft_ref f) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return 0;
    char *path = loft_file_path(cx, f);
    if (!path) {
        loft_fill_absent(cx, f);
        return 0;
    }
    int r = loft_fill_file(cx, f, path);
    free(path);
    return (uint8_t) r;
}

/* OpGetFileText (read_file_text_into): the whole file as text; nothing for a missing file, and
 * nothing — with a warning — for bytes that are not UTF-8. */
static void loft_get_file_text(loft_cx *cx, loft_ref f, loft_text *out) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return;
    out->len = 0;
    char *path = loft_file_path(cx, f);
    if (!path) return;
    FILE *h = loft_fopen(path, "rb");
    if (!h) { free(path); return; }
    char buf[4096];
    size_t n;
    while ((n = fread(buf, 1, sizeof buf, h)) > 0) loft_push(cx, out, buf, n);
    fclose(h);
    if (!loft_utf8_valid(cx, (const unsigned char *) out->p, out->len)) {
        fprintf(stderr, "warning: file(\"%s\").content() got non-UTF-8 bytes (%lu bytes in file) — returning null. "
                "Read the bytes exactly with `read_bytes(path)`, or a field at a time with `f#format = LittleEndian; "
                "f#read(n)` (or `BigEndian`); see the loft-write skill § File I/O.\n", path, (unsigned long) out->len);
        out->len = 0;
    }
    free(path);
}

/* Is a write or read through this File in little-endian order (its format LittleEndian)? */
static int loft_file_le(loft_cx *cx, loft_ref f) {
    return f.store_nr != 0xFFFF && f.rec != 0 && loft_file_format(cx, f) == 2;
}

/* OpWriteFile (State::write_file): `bytes` at the File's next position — the end of the file
 * when none was set, so `f += x` appends — through a handle opened on first use. */
static void loft_write_file(loft_cx *cx, loft_ref f, const loft_text *bytes) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return;
    uint8_t format = loft_file_format(cx, f);
    if (format != 1 && format != 2 && format != 3 && format != 5) return;
    int64_t next = loft_file_long(cx, f, 16);
    if (next == LOFT_NULL) {
        char *path = loft_file_path(cx, f);
        int64_t size = 0;
        next = path && loft_stat(cx, path, &size) != 0 ? size : 0;
        free(path);
    }
    loft_file_set_long(cx, f, 8, next);
    loft_fh h;
    char *path;
    if (!loft_file_get(cx, f, LOFT_OPEN_CREATE, &h, &path)) {
        if (!path) fputs("file open error: the path is refused\n", stderr);
        else loft_file_open_error(cx, path);
        free(path);
        return;
    }
    if (path && format == 5) *loft_at(cx, loft_store_of(cx, f), f.rec, f.pos + 32) = 1;
    free(path);
    /* Unbuffered, as the interpreter's `write_all` is: another handle on the path reads it.  A
     * file only readable fails here, and the position moves on as though it had been written. */
    if (bytes->len && !loft_fh_write(cx, h, bytes->p, bytes->len, next)) fputs("file write error\n", stderr);
    loft_file_put(cx, h);
    loft_file_set_long(cx, f, 16, next + (int64_t) bytes->len);
}

/* OpReadFile (State::read_file): up to `n` bytes from the File's next position into `*out`,
 * and how many came: a text takes what arrived, a binary value all `n` or none, and a count
 * below the value's width (`width`, 0 for a text) none at all.  -1 when the File cannot be
 * read, which reads nothing into the value.  `*out` is `buf` when the bytes fit its `cap`
 * (a scalar read's stack buffer), else malloc'd: the caller frees it when it is not `buf`. */
static int64_t loft_read_file_in(loft_cx *cx, loft_ref f, int64_t n, int text, int64_t width, uint8_t *buf, size_t cap, uint8_t **out) {
    *out = 0;
    if (f.store_nr == 0xFFFF || f.rec == 0) return -1;
    uint8_t format = loft_file_format(cx, f);
    if (format != 1 && format != 2 && format != 3 && format != 5) return -1;
    int64_t next = loft_file_long(cx, f, 16);
    if (next == LOFT_NULL) next = 0;
    loft_file_set_long(cx, f, 8, next);
    loft_fh h;
    char *path;
    if (!loft_file_get(cx, f, LOFT_OPEN_EXISTING, &h, &path)) {
        loft_file_open_error(cx, path);
        free(path);
        return -1;
    }
    free(path);
    size_t want = n < 0 ? 0 : (size_t) n;
    *out = buf && want + 1 <= cap ? buf : (uint8_t *) malloc(want + 1);
    if (!*out) { fputs("loft: out of memory\n", stderr); abort(); }
    size_t got = 0;
    if (text || n >= width) {
        got = loft_fh_read(cx, h, *out, want, next);
        if (!text && got < want) got = 0;
    }
    loft_file_put(cx, h);
    loft_file_set_long(cx, f, 16, next + (int64_t) got);
    return (int64_t) got;
}

static int64_t loft_read_file(loft_cx *cx, loft_ref f, int64_t n, int text, int64_t width, uint8_t **out) {
    return loft_read_file_in(cx, f, n, text, width, 0, 0, out);
}

/* OpSeekFile (`f#next = pos` writes it, then sets `#next` itself): a record with no handle open
 * takes `pos` as the position its first read or write opens at, as `State::seek_file` does; one
 * with a descriptor open has no stream position to move — every read and write starts at the
 * record's `#next`, which the assignment that follows sets. */
static void loft_seek_file(loft_cx *cx, loft_ref f, int64_t pos) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return;
    if (loft_file_fd(cx, f) < 0) loft_file_set_long(cx, f, 16, pos);
}

/* OpSizeFile: the file's size now, or null. */
static int64_t loft_size_file(loft_cx *cx, loft_ref f) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return LOFT_NULL;
    char *path = loft_file_path(cx, f);
    int64_t size = 0;
    int kind = path ? loft_stat(cx, path, &size) : 0;
    free(path);
    return kind == 0 ? LOFT_NULL : size;
}

/* OpSyncFile: the written bytes reach the device. */
static uint8_t loft_sync_file(loft_cx *cx, loft_ref f) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return 0;
#if defined(LOFT_FD)
    int32_t fd = loft_file_fd(cx, f);
    return (uint8_t) (fd < 0 || loft_fh_sync(cx, fd));
#else
    /* Each write closed what it opened: its bytes are the OS's already. */
    return 1;
#endif
}

/* OpTruncateFile: the file is `size` bytes long; an open handle is closed first. */
static uint8_t loft_truncate_file(loft_cx *cx, loft_ref f, int64_t size) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return 0;
    if (loft_file_fd(cx, f) >= 0) {
        loft_file_close(cx, f);
        loft_file_set_long(cx, f, 8, LOFT_NULL);
        loft_file_set_long(cx, f, 16, LOFT_NULL);
    }
    char *path = loft_file_path(cx, f);
    if (!path) return 0;
    int ok;
#if defined(LOFT_POSIX)
    int64_t old = 0;
    ok = loft_stat(cx, path, &old) == 1 && truncate(path, (off_t) (size < 0 ? 0 : size)) == 0;
#else
    FILE *r = loft_fopen(path, "rb");
    ok = r != 0;
    if (ok) {
        size_t keep = size < 0 ? 0 : (size_t) size;
        char *buf = (char *) calloc(keep + 1, 1);
        if (!buf) { fputs("loft: out of memory\n", stderr); abort(); }
        size_t got = fread(buf, 1, keep, r);
        (void) got;
        fclose(r);
        FILE *w = loft_fopen(path, "wb");
        ok = w && fwrite(buf, 1, keep, w) == keep;
        if (w) fclose(w);
        free(buf);
    }
#endif
    free(path);
    return (uint8_t) ok;
}

/* fs_classify: an errno as a FileResult code. */
static int64_t loft_fs_code(loft_cx *cx, int err) {
    if (err == ENOENT) return LOFT_FS_NOT_FOUND;
#if defined(LOFT_POSIX)
    if (err == EACCES || err == EPERM) return LOFT_FS_PERMISSION_DENIED;
#endif
    return LOFT_FS_OTHER;
}

static int loft_is_dir(loft_cx *cx, const char *path) {
    int64_t size;
    return loft_stat(cx, path, &size) == 4;
}

/* A path's form for telling two spellings of one file apart: a relative path is put under the
 * working directory (under `#cwd`, or wherever the program anchor is empty, a resolved path is
 * still relative), then folded — `../x` and `<cwd>/../x` and `<parent>/x` are one key. */
static char *loft_path_key(loft_cx *cx, const char *path) {
    char buf[4096];
    const char *cwd = "";
#if defined(LOFT_POSIX)
    if (getcwd(buf, sizeof buf)) cwd = buf;
#elif defined(_WIN32)
    if (_getcwd(buf, (int) sizeof buf)) cwd = buf;
#else
    (void) buf;
#endif
    char *joined = loft_normalize(cx, loft_lit(cx, path, strlen(path)), cwd, 0);
    char *key = loft_normalize(cx, loft_lit(cx, joined, strlen(joined)), "", 0);
    free(joined);
    return key;
}

/* OpDelete: the file goes; a directory is refused as one.  A file this program holds open goes
 * too: Windows refuses that unless the file was opened shared for deletion, which every handle
 * here is (`loft_fh_open`). */
static int64_t loft_fs_delete(loft_cx *cx, loft_str raw) {
    char *path = loft_resolve(cx, raw);
    if (!path) return LOFT_FS_OTHER;
    int64_t code = LOFT_FS_OK;
#if defined(LOFT_POSIX)
    if (unlink(path) != 0) code = loft_is_dir(cx, path) ? LOFT_FS_IS_DIRECTORY : loft_fs_code(cx, errno);
#elif defined(_WIN32)
    if (loft_wcall(path, _wremove) != 0) {
        /* Windows' CRT reports a directory as a permission error: asked, as POSIX's arm asks. */
        int e = errno;
        code = loft_is_dir(cx, path) ? LOFT_FS_IS_DIRECTORY : loft_fs_code(cx, e);
    }
#else
    if (remove(path) != 0) {
        int e = errno;
        code = loft_is_dir(cx, path) ? LOFT_FS_IS_DIRECTORY : loft_fs_code(cx, e);
    }
#endif
    free(path);
    return code;
}

/* OpMoveFile: `from` is renamed to `to`, replacing a file there (`loft_rename`). */
static int64_t loft_fs_move(loft_cx *cx, loft_str from, loft_str to) {
    char *a = loft_resolve(cx, from), *b = loft_resolve(cx, to);
    int64_t code = LOFT_FS_OTHER;
    if (a && b) {
        /* A file this program holds open moves too: every handle here is opened shared for
         * deletion, which on Windows lets the file be renamed (`loft_fh_open`). */
        code = loft_rename(cx, a, b) == 0 ? LOFT_FS_OK : loft_fs_code(cx, errno);
    }
    free(a);
    free(b);
    return code;
}

/* OpMkdir / OpMkdirAll: the directory (and with `all` its parents; one already there is fine). */
static int64_t loft_fs_mkdir(loft_cx *cx, loft_str raw, int all) {
    char *path = loft_resolve(cx, raw);
    if (!path) return LOFT_FS_OTHER;
    int64_t code = LOFT_FS_OTHER;
    /* A path that resolves to nothing names no directory; the walks below start one past
     * its first character. */
    if (!path[0]) {
        free(path);
        return loft_fs_code(cx, ENOENT);
    }
#if defined(LOFT_POSIX)
    if (all) {
        for (char *p = path + 1; *p; p++) {
            if (*p != '/') continue;
            *p = 0;
            if (mkdir(path, 0777) != 0 && errno != EEXIST) {
                *p = '/';
                code = loft_fs_code(cx, errno);
                free(path);
                return code;
            }
            *p = '/';
        }
        code = mkdir(path, 0777) == 0 || (errno == EEXIST && loft_is_dir(cx, path)) ? LOFT_FS_OK : loft_fs_code(cx, errno);
    } else {
        code = mkdir(path, 0777) == 0 ? LOFT_FS_OK : loft_fs_code(cx, errno);
    }
#elif defined(_WIN32)
    if (all) {
        /* Past the root a parent cannot be made of: `C:/`, `//srv/share/`, `/`. */
        char *p = path;
        if (p[0] && p[1] == ':') p += 2;
        else if (p[0] == '/' && p[1] == '/') {
            int seps = 0;
            for (p += 2; *p && seps < 2; p++) if (*p == '/') seps++;
            p--;
        }
        for (p++; *p; p++) {
            if (*p != '/') continue;
            *p = 0;
            if (loft_wcall(path, _wmkdir) != 0 && errno != EEXIST) {
                *p = '/';
                code = loft_fs_code(cx, errno);
                free(path);
                return code;
            }
            *p = '/';
        }
        struct _stat64 st;
        code = loft_wcall(path, _wmkdir) == 0 || (errno == EEXIST && loft_wstat64(path, &st) == 0 && (st.st_mode & _S_IFDIR))
            ? LOFT_FS_OK : loft_fs_code(cx, errno);
    } else {
        code = loft_wcall(path, _wmkdir) == 0 ? LOFT_FS_OK : loft_fs_code(cx, errno);
    }
#else
    (void) all;
#endif
    free(path);
    return code;
}

/* OpRmdir: the EMPTY directory goes. */
static int64_t loft_fs_rmdir(loft_cx *cx, loft_str raw) {
    char *path = loft_resolve(cx, raw);
    if (!path) return LOFT_FS_OTHER;
    int64_t code = LOFT_FS_OTHER;
#if defined(LOFT_POSIX) || defined(_WIN32)
#if defined(LOFT_POSIX)
    if (rmdir(path) == 0) code = LOFT_FS_OK;
#else
    if (loft_wcall(path, _wrmdir) == 0) code = LOFT_FS_OK;
#endif
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

/* The names in directory `dir`, without `.` and `..`, in byte order — `*n` of them, each
 * malloc'd — or null when it cannot be listed: POSIX's `readdir`, the Windows CRT's `_findfirst`
 * (`io.h`), nothing elsewhere. */
static void loft_dir_name_push(loft_cx *cx, char ***names, size_t *n, size_t *cap, const char *name) {
    if (!strcmp(name, ".") || !strcmp(name, "..")) return;
    if (*n == *cap) {
        *cap = *cap ? *cap * 2 : 16;
        *names = (char **) realloc(*names, *cap * sizeof **names);
        if (!*names) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    size_t len = strlen(name);
    (*names)[*n] = (char *) malloc(len + 1);
    if (!(*names)[*n]) { fputs("loft: out of memory\n", stderr); abort(); }
    memcpy((*names)[(*n)++], name, len + 1);
}

/* `@FR-Path-Utf8` — the directories a listing has already reported as holding a name that is not
 * text (`file_access::log_once`): FNV-1a hashes of their paths, each slot claimed once and
 * lock-free.  Past the 64 slots a directory is reported again — a repeated line, never a
 * missing one. */
static uint64_t loft_unspellable_said[64];

static int loft_unspellable_first(loft_cx *cx, const char *dir) {
    (void) cx;
    uint64_t h = UINT64_C(1469598103934665603);
    for (const unsigned char *c = (const unsigned char *) dir; *c; c++) h = (h ^ *c) * UINT64_C(1099511628211);
    if (h == 0) h = 1;
    for (size_t i = 0; i < sizeof loft_unspellable_said / sizeof *loft_unspellable_said; i++) {
        uint64_t held;
        LOFT_PUB_GET(loft_unspellable_said[i], held);
        if (held == h) return 0;
        if (held == 0) {
            uint64_t expect = 0;
            if (LOFT_PUB_CAS(loft_unspellable_said[i], expect, h)) return 1;
            if (expect == h) return 0;
        }
    }
    return 1;
}

static char **loft_dir_names(loft_cx *cx, const char *dir, size_t *n) {
    char **names = 0;
    size_t cap = 0;
    *n = 0;
#if defined(LOFT_POSIX)
    DIR *d = opendir(dir);
    if (!d) return 0;
    struct dirent *e;
    int unspellable = 0;
    while ((e = readdir(d)) != 0) {
        unspellable |= !loft_utf8_valid(cx, (const unsigned char *) e->d_name, strlen(e->d_name));
        loft_dir_name_push(cx, &names, n, &cap, e->d_name);
    }
    closedir(d);
    /* `@FR-Path-Utf8` — one line names the directory that holds such a name, as the
     * interpreter's listing says it. */
    if (unspellable && loft_unspellable_first(cx, dir))
        fprintf(stderr, "loft: `%s` holds a name that is not valid text; it is listed with U+FFFD in its place, and no path can name it (formal/paths.md)\n", dir);
#elif defined(_WIN32)
    size_t dl = strlen(dir);
    char *pattern = (char *) malloc(dl + 3);
    if (!pattern) { fputs("loft: out of memory\n", stderr); abort(); }
    memcpy(pattern, dir, dl);
    memcpy(pattern + dl, "/*", 3);
    loft_wfind fd;
    int lossy = 0;
    char *name = loft_wfind_first(&fd, pattern, &lossy);
    free(pattern);
    if (!name) return loft_is_dir(cx, dir) ? (char **) calloc(1, sizeof(char *)) : 0;
    /* `lossy` gathers over the listing: set once any name is not text. */
    for (; name; free(name), name = loft_wfind_next(&fd, &lossy)) loft_dir_name_push(cx, &names, n, &cap, name);
    /* `@FR-Path-Utf8` — one line names the directory that holds such a name, as the POSIX arm. */
    if (lossy && loft_unspellable_first(cx, dir))
        fprintf(stderr, "loft: `%s` holds a name that is not valid text; it is listed with U+FFFD in its place, and no path can name it (formal/paths.md)\n", dir);
#else
    (void) dir;
    (void) cap;
    return 0;
#endif
    if (!names) names = (char **) calloc(1, sizeof(char *));
    if (*n > 1) qsort(names, *n, sizeof *names, loft_cmp_cstr);
    return names;
}

/* OpGetDir (Stores::get_dir): a File record per entry of the directory, in name order, each
 * filled as `file()` fills one; `size` is a File record's.  True whenever the path is not
 * refused, whether or not it could be listed. */
static uint8_t loft_get_dir(loft_cx *cx, loft_str raw, loft_ref v, uint32_t size) {
    char *dir = loft_resolve(cx, raw);
    if (!dir) return 0;
    size_t n = 0;
    char **names = loft_dir_names(cx, dir, &n);
    size_t dl = strlen(dir);
    for (size_t i = 0; i < n; i++) {
        size_t nl = strlen(names[i]);
        char *full = (char *) malloc(dl + nl + 2);
        if (!full) { fputs("loft: out of memory\n", stderr); abort(); }
        memcpy(full, dir, dl);
        full[dl] = '/';
        memcpy(full + dl + 1, names[i], nl + 1);
        loft_ref el = loft_vec_append(cx, v, size);
        if (el.rec != 0) {
            loft_set_text(cx, el, 24, loft_lit(cx, full, strlen(full)));
            int32_t none = INT32_MIN;
            memcpy(loft_at(cx, loft_store_of(cx, el), el.rec, el.pos + 28), &none, 4);
            loft_file_set_long(cx, el, 8, LOFT_NULL);
            loft_file_set_long(cx, el, 16, LOFT_NULL);
            loft_vec_finish(cx, v);
            loft_fill_file(cx, el, full);
        }
        free(full);
        free(names[i]);
    }
    free(names);
    free(dir);
    return 1;
}

/* ── Binary forms (Stores::read_data / write_data) ──
 * A value's file form: an integer, long or float in 8 bytes, a single or character in 4, a
 * boolean or byte in 1, a text its bytes, a record its fields one after another, a vector its
 * elements — in the File's byte order. */

/* Is the host little-endian?  A written value in the host's order is its stored bytes (the
 * serialisers' block path, `files.rs`). */
static inline int loft_host_le(loft_cx *cx) {
    const uint16_t one = 1;
    return *(const unsigned char *) &one == 1;
}
static void loft_put(loft_cx *cx, loft_text *b, uint64_t v, int n, int le) {
    char c[8];
    for (int i = 0; i < n; i++) c[le ? i : n - 1 - i] = (char) (unsigned char) (v >> (8 * i));
    loft_push(cx, b, c, (size_t) n);
}

static uint64_t loft_take(loft_cx *cx, const uint8_t *d, int n, int le) {
    uint64_t v = 0;
    for (int i = 0; i < n; i++) v |= (uint64_t) d[le ? i : n - 1 - i] << (8 * i);
    return v;
}

/* `@FR-R-FileHold` (lower/file_hold.rs): a loop that reads one File and nothing else
 * that can see its record keeps the record's cursor in this guard — `#index` and `#next`,
 * the handle and the byte order, read once when the hold begins — and gives them back when
 * the loop is left.  Nothing held (no open handle yet, no position, a format that is not
 * read) is a read through the record, after which the hold is taken again. */
/* The held cursor's position — `#next`, the last read's start (`#index`), and the buffered
 * bytes `[at, end)` — with the handle and byte order.  A local the reads reach only through
 * inlined code: the cold refill is handed the handle and `next` and answers the new `[at, end)`
 * in two registers (`loft_fbuf`), so neither the position nor its address crosses a call and
 * the compiler keeps every field in a register.  `nr` is -1 where nothing is held. */
typedef struct { int64_t next, at, end, last; int32_t nr; int le; } loft_fpos;
typedef struct { int64_t at, end; } loft_fbuf;

/* The held File's record and the bytes of the file from `at` on: a read is served from here and
 * a miss refills it with one positional read, as native's handle reads through its buffer.  A
 * read through the record after the loop reads at its `#next`. */
typedef struct {
    loft_ref f;
    uint8_t buf[4096];
} loft_fcur;

static loft_fpos loft_fcur_take(loft_cx *cx, loft_fcur *c) {
    loft_fpos p = { 0, 0, 0, 0, -1, 0 };
    if (c->f.store_nr == 0xFFFF || c->f.rec == 0) return p;
    uint8_t format = loft_file_format(cx, c->f);
    int32_t h = loft_file_fd(cx, c->f);
    int64_t next = loft_file_long(cx, c->f, 16);
    if (h < 0 || next == LOFT_NULL
        || (format != 1 && format != 2 && format != 3 && format != 5)) return p;
    p.le = format == 2;
    p.last = loft_file_long(cx, c->f, 8);
    p.next = next;
    p.at = next;
    p.end = next;
    p.nr = h;
    return p;
}

/* Refill the cursor's buffer from `next`.  Answers the bytes the buffer now holds, `[at, end)`.
 * Only a build with descriptors holds a cursor (`loft_fcur_take`). */
LOFT_COLD static loft_fbuf loft_fcur_fill(loft_cx *cx, loft_fcur *c, int32_t nr, int64_t next) {
    loft_fbuf sp = { next, next };
#if defined(LOFT_FD)
    sp.end = next + (int64_t) loft_fh_read(cx, nr, c->buf, sizeof c->buf, next);
#else
    (void) c;
    (void) nr;
#endif
    return sp;
}

static loft_fpos loft_fcur_hold(loft_cx *cx, loft_fcur *c, loft_ref f) {
    c->f = f;
    return loft_fcur_take(cx, c);
}

/* `w` bytes in the File's order as an integer, sign-extended from `w` bytes when `sgn`. */
LOFT_HOT_INLINE int64_t loft_fcur_decode(loft_cx *cx, const uint8_t *b, int w, int sgn, int le) {
    uint64_t u = 0;
    for (int i = 0; i < w; i++) u |= (uint64_t) b[le ? i : w - 1 - i] << (8 * i);
    return sgn && w < 8 && u >= (UINT64_C(1) << (8 * w - 1)) ? (int64_t) u - (int64_t) (UINT64_C(1) << (8 * w)) : (int64_t) u;
}

/* A read with nothing held: through the record, which is then looked at again. */
LOFT_COLD static loft_fpos loft_fcur_through(loft_cx *cx, loft_fcur *c, int w, int sgn, int64_t *val) {
    uint8_t b[8];
    uint8_t *d;
    int64_t got = loft_read_file_in(cx, c->f, w, 0, w, b, sizeof b, &d);
    if (got == w) *val = loft_fcur_decode(cx, d, w, sgn, loft_file_le(cx, c->f));
    if (d != b) free(d);
    return loft_fcur_take(cx, c);
}

/* A fixed-width integer read: `w` bytes, sign-extended when `sgn`; a short read leaves `*val`
 * as it was and the cursor where it stood, as a read through the record does.  Inline, so
 * the width is a constant at each call and the decode unrolls. */
LOFT_HOT_INLINE void loft_fcur_read(loft_cx *cx, loft_fcur *c, loft_fpos *p, int w, int sgn, int64_t *val) {
    if (p->nr < 0) {
        *p = loft_fcur_through(cx, c, w, sgn, val);
        return;
    }
    p->last = p->next;
    if (LOFT_UNLIKELY(p->next < p->at || p->next + w > p->end)) {
        loft_fbuf sp = loft_fcur_fill(cx, c, p->nr, p->next);
        p->at = sp.at;
        p->end = sp.end;
    }
    if (p->next >= p->at && p->next + w <= p->end) {
        *val = loft_fcur_decode(cx, c->buf + (p->next - p->at), w, sgn, p->le);
        p->next += w;
    }
}

/* `@FR-R-TypedRead`'s float clause: an 8-byte `float` read as `loft_fcur_read` reads the bytes
 * and decoded as the record's read decodes them (`loft_f64`); a short read leaves `*val` and the
 * cursor as they were. */
LOFT_COLD static loft_fpos loft_fcur_through_f(loft_cx *cx, loft_fcur *c, double *val) {
    uint8_t b[8];
    uint8_t *d;
    int64_t got = loft_read_file_in(cx, c->f, 8, 0, 8, b, sizeof b, &d);
    if (got == 8) *val = loft_f64(cx, (uint64_t) loft_fcur_decode(cx, d, 8, 0, loft_file_le(cx, c->f)));
    if (d != b) free(d);
    return loft_fcur_take(cx, c);
}

LOFT_HOT_INLINE void loft_fcur_readf(loft_cx *cx, loft_fcur *c, loft_fpos *p, double *val) {
    if (p->nr < 0) {
        *p = loft_fcur_through_f(cx, c, val);
        return;
    }
    p->last = p->next;
    if (LOFT_UNLIKELY(p->next < p->at || p->next + 8 > p->end)) {
        loft_fbuf sp = loft_fcur_fill(cx, c, p->nr, p->next);
        p->at = sp.at;
        p->end = sp.end;
    }
    if (p->next >= p->at && p->next + 8 <= p->end) {
        *val = loft_f64(cx, (uint64_t) loft_fcur_decode(cx, c->buf + (p->next - p->at), 8, 0, p->le));
        p->next += 8;
    }
}

static void loft_fcur_release(loft_cx *cx, loft_fcur *c, loft_fpos p) {
    if (p.nr < 0) return;
    loft_file_set_long(cx, c->f, 8, p.last);
    loft_file_set_long(cx, c->f, 16, p.next);
}

static uint32_t loft_bits32(loft_cx *cx, float g) { uint32_t u; memcpy(&u, &g, 4); return u; }
static uint64_t loft_bits64(loft_cx *cx, double d) { uint64_t u; memcpy(&u, &d, 8); return u; }

static uint64_t loft_u64(loft_cx *cx, loft_store_t *s, uint32_t rec, uint32_t byte) {
    uint64_t v;
    memcpy(&v, loft_at(cx, s, rec, byte), 8);
    return v;
}

/* read_data of the builtin types, at a place in a store. */
static void loft_ser0(loft_cx *cx, loft_text *b, loft_ref r, int le) { loft_put(cx, b, loft_u64(cx, loft_store_of(cx, r), r.rec, r.pos), 8, le); }
static void loft_ser1(loft_cx *cx, loft_text *b, loft_ref r, int le) { loft_ser0(cx, b, r, le); }
static void loft_ser2(loft_cx *cx, loft_text *b, loft_ref r, int le) { loft_put(cx, b, loft_u32(cx, loft_store_of(cx, r), r.rec, r.pos), 4, le); }
static void loft_ser3(loft_cx *cx, loft_text *b, loft_ref r, int le) { loft_ser0(cx, b, r, le); }
static void loft_ser4(loft_cx *cx, loft_text *b, loft_ref r, int le) { (void) le; loft_push_char(cx, b, (char) *loft_at(cx, loft_store_of(cx, r), r.rec, r.pos)); }
static void loft_ser5(loft_cx *cx, loft_text *b, loft_ref r, int le) { (void) le; loft_text_append(cx, b, loft_get_text(cx, r, 0)); }
static void loft_ser6(loft_cx *cx, loft_text *b, loft_ref r, int le) { loft_ser2(cx, b, r, le); }

/* write_data of the builtin types: `d` holds `n` bytes, at least the type's width. */
static void loft_de0(loft_cx *cx, loft_ref r, int le, const uint8_t *d, size_t n) {
    (void) n;
    uint64_t v = loft_take(cx, d, 8, le);
    memcpy(loft_at(cx, loft_store_of(cx, r), r.rec, r.pos), &v, 8);
}
static void loft_de1(loft_cx *cx, loft_ref r, int le, const uint8_t *d, size_t n) { loft_de0(cx, r, le, d, n); }
static void loft_de2(loft_cx *cx, loft_ref r, int le, const uint8_t *d, size_t n) {
    (void) n;
    uint32_t v = (uint32_t) loft_take(cx, d, 4, le);
    memcpy(loft_at(cx, loft_store_of(cx, r), r.rec, r.pos), &v, 4);
}
static void loft_de3(loft_cx *cx, loft_ref r, int le, const uint8_t *d, size_t n) { loft_de0(cx, r, le, d, n); }
static void loft_de4(loft_cx *cx, loft_ref r, int le, const uint8_t *d, size_t n) { (void) le; (void) n; *loft_at(cx, loft_store_of(cx, r), r.rec, r.pos) = d[0]; }
static void loft_de5(loft_cx *cx, loft_ref r, int le, const uint8_t *d, size_t n) { (void) le; loft_set_text(cx, r, 0, loft_lit(cx, (const char *) d, n)); }
static void loft_de6(loft_cx *cx, loft_ref r, int le, const uint8_t *d, size_t n) { loft_de2(cx, r, le, d, n); }

/* ── file natives ── */

/* read_bytes (Stores::fs_read_bytes): the whole file as a `vector<u8>` in a store of its own,
 * or null for a missing or unreadable one. */
static loft_ref loft_read_bytes(loft_cx *cx, loft_str raw) {
    char *path = loft_resolve(cx, raw);
    FILE *h = path ? loft_fopen(path, "rb") : 0;
    free(path);
    if (!h) return loft_ref_null(cx);
    /* The whole file first, then one exact claim of its length, an empty file's included
     * (`vector::alloc_vector_from_bytes`, `@FR-Col-Capacity`): appended a byte at a time, the
     * vector took the growth step's room instead. */
    size_t len = 0, cap = 4096;
    uint8_t *data = (uint8_t *) malloc(cap);
    size_t n;
    while (data && (n = fread(data + len, 1, cap - len, h)) > 0) {
        len += n;
        if (len == cap) {
            uint8_t *grown = (uint8_t *) realloc(data, cap * 2);
            if (!grown) { free(data); data = 0; break; }
            data = grown;
            cap *= 2;
        }
    }
    fclose(h);
    if (!data) { fputs("loft: out of memory\n", stderr); abort(); }
    /* A vector counts its elements in 32 bits. A file with more bytes than that is not cut to
     * fit: the count is held at the largest one, which the capacity check refuses by name. */
    if (len > UINT32_MAX) len = UINT32_MAX;
    loft_ref r = loft_ref_null(cx);
    loft_vector_store(cx, &r);
    loft_vec_prealloc(cx, r, (uint32_t) len, 1);
    loft_store_t *s = loft_store_of(cx, r);
    uint32_t rec = loft_collection_rec(cx, s, r.rec, r.pos);
    loft_set_u32(cx, s, rec, 4, (uint32_t) len);
    if (len) memcpy(loft_at(cx, s, rec, 8), data, len);
    free(data);
    return r;
}

#if defined(LOFT_FOREIGN)
#if defined(LOFT_POSIX_HOST)
static void loft_unmap(void *bytes, size_t n) { munmap(bytes, n); }
#elif defined(_WIN32)
/* Declared as `windows.h` declares them, which loft_mapped.h brings in later in the unit. */
__declspec(dllimport) void *__stdcall CreateFileMappingW(void *file, struct _SECURITY_ATTRIBUTES *security,
                                                         unsigned long protect, unsigned long high,
                                                         unsigned long low, const wchar_t *name);
__declspec(dllimport) void *__stdcall MapViewOfFile(void *mapping, unsigned long access, unsigned long high,
                                                    unsigned long low, size_t bytes);
__declspec(dllimport) int __stdcall UnmapViewOfFile(const void *base);
static void loft_unmap(void *bytes, size_t n) {
    (void) n;
    UnmapViewOfFile(bytes);
}
#endif

/* file_map (Stores::fs_file_map): the whole file mapped read-only as a FOREIGN `vector<u8>` —
 * a store of its own whose slot names `LOFT_FOREIGN_REC`, the mapping its owner — or null for a
 * missing or unreadable one.  An empty file, or one that is not a regular file, is read as
 * read_bytes reads it; so is every file where there is no mmap.  Windows maps it too, as the
 * interpreter's `memmap2` does there: read into a store of its own instead, the bytes reached a
 * library call beside a vector of another store through that other store (`Stores::bridge_args`
 * copies a FOREIGN vector into the store the library is handed, and only a foreign one). */
static loft_ref loft_file_map(loft_cx *cx, loft_str raw) {
#if defined(_WIN32)
    char *path = loft_resolve(cx, raw);
    if (!path) return loft_ref_null(cx);
    struct _stat64 st;
    if (loft_wstat64(path, &st) != 0 || (uint64_t) st.st_size > UINT32_MAX) {
        free(path);
        return loft_ref_null(cx);
    }
    if (st.st_size == 0 || !(st.st_mode & _S_IFREG)) {
        free(path);
        return loft_read_bytes(cx, raw);
    }
    size_t n = (size_t) st.st_size;
    wchar_t *wp = loft_wpath(path);
    free(path);
    /* GENERIC_READ, shared for reading, writing and deletion, OPEN_EXISTING. */
    void *h = CreateFileW(wp, 0x80000000ul, 7ul, 0, 3ul, 0x80ul, 0);
    free(wp);
    if (h == (void *) (intptr_t) -1) return loft_ref_null(cx);
    /* PAGE_READONLY, then FILE_MAP_READ; the view keeps the mapping and the file alive. */
    void *m = CreateFileMappingW(h, 0, 0x02ul, 0, 0, 0);
    CloseHandle(h);
    void *map = m ? MapViewOfFile(m, 0x04ul, 0, 0, n) : 0;
    if (m) CloseHandle(m);
    if (!map) return loft_ref_null(cx);
#elif defined(LOFT_POSIX_HOST)
    char *path = loft_resolve(cx, raw);
    if (!path) return loft_ref_null(cx);
    int fd = open(path, O_RDONLY);
    free(path);
    if (fd < 0) return loft_ref_null(cx);
    struct stat st;
    if (fstat(fd, &st) != 0 || (uint64_t) st.st_size > UINT32_MAX) {
        close(fd);
        return loft_ref_null(cx);
    }
    if (st.st_size == 0 || !S_ISREG(st.st_mode)) {
        close(fd);
        return loft_read_bytes(cx, raw);
    }
    size_t n = (size_t) st.st_size;
    void *map = mmap(0, n, PROT_READ, MAP_PRIVATE, fd, 0);
    close(fd);
    if (map == MAP_FAILED) return loft_ref_null(cx);
#endif
#if defined(_WIN32) || defined(LOFT_POSIX_HOST)
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
    loft_ref r = loft_ref_null(cx);
    loft_database(cx, &r, 4);
    loft_set_u32(cx, loft_store_of(cx, r), r.rec, r.pos, 0);
    loft_st_make_foreign(cx, loft_store_of(cx, r), r.rec, r.pos, span);
    return r;
#else
    return loft_read_bytes(cx, raw);
#endif
}
#endif

/* write_bytes (Stores::fs_write_bytes): the file is the vector's bytes, replacing what it held. */
static uint8_t loft_write_bytes(loft_cx *cx, loft_str raw, loft_ref v) {
    char *path = loft_resolve(cx, raw);
    if (!path) return 0;
    FILE *h = loft_fopen(path, "wb");
    free(path);
    if (!h) return 0;
    int64_t n = loft_vec_len(cx, v);
    int ok = 1;
    if (n > 0) {
        uint32_t r = loft_collection_rec(cx, loft_store_of(cx, v), v.rec, v.pos);
        ok = fwrite(loft_at(cx, loft_store_of(cx, v), r, 8), 1, (size_t) n, h) == (size_t) n;
    }
    return (uint8_t) (fclose(h) == 0 && ok);
}

/* write_text_raw (Stores::write_file): `v` at the File's position — its first write creates
 * the file empty — leaving the position around what it wrote. */
static uint8_t loft_write_text_raw(loft_cx *cx, loft_ref f, loft_str v) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return 0;
    /* A record with no position yet makes its file anew; one a copy carries on where it stood. */
    int64_t start = loft_file_long(cx, f, 16);
    loft_fh h;
    char *path;
    if (!loft_file_get(cx, f, start == LOFT_NULL ? LOFT_OPEN_EMPTY : LOFT_OPEN_CREATE, &h, &path)) {
        if (path) fprintf(stderr, "loft: cannot create a file for writing: %s — write skipped\n", strerror(errno));
        free(path);
        return 0;
    }
    free(path);
    if (start == LOFT_NULL) start = 0;
    int ok = !v.len || loft_fh_write(cx, h, v.p, v.len, start);
    loft_file_put(cx, h);
    if (!ok) return 0;
    loft_file_set_long(cx, f, 8, start);
    loft_file_set_long(cx, f, 16, start + (int64_t) v.len);
    return 1;
}

/* list_dir (Stores::fs_list_dir): the entries' names in order, or null for a path that is not a
 * readable directory. */
static loft_ref loft_list_dir(loft_cx *cx, loft_str raw) {
    char *dir = loft_resolve(cx, raw);
    if (!dir) return loft_ref_null(cx);
    loft_ref r = loft_ref_null(cx);
    size_t n = 0;
    char **names = loft_dir_names(cx, dir, &n);
    if (names) {
        loft_vector_store(cx, &r);
        for (size_t i = 0; i < n; i++) {
            loft_ref el = loft_vec_append(cx, r, 4);
            loft_set_text(cx, el, 0, loft_lit(cx, names[i], strlen(names[i])));
            loft_vec_finish(cx, r);
            free(names[i]);
        }
        free(names);
    }
    free(dir);
    return r;
}

/* mtime: the path's modification time in Unix seconds, 0 when unknown. */
static int64_t loft_mtime(loft_cx *cx, loft_str raw) {
    char *path = loft_resolve(cx, raw);
    int64_t t = 0;
#if defined(LOFT_POSIX)
    struct stat st;
    if (path && stat(path, &st) == 0 && st.st_mtime > 0) t = (int64_t) st.st_mtime;
#elif defined(_WIN32)
    struct _stat64 st;
    if (path && loft_wstat64(path, &st) == 0 && st.st_mtime > 0) t = (int64_t) st.st_mtime;
#endif
    free(path);
    return t;
}

static uint8_t loft_is_dir_at(loft_cx *cx, loft_str raw) {
    char *path = loft_resolve(cx, raw);
    uint8_t r = (uint8_t) (path && loft_is_dir(cx, path));
    free(path);
    return r;
}

/* is_symlink (Stores::fs_is_symlink_at): the path itself is a link, not followed; false
 * without POSIX, where no link is seen. */
static uint8_t loft_is_symlink_at(loft_cx *cx, loft_str raw) {
    char *path = loft_resolve(cx, raw);
    uint8_t r = 0;
#if defined(LOFT_POSIX)
    struct stat st;
    r = (uint8_t) (path && lstat(path, &st) == 0 && S_ISLNK(st.st_mode));
#endif
    free(path);
    return r;
}

static uint8_t loft_is_file_at(loft_cx *cx, loft_str raw) {
    char *path = loft_resolve(cx, raw);
    int64_t size;
    uint8_t r = (uint8_t) (path && loft_stat(cx, path, &size) == 1);
    free(path);
    return r;
}

/* os_temp_dir (std::env::temp_dir): TMPDIR, else /tmp — on Windows (GetTempPath2's order)
 * TMP, else TEMP, else USERPROFILE, else the Windows directory. */
static loft_text loft_os_temp_dir(loft_cx *cx) {
#if defined(_WIN32)
    static const char *const vars[] = { "TMP", "TEMP", "USERPROFILE", "SystemRoot" };
    for (size_t i = 0; i < sizeof vars / sizeof *vars; i++) {
        const char *t = getenv(vars[i]);
        if (t && *t) return loft_given(cx, t);
    }
    return loft_given(cx, "C:\\Windows");
#else
    const char *t = getenv("TMPDIR");
    return loft_given(cx, t && *t ? t : "/tmp");
#endif
}

/* `dir_with_sub`: `base` in the portable form, `sub` joined to it as `Path::join` does. */
static loft_text loft_dir_with_sub(loft_cx *cx, const char *base, loft_str sub) {
    if (!sub.len || loft_str_is_null(cx, sub) || !*base) return loft_given(cx, base);
    loft_text t = { 0, 0, 0 };
    if (sub.p[0] != '/') {
        loft_push_cstr(cx, &t, base);
        if (t.len && t.p[t.len - 1] != '/') loft_push_char(cx, &t, '/');
    }
    loft_text_append(cx, &t, sub);
    loft_push_char(cx, &t, 0);
    loft_text r = loft_given(cx, t.p);
    loft_text_free(cx, &t);
    return r;
}

/* directory (Stores::os_directory_native): the working directory, `sub` joined to it. */
static loft_text loft_directory(loft_cx *cx, loft_str sub) {
    char buf[4096];
    const char *base = "";
#if defined(LOFT_POSIX)
    if (getcwd(buf, sizeof buf)) base = buf;
#elif defined(_WIN32)
    if (_getcwd(buf, (int) sizeof buf)) base = buf;
#else
    (void) buf;
#endif
    return loft_dir_with_sub(cx, base, sub);
}

/* user_directory (Stores::os_home_native): the home directory, `sub` joined to it — `HOME`, or
 * on Windows the profile directory `dirs::home_dir` answers there (`USERPROFILE`). */
static loft_text loft_user_directory(loft_cx *cx, loft_str sub) {
#if defined(_WIN32)
    const char *h = getenv("USERPROFILE");
#else
    const char *h = getenv("HOME");
#endif
    return loft_dir_with_sub(cx, h ? h : "", sub);
}

/* program_directory (Stores::os_executable_native): the directory holding the running
 * executable — `current_exe`'s answer: `/proc/self/exe` on Linux, `_NSGetExecutablePath` on
 * macOS, else `argv[0]` — `sub` joined to it. */
#if defined(__APPLE__)
extern int _NSGetExecutablePath(char *buf, uint32_t *size);
#endif
static loft_text loft_program_directory(loft_cx *cx, loft_str sub) {
    char buf[4096];
    buf[0] = 0;
#if defined(__linux__) && defined(LOFT_POSIX)
    /* `readlink` is POSIX: a LOFT_NO_POSIX build has no <unistd.h> and answers no path. */
    ssize_t n = readlink("/proc/self/exe", buf, sizeof buf - 1);
    buf[n > 0 ? n : 0] = 0;
#elif defined(__APPLE__)
    uint32_t size = sizeof buf;
    if (_NSGetExecutablePath(buf, &size) != 0) buf[0] = 0;
#elif defined(_WIN32)
    /* The CRT's full path of the program (`stdlib.h`) — no Win32 declaration, which would have
     * to agree with `windows.h`'s where a socket unit includes it. */
    char *pg = 0;
    if (_get_pgmptr(&pg) == 0 && pg && strlen(pg) < sizeof buf) strcpy(buf, pg);
#endif
#if defined(LOFT_POSIX_HOST)
    if (!buf[0] && loft_argv && loft_argv[0]) {
        char real[4096];
        if (realpath(loft_argv[0], real)) strcpy(buf, real);
    }
#else
    /* No `realpath` (WASI): an absolute `argv[0]` is taken as it stands. */
    if (!buf[0] && loft_argv && loft_argv[0] && loft_argv[0][0] == '/' && strlen(loft_argv[0]) < sizeof buf)
        strcpy(buf, loft_argv[0]);
#endif
    char *slash = strrchr(buf, '/');
#if defined(_WIN32)
    char *back = strrchr(buf, '\\');
    if (back && (!slash || back > slash)) slash = back;
#endif
    if (slash) *(slash == buf ? slash + 1 : slash) = 0; else buf[0] = 0;
    return loft_dir_with_sub(cx, buf, sub);
}

/* os_cache_dir (cache::cache_base_dir): $XDG_CACHE_HOME/loft, else $HOME/.cache/loft, else the
 * temporary directory's. */
static loft_text loft_os_cache_dir(loft_cx *cx) {
    loft_text t = { 0, 0, 0 };
    const char *x = getenv("XDG_CACHE_HOME"), *h = getenv("HOME"), *tmp = getenv("TMPDIR");
    if (x) {
        loft_push_cstr(cx, &t, x);
    } else if (h) {
        loft_push_cstr(cx, &t, h);
        loft_push_cstr(cx, &t, "/.cache");
    } else {
        /* `std::env::temp_dir`, as `loft_os_temp_dir` answers it on this host. */
        loft_text td = loft_os_temp_dir(cx);
        (void) tmp;
        loft_text_append(cx, &t, loft_view(cx, &td));
        loft_text_free(cx, &td);
    }
    if (t.len && t.p[t.len - 1] != '/') loft_push_char(cx, &t, '/');
    loft_push_cstr(cx, &t, "loft");
    loft_push_char(cx, &t, 0);
    loft_text r = loft_given(cx, t.p);
    loft_text_free(cx, &t);
    return r;
}

/* ymd_days_ago (Stores::ymd_days_ago_native): the UTC date `days` before today, `YYYY-MM-DD`,
 * by logger::days_to_ymd's civil-from-days. */
static loft_text loft_ymd_days_ago(loft_cx *cx, int64_t days) {
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
    return loft_text_own(cx, loft_lit(cx, b, (size_t) n));
}

/* env_variables (Stores::os_variables): every environment variable as an EnvVariable (name at 0,
 * value at 4) of `size` bytes, in the environment's order, in a vector of its own store. */
#if defined(LOFT_POSIX)
extern char **environ;
#define LOFT_ENVIRON environ
#elif defined(_WIN32)
/* The CRT's own copy of the environment (`stdlib.h`). */
#define LOFT_ENVIRON _environ
#endif
static loft_ref loft_env_variables(loft_cx *cx, uint32_t size) {
    loft_ref vec = loft_ref_null(cx);
    loft_database(cx, &vec, size * 8 - 8);
    loft_set_u32(cx, loft_store_of(cx, vec), vec.rec, vec.pos, 0);
#if defined(LOFT_ENVIRON)
    for (char **e = LOFT_ENVIRON; e && *e; e++) {
        const char *eq = strchr(*e, '=');
        if (!eq) continue;
        loft_ref el = loft_vec_append(cx, vec, size);
        loft_set_text(cx, el, 0, loft_lit(cx, *e, (size_t) (eq - *e)));
        loft_set_text(cx, el, 4, loft_lit(cx, eq + 1, strlen(eq + 1)));
        loft_vec_finish(cx, vec);
    }
#endif
    return vec;
}

/* source_dir: the program's own directory. */
static loft_text loft_source_dir(loft_cx *cx) {
    return loft_given(cx, loft_anchor);
}

static uint32_t loft_path_sep(loft_cx *cx) { return '/'; }

/* env_variable (Stores::os_variable): the variable's value, the null text when it is unset. */
static loft_text loft_env_variable(loft_cx *cx, loft_str name) {
    char *n = (char *) malloc(name.len + 1);
    if (!n) { fputs("loft: out of memory\n", stderr); abort(); }
    memcpy(n, name.p, name.len);
    n[name.len] = 0;
    const char *v = getenv(n);
    free(n);
    return loft_text_own(cx, v ? loft_lit(cx, v, strlen(v)) : loft_null_text(cx));
}
