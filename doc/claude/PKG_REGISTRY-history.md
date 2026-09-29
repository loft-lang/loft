<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->

# PKG_REGISTRY-history.md — the timeline behind PKG_REGISTRY.md

The timeline behind [PKG_REGISTRY.md](PKG_REGISTRY.md): design revisions and the R1–R9 implementation record.

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
| **R9** | Registry CI validator | S | **DONE 2026-05-24** — template scripts at `doc/claude/registry_ci_template/` (validate.py, pr-validate.yml, README, registry_README.md).  Drop into `loft-lang/registry` once bootstrapped.  Signing happens locally on the maintainer laptop via `loft-keygen sign`, NOT in CI — see [§ Why laptop signing, not CI](PKG_REGISTRY_TRUST.md#why-laptop-signing-not-ci). |
| **R2** | `loft.lock` schema + writer (PKG.7 from PACKAGES.md) | S | none |
| **R3** | Bootstrap empty `registry.json` in `loft-lang/registry` repo | XS | none |
| **R3.5** | Index signing (`index.json.sig`) + public-key embed in loft binary.  Borrowed from Debian's `Release.gpg`.  See [§ Index signing](PKG_REGISTRY_TRUST.md#index-signing--indexjsonsig). | S | R3 |
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


## Design revisions

**Revision 3 (2026-05-24)** — switched index signing from CI-based
(GitHub Actions + `REGISTRY_SIGNING_KEY_BASE64` secret) to
**maintainer-laptop-based**.  Same crypto (Ed25519); private key
now never leaves hardware the maintainer physically controls.
`loft-keygen sign` / `loft-keygen verify` subcommands added.
Removed `sign-and-commit.yml` + `sign-index.py` from the CI
template directory.  Rationale in [§ Why laptop signing, not CI](PKG_REGISTRY_TRUST.md#why-laptop-signing-not-ci).

**Revision 2 (2026-05-24)** — Debian-comparison pass added.
Promoted index signing from "Path 1 server feature" to "MVP R3.5"
(real security gap closed).  Added schema slots for `conflicts` /
`replaces` / `provides` / `binaries` / `prerelease` / `categories`
(Debian-inspired, reserved fields, resolver-side support deferred
to keep MVP scope tight).  Explicit "not adopted" table records
items deliberately rejected (pre/postinst scripts, debconf,
alternatives, triggers, epoch versioning, `main`/`contrib`/`non-free`)
so future PRs aren't re-litigated.  See
[§ Decoupled lifecycle — Debian-style](PKG_REGISTRY.md#decoupled-lifecycle--debian-style-the-registry-is-a-repo).
