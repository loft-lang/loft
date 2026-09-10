<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# PKG.REG — file-based registry MVP

Draft 2026-05-24.  This is the design for the **MVP** of the loft
package registry.  Goal: ship `loft install <name>` against a static
file before designing a full server.  The same URL surface migrates
to a server later with no client-side changes.

This doc fleshes out the "(b) central registry server (GitHub Pages +
static index acceptable for MVP)" sub-phase from
[PACKAGES.md § Open work](PACKAGES.md#open-work).

**Revision 3 (2026-05-24)** — switched index signing from CI-based
(GitHub Actions + `REGISTRY_SIGNING_KEY_BASE64` secret) to
**maintainer-laptop-based**.  Same crypto (Ed25519); private key
now never leaves hardware the maintainer physically controls.
`loft-keygen sign` / `loft-keygen verify` subcommands added.
Removed `sign-and-commit.yml` + `sign-index.py` from the CI
template directory.  Rationale in [§ Why laptop signing, not CI](#why-laptop-signing-not-ci).

**Revision 2 (2026-05-24)** — Debian-comparison pass added.
Promoted index signing from "Path 1 server feature" to "MVP R3.5"
(real security gap closed).  Added schema slots for `conflicts` /
`replaces` / `provides` / `binaries` / `prerelease` / `categories`
(Debian-inspired, reserved fields, resolver-side support deferred
to keep MVP scope tight).  Explicit "not adopted" table records
items deliberately rejected (pre/postinst scripts, debconf,
alternatives, triggers, epoch versioning, `main`/`contrib`/`non-free`)
so future PRs aren't re-litigated.  See
[§ Comparison to Debian](#comparison-to-debian--what-we-adopted-what-we-didnt).

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
| `loft.lock` format | TOML, see [§ Consumer's lock file](#consumers-lock-file--loftlock). | Same. |
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
[QUALITY.md § P54](QUALITY.md))
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

## Consumer's lock file — `loft.lock`

The consumer's project has a `loft.toml` declaring deps:

```toml
[package]
name = "my_app"
version = "0.1.0"

[dependencies]
crypto = ">=0.1"
web = "^0.1.0"
```

`loft install` resolves these against the registry and writes
`loft.lock` alongside `loft.toml`:

```toml
# Auto-generated by loft — do not edit by hand.
# Run `loft install` to update.
schema_version = 1

[[package]]
name = "crypto"
version = "0.1.0"
url = "https://github.com/loft-lang/loft-crypto/releases/download/v0.1.0/crypto-0.1.0.tar.gz"
sha256 = "abc123…"
source = "registry"

[[package]]
name = "web"
version = "0.1.0"
url = "…"
sha256 = "…"
source = "registry"
deps = ["crypto"]
```

Subsequent `loft` runs honour `loft.lock` exactly — same `sha256`
required.  Reproducible across machines / CI / collaborators.

`loft update [<pkg>]` re-resolves (one package, or all) and rewrites
the lock file.  CI uses `loft install --frozen` which refuses to
rewrite the lock and fails if it would.

Its work list is the **union of `loft.toml`'s declared (non-path)
dependencies and `loft.lock`'s entries** — never the lock alone.  The lock is
meant to describe the manifest, so a dependency the manifest gained since the
lock was written is the main thing this command exists to fix; resolving only
what the lock already names made such a package invisible, and the summary
counted lock entries, so the omission reported itself as `all N packages
up-to-date` (loft#830).  A declared package with no lock entry is resolved and
added; one that no index can resolve is **named**, never silently skipped, and
turns `loft update --check` red — that check asks whether the lock matches the
manifest, and there the answer is no.  With no lockfile at all but declared
dependencies, `loft update` builds the lock rather than reporting nothing to do.

The decision itself is `lockfile::update_worklist` — one pure function, so the
"which packages" question has one home and is unit-testable apart from the
network.

`source = "registry"` is recorded so future sources
(`source = "git"`, `source = "path"` for local overrides) can
coexist.

---

## Manifest-less resolution — a bare script takes the latest release

**Status: SHIPPED (@PLN143).** A script with no `loft.toml` is loft's `python
script.py` case: one file, a `use`, run it. It means *"the newest release of that
library"*, every run, with no ceremony and no files left behind.

### The invariant

> **Nothing a program produces by RUNNING may change which version a later run
> resolves.** A version is fixed only by an explicit act — `loft install`,
> `loft update`, `loft pin` — and that act writes exactly one declaration, beside
> the thing it governs. Where no declaration is in force, `use <pkg>` means *the
> newest release*, re-decided on every run.

The invariant is about **write**, not about state: the extracted-package cache
under `~/.loft/registry/` is written by running and that is fine, because it can
only make a resolution *faster*, never *different*. A lockfile is the opposite —
it is the declaration itself, so producing one as a side effect of a run makes a
program's meaning depend on its own history.

That is what decides every case, including the untested ones: **read the
declaration that governs this PROGRAM; never write one on its behalf.**

### Why "latest" is the right default, not a convenience

Because a published library that breaks a working consumer is a **bug**, not a
version bump — [COMPATIBILITY.md](COMPATIBILITY.md) covers `use`d libraries by
name, and evolution is additive. Under that promise, pinning is for reproducing a
*build*, not for hiding from breakage; there is nothing to hide from. A default
that pins would be treating the library ecosystem as untrustworthy, which is the
opposite of the commitment the project makes about it.

Two honest qualifications, and they shape the design rather than defeat it:

- **The promise binds at contract 1, which is not reached yet.** Until then a
  library genuinely can regress. The mitigation is that loft is *statically
  typed and compiled against*: a removed or re-signatured `pub fn` is a compile
  error naming the call site, not a wrong answer at run time. The failure mode
  of "latest" is therefore loud, and the escape is one command (`loft pin`).
- **A data break is silent where an API break is loud.** `data_compatible_with`
  in the index exists for exactly this, and it is why **a program that reads
  *stored* data is the one shape that should carry a declaration** — write a
  `loft.toml` (or `loft pin` the script) rather than relying on the default.

### The three scopes

Which declaration governs is a property of the program, so it is answered once —
`resolution_scope(script)` in [`src/resolution_scope.rs`](../../src/resolution_scope.rs):

| Scope | Detected by | Governs |
|---|---|---|
| **Pinned script** | `<script>.loft.lock` beside the script | that sidecar |
| **Package** | nearest ancestor `loft.toml` from the script | that root's `loft.lock` + the manifest's constraints |
| **Bare** | neither | nothing — newest release, every run |

A sidecar outranks an enclosing package: it is the declaration nearest the thing
it governs, and `loft pin <script>` wrote it for that script.

**The cwd plays no part.** It used to, and that was the defect: the same script
run from two directories resolved two different ways, from a file an earlier run
had written.

### Failure paths — what has to keep working

Enumerated first, because this is where the design earns its shape:

1. **Offline, bare script, package already in the cache.** Resolves to the
   newest version already extracted under `~/.loft/registry/` that this loft can
   load — skipping prereleases and any copy whose `loft` / `contract`
   requirement this build does not satisfy (asked of the same
   `manifest::check_version` the loader asks, so the filter cannot drift from
   what the loader accepts).
2. **Offline, bare script, nothing cached.** Fails with the standard "library
   not found" diagnostic.
3. **Steady state is silent.** A run that finds the package extracted prints
   nothing; `[registry] downloading <pkg> <version>` speaks where bytes are
   actually fetched ([GOALS.md](GOALS.md) — loft is noticed in its absence).
4. **Index staleness is bounded, not fatal.** The 1-hour TTL plus conditional
   GET; when the refresh fails the run continues and the cache answers.
   ⚠ **Right after a publish it reads as the publish having failed.** Two lags
   compound — the local TTL, and GitHub raw's CDN, which serves the pre-push
   index to some edges for a few minutes.  Measured 2026-08-21: `loft install`
   answered *"package `tween` not found in registry"* while `curl` on the SAME
   url returned the index containing it, and the cache held commit `205f839`
   byte for byte — two registry commits behind.  **Verify a publish with `loft
   install --refresh`**, and read a "not found" straight after a publish as a
   cache question before a publish one.  (`registry_maintain.sh`'s own coverage
   check already reads the LOCAL index copy for exactly this reason.)
5. **A transitive dep of a bare `use`** is resolved by the library's own manifest
   constraint, unchanged. "Latest" applies to what the *program* named, not to
   what its libraries pinned.
6. **`loft install` / `loft update` / `loft pin` still write.** They are the
   explicit acts; the invariant restricts running, not the verbs whose job is to
   declare.
7. **`loft install <pkg>@<version>` in a directory that is not a package** writes
   a minimal `loft.toml` beside the lock and says so (`created loft.toml (package
   \`x\`)`). That makes the directory a package, the walk-up finds it, and the
   scope table decides the rest — no special case. Without it the pin would be a
   file nothing reads. `loft install` with no argument in such a directory keeps
   refusing; there is nothing to install.

### A pin decides the install, not only the load

The governing lock answers two questions, and they used to have different
answers: *which file do I load* (it did) and *which version must be installed
when that file is not in the cache yet* (it did not — the install resolved
"newest satisfying the manifest", so a fresh box quietly ran a different version
than the machine that pinned it). A lock entry is an EXACT version, so it is now
the constraint the install runs under.

The one case where it is not is a pin the manifest has since EXCLUDED — someone
edited `^0.1` to `^0.2` and has not re-installed. A lockfile is the resolved form
of the declaration above it, so it cannot outrank the declaration it derives
from; the manifest wins and the lock is re-resolved.

That split is why `InstallOptions` reads `lock_path` (the lock that GOVERNS) and
`skip_lockfile` (whether this resolution may write it) as separate questions.
Reading them as one is what made a pinned script's sidecar decide the load and
nothing else — and it means the sidecar now gets the re-publish check too, since
a lockfile pins the bytes as well as the version.

### The cache fallback is `Bare` scope only

A fallback picks the newest cached version, and only where nothing is declared is
there no constraint it could be violating. A package's manifest may say `^0.1`
and a pinned script names an exact version, so a scope that HAS a declaration
fails instead of answering past it.

### When a pin has fallen behind

A `loft.lock` has no expiry, so a pin holds forever and nothing about the
program's own code says why — and the payoff of a release is not only API:
`cbor` 0.1.3 turned canonical map encode from O(n³) to O(n²) with no API change
at all, so a consumer pinned at 0.1.2 keeps hanging and every gate it has stays
green. Where a declaration IS in force and the **cached** index knows a newer
release, the build says so once:

```
[registry] cbor 0.1.2 is pinned; 0.1.3 is the newest release — run: loft install cbor
```

Its silences are the design: no fetch is ever added (cache only — no cache, no
line); once per package per run, not once per parse pass; never for a
dependency's own `use` (latest is about what the *program* named); silent under
`LOFT_OFFLINE`; and silent where nothing is pinned, because a bare script
re-decides every run and cannot be behind. A pinned script is told to
`loft pin <script>`, a package to `loft install <pkg>`. Off-switch:
`LOFT_NO_UPGRADE_NOTICE=1`.

It costs one parse of the cached index per run — measured at **10–20 ms** on a
692 KB index (0.06–0.08 s → 0.08–0.12 s for a one-line script), paid only where a
lock actually governs a package the program named. That is the honest price of
the notice; the off-switch is the way out of it.

Trust posture is the read-only one (`loft search` / `loft info`): a missing
signature is tolerated, an invalid one degrades to silence. Nothing is installed
from those bytes, and the cure it prints goes through the verifying path — where
a missing signature is refused (below).

### Where it stands (measured 2026-08-18, `probepkg` 0.1.0 + 0.2.0 cached)

| Probe | Before @PLN143 | Now |
|---|---|---|
| Bare script, first run | newest ✅ — and writes `./loft.lock` ❌ | newest ✅, writes nothing ✅ |
| Same script, second run | reads that lock — pinned forever ❌ | newest, re-decided ✅ |
| Same script, different cwd | re-resolves + drops a second lock ❌ | same answer, no file ✅ |
| Bare script, offline, versions extracted | fails to resolve ❌ | newest loadable cached ✅ |
| Package / pinned script | its lock governs ✅ | unchanged ✅ |
| A governing pin behind the registry | silent ❌ | one line, with the cure ✅ |
| Cost of re-resolving each run (cache warm) | ~10 ms ✅ | ~20 ms — 0.07 s pinned vs 0.09 s bare, one index parse per run ✅ |

The trade this makes is not speed — it is that two machines running the same bare
script on different days can get different versions, and the answer to that is
`loft pin <script>`, which already ships.

### Residuals, recorded

- **A package-scope auto-install still records into the project's `loft.lock`.**
  That write is the manifest's own lock, beside the declaration that governs it,
  and `loft install` is the verb that normally writes it — but it *is* a run
  producing a declaration, so the invariant holds absolutely only in `Bare` and
  pinned-script scope.
- **In a declared scope, an unsatisfiable offline resolve still reports "library
  not found"** rather than naming the constraint it could not satisfy. The
  answer is right; the message is thin.
- **A transitive dependency is still resolved by the root manifest's range, not
  by the lock that names it.** A package parsed out of the registry cache is
  bound by neither lock (the only declaration above it is the cached
  dependency's own), so a lock entry for a package nothing `use`s directly is a
  record rather than a constraint.

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
   3.3 Extract to ~/.loft/registry/<name>-<version>/

4. Write loft.lock (and, in a directory with no loft.toml, the manifest that
   makes it govern)
   4.1 Atomic rename: write loft.lock.tmp, rename to loft.lock
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
   `git tag v0.1.0 && git push --tags`).
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

## Index signing — `index.json.sig`

Closes a real security gap in the file-based design.  Without
signing, the chain of trust is HTTPS → `index.json` → tarball
sha256 — but **nothing protects the index itself**.  An attacker
who compromises the registry repo or MITM's the HTTPS connection
can serve a modified `index.json` pointing at malicious tarballs
with matching (recomputed) sha256s.

Borrowed from Debian's `Release.gpg` / `InRelease`.

### How it works

1. Loft maintainers hold a single trust-root signing key (Ed25519,
   stored on a trusted laptop + hardware-token backups, NOT in any
   service's secret manager).
2. The loft binary embeds the **public key** at compile time
   (`src/registry_keys.rs`, ~50 bytes).  Multiple keys can be
   embedded for rotation or for multi-maintainer setups; clients
   accept signatures from any listed key.
3. **Signing happens locally on the maintainer's laptop**, NOT in
   CI, via `scripts/registry-sign.sh` — DEFAULT is **on-card**: a
   YubiKey holds the Ed25519 key non-extractably (PIV slot 9C) and
   signs over PKCS#11 (`pkcs11-tool --mechanism EDDSA`); the private
   key never leaves the card, and PIN + touch are the confirmation.
   If no card is present, signing **falls back to a local key file**
   (`~/.loft/trust-root/registry-signing-key.bin`) behind a typed
   `yes` prompt — or, for a scripted or agent-driven publish, behind
   **`--expect <pkg>@<ver>`** (see § The third route below). Either way:
   - **Schema gate first**: `scripts/registry_schema_gate.sh` runs
     `tools/validate.py`'s gate 1 out of the checkout being signed, and
     `registry-sign.sh` refuses on a rejection.  It IMPORTS the deployed
     validator rather than restating its rules, so it inherits every
     future tightening and cannot drift from the check `pr-validate.yml`
     applies.  An ABSENT validator refuses rather than skips — a gate that
     skips looks exactly like one that passes, and this one stands in
     front of the signing key.
     ⚠ **This is the chokepoint, and the reason it is here rather than at
     publish time:** an index that fails gate 1 reaches `main` only through
     a signature, and once there it reddens EVERY later submission PR on a
     check that has nothing to do with that submission — which the person
     seeing it cannot clear, because clearing it needs this key.  Measured:
     `zttext` and `fixstep` went in with `"categories": []` (see the
     `categories` row in § Schema) and blocked an unrelated submission for
     days.
   - Maintainer reviews the diff `registry-sign.sh` prints (raw
     `index.json` diff + each changed release's provenance +
     re-downloaded tarball sha256) before confirming.
   - The script signs (on-card, or `loft-keygen sign --in index.json
     --key <private>.bin --out index.json.sig` for the local-key
     path) to produce a raw 64-byte Ed25519 signature.
   - **Trust gate**: the new signature must verify against a key
     already listed in `src/registry_keys.rs::TRUSTED_PUBLIC_KEYS`
     — if it doesn't, `registry-sign.sh` refuses to commit or push
     (a wrong/untrusted key would otherwise ship a signature every
     `loft install` rejects).
   - Maintainer commits `index.json` + `index.json.sig` **together**
     (so HEAD's index always matches its signature) and pushes.
4. Clients fetch both files.  Verify the signature over the bytes
   of `index.json`.  Refuse to use an unsigned/invalid-sig index
   unless `--allow-unsigned` is passed.

### The third route — `--expect`, a confirmation that CHECKS

A typed `yes` asserts that a human looked.  It verifies nothing, and
the failure that matters is not "nobody looked" but "somebody looked
and the diff carried something they did not read": one real run
published four packages and ALSO rewrote four unrelated descriptions,
losing `ssh`'s **"Native-only"** — the one fact a consumer needs before
choosing it.  Every gate stayed green, because none of them reads a
description (§ The first-`description` gotcha in the publish runbook).

`scripts/registry-sign.sh --expect drawing@0.1.0` (repeatable) binds the
signature to the versions the maintainer NAMED.  The run refuses unless
the diff, against its base:

- introduces **exactly** the named `<pkg>@<ver>` set — nothing more;
- **removes** no package and no version;
- leaves every other package's `description` / `homepage` /
  `categories` / `yanked` **byte-identical**.

On a match it signs with no prompt; on any mismatch it exits non-zero
and names which of the four rules broke.  It requires a real diff base
(`--since`, or the auto-picked one) and refuses without one — a claim
about what CHANGED is meaningless with nothing to have changed from.

This is **stricter than the prompt it replaces**, which is what makes
it an acceptable substitute rather than a bypass: it answers the exact
concern § Why laptop signing states below — "CI signing would sign
whatever lands in `main` — no human gate" — by making *whatever lands*
impossible to sign, while the key still never leaves the maintainer's
machine.  `--yes` remains the unchecked escape hatch and should not be
reached for when `--expect` will do.

Proven able to refuse, on the live index: an unasked-for version, a
rewritten description, a deleted package and a wrong version number
each turn the run red, and the named version alone signs.

`--yes` additionally **refuses when stdin is not a terminal**.  It means
"a human decided and is not here to type it", and the second half is
only true at a terminal — with no TTY the flag asserts a human who
cannot be present, which is exactly the shape a script has.  That is
also what lets the signer be allow-listed in a permission config
without the allow-list widening anything: a permission rule cannot tell
`--expect drawing@0.1.0` from `--yes`, and the script can.

**For a batch, `registry_maintain.sh --only <pkg>[,<pkg>]`** does the
whole publish — package, tag, release, index entry — for exactly the
named libraries, and hands the signer an `--expect` for each one it
actually published.  The filter is applied before the pre-flight (which
runs each worklist lib's suite, so filtering afterwards would spend
minutes on libraries nobody asked for), foreign PRs and staged
submissions are reported but **not acted on** — merging one would put a
version in the index the signer then refuses, *after* the merge landed
on the remote — and a name with nothing to publish is an error rather
than a quiet no-op.  So four library versions cost one command and no
prompt, with the signature bound to all four.

### Why laptop signing, not CI

Two reasons the maintainer signs locally instead of in GitHub
Actions:

- **No third-party trust dependency.**  CI signing would require
  storing the private key as a `REGISTRY_SIGNING_KEY_BASE64`
  GitHub secret — adding GitHub's secret-management to the trust
  chain.  Local signing keeps the key on hardware the maintainer
  physically controls.
- **Human in the loop.**  A maintainer reviewing + signing the
  merged commit IS the audit trail.  CI signing would sign
  whatever lands in `main` — no human gate.

Cost: ~30 seconds per merge.  For an early-stage ecosystem with
weekly publishes that's negligible.  When the ecosystem
outgrows laptop signing, the architecture migrates to Path 1
(real server) and signing follows.

### Two-stage bootstrap — interim `K_tmp` → permanent `K_real`

The registry MVP needs a single Ed25519 secret to sign
`index.json`.  Where that secret *lives* is independent of the
registry going live.  Running the full publish → install →
verification flow against a throwaway key (`K_tmp`) before the
permanent hardware-backed key (`K_real`) is ready is a deliberate
pattern, not a shortcut:

- **Interim (`K_tmp`)**: generate on the trusted dev laptop, no
  hardware backup, no off-site copies.  Use it to validate the
  full pipeline (loft-lang/registry online, first PR + sign +
  merge cycle, `loft install` against the live URL).  **Do not
  ship a public loft release with `K_tmp` embedded** — only the
  maintainer's local loft trusts it; blast radius is one
  machine.
- **Final (`K_real`)**: full 3-2-1 storage per
  [REGISTRY_BOOTSTRAP.md § Step 1.5](REGISTRY_BOOTSTRAP.md#step-15--store-the-private-key-for-the-long-haul).
  First public loft release embeds `K_real` in
  `TRUSTED_PUBLIC_KEYS`.

**Going from interim → final** is mechanically identical to the
"Compromised key" path: generate `K_real`, embed it in
`TRUSTED_PUBLIC_KEYS` removing `K_tmp`, re-sign every signed
index/asset with `K_real`, ship the public loft release.  The
recovery runbook for this is
[REGISTRY_RECOVERY.md § Scenario C](REGISTRY_RECOVERY.md#scenario-c).

Running this transition as a *planned* rotation has two
benefits:

- The end-to-end registry mechanic gets validated against a
  realistic load (real GitHub repo, real release assets, real
  install) before the trust root is consequential.
- The recovery procedure that's currently a runbook gets a real
  dry-run before you need it in anger.

### Multi-maintainer support

`TRUSTED_PUBLIC_KEYS` is `&[[u8; 32]]` — a slice.  Adding a
co-maintainer's public key alongside the primary key is a
one-line edit + a loft minor release.  Each maintainer signs on
their own laptop with their own private key.  Clients verify
against any embedded key.  No shared secrets; no privileged
"signing role" to compromise; bus-factor mitigated by having
multiple keys live in parallel.

For the bootstrap, one key is fine — the multi-key path is the
extension point when a co-maintainer joins.

### Key rotation

Add the new public key to a freshly-tagged loft binary release.
Sign the index with BOTH old + new keys for a transition window
(say, 3 months).  Once enough of the ecosystem has upgraded, drop
old-key signatures.  Compromised key: bump loft minor, embed only
the new key, distrust the old key via `~/.loft/distrusted_keys`
override.

### Why Ed25519, not GPG

GPG is a deep rabbit hole (web of trust, key servers, expiry,
revocation).  Ed25519 is one signature, one public key, ~120
lines of `ed25519-dalek` to verify.  Same security guarantee for
the threat model loft cares about (integrity of the index from a
single trusted root).

### Implementation phases — slotted into the R-list

* **R3.5** (between R3 bootstrap and R4 install) — add the
  `loft-keygen sign` + `loft-keygen verify` subcommands; embed
  the public key in the loft binary's `TRUSTED_PUBLIC_KEYS`.
  Required for R4 to verify on install.
* **R10.5** (after R10 first publish) — first real key rotation
  drill to validate the rotation path before key compromise
  happens for real.

---

## Reproducible-build verification at the registry

Borrowed from Debian's reproducible-builds initiative.  R1's
`loft package` already produces deterministic tarballs (same
source dir → same sha256 across runs).  Promote this from
"nice property" to "registry-side enforcement":

* When a PR adds a new version entry to `index.json`:
  1. The CI workflow checks out the package's tagged source
     (`git clone --depth=1 --branch v<version> <homepage>`).
  2. Runs `loft package` on the checkout.
  3. Compares the produced sha256 to the PR's claimed sha256.
  4. Rejects the PR on mismatch.

This catches:
- Honest mistakes (publisher manually edited the tarball,
  forgot to repackage).
- Malicious publishers (PR claims hash X but the published
  GitHub-release tarball has hash Y — only the consumer
  hash-check catches this today; the CI gate catches it at
  PR time).

Caveat: doesn't catch supply-chain attacks on the upstream
source itself (compromised git tags, etc.).  That's a different
trust problem (sigstore / cosign / `attestations.json`); out of
MVP scope.  Reproducible-build verification is the cheap layer
that handles the common honest-mistake / opportunistic-attack
class.

Lives in `R9` (registry-PR validator) — schema-wise no change;
it's a CI workflow addition.

---

## Nightly toolchain validation — every published package vs loft@main

The chunk repos' `library-ci` gates their own pushes, and the registry's
`pr-validate` gates schema + sha256 — but nothing re-checks a released
tarball after loft moves.  `registry-validation.yml` (in the loft repo,
04:30 UTC nightly + `workflow_dispatch`) closes that gap: one matrix leg
per non-yanked package, each running `scripts/registry_validate.sh <pkg>`
against loft built from main and the runner's current stable rustc.

The script validates the PUBLISHED artifact, exactly as a user gets it:
`loft install` (index fetch + tarball + sha256 + dep resolution), a
`cargo build --release` of the shipped `native/` crate if present, then
the package's own test suite on BOTH backends (`--interpret` and
`--native`; a testless package gets a `use <pkg>;` smoke run).  It is a
FUNCTIONAL gate — no `LOFT_DENY_WARNINGS`; warning-cleanliness stays the
source repo's job.  Run it locally the same way:

```bash
LOFT=target/release/loft scripts/registry_validate.sh crypto
LOFT=target/release/loft scripts/registry_validate.sh crypto@0.3.5   # a specific release
```

**Which version gets validated** — the newest stable in the index,
resolved from the index with a forced refresh (`loft api --registry
--refresh`) before anything is installed, and installed as an explicit
`<pkg>@<version>` pin.  The verdict names it: `OK — crypto 0.3.5`.

Both halves of that are load-bearing, and loft#1027 is why.  Run straight
after publishing `hex_way 0.1.1`, the script used to install against the
UN-refreshed index (which still ended at 0.1.0, so the install was a
no-op — *"Already cached (skipped)"*), then pick the highest version
directory in `~/.loft/registry/`, validate **0.1.0**, and print a bare
`OK`.  A publisher asking "is what I just shipped healthy?" got a green
light about the release it replaced.  Reading the local cache also made
the answer depend on which versions that machine had downloaded before —
had something already fetched 0.1.1, the same command would have
validated 0.1.1 while the install step still resolved 0.1.0.  Naming a
version that is not the newest is still allowed, because validating an
older release on purpose is legitimate; the run says so on its own line.

Rot classes it catches (all three found in the first live sample,
2026-07-04): a released package the new toolchain rejects (cbor 0.1.0,
DN1 type error), a machine-local `path =` dependency leaked into a
published `native/Cargo.toml` (crypto 0.3.4 — unbuildable anywhere but
the publisher's machine), and toolchain-driven native-crate breakage
(the loft-libs-core#14 class).  A red leg means "publish a fixed
version", not "edit the registry".

### Warning debt — reported nightly, never gated

Both nightlies are FUNCTIONAL gates, so a library that merely *warns*
against the newest loft stays green forever — and the debt only surfaces
as a red check on the next PR to that library's repo, where `library-ci`
runs `LOFT_DENY_WARNINGS=1` on code the author never touched (gridmesh
0.1.2: 20 warnings, every nightly green).  `revalidate-libs.yml` closes
that blind spot with a report, not a gate — warnings are non-contractual,
so a new deprecation must never fail a shipped artifact.

Each matrix leg runs `scripts/lib_warning_scan.py` twice and the `gate`
job merges the results into one table in the job summary:

| column | read from | what it means | fix |
|---|---|---|---|
| **published** | the suite log the hard gate already captured (free) | what a *user* of the library sees today | republish |
| **source** | a checkout of the repo's default branch | what that repo's own CI does on its next PR | clean the source |

Warnings raised inside a *dependency* are counted separately and never
charged to the package — the same rule `loft test --deps` applies.  Run
one reading locally:

```bash
LOFT=target/release/loft scripts/lib_warning_scan.py scan <pkg-dir> --label source
```

**A zero is only as good as the evidence behind it**, so every reading carries
that evidence and the report distinguishes three outcomes rather than two:

| shown | meaning |
|---|---|
| `clean` | the suite compiled N files and none warned |
| `**n/a**` | **inconclusive** — the suite never ran, so nothing could warn; a zero here means nothing |
| `†` | the reading came from the **registry copy**, not the scanned directory |

The `n/a` case is the one that matters: a run that dies before parsing (no loft on
`PATH`, an unresolvable dependency, an empty `tests/`) emits no warnings either,
and printing that as `clean` is the same silence-reads-as-coverage failure the
report exists to expose.  The summary line withholds its all-clear whenever any
reading is inconclusive.

The `†` marker records *which source was measured*.  A package's own tests say
`use <pkg>;`, and loft may satisfy that from the registry rather than the checkout
beside them (a cold cache says `[registry] downloading <pkg> <version>`; a warm one
resolves silently).  Scanning an older
version's directory after a newer one is published therefore reports the NEW
source: `hex_world` 0.1.2 has seven `not null` in its `src/`, yet scans clean now
that 0.2.0 exists.  The nightly is unaffected — `discover` always checks out the
LATEST tag, so the two coincide — but they coincide by luck rather than by
construction, which is exactly the kind of thing a report should say out loud.

Surveyed Debian/apt's ecosystem for prior art.  Decisions:

### Adopted in the MVP (schema-level)

| Debian concept | Loft equivalent | Where |
|---|---|---|
| `Release.gpg` / `InRelease` (signed index) | `index.json.sig` (Ed25519) | [§ Index signing](#index-signing--indexjsonsig) — phase R3.5 |
| `Section:` field (categorisation) | `packages.<name>.categories` | Schema field, free-form tags |
| Reproducible-build pledge | `loft package` deterministic sha256 + CI re-verify | R1 + R9 |
| Source-build install (consumer compiles native) | Default behaviour; pre-built optional via `binaries` field | Schema slot |
| `apt-get update` (separate index refresh) | Cached index with 1h TTL + `--refresh` flag | [§ Local cache layout](#local-cache-layout) |
| Mirrors via `sources.list` | `LOFT_REGISTRY_URL` env var | [§ Decoupled lifecycle](#decoupled-lifecycle--debian-style-the-registry-is-a-repo) |

### Adopted as schema slots — resolver support deferred

These get reserved fields NOW so the schema doesn't need a bump
when implementation lands.  No client-side resolver changes
required for MVP.

| Debian concept | Loft schema slot | When to implement |
|---|---|---|
| `Conflicts:` | `versions.<v>.conflicts: []` | When two real packages can't coexist. |
| `Replaces:` | `versions.<v>.replaces: []` | When a package is renamed / forked. |
| `Provides:` (virtual packages) | `versions.<v>.provides: []` | When alternative implementations of the same capability appear. |
| Per-arch `.deb` files | `versions.<v>.binaries: {<triple>: …}` | When local `cargo build` becomes the install bottleneck. |
| `testing` / `unstable` release pockets | `versions.<v>.prerelease: bool` | When a package author wants a beta channel. |

### Not adopted — incompatible with loft's model

| Debian concept | Why we don't want it |
|---|---|
| Pre/post install scripts (`preinst`, `postinst`) | Security disaster: arbitrary code execution at install time.  Loft packages stay as data + Rust source + loft source — no install scripts.  Compilation of the Rust crate is the *only* code path that runs (and it's `rustc`, not the package's own scripts). |
| `debconf` (interactive config) | Wrong UX for a programming-language ecosystem; install must be scriptable + non-interactive. |
| `dpkg-divert` / `update-alternatives` | Solves file-conflict resolution for system-wide installs.  Doesn't apply — each loft package lives in its own `~/.loft/registry/<pkg>-<version>/` directory; no global file conflicts possible. |
| Triggers (one package reacts to another's install/remove) | Overkill for a programming-language ecosystem.  Real use case has yet to emerge. |
| Epoch versioning (`1:2.3.4-5`) | Debian needed this because some upstreams reset their version numbers.  Semver covers loft's case; no epoch required. |
| `main` / `contrib` / `non-free` section split (license tiers) | Becomes relevant when packages with restrictive licenses appear.  Current ecosystem is uniformly LGPL/MIT/Apache; a single tier is fine.  `categories` field above can carry a `non-free` tag if/when needed. |
| Source vs binary package split (`.dsc` + tarball vs `.deb`) | Our tarball bundles both — source IS the install unit.  When pre-built distribution lands via the `binaries` schema slot, it'll be an OPTIONAL acceleration, not a separate package type. |

This split is **load-bearing**: future PRs that propose any of the
"not adopted" items should be redirected here.  The rationale is
recorded so the decision doesn't get re-litigated.

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

## Implementation phases (suggested order)

| Phase | Item | Effort | Blocker |
|---|---|---|---|
| **R1** | `loft package` CLI — produce tarball + sha256 from a `loft.toml` package | S | **DONE 2026-05-24** — `src/package.rs` + `loft package` subcommand.  4 unit tests + smoke-tested on `lib/crypto` (5.6 kB) and `lib/web` (21.5 kB), sha256 stable across runs. |
| **R2** | `loft.lock` reader/writer | S | **DONE 2026-05-24** — `src/lockfile.rs` (NOT feature-gated; lockfile is read on every loft invocation).  Atomic-rename writer, schema_version pin, 11 unit tests. |
| **R3** | Registry repo bootstrap docs | XS | **DONE 2026-05-24** — `doc/claude/REGISTRY_BOOTSTRAP.md` runbook + `doc/claude/registry_sample.json` template.  The actual GitHub repo creation is a maintainer-driven manual op. |
| **R3.5** | Index signing (Ed25519) | S | **DONE 2026-05-24; trust root BOOTSTRAPPED 2026-06-14 (PR #371)** — `src/registry_keys.rs` `TRUSTED_PUBLIC_KEYS` now holds **4 independent keys** (2 software laptop signers `K_laptop`/`K_laptop_tuxedo` + 2 on-card YubiKeys `K_yubiA`/`K_yubiB`; [REGISTRY_BOOTSTRAP.md](REGISTRY_BOOTSTRAP.md)), `src/registry_signing.rs` `verify_index` (4 tests). Live index signed; `scripts/registry-sign.sh` is the review-then-sign path (default on-card, local-key fallback, trust-gated against `TRUSTED_PUBLIC_KEYS`). Activates on the next loft release. |
| **R4** | `loft install <name>[@<v>]` — index fetch, sig verify, resolve, download, extract | M | **DONE 2026-05-24** — `src/registry_index.rs` (schema + parser + version constraint resolver + HTTPS fetcher + tarball extractor, 12 tests) + `src/install.rs` (orchestrator, 3 tests).  CLI flags wired: `--refresh`, `--offline`, `--prerelease`, `--allow-unsigned`, `--require-signature`.  Falls back to legacy text-format registry when `LOFT_LEGACY_REGISTRY` is set (preserves existing tooling). |
| **R5** | `loft install` (no args; reads project loft.toml) | S | **DONE 2026-08-18** (loft#966).  Marked done in 2026-05 as "subsumed by R4", which was a misreading: R4 gave `install_one` the transitive resolver, but the no-args ENTRY POINT was never wired to it — bare `loft install` installed the PROJECT into `~/.loft/lib/<dir>` instead, so the row above ("Reads `loft.toml`, resolves, installs") described a path no code took for three months, while `loft api` recommended the command for exactly the case it did not address.  `install_manifest_dependencies` (`src/main.rs`) now walks `[dependencies]`: registry deps through `install_one` with the declared requirement, path deps reported only when the path leads to no package.  `loft install .` keeps install-this-project.  Guarded by `tests/install_naming.rs`. |
| **R6** | `loft update [<name>]` | S | **DEFERRED** — re-runs the R4 flow with `--refresh`; needs a one-line subcommand to invalidate the lockfile pin for `<name>` before resolution.  Trivial extension once the registry is live; not blocking other phases. |
| **R7** | Diamond / transitive resolution | S | **DONE 2026-05-24** — `install::resolve_recursive` walks `deps` from each resolved version.  Diamond conflict detection (re-check the new constraint against the existing pin) is the next refinement; today the resolver picks the FIRST satisfying version per name. |
| **R8** | `loft search`, `loft info` | XS | **DONE 2026-05-24** — `loft search [query]` (case-insensitive match on name + description + categories) and `loft info <name>` (homepage, categories, latest, deps, version table with yanked/prerelease tags). |
| **R9** | Registry CI validator | S | **DONE 2026-05-24** — template scripts at `doc/claude/registry_ci_template/` (validate.py, pr-validate.yml, README, registry_README.md).  Drop into `loft-lang/registry` once bootstrapped.  Signing happens locally on the maintainer laptop via `loft-keygen sign`, NOT in CI — see [§ Why laptop signing, not CI](#why-laptop-signing-not-ci). |
| **R2** | `loft.lock` schema + writer (PKG.7 from PACKAGES.md) | S | none |
| **R3** | Bootstrap empty `registry.json` in `loft-lang/registry` repo | XS | none |
| **R3.5** | Index signing (`index.json.sig`) + public-key embed in loft binary.  Borrowed from Debian's `Release.gpg`.  See [§ Index signing](#index-signing--indexjsonsig). | S | R3 |
| **R4** | `loft install <name>[@<v>]` — index fetch, **signature verify**, resolve, download, verify tarball sha256, extract | M | R1, R2, R3, R3.5 |
| **R5** | `loft install` (no args) — read project loft.toml, install per lock | S | R4 |
| **R6** | `loft update [<name>]` — re-resolve | S | R4 |
| **R7** | Diamond / transitive resolution | S | R4 |
| **R8** | `loft search`, `loft info` — client-side index queries | XS | R4 |
| **R9** | CI validation script for `loft-lang/registry` PRs — schema lint + sha256 verify + **reproducible-build re-check** (rebuild from source, compare sha256) | S | R1, R3 |
| **R10** | First real publish: `lib_plans/12-library-extraction` Phase 4 (`loft-libs-core` chunk) extracts crypto + arguments + random + shapes | M | R1-R9 |

Total MVP scope to "PKG.REG done": **R1 through R9** (including
the new R3.5 signing phase).  R10 is the first user of the
registry, owned by plan-12.

| **R10.5** | First real key-rotation drill — exercise the rotation path while no real compromise is happening. | XS | R3.5, R10 |

### Status: MVP code complete 2026-05-24

R1-R9 (excluding R6 `loft update`, deferred until live registry
exists) all DONE in the client binary.  The remaining work is
ECOSYSTEM bootstrap:

1. Maintainer runs [REGISTRY_BOOTSTRAP.md](REGISTRY_BOOTSTRAP.md)
   — generate Ed25519 keypair offline, embed pubkey in
   `src/registry_keys.rs`, create `loft-lang/registry` repo, ship
   CI templates from
   [registry_ci_template/](registry_ci_template), seed empty
   `index.json`.
2. Loft minor release with the embedded trust root.
3. First publish (R10) — typically `crypto` from
   [`lib_plans/12-library-extraction/`](lib_plans/12-library-extraction)
   Phase 4.
4. R10.5 key-rotation drill before any real compromise.

### Coverage check — the registry must not drift behind the repos

`scripts/check_registry_coverage.sh` (loft repo) compares every
`loft-lang/loft-libs-*` library's `loft.toml` version against the
published `index.json` and warns on **missing** (no entry at all) and
**stale** (repo version newer than the newest published one) libraries.
It runs per-PR as the advisory CI job "library registry coverage"; run
it locally with `--strict` (exit 1 on findings) as a pre-publish gate.
The fix for any finding is the publish flow in
[REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md).  Libraries still inside the
loft repo's `lib/` are out of scope — they are unextracted by design
(PKG.EXTRACT).

### Maintainer fast-path — one run, one OK, one signature

`scripts/registry_maintain.sh` turns the findings into a single
sitting.  One run gathers the combined worklist — **own libs** to
publish (the coverage findings), **foreign submission PRs** on
`loft-lang/registry` with their validation-CI verdict, and **foreign
upstream drift** (an author's repo ahead of the registry,
informational).  After one confirmation it merges the green PRs,
re-filters the worklist against the post-merge index (a PR may have
covered a finding), then for each remaining own lib runs
`loft package` → creates the tag + GitHub release when absent →
`loft publish` → merges the emitted entry into `index.json`.  The
maintainer signs once (`loft-keygen sign`; key via `--key` /
`LOFT_REGISTRY_KEY`) and the script commits, pushes, and re-runs the
coverage check to confirm a clean state.  Without a key it stops after
staging and prints the two remaining commands — the signature never
moves off the maintainer's hardware (§ Why laptop signing, not CI).
`--dry-run` shows the worklist and changes nothing.

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

## Open work — `loft search` (registry discovery, R8+)

`loft search [query]` **exists** (PKG.REG R8: dispatch `src/main.rs:3922`, handler
`search_registry` `src/main.rs:507`; `loft info <name>` alongside it).  Today it loads +
verifies the index, filters case-insensitively over name / description / categories, and
prints one `name <latest> — description` line per hit, sorted alphabetically.  That ships
the *discovery loop everywhere* goal — from any loft project, not just the loft-repo
in-repo catalogue (`LIBRARIES.md`) — but it is the minimal cut.

**Shipped 2026-06-19 (commit `1a129a92`):** steps S0–S5 below brought it to the advertised
surface — shared loader, ranking, `⚡auto-use` marker, `--json` (through loft's own JSON
model), offline cache fallback.  The OPEN work now is **function-level API discovery**
([§ below](#next--function-level-api-discovery-search-the-callable-surface)): make the
*function* surface searchable so an agent finds a *capability*, not just a package — and
the discoverable front door for lazy auto-use
([lib_plans/59-lazy-stdlib](lib_plans/59-lazy-stdlib)).

### Gap — shipped (R8) vs target

| Spec clause | R8 today | Step |
|---|---|---|
| Reuse install's fetch/verify, **do not duplicate** | reuses, but via a hand-copied `loft_install_load_index` (`src/main.rs:2450`) duplicating private `install::load_index` (`src/install.rs:258`) | S0 |
| Rank exact-name > name-prefix > description/category | alphabetical only (`hits.sort_by name`) | S1 |
| Per-hit: `loft install <name>` line + **auto-use** marker when latest declares `triggers` | description line only | S2 |
| `(categories)` on the listing line | omitted | S2 |
| `--json` for tooling | absent | S3 |
| Offline → fall back to cache **and say so** | loader falls back silently; search prints no note | S4 |
| Discoverable in `--help` | `print_help` lists `install`/`pin`, not `search`/`info` | S5 |

### Output contract (the human format these steps converge on)

One line per package; a query also gets an install hint beneath each hit:

```
<name> <latest>[ ⚡auto-use] — <description>  (<cat1>, <cat2>)
    → loft install <name>          # printed only when there is a query
```

`⚡auto-use` appears only when the latest non-yanked, non-prerelease version has a
non-empty `triggers` (`Version.triggers`, `src/registry_index.rs:60`); `(categories)` is
omitted when empty.  This is the public CLI contract — pin it with the tests below so it
cannot drift.

### Design — verifiable steps (S0–S5 · ✅ SHIPPED 2026-06-19, commit `1a129a92`)

All six landed with regression tests — `rank_hits_orders_exact_prefix_then_description`,
`to_json_string_round_trips_through_parse` (the parser's inverse `json::to_json_string`),
and the `load_index_reporting_falls_back_*` e2e; clippy + fmt clean.  Two as-built
deltas from the plan: S4 was *contained* — `load_index` keeps returning the plain index
(install path untouched) and a new `load_index_reporting` carries `stale_fallback`, so the
blast radius stayed off the 8 install callers; and S5 was already present in `--help`
(only the `--json` mention was added).  The steps below are the record.

Effort XS–S total; each step is independently shippable.  Land **S0 first** (it makes the
spec's "reuse, do not duplicate" true and gives the later steps one place to change);
**S4 last** (it widens the shared loader's return type, so it carries the most blast
radius).  Build under `--features registry`.

- **S0 — collapse the duplicate loader.** Make `install::load_index` `pub`
  (`src/install.rs:258`), point `search_registry` + `package_info` at it, delete
  `loft_install_load_index` (`src/main.rs:2450`).
  - *Check:* `grep -c 'fn loft_install_load_index' src/main.rs` → `0`;
    `cargo test --features registry registry` stays green (install + e2e still pass).

- **S1 — ranking.** Extract a pure `rank_hits(index, query) -> Vec<&Package>` into
  `registry_index.rs` (exact-name > name-prefix > description/category hit; alphabetical
  within a tier) and call it from `search_registry`.
  - *Check:* unit test in `registry_index.rs` over a fixture where query `text` matches a
    package named `text` (exact), `text_utils` (prefix), and one whose description
    contains "text" (desc) → asserts that exact ordering.

- **S2 — richer hit line.** Append `(categories)` and the `⚡auto-use` marker (latest
  version `triggers` non-empty) to the listing line; under a query, print the
  `→ loft install <name>` hint per hit.  Mirror `render_catalog`'s `writeln!`-into-String
  style (`src/registry_index.rs:748`).
  - *Check:* e2e test (mirror `tests/registry_e2e.rs` `FixtureServer`) with one package
    whose latest version has `triggers` and one without → output contains `⚡auto-use` for
    the first and not the second, and the `→ loft install` line appears with a query and
    not for the bare-listing run.

- **S3 — `--json`, through loft's own JSON model.** Parse `--json` in the `search` arm
  (`src/main.rs:3922`).  Build the result set as a `json::Parsed` tree (`src/json.rs:33`,
  loft's canonical JSON value) — `Parsed::Array` of `Parsed::Object` with
  `name / version / description / categories / auto_use / install` — and render it; do
  **not** hand-roll a string builder, and do **not** use `ir_schema::value_to_json` (that
  emits the schema-tagged IR dialect `{"k":"Int",…}`, not plain JSON).  The `json` module
  today only *parses*; add its missing inverse `pub fn to_json_string(p: &Parsed) -> String`
  there (store-free, the natural home), reusing the string-escape routine the `to_json`
  native already carries (`src/native.rs:~3872` — lift it to a shared `json::escape_str` if
  that stays clean).  `--json` is then: build the tree, `println!("{}", json::to_json_string(&tree))`.
  - *Check (round-trip via the existing parser):* a `json.rs` unit test asserts string-level
    idempotence — `let s = to_json_string(&sample); assert_eq!(to_json_string(&json::parse(&s).unwrap()), s);`
    (compare the *strings*, not the trees: `Parsed::Object` carries byte offsets that differ
    between a built tree and a parsed one).  Plus
    `target/release/loft search text --json | python3 -m json.tool` exits `0` (this box has `python3`).

- **S4 — offline note.** Widen `load_index` to report cache-vs-fresh (e.g. return
  `LoadedIndex { index, from_cache: bool }`); when `from_cache` because the fetch failed,
  `search`/`info` print one stderr line: `registry unreachable — showing cached index`.
  - *Check:* e2e — prime the cache via a reachable `FixtureServer`, then re-run with
    `LOFT_REGISTRY_URL` pointed at a dead port → stderr contains `cached`; the reachable
    run prints no such line.

- **S5 — discoverability.** Add `search` + `info` to `print_help` after the `install` /
  `pin` lines (`src/main.rs:167`).
  - *Check:* `target/release/loft --help` lists both `search` and `info`.

**Why this is the right next slice:** it finishes the discovery loop's *surface* (ranking,
auto-use signposting, machine-readable output) so library reuse is frictionless from any
project, and it makes lazy auto-use ([lib_plans/59-lazy-stdlib](lib_plans/59-lazy-stdlib))
*discoverable* rather than magic.

### Next — function-level API discovery (search the callable surface)

> **Shipped 2026-06-20 — S6, S6b, S7-publish, S7-CI, S8, S9 (complete).**  `loft search <fn>`
> surfaces function-level hits (signature + one-line doc + how-to-get) across the index `api`
> field AND the embedded stdlib, grouped under their package; `--json` carries per-function
> records `{ source, package, version, signature, doc, get }`.  `Version.api: Vec<ApiItem>`
> parses optional / default-empty (`registry_index.rs`, like `triggers`); `loft publish`
> auto-derives it from `src/*.loft` via the same `parse_pkg_api` extractor
> (`documentation::pkg_api_items` → `extract_api_items`); the stdlib feeds from the binary's
> embedded `default/*.loft` (`stdlib_sources::STDLIB_SOURCES` — one home shared with the WASM
> runtime, no disk dependency).  `registry_index::search_results` is the pure, tested ranker.
>
> **S7-CI — the no-drift trust gate.**  `loft api --json <dir>` emits a source dir's
> function-level surface (via the same `pkg_api_items`, so it equals what publish embeds); the
> registry's `validate.py` `gate_reproducible_build` re-derives it from the cloned source and
> REJECTS a pasted `api` that disagrees — so discovery can never point at a function the source
> lacks.  (A package without a GitHub homepage has its `api` trusted-as-pasted, exactly like
> its sha256 — no source to clone.)  The live registry index carries no `api` yet, so registry
> function-search lights up as packages republish; the stdlib surface works today.

S0–S5 search **package** metadata (name / description / categories).  That is too coarse
for an agent: the real question is *"is there a function that does X, and how do I call
it?"*  Today a registry package's function-level API is visible **only after you install
it** (`loft api` / `.loft/api` stubs read the extracted source) — a discover-to-install
circularity.  This slice breaks it: surface each library's **functions** — signature +
doc — from any project, without the source, **and** put the **stdlib on the same surface**.

**One mechanism, two feeds, one surface.**  There is already a single API-surface
extractor — `parse_pkg_api` / `render_pkg_api_text` (`src/documentation.rs:1122`,`:1231`),
heuristic and *good enough* (it is what `loft api` and the `.loft/api` stubs already use).
Run that one routine in two feeds and render both identically:

```
                 ┌─ publish time, over each library's source  → index.json `api` field (auto, CI-verified)
parse_pkg_api ──>┤
                 └─ search time, over the binary's embedded default/*.loft → stdlib hits (always present)
                                          │
                            loft search / --json  ← one surface; stdlib and libraries the same shape
```

**The invariant** (the load-bearing claim to hold): *the index's `api` field is a pure,
CI-verified function of the published source, and the stdlib feeds the same surface from
the binary's embedded `default/*.loft` — a consumer never extracts from source it does not
have.*

**S6 — index schema: the `api` field (auto-derived, no hand-crafted data).**  `Version`
gains `api: Vec<ApiItem>` where `ApiItem = { sig: String, doc: String }` (a **one-line**
doc summary — full doc stays available via `loft api <pkg>`).  Parse it with the
nested-object idiom already used for `binaries` (`src/registry_index.rs:290`); it is
**optional**, so older indexes lacking it default to empty and never error (same as
`triggers`, `:281`).  It is **emitted automatically** by the publish path — the exact
mirror of how `triggers` are derived from source and written today
(`src/main.rs:2055-2069`, `crate::triggers::derive_triggers` `src/triggers.rs:58`): the
author runs `loft package` and pastes the whole generated entry; **nothing is
hand-written**.

**S7 — CI is the source of truth.**  The registry's `validate.py` gate 3 already clones
the tagged source and runs `loft package` to re-check the tarball sha256
([registry_ci_template/validate.py](registry_ci_template/validate.py)).  Extend that gate
to **re-derive `api` from source and reject (or overwrite) a mismatch** — so the field is
a pure function of the code and cannot drift, even though the author pasted it.  This is
what makes discovery *trustworthy*: an agent never finds a function that no longer exists.

**S8 — stdlib on the same surface.**  The loft binary already embeds `default/*.loft`.
`loft search` runs the **same** extractor over that embedded source at search time, so
stdlib functions appear as hits **identical in shape** to library functions — differing
only in the "how to get it" line ("built in — use directly", no install).  The stdlib API
therefore lives in the binary (always fresh) and **does not bloat the fetched index**.

**S9 — rich results: tell the agent what it needs.**  A function match is a function-level
hit carrying the three things needed to act — *what it does* (doc), *how to call it*
(signature), *how to get it* (install vs built-in).  Group the matching functions under
their package; `--json` carries them structured.

```
<pkg> <ver>
    <signature>
        <one-line doc>
    → loft install <pkg>            #   or:  (stdlib — built in, use directly)
```

`--json` per function: `{ "source": "stdlib"|"registry", "package", "version",
"signature", "doc", "get" }` — everything an agent needs to decide and call, in one place.

**Decided shape** (the three forks, resolved): functions **grouped under their package**
(scannable, agent still sees every match) · **one-line doc summary in the index**, full
doc on demand via `loft api` (keeps the index lean) · **CI re-derives** the `api` field
(no trusted hand-crafted data) · stdlib extracted **on the fly** from embedded source
(simple, always current).

**Failure paths** (the generative pass — each is why a property above is load-bearing):

- *Stale / edited `api` field* → S7's CI re-derive rejects the mismatch; the field can
  only ever equal the source.  (Without S7, discovery becomes confidently wrong — worse
  than absent.)
- *Signature drift across versions* → `api` is **per-`Version`**, so each version carries
  its own surface; an old pin still describes the functions it actually shipped.
- *stdlib vs library name collision in results* → every hit is tagged `source`
  (`stdlib`|`registry`); results never merge the two silently.
- *Index growth* → only a one-line summary per function lives in the index; if the single
  `index.json` still outgrows a single fetch, split to a per-package `api` file fetched on
  demand (the only change that dents the "one agnostic fetch" property — defer until
  measured).
- *Function with no/cryptic doc* → discoverable by name + signature only; quality is
  bounded by the library's doc quality, which ties discovery to
  [LIBRARY_CHECKLIST.md](LIBRARY_CHECKLIST.md) / [DOC_QUALITY.md](DOC_QUALITY.md) /
  [API_SURFACE.md](API_SURFACE.md) `api-lint`.
- *Heuristic extractor misses an exotic signature* → it degrades to "no hit / raw
  signature line", never a wrong call — `good enough` is the right bar for discovery.

**Sequencing & effort:** S6 schema + parse (XS) → S7 publish auto-derive + CI verify (S)
→ S8 stdlib feed (S) → S9 rich results + `--json` (S).  Builds directly on the shipped
S0–S5; the client stays agnostic throughout (it only reads the enriched index + the
embedded stdlib).

**Why this resolves agent data discovery** (per the evaluation that drove it): the
published ecosystem's *callable* surface becomes visible from any project **without
installing**, **authoritative** because CI-derived, and **unified with the stdlib** — so
"what can do X, and how do I call it?" is answered in one place, with the signature to use
it and the command to get it.

### Phase 2 — richer data + live deployment (S10–S12)

Phase 1 (S0–S9, shipped) built the mechanism; the gathered data is still thin (one-line
summaries) and lives only in the binary's stdlib — the **live registry index has 0 of 20
packages carrying `api`**.  Phase 2 widens the data and gets it into the live index two ways:
future releases self-populate, and a one-time backfill converts everything already published.
**The invariant is unchanged** — `api` is a pure, CI-verified function of a version's
published source, never hand-written, never drifting.

#### S10 — gather functions AND types; match the FULL doc (Google-like) — SHIPPED 2026-06-20

Each `api` item today is `{ sig, doc }` with `doc` = the *first* documentation line (a title),
so search is title-only: "anything about collision?" misses `rects_overlap` unless "collision"
is in its title.  Widen the gathered data to the whole paragraph so search matches ANY keyword
in the docs.  Per public item:

- **`sig`** — the declaration with the body stripped: `pub fn sha256(data: text) -> text`,
  `pub struct Rect`, `pub enum Shape`.  **Functions AND types** (`pub fn` / `pub struct` /
  `pub enum`) — searching `rect` finds both `struct Rect` and `fn rect_circle_overlap`.
  (`pub const` / values stay out: the question is "what can I CALL or USE", and their
  inline-comment tails make noisy signatures.  The extractor filters `pub ` items to
  fn/struct/enum and strips any trailing `// …` from the sig.)
- **`doc`** — the FULL contiguous documentation paragraph above the item (every `//` line,
  joined), not just the first.  This is the keyword corpus search matches.

**Distinguishing the doc from other comments** — the rule (already implemented in
`parse_pkg_api`, made load-bearing here): the doc is the CONTIGUOUS `//` block IMMEDIATELY
above the `pub` item; a blank line or a code line between a comment and the item severs it
(that comment is not the doc); `// --- Section ---` decorative headers are section markers
(they group items + reset the accumulator), never docs; a `#rust "…"` annotation line does not
sever the doc.  This is "good enough" because the rule is mechanical and the conventions are
ALREADY followed everywhere (verified: crypto's `// SHA-256 hash …` paragraphs above each fn,
shapes' `// Axis-aligned bounding box …` above `pub struct Rect`, the `// --- Public API ---`
markers).  A library wanting better search writes a fuller paragraph; one that writes nothing
is still found by name + signature.

Data-model change: `ApiItem.doc` holds the full paragraph (the matching corpus); the search
RESULT still prints the one-line summary (the first sentence) for a clean display; `--json` and
the index carry the full `doc`.  Matching becomes Google-like: lowercase the query, split into
terms, and require EVERY term to appear somewhere in `sig`+`doc` (AND-semantics, the search
default) — so "hash hex" narrows rather than widens.  Cost: the index grows from titles to
paragraphs — still small (a paragraph per item; the whole index is ~22 KB today), and keyword
search is the whole point.

#### S11 — forward: every release self-populates the index

Today a library release is MANUAL (verified): the author tags `<pkg>-v<ver>`, runs
`loft publish` locally, and pastes the entry into a `loft-lang/registry` PR; the registry
`validate.py` is VALIDATE-ONLY (it never writes `index.json`).  `loft publish` already EMITS
`api`, so the field is auto-GENERATED — the author never writes it.  The gap is ENFORCEMENT: an
author who uses `loft package` (the basic entry, no `api`) or hand-edits the row would ship a
wrong or absent field.

Make `validate.py` the authority.  Its `gate_reproducible_build` already clones the tagged
source and runs `loft`; extend it (the S7-CI gate, already in this repo's
`registry_ci_template/validate.py`) so that for every NEW version entry it (1) re-derives `api`
from the cloned tagged source via `loft api --json`, and (2) REQUIRES the entry's `api` to equal
it — rejecting a missing, stale, or hand-edited field.  Then every future release carries the
correct `api` automatically: the author gets it free from `loft publish`, and the gate makes it
non-optional and proven-against-source.  No registry write-access or bot is needed — the author
pastes what `loft publish` produced, the gate proves it equals the source.

Deployment (registry repo, one PR): port the template gate into
`~/workspace/registry/tools/validate.py`; add `api` to `gate_schema`'s required set for new
entries; keep the `registry` feature in the CI's `cargo build --release --bin loft` so
`loft api --json` exists.  The loft side (`loft publish` emitting, `loft api --json`) is already
shipped.  *(Optional convenience: a `--fix` mode that writes the corrected entry, so a wrong
submission is handed the fix rather than only rejected — not required for correctness.)*

#### S12 — backfill: one-time conversion of every existing lib

S11 only fills FUTURE releases; the ~20 already-published packages (0 with `api`) need a
one-time conversion.  A script (registry-repo `scripts/backfill-api.py`):

1. For each package's LATEST version in `index.json` (search shows the latest; all-versions is
   an optional superset), read its tag `<pkg>-v<latest>`.
2. Clone the package's monorepo at that tag into a tempdir — or, since the six `loft-libs-*`
   monorepos are checked out locally and freshly released, use the local subdir when its `main`
   equals the tag (the fast path; fall back to the tag clone when they differ).
3. `loft api --json <subdir>` → the `api` array.
4. Write it into that version entry.
5. Open ONE registry PR with all backfilled entries; the S11 gate verifies each — and PASSES,
   because they were re-derived from the very source the gate re-clones.

The backfill is the SAME derivation as the forward gate, applied to existing rows — consistent
by construction (one extractor, `loft api --json`).  After it merges, registry function-search
lights up for every shipped library; until then only the stdlib surface returns function hits.

**Sequencing & effort:** S10 widen-doc + types **(SHIPPED on `searching`: full doc paragraph
as the keyword corpus, AND-of-terms matching, fn/struct/enum/typedef/interface, consts dropped,
clean type sigs)** → S11 port the gate + require the field (S, registry repo) → S12 the backfill
script + one PR (S, registry repo).  S11 + S12 are registry-repo PRs that depend on a loft
release carrying `loft api --json`.

---

## See also

- [PACKAGES.md](PACKAGES.md) — package format reference; this doc
  is the registry-specific draft of that doc's "Open work" PKG.REG
  bullet.
- [lib_plans/12-library-extraction/](lib_plans/12-library-extraction) —
  consumer of PKG.REG.  Phases 4-8 unblock when PKG.REG ships.
- [STDLIB.md § Logging](STDLIB.md) — `loft install` should log via
  the same machinery; useful for `--verbose` output.
