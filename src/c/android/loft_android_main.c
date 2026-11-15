/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * The Android entry of a program built by the C backend (`loft --native-c --native-android`,
 * doc/claude/plans/106-android-build-target/c-backend.md piece 1).  Compiled beside the
 * program's unit, whose `main` is renamed `loft_program_main`, and the NDK's
 * `android_native_app_glue.c`, which turns the activity's callbacks into an event queue and
 * calls `android_main` on a thread of its own.
 *
 * - The program runs on that thread: it owns the activity's `ALooper`, which a library that
 *   draws polls (`loft_android_app()`).
 * - That thread gets the stack a C-backend program has on the desktop, 8 MiB (the main
 *   thread's usual limit, and `LOFT_PAR_STACK` for a `par` worker), where bionic's default is
 *   about 1 MiB: the glue is compiled with `-Dpthread_create=loft_android_glue_thread`, and this
 *   function sets the size on the attributes the glue built before it creates the thread.  The
 *   Rust writer's 512 MiB is not copied: a reservation that large can fail on a 32-bit device,
 *   and then the app thread never starts.
 * - stdout and stderr go to logcat, one line per entry, tags `loft-stdout` and `loft-stderr`,
 *   so a program's output is read with `adb logcat -s loft-stdout`.  logcat drops lines of a
 *   fast writer, so a build that is not `--ship` (`LOFT_ANDROID_STDOUT_COPY`) also keeps the
 *   whole stdout in `loft-stdout.txt` and the exit code in `loft-exit.txt` in the app's files
 *   directory, which a debuggable app lets `adb shell run-as` read
 *   (`scripts/c_sweep.py --android`).
 * - The working directory and the program's directory (`LOFT_SOURCE_DIR`, which a relative path
 *   resolves against) are the app's own files directory, so a relative path lands somewhere the
 *   app may write; TMPDIR is a `tmp` directory beside it.
 * - After the program returns, the activity lives on until the OS destroys it, so the process
 *   is not ended under it.  A program that stops the run (`exit(n)`, a failed assertion, the
 *   store limit) does the same: the runtime's `LOFT_EXIT` reaches `loft_android_exit`, which
 *   returns the run to `android_main` with that status.  Ending the process instead reads to
 *   the OS as a crash, and it answers one by starting the app again. */
#include <android/log.h>
#include <android_native_app_glue.h>
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <setjmp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

int loft_program_main(int argc, char **argv);

static struct android_app *loft_android_app_handle;

/* The activity a library reaches the window, the input queue and the looper through. */
struct android_app *loft_android_app(void) {
    return loft_android_app_handle;
}

#ifndef LOFT_ANDROID_STACK
#define LOFT_ANDROID_STACK ((size_t) 8 << 20)
#endif

/* The glue's `pthread_create`, under `-Dpthread_create=loft_android_glue_thread`.  The glue
 * passes its own attributes, so setting their stack size here changes only its app thread. */
int loft_android_glue_thread(pthread_t *thread, const pthread_attr_t *attr,
                             void *(*start)(void *), void *arg) {
    if (attr != NULL) {
        pthread_attr_setstacksize((pthread_attr_t *) attr, LOFT_ANDROID_STACK);
    }
    return pthread_create(thread, attr, start, arg);
}

/* One output stream's reader: the pipe the program's stream was moved to, its logcat tag, and
 * the file that keeps its whole text (-1 for none). */
struct loft_android_stream {
    int pipe_fd;
    const char *tag;
    int copy_fd;
    pthread_t thread;
};

static struct loft_android_stream loft_android_out = {-1, "loft-stdout", -1, 0};
static struct loft_android_stream loft_android_err = {-1, "loft-stderr", -1, 0};

/* Copy what the program writes into logcat, a line per entry, and into the copy file whole. */
static void *loft_android_log_reader(void *arg) {
    struct loft_android_stream *s = arg;
    char buf[1024];
    char line[4096];
    size_t len = 0;
    for (;;) {
        ssize_t n = read(s->pipe_fd, buf, sizeof buf);
        if (n <= 0) {
            break;
        }
        if (s->copy_fd >= 0) {
            ssize_t at = 0;
            while (at < n) {
                ssize_t w = write(s->copy_fd, buf + at, (size_t) (n - at));
                if (w <= 0) {
                    break;
                }
                at += w;
            }
        }
        for (ssize_t i = 0; i < n; i++) {
            if (buf[i] == '\n' || len == sizeof line - 1) {
                line[len] = 0;
                __android_log_write(ANDROID_LOG_INFO, s->tag, line);
                len = 0;
                if (buf[i] == '\n') {
                    continue;
                }
            }
            line[len++] = buf[i];
        }
    }
    if (len > 0) {
        line[len] = 0;
        __android_log_write(ANDROID_LOG_INFO, s->tag, line);
    }
    return NULL;
}

/* Move the program's stream `fd` onto a pipe that `s`'s reader drains. */
static void loft_android_redirect(struct loft_android_stream *s, int fd, FILE *stream) {
    int fds[2];
    if (pipe(fds) != 0) {
        return;
    }
    dup2(fds[1], fd);
    close(fds[1]);
    /* A pipe is fully buffered by default: a line reaches logcat when the program ends it. */
    setvbuf(stream, NULL, _IOLBF, 0);
    s->pipe_fd = fds[0];
    if (pthread_create(&s->thread, NULL, loft_android_log_reader, s) != 0) {
        s->thread = 0;
    }
}

