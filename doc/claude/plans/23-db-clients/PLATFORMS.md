<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 23 — building the db client on every platform

The client is four C libraries reached through `#c` (@PLN24), so "does it work"
is a different question on every host: the library has a different **name**, the
shim needs a different **link**, and on one target the capability does not exist
at all.

## The bar

@PLN24 sets it, and it is deliberately not *works everywhere*:

> Parity is **"a defined answer in every column"**, not *"works everywhere
> automatically"*.

`#native` does not get the browser for free either. So this document is finished
when the table below has no blank cell — each one a green, or a refusal that
names itself.

**It is finished: the table has no blank cell, and P1–P6 are answered** (the
ladder at the end says how each one was). Two of them are answered by a *refusal*
and a *skip* rather than a green, which is what the bar above asks for.

## Where it stands — measured

| platform | the shim links | the C library resolves | the four backends | state |
|---|---|---|---|---|
| Linux x86-64 | yes | yes | proven, both loft backends | **green** |
| macOS (Apple silicon) | yes, `@rpath` install name verified | yes — sqlite; the other three correctly skip | **sqlite proven, both loft backends** | **green** |
| Windows | yes — P1 verified, `LNK1181` gone; and it now LOADS (X7) | **yes — `winsqlite3.dll`, named by the package (X8)** | **sqlite proven, both loft backends** | **green** |
| wasm / browser | n/a | n/a | impossible by capability | **green — refused by name** (P6) |

The macOS row is now a measurement, not a hope —
[MACOS_RESULTS.md](MACOS_RESULTS.md) has the numbers. P2 is confirmed there:
`@PLN23 backends exercised: ["sqlite"]`, matching the SAME hard-coded assertion
strings Linux matches, with `--interpret` and `--native` agreeing. postgres,
maria and duckdb are absent from that list as *correct reported skips*, for the
reasons in the install table below — not as failures.

**The Windows row is now a measurement too** (2026-08-04): `--test native` on
`windows-latest` is **22 passed, 0 failed**, reporting `@PLN23 backends
exercised: ["sqlite"]` — the same list macOS reports, against the same
hard-coded assertion strings Linux matches. postgres, maria and duckdb are
absent as *correct reported skips*. Two defects stood between X5 and that line,
both found by reading artefacts rather than guessing (X7, X8 below).

Windows was red **on `main` too**, not only on the branch carrying the partial
fix. It was a pre-existing platform gap closed incrementally, not a regression.

**What that row used to say, and why it is worth remembering.** Before P2 the
same test *also* passed on macOS — 11.9 s on `main` — while exercising no
database at all: only that the duckdb shim compiles and links and that the
absent-library SKIP path works. Two greens, one meaning nothing and one meaning
what it says, and no way to tell them apart from the check name. That is the
whole argument for the guard P2 added.

## Two ways a green here means nothing

Both have already happened, and neither is visible from a check name.

**1. The check name is a placeholder.** `Test (windows-latest)` and
`Test (macos-latest)` in the PR list run on an `ubuntu-latest` runner and only
echo a notice. The real matrix job renders under the *same display name* when it
runs, so a placeholder and a real run are indistinguishable in `gh pr checks` —
read the runner OS or the log, never the name. Per-PR macOS coverage is real but
narrow (Miri-macOS, ASan-macOS) and **excludes `tests/native.rs`**, which is
where every fixture in this plan lives. The full suite does run on push-to-main
and daily, so the answer exists — it just arrives after the merge. For a
pre-merge answer, dispatch one:

```bash
gh workflow run ci.yml --ref <branch> -f os=macos-latest
gh workflow run ci.yml --ref <branch> -f os=windows-latest
```

**2. The availability probe was Linux-shaped.** *(Fixed by P2 — kept here because
it is the shape of the mistake, not a one-off.)*
`one_sql_interface_drives_four_different_c_libraries` gated each backend on

```rust
["/lib/x86_64-linux-gnu/", "/usr/lib/", "/usr/lib64/"]   // Linux dirs
    .iter().any(|d| Path::new(&format!("{d}{n}")).exists())   // n = "libsqlite3.so.0"
```

Both the directories and the spelling are Linux's, and it is the **spelling** that
does the damage: `/usr/lib` exists on macOS, but what lives there is
`libsqlite3.dylib`, so a search for `libsqlite3.so.0` finds nothing. On Windows
not even the directories exist. Either way the answer is **always false**, so the
sqlite, postgres and maria cells are skipped in silence — including sqlite, the
cell whose entire job is to be the one that always runs.
What remains is the unconditional duckdb cell, and that is exactly the one the
Windows run died in (`--native/duckdb`).

