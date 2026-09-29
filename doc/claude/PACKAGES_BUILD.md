
# Package build, test and targets

The package test suite, `loft build`, `loft install`, the build pipeline and the target matrix.  Split out of [PACKAGES.md](PACKAGES.md).

---

## Package test suite

Tests live in `tests/` alongside `src/` in the package root; each
`.loft` file is a test module.  Zero-parameter functions named
`test_*` (plus `main`) are discovered and run in isolation with a
fresh `State` — same rules as `loft --tests` on the main project.
The runner adds `src/` to the import path so `use graphics;` works,
sets cwd to the package root for fixture paths, and honours
`@EXPECT_FAIL` / `@EXPECT_ERROR` / `@EXPECT_WARNING` / `@IGNORE` /
`@ARGS` annotations.

**Every `@EXPECT_ERROR` substring must match an error**, not just one of them.
While any single matching error satisfied the whole set, a file with three
annotations and one live diagnostic passed — so an expectation could be reworded
out of existence and the run stayed green (loft#929).  `@EXPECT_WARNING` already
held itself to that bar; the two now agree.

### Layout

```
graphics/
├── src/
├── tests/
│   ├── draw.loft
│   ├── math.loft
│   ├── integration.loft
│   └── fixtures/           # PNG / text / binary test data
```

### Running

```bash
loft --tests graphics/tests                # interpreter
loft --tests graphics/tests/draw.loft      # single file
loft --tests graphics/tests/draw.loft::test_clear_canvas
loft --tests --native graphics/tests       # native backend
loft --tests --native-wasm graphics/tests  # WASM
```

Inside a package directory, `loft test` is shorthand for
`loft --tests tests/` with `src/` on the lib path; accepts optional
`<file>` or `<file>::<fn>` targets.  CI: `loft test` exits 0 on
all-pass, 1 on any unexpected failure.

### Manifest config

```toml
[test]
lib      = ["../other-package/src"]  # extra --lib dirs
skip     = ["tests/webgl.loft"]      # files or patterns
timeout  = 10                        # seconds per test (default 30)
fixtures = "tests/fixtures"          # copied by `loft install`
```

### Fixtures

Test data under `tests/fixtures/`.  Text via `file(...).lines()` or
`.content()`; binary via `f#format = LittleEndian; f#read(n) as T`.
Declare `fixtures = "tests/fixtures"` in `[test]` so `loft install`
copies it alongside the source.

**Update-or-compare pattern** for reference files:

```loft
fn update_or_compare(path: text, actual: text) {
  ref = file(path);
  if ref#format == NotExists {
    uc_f = file(path); uc_f += actual; uc_f += "\n";
  } else {
    expected = ref.content();
    assert(actual + "\n" == expected, "output differs from {path}");
  }
}
```

First run writes the reference; later runs compare.  Delete the
fixture to regenerate after an intentional change.

**Binary output** (PNGs, meshes): compare `f#size` bounds + re-load
and inspect structural fields rather than byte-identity, since
compression may vary across libraries.

### Test discovery rules

| Pattern | Discovered? |
|---------|-------------|
| `fn test_foo()` | yes (zero-param, `test_` prefix) |
| `fn main()` | yes (always an entry point) |
| `fn helper(x: integer)` | no — has parameters |
| `fn count() -> iterator<integer>` | no — returns an iterator |
| `fn _internal()` | no — leading `_` marks private helper |

---

## The build phase — `loft build` (@PLN100 Slice 2)

`loft build [target...]` builds a project's declared or default **targets** — a
target is a named (shape × triple × feature-set) with the toolchain it `requires`.
Built-in targets are **implicit**, so a zero-config project builds with no
`[build]` section:

| Target | Shape → action | Triple | Requires |
|---|---|---|---|
| `native` | `--native --check` (compile-check, no run) | host | rustc/cargo |
| `html` | `--html` (browser page) | `wasm32-unknown-unknown` | rustup `wasm32-unknown-unknown`; `wasm-opt` (soft) |
| `wasi` | `--native-wasm` (WASI `.wasm`) | `wasm32-wasip2` | rustup `wasm32-wasip2` |

```bash
loft build              # build [build] default-targets (or `native` if unset)
loft build html wasi    # build the named targets
loft build path.loft    # a .loft positional overrides the entry
```

The runtime wasm rlib each shape links is auto-built + isolated by Slice 1
(`target/loft/<shape>/`), so `loft build html` needs no prior `make`.

A `[build]` section **overrides** built-ins or **adds** targets. The
line-scanner manifest reader (`src/manifest.rs`) takes single-line arrays and a
`[build.target.<name>.requires]` **subtable** (not an inline table):

```toml
[build]
default-targets = ["native", "html"]   # what `loft build` makes with no args

[build.target.html]                      # overlay: keep the built-in shape/triple,
features = ["random", "png"]             # replace the features it builds with

[build.target.html.requires]
rust-targets = ["wasm32-unknown-unknown"]
tools = ["wasm-opt"]

[build.target.mobile]                    # a NEW named target
shape = "html"                           # required for a non-built-in name
triple = "wasm32-unknown-unknown"
```

Before compiling each target, `loft build` **doctor-checks** its `requires`: a
missing rustup target is a HARD failure (skip the target, print `rustup target
add …`); a missing tool is a SOFT warning (build proceeds). It exits non-zero if
any target fails. Resolution + requires logic lives in `src/build_phase.rs`
(unit-tested); the driver re-invokes this loft binary with the shape's flags per
target.

### Asset steps — `[[build.asset]]` (@PLN100 Slice 3)

A `[[build.asset]]` is a custom command that turns `inputs` into `outputs`, run
by `loft build` **before** the targets it feeds — but only when **stale**:

```toml
[[build.asset]]                 # built from local files
name    = "atlas"
run     = "scripts/pack_atlas.loft"   # a single .loft script, or any shell command
inputs  = ["art/**/*.png"]      # content-fingerprinted -> rebuild only on change
outputs = ["assets/atlas.bin"]  # a missing output forces a rebuild
targets = ["html"]              # runs only when `html` is being built (omit = always)

[[build.asset]]                 # fed by an EXTERNAL source
name     = "dataset"
run      = "scripts/fetch.loft"
outputs  = ["assets/dataset.pak"]
lifetime = "30d"                # freshness TTL — rebuild when older than this
```

**Staleness** = `output missing` **OR** `no prior build` **OR** `inputs content
changed` **OR** (`lifetime` set **AND** the output is older than it) **OR**
`--force`. A fingerprint of the inputs' content + a wall-clock build time are
stamped to `.loft/build/<name>.stamp`; the input fingerprint controls
*re-run-on-change*, the `lifetime` TTL controls *re-fetch-on-age* for
external-source outputs (no instrumentation of the source). `loft build --force`
(or `--fresh`) rebuilds every asset for a deterministic clean build (CI can pin
it). `lifetime` units: `s` `m` `h` `d` `w` `mo` (=30d) `y`.

`run` executes as a `.loft` script (with this loft binary) when it is a single
`.loft` path, else through the platform shell — trusted-by-declaration (it is the
project's own manifest; @PLN100 open question 3 / @PLN86). Input globs support
`*`, `?`, and `**`.

### Test phase — `[[test]]` + `loft check` (@PLN100 Slice 4)

`loft check` is the **build + test gate**: it builds the default targets, runs the
asset steps, then runs the declared `[[test]]` phase — one exit code for CI. A
`[[test]]` runs a `.loft` script over one or more **execution-backend** `targets`,
gated on the asset outputs it `needs`:

```toml
[[test]]
name    = "smoke"
run     = "tests/smoke.loft"
targets = ["interpret", "native"]  # run the SAME suite through each backend
needs   = ["atlas"]                # skip (and fail the gate) if `atlas` didn't build

[[test]]
name   = "atlas-integrity"
run    = "tests/check_atlas.loft"
inputs = ["assets/atlas.bin"]      # a test OVER a generated data file
```

- **Backends:** `interpret` (→ `loft <run>`) and `native` (→ `loft --native <run>`)
  — mirroring loft's own interpret+native harness. `html` / `wasi` have no headless
  runner yet and are reported as skipped (not silently passed).
- **`needs`** gates a test on named assets: if a needed asset's outputs are missing,
  the test is blocked and the gate fails.
- **Green-run caching (incremental `loft check`):** a passing test is cached by the
  `(run-script content + inputs content + target)` fingerprint in
  `.loft/test/<name>__<target>.stamp`; an unchanged test is skipped next run. The key
  includes the target, so a green `native` run does not vouch for `interpret`.
  `loft check --force` (or `--fresh`) reruns everything.

```bash
loft check            # build default-targets + assets, then run [[test]]
loft check --force    # rebuild + re-run every asset and test
loft check foo.loft   # (a .loft arg) compile-check that file instead — the old --check
```

The declared `[[test]]` phase is the *project-facing* surface; loft's own in-repo
`tests/scripts/*.loft` harness (and the `loft test` package-test runner) are
separate. Logic lives in `src/build_phase.rs` (unit-tested).

> **Minimal-scanner note:** the hand-rolled `loft.toml` reader takes single-line
> arrays and a `[build.target.<name>.requires]` subtable (no inline tables), and
> now strips `# …` comments (full-line and inline).

## Build pipeline

### Consumer's view

```bash
# Install a package (downloads or builds native code)
loft install graphics

# Use it — works on all targets
loft my_program.loft           # interpreter: dlopen libgraphics
loft --native my_program.loft  # native: link graphics_native.rlib
loft --native-wasm out.wasm my_program.loft  # wasm: link wasm variant
```

### The three spellings of `loft install`

| command | what it does |
|---|---|
| `loft install` | resolve every dependency this project's `loft.toml` declares |
| `loft install <pkg>[@<v>]` | install one package from the registry, and record it |
| `loft install .` / `loft install <dir>` | copy THAT package into `~/.loft/lib/<name>` for global use |

Bare install is the npm/cargo reading, and the one `loft api` names when it reports a
dependency unresolved. It used to do the third thing (loft#966), which left a copy in
`~/.loft/lib/<name>` shadowing the registry copy of the same name — loft#667, reached from
a command whose name reads like *install my dependencies*. A path dependency needs no
install: it resolves from the path it names, so bare install reports one only when the
path leads to no package.

The name an install is filed under is the manifest's `[package] name`, not the checkout
directory's — a package whose directory differs landed under a name no `use` can reach.

### What `loft install` does


```
1. Locate package (local path, or future: registry)
2. Read loft.toml
3. If [native] section exists:
   a. Check prebuilt/ for current target
   b. If missing or stale: cargo build native/ for current target
   c. Copy rlib to ~/.loft/lib/<package>/<target>/
4. Copy src/*.loft to ~/.loft/lib/<package>/src/
5. Register in ~/.loft/lib/<package>/loft.toml
```

### What `loft my_program.loft` does (enhanced)

```
1. parse_dir("default/")
2. For each `use <pkg>`:
   a. Find <pkg>/loft.toml in lib search path
   b. Parse src/<entry>.loft
   c. If [native] exists:
      - Interpreter: queue rlib for dlopen after byte_code()
      - Native: add --extern <pkg>_native=<rlib> to rustc
      - WASM: add --extern <pkg>_native=<wasm_rlib> to rustc
3. byte_code() — connects #native symbols to loaded functions
4. execute()
```

---

## Target matrix

| Feature | Interpreter | `--native` | `--native-wasm` | `--html` (browser) |
|---|---|---|---|---|
| Pure loft code | ✓ bytecode | ✓ compiled Rust | ✓ compiled WASM | ✓ compiled WASM |
| `#rust` inline (standard library only — C87) | ✓ fill.rs dispatch | ✓ emitted inline | ✓ emitted inline | ✓ emitted inline |
| `#native` external | ✓ dlopen rlib | ✓ linked rlib | ✓ linked wasm rlib | ✓ wasm.bridge crate (see below) |
| File I/O | ✓ OS calls | ✓ OS calls | ✓ VirtFS bridge | ✗ embedded assets only |
| OpenGL | ✓ glutin/gl | ✓ glutin/gl | ✗ WebGL (different API) | ✓ WebGL2 (via loft-gl-wasm.js) |
| Threading | ✓ rayon | ✓ rayon | ✗ sequential | ✗ sequential |
