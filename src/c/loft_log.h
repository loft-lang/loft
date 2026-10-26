/* ── The runtime logger (logger.rs) ──────────────────────────────────────────────────────
 * log_info / log_warn / log_error / log_fatal write `<utc> <SEV>  <file>:<line>  <message>`
 * records to the log file as the interpreter's `Logger` does: the config is `LOFT_LOG_CONF`,
 * else `.loft/log.conf` beside the program, else `log.conf` beside it (`resolve_config_path`);
 * without one the records at warning and above go to `.loft/log.txt` beside the program.  The
 * level filter with its `[levels]` overrides, the per-site rate limit and its suppression line,
 * daily and size rotation, and the five-second reload of a changed config are `Logger`'s.  The
 * file is opened at the first record that passes the filter, where the interpreter opens it at
 * start (an empty `.loft/log.txt` beside a program that never logs is the one difference).
 * `production = true` makes a failed assert or a panic a logged record the program continues
 * past (`runtime_error::production_logged`, @C66), and the run then exits 1. */

#define LOFT_SEV_INFO 0
#define LOFT_SEV_WARN 1
#define LOFT_SEV_ERROR 2
#define LOFT_SEV_FATAL 3

typedef struct { char *key; int sev; } loft_log_level;
typedef struct { char *file; int64_t line, window_us; uint32_t count, suppressed; } loft_log_rate;

static struct {
    int ready;
    char *conf, *path, *base;
    int64_t conf_mtime, checked_us;
    int level, production, daily;
    uint64_t max_size, size;
    uint32_t max_files, rate;
    loft_log_level *levels;
    size_t nlevels;
    loft_log_rate *rates;
    size_t nrates;
    FILE *f;
    uint32_t ymd;
} loft_lg;

static int loft_had_fatal = 0;

static char *loft_log_dup(const char *s, size_t n) {
    char *d = (char *) malloc(n + 1);
    if (!d) { fputs("loft: out of memory\n", stderr); abort(); }
    memcpy(d, s, n);
    d[n] = 0;
    return d;
}

static char *loft_log_join(const char *dir, const char *rel) {
    size_t a = strlen(dir), b = strlen(rel);
    char *p = (char *) malloc(a + b + 2);
    if (!p) { fputs("loft: out of memory\n", stderr); abort(); }
    memcpy(p, dir, a);
    if (a && dir[a - 1] != '/' && dir[a - 1] != '\\') p[a++] = '/';
    memcpy(p + a, rel, b + 1);
    return p;
}

static int loft_log_absolute(const char *p) {
    return p[0] == '/' || p[0] == '\\' || (p[0] && p[1] == ':');
}

static int loft_log_exists(const char *p) {
    FILE *f = fopen(p, "rb");
    if (f) fclose(f);
    return f != 0;
}

/* The directory part of `p`, "." when it has none. */
static char *loft_log_dirname(const char *p) {
    const char *s = strrchr(p, '/');
#if defined(_WIN32)
    const char *b = strrchr(p, '\\');
    if (b && (!s || b > s)) s = b;
#endif
    return s ? loft_log_dup(p, s == p ? 1 : (size_t) (s - p)) : loft_log_dup(".", 1);
}

static int64_t loft_log_mtime(const char *p) {
#if defined(LOFT_POSIX)
    struct stat st;
    return p && stat(p, &st) == 0 ? (int64_t) st.st_mtime : -1;
#else
    return p && loft_log_exists(p) ? 0 : -1;
#endif
}

static int loft_log_sev(const char *v, size_t n) {
    char low[8];
    size_t i;
    if (n >= sizeof low) return -1;
    for (i = 0; i < n; i++) low[i] = (char) (v[i] >= 'A' && v[i] <= 'Z' ? v[i] + 32 : v[i]);
    low[n] = 0;
    if (!strcmp(low, "info")) return LOFT_SEV_INFO;
    if (!strcmp(low, "warn") || !strcmp(low, "warning")) return LOFT_SEV_WARN;
    if (!strcmp(low, "error")) return LOFT_SEV_ERROR;
    if (!strcmp(low, "fatal")) return LOFT_SEV_FATAL;
    return -1;
}

