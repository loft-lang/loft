# Reproducible output

One loft binary, given the same program, produces the same C and the same executable — built
twice, and built from another directory.  The loft binary itself is reproducible too; that is
[RELEASE_PUBLISH.md](RELEASE_PUBLISH.md)'s `repro-verify.sh`.  This page is about what loft
PRODUCES.

The check is `scripts/repro-output.sh [--loft BIN] [PROGRAM_DIR [MAIN]]`: silent and exit 0
when the C and the executables agree, otherwise it names what differs.  CI runs it in the C
backend sweep.  `REPRO_KEEP=1` keeps its build directory for a look at the difference.

## A source file is named from its root

The one input a program's sources bring that is not the program is WHERE they lie.  So every
source file is named from its root, in everything a build writes and in every runtime fault
(`src/source_name.rs`, one function for every writer):

| File | Name |
|---|---|
| a standard-library file | `default/<file>` |
| a file beneath the program's directory — its project root (the nearest directory above the main file holding a `loft.toml`), else the main file's own directory | its path from that directory: `main.loft`, `lib/shapes.loft`, `sub/app.loft` |
| any other file (a library found elsewhere) | its path from its package's parent (the directory above its `loft.toml`), else its file name |
| a name already relative | itself |

The sites that apply it: the C writer (`Cx::source_name`), the parser where it makes a call site's `file` argument for an
`assert`, a `panic` or a field check (`Parser::site_file`), and the interpreter's fault
rendering (`RuntimeError::render`).  Compile-time diagnostics keep the absolute path: they
are written into nothing, and an editor opens the file from it.

## Where a fault finds its source line

A fault prints `--> <name>:<line>` and then the source line, read from the file.  The name is
found, in order:

1. as given, from the working directory — a program run from its project directory;
2. beneath `LOFT_SOURCE_ROOT` when that is set (this page is the switch's home);
3. beneath the program's roots — the interpreter, which knows them.

When none holds it, the fault prints its location without the source line.

A placed library's worker parses the library alone, so the program hands it its roots in
`LOFT_SOURCE_ROOTS` (the main directory, a newline, the standard library's), and a fault inside
the library reads the same placed or in-process.  The logger names a record's file from the same
directory, so a `[levels]` path-prefix key (`sub/`) addresses what a fault prints.

## A cached program writes the same C

The whole-program cache ([`startup_cache`](../../src/startup_cache.rs)) skips the parse on a
warm run, so whatever the cold parse did to the disk must still hold.  One such input is a `[c]`
library's file: the C writer opens a library it finds beside its package at start and calls it
through the handle, and links one it does not find by name.  A cold run builds a `[c] shim`'s
library into `native-auto/`.  So the manifest records the file each `[c]` library resolved to
(its `cfnd` line).  A warm run whose libraries resolve differently, for example after
`native-auto/` was removed, is a miss: the cold path rebuilds the shim and writes the same C.
Guard: `a_warm_load_whose_shim_is_gone_writes_the_same_c` in `tests/arc_e_program_cache.rs`.

## What is not reproducible yet

- A program with a `[c] shim` names the shim library by its absolute path (`loft_clibs`,
  under the package's `native-auto/`); `repro-output.sh`'s program has no shim.
- The debug tier (`#line` marks) names the C source it was written as, which carries the
  process id.
- A `--native-wasm` build names its C source with the process id; whether the module records
  that name is unmeasured.