So on a non-Linux host the test today proves *the duckdb shim compiles and links*
and nothing else about this plan. That is not a prediction — it is what the green
macOS cell on `main` is made of. Fixing the link without fixing the probe buys
Windows the same empty green.

## Windows

### What landed

`7066771c` — `-l` names a library, so the stem strips `.dll` beside `.so` /
`.dylib`, and `-Wl,-rpath` is no longer passed on a host that has no RPATH. That
turned `sqlite_shim_<hash>.dll.lib` into a request for the right name.

### What the run then reported

```
LINK : warning LNK4044: unrecognized option '/Wl,--allow-multiple-definition'; ignored
LINK : fatal error LNK1181: cannot open input file 'sqlite_shim_5aed31d6bafbf9f8.lib'
```

The name is now right and **the file does not exist** — the follow-up the landed
commit predicted in its own message. Two changes remain:

**W1 — the shim must emit an import library.** `c_shim.rs` builds every shim with
`-O2 -fPIC -shared`, and a MinGW `cc` emits no import library unless asked. MSVC
`link.exe` cannot link a bare `.dll`; it needs the `.lib`. The hook already
exists and already has a per-OS arm — `platform::shim_name_args`, which is where
macOS's `-Wl,-install_name` lives — so this is a Windows arm on a function that
was built for exactly this, adding `-Wl,--out-implib,<stem>.lib`.

Worth naming before it is attempted: a MinGW-produced COFF import library is
normally consumable by MSVC for plain cdecl x64 C symbols, but that is the
assumption this step is testing. If it does not hold, the fallbacks in order of
preference are to build the shim with MSVC directly (`cl /LD`), or to link the
shim's objects statically into the binary and drop the DLL entirely — the shims
are three-line trampolines that reference no database symbol, so static linking
costs little and removes the import-library question outright.

**W2 — stop sending MSVC a GNU linker flag.** `-Wl,--allow-multiple-definition`
is emitted at `main.rs` and `test_runner.rs`, each guarded `#[cfg(not(target_os =
"macos"))]` because ld64 rejects it. MSVC rejects it too, and says so three times
directly above the real error. The guard needs Windows for the same reason macOS
has it. This fixes no link by itself; it stops the log lying about where the
failure is.

### W1 + W2 landed, and the link is FIXED — measured

The dispatched Windows run has **zero `LNK1181` and zero `LNK4044`** in its whole
log. The shim now compiles, links and *runs*: the `#c` tests get as far as
executing and comparing values. That is the naming-and-linking layer closed.

What it uncovered underneath is a different layer, and it is worth having in one
place because it is the actual Windows worklist now. Six tests fail, in **three
classes, none of them about linking**:

**X1 — FIXED. `long` is 32 bits on Windows, and loft is right about that.** `CTarget::host()`
already models LLP64 (`long_bits: if cfg!(windows) { 32 } else { 64 }`), so loft
faithfully truncates. The bug is in the FIXTURES, which use C `long` to carry a
64-bit loft `integer`:

```
c_binding_matrix…   want `i64 1234567890123`   got `i64 1912276171`
                    1234567890123 & 0xFFFFFFFF  = 1912276171   — exactly the low half
loft_builds_…shim   want `scale 4621819117588971520`  got `scale 0`
                    10.0's bit pattern is 0x4024000000000000; its low 32 bits ARE 0
```

The `scale` case is worse than truncation: `lcs_shim_scale(long bits, …)` does
`memcpy(&v, &bits, 8)`, which on Windows **reads four bytes past the argument**.

The cure is the rule `CTarget`'s own doc comment already states — **write the
signature the header shows you** — and the declarations were not doing that.
`lc_types.c` has always said `int64_t`; only the `#c` declarations said `long`,
and on LP64 the two coincide, so Linux and macOS never noticed. Every fixed site
now names a width that means one thing on every platform:

