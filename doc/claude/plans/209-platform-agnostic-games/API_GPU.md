<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 209 — the GPU boundary: `gpu`'s loft API and the backend table

Step L1 of [GPU_LAYER.md](GPU_LAYER.md): the boundary on paper, tried on every caller there is.
Nothing here is built; L2 builds it against a recording backend.

## The rule it rests on

**Everything a draw depends on is named in the draw's own pass: a pipeline, a block of
uniforms, textures by slot, buffers.  Nothing is left set from an earlier call.**

That is the one thing Metal, Vulkan, Direct3D 12 and the console APIs share and GL does not
ask for.  A caller written to it runs on GL unchanged; a caller that leans on a switch set
earlier (today's `w.enable(GL_BLEND)` before `Painter.rect`) has no meaning on the others.

Where the rule is kept: in `gpu` itself, once.  A draw outside a pass, a draw with no
pipeline, a pipeline whose target shape is not the pass's, a texture slot left empty: `gpu`
refuses each before the backend is called, so a backend never sees one and a caller cannot
pass on one stack and fail on another.

## Measured and assumed

| Claim | State |
|---|---|
| A table of C functions crosses libraries in every build mode | Measured (L0) |
| A uniform block has one byte layout on GL and Metal when no field has three components | Measured (2026-11-15, an M3 Max): [probes/l1_block_gl.c](probes/l1_block_gl.c) and [probes/l1_block_metal.swift](probes/l1_block_metal.swift) answer the same 11 offsets and the size 256 |
| The same block WITH `vec3` fields has two layouts | Measured, same probes: a `float` after a `vec3` sits at byte 12 on GL and at 16 on Metal, and every later field moves with it |
| Every caller's row below can be written with no stack named | Written out below; the code that proves it is L2's |
| Vulkan lays the block out as GL does (both std140) | Assumed; V1 measures it |
| A backend can turn the layer's clip space (y up, depth −1 to 1) and its bottom-left origin into its own | Assumed; M1 (depth), M2's render-to-texture step (the row order of a target) |
| One pipeline per (program, state, target shape) is few enough to build ahead | Counted for the callers here: 18 pipelines in all (§ The callers) |

## The objects

| Object | What it is | Made by | Dies |
|---|---|---|---|
| `Device` | One window (or page canvas) and its GPU | `gpu::open(table, width, height, title)`; a game calls `gpu_host::open(width, height, title)` | `close()`, or with its owner; everything it made is freed with it |
| `Buffer` | Floats for vertices or instances, or an index list | `dev.buffer(data)`, `dev.index_buffer(list)` | with its owner |
| `Texture` | A colour image (RGBA, 8 bits a channel) or a depth image | `dev.texture(w, h)`, `dev.depth_texture(w, h)`, `dev.texture_from(pixels, w, h)` | with its owner |
| `Target` | What a pass draws into, other than the window: a colour texture, a depth texture, or both | `dev.target(colour, depth)` | with its owner; its textures live on |
| `Program` | A shader set: one declaration, a source per shading language | a struct literal (below) | a value |
| `Pipeline` | A program, its vertex layout and its render state, built for one target shape | `dev.pipeline(program, state)`, null when no source fits the backend or it does not compile | with its owner |
| `Block` | The values of a program's uniform block | `pipeline.block()` | a value (`vector<single>` inside) |

An object that outlives its device does nothing when it dies, as graphics' objects do today.

## The loft API

```loft
// A program: what the shader reads, and its text in each language a backend may ask for.
VIEW = gpu::Program {
  vertex:   [gpu::F3, gpu::F3, gpu::F2],       // per vertex: position, shade, texture coordinate
  instance: [],                                // per drawn copy
  block:    [gpu::MAT4, gpu::F4, gpu::F1, gpu::F4],   // mvp, eye, fog distance, fog colour
  textures: 1,
  sources:  [gpu::source("glsl-330", VERT_GL, FRAG_GL), gpu::source("msl", VERT_MTL, FRAG_MTL)],
};

dev   = gpu_host::open(960, 720, "view") ?? return;
solid = dev.pipeline(VIEW, gpu::State { depth: gpu::DEPTH_TEST_WRITE }) ?? return;
hand  = dev.pipeline(VIEW, gpu::State { depth: gpu::DEPTH_OFF }) ?? return;
atlas = dev.texture(512, 512);  atlas.nearest(true);
place = dev.buffer(vertices);
u     = solid.block();

while dev.poll() {
  u.mat4(0, mvp);  u.f4(1, ex, ey, ez, 0.0);  u.f1(2, fog);  u.f4(3, fr, fg, fb, 0.0);
  dev.begin_window(gpu::CLEAR, sky);           // a pass: the target, and what it starts from
  dev.use(solid);  dev.uniforms(u);  dev.sample(0, atlas);
  dev.vertices(place);  dev.draw(0, place.count());
  dev.use(hand);  dev.uniforms(u);  dev.sample(0, atlas);
  dev.vertices(hand_mesh);  dev.draw(0, hand_mesh.count());
  dev.end();
  dev.present();                               // waits for the display's next frame
}
```

| Group | Calls |
|---|---|
| Device | `open`, `poll`, `events`, `width`, `height`, `close`, `present`, `capture() -> vector<u32>` |
| Pass | `begin_window(load, colour)`, `begin(target, load, colour)`, `end()`; `load` is `CLEAR` or `KEEP` |
| In a pass | `use(pipeline)`, `uniforms(block)`, `sample(slot, texture)`, `vertices(buffer)`, `instances(buffer, first_float)`, `indices(buffer)`, `viewport(x, y, w, h)`, `scissor(x, y, w, h)`, `no_scissor()`, `draw(first, count)`, `draw_indexed(count)`, `draw_copies(count, copies)` |
| Buffer | `update(data)`, `count()` |
| Texture | `put(x, y, pixels, w, h)`, `nearest(on)`, `mipmaps()`, `width()`, `height()` |
| Block | `f1(field, x)`, `f2`, `f4`, `mat4(field, m)`, `f4_at(field, index, x, y, z, w)` |

**State** is five fields, each with a default, fixed for the pipeline's life:

| Field | Values | Default |
|---|---|---|
| `shape` | `TRIANGLES`, `LINES`, `POINTS` | `TRIANGLES` |
| `depth` | `DEPTH_OFF`, `DEPTH_TEST`, `DEPTH_TEST_WRITE` | `DEPTH_OFF` |
| `blend` | `BLEND_OFF`, or a source and a destination factor (the six graphics has) | `BLEND_OFF` |
| `cull` | `CULL_NONE`, `CULL_BACK`, `CULL_FRONT`; counter-clockwise is the front | `CULL_NONE` |
| `into` | `INTO_WINDOW`, `INTO_COLOUR`, `INTO_COLOUR_DEPTH`, `INTO_DEPTH` | `INTO_WINDOW` |

**Conventions**, the same on every backend: clip space has y up and depth from −1 to 1; a
viewport's and a scissor's origin is the bottom left; texture coordinate (0, 0) is the first
pixel handed to `texture_from` or `put`, and in a target it is the pixel drawn at clip
(−1, −1); a matrix is 16 floats, column-major.  These are GL's, because every caller there is
was written to them.

## Uniform blocks

A block field is one of `F1`, `F2`, `F4`, `MAT4`, or an array of `F4` (`gpu::f4s(32)`).  The
layout is the layer's, computed in loft from the declaration: a field starts at the next
multiple of its own size (4, 8, 16; a matrix and an array at 16), and the block's size is
rounded up to 16.  That is what std140 and Metal both give for these types (§ Measured).

- **No three-component field.**  A `vec3` is declared `F4` and read as `.xyz`: the two stacks
  place what follows a `vec3` differently.
- **No integer field.**  The one integer a caller sets that is not a texture slot (stage's
  light count) is a whole number far under 2^24, exact as a float.
- **A backend checks, the caller does not.**  When a pipeline is built the backend compares
  the compiled program's own field offsets with the layer's and answers null with both lists
  on stderr when they differ.  A hand-written source that declares a `vec3` in its block is
  refused there, on every stack, not drawn wrong on one.
- **Until track H** a field is its position in the declaration (`u.mat4(0, …)`); H2 makes the
  block a loft struct and the positions go.

## The backend table

`gpu` ships `c/gpu_backend.h`; a backend's C includes it by a path relative to its own
package (L0) and exports one `gpu_backend`.  Handles are the backend's own, 0 for none.

```c
#define GPU_BACKEND_VERSION 1
typedef struct {
    int64_t version;                 /* GPU_BACKEND_VERSION, checked by gpu before any call */
    const char *name;                /* for a diagnostic, never for a branch */
    const char *shading;             /* the source tag this backend takes: "glsl-330", "msl", … */
    /* the host part: G1 replaces it with a surface the platform layer hands over */
    int64_t (*open)(int64_t width, int64_t height, const char *title, int64_t fullscreen);
    int64_t (*poll)(int64_t dev);
    int64_t (*event_count)(int64_t dev);
    void    (*events)(int64_t dev, int64_t *out, int64_t n);
    int64_t (*size)(int64_t dev, int64_t which);          /* 0 width, 1 height, in pixels */
    void    (*close)(int64_t dev);
    /* resources */
    int64_t (*buffer)(int64_t dev, const float *data, int64_t n);
    int64_t (*index_buffer)(int64_t dev, const int64_t *idx, int64_t n);
    void    (*buffer_update)(int64_t buf, const float *data, int64_t n);
    int64_t (*texture)(int64_t dev, int64_t width, int64_t height, int64_t depth);
    void    (*texture_put)(int64_t tex, int64_t x, int64_t y, int64_t w, int64_t h,
                           const uint32_t *px, int64_t n);
    void    (*texture_nearest)(int64_t tex, int64_t on);
    void    (*texture_mipmaps)(int64_t tex);
    int64_t (*target)(int64_t dev, int64_t colour, int64_t depth);
    int64_t (*pipeline)(int64_t dev, const gpu_program *program, const gpu_state *state);
    void    (*release)(int64_t handle);
    /* a frame */
    void    (*begin)(int64_t dev, int64_t target, int64_t load, int64_t colour);
    void    (*use)(int64_t dev, int64_t pipeline);
    void    (*uniforms)(int64_t dev, const float *block, int64_t n);
    void    (*sample)(int64_t dev, int64_t slot, int64_t tex);
    void    (*vertices)(int64_t dev, int64_t buf);
    void    (*instances)(int64_t dev, int64_t buf, int64_t first_float);
    void    (*indices)(int64_t dev, int64_t buf);
    void    (*viewport)(int64_t dev, int64_t x, int64_t y, int64_t w, int64_t h);
    void    (*scissor)(int64_t dev, int64_t x, int64_t y, int64_t w, int64_t h, int64_t on);
    void    (*draw)(int64_t dev, int64_t first, int64_t count, int64_t indexed, int64_t copies);
    void    (*end)(int64_t dev);
    void    (*present)(int64_t dev);
    void    (*capture)(int64_t dev, uint32_t *out, int64_t n);
} gpu_backend;
```

`gpu_program` carries the two input lists, the block's field list with the layer's offsets,
the texture count and the one source pair whose tag is the backend's `shading`; `gpu_state`
the five fields.  A backend with no stack on this machine answers a null table.

The source tag is text the layer compares and never reads: `gpu` holds no list of shading
languages, so a console's tag needs no change in it.

## The callers

One row per program there is.  "Pipelines" counts the (state, target shape) pairs the caller
draws it with.

| Caller | Vertex / instance | Block | Tex | Pipelines | What changes for it |
|---|---|---|---|---|---|
| graphics `Painter` flat | `F3` | `MAT4`, `F4` colour with alpha | 0 | 2: blend off, blend on; depth off | The colour and the alpha become one `F4`.  It blends today "where the window has `GL_BLEND` on": the window's switch becomes which of its two pipelines the painter uses (§ graphics 0.12 on top) |
| graphics `Painter` textured | `F3` | `MAT4` | 1 | 2, as flat | The discard under 0.01 stays in its source |
| graphics `Sprites` | `F3` | `MAT4`, `F4` (the cell's corners) | 1 | 3: the painter's two, and depth test for `Sprites.draw` | Four floats become one `F4` |
| stage sprites | `F3` / `F4`, `F2`, `F4`, `F4`, `F2` | `F2` screen, `F2` camera, `F1` time | 1 | 1: blend (one, one minus source alpha), depth off | `instance_attrib` per run becomes `instances(buffer, first_float)`: the layout is the pipeline's, the offset the draw's.  Scissor per run stays a call in the pass |
| stage lights | `F2`, `F2` | `F2`, `F2`, `F1`, `F1`, `F1` count, `F4[32]`, `F4[32]` | 1 | 1: blend off, into the window | `"uLights[{i}]"` by formatted name becomes `f4_at(field, i, …)`; the light colours widen from `vec3` to `F4`; the count is a float.  Its scene is a `Target` of one colour texture, replaced on a resize |
| the game's view | `F3`, `F3`, `F2` | `MAT4`, `F4` eye, `F1` fog distance, `F4` fog colour | 1 | 2: depth test and write; depth off for the hand | `atlas.put` and nearest filtering stay.  Two pipelines replace the depth switch between draws |
| dryopea's ground and entities (one program, made twice) | `F3`, `F3` | `MAT4`, `F4` colour | 0 | 1: depth test and write, cull back | It is on graphics 0.9's `gl_*` calls, so it moves in one step from there (L8); its state was read from its call list, not from each draw |
| graphics' tests: indexed, lines, points, copies | `F3` (+ an instance `F4`) | `F4` | 0 | 4, one per shape and one with copies | `draw_indexed`, `draw_copies`; a point's size leaves the window (§ What no other stack has) |
| graphics' tests: render to a texture, depth texture | the quad's `F2`, `F2` | — | 1 | 2: `INTO_COLOUR_DEPTH`, then the window | `framebuffer` + `attach` + `render_to` become `target` + `begin(target, …)` |
| `GroupMeshes` | the caller's | the caller's | — | 0 of its own | Only buffers: `upsert` is `buffer` / `update`, `draw_all` takes the caller's pass |
| loft's gallery and docs (41 programs) | — | — | — | — | graphics 0.9's `gl_*` calls in loft's own fixture copy; they move at L7 and add no call the rows above lack (the call list was counted, the programs not read) |

No row needed a stack named.  Three rows changed the API while it was written: the painter's
ambient blend (below), the light arrays (`f4_at`, and no `vec3` in a block) and stage's
per-run instance offset (`instances(buffer, first_float)`).

## graphics 0.12 on top, while both exist (L3, L4)

graphics keeps its 0.12 calls until L7, with `gpu` beneath them from L4 on.  What makes that
possible is one small table inside graphics: its `Window` remembers the switches a caller
set (`enable`, `blend`, `cull`, `depth_write`), and a `Shader` builds a pipeline for each
combination it is first drawn with.  So `w.enabled(cap)` still answers, a painter still
blends where the caller turned blending on, and the same picture comes out.

This table is the only place the old model survives, and it goes with L7.

## What no other stack has

| GL-only thing | Used by | Ruling |
|---|---|---|
| Triangle fans | Nothing: the three draw modes are triangles, lines, points | Not in the API |
| `line_width` | No test, no library, no game | Dropped: a line is one pixel wide; a wider one is two triangles |
| `point_size` | One test | Leaves the window: the vertex stage writes the size, which every stack has |
| `finish` | No test, no library, no game | Dropped: `capture` waits for what it reads |
| Integer draw modes, `GL_*` and `BLEND_*` numbers | graphics' API | `State`'s named values |
| A texture bound to a numbered unit, a sampler uniform set to the unit | Every textured program | `sample(slot, texture)`; the slot is the texture's position in the declaration |
| Uniforms set one at a time by name | Every program | One block, set whole |

## What no layer hides

- **How a frame is presented.**  `present()` hands the frame over and waits for the display's
  next one where the machine has a frame clock; a program that must not wait (a test) asks
  `dev.pace(false)`.  Arc F's F7 gives the clock one home; until then it is the backend's.
- **A resize.**  The window's size is asked each frame (`width()`, `height()`); a `Resize`
  event says when it changed.  A window pass always covers the window as it is then; a
  `Target` keeps its size and is replaced by the caller, as stage does.
- **Texture formats.**  Two are promised everywhere: RGBA with 8 bits a channel, and a depth
  image.  A compressed form is the ship build's (F8b): it arrives as bytes with a format tag,
  through a call this step leaves out, and a backend that cannot take the tag says so before
  the ship build picks it.
- **Shaders compiled while the program runs.**  A console may refuse it (C1).  `Program`
  already separates the declaration from the sources, so a source can be a compiled blob
  under its own tag with no change to a caller.

## What L2 builds from this

1. `gpu`: the loft API above, the header, and the checks of § The rule it rests on.
2. `gpu_record`: every table call written down as a line, with a mode that refuses a source
   (C1).
3. Its guards: a scripted frame's record against a hand-written one; each refusal; the block
   layout against the 11 offsets measured here; every object released when its owner dies.

## See also

[GPU_LAYER.md](GPU_LAYER.md) · [README.md](README.md) · [API.md](API.md) (the 2D tier) ·
[C_BACKEND.md](../../C_BACKEND.md)
