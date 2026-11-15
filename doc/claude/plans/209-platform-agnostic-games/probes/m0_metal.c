/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * Build and run: cc -std=c99 -O2 -o m0 m0_metal.c && ./m0   (add -arch x86_64 for Rosetta)
 */
/* M0 probe (@PLN209 GPU_LAYER.md): Metal from C99 with no SDK header — the Objective-C runtime,
 * Metal, QuartzCore and AppKit opened at run time, every message through objc_msgSend cast to
 * the method's own type.  Two cells, each a clear to one colour read back:
 *   1. an offscreen texture;
 *   2. a CAMetalLayer on an NSWindow's content view (its drawable's texture).
 * `M0_BREAK=1` clears to another colour than the one checked: the harness must then fail. */
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef void *id;
typedef void *sel;
typedef struct { double r, g, b, a; } clear_color;                 /* MTLClearColor */
typedef struct { uint64_t x, y, z, w, h, d; } region;              /* MTLRegion */
typedef struct { double x, y, w, h; } rect;                        /* NSRect / CGRect */
typedef struct { double w, h; } size2;                             /* CGSize */

static void *objc, *metal, *quartz, *appkit;
static id (*get_class)(const char *);
static sel (*sel_reg)(const char *);
static void *msg; /* objc_msgSend */

#define CLS(n) get_class(n)
#define SEL(n) sel_reg(n)
#define M0(ret, o, s) ((ret (*)(id, sel)) msg)((id) (o), SEL(s))
#define M1(ret, o, s, t1, a1) ((ret (*)(id, sel, t1)) msg)((id) (o), SEL(s), a1)

static int check(const char *cell, const uint8_t *px, int w, int h, int stride) {
    /* BGRA8: the colour asked is r=1.0 g=0.5 b=0.0 a=1.0 -> B=0 G=128 (or 127) R=255 A=255 */
    int bad = 0;
    for (int y = 0; y < h; y++) {
        for (int x = 0; x < w; x++) {
            const uint8_t *p = px + y * stride + x * 4;
            if (p[0] != 0 || (p[1] != 128 && p[1] != 127) || p[2] != 255 || p[3] != 255) { bad++; }
        }
    }
    const uint8_t *f = px, *l = px + (h - 1) * stride + (w - 1) * 4;
    printf("%s: %dx%d first=%02x%02x%02x%02x last=%02x%02x%02x%02x bad=%d -> %s\n", cell, w, h,
           f[0], f[1], f[2], f[3], l[0], l[1], l[2], l[3], bad, bad ? "FAIL" : "ok");
    return bad != 0;
}

/* Clear `tex` to the colour through a render pass on `queue`, wait, and read it back. */
static int clear_and_read(const char *cell, id queue, id tex, int w, int h) {
    id rpd = M0(id, CLS("MTLRenderPassDescriptor"), "renderPassDescriptor");
    id atts = M0(id, rpd, "colorAttachments");
    id att = M1(id, atts, "objectAtIndexedSubscript:", uint64_t, 0);
    M1(void, att, "setTexture:", id, tex);
    M1(void, att, "setLoadAction:", uint64_t, 2);  /* MTLLoadActionClear */
    M1(void, att, "setStoreAction:", uint64_t, 1); /* MTLStoreActionStore */
    clear_color c = {1.0, 0.5, 0.0, 1.0};
    if (getenv("M0_BREAK")) { c.g = 0.25; }
    M1(void, att, "setClearColor:", clear_color, c);
    id cb = M0(id, queue, "commandBuffer");
    id enc = M1(id, cb, "renderCommandEncoderWithDescriptor:", id, rpd);
    if (!enc) { printf("%s: no encoder -> FAIL\n", cell); return 1; }
    M0(void, enc, "endEncoding");
    M0(void, cb, "commit");
    M0(void, cb, "waitUntilCompleted");
    uint8_t *px = malloc((size_t) w * h * 4);
    memset(px, 0x55, (size_t) w * h * 4);
    region r = {0, 0, 0, (uint64_t) w, (uint64_t) h, 1};
    ((void (*)(id, sel, void *, uint64_t, region, uint64_t)) msg)(
        tex, SEL("getBytes:bytesPerRow:fromRegion:mipmapLevel:"), px, (uint64_t) w * 4, r, 0);
    int bad = check(cell, px, w, h, w * 4);
    free(px);
    return bad;
}

