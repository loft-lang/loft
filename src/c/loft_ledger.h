/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * The test ledger on the C (`@F38`, `src/ledger.rs`): a program built under `LOFT_LEDGER` has
 * every fault site instrumented as `__ledger(site, kind, a, b)`, which answers the CHECKED value
 * — what the operator answers in an ordinary run — and counts the site as run, and as faulted
 * where the processor's answer would differ.  At exit the counts merge into the ledger file the
 * interpreter writes too: the same lines, the same order, so a run on either backend adds to one
 * ledger.  Compiled into a unit only when it is a ledger build (`LOFT_LEDGER` defined). */
#ifdef LOFT_LEDGER
#if defined(LOFT_POSIX)
#include <fcntl.h>
#include <sys/file.h>
#include <unistd.h>
#endif

/* One fault site: its function, the hash of that function's body, its ordinal, its kind. */
typedef struct {
    const char *function;
    uint64_t body;
    uint32_t ordinal;
    loft_word kind;
} loft_lg_site;

/* The armed program's table and counters: set once by `main` before anything runs. */
static const loft_lg_site *loft_lg_sites;
static size_t loft_lg_count;
static uint64_t *loft_lg_runs;
static uint64_t *loft_lg_faults;
static const char *loft_lg_path;

/* The name each kind is written under (`ledger::kind_name`). */
static const char *loft_lg_kind(loft_word kind) {
    static const char *names[] = { "add", "sub", "mul", "div", "rem", "neg", "and", "or", "xor", "shr" };
    return kind >= 0 && kind < 10 ? names[kind] : "?";
}

static void loft_lg_record(loft_word site, int faulted) {
    if (!loft_lg_runs || site < 0 || (uint64_t) site >= loft_lg_count) return;
    uint64_t sum;
    LOFT_PUB_ADD(loft_lg_runs[site], 1, sum);
    if (faulted) LOFT_PUB_ADD(loft_lg_faults[site], 1, sum);
    (void) sum;
}

/* `ledger::op`: the checked answer, with the site counted.  The processor's answer wraps, as
 * Rust's `wrapping_*` does; a zero divisor has none, so it counts as a fault. */
static loft_word loft_ledger_at(loft_cx *cx, loft_word site, loft_word kind, loft_word a, loft_word b, const char *file,
                              uint32_t line) {
    uint64_t ua = (uint64_t) a, ub = (uint64_t) b;
    loft_word checked;
    loft_word plain = 0;
    int has_plain = 1;
    switch (kind) {
    case 0: checked = loft_add_r(cx, a, b); plain = (loft_word) (ua + ub); break;
    case 1: checked = loft_sub_r(cx, a, b); plain = (loft_word) (ua - ub); break;
    case 2: checked = loft_mul_r(cx, a, b); plain = (loft_word) (ua * ub); break;
    case 3:
    case 4:
        if (b == 0) {
            checked = kind == 3 ? loft_div_at(cx, a, b, file, line) : loft_rem_at(cx, a, b, file, line);
            has_plain = 0;
        } else if (kind == 3) {
            checked = loft_div(cx, a, b);
            plain = b == -1 ? (loft_word) (0 - ua) : a / b;
        } else {
            checked = loft_rem(cx, a, b);
            plain = b == -1 ? 0 : a % b;
        }
        break;
    case 5: checked = loft_neg(cx, a); plain = (loft_word) (0 - ua); break;
    case 6: checked = loft_and(cx, a, b); plain = a & b; break;
    case 7: checked = loft_or(cx, a, b); plain = a | b; break;
    case 8: checked = loft_xor(cx, a, b); plain = a ^ b; break;
    /* Only a constant shift in 0..64 is a site. */
    case 9:
        plain = a >> (b & 63);
        checked = a == LOFT_NULL || b == LOFT_NULL ? LOFT_NULL : plain;
        break;
    default: checked = LOFT_NULL; has_plain = 0; break;
    }
    loft_lg_record(site, !has_plain || plain != checked);
    return checked;
}

static loft_word loft_ledger(loft_cx *cx, loft_word site, loft_word kind, loft_word a, loft_word b) {
    return loft_ledger_at(cx, site, kind, a, b, "", 0);
}

