/* ── lib/process: running a program (src/process_run.rs, src/platform_process.rs) ──────────
 * The one native behind `lib/process`, under its rule that **a stream is never left without a
 * reader**: every pipe a child holds is drained while it runs (one `poll` over all of them, where
 * the interpreter gives each a thread), so no pipe can fill while another is waited on.  The argv
 * arrives as one text, each word length-prefixed (`3:git3:log`); a text that starts with `@<op>:`
 * names one of the library's other operations, its words encoded the same way.  A child leads a
 * process group of its own, so a stop, a timeout and (Linux, `PR_SET_PDEATHSIG`) this program's
 * end reach everything it started; the terminal's signals and `SIGTERM` are passed on to every
 * group this program holds.  `lib/git`'s native (`git_query`) runs git through the same
 * spawn.  Compiled in for a unit that reaches either (LOFT_PROCESS). */
#ifdef LOFT_PROCESS
#include <ctype.h>
#if defined(LOFT_POSIX_HOST)
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>
#include <dirent.h>
#if defined(__linux__)
#include <sys/prctl.h>
#endif
#endif

static const char loft_proc_malformed[] =
    "process: a malformed argv reached the binary — the library and the binary disagree";
static const char loft_proc_empty[] = "an empty command: there is no program to run";

/* The words of a length-prefixed argv, each a NUL-terminated copy (its length beside it); 0 when
 * `enc` is not one. */
typedef struct { char **v; size_t *len; size_t n; } loft_words;

static void loft_words_free(loft_words *w) {
    for (size_t i = 0; i < w->n; i++) free(w->v[i]);
    free(w->v);
    free(w->len);
    w->v = 0;
    w->len = 0;
    w->n = 0;
}

static int loft_words_decode(const char *p, size_t len, loft_words *w) {
    w->v = 0;
    w->len = 0;
    w->n = 0;
    size_t i = 0;
    while (i < len) {
        size_t n = 0, digits = 0;
        while (i < len && p[i] >= '0' && p[i] <= '9') {
            n = n * 10 + (size_t) (p[i] - '0');
            i++;
            digits++;
        }
        if (!digits || i >= len || p[i] != ':' || n > len - i - 1) {
            loft_words_free(w);
            return 0;
        }
        i++;
        w->v = (char **) realloc(w->v, (w->n + 2) * sizeof *w->v);
        w->len = (size_t *) realloc(w->len, (w->n + 1) * sizeof *w->len);
        char *word = (char *) malloc(n + 1);
        if (!w->v || !w->len || !word) { fputs("loft: out of memory\n", stderr); abort(); }
        memcpy(word, p + i, n);
        word[n] = 0;
        w->len[w->n] = n;
        w->v[w->n++] = word;
        w->v[w->n] = 0;
        i += n;
    }
    return 1;
}

/* `String::from_utf8_lossy` over bytes that may hold a NUL: an invalid byte is U+FFFD. */
static void loft_push_lossy(loft_text *t, const char *c, size_t n) {
    const unsigned char *p = (const unsigned char *) c;
    if (loft_utf8_valid(p, n)) {
        loft_push(t, c, n);
        return;
    }
    for (size_t i = 0; i < n;) {
        size_t len = p[i] < 0x80 ? 1 : (p[i] >= 0xC2 && p[i] <= 0xDF) ? 2
                   : (p[i] >= 0xE0 && p[i] <= 0xEF) ? 3 : (p[i] >= 0xF0 && p[i] <= 0xF4) ? 4 : 0;
        if (len && i + len <= n && loft_utf8_valid(p + i, len)) {
            loft_push(t, c + i, len);
            i += len;
        } else {
            loft_push(t, "\xEF\xBF\xBD", 3);
            i++;
        }
    }
}

/* One answer of the native: the integer the library reads, and its two texts. */
typedef struct { int64_t code; loft_text out, err; } loft_proc_answer;

static void loft_proc_say(loft_proc_answer *a, int64_t code, const char *out, const char *err) {
    a->code = code;
    if (out) loft_push_cstr(&a->out, out);
    if (err) loft_push_cstr(&a->err, err);
}

#if defined(LOFT_POSIX_HOST)
/* ── The child: spawn, ask, stop (platform_process.rs) ── */

/* The owned groups a forwarded signal reaches, and what each forwarded signal did before. */
static volatile sig_atomic_t loft_proc_groups[256];
static const int loft_proc_forwarded[4] = { SIGINT, SIGQUIT, SIGHUP, SIGTERM };
static struct sigaction loft_proc_previous[4];
static uint8_t loft_proc_was_ignored[4];

static void loft_proc_forward(int sig) {
    for (size_t i = 0; i < 256; i++)
        if (loft_proc_groups[i] > 0) killpg((pid_t) loft_proc_groups[i], sig);
    for (int i = 0; i < 4; i++) {
        if (loft_proc_forwarded[i] != sig) continue;
        if (loft_proc_previous[i].sa_handler == SIG_DFL) {
            signal(sig, SIG_DFL);
            raise(sig);
        } else if (loft_proc_previous[i].sa_handler != SIG_IGN && loft_proc_previous[i].sa_handler != loft_proc_forward) {
            loft_proc_previous[i].sa_handler(sig);
        }
    }
}

