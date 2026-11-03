/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * The ESP-IDF half of a loft app (@PLN204, TANMATSU.md § A loft program as an app): written
 * after the unit by `emit_esp_program`, never part of a host build.
 *
 * - The program's ASSETS: the files the build packed (`loft_assets`, in the project's
 *   `loft_assets.c`), served read-only as a filesystem on ESP-IDF's empty VFS prefix.  ESP-IDF
 *   has no working directory, so a relative path matches no mounted filesystem and falls to
 *   this one, unchanged: `fopen("assets/x.png")` in the runtime or in a library's C finds the
 *   file the program found beside itself on the host.
 * - `app_main`: starts the board (NVS, then the BSP: display, keys, audio), mounts the assets,
 *   runs the program and goes back to the launcher.  Each step prints `@boot <µs> <step>` on the
 *   console — the USB serial port — so a host sees what the device did and how long it took. */
#include <errno.h>
#include <fcntl.h>
#include <string.h>
#include <sys/stat.h>
#include "esp_err.h"
#include "esp_timer.h"
#include "esp_vfs.h"
#include "esp_vfs_ops.h"
#include "nvs_flash.h"
#include "bsp/device.h"
#include "bsp/display.h"

struct loft_asset {
    const char *path;
    const unsigned char *data;
    size_t size;
};
extern const struct loft_asset loft_assets[];
extern const size_t loft_asset_count;

/* An open asset: which one, and how far it has been read. */
#define LOFT_ASSET_FDS 16
static struct {
    const struct loft_asset *a;
    size_t pos;
} loft_asset_fd[LOFT_ASSET_FDS];

/* `./a/b`, `/a/b` and `a/b` name the same asset. */
static const char *loft_asset_name(const char *p) {
    while (p[0] == '.' && p[1] == '/') p += 2;
    while (p[0] == '/') p++;
    return p;
}

static const struct loft_asset *loft_asset_find(const char *path) {
    size_t i;
    path = loft_asset_name(path);
    for (i = 0; i < loft_asset_count; i++)
        if (strcmp(loft_assets[i].path, path) == 0) return &loft_assets[i];
    return 0;
}

/* A directory is a path some asset lies under. */
static int loft_asset_is_dir(const char *path) {
    size_t i, n;
    path = loft_asset_name(path);
    n = strlen(path);
    if (n == 0) return 1;
    for (i = 0; i < loft_asset_count; i++)
        if (strncmp(loft_assets[i].path, path, n) == 0 && loft_assets[i].path[n] == '/') return 1;
    return 0;
}

static int loft_asset_open(void *ctx, const char *path, int flags, int mode) {
    const struct loft_asset *a;
    int fd;
    (void) ctx;
    (void) mode;
    if ((flags & O_ACCMODE) != O_RDONLY) {
        errno = EROFS;
        return -1;
    }
    a = loft_asset_find(path);
    if (!a) {
        errno = ENOENT;
        return -1;
    }
    for (fd = 0; fd < LOFT_ASSET_FDS; fd++)
        if (!loft_asset_fd[fd].a) {
            loft_asset_fd[fd].a = a;
            loft_asset_fd[fd].pos = 0;
            return fd;
        }
    errno = ENFILE;
    return -1;
}

static int loft_asset_ok(int fd) {
    if (fd < 0 || fd >= LOFT_ASSET_FDS || !loft_asset_fd[fd].a) {
        errno = EBADF;
        return 0;
    }
    return 1;
}

static ssize_t loft_asset_pread(void *ctx, int fd, void *dst, size_t size, off_t offset) {
    const struct loft_asset *a;
    (void) ctx;
    if (!loft_asset_ok(fd)) return -1;
    a = loft_asset_fd[fd].a;
    if (offset < 0 || (size_t) offset >= a->size) return 0;
    if (size > a->size - (size_t) offset) size = a->size - (size_t) offset;
    memcpy(dst, a->data + offset, size);
    return (ssize_t) size;
}

static ssize_t loft_asset_read(void *ctx, int fd, void *dst, size_t size) {
    ssize_t n = loft_asset_pread(ctx, fd, dst, size, loft_asset_ok(fd) ? (off_t) loft_asset_fd[fd].pos : 0);
    if (n > 0) loft_asset_fd[fd].pos += (size_t) n;
    return n;
}

static ssize_t loft_asset_write(void *ctx, int fd, const void *src, size_t size) {
    (void) ctx;
    (void) fd;
    (void) src;
    (void) size;
    errno = EROFS;
    return -1;
}

