<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Tests that need something outside cargo

The database servers, valgrind, a headless display and a browser engine: how to run the tests
that need one, and where to look when the environment, not loft, is what broke.  The
ordinary suite: [TESTING.md](TESTING.md).

## Database backends: sqlite gates CI, all four are the local bar

loft binds four SQL backends through `#c` — **sqlite, PostgreSQL, MariaDB and
duckdb** — behind one `SqlDb` interface, and the property worth testing is that a
generic routine gives the *same* answer on all four.  Only one of them can be a
gate.

**The rule:**

- **Every routine is CI-checked against sqlite.**  It needs no server and no
  install, so a skip there is never environmental — it means the library went
  missing or the availability question broke.  `tests/native.rs` asserts sqlite
  ran on Linux for exactly that reason.
- **All four are runnable LOCALLY, and that is the real bar.**  PostgreSQL and
  MariaDB need a live server, duckdb a 70 MB library no distribution ships;
  none of that belongs in CI.  Before landing anything that touches the SQL
  layer, run all four locally.

**CI cannot cover the other three, and the docs must not imply it does.**  A
green CI run means "sqlite agreed", never "the four agree".  The cross-backend
claim is a local measurement, and when it matters it should be re-run and the
result written into the plan or the commit message rather than assumed to have
held since last time.

