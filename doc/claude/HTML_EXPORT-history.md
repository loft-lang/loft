<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# HTML export — history

The record behind [HTML_EXPORT.md](HTML_EXPORT.md).

## The page the rustc writer built (retired, @PLN225)

Until @PLN225 (`@C170` in [DESIGN_DECISIONS_PLATFORM.md](DESIGN_DECISIONS_PLATFORM.md)) `--html`
generated Rust for the program and built it with rustc for `wasm32-unknown-unknown` as a
cdylib, against a `libloft.rlib` built for that target.  The sections below are kept as they
stood.  Prerequisites then: the Rust `wasm32-unknown-unknown` target (`rustup target add
wasm32-unknown-unknown`) and `libloft.rlib` + transitive deps for it (installed by `make
install` to `/usr/local/share/loft/wasm32-unknown-unknown/`).  Codegen entry point:
`src/generation/mod.rs` (`output_native_reachable` with `wasm_cdylib: bool`); GL / println /
yield imports were emitted from `src/generation/` (ops dispatch + native registry).

### Pipeline architecture

```
loft --html game.loft
  │
  ├─ Parse + compile (existing)
  ├─ Generate Rust source (existing output_native_reachable)
  ├─ rustc --target wasm32-unknown-unknown --crate-type cdylib
  ├─ wasm-opt -O1 (optional)
  └─ Assemble HTML (base64 WASM + inline JS bridge)
```

**Size:** ~200–400 KB raw WASM, ~70–130 KB gzipped (typical).

The CLI flag is parsed in `src/main.rs` (alongside `--native-emit`).
Output defaults to `game.html` next to the source; override with
`--html out.html`.

### Codegen — cdylib entry point

In normal `--native` mode, the generated Rust has `fn main()`.
For `--html`, codegen instead emits an exported entry point:

```rust
#[unsafe(no_mangle)]
pub extern "C" fn loft_start() {
    loft::codegen_runtime::install_browser_panic_hook();
    let cell = std::cell::UnsafeCell::new(Stores::new());
    // The program's stores, named for the panic hook while it runs.
    let _program = loft::codegen_runtime::browser_program(&cell);
    init(&cell);
    n_main(&cell);
}
```

The `cdylib` crate type produces a `.wasm` module rather than a
binary.  The exported `loft_start` is what the JS bridge calls.

### When a page faults (loft#950)

A browser page reports a fault to the page, not to stderr — on
`wasm32-unknown-unknown` stderr is a sink, so anything written there is
simply lost.

That matters because of how a fault ENDS on this target: a panic aborts,
and abort compiles to the `unreachable` instruction.  So the console's
whole symptom is:

```
RuntimeError: unreachable
```

No message, no location, no stack.  loft#950 was reported as an
undebuggable trap for exactly that reason.  Both fault paths now route
through the same host import `println` uses:

| fault | what the page gets |
|---|---|
| a Rust panic (incl. a failed `assert`) | `loft: panicked at …: <message>` + the loft frames under it |
| a loft `panic(…)` / any `RuntimeError` | the ordinary rendered diagnostic, with `--> file:line:col` |

The loft frames come from the shadow call stack `cr_call_push` already
maintains, and they are the half a reader acts on: the Rust location names
a spot in generated code or in loft's own runtime, which does not say what
the *program* was doing.

```
loft: panicked at prog.rs:502:18:
game.loft:2 assertion failed
  in inner() (game.loft:1)
  in middle() (game.loft:5)
  in main() (game.loft:6)
```

#### A trap is not a panic (loft#1059)

The table above is about a PANIC.  A **trap** — most often the stack running out
under deep recursion — does not run the panic hook at all, so none of that
rendering fires.  What a trap does do is throw into the JS that called the
module, and both page shells now catch it: the exception, the browser's own wasm
backtrace, and — when the message names an exhausted stack — a line saying that
bound belongs to the page's wasm engine rather than to loft, since the same
program halts with loft's diagnostic on `--interpret` and `--native`
([WASM.md § How deep a program can recurse](WASM.md)).

The report does not call back into the module for loft's own frames, and cannot:
a trap leaves the shadow-stack pointer wherever it died, because the epilogues
that would restore it never ran, so a second entry runs off the end of the stack
it just exhausted.  The browser's backtrace is the evidence instead, and
`--names` is what makes its frame numbers resolve.

The catch covers the boot call and the asyncify RESUME pump both — a trap during
a frame loop used to stop the pump silently, which reads as a page that simply
stopped.

**What is still silent:** an allocation failure.  `handle_alloc_error`
aborts without running the panic hook, so a page that OOMs still traps
bare.  That is worth knowing rather than worth fixing — after this, a trap
with no message before it has told you it was not a panic.

The hook is installed by the generated `loft_start`, ahead of `init`, so a
panic during startup is covered.  It is a no-op on every other target,
where stderr works and the native crash reporter owns this job.

#### `--names`: reading the browser's own backtrace (loft#954)

Chrome hands a trap a complete wasm backtrace, and it is the best evidence
there is — ten frames naming the failing function and its whole call chain:

```
[exception] RuntimeError: unreachable
    at wasm://wasm/0168beca:wasm-function[1073]:0x56a035
    at wasm://wasm/0168beca:wasm-function[1054]:0x567983
```

A default page cannot resolve one of those numbers, so the only route left
is bisecting the loft source by hand — rebuild, redrive the browser, move
one `println`.  **`loft --html --names`** makes them resolve.  It costs
roughly 10–15 % page size, so it is opt-in: the people who need it are
debugging and will take the bytes.

It has to do two things, and either one alone is useless:

| half | what it does | if it were missing |
|---|---|---|
| `wasm-opt --strip-dwarf -g` instead of `--strip-debug` | keeps the `name` custom section | no names at all — the default page |
| `#[inline(never)]` on each generated loft function | leaves a frame to name | measured on a four-function program, LLVM folded all of them into `loft_start`; the section then named 616 std/alloc internals and not one loft function |

The names are the generated Rust symbols, which carry the loft name
verbatim: `_RNvCs3DwF3yqkNJQ_4prog17n_part_thumb_wire` is `part_thumb_wire`.
Only loft's own functions are pinned out of line — std, alloc and loft's
runtime still inline freely, so the cost stays on the code the backtrace is
about.

Two things to know before leaning on it:

- **It is a different build.** Pinning functions out of line can move an
  optimiser-sensitive fault.  If a trap reproduces without `--names` and not
  with it, that difference is itself the finding.
- **A build that produced no section says so.**  `--names` is asked for
  exactly when a page must be debugged from its backtrace, so a silently
  nameless page — a binaryen that does not honour `-g` — is reported rather
  than shipped looking identical.

Guarded by `html_names_flag_makes_loft_functions_resolvable_in_a_backtrace`
in `tests/html_wasm.rs`, which asserts both halves and pins the default page
as still stripped, so a green there cannot come from names that were always
present.
