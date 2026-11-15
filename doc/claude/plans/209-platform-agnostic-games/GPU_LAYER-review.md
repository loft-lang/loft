<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The GPU layer against the field — a review of `GPU_LAYER.md` (2026-11-15)

A read of arc G's step list the day it was written, against how the graphics libraries and
game engines in use solve the same problem, for the stream building it.  `GPU_LAYER.md` is the
plan; this page is the review and changes nothing in it.  The owner's aim behind both: a loft
game runs unchanged on every machine loft reaches.

## How the field does it, in one table

| pattern | who | what it buys |
|---|---|---|
| a thin layer with a small command model — buffers, textures, samplers, pipelines, passes — each backend one file beneath it | sokol_gfx, bgfx, wgpu, Godot's RenderingDevice (late), Unreal's RHI | everything above is written once; a backend is added without touching it |
| the platform layer (window, input, audio, clock, files) apart from the GPU layer | SDL, GLFW, sokol_app | a renderer port does not drag an input port with it |
| one shader source compiled offline per backend, reflection data for binding by name | sokol-shdc, SPIRV-Cross, naga, Unity | no hand-written dialects; a fixed engine shader set keeps it trivial |
| capability tiers, never backend branches | every engine with more than one backend | the game asks what the machine has; the lowest tier is the contract |
| a software renderer as the floor and the oracle | Godot's compatibility renderer, raylib on llvmpipe, Flutter's Impeller against Skia goldens | a fallback for a machine without a GPU, and the reference every GPU backend is compared with in CI (llvmpipe, lavapipe, SwiftShader, ANGLE) |
| assets compiled per target offline | every engine | texture compression (BC, ETC2, ASTC) is the one thing no abstraction hides |
| a frame and time model the engine owns | every engine | a fixed step and interpolated presentation map onto vsync, a page's animation frame and a board alike |

## What `GPU_LAYER.md` gets right

- **The neutral layer is the pipeline-and-pass model** — the one Metal, Vulkan, Direct3D 12 and
  the console APIs share, with GL emulating it beneath.  The big engines learned to put GL under
  the model rather than on top of it late; sokol and wgpu started there.
- **A backend is a table of C functions handed over at start**, because the interpreter and
  `--dynamic` load each library's C as its own shared object, where a cross-library symbol does
  not resolve, and the table is the one form every build mode carries.  L0 probes it first.
- **"Ask, never detect", and the host picks the backend, never the game**, with a lint that fails
  any package but a host that depends on a backend: tiers as a contract, not a convention.
- **Pictures are compared, never looked at** — byte for byte on one stack, within a tolerance
  measured first between stacks — and a recording backend that needs no display runs the layer
  in every CI job.  This is the language's differential discipline applied to pixels.
- **Track P first**: the game plays on the Mac on the GL already there before the layer exists,
  and leaves a timing number for Metal to be measured against.
- **The console constraint kept in public**: a recording mode that refuses a shader given as
  source, and the ship build compiling shaders ahead — exactly what an NDA backend needs.

## Where it should be sharpened

1. **Track H is the risk, and is sized as one.**  A shading subset of loft with four emitters —
   GLSL, Metal, SPIR-V written as words by loft itself, C — is a second compiler project inside
   the graphics one.  The plan sequences it after `gpu_gl` and keeps hand-written per-language
   sources until then, and names the fallback (GLSL compiled to SPIR-V at ship time, translated
   to the others).  The fallback decision should be a dated gate, not a feeling: if H2's GLSL
   emitter does not draw the game's program byte-identical within one cycle, the fallback is
   taken.  H4, the C emitter, is the part to protect whatever happens: shaders with pixel tests
   and no GPU.
2. **M0 is the right probe and should run first.**  Driving Metal from C99 through the
   Objective-C runtime with no SDK turns on whether by-value structures (a clear colour, a
   viewport, a region) survive `objc_msgSend` on arm64 and x86_64 — `objc_msgSend_stret` and the
   struct-return rules differ between them.  Nothing in track M can be planned around until that
   answer exists; it costs an afternoon on the Mac.
3. **Two things no abstraction hides are absent.**  Texture compression formats differ per
   target and every engine solves it in the asset pipeline; G3b reworks meshes per target, but
   textures are not named.  And presentation — vsync, the swapchain, frame pacing — sits in the
   layer in every engine and is not in the table; `gpu_record` checks calls, not time.
4. **Keep the Tanmatsu out of `gpu`** (open question 1).  A pipeline-and-pass abstraction costs
   a device with no GPU and a small budget, and the frame tier is already the floor every game is
   held to.  H4 earns its place as the shader test path, not as the Tanmatsu's renderer.
5. **Rule 5 binds the work to machines.**  Metal on a Mac, Vulkan on Linux, WebGL2 headless.  The
   Mac in the fleet is the arm64 host the portal names, and the busy one; that is the practical
   constraint on cadence, more than any design choice.
6. **The tolerance between stacks needs its sources named beside the number.**  Filtering,
   multisampling, sRGB conversion and derivative precision are where two correct backends differ;
   a tolerance written without them is re-measured at every surprise.  M1 is the place to list
   them.

Against the table above, the plan covers the layer, the platform split, the single shader
source, the tiers, the software oracle and the no-user-shaders start.  What it lacks is a first
measurement, and M0 and L0 are both an afternoon.