static int loft_log_true(const char *v, size_t n) {
    return (n == 1 && v[0] == '1') ||
           (n == 4 && (v[0] | 32) == 't' && (v[1] | 32) == 'r' && (v[2] | 32) == 'u' && (v[3] | 32) == 'e');
}

static uint64_t loft_log_uint(const char *v, size_t n, int *ok) {
    uint64_t r = 0;
    size_t i;
    *ok = n > 0;
    for (i = 0; i < n; i++) {
        if (v[i] < '0' || v[i] > '9' || r > UINT64_MAX / 10 - 9) { *ok = 0; return 0; }
        r = r * 10 + (uint64_t) (v[i] - '0');
    }
    return r;
}

static void loft_log_trim(const char **s, size_t *n) {
    while (*n && ((*s)[0] == ' ' || (*s)[0] == '\t' || (*s)[0] == '\r')) { (*s)++; (*n)--; }
    while (*n && ((*s)[*n - 1] == ' ' || (*s)[*n - 1] == '\t' || (*s)[*n - 1] == '\r')) (*n)--;
}

static void loft_log_unquote(const char **s, size_t *n) {
    while (*n && (*s)[0] == '"') { (*s)++; (*n)--; }
    while (*n && (*s)[*n - 1] == '"') (*n)--;
}

/* The defaults of `RuntimeLogConfig`, then `parse_config_str` over `text`: `[log] file / level /
 * production`, `[rotation] max_size_mb / daily / max_files`, `[rate_limit] per_site`,
 * `[levels] <file or dir/> = <level>`.  A relative `file` is the config's directory's. */
static void loft_log_configure(const char *text, const char *conf_dir) {
    char section[32] = "";
    const char *line = text;
    size_t i;
    for (i = 0; i < loft_lg.nlevels; i++) free(loft_lg.levels[i].key);
    loft_lg.nlevels = 0;
    loft_lg.level = LOFT_SEV_WARN;
    loft_lg.production = 0;
    loft_lg.max_size = (uint64_t) 500 * 1024 * 1024;
    loft_lg.daily = 1;
    loft_lg.max_files = 10;
    loft_lg.rate = 5;
    while (line && *line) {
        const char *end = strchr(line, '\n'), *eq;
        size_t n = end ? (size_t) (end - line) : strlen(line);
        const char *l = line;
        loft_log_trim(&l, &n);
        line = end ? end + 1 : 0;
        if (!n || l[0] == '#') continue;
        if (l[0] == '[' && l[n - 1] == ']') {
            const char *sec = l + 1;
            size_t sn = n - 2;
            loft_log_trim(&sec, &sn);
            if (sn >= sizeof section) sn = sizeof section - 1;
            memcpy(section, sec, sn);
            section[sn] = 0;
            continue;
        }
        eq = (const char *) memchr(l, '=', n);
        if (eq) {
            const char *k = l, *v = eq + 1;
            size_t kn = (size_t) (eq - l), vn = n - kn - 1;
            int ok;
            uint64_t u;
            loft_log_trim(&k, &kn);
            loft_log_unquote(&k, &kn);
            loft_log_trim(&v, &vn);
            loft_log_unquote(&v, &vn);
#define LOFT_KEY(s) (kn == sizeof(s) - 1 && !memcmp(k, s, kn))
            if (!strcmp(section, "log") && LOFT_KEY("file")) {
                char *val = loft_log_dup(v, vn);
                free(loft_lg.path);
                loft_lg.path = loft_log_absolute(val) ? loft_log_dup(val, vn) : loft_log_join(conf_dir, val);
                free(val);
            } else if (!strcmp(section, "log") && LOFT_KEY("level")) {
                int s = loft_log_sev(v, vn);
                if (s >= 0) loft_lg.level = s;
            } else if (!strcmp(section, "log") && LOFT_KEY("production")) {
                loft_lg.production = loft_log_true(v, vn);
            } else if (!strcmp(section, "rotation") && LOFT_KEY("max_size_mb")) {
                u = loft_log_uint(v, vn, &ok);
                if (ok) loft_lg.max_size = u * 1024 * 1024;
            } else if (!strcmp(section, "rotation") && LOFT_KEY("daily")) {
                loft_lg.daily = loft_log_true(v, vn);
            } else if (!strcmp(section, "rotation") && LOFT_KEY("max_files")) {
                u = loft_log_uint(v, vn, &ok);
                if (ok && u <= UINT32_MAX) loft_lg.max_files = (uint32_t) u;
            } else if (!strcmp(section, "rate_limit") && LOFT_KEY("per_site")) {
                u = loft_log_uint(v, vn, &ok);
                if (ok && u <= UINT32_MAX) loft_lg.rate = (uint32_t) u;
            } else if (!strcmp(section, "levels")) {
                int s = loft_log_sev(v, vn);
                if (s >= 0) {
                    loft_lg.levels = (loft_log_level *) realloc(loft_lg.levels, (loft_lg.nlevels + 1) * sizeof *loft_lg.levels);
                    if (!loft_lg.levels) { fputs("loft: out of memory\n", stderr); abort(); }
                    loft_lg.levels[loft_lg.nlevels].key = loft_log_dup(k, kn);
                    loft_lg.levels[loft_lg.nlevels].sev = s;
                    loft_lg.nlevels++;
                }
            }
#undef LOFT_KEY
        }
    }
}

