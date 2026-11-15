<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 209 — the GPU layer and its backends, in small safe steps

Arc G of [@PLN209](README.md), cut into steps that each change one thing and can each go red on
their own.  It replaces the arc's G2–G4 wording; G1 and arc F stand as the README has them.

## Status

P1, P2, P3, L0, L1 and M0 are done and L2 is in review (2026-11-15); none changed § The shape.
Next: L3 (`gpu_gl`); publishing waits for the name (open question 2) and for @C164 in a
released loft.  V0
waits for a Linux machine.

| Track | Steps | State |
|---|---|---|
| P — the game on a Mac, on the GL that is there | P1–P5 | P1–P3 done; P4 stood in for until L3; P5 measured standing still |
| L — the neutral layer | L0–L8 | L0, L1, L2b done; L2 in review; L3–L8 open |
| M — the Metal backend | M0–M4 | M0 done; M1–M4 open |
| V — the Vulkan backend | V0–V4 | Open |
| H — one shader source | H1–H4 | Open |
| C — what a console backend needs kept free | C1–C2 | Open |

## Goal

1. The latest tile-world-game plays on a Mac as a native, fully optimised (`--ship`) program,
   in its 3D presentation.
2. A game, and `stage`, build against one GPU library that names no graphics stack and depends
   on none.  Each stack — OpenGL and WebGL2, Metal, a CPU renderer for a machine with no GPU
   (the Tanmatsu), Vulkan, later a Nintendo and a Sony backend — is a library of its own beneath it.

## The owner's rulings (2026-11-15)

- A library may be about one stack (GL, Metal, the Tanmatsu's renderer, a console's).
- The layer games build against depends on none of them: the arrows point from a backend to the
  layer, never back.
- The layer must admit the console backends when they become available, so nothing in it may
  assume what only some stacks can do.

## Where things stand (measured 2026-11-15, each library's `main`)

- **The whole GPU surface is one C file**: graphics 0.12.1's `c/graphics_gl.c` (2,719 lines)
  behind 48 `#c` declarations, with three hosts in it: X11 + GLX, a page's WebGL2, and macOS
  through AppKit and an `NSOpenGLContext`.
- **The macOS GL host works**: graphics' `window`, `shader`, `texture`, `state` and `buffers`
  tests pass against a real window on an Apple-silicon Mac (20 tests; interpreter only, the C
  backend not run).
- **The game cannot open a window on a Mac**: `screen`'s C excludes `__APPLE__`, so
  `screen::open` answers false.
- **The game's 3D view comes on only in a browser page**: `render3d::toggle` asks
  `tiles::on_display`, which is true only for a page's GPU-drawn surface.
- **The game pins library branches**: `screen` and `audio` at `tuxedo-screen-esp-present`,
  `tiles` at `tuxedo-native-pixels`.
- **GL shows through graphics' API** in four places: shader source is GLSL text; render state is
  switches on the window (`enable`, `blend`, `cull`, `depth_write`); textures bind to a numbered
  unit and uniforms are set by name; `line_width`, `point_size`, `finish` and integer draw modes.
- **GLSL programs that would move**: graphics 6, stage 4, tile-world-game 2, dryopea 4, loft's
  docs and gallery 41, graphics' tests 13, loft's tests 13.

## The shape

```
game, stage ──► gpu (neutral: the loft API, the backend table's C header, no stack named)
                 ▲            ▲             ▲              ▲
          gpu_gl   gpu_metal   gpu_vulkan   gpu_frame   a console's (private repo)
```

- **`gpu`** holds the whole loft API: pipelines (shader, vertex layout, blend, depth, cull
  together), render passes (a target, how it is loaded and stored), uniform blocks bound by slot
  from a loft struct, buffers, textures.  Its C part only passes calls on.
- **A backend** exports one table of C functions, declared in a header `gpu` ships.  The program
  hands the table to `gpu` when it starts.
