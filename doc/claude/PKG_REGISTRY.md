<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# PKG.REG — file-based registry MVP

> **What shipped differs from this draft in its names**: the index is `index.json` in
> `loft-lang/registry` (this draft says `registry.json`), it is signed on the maintainer's
> hardware key, and a release tag is `<name>-v<version>`.  For how to publish today, read
> [LIBRARY_PUBLISH.md](LIBRARY_PUBLISH.md) § 4 and [REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md);
> this doc is the design record.  Its revision history and the R1–R9 record are in
> [PKG_REGISTRY-history.md](PKG_REGISTRY-history.md).

Draft 2026-05-24.  This is the design for the **MVP** of the loft
package registry.  Goal: ship `loft install <name>` against a static
file before designing a full server.  The same URL surface migrates
to a server later with no client-side changes.

This doc fleshes out the "(b) central registry server (GitHub Pages +
static index acceptable for MVP)" sub-phase from
[PACKAGES.md § Open work](PACKAGES.md#open-work).

---

## The invariant — end-user experience is identical to a real server

The whole point of choosing a file-based MVP is that **the loft
client never knows the difference**.  Migration from file to server
later is a server-side swap; users do not need to change a flag,
edit a config, or upgrade the loft binary.

Concretely, **EVERY** part of the user-visible surface stays
constant across MVP → server:

| User-visible surface | MVP behaviour | Future-server behaviour |
|---|---|---|
| `loft install crypto` | Downloads tarball, extracts to `~/.loft/registry/`, writes `loft.lock`. | Same. |
| `loft install crypto@0.1.0` | Same — pinned version. | Same. |
| `loft install` (no args) | Reads `loft.toml`, resolves, installs. | Same. |
| `loft.toml` `[dependencies]` syntax | `crypto = "^0.1"` etc. | Same. |
| `loft.lock` format | TOML, see [§ Consumer's lock file](PKG_REGISTRY_RESOLUTION.md#consumers-lock-file--loftlock). | Same. |
| Sha256 verification | Mandatory. | Same. |
| Cache layout (`~/.loft/registry/<pkg>-<v>/`) | Identical. | Same. |
| Error messages | "package X not found", "version Y conflict", etc. | Same. |
| Search (`loft search foo`) | Client-side grep on cached index. | Server-side filter, faster — but same CLI args + same output shape. |
| `loft info <name>` | Read from cached index. | Read from server endpoint; same output. |
| Offline mode (`--offline`) | Uses cache. | Same. |
| Tarball format | See [§ Tarball format](#tarball-format). | Same. |

Things that change at MIGRATION time and are NOT user-visible:

- Where `registry.json` is hosted (a static file vs a server
  endpoint that emits the same JSON).
- How a publisher uploads a new version (PR against the registry
  repo vs `POST /v1/publish` with an auth token).
- Operational surface (no infra vs `loft-lang.org` server +
  database).

This invariant is load-bearing: the server can be built and
deployed without breaking any installed loft binary in the wild.
Every design decision below preserves it.

---

## Why a file is enough

A registry needs to answer two questions:

1. **"Given a package name + version, where is the tarball?"** —
   index lookup.
2. **"Did I get the tarball I asked for?"** — verification.

That's it.  Server-side features (search, auth, atomic publish,
download counts, yanking) are **non-essential for an early ecosystem**.
Cargo's earliest model was file-based; npm's still has a fundamentally
static GET-by-name shape.

Trade-off matrix:

| Concern | File-based MVP | Full server |
|---|---|---|
| Discovery (`loft search`) | Client-side grep of the index | Server endpoint |
| Auth on publish | PR review on the index repo | Server token auth |
| Atomic publish | git merge serialisation | DB transaction |
| Yanking a version | Edit the index file | API call |
| Download analytics | None | DB rows |
| Hosting cost | $0 (GitHub Pages / raw) | $20+/mo for a tiny server |
| Time to ship | ~1 week | ~1 quarter |
| Migration cost (later) | Server reads/writes same JSON shape | n/a |

The MVP buys us the per-library extraction work (Phase 4-8 in
[`lib_plans/12-library-extraction/`](lib_plans/12-library-extraction))
without funding a service.

---

## Index format — `registry.json`

A single JSON file, hosted at a stable URL.  Suggested:
`https://loft-lang.org/registry.json` (or
`https://raw.githubusercontent.com/loft-lang/registry/main/index.json`
for the lowest-infra option).

### Decoupled lifecycle — Debian-style "the registry IS a repo"

The registry lives in **its own GitHub repository**, completely
separate from the `loft-lang/loft` compiler repo.  This decouples
two lifecycles that have no reason to be linked:

| Lifecycle | Owner | Cadence |
|---|---|---|
| Loft compiler releases | `loft-lang/loft` (this repo) | Minor every 1-3 months |
| Package index updates | `loft-lang/registry` (separate repo) | Anytime — driven by package author publishes |

Mirrors apt's model: Debian's `Packages.gz` lives on mirror servers,
updated independently from `dpkg`/`apt` binary releases.  Anyone can
host a mirror by cloning the package list; the user picks which
mirror to consult via `sources.list`.  Loft's `LOFT_REGISTRY_URL`
env var (defaults to the canonical
`raw.githubusercontent.com/loft-lang/registry/main/index.json`)
plays the same role.

**Consequences this gets us for free**:

1. **Publishing a new package doesn't require a loft release.**  An
   author tags v0.1.0, opens a PR against `loft-lang/registry`, and
   the package is live for everyone the moment the PR merges.  No
   waiting for the next loft minor.
2. **Mirrors are trivial.**  Anyone with a GitHub account can fork
   `loft-lang/registry`, run their own additions, and point users
   at it via `LOFT_REGISTRY_URL=https://raw.githubusercontent.com/<user>/registry/main/index.json`.
   Useful for corporate / air-gapped deployments.
3. **The loft binary is small.**  It does not bundle a package list
   (which would go stale immediately).  The list is fetched at
   install time.
4. **History is git.**  Every package version add / yank is a
   commit in `loft-lang/registry`.  `git log` IS the audit trail.
   No separate publish-log infrastructure.
5. **CI runs against the registry repo independently** — same
   GitHub Actions style as any other repo.  PR validation
   (schema lint, sha256 verify) is its own workflow file in that
   repo.
6. **Bisecting a regression is `git bisect` on the registry repo**
   — when "package X 0.2.0 broke my build", you can `git bisect`
   `loft-lang/registry` to find the commit that added the bad
   version (and roll back by reverting it).

### Anatomy of the `loft-lang/registry` repo

```
loft-lang/registry/
├── README.md                       (publish workflow + how to PR)
├── index.json                      (the canonical registry — what loft fetches)
├── schema/
│   └── index-v1.json               (JSON schema for the index — used by CI lint)
├── tools/
│   ├── validate.py                 (PR validation: schema lint + sha256 verify)
│   └── add_version.sh              (helper: appends a version row)
└── .github/
    └── workflows/
        └── pr-validate.yml         (CI: runs validate.py on every PR)
```

`index.json` is the only file `loft install` cares about.
Everything else is repo-internal infrastructure for keeping
`index.json` correct.

### Schema

```json
{
  "schema_version": 1,
  "updated": "2026-05-24T08:00:00Z",
  "packages": {
    "<pkg_name>": {
      "description": "Short one-line description (optional).",
      "homepage": "https://github.com/loft-lang/loft-<pkg> (optional)",
      "categories": ["crypto", "stdlib-augment"],
      "yanked": ["0.1.2", …],
      "versions": {
        "<semver>": {
          "url": "https://github.com/loft-lang/loft-libs-<domain>/releases/download/<pkg>-v<semver>/<pkg>-<semver>.tar.gz",
          "sha256": "<64-hex>",
          "size": <bytes>,
          "loft": ">=0.8",
          "subpath": "<pkg>",
          "deps": {
            "<dep_pkg>": ">=0.1"
          },
          "triggers": ["matches:text", …],
          "api": [
            { "sig": "pub fn slugify(s: text) -> text", "doc": "One-line summary." },
            …
          ],
          "published": "2026-05-24T08:00:00Z"
        },
        …
      }
    },
    …
  }
}
```

`conflicts` / `replaces` / `provides` / `binaries` / `prerelease` are reserved
schema slots (resolver + pre-built-binary support deferred, see the field
reference below) — every version in the live index omits them; a publisher
only sets one explicitly once the corresponding feature ships.

### Field reference

| Field | Required | Notes |
|---|---|---|
| `schema_version` | yes | Integer.  Bump on incompatible schema changes.  Today: `1`. |
| `updated` | yes | ISO-8601 UTC timestamp of the last index modification.  Lets clients detect stale caches. |
| `packages.<name>.description` | **yes** | Display string for `loft search` output, and the library's one line in the generated catalogue.  At least 10 characters — `tools/validate.py` gate 1 rejects a shorter one. |
| `packages.<name>.homepage` | **yes** | http(s) URL to the package's repo / docs; the catalogue links to it.  Gate 1 rejects a non-http value. |
| `packages.<name>.categories` | **yes — non-empty** | Array of category tags for discovery.  Free-form; `loft search --category crypto` filters on them.  Inspired by Debian's `Section:` field and cargo's `categories = […]`.  Declared as `[package] categories = ["graphics", "game"]` in the library's `loft.toml`; `registry_maintain.sh` reads it from there and **refuses to publish a package new to the index without one**.  In use today: `geometry` `graphics` `text` `net` `world` `game` `time` `math` `random` `crypto` `encoding` `cli` `plugins` `asset-format` `animation` — reuse a tag before minting one. |
| `packages.<name>.yanked` | no | Array of yanked versions.  Yanked versions stay listed so `loft.lock`-pinned consumers don't break, but new installs / version resolution skip them. |
| `packages.<name>.versions.<semver>.url` | yes | Direct download URL for the tarball.  Convention: GitHub release asset. |
| `…sha256` | yes | Lowercase hex of the SHA-256 hash of the tarball bytes.  Verified post-download. |
| `…size` | yes | Byte length.  Bandwidth sanity check + early-abort on giant downloads. |
| `…loft` | yes | Required loft interpreter version.  `>=0.8` syntax mirrors `loft.toml::[package] loft`.  A **range**, because the platform is the one axis a library does not pick a single point on. |
| `…api_compatible_with` | new entries | Bare version.  The oldest release of THIS package whose public API this version is still a drop-in for.  Mirrors `loft.toml::[package] api_compatible_with`; emitted by `loft publish` / `loft package`, which refuse to produce an entry without it.  A real version, not an epoch — the claim is verified by fetching that release and running its own tests against this source, and a range would name nothing to fetch.  Absent on versions published before the contract existed. |
| `…data_compatible_with` | new entries | Bare version.  The oldest release whose stored / wire data this version still reads.  Separate from `api_compatible_with` because the failures differ in kind: an API break costs a recompile, a data break costs someone's stored file.  `hex_terrain` is the worked example — it kept its API and changed what it computed over stored heights, which one number cannot express. |
| `…subpath` | for monorepos | Package directory within the release repo (e.g. `crypto` inside `loft-libs-core`).  The loft-lang libraries are **domain monorepos** (`loft-libs-core`/`-net`/`-graphics`/`-game`/`-world`/`-assets`/`-docs`) tagged `<pkg>-v<version>`, so `subpath` tells the installer where the package lives in the unpacked tarball.  Omit for a one-repo-per-package layout. |
| `…deps` | no | Inter-package dependencies.  Resolved during install; failures abort before any download. |
| `…triggers` | no | Array of `"name:receiver"` strings (e.g. `"matches:text"`) — Tier-1 lazy-load triggers derived from this version's `pub fn` method surface at publish time, so a consumer's resolver can map `obj.method()` to the owning package without having the source.  Populated when the package opts into `[triggers]`; globally unique across the registry (enforced at PR gate 4, see REGISTRY_SUBMIT.md § Trigger uniqueness). |
| `…api` | no | Array of `{sig, doc}` — one entry per `pub` item (struct/fn) in this version's source, extracted by the same `parse_pkg_api` `loft api` uses, so `loft search` can answer "is there a function that does X, and how do I call it" without the source.  Per-version (an old pin still describes what it actually shipped); re-derived by the registry from source so it can't drift.  Empty for indexes published before this field existed. |
| `…conflicts` | no | **(Schema slot — resolver support deferred.)** Array of package names + version constraints that cannot coexist with this version in the same dependency graph.  Inspired by Debian's `Conflicts:`.  Reserved field so the schema doesn't need a bump when the resolver gains support.  Omitted (not emitted as `[]`) on every version in the live index today. |
| `…replaces` | no | **(Schema slot — resolver support deferred.)** Array of packages this version takes over from (rename / fork takeover).  Inspired by Debian's `Replaces:`.  Omitted today, same as `conflicts`. |
| `…provides` | no | **(Schema slot — resolver support deferred.)** Array of virtual capability names this version supplies.  Lets a different package satisfy the same `deps` constraint — e.g. `crypto-bcrypt` and `crypto-argon2` both provide `password-hash`.  Inspired by Debian's `Provides:`.  Omitted today, same as `conflicts`. |
| `…binaries` | no | **(Schema slot — pre-built distribution deferred.)** Map of `<target-triple>` → `{url, sha256}` pointing at pre-built cdylibs.  When present and a triple matches, `loft install` skips the local `cargo build` step.  When absent, consumer builds from the `native/` source in the tarball.  Inspired by Debian's per-arch `.deb` files.  Omitted today, same as `conflicts`. |
| `…prerelease` | no | Boolean.  `true` for beta / rc versions.  `loft install <pkg>` (no version) skips prereleases by default; `loft install <pkg>@<v>` honours an explicit pin; `loft install <pkg>@beta` resolves the latest prerelease.  Inspired by Debian's `testing` / `unstable` release pockets.  Omitted (defaults to `false`) on every version in the live index today. |
| `…published` | yes | ISO-8601 publish timestamp.  Audit trail. |

### Why JSON not TOML

The package manifests are TOML for human authoring.  The registry
index is JSON because it's machine-generated, never hand-edited, and
loft's existing `serde_json` (or `data.rs`'s JSON walker — see
[QUALITY-history.md § P54](QUALITY-history.md))
parses it without a TOML dependency on the client side.  Smaller
binary.

### Index size estimate

A version row without `api` is ~384 bytes (measured over the live
index), so the original sizing held: 100 packages × 5 versions ≈
200 kB, and 10,000 × 20 ≈ 77 MB.

**The `api` field changed that by an order of magnitude.**  It carries
one `{sig, doc}` per `pub` item *per version*, and on the live index it
is **91 % of the document** (494,993 of 544,552 compact bytes across
2,344 entries).  Re-measured with it: the 10,000 × 20 projection is
**825 MB**, not 80 MB — and loading it costs ~11 s at 3.7 GB RSS,
because the parsed form runs ~4.5× the file size.

Growth so far — 10 kB (May) → 37 kB (Jun) → 303 kB (Jul) → 693 kB
(Aug, 36 packages / 128 versions).  At ~5.4 kB per version row the
document reaches 50 MiB at roughly **10,000 package-versions**: 500
packages with 20 releases each, or 2,700 at today's 3.6.  Version rows
are never removed, so that number only moves one way.

**What this means for the format.**  Nothing needs doing at today's
size, and the client no longer breaks at a fixed ceiling (see § Download
ceiling).  But the shape that keeps scaling is to split *resolution*
data from *description* data: the thin index above (`url`, `sha256`,
`size`, `loft`, `deps`, `yanked`) stays a single fetch to ~200,000
versions, while `api` + docs move to per-package files fetched on
demand — cargo's sparse-index shape, which needs no server and keeps
every URL static, so § The invariant still holds.

---

## Tarball format

Each package ships as a gzipped tarball produced by `loft package`
(the publish-side CLI command).  Layout inside the tarball:

```
<pkg>-<version>/
├── loft.toml              (the manifest — verbatim from source)
├── src/                   (loft source files, verbatim)
│   └── <pkg>.loft
├── native/                (optional — only for packages with native code)
│   ├── Cargo.toml
│   ├── build.rs
│   └── src/lib.rs
├── tests/                 (optional — tested by `loft install --tests`)
│   └── *.loft
└── README.md              (optional)
```

**Excluded** (covered by `loft package`'s default ignore list):
- `.git/`, `target/`, `.loft/`
- Anything in `.gitignore` (if present)
- IDE state (`.vscode/`, `.idea/`)

The tarball is the canonical install unit.  Extracted into
`~/.loft/registry/<pkg>-<version>/` on install.

### Sha256 + size verification

After download, the client:
1. Hashes the tarball bytes (SHA-256).
2. Compares to `versions.<v>.sha256` from the index.
3. Compares byte count to `versions.<v>.size`.
4. Either matches → extract; mismatch → abort with a clear error
   and leave the cache untouched.

Hash mismatch is treated as a hard failure (likely indicates a
corrupted download or a tampered mirror).  Size mismatch is the
same family of error.

---

## Local cache layout

`~/.loft/` holds everything `loft install` writes to disk on the
consumer's machine.  Layout:

```
~/.loft/
├── trust-root/
│   └── registry-signing-key.bin       (private key — maintainer-only,
│                                       absent on consumer machines)
├── registry/
│   └── <registry-url-hash>/
│       ├── index.json                 (cached registry index)
│       ├── index.json.sig             (signature, verified on every load)
│       ├── index.json.fetched_at      (ISO-8601 — staleness clock)
│       └── index.json.etag            (HTTP conditional-GET hint)
├── packages/
│   └── <package>/
│       └── <version>/                 (extracted tarball, ready to compile)
│           ├── loft.toml
│           ├── src/
│           └── native/
└── cache/
    └── <sha256>.tar.gz                (raw downloaded tarballs — CAS-keyed
                                        so duplicate SHAs across packages /
                                        registries de-duplicate naturally)
```

**Why this shape**:

- **One `<registry-url-hash>/` subdir per registry URL.**  Lets the
  client run against `loft-lang/registry` (default) plus mirrors
  (`LOFT_REGISTRY_URL=...`) without their indexes colliding.  Hash is
  short (8 hex chars of SHA-256 of the URL).
- **`packages/<name>/<version>/` is the install address.**  When loft
  resolves `crypto = "0.1.0"`, it looks for
  `~/.loft/packages/crypto/0.1.0/loft.toml`.  Multiple versions
  coexist by design — project A pinned to 0.1.0 and project B pinned
  to 0.2.0 both work without re-download.
- **`cache/<sha256>.tar.gz` is content-addressable.**  The same
  bytes are stored once even if multiple registry entries (or
  mirrors) reference them.  Also gives the trust-root re-validation
  chain a stable target — the SHA-256 in the lockfile points at the
  same bytes years later.
- **Atomic install**: download to `cache/<sha>.tar.gz.tmp`, hash-verify,
  rename to final.  Extract to `packages/<name>/<version>.tmp/`,
  rename to final.  No half-written state survives a crash.

**Index refresh policy** (per registry URL):

- TTL: 1 hour by default.
- Re-fetched when `index.json.fetched_at` is older than 1 hour, OR
  when `loft install --refresh` is passed, OR when a requested
  package is missing from the cached index.
- Conditional GET via the cached `index.json.etag` — if the
  registry hasn't changed, the response is 304 and no bytes
  transfer.  **Designed, not implemented**: `fetch_index` sends no
  `If-None-Match`, and the `index.json.fetched_at` path that
  `index_paths()` returns is read by nothing.  Every refresh
  transfers the whole document (gzipped by the transport — ureq
  carries `flate2`, so ~6.3× smaller on the wire).
- Signature verified on every load, even cache-hit paths.  **The advisory feed
  (`advisories.json`) is held to the same sentence**, through one verified
  reader rather than four call sites — one of which used not to verify at all,
  so `--offline` served whatever was on disk (loft#1048).  It is also cached
  only AFTER its signature is checked: writing first and checking after leaves
  a refused feed on disk, where the next run reads it against the previous
  still-valid signature and fails the same way, with no retry able to clear it.
- **Atomic refresh, and a reader that waits one out.**  Each of
  `index.json` and its `.sig` is written beside itself and renamed into
  place, so a concurrent reader gets the whole old file or the whole new
  one — a plain write truncates first, and on a 692 KB index that window
  swallowed 194 of 200 concurrent reads in the regression harness.  The
  pair is still two renames and cannot be swapped in one step, so a
  reader that finds the signature not matching re-reads the pair for a
  bounded moment before believing it: that mismatch is otherwise
  reported as *the signature exists but doesn't verify against any known
  key*, a failure deliberately un-bypassable even with
  `--allow-unsigned`, which sends the reader to look at trust roots when
  the cause was a refresh landing mid-run (loft#1045).  The signature is
  an exact oracle for this — one that verifies over the content it was
  read with proves both came from the same generation — so a pair
  accepted after settling is matched, and one that never agrees is
  refused in the words it would have used anyway.

**Download ceiling.**  A single HTTP response is capped at 512 MiB
(`DEFAULT_MAX_DOWNLOAD`), overridable with `LOFT_MAX_DOWNLOAD=<size>`
(`1G`, `512M`, `0` = no ceiling); a typo keeps the default and says so.
Exceeding it is a **refusal naming the ceiling**, never a truncation.

The ceiling was 50 MB and truncated silently, which made it a trap
rather than a guard: a 70 MB index came back cut to exactly 52,428,800
bytes and failed as `JSON parse error: unterminated string` at a byte
offset inside an unrelated package.  Worse, the number is compiled into
every released binary — an index growing past it would have taken the
registry down for every client already in the wild, and no server-side
change could have lifted it.  A ceiling that is reachable has to
announce itself.

**Derived trigger sidecar** — `~/.loft/registry/triggers.json`.  The
parser consults the catalog's `method -> package` map whenever a bare
`line.matches(p)` names no declared dependency, i.e. *while compiling*.
That map is a few hundred bytes; reading it out of the index meant
parsing the whole published catalog on the compile path — at 100×
today's index, `loft --check` on a four-line program cost **1.05 s and
344 MB RSS**, all of it `api` rows the compiler never looks at.

The sidecar holds just the map, stamped with the index's byte length +
mtime.  A missing or mismatched stamp rebuilds it from the index and
writes it back, so the parse lands once per index rather than once per
compile; the registry commands that fetch an index refresh it from the
copy they already hold (`install::load_index_inner`).  Same program,
same 100× index, with the sidecar: **0.04 s and 11.7 MB** — the cost of
having no catalog at all.  It is derived data: any doubt rebuilds it,
nothing trusts it, and a read-only cache directory costs the fast path
and nothing else.

**Cache lifecycle — no automatic cleanup, conservative by design**:

- **Old versions stay** when newer versions install.  Lockfile
  reproducibility requires this: project A pins `crypto = 0.1.0`,
  project B pins `crypto = 0.2.0`, both coexist forever in
  `packages/crypto/`.  Auto-deleting on newer install would
  silently break A's offline rebuild.
- **Re-download is allowed**, not forced.  If `cache/<sha>.tar.gz`
  is deleted but `packages/<name>/<version>/` survives, the
  extracted tree is still usable; cache file is re-fetched only
  when needed for verification.

**Manual cleanup commands** (MVP scope marked):

| Command | Effect | Status |
|---|---|---|
| `loft cache list` | Show what's in `~/.loft/{packages,cache}/` with sizes | MVP |
| `loft cache clean` | Wipe `~/.loft/{registry,packages,cache}/` (nuclear) | MVP |
| `loft cache prune` | Drop entries not referenced by any `loft.lock` in `$PWD` (recursive scan) | v0.2 |
| `loft cache prune --scan <dir>` | Same with explicit roots | v0.2 |

Why prune lands later: lockfile-aware GC is easy to get wrong (a
lockfile in a stale clone shouldn't pin a tarball forever).  Ship
the nuclear option first; iterate on the conservative one once
real-world feedback is in.

**Disk cost rough estimate**:

A typical package = 30 kB tarball + ~150 kB extracted.  100
packages with ~3 versions each = ~50 MB total.  Negligible on a
modern machine; visible-but-fine on a constrained dev VM.

---

The consumer's `loft.lock` and manifest-less resolution (a bare script takes the latest release) are in [PKG_REGISTRY_RESOLUTION.md](PKG_REGISTRY_RESOLUTION.md).

---

## `loft install` flow

Driver: `loft install` (no args, in a directory with `loft.toml`)
OR `loft install <name>[@<version>]` (one-off install — adds to
`loft.toml`'s `[dependencies]` and installs; in a directory with no manifest it
writes a minimal one first, so the lock it writes has a root that governs it —
§ Manifest-less resolution, failure path 7).

### Pseudo-flow

```
1. Resolve dependency graph
   1.1 Read ./loft.toml — collect [dependencies]
   1.2 Read ./loft.lock if present — collect pinned versions
   1.3 Refresh index if stale (> 1h since last fetch or --refresh)
   1.4 For each dep:
       - If in loft.lock and the registry still has that version (not yanked),
         use the locked version
       - Else resolve highest non-yanked version satisfying the constraint
   1.5 Recurse into transitive deps from registry.json::deps
   1.6 Diamond resolution: pick highest version satisfying all
       constraints (cargo-style); fail if no version works

2. Plan downloads
   2.1 For each resolved (name, version):
       - If ~/.loft/registry/<name>-<version>/ exists AND its
         loft.toml matches registry data → already installed, skip
       - Else: queue download

3. Download + verify
   3.1 Parallel fetch tarballs (small pool, 4 at a time)
   3.2 For each: hash, compare to registry's sha256, fail loudly
       on mismatch
   3.3 Extract into a staging directory of this install's own, then rename
       it to ~/.loft/registry/<name>-<version>/ — the directory appears
       whole or not at all, so several processes installing one package
       at once (a server and its clients) never read one half-filled; a
       rename that finds the directory already placed counts the package
       as cached

4. Write loft.lock (and, in a directory with no loft.toml, the manifest that
   makes it govern)
   4.1 Atomic rename: write a scratch file named for the process
       (loft.lock.tmp<pid>-<n>), rename it to loft.lock
   4.2 Includes resolved versions, urls, sha256s, transitive deps

5. Print summary
   "Installed crypto 0.1.0, web 0.1.0 (2 packages)"
   "Resolved from registry.json (cached 14 min ago)"
```

### Error paths

| Condition | Behaviour |
|---|---|
| Registry URL unreachable | Use cached index if not stale; else fail with "registry unreachable, try `--offline` if you have a cache" |
| `--offline` flag | Never fetch; use cache; fail if a requested package isn't cached |
| Package not in registry | Fail with available alternatives (Levenshtein distance ≤ 2 on package names) |
| No version satisfies constraint | Print conflicting constraints with package + dep chain |
| sha256 mismatch | Delete partial download; fail; do NOT retry (could be a real attack) |
| `--frozen` + lock file would change | Fail with diff of intended changes |
| Loft version requirement not met | Fail with "package X requires loft Y, you have Z" |

### CLI surface

| Command | Behaviour |
|---|---|
| `loft install` | Honour loft.toml + loft.lock; install missing packages. |
| `loft install <name>` | Add `<name> = "^<latest>"` to loft.toml, install. |
| `loft install <name>@<version>` | Add `<name> = "<version>"` to loft.toml, install. |
| `loft install --refresh` | Force re-fetch the registry index. |
| `loft install --offline` | Use cache only; fail if anything's missing. |
| `loft install --frozen` | Fail if loft.lock would change.  CI mode. |
| `loft update` | Re-resolve all deps; rewrite loft.lock. |
| `loft update <name>` | Re-resolve only `<name>`. |
| `loft search <query>` | Client-side filter on index `description` + name. |
| `loft info <name>` | Print available versions, latest, homepage. |

`loft install` is idempotent — running it twice does nothing the
second time (cache hit, lock file unchanged).

---

## Publishing flow (MVP)

Manual PR-based.  Acceptable while loft maintainers are reviewing
every publish.

**Author-facing guide**: [REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md)
walks through the same flow step-by-step with troubleshooting
for common failure modes (sha256 mismatch, reproducible-build
mismatch).  The section below is the design/reference
description; that doc is what you'd hand to a contributor.

### Author side

1. Tag the release in the per-library repo (e.g.,
   `git tag <name>-v0.1.0 && git push --tags`).
2. Run `loft package` (new CLI command — minimal scope):
   - Reads `loft.toml` for name + version.
   - Builds the tarball from the package layout (excludes per
     [§ Tarball format](#tarball-format)).
   - Computes SHA-256 + byte size.
   - Prints:
     ```
     <pkg>-0.1.0.tar.gz       (size: 12.4 kB)
     sha256: abc123…
     ```
3. Attach the tarball to a GitHub release for the tag (`gh release
   create v0.1.0 <pkg>-0.1.0.tar.gz` — or a CI workflow does this
   automatically).
4. Open a PR against `loft-lang/registry` adding the version entry:

   ```diff
    "crypto": {
      "versions": {
   +    "0.2.1": {
   +      "url": "https://github.com/loft-lang/loft-libs-core/releases/download/crypto-v0.2.1/crypto-0.2.1.tar.gz",
   +      "sha256": "abc123…",
   +      "size": 12698,
   +      "loft": ">=0.8",
   +      "subpath": "crypto",
   +      "published": "2026-05-24T08:00:00Z"
   +    }
      }
    }
   ```

### Maintainer side

1. CI validates the PR:
   - `schema_version` unchanged.
   - New entries follow the schema (script-lints required fields).
   - `url` is HTTP-accessible (HEAD request succeeds).
   - `sha256` matches the actual file at `url` (downloads + hashes;
     <30s per published tarball at typical sizes).
2. If validation passes, merge.
3. GitHub Pages (or raw.githubusercontent.com) serves the updated
   `registry.json` immediately.

The maintainer pipeline is just GitHub Actions + branch protection
+ a small Python validator script.  No custom service.

### Yanking

PR adds the version to the package's `yanked` array and **leaves its
`versions` entry exactly where it is**.  Yanking discourages a version;
it never withdraws one.  A yanked version stays listed and stays
downloadable, so a `loft.lock` pinned to it still resolves, while new
installs and version resolution skip it.

**Never delete a `versions` entry.**  Deleting is not a stronger yank,
it is an unrecoverable one: `web 0.2.2` was yanked correctly and then
had its entry removed a commit later, and by the time anyone noticed,
its release asset and tag were gone too — leaving a `yanked` marker
that implies a version which no longer exists anywhere.  An earlier
revision of this section said to remove the entry, contradicting the
promise in the sentence right after it; that wording is how the loss
happened.  `scripts/registry_retention_check.py` now fails the nightly
if any version leaves the index or stops downloading.

**Signing a yank — `--expect-yank`.**  A yank adds no version, so the
`--expect <pkg>@<ver>` that binds an ordinary publish cannot describe
one: it refuses for *"asked for but ABSENT from the diff"* while the
`yanked` edit itself reads as untouchable package-metadata drift.  That
left `--yes` as the only route, and `--yes` asserts nothing about what
is signed — the exact gap the bound form exists to close.  So the
signer takes a second bound form:

```bash
scripts/registry-sign.sh --registry-dir <checkout> --expect-yank imaging@0.1.0 \
  --message 'yank imaging 0.1.0 — <why>'
```

It refuses unless the diff adds exactly the named versions to those
packages' `yanked` arrays: no version added or changed, nothing
un-yanked, no other field of any package touched, and every named
version still present in `versions` (a yank of an unlisted version
would sign a marker pointing at nothing — the `web 0.2.2` shape above).
The two forms combine for a publish that also yanks.

**Correcting metadata — `--expect-meta`.**  The same gap, a third
time, for the write that fixes a description, homepage or category.  It
adds no version and touches no `yanked` array, so `--expect` refuses it
as *"ABSENT from the diff"* while the correction itself reads as
untouchable drift — and `--yes` is refused outright without a terminal.
Every route was closed, which left bundling the fix into an unrelated
publish as the only path, and that is precisely what the scope check
exists to prevent:

```bash
scripts/registry-sign.sh --registry-dir <checkout> --expect-meta hex_grid \
  --message 'hex_grid: the index described the wrong coordinate system'
```

It refuses unless exactly the named packages' metadata changed: no
version added or removed anywhere, no `yanked` array touched **on the
named package either** (metadata only means metadata — otherwise the
flag becomes a way to smuggle a version past the check), no other
package altered, and each named package must ACTUALLY have changed — a
name that matches nothing is a claim about a diff that is not there.
The run prints the old and new value of every changed field, because
sections 2 and 3 of its output have no tarball and no release to show
and would otherwise render as a signature over an empty diff.

**Correcting a field INSIDE a published version — plain `--expect`, and
the one write it refuses.**  `--expect-meta` is package-level; a
version's own fields (`deps`, `api`) are not metadata in its sense.
There is no fourth flag, because `--expect <pkg>@<ver>` already matches
a **CHANGED** version as well as a new one:

```bash
scripts/registry-sign.sh --registry-dir <checkout> \
  --expect hex_terrain@0.1.0 --expect hex_terrain@0.1.1 \
  --message 'hex_terrain 0.1.0/0.1.1: the entries omitted the hex_grid dep'
```

This is the route `registry_maintain.sh` names when a publish carries no
`deps` — *"Add them to `[dependencies]` or to the previous entry's
`deps` first"* — and it is the one that matters for a multi-package
repo, which deliberately keeps registry deps OUT of `loft.toml` so
`--lib` consumption keeps working (loft#1352: a declared dependency is
searched BEFORE the `--lib` flag).  The fold then carries the corrected
`deps` forward to the next version automatically.

⚠ **But a published version's BYTES may not move.**  `url`, `sha256`
and `size` are *which bytes* a version is, and a lock file names the
version, not the bytes — so changing them re-points something already
installed.  "Published releases are immutable" (REGISTRY_SUBMIT.md) was
the rule with no enforcement: `--expect` read a tarball swap on a
published version exactly like a `deps` fix — the same scope line, the
same *"nothing else added, removed or altered"* — and the download check
confirms only that the new sha matches the new url, never that either
still matches what consumers already resolved.  The signer now refuses
it and says to publish a NEW version instead.

Two smaller consequences of the same reading.  A CHANGED version now
reports **which fields moved, was → now**, because the entry block
prints the post-state and a correction and a swap render identically in
it; the reviewer is the trust root, so the delta has to be visible, not
merely permitted.  And `--expect-meta`'s refusal for a per-version field
now NAMES `--expect <pkg>@<ver>`: that refusal used to end the trail,
which reads as *"the signer has no bound form for this"* when one exists
a flag away.  Guarded by `tests/registry_sign_scope.rs`, which lifts the
review block out of the script rather than restating its rules, and
asserts the ordinary new-version publish stays untouched — a refusal
that also blocked the everyday route would be switched off, not fixed.

Measured: `hex_grid`'s index entry called a pointy-top **odd-r offset**
package *"axial"* for twelve days after its manifest was corrected
(loft-libs-world `8e9c93d`), because nothing had published that package
since — and axial-versus-offset is the exact confusion its coordinates
are most often got wrong on, silently, since both spellings are
`(integer, integer)`.  All three forms combine.

**What a yank does and does not change.**  Measured on `imaging` 0.1.0
(loft#1448): a range or `*` skips it (`>=0.1` resolves 0.3.2, as it did
before), an **exact** pin still resolves it — that is the retention
promise `find_best_version` keeps for a `loft.lock` — and a constraint
whose only matches are yanked now FAILS where it used to install an
unbuildable release.  That last case is the one the yank exists for, and
its message names the yank (`available: 0.1.0 (yanked), …  — every
version that matches is yanked`): listing the keys bare said "0.1.0 is
available" in the same breath as refusing a constraint 0.1.0 satisfies,
which reads as a resolver bug rather than as a maintainer's decision.

---

Index signing, reproducible-build verification and nightly toolchain validation are in [PKG_REGISTRY_TRUST.md](PKG_REGISTRY_TRUST.md).

---

## Migration to a real server (later)

**Hard constraint: the user-visible behaviour must NOT change.** See
[§ The invariant](#the-invariant--end-user-experience-is-identical-to-a-real-server)
above.  Migration is purely an infrastructure swap.

Two paths, both compatible with the MVP's URL surface:

### Path 1 — Server backs the same JSON file

The server reads from a database, serves `GET /registry.json` with
the same shape.  Existing loft clients in the wild keep working
without recompilation or config change.  New features (search
endpoint, publish API, signing) layer on top:

| Endpoint | Purpose |
|---|---|
| `GET /registry.json` | Existing.  All clients hit this. |
| `GET /index.json.sig` | Existing.  Ed25519 signature; clients verify with embedded public key.  Server signs on every update with the same maintainer key the MVP used. |
| `GET /v1/search?q=` | New.  Server-side index, faster than client-grep at scale. |
| `POST /v1/publish` | New.  Replaces the manual PR; auth via token.  Server still produces a signed `index.json` after each publish; signing key lives in the server's HSM. |
| `POST /v1/yank` | New.  Replaces yank PRs. |
| `GET /v1/packages/<name>` | New.  Single-package metadata (skip the full index). |
| `GET /v1/attestations/<pkg>-<v>` | Future.  Per-tarball publisher attestations (sigstore-style) — finer-grained than the single trust-root signing the MVP ships. |

### Path 2 — Stay file-based, add tooling

Some ecosystems (Homebrew, AUR) are file-based forever.  If loft's
ecosystem stays small (~100 packages), the MVP's PR-based publish
flow may never need replacing — just add tooling to automate the
PR process (a GitHub App that auto-opens a PR when a release tag
pushes).

**Decision deferred.**  Path 1 vs Path 2 is a 1.x decision driven
by actual ecosystem growth.  The MVP commits to neither.

---

## What this does NOT cover

Out of scope for the file-based MVP — captured here so future
contributors don't redesign each on the spot.

1. **Auth on publish.** Acceptable while maintainers review every
   PR.  Token-based auth lives in Path 1's server.
2. **Package namespaces (e.g. `@user/pkg`)**.  Defer until naming
   conflicts emerge.
3. **Cargo-style features** (`[features]` per dependency).  Loft
   packages don't have feature flags today.
4. **Pre-built native binaries** (PACKAGES.md Open Q #14).
   Tarballs ship loft + Rust source; the consumer's machine builds
   the cdylib via the same `loft-ffi-build`-driven build.rs as the
   monorepo libraries.  Pre-built distribution is a future
   acceleration.
5. **Multi-registry support** (alternative registries / private
   mirrors).  Single registry URL hardcoded in the loft binary.
   Multi-registry is Path 1's job.
6. ~~**Signing (Ed25519 / sigstore).** SHA-256 hash + HTTPS download
   covers integrity for the MVP.  Cryptographic signing is a Path 1
   feature.~~ **REVISED 2026-05-24** — index signing IS in the MVP
   via R3.5 (Ed25519 over `index.json`, single trust-root key).
   Per-tarball signing (sigstore / cosign / per-publisher keys) is
   still a Path 1 feature; the file-based MVP's trust model is "the
   index is signed by loft maintainers; that index attests to every
   tarball's sha256."

---

## Open questions

These need decisions before implementation starts.

1. **Registry URL.**  `loft-lang.org/registry.json` (DNS-controlled,
   loft-lang owned) vs `raw.githubusercontent.com/loft-lang/registry/main/index.json`
   (zero infra, fully GitHub-backed).  Recommendation: start with
   the raw GitHub URL (zero cost, zero ops); add the DNS alias when
   ecosystem maturity justifies it.  Either way the URL is
   overridable via `LOFT_REGISTRY_URL` env var to enable mirrors,
   private registries, and per-CI pinned snapshots.
2. **Loft-version requirements.**  Should the registry index
   include the loft version requirement, or read it from the
   tarball's `loft.toml` post-extract?  Recommendation: include it
   in the index so the client can fail BEFORE downloading an
   incompatible version.
3. **Index cache TTL.**  1 hour as default — too long? too short?
   The registry is small + cheap to refetch; 1 hour seems fine.
   Settable via env var (`LOFT_REGISTRY_TTL=300` for 5 min).
4. **Native package distribution.**  Today the tarball ships the
   Rust `native/` source; the consumer builds.  Future: distribute
   pre-built cdylibs per platform via the same release.  Out of MVP
   scope.
5. **First-class deps.**  Should the index's `deps` field carry
   version constraints, or just package names?  Cargo carries
   constraints (`crypto = ">=0.1"`); loft should mirror for parity.

---

The `loft search` design (registry discovery, R8+) and its open work are in [PKG_REGISTRY_SEARCH.md](PKG_REGISTRY_SEARCH.md).

---

## See also

- [PACKAGES.md](PACKAGES.md) — package format reference; this doc
  is the registry-specific draft of that doc's "Open work" PKG.REG
  bullet.
- [lib_plans/12-library-extraction/](lib_plans/12-library-extraction) —
  consumer of PKG.REG.  Phases 4-8 unblock when PKG.REG ships.
- [STDLIB.md § Logging](STDLIB.md) — `loft install` should log via
  the same machinery; useful for `--verbose` output.