static char *loft_log_read(const char *p) {
    FILE *f = fopen(p, "rb");
    loft_text t = { 0, 0, 0 };
    char buf[4096];
    size_t n;
    if (!f) return 0;
    while ((n = fread(buf, 1, sizeof buf, f)) > 0) loft_push(&t, buf, n);
    fclose(f);
    loft_push(&t, "", 1);
    return t.p;
}

/* `source_base_for`: the nearest directory above the program holding a `loft.toml`, else the
 * program's own directory, canonical where the host can say. */
static char *loft_log_source_base(void) {
    char *dir;
#if defined(LOFT_POSIX_HOST)
    char *real = realpath(*loft_anchor ? loft_anchor : ".", 0);
    dir = real ? real : loft_log_dup(loft_anchor, strlen(loft_anchor));
#else
    dir = loft_log_dup(loft_anchor, strlen(loft_anchor));
#endif
    {
        char *cur = loft_log_dup(dir, strlen(dir));
        for (;;) {
            char *toml = loft_log_join(cur, "loft.toml");
            int found = loft_log_exists(toml);
            char *up;
            free(toml);
            if (found) { free(dir); return cur; }
            up = loft_log_dirname(cur);
            if (!strcmp(up, cur) || !strcmp(up, ".")) { free(up); break; }
            free(cur);
            cur = up;
        }
        free(cur);
    }
    return dir;
}

static void loft_log_init(void) {
    const char *env = getenv("LOFT_LOG_CONF");
    char *text, *conf_dir;
    if (loft_lg.ready) return;
    loft_lg.ready = 1;
    if (env && *env) {
        loft_lg.conf = loft_log_dup(env, strlen(env));
    } else {
        char *dot = loft_log_join(loft_anchor, ".loft/log.conf");
        if (loft_log_exists(dot)) loft_lg.conf = dot;
        else { free(dot); loft_lg.conf = loft_log_join(loft_anchor, "log.conf"); }
    }
    text = loft_log_read(loft_lg.conf);
    /* With a config, a log without `file` is `log.txt` (the default config's, relative to the
     * working directory); without one, `.loft/log.txt` beside the program. */
    loft_lg.path = text ? loft_log_dup("log.txt", 7) : loft_log_join(loft_anchor, ".loft/log.txt");
    conf_dir = loft_log_dirname(loft_lg.conf);
    loft_log_configure(text ? text : "", conf_dir);
    free(conf_dir);
    loft_lg.conf_mtime = text ? loft_log_mtime(loft_lg.conf) : -1;
    if (!text) { free(loft_lg.conf); loft_lg.conf = 0; }
    free(text);
    loft_lg.base = loft_log_source_base();
    loft_lg.checked_us = loft_now_us();
}