int main(void) {
    objc = dlopen("/usr/lib/libobjc.A.dylib", RTLD_NOW);
    metal = dlopen("/System/Library/Frameworks/Metal.framework/Metal", RTLD_NOW);
    quartz = dlopen("/System/Library/Frameworks/QuartzCore.framework/QuartzCore", RTLD_NOW);
    appkit = dlopen("/System/Library/Frameworks/AppKit.framework/AppKit", RTLD_NOW);
    if (!objc || !metal || !quartz || !appkit) { printf("a framework did not open -> FAIL\n"); return 1; }
    get_class = (id (*)(const char *)) dlsym(objc, "objc_getClass");
    sel_reg = (sel (*)(const char *)) dlsym(objc, "sel_registerName");
    msg = dlsym(objc, "objc_msgSend");
    id (*create_device)(void) = (id (*)(void)) dlsym(metal, "MTLCreateSystemDefaultDevice");
    id pool = M0(id, M0(id, CLS("NSAutoreleasePool"), "alloc"), "init");
    id device = create_device ? create_device() : NULL;
    if (!device) { printf("no Metal device -> none\n"); return 2; }
    id name = M0(id, device, "name");
    printf("device: %s\n", M0(const char *, name, "UTF8String"));
    id queue = M0(id, device, "newCommandQueue");
    int bad = 0;

    /* Cell 1: an offscreen texture. */
    int w = 64, h = 48;
    id desc = ((id (*)(id, sel, uint64_t, uint64_t, uint64_t, signed char)) msg)(
        CLS("MTLTextureDescriptor"), SEL("texture2DDescriptorWithPixelFormat:width:height:mipmapped:"),
        80 /* BGRA8Unorm */, (uint64_t) w, (uint64_t) h, 0);
    M1(void, desc, "setUsage:", uint64_t, 4 | 1); /* render target | shader read */
    id tex = M1(id, device, "newTextureWithDescriptor:", id, desc);
    if (!tex) { printf("offscreen: no texture -> FAIL\n"); return 1; }
    bad |= clear_and_read("offscreen", queue, tex, w, h);

    /* Cell 2: a CAMetalLayer on a window's content view. */
    id app = M0(id, CLS("NSApplication"), "sharedApplication");
    M1(void, app, "setActivationPolicy:", int64_t, 0);
    rect fr = {100, 100, 64, 48};
    id win = ((id (*)(id, sel, rect, uint64_t, uint64_t, signed char)) msg)(
        M0(id, CLS("NSWindow"), "alloc"), SEL("initWithContentRect:styleMask:backing:defer:"), fr,
        1 /* titled */, 2 /* buffered */, 0);
    id view = M0(id, win, "contentView");
    id layer = M0(id, CLS("CAMetalLayer"), "layer");
    M1(void, layer, "setDevice:", id, device);
    M1(void, layer, "setPixelFormat:", uint64_t, 80);
    M1(void, layer, "setFramebufferOnly:", signed char, 0);
    size2 ds = {64, 48};
    M1(void, layer, "setDrawableSize:", size2, ds);
    M1(void, view, "setWantsLayer:", signed char, 1);
    M1(void, view, "setLayer:", id, layer);
    M1(void, win, "makeKeyAndOrderFront:", id, NULL);
    id drawable = M0(id, layer, "nextDrawable");
    if (!drawable) { printf("layer: no drawable -> FAIL\n"); return 1; }
    id dtex = M0(id, drawable, "texture");
    int dw = (int) M0(uint64_t, dtex, "width"), dh = (int) M0(uint64_t, dtex, "height");
    bad |= clear_and_read("layer", queue, dtex, dw, dh);
    id cb = M0(id, queue, "commandBuffer");
    M1(void, cb, "presentDrawable:", id, drawable);
    M0(void, cb, "commit");
    M0(void, cb, "waitUntilCompleted");
    M0(void, win, "close");
    M0(void, pool, "drain");
    printf("M0: %s\n", bad ? "FAIL" : "ok");
    return bad;
}
