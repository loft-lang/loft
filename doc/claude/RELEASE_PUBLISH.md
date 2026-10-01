<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Publishing a release

How a release is tagged, built, published and made installable, and what the tag pipeline
proves about the artifacts it ships.  Whether a release may be cut at all is
[RELEASE.md](RELEASE.md); the owner tags and publishes by hand
([RELEASE.md § No Automated Releases](RELEASE.md#no-automated-releases)).  The incidents
behind these rules are in [RELEASE-history.md](RELEASE-history.md).

## Contents
- [Tag & publish — the mechanics](#tag--publish--the-mechanics-draft-first-under-immutable-releases)
- [The bundles and the registry entry](#the-bundles-and-the-registry-entry)
- [What the tag pipeline proves about the artifacts](#what-the-tag-pipeline-proves-about-the-artifacts)
- [The reference PDF](#the-reference-pdf)
- [Two install layouts](#two-install-layouts)
- [Reproducible builds](#reproducible-builds)
- [Release artifacts](#release-artifacts)

## Tag & publish — the mechanics (draft-first, under immutable releases)

The org enforces **immutable releases**: a release's assets freeze the moment it is
published.  So the four platform bundles are attached while the release is still a
**draft**, and the owner never publishes an empty release and waits for binaries.  **Never
create and publish a release in one step:** publishing creates the tag and freezes the
release before anything is built, and the upload is then rejected.

1. **Push the annotated tag** — `git tag -a vX.Y.Z -m "…" && git push origin vX.Y.Z`.  The
   tag push, not a published release, triggers `release.yml`.
2. **Let CI build the draft.**  `release.yml` builds the four targets, smoke-runs each bundle
   ([below](#what-the-tag-pipeline-proves-about-the-artifacts)), and creates the release as a
   **draft** with every bundle and `.sha256` attached and notes generated.  The draft job also
   attaches `loft-<v>-src.zip` (the source archive the registry entry names) and
   `loft-<v>-registry-entry.json`.  If any build leg fails, no draft appears — investigate;
   never ship a partial release.
3. **Review, then publish.**  Open the draft, confirm the four bundles are present, edit the
   title or body if wanted, then click **Publish**.  Only this click freezes the release, and
   by then the binaries are attached.  Publishing an existing-tag draft does not rebuild.
4. **Submit the registry entry** (`M-registry-splice`).  Splice
   `loft-<v>-registry-entry.json` from the published release into `loft-lang/registry`'s
   `index.json` under `packages.loft`, and re-sign (`scripts/registry-sign.sh`,
   [REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md)).  This makes the release reachable by
   `loft self-update`, and it is the ONLY step that puts the binaries under a signature: the
   `.zip.sha256` sidecars travel with the zips, so they catch a corrupted download, not a
   substituted one.  The signed index is the root:

   ```
   index.json                        ← the ONE signature (Ed25519, 4 trust roots)
    ├ binaries[triple].sha256          → loft-<v>-<triple>.zip   checked once, at download
    │  └ manifest_sha256              → SHA256SUMS             checked any time, on what is INSTALLED
    │     └ bin/loft, default/*.loft, and every other file the bundle shipped
    └ version.sha256                   → loft-<v>-src.zip        the source the release was built from
   ```

   Never hand-edit the hashes.  The entry is generated from the artifacts of the run that built
   them; retyping it produces an index that is correctly signed and names the wrong bytes.
   Land the entry before the version-bump PR: `check-release-published.py` gates that PR.

**A forgotten step 4 is caught twice.**  Only step 2 fails loudly; a missing entry just
leaves `self-update` saying "no releases published to compare against", which pages nobody.
- **On this release**, four automatic checklist items measure step 4's effect:
  - `A-validator-dryrun` runs the registry's own validator against the generated entry,
    spliced into a clone of the live index, BEFORE submission (`scripts/validator-dryrun.py`;
    `--replay` / `--corrupt` falsify the instrument).
  - `A-registry-this` asks the index directly for the entry, every published triple and each
    `manifest_sha256`.  The CLI cannot witness this: `self-update`'s `Current` verdict prints
    the running version whether the index carries this release or only an older one.
  - `A-selfupdate-resolves` runs `loft self-update --dry-run --refresh` and reads it; the
    empty-index message is a FAIL.  Pass `--refresh` whenever you check by hand: the index is
    cached under a TTL, and a cache older than the splice says the same words as an empty
    index.
  - `A-acquisition` (`scripts/acquisition-chain.sh`) runs install.sh over the real transport →
    `--version` → resolution → the literal `origin: matches the signed registry index` line →
    a program executed.  `.github/workflows/post-publish-verify.yml` is its standing copy on
    every publish, dispatchable mid-cycle against the previous release, because the live chain
    staying acquirable is a stability property.
- **On the next release**, the `previous release reached the registry` CI job is red on the
  PR that bumps `Cargo.toml` unless the last release's entry is in the signed index with a
  binary per published triple and a `manifest_sha256` on each
  (`scripts/check-release-published.py`).  It gates that PR only; red on every PR during the
  publish-to-merge window would teach everyone to merge past it.  A release with no
  `loft-<v>-src.zip` predates the mechanism and is exempt, derived from its assets.

## The bundles and the registry entry

The registry ([PKG_REGISTRY.md](PKG_REGISTRY.md)) is the trusted distribution point, so the
toolchain ships through it — signed, with checksums a user can verify offline.

- **One bundle per supported target**, built by `release.yml` through
  `scripts/make-release.sh`: `x86_64-unknown-linux-musl`, `x86_64-apple-darwin`,
  `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`.  An `aarch64-unknown-linux-*` target is a
  matrix row to add when it is needed.
- **Each bundle is a self-contained zip** — `bin/loft`, the `default/` stdlib, examples,
  `loft-reference.pdf`, `BUILD-INFO` and `SHA256SUMS` — attached to the draft as
  `loft-<version>-<triple>.zip` with its `.zip.sha256`.
- **One manifest per bundle.**  `SHA256SUMS` covers every file the bundle ships and is the
  authoritative list of what it owns (`self_update::owned_files` reads the same file).  There
  is no second, stdlib-only manifest: it would be a second way to validate one installation.
- **The registry entry** carries, per target, the bundle URL and sha256 **and
  `manifest_sha256`**, the digest of that bundle's `SHA256SUMS`.  The zip's hash is checkable
  once, at download; the manifest digest lets `loft verify-self` re-check an INSTALLED tree
  against the signature at any time.
- **Verify on a clean host:** `loft self-update` resolves a bundle, checks its hash against the
  signed index and installs it; `loft verify-self` then reports "matches the release published
  in the signed registry index".
- **Verify on Windows by hand, once per release, before announcing** — run a real
  `loft self-update` from the previous release to this one.  Replacing a RUNNING executable is
  the one platform-divergent step: `apply_bundle` renames the target aside and copies in,
  because Windows cannot overwrite a running binary but can rename it.  The unit tests exercise
  rename-then-copy on the Windows leg, never against the `loft.exe` executing them.

## What the tag pipeline proves about the artifacts

Artifact-level properties — does the binary run, is it the version the tag claims, does the
manifest describe the files, do the shipped examples work — do not exist before the zip does,
so the tag run is the only place they can be checked.

- **Each build leg unpacks its own zip** (not the staging directory: the round trip is under
  test) and asserts `--version` against the tag, `loft verify-self`, and every
  `examples/*.loft` under `--interpret` with **empty stderr**, not just exit 0: a loft program
  that cannot write its output prints `… — write skipped` and exits 0.  This per-bundle smoke is
  the cross-platform walkthrough; `A-smoke` reads it.
- **One leg cannot always run its artifact:** `x86_64-apple-darwin` is cross-built on an arm64
  runner and needs Rosetta 2.  It reports a loud skip, and `make release-checklist` turns that
  into the manual `M-rosetta` item instead of a silence.
- **The consumers, at the tag's commit.**  `release.yml` calls `consumer-main-health.yml`,
  which runs every package of the programs built with loft (moros, dryopea, crawler, the
  private economy model) on both backends — the same file that answers "main, tonight" on the
  schedule, so the two cannot drift.  It does not gate the draft: a red consumer is either loft
  moving under it or the consumer's own tip, and which it is decides whether the release ships.
  `A-consumers` reads the run per consumer; a red one holds the release unless the record names
  a cause outside loft (RELEASE.md § Explicitly out of scope here).
- **Installing from the zip.**  The hands-on path is the release ZIP, never a fresh git clone:
  the clone is not what users take.  `scripts/install.sh` — the documented `curl | sh` path —
  runs end to end in `tests/self_update_swap.rs` against a bundle built the way
  `make-release.sh` builds one, served over `file://`, and `tests/doc_hygiene.rs` checks that
  its `uname` mapping matches `PUBLISHED_TRIPLES`.  What only a by-hand run covers is the real
  transport (the GitHub CDN and its sidecar) and the shipped binary's own `verify-self`.

## The reference PDF

**`make-release.sh` copies `doc/loft-reference.pdf` into all four bundles and never builds it.**
The HTML docs are committed and GitHub Pages serves them from `main:/docs` (the `docs -> doc`
symlink), republished on every push to `main`, so they cannot go stale at a tag and the tag
pipeline has no docs step.  The PDF is a committed file only `gendoc` + `make pdf` update, so
three checklist items guard it:

- **`A-pdf`** — current against what decides its content: `tests/docs/`, `default/`,
  `src/gendoc.rs`, `src/documentation.rs` and `Cargo.toml`.  Not against
  `doc/loft-reference.typ`, which `gendoc` itself generates: when the real inputs move and
  nobody re-runs `gendoc`, both derived files sit still and that comparison reads green.
- **`A-pdf-version`** — the PDF says it is this release, read from its own bytes.  `gendoc`
  stamps the title page and keywords from `CARGO_PKG_VERSION`, so a version bump without a
  re-run leaves a fresh, correct-looking reference headed with the previous version.
- **`A-pdf-content`** — every level-1 part is present.  Every way a chapter enters the document
  can drop it in silence: `documentation::get_topic_sources` skips an unreadable topic file
  (`.ok()`, `filter_map`), and *Getting Started*, *vs Rust*, *vs Python* and *Roadmap* are each
  read from a `doc/*.html` file under `if let Ok(…)`.  So the check requires each topic's
  `@NAME` (the heading `gendoc` emits, not `@TITLE`) and each of those four chapters in the
  PDF's text, matched on word boundaries (a substring test finds `map` in "Roadmap").
  *Standard Library*'s heading is pushed unconditionally, so the check also requires that
  chapter to name at least one stdlib function.  No placeholder marker (`TODO`, `FIXME`, `TBD`,
  "not yet implemented") may ship in a document read offline.  The stdlib count rides along as
  evidence, not a gate: many functions are documented as methods on their receiver, so "every
  `pub fn` appears" would fail falsely.

None of the three reads a sentence; whether the reference still describes the language is
[REFERENCE_REVIEW.md](REFERENCE_REVIEW.md) (`make reference-review`, reported by
`A-reference-review`).

## Two install layouts

A release bundle is `<prefix>/bin/loft` beside `<prefix>/default/`; a source install
(`make install`) is `<prefix>/bin/loft` with `<prefix>/share/loft/{default,libloft.rlib,deps}`.
The runtime's resolver (`native_utils::project_root_for`) prefers `<prefix>/share/loft/`
whenever it exists, so `self-update` over a prefix that was once source-installed would write
a `default/` nothing loads and leave the older standard library winning under a newer binary.

`self-update` therefore REFUSES that case before writing anything, naming both trees and the
two layouts to choose between; `--force` installs anyway and says what state it leaves.  It
refuses rather than relocating the write because a release bundle carries no `libloft.rlib`,
and `--native` links that rlib from `share/loft/`: writing the stdlib into the source tree
would swap one mismatch for a subtler one.  Writer and loader ask the same home,
`stdlib_default_dir()`, which `loft run`, `loft fmt`, the LSP and `verify-self` all use.

## Reproducible builds

`SHA256SUMS` is integrity.  A byte-identical rebuild upgrades what the published hash MEANS,
from "the artifact the maintainer uploaded" to "the artifact the source produces".  The registry
re-checks this for libraries (gate 3 re-runs `loft package` on the tag); the toolchain is
exempt, because it is not a `loft package`, and the weekly `repro-build.yml` covers the gap
(@PLN78 step 7).

- **Paths:** `scripts/repro-flags.sh` remaps the build roots on BOTH sides (the release and the
  verifier source it), and `build.rs` drops every `--remap-path-prefix` entry before baking
  `LOFT_BUILD_RUSTFLAGS`.  That string must stay a string: `extensions.rs` passes it to child
  cargo builds so a shared dependency's SVH matches loft's own (#274).  Bundles carry
  `reproducible-paths = yes`.
- **The platform C toolchain:** `ring` (via rustls → ureq) compiles C through the `cc` crate
  with the runner image's compiler — `cl.exe` on Windows, Apple clang on macOS — and a hosted
  runner's toolset moves with its image.  So a bundle records its `c-toolchain` in `BUILD-INFO`
  (`scripts/repro-toolchain.sh`, sourced by `make-release.sh` and `repro-verify.sh`), and the
  verifier calls a difference the source's only when its own C toolchain matches that record;
  otherwise it exits 3, naming both.  Identical bytes need no record.  Pinned by
  `tests/doc_hygiene.rs::the_release_records_the_c_toolchain_the_verifier_compares`.
- **The Windows link:** `rust-lld` stamps a wall-clock TimeDateStamp and PDB GUID;
  `scripts/repro-flags.sh` passes `/Brepro` and `/DEBUG:NONE` on a Windows host.  A Windows
  bundle ships no PDB.
- **Comparing with a GitHub artifact** needs the musl target: releases ship
  `x86_64-unknown-linux-musl`, and a local `cargo build --release` is `-gnu`.

## Release artifacts

| Artifact | How |
|---|---|
| Annotated tag `vYYYY.M.P` | `git tag -a vYYYY.M.P -m "…" && git push origin vYYYY.M.P` — the push triggers `release.yml` |
| Four bundles `loft-<v>-<triple>.zip` + `.sha256`, each with `bin/`, `default/`, `examples/`, `loft-reference.pdf`, `BUILD-INFO`, `SHA256SUMS` | `release.yml` → `scripts/make-release.sh`, attached to the DRAFT, each smoke-run from its own zip |
| `loft-<v>-src.zip` + `loft-<v>-registry-entry.json` | the draft job, derived from the bundles it just built |
| crates.io `loft` | `release.yml` `crates-io` job, when `CARGO_REGISTRY_TOKEN` is set |
| HTML docs | committed under `doc/`, served from `main:/docs` and rebuilt on every push — no release step |
| The signed registry entry | step 4 above, by hand (`M-registry-splice`), read back by `A-registry-this` |

The Rust library API (`lib.rs`) is not a public stable API; what is stable is
[COMPATIBILITY.md](COMPATIBILITY.md)'s business.