| where | was | now | why |
|---|---|---|---|
| `lcabi` (11 decls) | `long` | `int64_t` | the C already said `int64_t`; only the declaration disagreed |
| `lcshim` (3 decls + `lc_shim.c`) | `long` both sides | `int64_t` | the only site where the C itself was wrong — and where `memcpy(…, 8)` read past the argument |
| maria `lm_*` (12 decls + `row.c`, `stmt.c`) | `long` | `int64_t` | loft-authored shim carrying a loft `integer` |
| duckdb (7 decls) | `unsigned long` | `uint64_t` | duckdb.h says `idx_t`, which is `uint64_t` — never `unsigned long` |
| `mysql_num_rows` | `long` | `uint64_t` | MariaDB declares `my_ulonglong`, 64-bit everywhere |

It is deliberately **not** a blanket substitution. Four declarations keep `long`,
because that is what libmariadb's own header says — `mysql_real_connect`'s flags,
`mysql_stmt_prepare`'s length, `mysql_stmt_param_count`, `mysql_stmt_fetch_column`'s
offset. Their `unsigned long` is 32-bit on Windows *in the library too*, so
matching it is the correct binding and widening it would be the new bug. Likewise
the `MYSQL_BIND` struct in `stmt.c` keeps every `unsigned long` field: that layout
is libmariadb's ABI, verified field-by-field against the real header, and it is
not ours to change.

Linux is unaffected — `long`, `long long` and `int64_t` are all 64-bit there, so
the seven `#c` tests pass unchanged. **Windows is unverified until the next
dispatch.**

**X2 — FIXED. A Windows path becomes an escape sequence.**
`an_available_library_must_export_what_was_declared` builds a fixture library in a
temp directory and pastes its absolute path into **two** string literals: a TOML
basic string (`optional-libs = "…"`) and a loft one
(`pub const SKEW_SONAME = "…"`). Backslashes are escapes in both, so
`C:\Users\runneradmin\AppData\Local\Temp\loft_skew_2244\…` failed to lex at the
`\U`: `error: Unknown escape sequence`, at exactly line 4 column 29 of the
generated file.

Fixed by writing the path with forward slashes. Escaping for each syntax
separately would also work, but that is two escapers to keep right in two
different parsers; a separator neither treats as special has no such failure mode,
and Windows accepts `/` everywhere loft passes this on — `lib_variants` already
splits a directory off on either separator. Audited: this is the only place in
the suite that interpolates a path into generated `.loft` or `.toml` source.

A test-harness bug, not a `#c` one — but it would have failed the Windows leg on
its own regardless of X1.

**X3 — was not a thing.** I guessed `interpreted_and_native_c_bindings_agree` was
"probably X1 again" without diagnosing it. It was not; it was X5 below. Two
lessons, both cheap: an undiagnosed failure attributed to a known cause is a
guess wearing a label, and the reason it went unread was an interleaved
`--nocapture` log — the fix for which is fewer concurrent failures, not more
squinting.

**X4 — FIXED. A second import library, with a different builder.**
`LNK1181: cannot open input file 'lc_types.lib'`. W1 gave `c_shim.rs` its
`--out-implib`, but `tests/fixtures/c_abi/Makefile` builds `liblc_types` *itself*
and had only a Darwin arm and a Linux `else`; on MSYS/MinGW it fell through to
`else`, produced a `.so`, and left MSVC nothing to link. The Windows arm now
emits `liblc_types.dll` plus `-Wl,--out-implib,lc_types.lib`. The `.lib` stem is
fixed by loft passing `-l lc_types` (taken from the declared `liblc_types.so`),
and the DLL keeps its `lib` prefix because `lib_variants` tries
`liblc_types.dll` beside `lc_types.dll`.

**X5 — FIXED. Windows has `write(2)`'s behaviour but not its name.**
`` `#c` symbol 'write' not found ``: the CRT exports it as `_write`. The parity
fixtures now spell that declaration per host.

X5 is worth keeping as a check on X1's rule. **`long` is genuinely correct for
that return on both platforms** — POSIX `write` gives `ssize_t` (64-bit on LP64),
`_write` gives `int` (32-bit), which is exactly what C `long` means on each. A
blanket `long` → `int64_t` sweep would have broken this binding. That is the
whole reason X1 was scoped to loft-*authored* shims instead of applied
everywhere, and it is why the audit had to be per-declaration.

