<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Third-party code — what we ship that we did not write

Every package declares the code it ships, links or builds in that someone else wrote: where
it came from, which version, and which advisory registers report on it.  `make thirdparty`
checks each declaration against the tree and asks those registers, for loft and for every
library in the registry.  A dependency nobody declared, a copy that differs from its upstream,
an advisory nobody has read: each is a line that needs action.

## The declaration

A library writes it in its `loft.toml`; loft writes it in `thirdparty.toml` at the repository
root.  One `[thirdparty.<name>]` table per dependency, in one of four kinds:

| `kind` | What it is | Required keys |
|---|---|---|
| `vendored` | source copied into the package and compiled into what it ships | `path`, `upstream`, `commit`, `digest`, `advisories` |
| `sdk` | fetched by a build step and compiled into what it builds (ESP-IDF, the Tanmatsu BSP) | `source`, `version`, `pinned`, `advisories` |
| `system` | linked or opened at run time from the machine, never shipped | `names`, `provider` |
| `cargo` | the Rust crates a shipped `Cargo.lock` pins | `lockfile` (a path or a list); `compiles-c` lists the crates whose build compiles C |

```toml
[thirdparty.stb_vorbis]
kind       = "vendored"
path       = "c/vendor/stb_vorbis"                # relative to the manifest
upstream   = "https://github.com/nothings/stb"    # a git URL
commit     = "2c980bb59875b0d32144a71867fbdebb2f77cd20"
version    = "1.22"
digest     = "sha256:…"                           # scripts/thirdparty.py digest <path>
advisories = ["nvd:stb_vorbis", "osv:commit"]
renames    = { "lodepng.c" = "lodepng.cpp" }      # ours = upstream's name, when renamed
modified   = ["x.c"]                              # files allowed to differ from upstream
upstream-reviewed = "<upstream sha>"              # upstream changes read up to here

[thirdparty.stb_vorbis.reviewed]                  # one line per advisory a register reports
CVE-2019-13217 = "fixed: in v1.17, before our pin"
CVE-2023-45675 = "applies: …"

[thirdparty.esp-idf]
kind    = "sdk"
source  = "https://github.com/espressif/esp-idf"  # or esp-registry:<namespace>/<name>
version = "v6.0.2"
pinned  = "src/esp_app.rs:IDF_VERSION"            # the `const` that pins it

[thirdparty.x11]
kind     = "system"
names    = ["X11", "Xtst"]                        # as in -lX11, libX11.so.6, X11.dll
provider = "the OS (Xorg / XWayland)"             # who ships it and patches it
```

loft reads none of these keys: an older loft ignores the table, so a library can declare
before every user upgrades.

## The registers

| `advisories` entry | Asks | Notes |
|---|---|---|
| `nvd:<keyword>` | NVD's CVE search | needs a person's verdict for every hit; `NVD_API_KEY` lifts the rate limit |
| `osv:commit` | OSV.dev, by the pinned `commit` | answers for the whole upstream repository, so a multi-file repo (stb) reports advisories for files we do not vendor |
| `github:<owner>/<repo>` | that repository's security advisories (`gh api`) | an advisory whose version range leaves out our `version` is settled without a verdict |

A verdict starts with one word: `fixed:` (our pin has the fix), `not-reached:` (the code is
not compiled in or not called; say how you know), or `applies:`.  `applies` stays a line that
needs action until the dependency is updated, because the package ships a known flaw.

Rust crates go through `cargo audit`, for every `Cargo.lock` a package ships, declared or not;
an undeclared one needs action.  Each vulnerability, and each unmaintained, unsound or yanked
crate, needs a verdict in the `cargo` entry's `reviewed` table, keyed by its RUSTSEC id (a
yanked crate has none: `yanked:<crate>@<version>`).  The report also counts the patch
updates `cargo update` would take.  A `[native] crate` without a lockfile needs action,
because its dependencies cannot be audited.

```toml
[thirdparty.rust]
kind     = "cargo"
lockfile = ["Cargo.lock", "loft-ffi/Cargo.lock"]

[thirdparty.rust.reviewed]
RUSTSEC-2020-0077 = "applies: memmap 0.7 is unmaintained — replace with memmap2"
```