static void loft_proc_track(pid_t group, int on) {
    static int installed = 0;
    if (!installed) {
        installed = 1;
        for (int i = 0; i < 4; i++) {
            sigaction(loft_proc_forwarded[i], 0, &loft_proc_previous[i]);
            /* An ignored signal is ignored by the children too: there is nothing to pass on. */
            loft_proc_was_ignored[i] = loft_proc_previous[i].sa_handler == SIG_IGN;
            if (loft_proc_was_ignored[i]) continue;
            struct sigaction act;
            memset(&act, 0, sizeof act);
            act.sa_handler = loft_proc_forward;
            act.sa_flags = SA_RESTART;
            sigemptyset(&act.sa_mask);
            sigaction(loft_proc_forwarded[i], &act, 0);
        }
    }
    for (size_t i = 0; i < 256; i++) {
        if (on && loft_proc_groups[i] == 0) {
            loft_proc_groups[i] = group;
            return;
        }
        if (!on && loft_proc_groups[i] == group) loft_proc_groups[i] = 0;
    }
}

/* A started child: its id (which names its group) and the pipes this program holds, -1 for one
 * it does not. */
typedef struct { pid_t pid; int in, out, err; } loft_child;

static void loft_proc_cloexec(int fd) {
    fcntl(fd, F_SETFD, fcntl(fd, F_GETFD) | FD_CLOEXEC);
}

/* Start `argv` in a group of its own: stdin a pipe when `pin` (else empty), stdout and stderr
 * pipes when `pout`/`perr` (else this program's own).  0 and `why` the OS's error, said as
 * `{program}: {error} (os error N)`, when it cannot be started. */
static int loft_spawn(char **argv, const size_t *lens, size_t n, int pin, int pout, int perr, loft_child *c,
                      loft_text *why) {
    for (size_t i = 0; i < n; i++) {
        if (strlen(argv[i]) != lens[i]) {
            loft_push_cstr(why, argv[0]);
            loft_push_cstr(why, ": nul byte found in provided data");
            return 0;
        }
    }
    int in[2] = { -1, -1 }, out[2] = { -1, -1 }, err[2] = { -1, -1 }, report[2] = { -1, -1 };
    if ((pin && pipe(in) != 0) || (pout && pipe(out) != 0) || (perr && pipe(err) != 0) || pipe(report) != 0) {
        int e = errno;
        loft_push_cstr(why, argv[0]);
        loft_push_cstr(why, ": ");
        loft_push_cstr(why, strerror(e));
        return 0;
    }
    int ends[8] = { in[0], in[1], out[0], out[1], err[0], err[1], report[0], report[1] };
    for (int i = 0; i < 8; i++)
        if (ends[i] >= 0) loft_proc_cloexec(ends[i]);
    /* Whatever this program wrote reaches its stream before the child's output does. */
    fflush(0);
    pid_t parent = getpid();
    pid_t pid = fork();
    if (pid == 0) {
        setpgid(0, 0);
#if defined(__linux__)
        prctl(PR_SET_PDEATHSIG, SIGTERM);
        if (getppid() != parent) _exit(0);
#else
        (void) parent;
#endif
        /* The dispositions this program changed are not the child's (`std` resets SIGPIPE). */
        signal(SIGPIPE, SIG_DFL);
        sigset_t none;
        sigemptyset(&none);
        sigprocmask(SIG_SETMASK, &none, 0);
        int devnull = pin ? -1 : open("/dev/null", O_RDONLY);
        dup2(pin ? in[0] : devnull, 0);
        if (devnull > 2) close(devnull);
        if (pout) dup2(out[1], 1);
        if (perr) dup2(err[1], 2);
        execvp(argv[0], argv);
        int e = errno;
        ssize_t w = write(report[1], &e, sizeof e);
        (void) w;
        _exit(127);
    }
    int e = errno;
    close(report[1]);
    if (pin) close(in[0]);
    if (pout) close(out[1]);
    if (perr) close(err[1]);
    int child_errno = 0;
    ssize_t got = pid > 0 ? read(report[0], &child_errno, sizeof child_errno) : 0;
    close(report[0]);
    if (pid < 0 || got == (ssize_t) sizeof child_errno) {
        if (pid > 0) {
            int status;
            waitpid(pid, &status, 0);
            e = child_errno;
        }
        if (pin) close(in[1]);
        if (pout) close(out[0]);
        if (perr) close(err[0]);
        char m[64];
        snprintf(m, sizeof m, " (os error %d)", e);
        loft_push_cstr(why, argv[0]);
        loft_push_cstr(why, ": ");
        loft_push_cstr(why, strerror(e));
        loft_push_cstr(why, m);
        return 0;
    }
    setpgid(pid, pid);
    loft_proc_track(pid, 1);
    c->pid = pid;
    c->in = pin ? in[1] : -1;
    c->out = pout ? out[0] : -1;
    c->err = perr ? err[0] : -1;
    if (c->in >= 0) fcntl(c->in, F_SETFL, fcntl(c->in, F_GETFL) | O_NONBLOCK);
    return 1;
}

/* Has the child ended?  Asked without reaping it (`WNOWAIT`), so its id stays its own and a stop
 * of its group reaches nothing else; `block` waits.  A child that cannot be asked reads as ended. */
static int loft_child_exited(pid_t pid, int block) {
    for (;;) {
        siginfo_t info;
        memset(&info, 0, sizeof info);
        int r = waitid(P_PID, (id_t) pid, &info, WEXITED | WNOWAIT | (block ? 0 : WNOHANG));
        if (r == 0) return info.si_pid != 0;
        if (errno != EINTR) return 1;
    }
}

