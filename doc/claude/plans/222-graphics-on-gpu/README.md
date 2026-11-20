<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 222 — graphics draws through gpu

Tracker: [@PLN222](https://github.com/loft-lang/plans/issues/222).  The issue holds the design,
the 23 steps with their code references, and one comment per result; this file is the map.

## Status

**Steps 1–16 SHIPPED 2026-11-20** as `graphics` 0.14.0, `stage` 0.21.0, `gpu` 0.4.1 and
`gpu_gl` 0.3.2.  Open: 6c, 6d, 6f, 17–23.  The API that follows is
[@PLN223](../223-drawing-api/README.md).

## Goal

`graphics` keeps no OpenGL and no C of its own: its window is a `gpu` device, so what
`graphics` draws runs on every backend `gpu` has.  `gpu` stays lean — it holds only what two
backends implement differently — and `graphics` carries everything that does not depend on
the hardware.

## Effort + design

- **Effort:** L
- **Design:** ✓ (the issue's body)

## Sub-arcs

| Steps | What | State |
|---|---|---|
| 1–3 | Probes: a shader's loose uniforms into one block; window switches as pipeline state; cost per draw | done; the cost on a real GPU's OpenGL is unmeasured (@PLN223 T1) |
| 4–6 (A) | `gpu` 0.4.0: `DEPTH_WRITE`, which a clear inside a scissor needs | shipped |
| 6b | A checked build of `gpu_gl` (`GPU_GL_CHECKED`): KHR_debug, an error read per call, a handle register, freed pipelines poisoned | shipped, `gpu_gl` 0.3.1 |
| 6c | An AddressSanitizer leg in the library CI | open |
| 6d | The handle register for Vulkan and Metal | open |
| 6e | A device keeps its backend, and so does what it makes | shipped, `gpu` 0.4.1 |
| 6f | 300 windows opened in a row fail some runs (`graphics/tests/window.loft::test_drop_closes`), on 0.13 too | open |
| 7–12 (B) | `shader_rewrite.loft`, `on_gpu.loft`: uniforms by name onto a block, kept state onto pipelines, a frame as passes | shipped |
| 13–16 | The GPU half of `graphics.loft` over `gpu`; `graphics_gl.c` deleted; `stage` ported; `gpu_gl` owns its window code | shipped |
| 17–23 | A `graphics` window on Vulkan and Metal: a program's GLSL carried to SPIR-V and MSL | open; it is @PLN223 step C2 |

## Rulings the build rests on

- `gpu` stays lean; a gap `graphics` exposes in it is filled in `gpu`, and that is the point of
  drawing `graphics` through it.
- No C is added to `graphics`.
- A `const` that is wrong is removed; no state is hidden to keep one.  So in 0.14 a draw names
  its window, and a parameter that draws or sets a uniform is not `const`.
- A check that costs at run time is a separate build (`#ifdef`), run in tests.

## What it found outside the libraries

| Finding | Where |
|---|---|
| A library's cached artifact carries the `__C_LIBS` of the program that built it first: a path dependency beside an installed copy loads both | loft#2595 |
| A `reference<T>` into a record a function answered writes elsewhere | loft#2596, fixed |
| A record with a `reference<T>` field runs T's `OpDrop` | loft#2597, fixed |
| `loft compat check` passes with an empty install cache; it holds a layout break against the API floor | loft#2599, loft#2598, fixed |

## Documentation

- `graphics/README.md` § Moving from 0.13 (in loft-libs-graphics) — every call that changes.
- [LIBRARY_DEPENDENCIES.md](../../LIBRARY_DEPENDENCIES.md) — the release chain a change to
  `gpu`'s records needs, written from `gpu` 0.4.1.
- When 17–23 ship: `gpu_host`'s README and `graphics`' § Native code and targets.

## See also

- The probes of steps 1–3: branch `plan222-probes` of loft-libs-graphics, `probes/plan222/`.
- The consumers' ports: jjstwerff/crawler#22, jjstwerff/dryopea#4.