## What the audit checks

- **Undeclared code.**  A C, C++, JS, CSS or shader file under a `vendor/` directory, or with
  a copyright notice that is not ours, must lie under a declared `vendored` path.
- **Unattributed code** (loft only).  Every file loft's binary embeds (each `include_str!` /
  `include_bytes!` target, wherever it lives) and every file under `src/c/` must carry our
  licence header, match a glob in `[own] paths`, or lie under a `vendored` path.  A notice is
  a heuristic; this list is exact, so third-party code without a notice cannot pass.
- **Undeclared libraries.**  Every library the package links or opens must be named by a
  `system` entry: `[c] libs`, `optional-libs`, `[native] runtime-libs`, a soname, framework or
  DLL literal in its C (vendored code included), `#pragma comment(lib, …)`, loft's own `-l`
  flags, and what the built `target/release/loft` is linked against (`readelf -d`).
- **Rust crates that are native.**  Over the dependency graph each shipped platform resolves
  (normal dependencies only — a build tool does not ship), a crate whose build script compiles
  C must be listed in the `cargo` entry's `compiles-c`, and every library a crate links
  (`#[link(name = …)]`, also inside `cfg_attr`) must be named by a `system` entry.  `libc` is
  the platform C library and is not listed.  A lockfile that does not match its
  `Cargo.toml` needs action: what builds is then not what was audited.
- **Undeclared build inputs** (loft only).  An ESP component dependency or a `git clone` URL
  in loft's Rust source must be declared as `sdk`, and its `pinned` constant must hold the
  declared `version`.  An `esp-registry:` source is resolved through the component registry
  for its `target` (default `esp32p4`): every component it brings in is listed, and the
  security advisories of each one's GitHub repository count as the entry's own.
- **Provenance.**  A vendored path must hash to its `digest`, and every file must be
  byte-identical to the upstream file of that name at `commit` (`renames`, `modified`).
  `scripts/thirdparty.py identify <dir> <url>` finds the commit a copy came from.
- **Freshness.**  A vendored dependency reports upstream commits since the pin that touch one
  of our files, until `upstream-reviewed` names the commit they were read up to.  An `sdk`
  dependency reports a newer release in its own `major.minor` series as needing action, and a
  newer series as information only.

## Running it

```bash
make thirdparty                                 # loft + every published library
make thirdparty ARGS=--check                    # silent unless something needs action
make thirdparty ARGS="--repo ../loft-libs-game" # one library checkout (its testbed)
make thirdparty ARGS="--source main"            # each library repository's main, not its release
make thirdparty ARGS=--offline                  # the tree checks only
make thirdparty ARGS="--gate --repo <pkg-dir>"  # the gate below, on one package
```

The libraries are the latest release each one lists in the registry index, which is what
`loft install` hands out.  Network answers are cached for a day under `~/.cache/loft/thirdparty`
(`--refresh` asks again).  The release checklist runs it as `A-thirdparty`
([RELEASE.md](RELEASE.md)); a library adds a dependency by adding its table in the same change
([LIBRARY_AUTHORING.md](LIBRARY_AUTHORING.md) contract rule 29).

## Where it gates

New code cannot reach a user undeclared: `--gate` (no network, no registers — it fails on what
is undeclared, never on what an advisory says) runs at every way in.

| Way in | Gate | What it reads |
|---|---|---|
| a loft PR or push | `daily-build.yml`, every platform leg | the tree, and that leg's built binary (`readelf`, `otool -L`, the PE import table) |
| a library PR or push | `library-ci-reusable.yml`, per package | the package, and its `[c] shim` built and read |
| our library to the registry | `registry_maintain.sh`, before the tag | the package as it will be tagged |
| a submitted library | `vet-lib.sh` step V7 | the package at the submitted ref |
| any version signed into the index | `registry-sign.sh`, every route | the downloaded tarball, the bytes being signed |

What the registers know is asked nightly (`miri.yml`, the `audit` job): a new advisory against
anything loft or a published library ships fails that job the next morning.

A package's own code is recognised by our licence header; a package by another author names
its own notice with `[own] notice = "<text>"`, and lists files that carry none in
`[own] paths`.

