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
 * - stdout and stderr go to logcat, one line per entry, tag `loft-stdout`, so a program's
 *   output is read with `adb logcat -s loft-stdout`.
 * - The working directory is the app's own files directory, so a relative path lands
 *   somewhere the app may write.
 * - After the program returns, the activity lives on until the OS destroys it, so the process
 *   is not ended under it. */
#include <android/log.h>
#include <android_native_app_glue.h>
#include <errno.h>
#include <pthread.h>
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

static int loft_android_log_fd = -1;

/* Copy what the program writes into logcat, a line per entry. */
static void *loft_android_log_reader(void *unused) {
    char buf[1024];
    char line[4096];
    size_t len = 0;
    (void) unused;
    for (;;) {
        ssize_t n = read(loft_android_log_fd, buf, sizeof buf);
        if (n <= 0) {
            break;
        }
        for (ssize_t i = 0; i < n; i++) {
            if (buf[i] == '\n' || len == sizeof line - 1) {
                line[len] = 0;
                __android_log_write(ANDROID_LOG_INFO, "loft-stdout", line);
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
        __android_log_write(ANDROID_LOG_INFO, "loft-stdout", line);
    }
    return NULL;
}

static void loft_android_stdio_to_logcat(void) {
    int fds[2];
    pthread_t reader;
    if (pipe(fds) != 0) {
        return;
    }
    dup2(fds[1], STDOUT_FILENO);
    dup2(fds[1], STDERR_FILENO);
    close(fds[1]);
    /* A pipe is fully buffered by default: a line reaches logcat when the program ends it. */
    setvbuf(stdout, NULL, _IOLBF, 0);
    setvbuf(stderr, NULL, _IOLBF, 0);
    loft_android_log_fd = fds[0];
    if (pthread_create(&reader, NULL, loft_android_log_reader, NULL) == 0) {
        pthread_detach(reader);
    }
}

void android_main(struct android_app *app) {
    static char name[] = "loft";
    char *argv[] = {name, NULL};
    int code;
    loft_android_app_handle = app;
    loft_android_stdio_to_logcat();
    if (app->activity != NULL && app->activity->internalDataPath != NULL) {
        mkdir(app->activity->internalDataPath, 0700);
        if (chdir(app->activity->internalDataPath) != 0) {
            __android_log_print(ANDROID_LOG_WARN, "loft", "cannot enter %s: %s",
                                app->activity->internalDataPath, strerror(errno));
        }
    }
    __android_log_write(ANDROID_LOG_INFO, "loft", "android_main reached");
    code = loft_program_main(1, argv);
    fflush(stdout);
    fflush(stderr);
    __android_log_print(ANDROID_LOG_INFO, "loft", "program returned %d", code);
    while (!app->destroyRequested) {
        int events;
        struct android_poll_source *source = NULL;
        if (ALooper_pollOnce(-1, NULL, &events, (void **) &source) >= 0 && source != NULL) {
            source->process(app, source);
        }
    }
}