/* `exit_code`: the program's own code, `128 + n` for signal `n`, -1 when it cannot be told. */
static int64_t loft_child_reap(pid_t pid) {
    loft_proc_track(pid, 0);
    int status = 0;
    while (waitpid(pid, &status, 0) < 0) {
        if (errno != EINTR) return -1;
    }
    if (WIFEXITED(status)) return WEXITSTATUS(status);
    if (WIFSIGNALED(status)) return 128 + WTERMSIG(status);
    return -1;
}

/* Wait up to `ms` for the child to end, backing off from 1 ms to 20: 1 when it did. */
static int loft_child_wait(pid_t pid, int64_t ms) {
    int64_t until = loft_now_us() + ms * 1000, step = 1000;
    for (;;) {
        if (loft_child_exited(pid, 0)) return 1;
        int64_t now = loft_now_us();
        if (now >= until) return 0;
        int64_t nap = step < until - now ? step : until - now;
        struct timespec ts = { (time_t) (nap / 1000000), (long) (nap % 1000000) * 1000 };
        nanosleep(&ts, 0);
        step = step * 2 < 20000 ? step * 2 : 20000;
    }
}

/* `stop_tree`: the group is asked to end (`SIGTERM`), given two seconds, then killed, and the
 * child reaped — so its code usually reads as ended by `SIGTERM`. */
static int64_t loft_child_stop(pid_t pid) {
    killpg(pid, SIGTERM);
    int64_t until = loft_now_us() + 2000000;
    while (loft_now_us() < until && !loft_child_exited(pid, 0)) {
        struct timespec ts = { 0, 5000000 };
        nanosleep(&ts, 0);
    }
    killpg(pid, SIGKILL);
    kill(pid, SIGKILL);
    return loft_child_reap(pid);
}

/* One `poll` round over the child's pipes: each output read into its text (EOF closes it), the
 * input fed as far as the pipe takes it (closed once all of it is in, or the child closed its
 * end).  `ms` -1 blocks.  Answers whether any pipe is still open. */
static int loft_child_pump(loft_child *c, loft_text *out, loft_text *err, const char *in, size_t inlen,
                           size_t *fed, int ms) {
    struct pollfd fds[3];
    int nf = 0, io = -1, ie = -1, ii = -1;
    if (c->out >= 0) { fds[nf].fd = c->out; fds[nf].events = POLLIN; io = nf++; }
    if (c->err >= 0) { fds[nf].fd = c->err; fds[nf].events = POLLIN; ie = nf++; }
    if (c->in >= 0) { fds[nf].fd = c->in; fds[nf].events = POLLOUT; ii = nf++; }
    if (nf == 0) return 0;
    if (poll(fds, (nfds_t) nf, ms) <= 0) return 1;
    char buf[65536];
    int *ends[2] = { &c->out, &c->err };
    loft_text *into[2] = { out, err };
    int at[2] = { io, ie };
    for (int k = 0; k < 2; k++) {
        if (at[k] < 0 || !(fds[at[k]].revents & (POLLIN | POLLHUP | POLLERR))) continue;
        ssize_t r = read(*ends[k], buf, sizeof buf);
        if (r > 0) {
            if (into[k]) loft_push(into[k], buf, (size_t) r);
        } else if (r == 0 || (errno != EINTR && errno != EAGAIN)) {
            close(*ends[k]);
            *ends[k] = -1;
        }
    }
    if (ii >= 0 && (fds[ii].revents & (POLLOUT | POLLERR | POLLHUP))) {
        ssize_t w = *fed < inlen ? write(c->in, in + *fed, inlen - *fed) : 0;
        if (w > 0) *fed += (size_t) w;
        /* A child that exits without reading its input closes the pipe: its choice, not a
         * failure of the run. */
        if (*fed >= inlen || (w < 0 && errno != EAGAIN && errno != EINTR)) {
            close(c->in);
            c->in = -1;
        }
    }
    return c->out >= 0 || c->err >= 0 || c->in >= 0;
}

/* `run_live`: run to completion with `input` on stdin, both streams collected; with `limit_ms`
 * >= 0 a child still running then is stopped with its tree and the code is -2. */
static void loft_run_live(loft_words *w, const char *in, size_t inlen, int64_t limit_ms, loft_proc_answer *a) {
    loft_child c;
    loft_text out = { 0, 0, 0 }, err = { 0, 0, 0 }, why = { 0, 0, 0 };
    if (!loft_spawn(w->v, w->len, w->n, inlen > 0, 1, 1, &c, &why)) {
        a->code = -1;
        loft_push(&a->err, why.p, why.len);
        free(why.p);
        return;
    }
    size_t fed = 0;
    int timed_out = 0;
    if (limit_ms >= 0) {
        /* The clock is watched on the child's end, not on its pipes. */
        int64_t until = loft_now_us() + limit_ms * 1000;
        for (;;) {
            if (loft_child_exited(c.pid, 0)) break;
            int64_t left = until - loft_now_us();
            if (left <= 0) {
                timed_out = 1;
                break;
            }
            int ms = (int) (left / 1000 < 20 ? left / 1000 : 20);
            if (!loft_child_pump(&c, &out, &err, in, inlen, &fed, ms)) {
                if (!loft_child_wait(c.pid, left / 1000 + 1)) timed_out = 1;
                break;
            }
        }
        a->code = timed_out ? loft_child_stop(c.pid) : loft_child_reap(c.pid);
        if (timed_out) a->code = -2;
        while (loft_child_pump(&c, &out, &err, in, inlen, &fed, -1)) {}
    } else {
        while (loft_child_pump(&c, &out, &err, in, inlen, &fed, -1)) {}
        a->code = loft_child_reap(c.pid);
    }
    loft_push_lossy(&a->out, out.p, out.len);
    loft_push_lossy(&a->err, err.p, err.len);
    free(out.p);
    free(err.p);
}