/* The UTC time as `YYYY-MM-DDTHH:MM:SS.mmmZ` (`utc_timestamp`), and today's date as
 * year * 10000 + month * 100 + day (`today_ymd`). */
static uint32_t loft_log_clock(char *out) {
    int64_t ms;
#if defined(LOFT_POSIX)
    struct timespec t;
    clock_gettime(CLOCK_REALTIME, &t);
    ms = (int64_t) t.tv_sec * 1000 + t.tv_nsec / 1000000;
#else
    ms = (int64_t) time(0) * 1000;
#endif
    {
        int64_t secs = ms / 1000, rem = secs % 86400;
        int64_t z = secs / 86400 + 719468;
        int64_t era = (z >= 0 ? z : z - 146096) / 146097;
        uint64_t doe = (uint64_t) (z - era * 146097);
        uint64_t yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        int64_t y = (int64_t) yoe + era * 400;
        uint64_t doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        uint64_t mp = (5 * doy + 2) / 153;
        uint64_t d = doy - (153 * mp + 2) / 5 + 1;
        uint64_t m = mp < 10 ? mp + 3 : mp - 9;
        if (m <= 2) y++;
        if (out)
            sprintf(out, "%04u-%02u-%02uT%02u:%02u:%02u.%03uZ", (unsigned) y, (unsigned) m, (unsigned) d,
                    (unsigned) (rem / 3600), (unsigned) (rem % 3600 / 60), (unsigned) (rem % 60), (unsigned) (ms % 1000));
        return (uint32_t) y * 10000 + (uint32_t) m * 100 + (uint32_t) d;
    }
}

static void loft_log_open(void) {
    char *dir = loft_log_dirname(loft_lg.path);
#if defined(LOFT_POSIX)
    /* `create_dir_all` of the log's directory, one level at a time. */
    {
        size_t i, n = strlen(dir);
        for (i = 1; i <= n; i++) {
            if (dir[i] == '/' || dir[i] == 0) {
                char c = dir[i];
                dir[i] = 0;
                mkdir(dir, 0777);
                dir[i] = c;
            }
        }
    }
#endif
    free(dir);
    loft_lg.f = fopen(loft_lg.path, "ab");
    if (loft_lg.f) {
        long at;
        fseek(loft_lg.f, 0, SEEK_END);
        at = ftell(loft_lg.f);
        loft_lg.size = at > 0 ? (uint64_t) at : 0;
        loft_lg.ymd = loft_log_clock(0);
    }
}

/* `rotate`: the oldest archive deleted, each `<stem>.<i><ext>` moved up one, the log itself to
 * `<stem>.1<ext>`, and a fresh log opened. */
static void loft_log_rotate(void) {
    char *dir = loft_log_dirname(loft_lg.path);
    const char *name = loft_lg.path + strlen(dir) + (strcmp(dir, ".") ? 1 : 0);
    const char *dot = strrchr(name, '.');
    size_t stem_n = dot && dot != name ? (size_t) (dot - name) : strlen(name);
    char *stem = loft_log_dup(name, stem_n);
    const char *ext = dot && dot != name ? dot : "";
    char a[4096], b[4096];
    uint32_t i, max = loft_lg.max_files;
    if (loft_lg.f) fclose(loft_lg.f);
    loft_lg.f = 0;
    if (strlen(dir) + strlen(name) + 32 < sizeof a) {
        if (max > 0) {
            snprintf(a, sizeof a, "%s/%s.%u.%s", dir, stem, (unsigned) (max - 1), *ext ? ext + 1 : "");
            remove(a);
        }
        for (i = max > 1 ? max - 2 : 0; max > 1 && i >= 1; i--) {
            snprintf(a, sizeof a, "%s/%s.%u%s", dir, stem, (unsigned) i, ext);
            snprintf(b, sizeof b, "%s/%s.%u%s", dir, stem, (unsigned) (i + 1), ext);
            rename(a, b);
        }
        snprintf(a, sizeof a, "%s/%s.1%s", dir, stem, ext);
        rename(loft_lg.path, a);
    }
    free(stem);
    free(dir);
    loft_lg.size = 0;
    loft_log_open();
}

