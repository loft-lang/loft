/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later */
/* For window tests on a Mac that somebody is working on: a window a program centres opens on
 * the screen that does NOT hold the keyboard, and the program does not take the keyboard.
 * It changes no library: it is loaded into the test run alone.
 *
 *   cc -dynamiclib -o /tmp/aside.dylib scripts/mac_windows_aside.c
 *   DYLD_INSERT_LIBRARIES=/tmp/aside.dylib GRAPHICS_TEST_WINDOW=1 loft --tests tests
 *
 * It replaces three AppKit methods for that process: `-[NSWindow center]` (centre on the first
 * screen that is not the main one; on a Mac with one screen, where it was),
 * `-[NSWindow makeKeyAndOrderFront:]` (shown, not made key) and
 * `-[NSApplication activateIgnoringOtherApps:]` (nothing).  A window not centred — a
 * whole-screen one — stays where its program put it.  Injected keys still arrive: a test posts
 * them into its own queue. */
#include <dlfcn.h>
#include <stddef.h>
typedef void *id; typedef void *sel;
typedef struct { double x, y, w, h; } rect;
typedef struct { double x, y; } point;
static id (*cls)(const char *); static sel (*s)(const char *); static void *msg;
static void centre_aside(id self, sel cmd) {
    (void) cmd;
    id screens = ((id (*)(id, sel)) msg)(cls("NSScreen"), s("screens"));
    id busy = ((id (*)(id, sel)) msg)(cls("NSScreen"), s("mainScreen")), target = NULL;
    unsigned long n = ((unsigned long (*)(id, sel)) msg)(screens, s("count")), i;
    for (i = 0; i < n && !target; i++) {
        id sc = ((id (*)(id, sel, unsigned long)) msg)(screens, s("objectAtIndex:"), i);
        if (sc != busy) target = sc;
    }
    if (!target) target = busy;
    rect on = ((rect (*)(id, sel)) msg)(target, s("visibleFrame"));
    rect own = ((rect (*)(id, sel)) msg)(self, s("frame"));
    point at = {on.x + (on.w - own.w) / 2, on.y + (on.h - own.h) / 2};
    ((void (*)(id, sel, point)) msg)(self, s("setFrameOrigin:"), at);
}
static void quiet(id self, sel cmd, signed char flag) { (void) self; (void) cmd; (void) flag; }
static void front(id self, sel cmd, id sender) { (void) cmd; ((void (*)(id, sel, id)) msg)(self, s("orderFront:"), sender); }
__attribute__((constructor)) static void aside(void) {
    void *objc = dlopen("/usr/lib/libobjc.A.dylib", RTLD_NOW);
    dlopen("/System/Library/Frameworks/AppKit.framework/AppKit", RTLD_NOW | RTLD_GLOBAL);
    cls = (id (*)(const char *)) dlsym(objc, "objc_getClass");
    s = (sel (*)(const char *)) dlsym(objc, "sel_registerName");
    msg = dlsym(objc, "objc_msgSend");
    void *(*replace)(id, sel, void (*)(void), const char *) =
        (void *(*)(id, sel, void (*)(void), const char *)) dlsym(objc, "class_replaceMethod");
    replace(cls("NSWindow"), s("center"), (void (*)(void)) centre_aside, "v@:");
    replace(cls("NSWindow"), s("makeKeyAndOrderFront:"), (void (*)(void)) front, "v@:@");
    replace(cls("NSApplication"), s("activateIgnoringOtherApps:"), (void (*)(void)) quiet, "v@:c");
}