/* ── Recordings (process_run.rs `recording`): LOFT_RUN_RECORD writes every collecting run as
 * `<dir>/<NNN>-<program>/{argv,stdin,stdout,stderr,code}`; LOFT_RUN_REPLAY answers a run from
 * the first unused entry whose `argv` and `stdin` match (the same call twice walks its entries
 * and stays on the last), and a run with none answers -1 and says so. ── */

static const char *loft_rec_dir(const char *var) {
    const char *d = getenv(var);
    return d && *d ? d : 0;
}

/* The `argv` file's bytes: one word per line; 0 for a word a line cannot hold. */
static int loft_rec_key(loft_words *w, loft_text *key) {
    for (size_t i = 0; i < w->n; i++) {
        if (memchr(w->v[i], '\n', w->len[i])) return 0;
        loft_push(key, w->v[i], w->len[i]);
        loft_push(key, "\n", 1);
    }
    return 1;
}

static int loft_rec_cmp(const void *a, const void *b) {
    return strcmp(*(char *const *) a, *(char *const *) b);
}

/* The subdirectories of `dir`, in name order, malloc'd; their count in `*n`. */
static char **loft_rec_entries(const char *dir, size_t *n) {
    *n = 0;
    char **out = 0;
    DIR *d = opendir(dir);
    if (!d) return 0;
    struct dirent *e;
    while ((e = readdir(d)) != 0) {
        if (strcmp(e->d_name, ".") == 0 || strcmp(e->d_name, "..") == 0) continue;
        size_t len = strlen(dir) + strlen(e->d_name) + 2;
        char *p = (char *) malloc(len);
        if (!p) { fputs("loft: out of memory\n", stderr); abort(); }
        snprintf(p, len, "%s/%s", dir, e->d_name);
        struct stat st;
        if (stat(p, &st) != 0 || !S_ISDIR(st.st_mode)) {
            free(p);
            continue;
        }
        out = (char **) realloc(out, (*n + 1) * sizeof *out);
        if (!out) { fputs("loft: out of memory\n", stderr); abort(); }
        out[(*n)++] = p;
    }
    closedir(d);
    if (*n) qsort(out, *n, sizeof *out, loft_rec_cmp);
    return out;
}

/* `entry/name`'s bytes into `t`; 0 when it cannot be read. */
static int loft_rec_read(const char *entry, const char *name, loft_text *t) {
    char p[8192];
    snprintf(p, sizeof p, "%s/%s", entry, name);
    FILE *f = fopen(p, "rb");
    if (!f) return 0;
    char buf[65536];
    size_t r;
    while ((r = fread(buf, 1, sizeof buf, f)) > 0) loft_push(t, buf, r);
    fclose(f);
    return 1;
}

static void loft_rec_write(const char *entry, const char *name, const char *p, size_t n) {
    char path[8192];
    snprintf(path, sizeof path, "%s/%s", entry, name);
    FILE *f = fopen(path, "wb");
    if (!f) return;
    if (n) fwrite(p, 1, n, f);
    fclose(f);
}

static int loft_rec_replay(loft_words *w, const char *in, size_t inlen, loft_proc_answer *a) {
    const char *dir = loft_rec_dir("LOFT_RUN_REPLAY");
    if (!dir) return 0;
    static loft_text *used_key = 0;
    static size_t *used_n = 0, nused = 0;
    loft_text key = { 0, 0, 0 };
    if (!loft_rec_key(w, &key)) {
        a->code = -1;
        loft_push_cstr(&a->err, "process: no recording in ");
        loft_push_cstr(&a->err, dir);
        loft_push_cstr(&a->err, " for: a word holding a newline cannot be recorded");
        free(key.p);
        return 1;
    }
    size_t n = 0, nm = 0;
    char **all = loft_rec_entries(dir, &n);
    char **matching = (char **) malloc((n + 1) * sizeof *matching);
    if (!matching) { fputs("loft: out of memory\n", stderr); abort(); }
    for (size_t i = 0; i < n; i++) {
        loft_text argv = { 0, 0, 0 }, stdin_ = { 0, 0, 0 };
        int ok = loft_rec_read(all[i], "argv", &argv) && argv.len == key.len &&
                 (key.len == 0 || memcmp(argv.p, key.p, key.len) == 0);
        loft_rec_read(all[i], "stdin", &stdin_);
        if (ok && stdin_.len == inlen && (inlen == 0 || memcmp(stdin_.p, in, inlen) == 0)) matching[nm++] = all[i];
        free(argv.p);
        free(stdin_.p);
    }
    if (nm == 0) {
        a->code = -1;
        loft_push_cstr(&a->err, "process: no recording in ");
        loft_push_cstr(&a->err, dir);
        loft_push_cstr(&a->err, " for: ");
        for (size_t i = 0; i < w->n; i++) {
            if (i) loft_push(&a->err, " ", 1);
            loft_push(&a->err, w->v[i], w->len[i]);
        }
    } else {
        size_t slot = nused;
        for (size_t i = 0; i < nused; i++)
            if (used_key[i].len == key.len && memcmp(used_key[i].p, key.p, key.len) == 0) slot = i;
        if (slot == nused) {
            used_key = (loft_text *) realloc(used_key, (nused + 1) * sizeof *used_key);
            used_n = (size_t *) realloc(used_n, (nused + 1) * sizeof *used_n);
            if (!used_key || !used_n) { fputs("loft: out of memory\n", stderr); abort(); }
            used_key[nused] = loft_text_own(loft_view(&key));
            used_n[nused++] = 0;
        }
        size_t k = used_n[slot] < nm - 1 ? used_n[slot] : nm - 1;
        used_n[slot]++;
        loft_text code = { 0, 0, 0 };
        a->code = -1;
        if (loft_rec_read(matching[k], "code", &code)) {
            loft_push(&code, "", 1);
            char *end = 0;
            const char *s = code.p;
            while (*s == ' ' || *s == '\t' || *s == '\n' || *s == '\r') s++;
            long long v = strtoll(s, &end, 10);
            while (end && (*end == ' ' || *end == '\t' || *end == '\n' || *end == '\r')) end++;
            if (end && end != s && *end == 0) a->code = v;
        }
        free(code.p);
        loft_rec_read(matching[k], "stdout", &a->out);
        loft_rec_read(matching[k], "stderr", &a->err);
    }
    for (size_t i = 0; i < n; i++) free(all[i]);
    free(all);
    free(matching);
    free(key.p);
    return 1;
}

