<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 22 — Backend-portable `Renderer` layer

## Status

**Closed — superseded by [@PLN209](../../plans/209-platform-agnostic-games/README.md) arc G**
(the GPU boundary and the shader source, on graphics 0.10's object API).  Kept as the record of
the evaluation; nothing below is current.

Open — design/evaluation, no implementation.  **Prerequisite** for any
native GPU backend (and therefore for native Android/iOS rendering).
Surfaced 2026-05-25 while evaluating Vulkan/Metal backends — see
[`../02-graphics/` § Native mobile backends](../58-graphics/README.md#native-mobile-backends-android--ios--evaluation-2026-05-25).

The decision there: a new GPU backend must plug in at the **high-level
`Renderer`/`Scene` layer** (`render.loft`/`scene.loft` —
`create_renderer`/`upload_scene`/`render_frame`), NOT at the
49-primitive `gl_*` contract, because scene-level ops map cleanly onto
Vulkan/Metal/wgpu while the GL-state-machine `gl_*` calls do not.

That layer is **good enough for the common path** (19 of 20 graphics
examples go through `create_renderer`/`render_frame`; `render_frame`
already runs shadow + color passes + swap internally) but is **not yet a
complete backend-portable contract**:

1. **Shaders are embedded GLSL** in `render.loft` (`uModel`/`uLightVP`/
   PBR + shadow source strings) — GL-specific; a Vulkan/Metal/wgpu
   backend can't consume them as-is.
2. A few examples **escape to raw `gl_*`** — custom shaders
   (`gl_create_shader`/`gl_set_uniform_*`, ~2 examples) and render
   targets / post-processing (`gl_create_framebuffer`/
   `gl_bind_framebuffer`, e.g. `17-post-processing.loft`).  Any script
   that reaches past the Renderer into raw `gl_*` bypasses the backend
   boundary.

This plan closes both gaps so the Renderer/Scene API becomes the single,
GPU-backend-agnostic rendering contract.

## Goal

Make the high-level `Renderer`/`Scene` API the **complete,
backend-portable** rendering contract — portable shaders, scene-level
custom materials, and render-target/post-process passes — so one GPU
backend beneath it serves desktop + web + mobile and no script reaches
into raw `gl_*` for rendering.

## Effort + design

- **Effort:** H — touches the shader pipeline, the material/scene API,
  and every example that escapes to `gl_*`.
- **Design:** ~ partial — boundary located + gaps enumerated; shader-IR
  choice and escape-hatch policy still open.
- **Last touched:** 2026-05-25

## Sub-arcs

| Item | Source | Status |
|---|---|---|
| **A** — Portable shader representation: move `render.loft`'s embedded GLSL to a neutral form (WGSL + `naga` cross-compile, or a small shader IR) so the same built-in materials render on GL / WebGL / wgpu | `render.loft` | Open |
| **B** — Scene-level custom material/shader API that absorbs the `gl_create_shader` / `gl_set_uniform_*` escape hatch | `scene.loft`, `render.loft` | Open |
| **C** — Scene-level render targets + post-process passes (absorb `gl_create_framebuffer` / `gl_bind_framebuffer`; `17-post-processing.loft`) | `scene.loft`, `render.loft` | Open |
| **D** — Audit + lock: no example / user path reaches raw `gl_*` for rendering; `gl_*` becomes backend-internal only (or an explicitly-marked power-user hatch) | `examples/`, tests | Open |

## Phase ordering

1. **A first** — the shader representation is the hardest backend
   coupling and everything else (materials, post passes) references
   shaders.  Pick the IR (see Open questions), port the built-in PBR +
   shadow shaders, verify GL + WebGL still render identically.
2. **B** — custom-material/shader API on top of A; migrate the
   custom-shader examples off raw `gl_*`.
3. **C** — render targets + post-process passes; migrate
   `17-post-processing` and any framebuffer users.
4. **D** — audit the example suite + add a test/lint that fails if a
   non-backend `.loft` calls a rendering `gl_*` primitive.

## Open design questions

1. **Shader IR.**  This is a SOLVED problem — the industry pattern is
   *one source language → SPIR-V (Khronos IR hub) → per-target*
   (GLSL/GLSL-ES/MSL/HLSL/WGSL).  Two mature toolchains:
   - **WGSL + `naga`** (pure Rust; the wgpu ecosystem) — author once in
     WGSL, `naga` emits SPIR-V (Vulkan), MSL (Metal), HLSL (D3D),
     GLSL **and GLSL-ES** (desktop GL **+ WebGL2 — so today's
     `loft-gl.js` keeps working**), and WGSL → WebGPU.  Covers every
     loft backend; no C++ deps; already bundled by wgpu.
   - **glslang + SPIRV-Cross** (Khronos, C++) — keep authoring GLSL,
     compile→SPIR-V→transpile to MSL/GLSL-ES/etc.  Avoids rewriting the
     existing shaders but adds a C++ toolchain + FFI.
   (Higher-level alternatives: NVIDIA/Khronos **Slang**, **rust-gpu**,
   Google **Tint** — not needed here.)
   **Recommendation: WGSL + `naga`** — it is the all-worlds option, is
   pure Rust, pairs with the recommended wgpu backend, and still feeds
   the current WebGL2 backend via GLSL-ES output.  Cost: rewrite the
   built-in PBR + shadow shaders once in WGSL.  Pick glslang+SPIRV-Cross
   only if NOT rewriting the GLSL shaders outweighs taking a C++
   toolchain.
2. **Escape-hatch policy.**  Seal `gl_*` entirely behind the Renderer,
   or keep it as an explicitly-marked power-user hatch (documented as
   GL-backend-only, non-portable)?  Sealing maximises portability;
   a hatch preserves today's flexibility.
3. **Scope of "complete."**  Which advanced features (instancing,
   compute, MRT, custom blend) are in-contract vs deferred?

## Cross-arc dependencies

- **lib_plans/58-graphics** — this completes that plan's
  `Renderer` layer into a backend boundary; the § Native mobile backends
  evaluation is the rationale.
- **Native GPU backend (future, not yet a plan)** — the wgpu (→
  Vulkan/Metal/D3D/GL/WebGPU) backend that delivers native Android/iOS
  is a SEPARATE follow-on plan, **blocked on this one**.  Hand-written
  Vulkan+Metal is explicitly NOT recommended (see 02-graphics eval).
- **MIGRATION.md (trainer phone path)** — independent: the trainer's
  near-term phone path is WebGL-in-webview, which needs neither this nor
  a native backend.

## See also

- [`../02-graphics/README.md` § Native mobile backends](../58-graphics/README.md#native-mobile-backends-android--ios--evaluation-2026-05-25) — the evaluation that surfaced this prerequisite.
- `lib/graphics/src/render.loft` / `scene.loft` — the layer this plan completes.
- ROADMAP.md `F` (Foundation) row `GFX.PORTABLE`.
