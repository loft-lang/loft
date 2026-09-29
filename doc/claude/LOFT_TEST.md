<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `loft test` — testing a loft program or library

The test runner loft ships to its users: how a test is written, how it runs on each backend,
what its output means, and what a green run did not check.  Testing the loft interpreter itself
is [TESTING.md](TESTING.md).

## Loft Test Runner (`--tests`)

The `--tests` CLI flag provides a built-in test runner for loft programs.  It
discovers and executes test functions in `.loft` files without requiring Rust
or `cargo test`.

### Writing tests

Any zero-parameter function whose name starts with `test_` is a test function —
the underscore is part of the rule, so `testDouble` and a function called exactly
`test` are not tests and nothing says so:

```loft
fn test_addition() {
    assert(1 + 2 == 3, "basic addition");
    assert(10 + 20 == 30, "larger addition");
}

fn test_string_length() {
    assert("hello".len() == 5, "text length");
}
```

Test functions use `assert(condition, message)` to validate behaviour.  A
failing assertion marks the test as failed; the runner continues with the
remaining tests in the file.

Helper functions, structs, and other definitions can coexist in the same file —
once the file names at least one `test_*`, those are the whole set and everything
beside them is a helper.

**A file that names NO `test_*` is the other case, and it is the one that lets
`--tests` be pointed at a plain program at all**: there, every zero-parameter
function is run and counted, `main` included, and a parameter — even an unused
one — is the only way to say "not an entry point" (loft#1010). That is how the
reference chapters are checked. The rule is one `if` in
`src/test_runner.rs`: a file that declares a `test_*` has said which functions
are tests.

### Running tests

```bash
loft --tests                  # run tests in current directory (recursive)
loft --tests tests/           # run tests in a specific directory
loft --tests file.loft        # run all tests in a single file
loft --tests file.loft::name  # run a single test function
loft --tests 'file.loft::{a,b}'  # run specific test functions
loft --tests --no-warnings    # suppress warning output
```

Inside a package, `loft test [target]` runs `tests/` and takes the file **however you
spell it** — `loft test draw`, `loft test draw.loft` and `loft test tests/draw.loft`
are one target, so a path pasted out of the runner's own output works (loft#913).  A
`::selector` combines with any of them (`loft test tests/draw.loft::test_foo`), and a
selector that names no test function is an ERROR, not a `0 passed` success.

The runner:
1. Recursively discovers `.loft` files under the given directory (default: `.`).
   When given a single `.loft` file, runs only that file.
2. Parses each file and finds all callable functions (zero-parameter, or
   single `vector<text>` parameter when `@ARGS` provides argv).
3. Applies the optional `::name` or `::{a,b}` filter to select specific functions.
4. Runs each test function independently.  A failed `assert` marks the test as
   failed but does not abort the run.
5. Reports per-file and per-directory summaries.
6. Exits with code 0 if all tests pass, 1 if any fail.

### Native mode (`--tests --native`)

```bash
loft --tests --native tests/scripts     # compile and run all scripts natively
loft --tests --native file.loft         # single file
loft --tests --native file.loft::name   # single function
```

When `--native` is combined with `--tests`, each file is compiled to a native
Rust binary via `output_native_reachable` + `rustc`, then executed:

1. Generate Rust source with all selected test functions called from a
   generated `main()`.  Files with `fn main()` use the loft main directly.
2. Compile with `rustc` (links against `libloft.rlib`).
3. Run the binary and check exit status.

**Binary cache:** each compiled test binary is kept in the temp directory as
`loft_test_native_<stem>_<key>_bin`, keyed by `native_cache_key` over the generated source and
the `libloft.rlib` it links; an unchanged file does not recompile.  The source is built in a
per-process directory and the binary published under its key by an atomic rename, so parallel
runs never share a half-written file.

**Stale rlib detection:** run from loft's own source root (a `Cargo.toml` in the working
directory), the runner compares `libloft.rlib` against the newest `src/` and `default/` file
and runs `cargo build --lib` when a source is newer.  Anywhere else it uses the installed rlib
as it is.

