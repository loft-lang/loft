<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# Direct C binding — history

The record behind [PACKAGES_C_BINDING.md](PACKAGES_C_BINDING.md).

## A shim on the rustc writer's wasm targets (retired, @PLN225)

While the rustc writer built `--native-wasm` and `--html`, loft compiled a shim's C sources
to wasm32 objects itself — `clang --target=wasm32-unknown-unknown`, freestanding, against
the headers of `src/c/wasm/` and loft's own small C library (`loft_wasm_libc.c`: the string
functions, an allocator over the module's memory, `snprintf`, `abort`) — and linked them into
the module (`c_shim::wasm_objects`, cached under `native-auto/`).  The bindings were declared
as typed externs at **wasm32's widths** (ILP32: `long` is 32 bits).  `#native` then named the
**Rust** binding: a hand-written `extern "C"` fn compiled by rustc into a cdylib, with `#c`
its planned sibling.  The section as it stood:

**A package's `[c] shim` reaches both wasm targets; a declared library does not.**  On
`--native-wasm` (wasip2) and `--html` (the browser) loft compiles the shim's C sources to
wasm32 objects itself — `clang --target=wasm32-unknown-unknown`, freestanding, against the
headers of `src/c/wasm/` and loft's own small C library (`loft_wasm_libc.c`: the string
functions, an allocator over the module's memory, `snprintf`, `abort`) — and links them
into the module (`c_shim::wasm_objects`, cached under `native-auto/`).  The bindings of such
a package are declared as typed externs at **wasm32's widths** (ILP32: `long` is 32 bits)
and called directly, exactly as `--native` calls them.  So a library whose native part is a
`[c] shim` of pure computation — `random`, `regex`, `crypto` — is a wasm and a browser
library with no bridge crate, from the one C source.  A program links one copy of loft's
small C library however many shims it carries (`tests/native.rs`
`two_c_shims_link_into_the_wasm_target`).

**A shim built for a browser page sees `LOFT_BROWSER` defined** (the Rust writer's page and
the C writer's alike; a `--native-wasm` module never), and there it may call the page's
`loft_gl` imports — `__attribute__((import_module("loft_gl"), import_name("…")))` — which
`doc/loft-gl-wasm.js` supplies: the engine's render service (BROWSER_INTEROP.md § The one
invariant), not a library's own JavaScript.  graphics 0.10 is the user: its shim's GL
function table is filled from `gg_gl*` imports (one per GL entry point, of the GL
signature) on a page, and through GLX on Linux.  `LOFT_WASM_CC` names another
compiler than `clang`; `gcc` has no wasm backend.

What the shim may use there is the C the small library carries: `<string.h>`, `<stdlib.h>`,
`<stdio.h>`'s `snprintf` (a `stderr` write goes nowhere), and the freestanding headers
clang itself provides (`<stdint.h>`, `<stddef.h>`, `<stdarg.h>`, `<limits.h>`).  A shim that
opens a file, reads a clock or asks for entropy does not compile for wasm32, and that is the
right answer — the browser has none of those — so such a package's bindings are **refused
at the call** on the wasm targets, with the compiler's words in the message, while a program
that never reaches them still builds (`imaging`, whose codec reads files).