static void loft_rec_record(loft_words *w, const char *in, size_t inlen, const loft_proc_answer *a) {
    const char *dir = loft_rec_dir("LOFT_RUN_RECORD");
    if (!dir) return;
    loft_text key = { 0, 0, 0 };
    if (!loft_rec_key(w, &key)) {
        free(key.p);
        return;
    }
    mkdir(dir, 0777);
    size_t n = 0;
    char **all = loft_rec_entries(dir, &n);
    for (size_t i = 0; i < n; i++) free(all[i]);
    free(all);
    const char *slash = strrchr(w->v[0], '/');
    const char *program = slash ? slash + 1 : w->v[0];
    char entry[8192];
    snprintf(entry, sizeof entry, "%s/%03zu-%s", dir, n + 1, *program ? program : "program");
    mkdir(entry, 0777);
    char code[32];
    snprintf(code, sizeof code, "%lld\n", (long long) a->code);
    loft_rec_write(entry, "argv", key.p, key.len);
    loft_rec_write(entry, "stdin", in, inlen);
    loft_rec_write(entry, "stdout", a->out.p, a->out.len);
    loft_rec_write(entry, "stderr", a->err.p, a->err.len);
    loft_rec_write(entry, "code", code, strlen(code));
    free(key.p);
}

/* `run_until`: the run, answered from a recording when one is armed, and written to one. */
static void loft_run_until(loft_words *w, const char *in, size_t inlen, int64_t limit_ms, loft_proc_answer *a) {
    if (w->n == 0) {
        loft_proc_say(a, -1, 0, loft_proc_empty);
        return;
    }
    if (loft_rec_replay(w, in, inlen, a)) return;
    loft_run_live(w, in, inlen, limit_ms, a);
    loft_rec_record(w, in, inlen, a);
}

/* ── Started programs and line cursors, by the handle the library holds ── */

typedef struct { int64_t h; pid_t pid; } loft_started;
static loft_started *loft_started_tab = 0;
static size_t loft_nstarted = 0;
static int64_t loft_next_started = 1;

static loft_started *loft_started_find(int64_t h) {
    for (size_t i = 0; i < loft_nstarted; i++)
        if (loft_started_tab[i].h == h) return &loft_started_tab[i];
    return 0;
}

static void loft_started_drop(loft_started *s) {
    *s = loft_started_tab[--loft_nstarted];
}

/* A cursor over ONE stream of a running program (`lines()`): each `next` reads that pipe up to a
 * line, draining the other stream and feeding the input meanwhile, so a program that talks on
 * both can never deadlock — a consumer that stops asking only slows it. */
typedef struct {
    int64_t h;
    loft_child c;
    int err;           /* the cursor reads stderr; the collected stream is then stdout */
    loft_text line;    /* bytes of the chosen stream not yet handed over */
    loft_text other;   /* the other stream, collected */
    char *input;
    size_t inlen, fed;
    int finished;
    int64_t code;
} loft_cursor;
static loft_cursor **loft_cursors = 0;
static size_t loft_ncursors = 0;
static int64_t loft_next_cursor = 1;

static loft_cursor *loft_cursor_find(int64_t h) {
    for (size_t i = 0; i < loft_ncursors; i++)
        if (loft_cursors[i]->h == h) return loft_cursors[i];
    return 0;
}

static void loft_cursor_open(int err, loft_words *w, loft_str input, loft_proc_answer *a) {
    if (w->n == 0) {
        loft_proc_say(a, -1, 0, loft_proc_empty);
        return;
    }
    loft_cursor *k = (loft_cursor *) calloc(1, sizeof *k);
    if (!k) { fputs("loft: out of memory\n", stderr); abort(); }
    loft_text why = { 0, 0, 0 };
    if (!loft_spawn(w->v, w->len, w->n, input.len > 0, 1, 1, &k->c, &why)) {
        a->code = -1;
        loft_push(&a->err, why.p, why.len);
        free(why.p);
        free(k);
        return;
    }
    k->err = err;
    k->input = (char *) malloc(input.len + 1);
    if (!k->input) { fputs("loft: out of memory\n", stderr); abort(); }
    if (input.len) memcpy(k->input, input.p, input.len);
    k->inlen = input.len;
    k->h = loft_next_cursor++;
    loft_cursors = (loft_cursor **) realloc(loft_cursors, (loft_ncursors + 1) * sizeof *loft_cursors);
    if (!loft_cursors) { fputs("loft: out of memory\n", stderr); abort(); }
    loft_cursors[loft_ncursors++] = k;
    a->code = k->h;
}