/* `__ledger_div`: a division or remainder, whose zero divisor reports as `OpDivInt` does. */
static loft_word loft_ledger_div(loft_cx *cx, loft_word site, loft_word kind, loft_word a, loft_word b) {
    return loft_ledger_at(cx, site, kind, a, b, "", 0);
}

/* One ledger line (`ledger::Entry`). */
typedef struct {
    char *function;
    uint64_t body;
    uint32_t ordinal;
    char kind[8];
    uint64_t runs, faults;
} loft_lg_entry;

#define LOFT_LG_HEADER "# loft test ledger 1: function, body hash, site, operator, runs, faults"

static int loft_lg_cmp(const void *x, const void *y) {
    const loft_lg_entry *a = (const loft_lg_entry *) x, *b = (const loft_lg_entry *) y;
    int c = strcmp(a->function, b->function);
    if (c) return c;
    if (a->body != b->body) return a->body < b->body ? -1 : 1;
    if (a->ordinal != b->ordinal) return a->ordinal < b->ordinal ? -1 : 1;
    return strcmp(a->kind, b->kind);
}

/* The entries of the ledger at `path` (none for an absent or empty file), or -1 for a file
 * that is not a ledger, or has a malformed line, with `why` saying which. */
static long loft_lg_read(const char *path, loft_lg_entry **out, char *why, size_t why_cap) {
    *out = 0;
    FILE *f = fopen(path, "rb");
    if (!f) return 0;
    long count = 0, cap = 0;
    char buf[4096];
    int first = 1;
    long n = 1;
    while (fgets(buf, sizeof buf, f)) {
        size_t len = strlen(buf);
        while (len && (buf[len - 1] == '\n' || buf[len - 1] == '\r')) buf[--len] = 0;
        if (first) {
            first = 0;
            if (strcmp(buf, LOFT_LG_HEADER) != 0) {
                snprintf(why, why_cap, "not a loft test ledger (its first line is not the header)");
                fclose(f);
                return -1;
            }
            continue;
        }
        n++;
        if (!len) continue;
        char *fields[6];
        int nf = 0;
        char *p = buf;
        fields[nf++] = p;
        for (; *p; p++)
            if (*p == '\t') {
                *p = 0;
                if (nf == 6) { nf = 7; break; }
                fields[nf++] = p + 1;
            }
        char *end = 0;
        loft_lg_entry e;
        int bad = nf != 6 || strlen(fields[3]) >= sizeof e.kind;
        if (!bad) {
            e.body = strtoull(fields[1], &end, 16);
            bad |= *end != 0 || !*fields[1];
            e.ordinal = (uint32_t) strtoul(fields[2], &end, 10);
            bad |= *end != 0 || !*fields[2];
            e.runs = strtoull(fields[4], &end, 10);
            bad |= *end != 0 || !*fields[4];
            e.faults = strtoull(fields[5], &end, 10);
            bad |= *end != 0 || !*fields[5];
        }
        if (bad) {
            snprintf(why, why_cap, "line %ld is malformed", n);
            fclose(f);
            for (long i = 0; i < count; i++) free((*out)[i].function);
            free(*out);
            *out = 0;
            return -1;
        }
        e.function = (char *) malloc(strlen(fields[0]) + 1);
        if (!e.function) { fputs("loft: out of memory\n", stderr); abort(); }
        strcpy(e.function, fields[0]);
        strcpy(e.kind, fields[3]);
        if (count == cap) {
            cap = cap ? cap * 2 : 64;
            *out = (loft_lg_entry *) realloc(*out, (size_t) cap * sizeof **out);
            if (!*out) { fputs("loft: out of memory\n", stderr); abort(); }
        }
        (*out)[count++] = e;
    }
    fclose(f);
    return count;
}

/* `ledger::flush`: merge the run's counts into the file, under an exclusive lock on a sibling
 * lock file so test processes running at once each add theirs; the new text replaces the file
 * by a rename. */