/* `check_reload`: at most every five seconds, a changed config is read again; a new log path
 * reopens the log. */
static void loft_log_reload(void) {
    int64_t now = loft_now_us(), m;
    char *text, *old, *conf_dir;
    if (now - loft_lg.checked_us < 5000000) return;
    loft_lg.checked_us = now;
    if (!loft_lg.conf) return;
    m = loft_log_mtime(loft_lg.conf);
    if (m == loft_lg.conf_mtime) return;
    loft_lg.conf_mtime = m;
    text = loft_log_read(loft_lg.conf);
    if (!text) return;
    old = loft_log_dup(loft_lg.path, strlen(loft_lg.path));
    free(loft_lg.path);
    loft_lg.path = loft_log_dup("log.txt", 7);
    conf_dir = loft_log_dirname(loft_lg.conf);
    loft_log_configure(text, conf_dir);
    free(conf_dir);
    free(text);
    if (strcmp(old, loft_lg.path) && loft_lg.f) {
        fclose(loft_lg.f);
        loft_lg.f = 0;
        loft_lg.size = 0;
        loft_log_open();
    }
    free(old);
}

static void loft_log_write(const char *s, size_t n) {
    if (!loft_lg.f) return;
    fwrite(s, 1, n, loft_lg.f);
    fflush(loft_lg.f);
    loft_lg.size += n;
}

static const char *const loft_sev_names[] = { "INFO ", "WARN ", "ERROR", "FATAL" };

/* `Logger::log`: the file shown relative to the source base, filtered by its level, rate
 * limited per file and line, rotated by day and by size, written as one line. */