**X7 — FIXED. A linker records the name it was GIVEN, not the name you rename
to.** `STATUS_DLL_NOT_FOUND` (`0xC0000135`) before `main`, both streams empty.
loft builds the shim to `<stem>.<pid>.tmp` and renames it over the final name so
the publish is atomic; `--out-implib` writes the import library *during that
build* and records the DLL name from `-o`, so the `.lib` said `.tmp`. The rename
moved the file and left the recorded name behind, and every binary linking that
`.lib` asked the loader for a temporary nobody published. Read straight off the
PE import table: it wanted `lcshim_shim_<key>.8496.tmp` while
`lcshim_shim_<key>.dll` sat in the same directory. Fix: stage in a temp
**DIRECTORY** with the artifacts already carrying their **FINAL** names — the
recorded name follows the BASENAME and the rename still lands in the same
directory, so it stays atomic. Same class as the macOS install-name bug, which
is why `platform.rs` already said *every artifact loft publishes by rename needs
this*; the Windows arm is not an install name, so `install_name_args` returning
empty off macOS left it open.

Two hypotheses died first, both cheaply, and both are worth recording because
each *looked* right: the shim imports only `KERNEL32` and the UCRT, so no MinGW
runtime was ever involved (`-static-libgcc` had already done its job); and
`-Wl,--soname,<final>` changes nothing because **PE ignores it** — verified side
by side (`.tmp`+rename → imports `.tmp`, exit 127 · `--soname` → identical ·
built under the final name → runs). **A missing import names no name**, so
guessing is unbounded; the import table is the only instrument that converges.

**X8 — FIXED. A translated library name is not an identity, and the gate asked
the weaker question.** With the shim loading, sqlite faulted with `0xC0000005`.
`libsqlite3.so.0` translates to `sqlite3.dll`, and the first one on a runner's
PATH belongs to the AWS CLI: it loads, it exports `sqlite3_open` at a real
address so every cheap check passes, and it faults when called — reproduced in
~20 lines of plain C, so nothing in loft's `#c` path was implicated. Windows'
own SQLite is `winsqlite3.dll`, which no stem rule can derive from a Linux
soname, so the package names it (`optional-libs` is already a list, which is the
right shape for "SQLite, however this platform spells it").

The second half was ours: the harness asked `host_library_loadable` — a bare
`Library::new(name).is_ok()` — while `c_library_available` is true only when the
library opens **and every declared symbol resolves**. Its own doc says why: *a
file-granular answer would say yes where the call still faults*. So an unusable
library got CALLED where the fixture's contract promised a SKIP. `duckdb`
already had this right here; sqlite had no guard at all (serverless does not
mean the LIBRARY is present), and postgres/maria were only saved by the harness
gate. All three now answer for themselves and the harness asks nothing — it runs
every mode and reads the verdict off stdout. One home for the fact.

None of these is a regression from W1/W2 — they were unreachable while nothing
linked, which is the ordinary shape of fixing a bottom layer.

### How big the Windows gap actually is

A full `ci.yml` matrix run on Windows: **3690 tests, 3685 passed, 5 failed — and
all five are the `#c` tests in `tests/native.rs`.** Nothing else on the platform
is broken. That bounds the problem and it also says the cheap loop is sufficient:
`windows-probe.yml` running one test binary found exactly the same five.

**Iterate on the probe, not the matrix.**

```bash
gh workflow run windows-probe.yml --ref <branch> -f tests="--test native"
```

Six minutes against the matrix's forty-five, because it builds one test binary
instead of everything and skips ~3670 tests that were never going to fail. The
workflow was built for this and says so in its own header — *"a 24-hour loop for
a question you can usually answer in minutes"*. Reach for the full matrix once at
the end, to confirm nothing else moved. Note `cargo test` takes ONE test-name
filter; passing several is `error: unexpected argument`, so filter by test BINARY
and accept the extra tests.

### After that, the databases themselves

A Windows runner ships none of these libraries. All four are declared
`[c] optional-libs`, so an absent one is a reported SKIP rather than a failure —
which means **W1 + W2 turn the Windows cell green while still testing no
database**. That is the second failure mode above, so the Windows work is not
done at W2; it is done when the probe asks the host's own question (P2 below) and
sqlite answers it.

## macOS

Nothing is broken; nothing is proven either. The full suite **does** run on
push-to-main and daily, and passes — so the shim compiles, links and loads under
`-Wl,-install_name,@rpath/…`, on both loft backends. What it never does is open a
database, because the probe does not ask macOS a question macOS can answer. P2 is
the entire macOS story, and P3 is then a re-run rather than a repair.

The pieces that must line up when it does ask:

- **The library name.** `platform::lib_variants` translates the declared Linux
  spelling at use time. macOS system sqlite is `/usr/lib/libsqlite3.dylib`, which
  the translation already produces.
