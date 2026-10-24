<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# A library's Rust linked into the binary — the native-boundary class, measured

Taken 2026-10-24 on x86-64 (laptop, lean runtime, `stats.py --lanes native,rust,ship`).  The
gap sweep over the fourteen routines then over 3× ship (`gap-sweep/`, run on the ship form
with `LOFT_RELEASE_PASS_PROBE=1 … --lean`) found a quarter of web `pack_u8`'s instructions
in `__tls_get_addr`: a library loaded as a shared object reaches its own thread-locals
through the general-dynamic TLS model, and is called through a slot bound at start
(@PLN195).  The twin compiles the same body into its executable.

## What the static link answered

`--static` (or `#link static`) links each library's rlib into the binary.  It first had to
be fixed (loft#2162: the package crate was built with `--remap-path-prefix` and loft from a
checkout without, so a shared crates.io dependency such as `cfg_if` reached the link twice).

| routine | dynamic ship | static ship | change |
|---|---:|---:|---:|
| web `pack_u8` | 3.14× | 1.79× | −43 % |
| web `byte_at` | 2.01× | 1.34× | −33 % |
| ssh `byte_at` | 2.00× | 1.34× | −33 % |
| random `rand` | 2.44× | 1.81× | −26 % |
| text2d `write_text` (checked 2.99×) | — | 1.82× | |
| stage `render_stage` (checked 3.28×) | — | 2.09× | |
| graphics `draw_bezier` (checked 3.59×) | — | 2.77× | |

The class reads 2.24× checked (dynamic) and 1.45× ship (static) at the median; crypto's
rows, where the callee does real work, did not move.  The ship lane now builds with `--static` (@PLN201 step 3: the ship build
recompiles every library inside the game's build, so its Rust is linked in as well).

## Where the static link belongs (owner, 2026-10-24)

Only in the SHIP build.  Development uses precompiled libraries wherever possible and does
not chase the last drop of speed.  A static link cannot use a precompiled library: it needs
the library's rlib built against the exact loft build that links it (one rustc, the same
flags, the same shared-dependency SVHs, the class loft#274 and loft#2162 belong to), so it
recompiles the library's native crate from source.  The C-ABI shared object seals its own
dependencies and is what a precompiled library is.  The ship build recompiles every library
from source anyway (@PLN201), so there the static link costs nothing extra; measured on web
and random it added no build time to speak of (1.4 → 1.6 s, 1.5 → 1.5 s) and changed the
binary size by under 3 %.  `--native` and `--native-release` keep the dynamic default.

## Measured on the way, and not kept as levers

- The stack shadow's armed test in every store write (`LOFT_VERIFY_STACK`, a debugging aid):
  removed from the production runtime as the `stack-verify` feature — cbor `encode` −4.5 %,
  `decode` −3.1 %, arguments `parse` −3.0 %, `truncate_to` −3.7 %, median of 19 routines
  about −1 %.
- `(R-FaultFlag)` refused a library function with a loft body as "not a loft function" (the
  interpreter bridge name in `native`); fixed for the trace's sake, no admission moved over
  the 57 census programs.  text2d `draw_quads` is refused for its real reason: `set_pixel`
  stores.
- Lane 16's "regressions since 2026-10-20" (`mesh_emit`, `enum_match`, `f32_build`) are the
  reference, not loft: on that one run every lane-16 twin measured ~1.5× slow while loft's
  native ns stayed flat (`perf_trend.py --routine mesh_emit`).