**Limitations:**
- `@EXPECT_FAIL` tests are skipped (native can't catch panics for matching).
- `@EXPECT_ERROR` files are skipped (can't compile intentionally broken code).

### Output format

```
  ok    tests/math.loft  (2 tests)
  FAIL  tests/text.loft::test_empty_concat
  FAIL  tests/text.loft  (1 failed, 3 passed)

  tests/: 1 failed, 5 passed

test result: FAILED. 1 failed; 5 passed; 6 total; 2 files  [ran on the interpreter only — native not exercised: loft test --native]
```

**The result line names the backend it came from.**  `loft test` and `loft test
--native` each exercise exactly ONE backend, and a bare `ok` cannot tell a clean other backend
from one never compiled.  The note rides on the DEFAULT path, because that is the run people
read as covering both.

Under `--native` the note also reports `N skipped` — tests counted as passing
that never ran on that backend (`@EXPECT_FAIL` / `@IGNORE`, or a file with no
native-runnable function), so a green count cannot stand in for coverage it does
not have.

**The same line reports @PLN86 admission scope** (#631).  `loft test` applies the
`[sandbox]` policy from the nearest `loft.toml` at or above each test file (the
package root, since a test lives in `tests/` while the code it exercises lives in
`src/`), and an admission violation FAILS the file just as a compile error does —
a rejected script cannot run at all, so a suite that reported it green was
reporting on something the host would refuse to load.  Three states:

| Note | Meaning |
|---|---|
| `admission checked on N files` | a policy designated code, and it was admitted |
| `a [sandbox] policy is present but designated nothing here` | the selectors matched no function — admission covered NOTHING |
| `no [sandbox] policy — admission not exercised` | nothing to check |

The middle state is the one worth reading: passing quietly under a policy that matches
nothing looks identical to real coverage, which is why it gets its own note rather than
falling back to "no policy".

Hidden directories (starting with `.`) and `.loft/` artifact directories are excluded from
the recursive walk.

### Flags

| Flag | Effect |
|------|--------|
| `--tests [dir\|file]` | Discover and run test functions (default dir: `.`) |
| `--tests file::name` | Run a single test function in a file |
| `--tests file::{a,b}` | Run specific test functions in a file |
| `--native` | Compile to native Rust instead of interpreting (with `--tests`) |
| `--no-warnings` | Suppress warning diagnostics in test output |

⚠ **Put the mode flag BEFORE `--tests`.**  `--tests` takes the NEXT token as its optional
`[dir|file]`, and it does not check whether that token is a flag — so `loft --tests --interpret
g.loft` consumes `--interpret` as the target, falls back to the default `.` and runs **every
`.loft` under the current directory**.  From the repo root that is the whole project.
`loft --interpret --tests g.loft` runs the one file.  The two spellings differ only in flag
order, both exit 0 on a clean tree, and the wrong one prints `2 files` where the right one
prints `1 file` — that count is the only thing that distinguishes them, so read it.

### The shared library base (loft#925)

Each test file is its own program with its own parser — a shared one would let one
file's definitions leak into the next.  Loading each file's `use`d libraries from source would
make a suite pay the PRODUCT of its file count and its libraries' size, so files are grouped
by their leading `use` region (a `#cwd` directive included, verbatim), the region is parsed
once per group, and every file after the first starts from a copy of that parse; output is
byte-identical.  [STARTUP_CACHE.md § The shared library base](STARTUP_CACHE.md#the-shared-library-base-loft925)
has the design.

| Env var | Effect |
|---|---|
| `LOFT_NO_TEST_BASE=1` | Parse every file's libraries for itself, as before.  The control half of an A/B on ONE binary, and what `tests/test_base_equivalence.rs` compares against — a run whose output differs from it is a bug in the sharing. |
| `LOFT_TEST_BASE_REPORT=1` | Name on stderr each `use` region that got a shared base, or was refused one.  Reach for it when a suite did not get faster; it is also what keeps the equivalence guard from silently comparing a run to itself. |

A group of ONE file never builds a base — the base is built when a second file asks
for the same region — so `loft test <one-file>` costs exactly what it did.  A base
is also refused outright under a `[sandbox]` policy (admission reads what the parse
recorded about designated functions) and whenever the region's own parse raises an
error (the error belongs to the file the reader is shown, so it is left to that
file's own parse to re-emit).

---

## What a run did NOT check — scope, admission, coverage

`loft test` ends with a line stating what the run **left out**, not only what it did,
because a bare `ok` looks identical whether the other half was checked or never ran once, and
that silence reads as coverage.  Three things are reported for that reason:

- **Backend scope** — `[ran on the interpreter only — native not exercised: loft test
  --native]`. Each invocation exercises exactly one backend.
- **Admission** — whether a `[sandbox]` policy was present and how many files it
  actually covered. A policy that designates nothing says so.
- **Function coverage** — the functions the suite never entered.

### Function coverage

Every function defined in the package under test that no test entered is listed with
its file, line, and name:

```
coverage: 4 of 8 functions were never entered by these tests
  src/regex.loft:100  find
  src/regex.loft:105  split
  src/regex.loft:110  text.regex_find
  src/regex.loft:115  text.regex_split
```

A fully-covered package says so explicitly (`coverage: all 36 functions were entered`),
so "no coverage line" can never be misread as "everything is covered" — which would
reproduce the very defect the report exists to remove. Ten entries print by default;
`LOFT_COVERAGE=list` prints them all.

**It is a list, never a percentage, and never a gate.** A percentage becomes a target,
and a coverage target produces tests written to reach a line rather than tests that
check a behaviour — the metric goes green over code nobody validated. And a gate would
fail exactly the case the package system exists to support: a library is written
*before* its consumers, so it legitimately starts with little coverage. Each line here
is instead an individual, checkable fact — this code did not run — and the only way to
remove one is to actually run the function.

What is deliberately **not** counted, because counting it would make the number lie:

| Excluded | Why |
|---|---|
| `#native` declarations | No loft body to enter — they dispatch to Rust, so a native-backed package would read 100% uncovered however well tested. |
| Dependencies, the stdlib | A package is not answerable for code it did not write; charging it would make its number depend on how much of a dep it happens to touch. |
| The test file's own functions | They are the drivers, and the runner already reports on them. |
| Generated lambdas | Not written by the author. |

Generators count when **iterated**, not when created: a generator's body runs on resume,
so `it = gen();` with no loop over it has run none of it and stays on the list.

Coverage is recorded on the interpreter (`State::fn_call` and the coroutine resume), so
`loft test --native` prints no coverage line — the interpreter leg carries it. Test
adequacy is a property of the tests, not of the backend.

Guarded by `tests/function_coverage.rs`, which asserts the quiet directions as hard as
the loud one.

---