- **The shim's install name.** `shim_name_args` passes
  `-Wl,-install_name,@rpath/<final name>`, accounting for the
  build-to-`.tmp`-then-rename publish that once shipped a `.tmp` install name.
  **Verified**: `otool -D` on all four shims gives
  `@rpath/lib<x>_shim_<key>.dylib` — the final name, no `.tmp`, no build
  directory — and the sqlite shim depends only on `libSystem.B.dylib`.

**One latent defect found while checking that — now FIXED.** The *other* artifact
in each `native-auto/` — the package cdylib `libloft_auto_<mode>_<hash>.dylib`,
built by `native_lib.rs` rather than `c_shim.rs` — carried an install name of

```
/Users/…/native-auto/loft_auto_<mode>_<hash>.building
```

the temporary stem with the absolute build directory baked in: precisely the shape
the install-name flag exists to prevent, on the artifact that never got it. It
broke nothing, because those cdylibs are loaded **by path** and `dlopen`-by-path
ignores the recorded name — which is exactly why it survived. It would have become
real the first time one was resolved through `@rpath` or simply moved.

`native_lib.rs` now passes the same flag, through the same helper, which is
renamed `platform::install_name_args` because it is no longer shim-specific: it
belongs to **every artifact loft publishes by rename**, and the bug was that one
of the two builders knew that and the other did not. Linux is unaffected by
construction — the helper returns an empty list off macOS, so the rustc argument
list there is byte-identical. **Needs an `otool -D` on a Mac to confirm** (see
[MACOS_HANDOFF.md](MACOS_HANDOFF.md) task 6); it is not verifiable from Linux, and
a blind macOS change is how the original `.tmp` install name shipped.

| declared | macOS is asked for | Windows is asked for |
|---|---|---|
| `libsqlite3.so.0` | `libsqlite3.0.dylib`, `libsqlite3.dylib` | `sqlite3.dll`, `libsqlite3.dll` |
| `libpq.so.5` | `libpq.5.dylib`, `libpq.dylib` | `pq.dll`, `libpq.dll` |
| `libmariadb.so.3` | `libmariadb.3.dylib`, `libmariadb.dylib` | `mariadb.dll`, `libmariadb.dll` |
| `libduckdb.so` | `libduckdb.dylib` | `duckdb.dll`, `libduckdb.dll` |

The declared spelling is always tried first and stays authoritative; these are
what a host is asked for when its own convention differs.

### Getting the libraries onto a machine

Only the **runtime** library is needed — never the headers, and never a compiler
beyond the `cc` that builds the shims. That is the whole point of binding through
`#c`: `maria/src/stmt.c` hand-declares `MYSQL_BIND` precisely so a consumer needs
`libmariadb.so.3` and not `libmariadb-dev`.

| | sqlite | libpq | libmariadb | duckdb |
|---|---|---|---|---|
| Debian / Ubuntu | `libsqlite3-0` | `libpq5` | `libmariadb3` | not packaged — release tarball |
| macOS | in the base system, **in the dyld cache and not on disk** | `brew install libpq` — **keg-only** | `brew install mariadb-connector-c` — **keg-only** | **release tarball; brew ships the CLI only** |
| Windows | not shipped — vcpkg or the official binaries | same | same | same |

Every cell in the macOS row is measured ([MACOS_RESULTS.md](MACOS_RESULTS.md)),
and two of them corrected a guess this document previously made:

- **`brew install duckdb` gives you no library.** It installs the CLI binary and
  nothing else — there is no `libduckdb.dylib` anywhere under `/opt/homebrew`. The
  shared library needs the release tarball, exactly as the Debian cell says.
- **libpq and the MariaDB connector install keg-only.** Both are present under
  `/opt/homebrew/opt/<name>/lib`, and a bare `dlopen("libpq.5.dylib")` still fails
  with *"not in dyld cache"*; adding `DYLD_LIBRARY_PATH=$(brew --prefix libpq)/lib`
  makes it succeed. So `host_library_loadable` answers **false** on a Mac that has
  libpq installed, and **P5 on macOS is a search-path question, not a
  `brew install` one** — either loft learns the `/opt/homebrew/opt/<x>/lib` layout,
  or the environment supplies it.

That is why all four are declared `[c] optional-libs` and why the fixture reaches
its own copy through `LD_LIBRARY_PATH` rather than a system install. Provisioning
them on a CI runner is P5, and a declared skip there is an acceptable answer; an
untested green is not.