/* One round over the cursor's pipes: the chosen stream into `line`, the other into `other`. */
static int loft_cursor_pump(loft_cursor *k, int ms) {
    return k->err ? loft_child_pump(&k->c, &k->other, &k->line, k->input, k->inlen, &k->fed, ms)
                  : loft_child_pump(&k->c, &k->line, &k->other, k->input, k->inlen, &k->fed, ms);
}

/* Wait for the program to end and settle its streams, once: nobody reads the chosen stream any
 * more (its pipe closes), the other is drained to its end; `stop` ends the tree first. */
static void loft_cursor_finish(loft_cursor *k, int stop) {
    if (k->finished) return;
    k->finished = 1;
    int *chosen = k->err ? &k->c.err : &k->c.out;
    if (*chosen >= 0) {
        close(*chosen);
        *chosen = -1;
    }
    if (stop) k->code = loft_child_stop(k->c.pid);
    while (loft_cursor_pump(k, -1)) {}
    if (!stop) k->code = loft_child_reap(k->c.pid);
}

static void loft_cursor_next(int64_t h, loft_proc_answer *a) {
    loft_cursor *k = loft_cursor_find(h);
    if (!k) {
        a->code = -1;
        return;
    }
    for (;;) {
        char *nl = k->line.len ? (char *) memchr(k->line.p, '\n', k->line.len) : 0;
        int open = (k->err ? k->c.err : k->c.out) >= 0 && !k->finished;
        if (nl || (!open && k->line.len)) {
            size_t n = nl ? (size_t) (nl - k->line.p) : k->line.len;
            size_t take = nl ? n + 1 : n;
            if (nl && n > 0 && k->line.p[n - 1] == '\r') n--;
            loft_push_lossy(&a->out, k->line.p, n);
            memmove(k->line.p, k->line.p + take, k->line.len - take);
            k->line.len -= take;
            a->code = 1;
            return;
        }
        if (!open) {
            loft_cursor_finish(k, 0);
            a->code = 0;
            return;
        }
        loft_cursor_pump(k, -1);
    }
}

static void loft_cursor_done(int64_t h, int stop, loft_proc_answer *a) {
    size_t at = loft_ncursors;
    for (size_t i = 0; i < loft_ncursors; i++)
        if (loft_cursors[i]->h == h) at = i;
    if (at == loft_ncursors) {
        a->code = -1;
        return;
    }
    loft_cursor *k = loft_cursors[at];
    loft_cursors[at] = loft_cursors[--loft_ncursors];
    loft_cursor_finish(k, stop);
    a->code = k->code;
    loft_push_lossy(k->err ? &a->out : &a->err, k->other.p, k->other.len);
    free(k->line.p);
    free(k->other.p);
    free(k->input);
    free(k);
}

/* `start()`: the program runs beside this one, its input empty and its output this program's. */
static void loft_proc_start(loft_words *w, loft_proc_answer *a) {
    if (w->n == 0) {
        loft_proc_say(a, -1, 0, loft_proc_empty);
        return;
    }
    loft_child c;
    loft_text why = { 0, 0, 0 };
    if (!loft_spawn(w->v, w->len, w->n, 0, 0, 0, &c, &why)) {
        a->code = -1;
        loft_push(&a->err, why.p, why.len);
        free(why.p);
        return;
    }
    loft_started_tab = (loft_started *) realloc(loft_started_tab, (loft_nstarted + 1) * sizeof *loft_started_tab);
    if (!loft_started_tab) { fputs("loft: out of memory\n", stderr); abort(); }
    loft_started_tab[loft_nstarted].h = loft_next_started;
    loft_started_tab[loft_nstarted++].pid = c.pid;
    a->code = loft_next_started++;
}
#endif

#if defined(LOFT_POSIX_HOST)
/* ── lib/git (src/git_query.rs): a closed vocabulary of questions, each one's argv built here —
 * what the caller supplies is values (a ref, a path, a count), never options or a subcommand. ── */

/* Run `git [-C dir] <words>` to completion, stdin empty, stdout collected: 1 and its exit code
 * (-2 for one a signal ended), or 0 and why git could not be run. */
static int loft_git_run(const char *dir, char **words, size_t nwords, loft_text *out, int64_t *code, loft_text *why) {
    char *argv[16];
    size_t lens[16], n = 0;
    argv[n++] = (char *) "git";
    if (*dir) {
        argv[n++] = (char *) "-C";
        argv[n++] = (char *) dir;
    }
    for (size_t i = 0; i < nwords && n < 15; i++) argv[n++] = words[i];
    argv[n] = 0;
    for (size_t i = 0; i < n; i++) lens[i] = strlen(argv[i]);
    loft_child c;
    loft_text err = { 0, 0, 0 };
    if (!loft_spawn(argv, lens, n, 0, 1, 1, &c, why)) return 0;
    size_t fed = 0;
    while (loft_child_pump(&c, out, &err, "", 0, &fed, -1)) {}
    free(err.p);
    loft_proc_track(c.pid, 0);
    int status = 0;
    while (waitpid(c.pid, &status, 0) < 0 && errno == EINTR) {}
    *code = WIFEXITED(status) ? WEXITSTATUS(status) : -2;
    return 1;
}

/* `git_query(kind, a, b, n, dir, out) -> integer`: the exit status and the output; -1 when git
 * could not be asked at all, `out` then saying why (loft#1061: a failed query outside a
 * repository is not an empty answer). */