static void loft_lg_flush(void) {
    if (!loft_lg_runs || !loft_lg_path) return;
    const char *path = loft_lg_path;
    loft_lg_path = 0;
    size_t pl = strlen(path);
    char *lock_path = (char *) malloc(pl + 6);
    char *tmp = (char *) malloc(pl + 32);
    if (!lock_path || !tmp) { fputs("loft: out of memory\n", stderr); abort(); }
    snprintf(lock_path, pl + 6, "%s.lock", path);
#if defined(LOFT_POSIX)
    int lock = open(lock_path, O_RDWR | O_CREAT, 0644);
    if (lock >= 0) (void) flock(lock, LOCK_EX);
    snprintf(tmp, pl + 32, "%s.%ld.tmp", path, (long) getpid());
#else
    snprintf(tmp, pl + 32, "%s.tmp", path);
#endif
    char why[128] = "";
    loft_lg_entry *e = 0;
    long n = loft_lg_read(path, &e, why, sizeof why);
    if (n < 0) {
        fprintf(stderr, "loft: the test ledger %s was not written: %s: %s\n", path, path, why);
    } else {
        long cap = n + (long) loft_lg_count;
        e = (loft_lg_entry *) realloc(e, (size_t) (cap ? cap : 1) * sizeof *e);
        if (!e) { fputs("loft: out of memory\n", stderr); abort(); }
        for (size_t s = 0; s < loft_lg_count; s++) {
            const loft_lg_site *site = &loft_lg_sites[s];
            const char *kind = loft_lg_kind(site->kind);
            long at = -1;
            for (long i = 0; i < n && at < 0; i++)
                if (e[i].body == site->body && e[i].ordinal == site->ordinal && !strcmp(e[i].kind, kind)
                    && !strcmp(e[i].function, site->function))
                    at = i;
            if (at < 0) {
                at = n++;
                e[at].function = (char *) malloc(strlen(site->function) + 1);
                if (!e[at].function) { fputs("loft: out of memory\n", stderr); abort(); }
                strcpy(e[at].function, site->function);
                e[at].body = site->body;
                e[at].ordinal = site->ordinal;
                snprintf(e[at].kind, sizeof e[at].kind, "%s", kind);
                e[at].runs = 0;
                e[at].faults = 0;
            }
            e[at].runs += loft_lg_runs[s];
            e[at].faults += loft_lg_faults[s];
        }
        if (n > 1) qsort(e, (size_t) n, sizeof *e, loft_lg_cmp);
        FILE *f = fopen(tmp, "wb");
        int ok = f != 0;
        if (f) {
            fprintf(f, "%s\n", LOFT_LG_HEADER);
            for (long i = 0; i < n; i++)
                fprintf(f, "%s\t%016llx\t%u\t%s\t%llu\t%llu\n", e[i].function, (unsigned long long) e[i].body,
                        (unsigned) e[i].ordinal, e[i].kind, (unsigned long long) e[i].runs,
                        (unsigned long long) e[i].faults);
            ok = fclose(f) == 0;
        }
#if defined(_WIN32)
        if (ok) (void) remove(path);
#endif
        if (!ok || rename(tmp, path) != 0)
            fprintf(stderr, "loft: the test ledger %s was not written: %s\n", path, strerror(errno));
        for (long i = 0; i < n; i++) free(e[i].function);
    }
    free(e);
#if defined(LOFT_POSIX)
    if (lock >= 0) close(lock);
#endif
    free(lock_path);
    free(tmp);
}

/* `ledger::arm_static`: the program's site table, armed by `main` when `LOFT_LEDGER` names a
 * file; the counts merge into it at exit.  A ledger build run without it records nothing. */
static void loft_ledger_arm(loft_cx *cx, const loft_lg_site *sites, size_t count) {
    (void) cx;
    const char *path = getenv("LOFT_LEDGER");
    if (!path || !*path || !count) return;
    loft_lg_runs = (uint64_t *) calloc(count, sizeof *loft_lg_runs);
    loft_lg_faults = (uint64_t *) calloc(count, sizeof *loft_lg_faults);
    if (!loft_lg_runs || !loft_lg_faults) { fputs("loft: out of memory\n", stderr); abort(); }
    loft_lg_sites = sites;
    loft_lg_count = count;
    loft_lg_path = path;
    atexit(loft_lg_flush);
}

#endif