## wasm and the browser — a defined NO

**This target is a known gap and stays one.** A browser cannot open a TCP socket
to a database server, so no amount of build work makes libpq or libmariadb
reachable; @PLN24 arc E names a database client as its example of a capability
that does not exist in wasm. The out-of-process route (@PLN119) is a real answer
and is not this plan's.

What is owed is therefore not support — it is a **clear error**, and that is now
what arrives (@PLN24 arc E). Both wasm shapes refuse a REACHABLE `#c` call in one
message naming the loft function, the C symbol, the declaring package and the
target; an unused declaration still builds, so a library carrying `#c` bindings
is not thereby unbuildable for wasm. What follows is what the author used to read
instead — rustc's account of loft's internals:

```
error[E0433]: cannot find `c_call` in `loft`
  --> /…/loft_wasm_3225277/prog.rs:36:32
   |
36 |   match P.get_or_init(|| loft::c_call::resolve_native("sqlite3_open", …))
   |                                ^^^^^^ could not find `c_call` in `loft`
note: found an item that was configured out
  --> src/lib.rs:145:9  |  pub mod c_call;
   ::: src/c_call.rs:34:8  |  #![cfg(feature = "native-extensions")]
```

— once per bound symbol, **including symbols the program never called**. The cause
was plain: `#c` codegen emitted `loft::c_call::resolve_native(…)` unconditionally,
and `c_call` is gated behind `native-extensions`, which a wasm target does not
enable. Two further cells were worse than this one, because they did not fail at
all: a symbol the WASI sysroot happens to export LINKED and then trapped at the
call (wasm32 is ILP32, so the host-width extern is a signature mismatch), and
`c_library_available` — the query a library is told to ask before calling into an
optional backend — did not compile under `--html`.

The fix is the one loft-library's cross-target gate wants: a declared-unsupported
column rather than a claimed-but-broken one. Nothing `#c` is emitted on a wasm
target, and the refusal sits at the CALL, which scopes it to reachability for
free.

## The ladder

Each step is verifiable on its own, and the order is chosen so that no step can
produce a green that means nothing.

| step | what it proves | how it is proved |
|---|---|---|
| **P1** — done | the Windows shim links | W1 + W2; **verified** — zero `LNK1181` / `LNK4044`, the `#c` tests run and compare values |
| **X1** — built | 64-bit values survive Windows | exact-width spellings (`int64_t` / `uint64_t`) wherever a 64-bit value crosses; `long` kept only where the real header says `long`. Linux unchanged; **still unconfirmed** — the tests that would show it were failing earlier, at the link |
| **X4** — built | the fixture library links too | the c_abi `Makefile` gains a Windows arm emitting `lc_types.lib` |
| **X5** — built | a POSIX name that Windows spells differently | `write` → `_write`, branched per host in the generated fixtures |
| **P2** — built | the probe asks the HOST's question | `platform::host_library_loadable` translates the declared soname to the host's spellings and `dlopen`s them; sqlite stops being skipped on macOS and Windows |
| **P3** — done for sqlite | macOS runs what Linux runs | **confirmed on Apple silicon**: `["sqlite"]` exercised, matching the same hard-coded `sqlite` / `sqlite bound` / `sqlite tx` constants Linux matches, `--interpret` == `--native`. postgres / maria / duckdb remain correct skips until P5 answers the search path |
| **P4** — done | Windows runs what Linux runs | **measured**: `--test native` on `windows-latest` is 22 passed / 0 failed, reporting `@PLN23 backends exercised: ["sqlite"]` against the same hard-coded strings Linux matches, on both loft backends. Two defects stood between X5 and that line, and neither was about linking — X7, X8 above |
| **P5** — answered, by the skip | the servers, or a declared skip | **a declared skip.** A stock macOS or Windows runner ships none of postgres, maria or duckdb; each backend now answers `c_library_available` **for itself** (X8), and the run PRINTS the list it exercised, so the skip is reported rather than assumed. Provisioning those servers is a CI question, and this plan does not take it |
| **P6** — done | wasm refuses clearly | **@PLN24 arc E**: one named error naming the function, the C symbol, the package and the target, on BOTH wasm shapes; asserted end to end (`pln24_a_reachable_c_binding_is_refused_end_to_end_on_wasm`) including the "exactly one message" half, so it cannot regress into a rustc dump. An unused `#c` declaration still builds |