static int64_t loft_git_query(int64_t kind, loft_str a, loft_str b, int64_t n, loft_str dir, loft_text *out) {
    out->len = 0;
    char *sa = loft_cstr_own(a), *sb = loft_cstr_own(b), *sd = loft_cstr_own(dir);
    char count[32], range[8192], pretty[64];
    snprintf(count, sizeof count, "-%lld", (long long) (n < 1 ? 1 : n > 100000 ? 100000 : n));
    snprintf(range, sizeof range, "%s...HEAD", sa);
    char *w[8];
    size_t nw = 0;
#define LOFT_W(x) (w[nw++] = (char *) (x))
    switch (kind) {
    case 0: LOFT_W("rev-parse"); LOFT_W("--abbrev-ref"); LOFT_W("HEAD"); break;
    case 1: LOFT_W("rev-parse"); LOFT_W("--short"); LOFT_W("HEAD"); break;
    case 2: LOFT_W("log"); LOFT_W("-1"); LOFT_W("--pretty=%s"); break;
    case 3: LOFT_W("rev-parse"); LOFT_W("--verify"); LOFT_W("--quiet"); LOFT_W(sa); break;
    case 4: LOFT_W("rev-list"); LOFT_W("--left-right"); LOFT_W("--count"); LOFT_W(range); break;
    case 5: LOFT_W("diff"); LOFT_W("--name-status"); LOFT_W(range); break;
    case 6:
        snprintf(pretty, sizeof pretty, "--pretty=%%h%%x1f%%ad%%x1f%%s");
        LOFT_W("log"); LOFT_W(count); LOFT_W(pretty); LOFT_W("--date=short");
        break;
    case 7: LOFT_W("status"); LOFT_W("--short"); break;
    case 8: LOFT_W("diff"); LOFT_W(range); LOFT_W("--"); LOFT_W(sb); break;
    case 9: LOFT_W("show"); LOFT_W(sa); break;
    case 10: LOFT_W("show"); LOFT_W(sa); LOFT_W("--numstat"); LOFT_W("--pretty=format:"); break;
    case 11: LOFT_W("log"); LOFT_W(count); LOFT_W("--pretty=%h"); break;
    case 12: LOFT_W("diff"); LOFT_W("--name-only"); LOFT_W(range); break;
    case 13: LOFT_W("ls-files"); LOFT_W("--cached"); LOFT_W("--others"); LOFT_W("--exclude-standard"); break;
    default: break;
    }
#undef LOFT_W
    int64_t code = -1;
    char m[256];
    if (nw == 0) {
        snprintf(m, sizeof m, "this loft does not know git query %lld — the library is newer than the binary",
                 (long long) kind);
        loft_push_cstr(out, m);
    } else if (a.len && a.p[0] == '-') {
        /* A git ref cannot begin with `-`: refusing one closes the option position. */
        loft_push_cstr(out, "'");
        loft_push(out, a.p, a.len);
        loft_push_cstr(out, "' cannot be a git ref — a ref may not begin with '-'");
    } else {
        loft_text got = { 0, 0, 0 }, why = { 0, 0, 0 };
        if (!loft_git_run(sd, w, nw, &got, &code, &why)) {
            code = -1;
            loft_push_cstr(out, "cannot run git: ");
            /* The spawn says `git: <error>`; the query says it after its own words. */
            size_t skip = why.len >= 5 && memcmp(why.p, "git: ", 5) == 0 ? 5 : 0;
            loft_push(out, why.p + skip, why.len - skip);
        } else if (code == 0) {
            loft_push_lossy(out, got.p, got.len);
        } else {
            /* Asked in an exit status (`rev-parse --git-dir`), never read from git's prose. */
            char *probe[2] = { (char *) "rev-parse", (char *) "--git-dir" };
            loft_text o = { 0, 0, 0 }, y = { 0, 0, 0 };
            int64_t pc = -1;
            if (loft_git_run(sd, probe, 2, &o, &pc, &y) && pc == 0) {
                loft_push_lossy(out, got.p, got.len);
            } else {
                code = -1;
                loft_push_cstr(out, "not a git repository: ");
                if (*sd) {
                    loft_push_cstr(out, sd);
                } else {
                    char cwd[4096];
                    loft_push_cstr(out, getcwd(cwd, sizeof cwd) ? cwd : "the working directory");
                }
            }
            free(o.p);
            free(y.p);
        }
        free(got.p);
        free(why.p);
    }
    free(sa);
    free(sb);
    free(sd);
    return code;
}
#else
static int64_t loft_git_query(int64_t kind, loft_str a, loft_str b, int64_t n, loft_str dir, loft_text *out) {
    (void) kind; (void) a; (void) b; (void) n; (void) dir;
    out->len = 0;
    loft_push_cstr(out, "cannot run git: this build cannot start a program");
    return -1;
}
#endif

/* A word that is a whole decimal integer (`str::parse::<i64>`); 0 when it is not one. */
static int loft_proc_number(loft_words *w, size_t i, int64_t *v) {
    if (i >= w->n || w->len[i] == 0) return 0;
    char *end = 0;
    errno = 0;
    long long x = strtoll(w->v[i], &end, 10);
    if (errno || !end || *end || isspace((unsigned char) w->v[i][0])) return 0;
    *v = x;
    return 1;
}