static void loft_log_record(int sev, loft_str message, loft_str file, int64_t line) {
    char *shown, ts[40];
    const char *base, *rel;
    size_t i, bn;
    int level;
    uint32_t today;
    loft_text out = { 0, 0, 0 };
    loft_log_init();
    loft_log_reload();
    shown = loft_log_dup(file.p, file.len);
    base = loft_lg.base;
    bn = base ? strlen(base) : 0;
    rel = shown;
    if (bn && strncmp(shown, base, bn) == 0) {
        rel = shown + bn;
        while (*rel == '/' || *rel == '\\') rel++;
    }
#if defined(_WIN32)
    for (char *c = (char *) rel; *c; c++) if (*c == '\\') *c = '/';
#endif
    level = loft_lg.level;
    {
        const char *slash = strrchr(rel, '/');
        const char *bname = slash ? slash + 1 : rel;
        int found = 0;
        for (i = 0; i < loft_lg.nlevels; i++)
            if (!strcmp(loft_lg.levels[i].key, bname)) { level = loft_lg.levels[i].sev; found = 1; break; }
        for (i = 0; !found && i < loft_lg.nlevels; i++) {
            size_t kn = strlen(loft_lg.levels[i].key);
            if (kn && loft_lg.levels[i].key[kn - 1] == '/' && !strncmp(rel, loft_lg.levels[i].key, kn)) {
                level = loft_lg.levels[i].sev;
                break;
            }
        }
    }
    if (sev < level) { free(shown); return; }
    if (loft_lg.rate > 0) {
        int64_t now = loft_now_us();
        loft_log_rate *e = 0;
        for (i = 0; i < loft_lg.nrates; i++)
            if (loft_lg.rates[i].line == line && !strcmp(loft_lg.rates[i].file, rel)) { e = &loft_lg.rates[i]; break; }
        if (!e) {
            loft_lg.rates = (loft_log_rate *) realloc(loft_lg.rates, (loft_lg.nrates + 1) * sizeof *loft_lg.rates);
            if (!loft_lg.rates) { fputs("loft: out of memory\n", stderr); abort(); }
            e = &loft_lg.rates[loft_lg.nrates++];
            e->file = loft_log_dup(rel, strlen(rel));
            e->line = line;
            e->window_us = now;
            e->count = 0;
            e->suppressed = 0;
        }
        if (now - e->window_us > 60000000) {
            if (e->suppressed > 0) {
                char head[96];
                int hn;
                if (!loft_lg.f) loft_log_open();
                loft_log_clock(ts);
                hn = sprintf(head, "%s %s  ", ts, loft_sev_names[sev]);
                out.len = 0;
                loft_push(&out, head, (size_t) hn);
                loft_push(&out, rel, strlen(rel));
                hn = sprintf(head, ":%lld  [suppressed %u identical messages in the last 60s]\n", (long long) line, (unsigned) e->suppressed);
                loft_push(&out, head, (size_t) hn);
                loft_log_write(out.p, out.len);
            }
            e->count = 0;
            e->suppressed = 0;
            e->window_us = now;
        }
        if (e->count >= loft_lg.rate) { e->suppressed++; free(out.p); free(shown); return; }
        e->count++;
    }
    if (!loft_lg.f) loft_log_open();
    today = loft_log_clock(ts);
    if (loft_lg.daily && loft_lg.ymd != 0 && today != loft_lg.ymd) loft_log_rotate();
    loft_lg.ymd = today;
    if (loft_lg.max_size > 0 && loft_lg.size >= loft_lg.max_size) loft_log_rotate();
    {
        char head[96];
        int hn = sprintf(head, "%s %s  ", ts, loft_sev_names[sev]);
        out.len = 0;
        loft_push(&out, head, (size_t) hn);
        loft_push(&out, rel, strlen(rel));
        hn = sprintf(head, ":%lld  ", (long long) line);
        loft_push(&out, head, (size_t) hn);
        loft_push(&out, message.p, message.len);
        loft_push(&out, "\n", 1);
        loft_log_write(out.p, out.len);
    }
    free(out.p);
    free(shown);
}

static void loft_log_info(loft_str m, loft_str f, int64_t l) { loft_log_record(LOFT_SEV_INFO, m, f, l); }
static void loft_log_warn(loft_str m, loft_str f, int64_t l) { loft_log_record(LOFT_SEV_WARN, m, f, l); }
static void loft_log_error(loft_str m, loft_str f, int64_t l) { loft_log_record(LOFT_SEV_ERROR, m, f, l); }
static void loft_log_fatal(loft_str m, loft_str f, int64_t l) { loft_log_record(LOFT_SEV_FATAL, m, f, l); }

/* A halting fault in a production run (`production_logged`): logged as
 * `[<label>] <prefix><message>` at its kind's severity, and the program carries on; the run
 * exits 1 at its end.  0 outside production, where the caller halts. */
static int loft_log_production_fault(int sev, const char *label, const char *prefix, loft_str message,
                                     loft_str file, int64_t line) {
    loft_text m = { 0, 0, 0 };
    loft_log_init();
    if (!loft_lg.production) return 0;
    loft_push(&m, "[", 1);
    loft_push(&m, label, strlen(label));
    loft_push(&m, "] ", 2);
    loft_push(&m, prefix, strlen(prefix));
    loft_push(&m, message.p, message.len);
    loft_log_record(sev, loft_lit(m.p, m.len), file, line);
    free(m.p);
    loft_had_fatal = 1;
    return 1;
}