**How a fixture selects a backend:** `LOFT_SQLDB_MODE=sqlite|postgres|maria|duckdb`
for `uniform.loft` and `round_trip.loft` (@PLN133's gate).  duckdb additionally
needs `LD_LIBRARY_PATH=$HOME/.local/lib` unless the library is on the loader
path.  A backend that is not reachable prints `SKIP`, and the driver in
`tests/native.rs` **recognises a skip as a skip**: it is never counted as a pass, and the set
that actually ran is printed (`@PLN23 backends exercised: […]`).  Read that line — it is the
only thing distinguishing "four agreed" from "sqlite agreed and three were absent".

**The general rule, for every probe that needs something outside the process** (a
database, a browser, a server): *reaching the subject is a PRECONDITION, not a
measurement.* A probe that could not reach it rendered nothing and asserted nothing, so a
FAILURE from it is a claim it never made — it must SKIP. Everything after the connection
keeps its failure codes: once the subject answers, a wrong result is a real result and
stays red.

So an installed tool that never answers (a chrome that does not open its debugging port) is
the same case as a missing one: skip, never fail.

⚠ **Widening a skip is a reduction in coverage, so prove BOTH directions before believing
it** — that the unreachable case now skips, AND that the reachable case still runs and
still asserts. A skip that swallows a genuine failure is worse than the red it replaced
(the sibling hazard is § "a gate that skips looks like a gate that passes").

### Running the other three

Both servers run as ordinary system services on a development box; the fixtures
find them through environment variables with working defaults:

| backend | how it is reached | default |
|---|---|---|
| sqlite | `libsqlite3.so.0`, no server | `:memory:` |
| PostgreSQL | `LOFT_PG_CONN` — set up with `scripts/setup-test-databases.sh --pg` | `dbname=loft_test_pg` |
| MariaDB | `LOFT_MY_CONN` — set up with `scripts/setup-test-databases.sh --maria` | `host=127.0.0.1 user=loft pass=loft db=loft_test_uni` |
| duckdb | `libduckdb.so` on `LD_LIBRARY_PATH` — install with `scripts/fetch-duckdb.sh` | declared `[c] optional-libs`, so absence is not an error |

### PostgreSQL and MariaDB: the setup, and where to look when it breaks

**`scripts/setup-test-databases.sh` creates both.**  It is idempotent, never
drops anything, and **checks before it escalates** — on a box that is already set
up it needs no `sudo` at all and acts as a verifier.  Run `--pg` or `--maria` for
one half.

| | PostgreSQL | MariaDB |
|---|---|---|
| measured against | 16.14 | 10.11.14 |
| service | ordinary system service, port 5432 | ordinary system service, port 3306 |
| database | `loft_test_pg` | `loft_test_uni` |
| identity | **the OS user**, via unix-socket peer auth | `loft@localhost` / `loft@127.0.0.1`, password `loft` |
| override | `LOFT_PG_CONN` | `LOFT_MY_CONN` |

**The two servers authenticate differently, and that is not an accident of this
box.**  The fixture's PostgreSQL default is `dbname=loft_test_pg` — no user, no
host — so it is a peer connection as whoever runs the tests, and the role to
create is *that person*, not a shared `loft` role.  MariaDB is reached over TCP
with an explicit user and password, which is why one has a credential in the
fixture and the other does not.

**The MariaDB user is SCOPED on purpose**: `GRANT ALL ON \`loft\_test%\`.*` and
nothing else, so anything outside `loft_test*` answers `ERROR 1044`.  A suite that
can drop a developer's other schemas is one bad `DROP` away from a very bad day.
The setup script **verifies** this by trying to create a database outside the
pattern and expecting refusal — the GRANT text saying the right thing is not the
same as the server enforcing it.

The password is `loft`, in the clear, in `uniform.loft`.  That is deliberate and
safe **only** because of the scoping above: it is a local test credential for a
user that can reach nothing but `loft_test*`.  Do not reuse the pattern for a
user with wider rights.

**Symptom → where to look:**

| what you see | where the fault is |
|---|---|
| `SKIP postgres …` / `SKIP maria …` | the server is down, or the database/role is missing.  Run the setup script — it will tell you which. |
| `@PLN23 backends exercised:` missing one | same thing from the driver.  **Read this line**; green with three backends absent looks exactly like green with four passing. |
| `ERROR 1044` from MariaDB | the scope working as designed.  The test wanted a schema outside `loft_test*` — fix the test, do not widen the grant. |
| `peer authentication failed` on PostgreSQL | the OS user has no role.  `sudo -u postgres createuser --createdb $(id -un)`. |
| PostgreSQL passes but floats differ | check `extra_float_digits` — it must be ≥ 1, and it defaulted to 0 before PG12 (@PLN133 P3). |
| both servers fine, results differ between them | a real finding: the `SqlDb` contract is what makes the four interchangeable.  Compare the whole line. |

**CI has neither server** and is not expected to.  Nothing here is reproduced by
a green CI run.

### duckdb: where it comes from, and where to look when it breaks

**`scripts/fetch-duckdb.sh` installs it into `~/.local/lib`.**  It downloads a
PINNED upstream release, verifies a recorded `sha256` of the extracted
`libduckdb.so`, and refuses to install anything else.  Nothing else fetches it —
not CI, not `loft install`, not the test suite.

It is **not** vendored, for two reasons worth keeping straight: duckdb is MIT
licensed so redistribution would be legal, but the library is ~70 MB and git
history is permanent, and upstream already publishes exactly this artifact.  It
does not live in `~/.loft/lib` either — that holds loft *packages*, not native
shared libraries.

**The dependency chain, in the order a failure travels it:**

```
scripts/fetch-duckdb.sh   pins VERSION + EXPECT_SHA, writes ~/.local/lib/libduckdb.so
        ↓
LD_LIBRARY_PATH           the only thing that makes it findable — no rpath, no ldconfig
        ↓
[c] optional-libs         tests/fixtures/sqldb/duckdb/loft.toml declares "libduckdb.so"
        ↓
c_call::resolve           dlopens it on the first miss (@PLN24 arc G)
        ↓
src/shim.c                loft compiles this with `cc` at parse time — it names NO
                          duckdb symbol, so it builds even where the library is absent
        ↓
LOFT_SQLDB_MODE=duckdb    selects the backend in uniform.loft
```

**Symptom → where to look:**

| what you see | where the fault is |
|---|---|
| `SKIP duckdb …`, everything else green | the library was not found — `LD_LIBRARY_PATH` unset, or `~/.local/lib/libduckdb.so` gone.  Re-run `scripts/fetch-duckdb.sh`. |
| `@PLN23 backends exercised: [...]` without `duckdb` | the same thing, seen from the driver.  **Read this line** — a green run with duckdb absent looks identical to one with duckdb passing. |
| `sha256 mismatch … NOT installing` | upstream re-cut the release, or the pin is stale.  Decide which, then edit `EXPECT_SHA` **on purpose** — never to make the message go away. |
| `the archive did not contain libduckdb.so` | upstream changed the zip layout.  The script prints the listing; fix the extraction, do not work around it by hand. |
| a `cc` failure mentioning `shim.c` | not a duckdb problem at all — the shim is deliberately free of duckdb symbols so it compiles without the library.  Look at the C toolchain. |
| duckdb answers but DIFFERS from the other three | a real finding: the `SqlDb` contract is what makes the four interchangeable.  Compare the whole line, not one field. |

**One diagnosed caveat, so it is not rediscovered** (@PLN133 P3): a float written
through this fixture round-trips exactly on PostgreSQL and MariaDB and fails 19
times in 500 on duckdb.  Fifteen of those are a **duckdb parser bug** —
a decimal literal whose digit run is **275–294 characters** is read as a value
exactly **10^256 too small**, silently, while the same value in *exponent
notation* is correct.  The other four are ordinary 1–2 ULP parser rounding.

loft walks into it because **`"{v}"` renders a float as a full decimal expansion
with no exponent**, so any float above ~1e274 becomes a 275+ character literal.
**Write floats to SQL quoted (`CAST('{v}' AS DOUBLE)`), in exponent notation, or
bound — never as a bare `"{v}"`.**  Quoting is measured at 0/500 on duckdb and
needs no change to loft's rendering; it does NOT help sqlite, whose own
text→REAL converter loses the same 1 in 2000 either way.

**A copy in a scratchpad or a build directory is not durable.**  When it
evaporates the duckdb cell silently drops back to `SKIP` and the local
four-backend bar quietly becomes a three-backend one — the same class of
invisible coverage loss as a self-skipping test.

**Do not assume a float survives a text round trip** (@PLN133 P3): the four engines do not
render a `double` to text the same way — `CAST(v AS TEXT)` on sqlite is inexact for 94% of
random doubles.  Measure it per backend, with a sweep rather than a handful of hand-picked
values.

---

## A private `HOME` hides the Rust toolchain

A test that runs `loft` with `HOME` pointed at a scratch directory (the usual way to give it a
private registry cache) also moves rustup, which finds its toolchains through `HOME`.  With no
`rustc` reachable, every `use`d library interprets and prints *"has no native build and no
Rust toolchain"* — so the auto-native path is silently untested, and a defect that lives
only there shows up as an "intermittent" failure in whichever runner happens to expose
`rustc` (nextest does).  Point `LOFT_HOME` at the scratch directory and leave `HOME` alone,
or set `RUSTUP_HOME` / `CARGO_HOME` to the real ones, when the native half is part of what
the test claims.  `n3_use_native.rs::a_dependency_version_is_part_of_its_users_native_artifact`
is the worked example; the defect it guards read as a flake in `manifest_less_resolution.rs`.

## macOS: what makes the suite slow, and the setup that removes it

**A freshly linked binary pays a security assessment on its first launch** — about 260 ms on top
of the ~30 ms a second launch takes, and the system runs these assessments nearly one at a time
(14 fresh binaries launched together take about 82 % of the fully serial time).  The native
corpus compiles and launches thousands of fresh executables, so on an unprepared Mac that queue
alone is tens of minutes and parallel tests stall behind it until they time out.  An existing
binary starts in ~10 ms; only NEW ones pay.  To exempt processes started from your terminal:

1. `sudo DevToolsSecurity -enable` (developer mode — necessary, not sufficient on its own);
2. System Settings → Privacy & Security → Developer Tools → add the terminal app you run tests
   from, and switch it on;
3. quit and reopen that terminal: the exemption applies to processes started after it relaunches.

Check it with two launches of a fresh binary (`rustc -O h.rs -o h && time ./h && time ./h`): the
first should cost about what the second does.  A managed endpoint agent can still scan on its
own; the same check shows that.  Whatever does not fit the 20-minute cap afterwards is split or
moved, never given a longer limit (`scripts/hard_cap.sh`, CI_BUDGET.md).

**Package cdylibs are linked without a post-link `strip`.**  Cargo's release default runs the
system `strip` over a linked dylib; on one holding `ring`'s objects (every TLS package) that left
the string table misaligned and the linker refused to link against it (`ld: mis-aligned LINKEDIT
string pool`).  The linker drops debug symbols itself instead (`cache::NATIVE_LINK_RECIPE`).

## Occasional valgrind pass (Linux)

The loft codebase has a large `unsafe` surface in `src/store.rs`,
`src/database/`, and `src/parallel.rs` (raw `addr`/`addr_mut`, LLRB
free-tree rotations, claim/free splits, worker store adoption).  Linux
runs the test suite cleanly because the system allocator over-allocates
small chunks; latent OOB writes land in slack and don't corrupt anything
visibly.  The same code on Windows hit `STATUS_HEAP_CORRUPTION` once the
heap manager validated chunk metadata at deallocation.

Valgrind's memcheck tool catches this class of bug instantly: every
load/store is instrumented and OOB accesses fail loudly, regardless of
allocator behaviour.

### Recipe

```bash
scripts/valgrind-sweep.sh              # every script + document, interpreter AND native
scripts/valgrind-sweep.sh tests/docs   # one tree, or a list of files
```

One command, one verdict, per-file logs in `target/vg/`.  It runs `loft --interpret` on every
file (`--tests` for `tests/scripts`, whose files have no `main`) and, for `tests/docs`, builds
each document with `loft --native` and hands the cached binary in `<dir>/.loft/cache/` to
memcheck directly — the compiled program is where the native runtime's `unsafe` runs, and
`--trace-children` cannot reach it without also tracing rustc.  `VG_JOBS` bounds the parallel
memchecks (each takes ~200 MB); the default is every core on a machine of four or fewer (the
CI runner) and five sixths of a larger one.

It has to fit a 4-core runner inside the nightly job's limit, and it does so by doing less
repeated work, never by a longer limit:

- **The standard library is parsed once.**  Parsing `default/` under memcheck was ~5.5 s of a
  trivial file's 6.9 s.  The first run parses it cold under memcheck and writes the stdlib
  bundle into the sweep's own cache directory; every other run starts warm from that bundle
  (`LOFT_STDLIB_CACHE=1`), so the parse, the writer and the reader are all still memchecked.
- **A plain pre-pass plans the work.**  Each file runs once without memcheck (a median file
  takes 0.04 s).  Its time predicts the memcheck cost, and the work starts longest first.  A
  file that takes 0.5 s or more with two or more test functions is memchecked one function at
  a time (`file::name`, the names read off the runner's own `(N fns: …)` line), so a
  store-ceiling guard's 70 000-iteration cells spread over the jobs instead of running in
  series into the per-run limit.
- **A run the per-run limit ends is red**, not quietly counted: it exits 124 and was checked
  only up to where it stopped.  A plan with no runs in it refuses to sweep rather than
  reporting GREEN over nothing.

Two decisions are built in, and both are measurements rather than taste:

- **Only an invalid access or a DEFINITELY lost block is red.**  Rust's hashbrown tables and
  boxed strings keep interior pointers, so every process-lifetime table — the parser's
  `Data`, the native emitter registry — reads as "possibly lost" at exit: 179 such records on
  a run with no defect in it.  `--errors-for-leak-kinds=definite` is that decision spelled
  where valgrind reads it; a possibly-lost record is still in the log for anyone who wants it.
  The one suppression, `scripts/valgrind.supp`, is the deliberate interning of a declared
  text field default — bounded, one block per field — and nothing else: the LSan file also
  hides the four text-construction frames on the premise that they leak only on a fault
  path, and this sweep measured that premise false (a text returned from a call on two arms,
  or read straight out of a vector element, loses one buffer PER CALL with no fault at all).
- **A leaked or over-freed STORE is not a valgrind error.**  The store arena is one valid
  allocation (DEBUG_STORES.md § Debugging store-ownership bugs), so that half of the release's
  memory gate is `M-leaks` under `LOFT_STRICT_STORES=1`, and this sweep does not pretend to
  cover it.

### When to run

- **Before a release** — once per release cycle.  Catches any latent
  UB introduced since the last pass.
- **After significant `unsafe` changes** in `Store`, `Stores`, the
  parallel runtime, or the LLRB free-tree (`fl_*` in `src/store.rs`).
- **When a Windows-only failure appears** with heap-corruption-style
  symptoms (`0xc0000374`, `LdrpAllocate*`, `RtlReportFatalFailure`).

Not a CI default: too slow for every PR.  Tracked as a release-blocker
gate in [RELEASE.md](RELEASE.md) — run on the tag candidate, not on
every push.

---

## Headless OpenGL testing (Xvfb)

Loft GL examples create a real winit/GLX window, so they normally need an X
display. For CI / sandbox environments without `$DISPLAY`, we run them under
**Xvfb** (the X Virtual Framebuffer). Xvfb is a software X server that
keeps everything in memory — no GPU, no monitor, no compositor required.

### Required tools

```bash
sudo apt-get install -y xvfb x11-utils x11-apps xdotool imagemagick
```

- `xvfb-run` — wrapper that starts Xvfb on a free display, runs the inner
  command with `$DISPLAY` set, and tears Xvfb down on exit.
- `xdotool` — searches for a window by name and returns its X11 ID.
- `import` (from ImageMagick) — captures a window or the root drawable
  to a PNG file.

### Running a single GL example headlessly

```bash
xvfb-run -a -s "-screen 0 800x600x24" \
    target/release/loft --interpret --lib <dir holding graphics> <program>.loft
```

`-a` picks an unused display number. `-s` passes args to Xvfb itself.
Mesa's software rasterizer (`swrast`/`llvmpipe`) handles the actual GL
draw calls — the binary doesn't know it's running headless.  `make test-gl-headless` runs
the GL examples this way; `make test-gl-golden` compares a snapshot against
`tests/golden/00-smoke.png`.

### Capturing a screenshot

The capture has to happen *while* loft is running: `xvfb-run` tears Xvfb down when its
command returns.  So the command `xvfb-run` runs is a wrapper that

1. starts loft in the background;
2. polls for loft's window with `xdotool search --name "."`;
3. waits for the render loop to produce the frame wanted;
4. captures it with `import -window <id> out.png`;
5. kills loft and exits.

`tests/scripts/snap_smoke.sh` is that wrapper for the golden test; copy its shape.  A program
whose render loop is finite exits before the capture, which is why `snap_smoke.sh` wraps the
smoke program's body in a long-running loop.

### Gotchas

- **The loft window is a child of the X root, not the root itself.**
  `import -window root` captures an empty Xvfb root if no window manager
  is parenting/compositing children. Always grab loft's window by ID.
- **`LIBGL_ALWAYS_SOFTWARE=1` makes things WORSE under Xvfb.** Without it,
  Mesa picks `swrast_dri.so` automatically; with it, the GL context fails
  to initialise and `gl_create_window` returns false.
- **Captured PNGs have R and B swapped.**  Xvfb + Mesa-swrast + ImageMagick `import` reads
  the framebuffer with the channels exchanged; on-screen rendering is correct.
  `snap_smoke.sh` applies `convert -separate -swap 0,2 -combine` after `import`.
- **Polling for `xdotool search --name "."`** matches *any* named window.
  If the test environment has other X clients running, narrow it down by
  passing the window title used in `gl_create_window`.

### Using Xvfb to run the cargo test suite

```bash
# Run all GL-touching tests under Xvfb in one shot
xvfb-run -a cargo test --release
```

The test process inherits `$DISPLAY` from `xvfb-run`. Tests that don't
touch GL ignore it; tests that *do* touch GL get a working framebuffer.

### Headless valgrind on a GL example

For leak/UB checking on a GL example, combine Xvfb with valgrind:

```bash
xvfb-run -a -s "-screen 0 800x600x24" \
    valgrind --tool=memcheck --leak-check=full \
             --show-leak-kinds=all --log-file=/tmp/v.log \
        target/debug/loft --interpret --lib <dir holding graphics> <program>.loft

grep -E "definitely lost|indirectly lost|possibly lost|ERROR SUMMARY" /tmp/v.log
```

Debug-build loft + valgrind + Mesa swrast is **very slow** — expect
10-100x slowdown. Use a short loop count for ad-hoc checks.

---

## Debugging `loft --html` WASM traps

WASM compiled for `wasm32-unknown-unknown` with `-O` uses `panic = "abort"`: a panic becomes
a bare `unreachable` instruction with no message, no location and no panic handler, so the
engine reports only `RuntimeError: unreachable executed` and a WASM function index.  The
panic site is recovered by rebuilding the same generated Rust without `-O`.

### The technique

1. **Write a minimal reproducer.** If `fn main() { }` or
   `fn main() { println("hi"); }` traps, the bug is in WASM init
   (Stores::new / stdlib load / host-import wiring), **not** in any
   user-code path.  Don't bisect user code yet.

2. **Keep the generated Rust.**  `LOFT_KEEP_NATIVE_RS=1` makes `loft --html` keep the
   `prog.rs` it compiled and print where (`browser-wasm source preserved at …`, a per-process
   directory under the temp dir):

   ```bash
   LOFT_KEEP_NATIVE_RS=1 ./target/release/loft --html /tmp/app.html app.loft
   cp <printed path> /tmp/loft_html_saved.rs
   ```

3. **Compile the saved Rust without `-O`** so Rust's panic machinery
   still emits symbols:

   ```bash
   rustc --edition=2024 --target wasm32-unknown-unknown \
     --crate-type cdylib \
     --extern loft=target/loft/html/wasm32-unknown-unknown/release/libloft.rlib \
     -L dependency=target/loft/html/wasm32-unknown-unknown/release/deps \
     -L dependency=target/release/deps \
     /tmp/loft_html_saved.rs -o /tmp/debug.wasm
   ```

4. **Run in Node with `tools/wasm_repro.mjs`** and print the stack:

   ```bash
   node tools/wasm_repro.mjs /tmp/debug.wasm
   ```

   The debug build's stack shows `_ZN...` mangled Rust symbols.  The
   first non-panic-machinery symbol is the function that panicked —
   often `std::...::now`, `Vec::index`, `Option::unwrap`,
   `core::panicking::panic_fmt`.

### Repro harness — `tools/wasm_repro.mjs`

Loads a WASM file with loose stub imports (loft_io and loft_gl via a
Proxy that answers any method with a no-op) and runs `loft_start`.

```bash
node tools/wasm_repro.mjs <path/to/wasm> [--trace]
```

Exit code 0 = clean run; 1 = trap.  `--trace` records every host
import call into a buffer printed on trap — revealing which loft
function last reached the host boundary before the fault.

Used by the **`tests/html_wasm.rs::p137_html_hello_world_does_not_trap`**
regression test, which builds a hello-world `.loft` program through
`--html`, extracts the WASM, and runs the harness.  Skipped in
environments without node or the wasm32-unknown-unknown rustup target.

