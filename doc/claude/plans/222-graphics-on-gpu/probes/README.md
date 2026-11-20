# Probes of plan 222 (graphics draws through gpu)

Throwaway programs that measured the plan's premises; each becomes a test fixture when its
step is built (loft-lang/plans#222).  Run one from its directory under a display:

```
loft install
MESA_VK_WSI_DEBUG=sw xvfb-run -a loft --interpret src/main.loft
```

| Probe | Asks | Measured on loft main d268b48cc, graphics 0.13.3, gpu 0.3.1 |
|---|---|---|
| `uniforms_and_state` | Do loose named uniforms and window render state lower onto a block and pipelines? | OpenGL: 0 of 3,072 pixels differ from graphics. Vulkan: 31, all on colour edges |
| `instanced_locations` | Does crawler's instanced hex shader (per-copy inputs at locations 5, 6, 7) draw through gpu once the rewrite renumbers them? | OpenGL: 0 of 6,144 pixels differ. Vulkan: 592 differ by 1 in one channel (rounding), 55 by more, all on edges |
| `scissored_clear` | Can a quad stand in for a clear while a scissor is on? | No. OpenGL's clear resets the depth inside the scissor; a quad cannot, with any of gpu's three depth states (1,536 of 3,072 pixels differ each time) |
| `uniform_cost` | What does a draw cost when its uniforms changed? Microseconds for 1,000 draws, native | graphics 3,000. gpu_gl (llvmpipe) 41,000 with a block per draw, 900 with one shared. gpu_vulkan on lavapipe 6,400 / 600; on the GPU (RADV) 320 / 190 |

Run `uniform_cost` with `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json` or `radeon_icd.json`
to pick the Vulkan driver.  Use registry dependencies only: a `path =` dependency on a package that
is also installed loaded both copies here, and the timing was of the wrong one.

The SPIR-V constants in each file are written by `gpu_vulkan/spirv.py`.