P1 and P6 are independent of everything else and can land in either order. **P2
is the one that decides whether P3 and P4 are worth running**, so it comes before
both.

### What P1 and P2 changed

**P2 asks the question the way the dynamic linker asks it.** The probe is now
`platform::host_library_loadable`, beside `host_lib_variants` where the spelling
rules already live: it takes the declared `libsqlite3.so.0`, expands it to this
host's spellings, and **`dlopen`s** them. Two reasons it opens rather than stats.
The spelling is the host's, not the declaration's — the original bug. And on
macOS 11+ system libraries live in the dyld shared cache and are **absent from
the filesystem**, so anything built on `Path::exists` reports "not installed" for
a library that works. `dlopen` consults the shared cache, `LD_LIBRARY_PATH` /
`DYLD_LIBRARY_PATH`, the `PATH` DLL search and the system directories — every
route the real call takes, which is what makes a `true` here mean what a `true`
at the call site means.

That second reason was written from general knowledge and flagged as such; it is
now **measured**. On Apple silicon `stat /usr/lib/libsqlite3.dylib` fails while
`dlopen` succeeds — for *both* spellings `host_lib_variants` produces. So the
`dlopen` design is required, not a preference, and a file-existence probe would
have reported macOS's own sqlite as missing.

**It deliberately does NOT use loft's own `c_library_available`,** which was the
first design and is the more attractive one ("one home for the fact"). That
function additionally requires every `#c` symbol attributed to the library to
resolve — and **a `#c` annotation never names the library it came from**, so a
package that declares a library *and* builds a shim has both symbol sets
attributed to the one soname. Measured on this box: `c_library_available(
"libsqlite3.so.0")` answers **false** with `/lib/x86_64-linux-gnu/libsqlite3.so.0`
present and every sqlite call working. Adding an `eprintln` inside the function
flips it to true — the signature of loft's known duplicate-statics hazard, where
a package cdylib links its own copy of loft and `DECLARED_LIBS` / `LIB_SYMBOLS`
therefore exist twice (the same effect the code comments already record for
`duckdb_available()`). **That is a real pre-existing defect, tracked separately.**
It is named here because it is the reason the tidier design was abandoned, and
because anything else built on that function will inherit the same flakiness.

The test keeps a list of which backends actually ran, prints it
(`@PLN23 backends exercised: ["sqlite", "postgres", "maria"]`), and **requires
sqlite on Linux** — the cell with no server to be unreachable, so a skip there
can only mean the library vanished or the availability question broke. Proven to
FIRE by pointing the probe at a library that does not exist: the list drops to
`["postgres", "maria"]` and the test fails on the guard instead of passing. That
is the assertion the old shape could not make, and the reason macOS was green for
months.

**P1 gave the shim an import library.** `platform::shim_implib_name` /
`shim_implib_args` are the Windows counterpart to the macOS `-Wl,-install_name`
arm, and like it they are pure functions of a passed-in `LibOs` so both spellings
are checkable from a Linux box. `c_shim.rs` builds the `.lib` to a temporary
beside the DLL's own and renames both, because the DLL's rename is what publishes
the shim to the cache check — a consumer that found the `.dll` without the `.lib`
would fail exactly as if the flag had never been added. W2 (the MSVC guard on
`-Wl,--allow-multiple-definition`) landed with it.

**P1 is not verified on Windows** and cannot be from here. The assumption it
tests is that a MinGW-produced COFF import library satisfies MSVC `link.exe`; if
that fails, the fallbacks are named under W1 above.

## What this does not cover

- **Which loft backend.** Every cell above must hold on `--interpret` and
  `--native` alike; the existing test already asserts the two agree, and that
  assertion carries to each new platform unchanged.
- **Architecture.** aarch64 Linux and Apple silicon differ from x86-64 in the
  register/stack split the `#c` trampolines are built on (@PLN24 arc B). Proven
  on neither; not a naming problem, so not this document's.
- **Shipping a database library.** Everything here assumes the C library is
  already on the machine. Vendoring one, or `loft install` placing it, is
  @PLN21's question.

## See also

- [README.md](README.md) — the plan, and the S-ladder this runs beside.
- [@PLN24 arc E](../24-c-abi-binding/README.md) — what a `#c` library does on
  wasm, and the three honest answers.
- [WINDOWS.md](../../WINDOWS.md) — the host's own notes.
