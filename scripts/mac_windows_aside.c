/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later */
/* For window tests on a Mac that somebody is working on: a window a program centres opens on
 * the screen that does NOT hold the keyboard, and the program does not take the keyboard.
 * It changes no library: it is loaded into the test run alone.
 *
 *   cc -dynamiclib -o /tmp/aside.dylib scripts/mac_windows_aside.c
 *   DYLD_INSERT_LIBRARIES=/tmp/aside.dylib GRAPHICS_TEST_WINDOW=1 loft --tests tests
 *
 * It replaces four AppKit methods for that process: `+[NSScreen mainScreen]` (the first screen
 * the pointer is not on, so a whole-screen window opens there too), `-[NSWindow center]` (centre
 * on that screen; on a Mac with one screen, where it was),
 * `-[NSWindow makeKeyAndOrderFront:]` (shown, not made key) and
 * `-[NSApplication activateIgnoringOtherApps:]` (nothing).  Injected keys still arrive: a test posts
 * them into its own queue. */
#include <dlfcn.h>
#include <stddef.h>
typedef void *id; typedef void *sel;
typedef struct { double x, y, w, h; } rect;
typedef struct { double x, y; } point;
static id (*cls)(const char *); static sel (*s)(const char *); static void *msg;
/* The first screen the pointer is not on: where the user is not working.  With one screen, it. */
static id aside_screen(id self, sel cmd) {
    (void) self;
    (void) cmd;
    id screens = ((id (*)(id, sel)) msg)(cls("NSScreen"), s("screens")), first = NULL;
    point at = ((point (*)(id, sel)) msg)(cls("NSEvent"), s("mouseLocation"));
    unsigned long n = ((unsigned long (*)(id, sel)) msg)(screens, s("count")), i;
    for (i = 0; i < n; i++) {
        id sc = ((id (*)(id, sel, unsigned long)) msg)(screens, s("objectAtIndex:"), i);
        rect f = ((rect (*)(id, sel)) msg)(sc, s("frame"));
        if (!first) first = sc;
        if (at.x < f.x || at.x >= f.x + f.w || at.y < f.y || at.y >= f.y + f.h) return sc;
    }
    return first;
}
static void centre_aside(id self, sel cmd) {
    (void) cmd;
    id target = aside_screen(NULL, NULL);
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
    id (*class_of)(id) = (id (*)(id)) dlsym(objc, "object_getClass");
    replace(class_of(cls("NSScreen")), s("mainScreen"), (void (*)(void)) aside_screen, "@@:");
    replace(cls("NSWindow"), s("center"), (void (*)(void)) centre_aside, "v@:");
    replace(cls("NSWindow"), s("makeKeyAndOrderFront:"), (void (*)(void)) front, "v@:@");
    replace(cls("NSApplication"), s("activateIgnoringOtherApps:"), (void (*)(void)) quiet, "v@:c");
}