/* After the program: close its streams, so each reader drains to the end and stops, and wait
 * for both, so the copy is whole before its exit code is written beside it. */
static void loft_android_streams_done(void) {
    fflush(stdout);
    fflush(stderr);
    close(STDOUT_FILENO);
    close(STDERR_FILENO);
    if (loft_android_out.thread != 0) {
        pthread_join(loft_android_out.thread, NULL);
    }
    if (loft_android_err.thread != 0) {
        pthread_join(loft_android_err.thread, NULL);
    }
}

#if defined(LOFT_ANDROID_STDOUT_COPY)
/* `name` in the app's files directory, created empty. */
static int loft_android_open_copy(const char *dir, const char *name) {
    char path[4096];
    snprintf(path, sizeof path, "%s/%s", dir, name);
    return open(path, O_WRONLY | O_CREAT | O_TRUNC, 0600);
}
#endif

/* The app's files directory, NULL before `android_main` reads it. */
static const char *loft_android_files;

/* Write the program's status to `loft-exit.txt` (a build that is not `--ship`). */
static void loft_android_record_exit(int code) {
#if defined(LOFT_ANDROID_STDOUT_COPY)
    if (loft_android_files != NULL) {
        int fd = loft_android_open_copy(loft_android_files, "loft-exit.txt");
        if (fd >= 0) {
            char text[16];
            int len = snprintf(text, sizeof text, "%d\n", code);
            if (len > 0) {
                ssize_t w = write(fd, text, (size_t) len);
                (void) w;
            }
            close(fd);
        }
    }
#else
    (void) code;
#endif
}

/* Where a stopped run returns to: the program's thread inside `android_main`, while it runs. */
static jmp_buf loft_android_run;
static pthread_t loft_android_run_thread;
static volatile int loft_android_running;

/* The runtime's `LOFT_EXIT` (src/c/loft_cc.h): the run ends with status `code`.  On the program's
 * own thread it unwinds to `android_main`, which finishes as after a return (`setjmp` cannot
 * carry 0, so the status travels as code + 1).  From another thread, a `par` worker, the jump
 * would land on a stack that is not its own: the status is recorded and the process ends. */
__attribute__((noreturn)) void loft_android_exit(int code) {
    code &= 0xFF;
    fflush(stdout);
    fflush(stderr);
    if (loft_android_running && pthread_equal(pthread_self(), loft_android_run_thread)) {
        longjmp(loft_android_run, code + 1);
    }
    loft_android_record_exit(code);
    _exit(code);
}

void android_main(struct android_app *app) {
    static char name[] = "loft";
    char *argv[] = {name, NULL};
    int code, stopped;
    const char *files = NULL;
    loft_android_app_handle = app;
    if (app->activity != NULL && app->activity->internalDataPath != NULL) {
        files = app->activity->internalDataPath;
        mkdir(files, 0700);
        if (chdir(files) != 0) {
            __android_log_print(ANDROID_LOG_WARN, "loft", "cannot enter %s: %s", files,
                                strerror(errno));
        }
        /* A relative path resolves against the program's own directory, which the runtime takes
         * from `LOFT_SOURCE_DIR` (the desktop driver hands it every run) before the executable's:
         * here that is the VM launcher in a read-only system directory.  The app's own files
         * directory is the program's, and a `tmp` beside it its TMPDIR (there is no /tmp). */
        setenv("LOFT_SOURCE_DIR", files, 1);
        {
            char tmp[4096];
            snprintf(tmp, sizeof tmp, "%s/tmp", files);
            mkdir(tmp, 0700);
            setenv("TMPDIR", tmp, 1);
        }
    }
#if defined(LOFT_ANDROID_STDOUT_COPY)
    if (files != NULL) {
        int stale = loft_android_open_copy(files, "loft-exit.txt");
        if (stale >= 0) {
            close(stale); /* the exit code is written only when the program returns */
        }
        loft_android_out.copy_fd = loft_android_open_copy(files, "loft-stdout.txt");
    }
#endif
    loft_android_redirect(&loft_android_out, STDOUT_FILENO, stdout);
    loft_android_redirect(&loft_android_err, STDERR_FILENO, stderr);
    __android_log_write(ANDROID_LOG_INFO, "loft", "android_main reached");
    loft_android_files = files;
    loft_android_run_thread = pthread_self();
    stopped = setjmp(loft_android_run);
    if (stopped == 0) {
        loft_android_running = 1;
        code = loft_program_main(1, argv);
    } else {
        code = stopped - 1;
    }
    loft_android_running = 0;
    loft_android_streams_done();
#if defined(LOFT_ANDROID_STDOUT_COPY)
    if (loft_android_out.copy_fd >= 0) {
        close(loft_android_out.copy_fd);
    }
#endif
    loft_android_record_exit(code);
    __android_log_print(ANDROID_LOG_INFO, "loft", "program returned %d", code);
    while (!app->destroyRequested) {
        int events;
        struct android_poll_source *source = NULL;
        if (ALooper_pollOnce(-1, NULL, &events, (void **) &source) >= 0 && source != NULL) {
            source->process(app, source);
        }
    }
}