- **Why a table, not symbols a backend defines**: a `--ship` program links every library's C
  into one binary, where either would work; the interpreter and `--dynamic` load each library's
  C as its own shared library, where a symbol declared in one and defined in another does not
  resolve (C_BACKEND.md § the `[c] shim`).  The table is the one form every mode can carry.  L0
  probes it.
- **Who picks the backend**: never a game (F8's gate).  `gpu_host` asks each public backend in
  turn; a backend on a machine without its stack answers "none", as every platform section does
  (HARDWARE_BACKENDS.md).  A console's host package lives beside its backend, in its repo.
  `loft.toml` has no per-target dependencies, and this shape needs none.

## The rules every step keeps

1. **One thing a step**, verified on the interpreter and the C backend, and on a page where the
   step touches one.
2. **The old path stays until the new one matches it.**  A step that moves drawing compares the
   same scene through both paths on the same machine; only then does the old one go.
3. **Each step names what goes red.**  A new guard is shown to fail on the code before the step
   (`make falsify`).
4. **Pictures are compared, never looked at**: byte for byte on one stack; between two stacks
   within a tolerance that is measured first (M1) and written beside the guard.
5. **A backend is verified on a machine that has its stack**: Metal on a Mac, Vulkan and
   X11 + GLX on a Linux machine, WebGL2 in a headless page.  A step done on another machine is
   not done.
6. **The games are their agents' to change.**  A step that needs a line in a game says so, and is
   verified here on the library's own testbed.

## Track P — the game on a Mac, on the GL that is there

Nothing here waits for the neutral layer or for Metal.

- **P1 — graphics on the C backend on a Mac (S).**  graphics' whole suite with
  `GRAPHICS_TEST_WINDOW=1` on `--native-c`, and one example as a `--ship` build.
  Goes red when: a window test fails on the C backend that passes on the interpreter.
  **Done (2026-11-15, an M3 Max, graphics 0.12.1):** 138 tests in 25 files pass against a real
  window on the interpreter and on `--native-c`.  The shader test's square as a `--native-c
  --ship` program is one 191 KB executable that links only the system library; it reads back
  the hand-computed colours, and fails when its uniform is changed.
- **P2 — `screen` on macOS (M; the macOS half of F3).**  An `NSWindow`, the frame as a bitmap,
  keys, the pointer, the wheel and typed text, through the Objective-C runtime as graphics opens
  its window.  The 2D game plays natively after this step.
  Goes red when: the window's pixels read back differ from the offscreen frame; an injected key
  (graphics' `desktop/input_mac.c` pattern) does not arrive.
  **Done (2026-11-15): loft-libs-game#32, merged** (screen 0.2.0, on the merge of the
  branch the game pins).  `screen/desktop/mac_check.sh` reads the image the window's layer
  holds and posts keys, clicks and wheel steps into its own queue; a window opens through
  loft on the interpreter, `--native-c`, `--ship` and `--native`.  The game's `main`, in a
  scratch copy, built `--native-c --ship`, plays in it: 832 x 640, `show` 2.7 ms a frame.
  Three things it met:
  - the game did not compile on loft's `main`: a body local two loops bind at types that
    differ only in absence was split on pass 2 and left undefined ("Unknown variable
    'l#b1'"); fixed, guard `a_loop_local_split_on_absence_alone_is_defined_where_it_is_bound`;
  - the game still builds a `Canvas` from a `vector<integer>` (`render3d.loft`), which
    graphics 0.12 refuses: the game's agent's to change;
  - a shipped program finds its files beside the executable, not in the working directory,
    and needs `libloft_net.dylib` (the `web` library's component) beside it.
- **P3 — the game's libraries on `main` (S).**  The `screen`, `audio` and `tiles` branches the
  game pins are merged and published, so its fetch list names `main` throughout (the game's
  agent changes the list).
  Goes red when: the game's tests and `checks/game.loft` differ between the pinned branches and
  `main`.
  **Built (2026-11-15), in review:** loft-libs-game#32 carries `screen` and `audio`,
  loft-libs-graphics#81 `tiles`; both branches merged into `main` without conflicts.  `tiles`'
  43 tests pass on the interpreter, `--native` and `--native-c` on a Mac — on the
  interpreter only with loft#2508 fixed (narrow C arguments past the eighth on Apple silicon,
  met here through `tl_draw_ground`).  loft-libs-game#32 is merged; #81 is in review.  The
  game's own tests (a scratch copy at its `main`, with these libraries): 480 in 52 files pass on
  the interpreter and on `--native-c`, and `checks/game.loft` passes under the interpreter —
  which needs loft#2507's fix, reported from the game's side as loft#2510.
- **P4 — a desktop frame the GPU draws (M).**  On a desktop with GL, `screen`'s window carries a
  GL context, `tiles`' display surface is drawn by it as in a page, and
  `graphics::open_fullscreen` answers that same window.  `tiles::on_display` is then true and
  the game's F1 view comes on, with no change expected in the game.
  Goes red when: a frame shown through the GPU surface differs by a byte from the same frame
  shown as a bitmap; `checks/view3d.loft`'s scene captured on the desktop differs from the
  page's beyond the measured tolerance.
  **Tried first (2026-11-15), in a scratch copy of the game:** with the `on_display` gate
  taken out and a `swap` after the draw, the game's own `render3d` draws its first-person view
  in a window `graphics` opens beside `screen`'s, on the Mac's GL 4.1: the same picture from a
  `--native-c` and a `--ship` build, 6.7 ms a frame shipped (9.3 ms unshipped), keys still
  arriving through `screen`.  So the renderer and graphics 0.12 are sound on a Mac; what P4
  builds is only the one window.
  **What P4 costs, measured:** in a page the three libraries meet in the page's one context,
  each through its own `loft_gl` imports (screen's frame, 470 lines of tiles' GPU drawing,
  graphics' whole GL table).  A desktop has no such host, so P4 is three C parts retargeted
  to a context one of them owns — work track L replaces.
  **Ruled (owner, 2026-11-15): the two-window form stands in until L3** gives the game's view
  one window through the neutral layer; P4 as written is not built.  The game's two changes
  are in [FOR_THE_GAME.md](FOR_THE_GAME.md); `screen::program_keys()` is the capability its
  gate asks (true on macOS, where one event queue serves every window of the program).
- **P5 — played, and timed (S).**  The `--ship` build on the Mac, a scripted run (`TW_MOVES`)
  with the play log on.  The number this step records is what M4 is compared against.
  Goes red when: the play log names a slow frame in the scripted run.
  **Measured standing still (2026-11-15, an M3 Max, `--native-c --ship`):** 25 seconds each,
  no slow frame (over 50 ms) in the play log.  2D at 832 x 640: draw 2.4 ms, copy 0.1 ms,
  show 3.0 ms a frame.  The first-person view in its own 960 x 720 window: 6.3 to 6.9 ms a
  frame.  The scripted run is still open: `TW_MOVES` plays only into a picture (`TW_SHOT`),
  so a windowed run needs the game to replay keys (the game's agent's).

## Track L — the neutral layer

- **L0 — probe: a table across libraries (XS).**  Two fixture packages: a layer whose C holds a
  table pointer, and a backend that exports a table.  A program hands one to the other and
  calls through it; a second backend answers "none" and the host skips it.
  Goes red when: the call does not arrive on the interpreter, `--native-c`, `--ship` or a page.
  A red here changes § The shape before anything is built on it.
  **Done (2026-11-15):** the call arrives on the interpreter, `--native-c`, `--native-c --ship`,
  `--ship --dynamic`, `--native` and both writers' pages (headless Chrome).  A backend's C
  includes the layer's header by a path relative to its own package, in every mode.  The guard
  is `a_backend_table_crosses_libraries_in_every_build` (tests/c_emit.rs) over the `lctab*`
  fixtures.  It went red once, for a defect of loft's own, fixed with it: the interpreter found
  a compiled library's function by its bare name, so three backends that each export `table`
  all answered with the first one's (`library_bridge_ident` now leads with the library's name;
  guard `two_compiled_libraries_exporting_one_name_each_answer_their_own`).
- **L1 — the boundary on paper, tried on every caller (S).**  The table's header and the loft
  API, in `API_GPU.md` beside this file, with one row per existing use: graphics' 6 programs,
  stage's 4, the game's view, the instanced and render-to-texture tests.  It rules on what no
  other stack has (triangle fans, line width, point size, `finish`): dropped, or done in the
  layer.  Two things no layer hides get a row each: how a frame is presented (the display's
  frame clock, a resize) and the texture formats a target takes (compressed forms are the ship
  build's, F8b).
  Goes red when: a row cannot be written in the API without naming a stack.
  **Done (2026-11-15): [API_GPU.md](API_GPU.md).**  No row needed a stack named; three rows
  changed the API while it was written (the painter's ambient blend, stage's light arrays and
  its per-run instance offset).  One premise was measured and came out against the plan's
  wording: a uniform block "from a loft struct" has no single layout once a field has three
  components — a `float` after a `vec3` sits at byte 12 on GL and 16 on Metal
  (`probes/l1_block_gl.c`, `probes/l1_block_metal.swift`) — so a block takes no `vec3` and no
  integer, and a backend refuses a program whose own offsets differ from the layer's.  Counted
  as programs, stage has 2 and dryopea 1 (§ Where things stand counts sources).
- **L2 — `gpu` with a recording backend (M).**  The library, and `gpu_record`, a backend that
  writes down every call it receives.  It needs no display, so it runs in every CI job.
  Goes red when: a scripted frame's record differs from the hand-written one; a buffer, texture
  or pipeline is still held after its owner died.
  **Built (2026-11-15), in review: loft-libs-graphics#83** (`gpu` 0.1.0, `gpu_record` 0.1.0).
  `gpu`'s 3 tests and `gpu_record`'s 9 pass on the interpreter, `--native-c` and `--native`
  with warnings denied; one frame's log is byte-equal on the interpreter, `--native-c`,
  `--native-c --ship`, wasm under wasmtime and both writers' pages.  Six planted breaks each
  turn a guard red (a pair aligned on 1, the no-vertices check off, `bind` keeping the block,
  the target kind unchecked, a buffer never released, `F3` allowed in a block).  Two things
  it met: a wasm build's `printf` formats no float, so the recorder writes thousandths as
  whole numbers; and a backtick text drops the indentation of its first content line from
  every line, not the common one, so an expected log is built from a list of lines.
  **Open:** `gpu_record` is not in the library CI's package list — CI resolves `gpu` from the
  registry, and `gpu` is not published while its name is a working name (open question 2).
  C1's mode is `gpu_record::refuse_source`.
- **L2b — a backend finds the layer's header once both are installed (M, in loft).**  L0
  measured the table with the packages side by side under their plain names.  Installed from
  the registry a package's directory is `<name>-<version>`, and a backend's
  `#include "../../gpu/c/gpu_backend.h"` then names no file.  A shim compiles with fixed flags
  and includes only by a path relative to its source (@C158), so nothing a package writes
  cures it.
  Goes red when: `lctab_a`, installed as `lctab_a-0.1.0` beside `lctab-0.1.0`, does not
  compile in any build mode.
  **Done (2026-11-15, owner's choice): @C164.**  A shim writes
  `#include "<dependency>/<header>.h"` for a package its `loft.toml` declares, and loft puts
  that dependency's headers (the `*.h` beside its own shim sources, from the copy the program
  resolved) where every shim compile searches.  They join the shim's cache key and the startup
  cache.  Measured with `gpu` and `gpu_record` under a scratch registry: the same frame log
  on the interpreter, `--native-c`, `--native-c --ship`, `--native`, both writers' wasm and
  both writers' pages, with the backend named before or after the layer and with only the
  backend named.  The guard is
  `a_backend_finds_the_layers_header_wherever_the_two_are_installed` (tests/c_emit.rs): the
  `lctab` fixtures under versioned directory names, each mode cold and from the startup
  cache.  It fails with header staging off, and its warm `--native-c` run fails with the
  cache's `chdr` lines left out.  Three things it met:
  - a backend's loft file need not `use` the layer (only its C includes the header), so its
    shim waits for the layer to register, not for its own file's `use`s;
  - a warm start skips the parse that declared the headers, and a C build compiles a shim
    after it: the first form failed on every second `--native-c` run;
  - the C writer's wasm build compiles shim sources with the unit and had no directory of
    loft's on its search path, so it is a seventh site.
  **Open:** loft-libs-graphics#83 keeps the relative include, which loft's `main` can build
  side by side.  `gpu_record` moves to `"gpu/gpu_backend.h"` and raises its `loft` floor in
  the change that follows @C164 reaching loft's `main`.
- **L3 — `gpu_gl` (M).**  Today's GL code behind the table; graphics 0.12's own API untouched
  beside it.  Shaders arrive as a set: one declaration (inputs, the uniform block, the texture
  slots) with a source per shading language, until track H replaces the sources.
  Goes red when: a scene through `gpu` + `gpu_gl` differs by a byte from the same scene through
  graphics 0.12, on X11, on a Mac and in a page.
- **L4 — graphics' own drawing moves, one part a step (S each).**  `Painter`, `Sprites`,
  `GroupMeshes`, text textures, `quad`: each onto `gpu`, each compared with its 0.12 pixels.
  Goes red when: that part's tests draw another byte.
- **L5 — `stage` moves (M).**  Its four programs and its batching onto `gpu`.
  Goes red when: stage's suite draws other pixels than before the step.
- **L6 — `gpu_host`, and the gate (S).**  The host package of § The shape; F8's lint gains a
  rule: a package other than a host that depends on a backend fails.
  Goes red when: a fixture game that depends on `gpu_gl` passes the gate; a machine with two
  stacks opens other than the first that answers.
- **L7 — graphics' GL-shaped calls go (S; a declared break).**  `Window.shader(text, text)`, the
  state switches, `Texture.bind` and the `GL_*` constants leave graphics; its examples, guide
  and loft's gallery (41 programs) move to `gpu`.
  Goes red when: a gallery page's picture differs from its picture before the step
  (graphics' `browser/check.sh`).
- **L8 — the games move (with each game's agent).**  tile-world-game's view first (2 programs),
  then dryopea.  This stream supplies the library and reports gaps; the game's agent moves the
  code.
  Goes red when: the game's `checks/view3d.loft` scene draws other pixels than on graphics 0.12.

## Track M — the Metal backend

M0 can run at once; M1 onwards need L3, so every Metal step has a GL picture to match.

- **M0 — probe: Metal with no SDK (XS).**  From C99 through the Objective-C runtime: a
  `CAMetalLayer` on the window, a clear to one colour, read back.  It settles whether Metal's
  by-value structures (a clear colour, a region, a viewport) pass through `objc_msgSend` on
  arm64 and x86_64 as AppKit's rectangles do today.
  Goes red when: the pixels read back are not the colour.
  **Done (2026-11-15, an M3 Max):** [probes/m0_metal.c](probes/m0_metal.c) clears an offscreen
  texture and a `CAMetalLayer`'s drawable on an `NSWindow` and reads both back, built for arm64
  and for x86_64 (under Rosetta); with the colour changed it fails.  A clear colour, a region
  and a drawable size pass by value through `objc_msgSend` cast to the method's type on both;
  no method here returns a structure, so `objc_msgSend_stret` is not needed yet.  An Intel
  Mac's own GPU is not measured.
- **M1 — one triangle, and the tolerance (S).**  A shader compiled from source when the program
  runs, one triangle, compared with `gpu_gl`'s.  The difference found is the tolerance rule 4
  asks for, written beside the guard with what causes it (filtering, sample count, colour
  space, derivative precision), so a later surprise is read against a cause.
  Goes red when: the triangle is absent, or the difference is larger than a rounding step.
- **M2 — `gpu_metal`, one capability a step (S each).**  In L2's order of use: buffers and
  indexed drawing; instancing; textures, `put` and mipmaps; depth; blend, cull and scissor;
  render to a texture; capture.
  Goes red when: that capability's scene differs from `gpu_gl`'s beyond the tolerance.
- **M3 — the host picks Metal on a Mac (S).**
  Goes red when: graphics' and stage's suites, run on a Mac through the host, differ from their
  GL run beyond the tolerance.
- **M4 — the game on Metal, timed (S).**  P5's scripted run through `gpu_metal`.
  Goes red when: a frame takes longer than on GL — a finding to explain before Metal becomes
  the Mac's default, never a number to accept.

## Track V — the Vulkan backend

Vulkan is not part of the GL library: graphics speaks OpenGL only (GLX on Linux), and the two
share no calls, no shader form and no driver entry.  What Vulkan adds — work recorded ahead and
on several threads, pipelines built before they are drawn with, no driver guessing at state — is
reached only by a backend of its own.  Every step here is built and verified on a Linux machine
(rule 5); V1 onwards need L3.

- **V0 — probe: Vulkan with no SDK (XS).**  `libvulkan.so.1` opened when the program runs, as
  `libGL.so.1` is today, the few types declared in the file; an instance, a device, an image
  cleared to one colour and read back, with no window.
  Goes red when: the pixels read back are not the colour; on a machine with no Vulkan driver the
  backend answers anything but "none".
- **V1 — a window and one triangle (S).**  A surface on the X11 window (`VK_KHR_xlib_surface`),
  a swapchain, the triangle of M1.  Vulkan takes a shader only as SPIR-V, so this step's program
  is a SPIR-V blob written out once by hand, and H3b replaces it.
  Goes red when: the triangle differs from `gpu_gl`'s beyond the tolerance.
- **V2 — `gpu_vulkan`, one capability a step (S each).**  M2's list, in the same order.
  Goes red when: that capability's scene differs from `gpu_gl`'s beyond the tolerance.
- **V3 — the host's order on Linux (S).**  Vulkan where a driver answers, GL otherwise.
  Goes red when: graphics' and stage's suites through the host differ from their GL run beyond
  the tolerance; a machine without a Vulkan driver opens no window.
- **V4 — timed against GL (S).**  The game's scripted run (P5) on the Linux machine, both ways.
  Goes red when: a frame takes longer on Vulkan than on GL — explained before Vulkan is first in
  the host's order.

## Track H — one shader source

The README's decision: shaders are written in a shading subset of loft, compiled by loft to each
stack's language.  Until H2 a shader set carries hand-written sources (L3); H removes them.

H1 is the gate: if the game's program cannot be written in the subset and drawn byte for byte,
the README's fallback (GLSL compiled to SPIR-V at ship time and translated) is taken for H2–H3b.
H4 is kept either way: it is what gives a shader a pixel test on a machine with no GPU.

- **H1 — probe: one program in the subset (XS).**  The game's view program written as loft
  functions, its GLSL written out by a throwaway emitter.
  Goes red when: it draws other pixels than the hand-written GLSL.
- **H2 — the GLSL emitter (L), then each program rewritten (S each).**
  Goes red when: a rewritten program draws another byte than its original.
- **H3 — the Metal emitter (M).**
  Goes red when: a program differs from its GLSL build beyond the tolerance.
- **H3b — the SPIR-V emitter (M).**  SPIR-V is a list of numbered words, so loft writes it
  itself; no compiler is shipped or called.
  Goes red when: a program differs from its GLSL build beyond the tolerance; the blob fails the
  driver's own validation layer.
- **H4 — the C emitter (M).**  A program run on the CPU: pixel tests for shaders with no GPU,
  and what `gpu_frame` draws with.
  Goes red when: the CPU picture differs from the GPU's beyond the tolerance.

## Track C — what a console backend needs kept free

A console backend is written in a private repo under its maker's licence, against `gpu`'s
header.  Two things are kept true in public so that it can be:

- **C1 — no step assumes a shader can be compiled while the program runs (S, with L2).**
  `gpu_record` has a mode that refuses a shader given as source.
  Goes red when: graphics, stage or a fixture game creates a shader from text in that mode once
  H2 has moved its programs.
- **C2 — the ship build compiles shaders ahead (M, after H2).**  `--ship` for a target writes
  each program in that target's form beside the assets (the README's F8b).
  Goes red when: a second ship build with no change compiles anything; a shipped program draws
  other pixels than its source.

## Order

1. P1, L0 and M0 — a measurement and the two probes; none needs another.
2. P2 → P3 → P4 → P5: the game plays on the Mac.
3. L1 → L2 → L2b → L3 → L4 → L5 → L6 → L7 → L8, with C1 beside L2.
4. M1 → M2 → M3 → M4, from L3 on, on a Mac.
5. V0 at any time; V1 → V2 → V3 → V4, from L3 on, on a Linux machine.
6. H1 at any time; H2 → H3 → H3b → H4 after L3; C2 after H2.

## Open questions

1. **`gpu_frame`** — whether the Tanmatsu's renderer sits under `gpu` (through H4), or stays
   the frame tier's own (`tiles`, [@PLN218](https://github.com/loft-lang/plans/issues/218)).
   Recommended: the second.  Pipelines and passes cost a small machine with no GPU, and the
   frame tier is already the floor every game is held to; H4 then earns its place as the
   shader test path.  The owner's "like the Tanmatsu" may mean the first: the owner's call.
2. **The name** — `gpu` here is a working name; the catalogue's naming rules decide it.

## Review

A sibling stream reviewed this plan against sokol, bgfx, wgpu, Godot and Unreal
(`GPU_LAYER-review.md`, branch `laptop-superinstructions`).  Taken up here: presentation and
texture formats in L1, the tolerance's causes in M1, H1 as the gate for the shader fallback with
H4 kept, and the recommendation on open question 1.  Its M0 question is answered by M0's result.
Its cadence point stands: Metal work needs the Mac, so M's steps are kept small.

## Documentation

- [HARDWARE_BACKENDS.md](../../HARDWARE_BACKENDS.md) — the GPU tier's rule: a backend is a
  library beneath a layer that names none (after L6).
- `API_GPU.md` — the boundary (L1); the `gpu`, `gpu_gl`, `gpu_metal`, `gpu_vulkan` and `gpu_host` READMEs and
  guides; graphics' and stage's READMEs (L4, L5, L7).
- [THIRDPARTY.md](../../THIRDPARTY.md) — Metal and QuartzCore (M2) and Vulkan (V2) as system libraries.
- `make libcatalogue` regenerated; the gallery pages (L7); [ROADMAP.md](../../ROADMAP.md) row.
- The portability gate's new rule in
  [LIBRARY_AUTHORING.md § The library contract](../../LIBRARY_AUTHORING.md) (L6).

## See also

[README.md](README.md) · [API.md](API.md) · [HARDWARE_BACKENDS.md](../../HARDWARE_BACKENDS.md) ·
[C_BACKEND.md](../../C_BACKEND.md) · [TANMATSU.md](../../TANMATSU.md)
