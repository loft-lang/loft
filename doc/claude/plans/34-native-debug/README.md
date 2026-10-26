<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN34 — Native debugging — GDB / LLDB integration for `--native` builds

**Status:** PARKED (2026-07-29) — **NDB.0 shipped, NDB.1/NDB.2 not built.**
`--native-debug` exists (`-Cdebuginfo=2`, no `-O`, generated `.rs` preserved so
`.debug_line` points at a real file), so a GDB user can step the generated RUST today.
There is no `loft-gdb.py` / `loft-lldb.py` and no `.loft.map` sidecar, so `step` does not
yet walk `.loft` lines.

**Trigger to un-park:** a real user who wants source-level `.loft` debugging of a
`--native` binary under stock GDB/LLDB — a CLI/Emacs/vim/Eclipse workflow DAP does not
serve, or an IDE that can only drive GDB-MI. The justification below is a POPULATION
argument, not a named consumer, and the debugging need people actually bring is served by
@PLN120's interpreter debugger — so building NDB.1/NDB.2 now is speculative build
(@PLN102's driver-consumer discipline). NDB.1 is the first phase to build when that
arrives; it also buys "DAP-via-stock-adapter for free", so either consumer gets both.

Note this is NOT the same tool as `loft debug`: that drives the **interpreter**; this is
post-compilation debugging of the binary.

`loft --native` produces a real ELF / Mach-O / PE binary via rustc.
This doc covers how to make that binary debuggable with stock
GDB or LLDB, so users in CLI workflows, Emacs gud, vim termdebug,
Eclipse CDT, KDevelop, and any IDE driving GDB through MI get
**source-level debugging in `.loft` source**, not in the generated
Rust intermediate.

The complementary path for **interpreter-mode** debugging is the
DAP server (LSP.3 in [LSP.md](../../lib_plans/63-lsp/README.md)) — that's the right tool when
the program is interpreted rather than compiled.  This doc is only
about post-compilation debugging.

---

## Why GDB / LLDB and not just DAP

DAP is the modern path; every IDE we care about (VSCode, Eclipse via
DSP4E, JetBrains via LSP4IJ, Neovim via nvim-dap) speaks it.  But:

- A non-trivial population lives in **GDB CLI / LLDB CLI** workflows
  and stays there by choice — kernel hackers, embedded developers,
  systems engineers.
- **Emacs gud** and **vim termdebug** drive GDB directly.
- **Eclipse CDT** and **KDevelop** still use GDB/MI as their primary
  debug backend.
- Stock `cppdbg` / `lldb-dap` adapters that VSCode and Eclipse already
  ship can debug *any* DWARF-bearing binary as long as the DWARF
  points at sensible source files.  If `loft --native` produces such
  a binary, **we get DAP-via-stock-adapter for free** — no `loft-dap`
  fork in the loop, no maintenance burden.

Investment cost: a one-line CLI flag (NDB.0) plus a source-map
sidecar (NDB.1) that's already needed by the LSP.3 DAP server.  The
GDB / LLDB story rides on the same data the in-process DAP server
needs anyway.

---

## Three tiers, independently shippable

| Tier | What user sees | Effort | Status for tier completion |
|---|---|---|---|
| **NDB.0** | Step through the generated `.rs`; set breakpoints by `.rs:line`; inspect Rust-named locals (`var_x`, `_vp_data`, …) | XS | Ships when `--native-debug` exists |
| **NDB.1** | `step` walks `.loft` lines, `bt` shows `.loft` fn names, `info locals` uses the user's identifiers, breakpoints set by `.loft:line` | M | Ships when `loft-gdb.py` + `loft-lldb.py` are auto-loaded |
| **NDB.2** | Stock GDB / LLDB needs no plugin — `.debug_line` and `.debug_info` already point at `.loft` source.  Eclipse CDT, Emacs gud, vim termdebug "just work" | MH | Ships when DWARF rewrite is reliable across rustc versions |

Each tier is a strict superset of the previous.  Most users will get
"good enough" from NDB.1 — the plugin auto-loads from a standard GDB
path, no per-user setup.  NDB.2 is polish for users running stock
debuggers in environments where Python plugins are unavailable
(some embedded GDB builds, certain CI runners).

---

## NDB.0 — `--native-debug` CLI flag

### Surface

```
loft --native --native-debug program.loft
loft --native --native-debug --native-emit out.rs program.loft
```

`--native-debug`:
- passes `-Cdebuginfo=2 -g` to rustc,
- omits `-O` (debug builds; opt level 0) — combine with
  `--ship` if you want optimised + debug-info,
- preserves the `/tmp/loft_native.rs` (or `--native-emit` target) on
  disk so the DWARF's source-line entries point at a real file,
- emits a `.loft.map` sidecar next to the binary (NDB.1 consumes this;
  NDB.0 emits it but doesn't use it).

### Implementation sketch

`src/main.rs` already builds the rustc command.  Two new flags:
- `native_debug: bool` — push `-Cdebuginfo=2`, drop `-O` unless
  `--ship` was also passed.
- Always emit the `.loft.map` (negligible cost; LSP.3's DAP server
  consumes it from the same location).

The `.rs` file already lives at `/tmp/loft_native.rs`; rustc will
record it as the source-of-truth in DWARF's `.debug_line` table.
GDB / LLDB will offer to step through it as long as the file is
present at debug time.

### What users see

```
$ loft --native --native-debug hello.loft
   Compiling /tmp/loft_native.rs (debug, with debuginfo)
   Output: /tmp/loft_native_bin

$ gdb /tmp/loft_native_bin
(gdb) break n_main
(gdb) run
Breakpoint 1, n_main () at /tmp/loft_native.rs:42
42         let var_x: i64 = 5;
(gdb) step
43         let var_y: i64 = var_x + 7;
(gdb) print var_x
$1 = 5
```

Variable names are rust-internal until NDB.1.  Source file is
`/tmp/loft_native.rs`, not the user's `.loft`.  But the basic motion
— step, break, print — works.

---

## NDB.1 — source map + GDB / LLDB plugins

### `.loft.map` format

A small JSON file written next to the binary at compile time.

```json
{
  "version": 1,
  "rs_file": "/tmp/loft_native.rs",
  "binary": "/tmp/loft_native_bin",
  "lines": [
    { "rs": 42,  "loft_file": "hello.loft", "loft_line": 3 },
    { "rs": 43,  "loft_file": "hello.loft", "loft_line": 4 },
    { "rs": 87,  "loft_file": "lib/foo.loft", "loft_line": 12 }
  ],
  "vars": [
    { "rs": "var_x", "loft": "x" },
    { "rs": "var_y", "loft": "y" },
    { "rs": "_vp_data", "loft": "data.ptr (synthetic)", "synthetic": true }
  ],
  "fns": [
    { "rs": "n_main",      "loft": "main",      "loft_file": "hello.loft" },
    { "rs": "n_factorial", "loft": "factorial", "loft_file": "lib/foo.loft" }
  ]
}
```

Naming convention: `<binary>.loft.map` (e.g. `/tmp/loft_native_bin.loft.map`).
The plugins look it up by stripping the binary's path and appending
`.loft.map`.

### Codegen instrumentation

Loft's native codegen at `src/generation/` walks the IR and emits
Rust.  At every emit site it already knows:
- the loft definition (`d_nr`),
- the position in the source (line, col) — recorded in `Lexer`
  during parsing and threaded through `Definition::position`,
- the variable `name` for every `let var_X` it writes.

The map is built incrementally: every time codegen emits a Rust line
that corresponds to a loft statement, it appends an entry.  Every
`let var_X` that mirrors a loft local appends a `vars` entry.

Most-relevant files:
- `src/generation/mod.rs::output_function` — fn entry, emit `fns` row.
- `src/generation/dispatch.rs::output_block` — statement loop, emit
  `lines` row per loft statement.
- `src/generation/calls.rs` and `src/generation/expressions.rs` —
  emit `vars` rows when introducing temporaries.

Output: `let map = SourceMap::default(); ... map.write(path)?;`
where `SourceMap` is a small new struct in `src/source_map.rs`
that serializes via `serde_json` (already pulled in for the
`registry` feature).

### `loft-gdb.py` plugin

```python
# Loaded automatically by GDB when /tmp/loft_native_bin is debugged
# if the user has `add-auto-load-safe-path` set for the loft install.
# Otherwise: `(gdb) source /path/to/loft-gdb.py`.

import gdb, json, os

class SourceMap:
    def __init__(self, binary_path):
        map_path = binary_path + ".loft.map"
        if not os.path.exists(map_path):
            self.lines, self.vars, self.fns = {}, {}, {}
            return
        m = json.load(open(map_path))
        self.lines = {(m["rs_file"], r["rs"]): r for r in m["lines"]}
        self.vars  = {v["rs"]: v["loft"] for v in m["vars"]}
        self.fns   = {f["rs"]: f for f in m["fns"]}

class LoftFrameDecorator(gdb.FrameDecorator.FrameDecorator):
    def __init__(self, frame, smap):
        super().__init__(frame)
        self.smap = smap
    def function(self):
        rs_name = super().function()
        return self.smap.fns.get(rs_name, {}).get("loft", rs_name)
    def filename(self):
        rs_name = super().function()
        return self.smap.fns.get(rs_name, {}).get("loft_file",
            super().filename())
    def line(self):
        rs_line = super().line()
        rs_file = super().filename()
        return self.smap.lines.get((rs_file, rs_line), {}).get(
            "loft_line", rs_line)

class LoftFrameFilter:
    def __init__(self):
        self.name = "loft"
        self.priority = 100
        self.enabled = True
        gdb.frame_filters[self.name] = self
    def filter(self, frame_iter):
        smap = SourceMap(gdb.current_progspace().filename)
        for f in frame_iter:
            yield LoftFrameDecorator(f, smap)

LoftFrameFilter()

# Pretty-printer for var_X locals — rename to user identifier
class LoftLocalPrinter:
    def __init__(self, val, loft_name):
        self.val, self.loft_name = val, loft_name
    def to_string(self):
        return f"{self.loft_name} = {self.val}"

def lookup_loft_local(val):
    smap = SourceMap(gdb.current_progspace().filename)
    sym = val.symbol
    if sym and sym.name in smap.vars:
        return LoftLocalPrinter(val, smap.vars[sym.name])
    return None

gdb.pretty_printers.append(lookup_loft_local)
```

This is the v1 sketch; the real plugin will need:
- A breakpoint translator: `b hello.loft:3` → `b /tmp/loft_native.rs:42`.
  Done by overriding `gdb.Breakpoint.__init__` to consult the source
  map when the spec contains `.loft`.
- A frame decorator that hides the synthetic `_vp_*` / `_vc_*` Rust
  locals (DbRef pointer + count pairs) from `info locals`, so users
  don't see implementation noise.
- A `loft-bt` convenience command alias that prints `bt` with the
  filtered frames first.

### `loft-lldb.py` plugin

LLDB's Python API is similar to GDB's but uses different class names.
The same source map drives a `lldb.formatters.cpp.SBValuePrinter`,
`lldb.SBFrame.GetFunctionName` override via a `target.frame-format`
custom format string, and a Python script that loads via
`command script import loft-lldb.py`.

macOS users default to LLDB; Linux users to GDB.  Both plugins ship
in `tools/native_debug/` and install to `~/.loft/native_debug/` via
`loft install`.

### Auto-loading

GDB consults `~/.gdbinit` and `add-auto-load-safe-path` directives.
The cleanest install:

```
$ loft install --native-debug-plugin
   wrote ~/.loft/native_debug/loft-gdb.py
   wrote ~/.loft/native_debug/loft-lldb.py
   appended to ~/.gdbinit:
       source ~/.loft/native_debug/loft-gdb.py
   appended to ~/.lldbinit:
       command script import ~/.loft/native_debug/loft-lldb.py
```

User-visible: `loft install` becomes the one-liner that wires up
debugging in their existing toolchain.

### Ship criterion

A test fixture (`tests/native_debug/hello.loft`) compiled with
`--native-debug`, debugged through a scripted GDB session
(`gdb --batch -x tests/native_debug/expect.gdb`), produces exactly
the expected stack trace + variable names — all in `.loft` terms.
Same fixture under LLDB.  Cross-checked against `cppdbg` (VSCode
adapter) to confirm the DAP-via-stock-adapter path also works.

---

## NDB.2 — DWARF rewrite

### Goal

Stock GDB / LLDB / `cppdbg` / `lldb-dap` need no Loft plugin and no
source-map sidecar — they read the binary's own DWARF and see `.loft`
files directly.

### Mechanism

Post-process the rustc output ELF (or Mach-O / PE):
1. Read `.debug_line` and `.debug_info` sections via `gimli`.
2. For each line-program entry, look up `(rs_file, rs_line)` in the
   source map and replace with `(loft_file, loft_line)`.
3. For each DWARF DIE that names a local variable (`DW_TAG_variable`),
   replace the name with the loft identifier.
4. For each `DW_TAG_subprogram` (function), replace `DW_AT_name` with
   the loft fn name and `DW_AT_decl_file` / `DW_AT_decl_line` with
   the loft source location.
5. Write the modified sections back via `object` crate's writer.

### Risks

- **rustc's DWARF version drift.**  rustc upgrades occasionally bump
  DWARF versions (3 → 4 → 5).  `gimli` handles all of them but the
  rewrite logic must be tested against each major rustc release.
  Mitigation: a CI job that compiles the test fixture with the
  current rustc, runs the rewriter, and validates the output with
  `dwarfdump`.
- **Inlined functions.**  rustc inlines aggressively; one Rust
  source-line entry can correspond to multiple loft lines after
  inlining.  Mitigation: emit DWARF inline subroutine info that maps
  the inlined call site to the loft caller.
- **Optimised builds.**  `--ship` + `--native-debug` produces
  DWARF where source-line ordering is non-monotonic (the optimiser
  reorders code).  GDB handles this fine in stock Rust binaries; the
  rewrite preserves line ordering by entry not by file order, so it
  inherits the same handling.
- **Cross-platform consistency.**  ELF / Mach-O / PE have different
  section name conventions.  `object` crate abstracts this; tests
  must run on Linux, macOS, and Windows MinGW.

### Why MH not L

`gimli` and `object` do all the heavy lifting; the rewriter logic is
~500-1000 lines of Rust.  The hard part is the test matrix (every
rustc version × every platform × every binary format).

### When to ship

NDB.1's plugin satisfies most users.  NDB.2 ships when:
- Eclipse CDT / KDevelop users complain about the plugin install step,
  OR
- A CI job needs to debug a Loft binary without setting up Python
  plugins,
- A user-supplied binary needs to be debuggable from any toolchain
  the user already has.

Realistic timeline: 1.1+.

---

## Platform realities

| Platform | GDB | LLDB | Verdict |
|---|---|---|---|
| Linux x86_64 | Stock GDB + Python plugin works perfectly | LLDB available, plugin works | **Primary target** for NDB.0/1 |
| Linux aarch64 | Same as x86_64 | Same | Secondary target |
| macOS x86_64 / aarch64 | GDB requires codesigning hassle; most users default to LLDB | System default; plugin works out of box | LLDB plugin is mandatory; GDB plugin is best-effort |
| Windows MSVC | GDB doesn't read PDB; users debug via VS / WinDbg / `vscode-cpptools` | n/a | NDB.0 only (stock VS debugger sees the rustc-emitted PDB) |
| Windows MinGW | Stock GDB works; Python plugin works | n/a | Same support as Linux |

`vscode-cpptools` (Microsoft's DAP adapter) drives either GDB, LLDB,
or the Visual Studio debugger transparently.  Tier 1 + 2 binaries
work in VSCode automatically through that adapter — Loft doesn't
need to ship a custom DAP adapter for native binaries.

---

## Overlap with LSP.3 — shared source map

The `.loft.map` sidecar is **the same file** that LSP.3's DAP server
needs in-process for breakpoint resolution.  Same producer
(codegen), same format (JSON), same consumers:

- `loft-dap` reads it to translate `(file, line)` breakpoints to
  bytecode positions.  *(Actually — LSP.3 targets the interpreter,
  not the native binary.  But when LSP.3 grows native-mode support,
  it'll consume the same map.)*
- `loft-gdb.py` / `loft-lldb.py` read it for stack-frame and local
  rewriting.
- A future `loft trace` profiling tool reads it to correlate samples
  with `.loft` source.

Decision: emit the source map unconditionally on every `--native`
build (cost: tens of milliseconds + a few KB on disk per binary),
not just under `--native-debug`.  That way debugging existing builds
works without recompilation.

---

## Sequencing

| Tier | Milestone | Depends on |
|---|---|---|
| NDB.0 | 0.8.5 (XS — fits as a sibling of SH.2 / DX.1) | nothing |
| NDB.1 | 0.9.0 (M — natural sibling of LSP.3 since they share the source map) | NDB.0 + LSP.3's source-map producer |
| NDB.2 | 1.1+ (MH — polish; most users are happy with NDB.1) | NDB.1 |

NDB.0 lifting to 0.8.5 (from the original "next to LSP.1 in 0.8.6")
is fine — it's literally one CLI flag and a `-Cdebuginfo=2` arg.  No
LSP dependency.

---

## Cross-references

- [LSP.md](../../lib_plans/63-lsp/README.md) — LSP.1 / LSP.2 / LSP.3 design.  LSP.3's DAP server
  consumes the same source map as the GDB plugin.
- [NATIVE.md](../../NATIVE.md) — `--native` codegen pipeline; this doc adds
  a debug-info concern on top of the existing emit.
- [STACKTRACE.md](../../STACKTRACE.md) — TR1.3 `vector<StackFrame>` API,
  used by interpreter-mode DAP.  Native-mode debugging instead reads
  the OS-level call stack via DWARF unwinding.

---

## Landing procedure (NDB.0 in 0.8.5 release)

Per-item ship criteria for NDB.0 in the 0.8.5 release.
Generic per-release content (tooling prerequisites, repository
hygiene gates, cross-platform smoke, release artefacts) lives
in [`../../../RELEASE.md`](../../RELEASE.md).  NDB.1 + NDB.2
landing procedures will be filled in when those phases land
(0.8.6 / 0.9.0+).

### NDB.0 — `--native-debug` flag (DWARF in `--native` builds) — ⬜ OPEN

**Effort:** XS (~1-2 hours).

Build checklist (per § NDB.0 design above):
- Add `--native-debug` CLI flag in `src/main.rs` (parse + bool flag).
- When flag is set:
  - Push `-Cdebuginfo=2 -g` to the rustc invocation.
  - Drop `-O` (debug builds; opt level 0) UNLESS
    `--ship` is also set.
  - Preserve `/tmp/loft_native_<pid>.rs` (per-process tmp) on
    disk so DWARF's `.debug_line` table points at a real file.
    Today's behaviour deletes it after compilation.
  - Emit a `.loft.map` sidecar next to the binary (NDB.1
    consumes this; NDB.0 emits it but doesn't use it).  Defer
    the format of `.loft.map` to NDB.1; for NDB.0 either emit
    a stub `{}` or skip emission entirely (decide at impl time).

Quality gates (before ship):
- **Build works** — `loft --native --native-debug examples/hello.loft`
  produces a runnable binary.  Cache key now includes the
  debug flag so switching between `--native` and
  `--native-debug` doesn't reuse the wrong binary.
- **Binary contains DWARF** — verify with
  `objdump -h <binary>`.  Should show `.debug_info`, `.debug_line`,
  `.debug_str`, `.debug_aranges`, `.debug_abbrev`, `.debug_ranges`
  sections.
- **GDB can launch the binary** — `gdb <binary>` then `run`,
  `start`, `break n_main`, `step`, `info locals`.  All work.
  Source view shows the **generated Rust** intermediate (NOT
  yet the loft source — that's NDB.1).
- **LLDB equivalent** — `lldb <binary>` then `process launch`,
  `breakpoint set --name n_main`, `step`, `frame variable`.
- **Tmp source preserved** — the `/tmp/loft_native_<pid>.rs`
  file referenced by DWARF's `.debug_line` exists for the
  binary's lifetime.  GDB / LLDB shows source on `list`.
- **Cache invalidates correctly** — running with `--native`
  then `--native-debug` produces two distinct cached binaries.
- **Performance penalty acceptable** — debug build runs
  noticeably slower than release.  Document trade-off in `--help`.
- **`--ship` flag** — optional sibling combining
  `-Cdebuginfo=2` (debug symbols) WITH `-O` (optimisation).
  Useful for production debugging.  May land in NDB.0 or
  defer to NDB.1 (decide at impl time).

Cross-platform notes:
- **Linux primary** — `gdb` is the reference debugger.  DWARF
  is the standard format on ELF.
- **macOS** — `lldb` is the reference; DWARF embedded in
  Mach-O.  `dsymutil` may or may not be needed for
  externalised debug info (default to embedded).
- **Windows** — `lldb` works on PE; `gdb` via MinGW also works.
  PDB format is the Microsoft alternative; out of scope for
  NDB.0 (Rust's PDB output via MSVC toolchain works
  automatically when `-Cdebuginfo=2` is set).

Risks:
- Binary size growth — debug info is large.  Document the
  trade-off; users opt in via the flag.
- Cache key collision risk if the flag isn't included in the
  hash.  Tested by the cache-invalidation gate above.
- DWARF version compatibility — Rust emits DWARF 4 by default;
  ancient `gdb` versions may need DWARF 2 or 3.  Specify
  minimum versions in `--help` text.