/* `control`: one of the library's operations, `@<op>:<words>` (see process_run.rs). */
static void loft_proc_control(const char *op, size_t oplen, loft_words *w, loft_str input, loft_proc_answer *a) {
#define LOFT_OP(s) (oplen == sizeof s - 1 && memcmp(op, s, oplen) == 0)
    int64_t h = 0, ms = 0;
    if (LOFT_OP("path")) {
        /* PathText::program on this host: a name no platform takes refuses the path; the
         * child is handed its normal form. */
        loft_str raw = w->n ? loft_lit(w->v[0], w->len[0]) : loft_lit("", 0);
        for (size_t i = 0; i < raw.len;) {
            size_t s = i;
            while (i < raw.len && raw.p[i] != '/' && raw.p[i] != '\\') i++;
            size_t why_len = 0;
            char *why = i > s ? loft_name_refusal(raw.p + s, i - s, &why_len) : 0;
            i++;
            if (why) {
                a->code = -1;
                loft_push_cstr(&a->err, "path `");
                loft_push(&a->err, raw.p, raw.len);
                loft_push_cstr(&a->err, "`: ");
                loft_push(&a->err, why, why_len);
                free(why);
                return;
            }
        }
        char *norm = loft_normalize(raw, "", 0);
        a->code = 0;
        loft_push_cstr(&a->out, norm ? norm : "");
        free(norm);
        return;
    }
#if defined(LOFT_POSIX_HOST)
    if (LOFT_OP("start")) {
        loft_proc_start(w, a);
    } else if (LOFT_OP("run")) {
        if (w->n == 0) {
            loft_proc_say(a, -1, 0, loft_proc_malformed);
            return;
        }
        int64_t limit = 0;
        if (!loft_proc_number(w, 0, &limit) || limit < 0) limit = 0;
        loft_words rest = { w->v + 1, w->len + 1, w->n - 1 };
        loft_run_until(&rest, input.p, input.len, limit, a);
    } else if (LOFT_OP("wait")) {
        if (!loft_proc_number(w, 0, &h) || !loft_proc_number(w, 1, &ms)) {
            loft_proc_say(a, -1, 0, loft_proc_malformed);
            return;
        }
        loft_started *s = loft_started_find(h);
        if (!s) {
            a->code = -1;
            return;
        }
        if (loft_child_wait(s->pid, ms < 0 ? 0 : ms)) {
            char code[32];
            snprintf(code, sizeof code, "%lld", (long long) loft_child_reap(s->pid));
            loft_started_drop(s);
            loft_proc_say(a, 1, code, 0);
        } else {
            a->code = 0;
        }
    } else if (LOFT_OP("alive")) {
        loft_started *s = loft_proc_number(w, 0, &h) ? loft_started_find(h) : 0;
        a->code = s && !loft_child_exited(s->pid, 0);
    } else if (LOFT_OP("stop")) {
        loft_started *s = loft_proc_number(w, 0, &h) ? loft_started_find(h) : 0;
        if (!s) {
            a->code = 0;
            return;
        }
        char code[32];
        snprintf(code, sizeof code, "%lld", (long long) loft_child_stop(s->pid));
        loft_started_drop(s);
        loft_proc_say(a, 1, code, 0);
    } else if (LOFT_OP("lines")) {
        if (w->n == 0) {
            loft_proc_say(a, -1, 0, loft_proc_malformed);
            return;
        }
        loft_words rest = { w->v + 1, w->len + 1, w->n - 1 };
        loft_cursor_open(w->len[0] == 3 && memcmp(w->v[0], "err", 3) == 0, &rest, input, a);
    } else if (LOFT_OP("next") || LOFT_OP("done") || LOFT_OP("lstop")) {
        if (!loft_proc_number(w, 0, &h)) {
            loft_proc_say(a, -1, 0, loft_proc_malformed);
            return;
        }
        if (LOFT_OP("next")) loft_cursor_next(h, a);
        else loft_cursor_done(h, LOFT_OP("lstop"), a);
    } else {
        loft_proc_say(a, -1, 0, loft_proc_malformed);
    }
#else
    (void) h;
    (void) ms;
    (void) input;
    loft_proc_say(a, -1, 0, "process: this build cannot start a program");
#endif
#undef LOFT_OP
}

/* `process_run(argv, input, out, err) -> integer`: the whole run, `out` and `err` replaced. */
static int64_t loft_process_run(loft_str argv, loft_str input, loft_text *out, loft_text *err) {
    loft_proc_answer a = { 0, { 0, 0, 0 }, { 0, 0, 0 } };
    loft_words w;
    if (argv.len && argv.p[0] == '@' && memchr(argv.p, ':', argv.len)) {
        const char *colon = (const char *) memchr(argv.p, ':', argv.len);
        if (loft_words_decode(colon + 1, argv.len - (size_t) (colon + 1 - argv.p), &w)) {
            loft_proc_control(argv.p + 1, (size_t) (colon - argv.p - 1), &w, input, &a);
            loft_words_free(&w);
        } else {
            loft_proc_say(&a, -1, 0, loft_proc_malformed);
        }
    } else if (loft_words_decode(argv.p, argv.len, &w)) {
#if defined(LOFT_POSIX_HOST)
        loft_run_until(&w, input.p, input.len, -1, &a);
#else
        loft_proc_say(&a, -1, 0, "process: this build cannot start a program");
#endif
        loft_words_free(&w);
    } else {
        loft_proc_say(&a, -1, 0, loft_proc_malformed);
    }
    out->len = 0;
    err->len = 0;
    loft_push(out, a.out.p, a.out.len);
    loft_push(err, a.err.p, a.err.len);
    free(a.out.p);
    free(a.err.p);
    return a.code;
}
#endif
