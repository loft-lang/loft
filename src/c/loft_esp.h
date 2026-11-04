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
#include "driver/gpio.h"
#include "esp_heap_caps.h"
#include "bsp/device.h"
#include "bsp/display.h"

/* The BSP's own start-up steps, in the order `bsp_device_initialize` runs them (badge-bsp
 * 1.6.4, the version the project pins): run here one by one, so each step's time is on the
 * console and the screen is black as soon as the display can show it.  A part starts when its
 * `LOFT_ESP_<PART>` is defined: the project writer defines the ones the app's C calls, and
 * the ones with users no call names (src/esp_app.rs `PARTS`; the motion sensor alone took
 * 1.16 s of every start). */
esp_err_t bsp_i2c_primary_bus_initialize(void);
esp_err_t bsp_device_initialize_custom(void);
esp_err_t bsp_display_initialize(const bsp_display_configuration_t *configuration);
esp_err_t bsp_input_initialize(void);
void bsp_input_hooks_initialize(void);
esp_err_t bsp_power_initialize(void);
esp_err_t bsp_rtc_initialize(void);
esp_err_t bsp_audio_initialize(void);
esp_err_t bsp_led_initialize(void);
esp_err_t bsp_storage_initialize(void);
esp_err_t bsp_orientation_initialize(void);
esp_err_t bsp_sensor_initialize(void);
esp_err_t bsp_catt_initialize(void);
esp_err_t bsp_sao_initialize(void);

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

/* The start-up's `@boot` lines are held until the program starts: they are written within a
 * second of the reset, while the PC is still connecting the USB port again, and a line the
 * port takes before a reader has it open is lost.  `loft_esp_release` prints them. */
static char loft_esp_held[2048];
static size_t loft_esp_held_len = 0;
static int loft_esp_holding = 1;

static void loft_esp_note(const char *step, long long value) {
    char line[128];
    int n = value >= 0 ? snprintf(line, sizeof line, "@boot %lld %s %lld\n", (long long) esp_timer_get_time(), step, value)
                       : snprintf(line, sizeof line, "@boot %lld %s\n", (long long) esp_timer_get_time(), step);
    if (n <= 0) return;
    if ((size_t) n >= sizeof line) n = (int) sizeof line - 1;
    if (loft_esp_holding && loft_esp_held_len + (size_t) n < sizeof loft_esp_held) {
        memcpy(loft_esp_held + loft_esp_held_len, line, (size_t) n);
        loft_esp_held_len += (size_t) n;
    } else {
        fwrite(line, 1, (size_t) n, stdout);
    }
}

static void loft_esp_release(void) {
    loft_esp_holding = 0;
    fwrite(loft_esp_held, 1, loft_esp_held_len, stdout);
    loft_esp_held_len = 0;
    fflush(stdout);
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

/* The whole panel black.  Until the first frame the panel shows what it shows without a
 * picture (blue), through every slower step after the display's own. */
static void loft_esp_black(void) {
    size_t h = 0, v = 0;
    bsp_display_color_format_t fmt;
    bsp_display_endianness_t endian;
    void *black;
    if (bsp_display_get_parameters(&h, &v, &fmt, &endian) != ESP_OK || h == 0 || v == 0) return;
    black = heap_caps_calloc(h * v, fmt == BSP_DISPLAY_COLOR_FORMAT_16_565RGB ? 2 : 3, MALLOC_CAP_SPIRAM);
    if (!black) return;
    bsp_display_blit(0, 0, h, v, black);
    bsp_display_wait(100);
    heap_caps_free(black);
    loft_esp_note("black", -1);
}

/* The board: display (RGB565, one frame buffer — what `screen` writes), keys, power, the
 * clock, audio, the LEDs and the SD card's supply, as `bsp_device_initialize` starts them.  A step that fails is reported and the next
 * runs, as there; a board the BSP does not start still runs the program, which then finds no
 * screen. */
static void loft_esp_board(void) {
    bsp_display_configuration_t display;
    memset(&display, 0, sizeof display);
    display.requested_color_format = BSP_DISPLAY_COLOR_FORMAT_16_565RGB;
    display.num_fbs = 1;
    gpio_install_isr_service(0);
    loft_esp_note("i2c", (long long) bsp_i2c_primary_bus_initialize());
    loft_esp_note("coprocessor", (long long) bsp_device_initialize_custom());
    {
        esp_err_t e = bsp_display_initialize(&display);
        loft_esp_note("display", (long long) e);
        if (e == ESP_OK) loft_esp_black();
    }
#if defined(LOFT_ESP_INPUT)
    {
        esp_err_t e = bsp_input_initialize();
        if (e == ESP_OK) bsp_input_hooks_initialize();
        loft_esp_note("input", (long long) e);
    }
#endif
#if defined(LOFT_ESP_POWER)
    loft_esp_note("power", (long long) bsp_power_initialize());
#endif
#if defined(LOFT_ESP_RTC)
    loft_esp_note("rtc", (long long) bsp_rtc_initialize());
#endif
#if defined(LOFT_ESP_AUDIO)
    loft_esp_note("audio", (long long) bsp_audio_initialize());
#endif
#if defined(LOFT_ESP_LED)
    loft_esp_note("led", (long long) bsp_led_initialize());
#endif
#if defined(LOFT_ESP_STORAGE)
    loft_esp_note("storage", (long long) bsp_storage_initialize());
#endif
#if defined(LOFT_ESP_ORIENTATION)
    loft_esp_note("orientation", (long long) bsp_orientation_initialize());
#endif
#if defined(LOFT_ESP_SENSOR)
    loft_esp_note("sensor", (long long) bsp_sensor_initialize());
#endif
#if defined(LOFT_ESP_CATT)
    loft_esp_note("catt", (long long) bsp_catt_initialize());
#endif
#if defined(LOFT_ESP_SAO)
    loft_esp_note("sao", (long long) bsp_sao_initialize());
#endif
}

static void loft_esp_assets(void) {
    esp_err_t e = esp_vfs_register_fs("", &loft_asset_fs, ESP_VFS_FLAG_CONTEXT_PTR | ESP_VFS_FLAG_READONLY_FS | ESP_VFS_FLAG_STATIC, NULL);
    loft_esp_note(e == ESP_OK ? "assets" : "assets-failed", e == ESP_OK ? (long long) loft_asset_count : (long long) e);
}

/* After the program: its exit line, the console flushed, then the launcher.  The backlight
 * goes off first: the keyboard's coprocessor drives it and keeps it through the reset, while
 * the panel, with no picture, shows blue until the launcher draws.  The launcher sets the
 * backlight from its settings once it has started. */
static void loft_esp_end(int failed) {
    printf("@loft exit %d\n", failed);
    fflush(stdout);
    fflush(stderr);
    bsp_display_set_backlight_brightness(0);
    loft_esp_note("launcher", -1);
    fflush(stdout);
    bsp_device_restart_to_launcher();
}