static off_t loft_asset_lseek(void *ctx, int fd, off_t offset, int whence) {
    off_t base;
    (void) ctx;
    if (!loft_asset_ok(fd)) return -1;
    base = whence == SEEK_SET ? 0
         : whence == SEEK_CUR ? (off_t) loft_asset_fd[fd].pos
         : (off_t) loft_asset_fd[fd].a->size;
    if (base + offset < 0) {
        errno = EINVAL;
        return -1;
    }
    loft_asset_fd[fd].pos = (size_t) (base + offset);
    return base + offset;
}

static int loft_asset_close(void *ctx, int fd) {
    (void) ctx;
    if (!loft_asset_ok(fd)) return -1;
    loft_asset_fd[fd].a = 0;
    return 0;
}

static void loft_asset_fill(struct stat *st, size_t size, int dir) {
    memset(st, 0, sizeof *st);
    st->st_mode = dir ? (S_IFDIR | 0555) : (S_IFREG | 0444);
    st->st_size = (off_t) size;
}

static int loft_asset_fstat(void *ctx, int fd, struct stat *st) {
    (void) ctx;
    if (!loft_asset_ok(fd)) return -1;
    loft_asset_fill(st, loft_asset_fd[fd].a->size, 0);
    return 0;
}

static int loft_asset_stat(void *ctx, const char *path, struct stat *st) {
    const struct loft_asset *a = loft_asset_find(path);
    (void) ctx;
    if (a) {
        loft_asset_fill(st, a->size, 0);
        return 0;
    }
    if (loft_asset_is_dir(path)) {
        loft_asset_fill(st, 0, 1);
        return 0;
    }
    errno = ENOENT;
    return -1;
}

static int loft_asset_access(void *ctx, const char *path, int amode) {
    (void) ctx;
    if (amode & W_OK) {
        errno = EROFS;
        return -1;
    }
    if (loft_asset_find(path) || loft_asset_is_dir(path)) return 0;
    errno = ENOENT;
    return -1;
}

#ifdef CONFIG_VFS_SUPPORT_DIR
static const esp_vfs_dir_ops_t loft_asset_dir_ops = {
    .stat_p = loft_asset_stat,
    .access_p = loft_asset_access,
};
#endif

static const esp_vfs_fs_ops_t loft_asset_fs = {
    .write_p = loft_asset_write,
    .lseek_p = loft_asset_lseek,
    .read_p = loft_asset_read,
    .pread_p = loft_asset_pread,
    .open_p = loft_asset_open,
    .close_p = loft_asset_close,
    .fstat_p = loft_asset_fstat,
#ifdef CONFIG_VFS_SUPPORT_DIR
    .dir = &loft_asset_dir_ops,
#endif
};

static void loft_esp_note(const char *step, long long value) {
    if (value >= 0) printf("@boot %lld %s %lld\n", (long long) esp_timer_get_time(), step, value);
    else printf("@boot %lld %s\n", (long long) esp_timer_get_time(), step);
}

/* NVS, which the BSP and the radio read their settings from; a full or older partition is
 * erased and made again, as every ESP-IDF app does. */
static void loft_esp_nvs(void) {
    esp_err_t e = nvs_flash_init();
    if (e == ESP_ERR_NVS_NO_FREE_PAGES || e == ESP_ERR_NVS_NEW_VERSION_FOUND) {
        if (nvs_flash_erase() == ESP_OK) e = nvs_flash_init();
    }
    loft_esp_note("nvs", (long long) e);
}

/* The board: display (RGB565, one frame buffer — what `screen` writes), keys, audio.  A
 * board the BSP does not start still runs the program, which then finds no screen. */
static void loft_esp_board(void) {
    bsp_configuration_t cfg;
    memset(&cfg, 0, sizeof cfg);
    cfg.display.requested_color_format = BSP_DISPLAY_COLOR_FORMAT_16_565RGB;
    cfg.display.num_fbs = 1;
    loft_esp_note("bsp", (long long) bsp_device_initialize(&cfg));
}

static void loft_esp_assets(void) {
    esp_err_t e = esp_vfs_register_fs("", &loft_asset_fs, ESP_VFS_FLAG_CONTEXT_PTR | ESP_VFS_FLAG_READONLY_FS | ESP_VFS_FLAG_STATIC, NULL);
    loft_esp_note(e == ESP_OK ? "assets" : "assets-failed", e == ESP_OK ? (long long) loft_asset_count : (long long) e);
}

/* After the program: its exit line, the console flushed, then the launcher. */
static void loft_esp_end(int failed) {
    printf("@loft exit %d\n", failed);
    fflush(stdout);
    fflush(stderr);
    loft_esp_note("launcher", -1);
    fflush(stdout);
    bsp_device_restart_to_launcher();
}
