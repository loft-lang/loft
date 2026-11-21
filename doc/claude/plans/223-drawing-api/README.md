<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 223 — The drawing API after 0.14

Tracker: [@PLN223](https://github.com/loft-lang/plans/issues/223).  The issue holds the design
in full (the objects, the calls that go, the frame before and after); this file holds the
state, the steps, and the probes the design rests on.

## Status

**Design written, awaiting approval** (`status:need-approval`).  Steps A1 and A2 are done; B1
waits on a loft release that carries loft#2596, loft#2597 and the escaping-holder fix.

## Goal

A draw is `pass.draw(mesh, material)`, and nothing a draw reads is set anywhere else: a pass
is a value that lives exactly as long as it is drawn into; a material holds its shader, its
values, its textures and its depth, blend and cull; the window holds no setting a draw reads.
That is `gpu`'s own model with names instead of positions.

## Effort + design

- **Effort:** L (B1–B5), S each for A and C1
- **Design:** ✓ for the shape; the timing rows T1 and T3 can still change it

## Phase ordering

| # | Step | Goes red on | State |
|---|---|---|---|
| A1 | `gpu` 0.4.1 declares its break; every dependent asks for it | `loft compat check --full` | done: loft-libs-graphics#119, ten packages published |
| A2 | `graphics` 0.14.0, `stage` 0.21.0, `gpu_gl` 0.3.2 published; the consumers on 0.14 | their suites | published; crawler#22 and dryopea#4 open |
| B1 | `Material` and `Pass` beside the 0.14 calls, both through one core | graphics' tests written twice, same pixels | waits on loft |
| B2 | The window's 2D kit on `gpu_paint`; `Painter` calls become pass calls | `gpu_paint/tests/same_as_graphics.loft` | |
| B3 | `stage` on the new calls | its 166 tests | |
| B4 | The 0.14 calls removed; `gpu::Texture` and `gpu::Target` unwrapped: `graphics` 0.15.0 | the four predictions below | |
| B5 | crawler and dryopea on 0.15 | their pixel probes | |
| B6 | **Decals**: a depth bias in `gpu`'s pipeline state (`State.bias`, units of depth the backend's own way: OpenGL's polygon offset, Vulkan's depth bias, Metal's `setDepthBias`), carried by a material as `Depth.Test` with a bias; `scene3d` and `graphics` draw a decal as a blended quad that tests depth and does not write it | a decal on a wall at a grazing angle: no pixel of the wall shows through it and none of it shows through a nearer wall, the same pixels on OpenGL and Vulkan; `gpu_record` logs the bias | after B1; the three-texture draw (base, light, decal in one shader) comes with B1's material |
| C1 | `canvas` split out of `graphics`, which hands its names on | a canvas-only program building without `gpu_gl` | unprobed: can a library pass on another package's names whole? |
| C2 | A window on any backend (@PLN222 steps 17–23) | — | |

Predictions the build is held against: the pipeline key is integers known when a material
and a mesh are made (0.14 builds a text per draw); no test changes its expected pixels;
`graphics.loft` gets shorter; each consumer's port removes more lines than it adds.

## Why decals want a step of their own

A decal as its own blended draw needs one texture, so the two textures a 0.14 draw takes do
not stop it; what does is that it lies exactly on its surface and fights it for depth.
`gpu`'s state has shape, depth mode, blend, cull and target, and no bias.  Until B6 a decal's
vertex stage moves it towards the eye by hand, tuned per game.  A bias is set differently by
each backend, so it belongs in `gpu`.  One draw that samples three textures or more (base,
light map, decal) waits for B1's material.

## The probes

[`probes/`](probes/) — stand-in types that print what they release, run on the interpreter
and `--native`.  Each file's header says what holds.

| Probe | What it settles |
|---|---|
| `pass_is_a_value.loft` | A pass holding its window ends where its value ends, in a block and in a helper |
| `pass_misuse_is_seen.loft` | An early `return` ends a pass; `present` inside a pass and a nested pass are detectable |
| `materials_share_textures.loft` | Materials share a shader and textures; each texture released once |
| `const_is_deep.loft` | A draw's receiver cannot be `const`: the receiver is what a draw changes |
| `reference_field_drops_once.loft` | loft#2597, the fix the materials stand on |
| `window_moved_into_a_record.loft` | **Why a mesh does not remember its window**: a moved window leaves the mesh naming the old record |
| `holder_outlives_its_window.loft` | A holder returned past its owner draws on a closed window (loft3 has it) |
| `census.sh <src dir>…` | Call sites by kind in a user's source: what each frame pays in state and target switches |

The two that fail today decide where references are allowed: a short-lived pass to its
window, and a material to records that never change.

## Timing backlog

T1–T7 in the issue: a draw with a uniform block on each backend (a real GPU's OpenGL is
unmeasured), finding the pipeline, a whole `stage` frame, the consumers' frames, alternating
materials, a cold native build, opening a window.  T1 and T3 first: they can change the
design.  Run on the build that ships, under a fresh `LOFT_HOME`, on a real GPU and on the
software renderers.

## Cross-arc dependencies

- loft: the three reference fixes in a release; `loft#2598` and `loft#2599` merged, so the
  library CI's compatibility step means something.
- @PLN222 steps 17–23 are this plan's C2.

## Documentation

When B4 ships: `graphics/README.md` § Moving from 0.14; the `stage` and `gpu_paint` READMEs;
the generated `LIBRARIES.md` through `make libcatalogue`.

## See also

- [222-graphics-on-gpu](../222-graphics-on-gpu/README.md) — what 0.14 is built on.
- [LIBRARY_DEPENDENCIES.md](../../LIBRARY_DEPENDENCIES.md) — how A1 was released.
