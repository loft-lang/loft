<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN102 — Library validation + race-free shipping (design)

> **Status: DESIGN (2026-07-15).** Built on what already exists — the file-based
> registry (`index.json` + `index.json.sig`), `loft package` / `loft test` /
> `loft api-surface`, `scripts/registry_maintain.sh` + `registry-sign.sh`. This spec
> adds three things: a **validation gate** every version must pass before it is
> trusted, a **race-free ship transaction** so concurrent lib landings can't corrupt
> the index, and a **key-gated trust split** (a key-present machine ships autonomously;
> a key-absent one defers). Companion decisions: **@C96** (key-gated ship) and **@C97**
> (library symbols are module-namespaced) — see [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md).

## The substrate (what we already have)

- **Registry** (`loft-lang/registry`): a static `index.json` + detached
  `index.json.sig` (Ed25519, trust-root key), on `main`. `loft install` fetches the
  index via the raw-GitHub CDN, **verifies the signature**, then fetches the package
  and checks its **sha256 + size** against the entry.
- **Tools**: `loft package` (source tarball + printed index entry), `loft test`
  (per-target test run — the parity gate), `loft api-surface` (public API snapshot,
  driven by `api-compat.yml`), `registry_maintain.sh` (discover → package → release →
  index → sign → push), `registry-sign.sh` (sign + trust-gate vs `TRUSTED_PUBLIC_KEYS`),
  `registry_validate.sh` / `check_registry_coverage.sh`.
- **Trust model today**: own libs are signed + pushed directly to `main`; a foreign
  author opens a **PR** that edits `index.json` and the maintainer re-signs. The trust
  key on this (`tuxedo`) machine is a **file**: `~/.loft/trust-root/registry-signing-key.bin`.

Two weaknesses this design closes: (1) nothing systematically **validates** a version
before it's trusted — a lib that fails the parity gate, breaks compat, or doesn't
compile against the current loft can be signed anyway (exactly the `shapes`/`time`
@C95 class); (2) a foreign PR and a maintain run **both edit `index.json`**, so two
libs landing near-simultaneously **race** on that one file.

## Part 1 — The validation gate (before a version is trusted)

A version is **signable only if it passes every check below**. The checks are pure
functions of `(source@tag, prev-registered-version, current-loft)`, so they run
identically in CI (on a submission PR) and on the signing machine (authoritatively,
just before signing — the signature then *means* "validated by a trust holder").

| # | Check | Reuses | Catches |
|---|---|---|---|
| **V1** | **Reproducible package** — `loft package` of the pinned source byte-matches the release tarball (sha256 + size). | `loft package` | stale-release drift (main moved past the tag with no version bump) |
| **V2** | **Parity gate** — `loft test` on every claimed target; **interpret == native == native-wasm (== browser)**, not just each exits 0. Bounded by `LOFT_TIMEOUT`; leak check where it allocates. | `loft test`, the ship skill's gate | interp/native/wasm divergence (the #1 lib-bug class) |
| **V3** | **Compiles against current loft** — clean parse+type on `--interpret` and `--native`. | `loft` | @C95 stdlib-name collisions, DN nullable-cast breakage — the whole `shapes`/`time` class |
| **V4** | **Compat** — `loft api-surface` diff `prev → new` is **additive-only**, unless the semver step declares breaking (pre-1.0: a **minor** bump may break, a **patch** may not; 1.x+: only a **major** may break). | `loft api-surface`, `api-compat.yml` | silent public-API removal/change (e.g. `shapes` dropping `clamp` is breaking → allowed only because 0.2→**0.3**) |
| **V5** | **Metadata** — `loft.toml` well-formed; `version` matches the tag; license present; package name == its registry namespace; every `dep` resolves in the index; no reserved / duplicate `(name, version)`. | `registry_validate.sh` | squatting, dangling deps, immutability violations (re-publishing an existing version) |
| **V6** | **Risk tier** (gates *how* it's trusted, not pass/fail) — a **pure-loft** lib with only declared capabilities is **auto-trustable**; a lib carrying `#rust`/`#native` external code, or requesting broad capabilities, **requires human review** before its *first* trusted release. | capability annotations, `#native`/`#rust` scan | arbitrary native code from an untrusted source |

**V6 is the one deliberate human gate, and it is a *policy* decision, not a
per-release step.** Own libs are already trusted (I wrote them — `#native` is fine).
An outside lib with native code is reviewed **once** at first admission; subsequent
additive versions re-run V1–V5 automatically. This is the same shape as @C96's trust
split: humans sign *policy* (admit this source), machines validate *releases*.

## Part 2 — The race-free ship transaction

Three invariants make concurrent lib landings safe:

1. **Published versions are immutable** ⇒ index edits are **append-only** ⇒ **commutative**
   (two disjoint new versions can't conflict — order is irrelevant).
2. **`index.json` has exactly one writer**: the ship transaction on a key-present
   machine. *Nothing else edits it.*
3. **Foreign submissions never touch `index.json`** — they land in a `submissions/`
   staging dir (tarball + printed entry + provenance), which the ship transaction
   drains. So a foreign PR and a maintain run can never conflict on the signed file.

The transaction (`loft ship`, i.e. `registry_maintain.sh` evolved):

```
loop:
  1. git fetch registry; let H = origin/main
  2. pending = { own libs whose loft.toml version > index }        (from tags/main)
             ∪ { validated entries in submissions/ }
  3. for each p in pending: run V1..V6; drop + report failures
  4. index' = index (unchanged, immutable) + append(passing pending); bump `updated`
  5. sign index' -> index'.sig with the local key; trust-gate the signature
  6. commit { index.json, index.json.sig, remove drained submissions } atomically
  7. git push origin main  EXPECTING remote == H     (compare-and-swap)
     - success -> done
     - rejected (remote moved) -> goto 1   (re-fold latest, re-sign, retry)
```

**Why it's race-free**: single signer ⇒ the signed index is always rebuilt from the
*latest* state by one process; appends commute + are immutable ⇒ nothing is lost or
reordered; the **CAS push** (step 7) serializes concurrent pushers at the git ref —
the loser simply re-folds and retries, so a lib that landed "at the same time" is
picked up on the next iteration, never dropped, and the index is never signed stale.
A workflow **concurrency-group lock** on the signer is a cheap first line; CAS is the
correctness backstop that also covers *two* key-present machines.

This also retires the **re-sign foot-gun** structurally: `index.json` and its `.sig`
are only ever written together, in step 6, by the one transaction — there is no path
that edits the index without re-signing.

## Part 3 — Key-gated trust tiers (C96), `tuxedo` default

- **Key present (this machine — the default):** the machine *is* the signer, so ship
  is fully autonomous — no touch, no prompt. `registry-sign.sh` **defaults to the file
  key** (`~/.loft/trust-root/registry-signing-key.bin`) when it is present and no
  YubiKey is configured (today it prompts / prefers YubiKey; the delta is making
  file-key-present the default on a trusted machine). `LOFT_REGISTRY_SIGNER=file` is
  the explicit override; on `tuxedo` it is the default.
- **Key absent (contributors, CI, other machines):** cannot sign — and must not fake
  it. It runs V1–V5 locally, then opens a **submission** (a PR adding to
  `submissions/`, *not* to `index.json`). A key-present ship run re-validates and signs
  it in. The trust-root never leaves a key holder.

## Part 4 — What you do about auto-mode: authorize specific commands

The signing is autonomous (your standing decision: on `tuxedo`, use the local key).
The **only** thing gating me is that Claude Code's auto-mode classifier blocks writes
to external repos (`git push`, `gh release`). Its own exception is **an explicit user
request naming the action** — so your part is a **one-line go-ahead naming the
commands below**, and I run them. Not a settings edit, not commands you run.

The exact commands I would run, in order (say e.g. *"run the ship steps"* to authorize
all, or name a subset):

```
# S1 — land the two verified fixes on their repos' main (fast-forward; source already pushed)
git -C <graphics-clone> push origin fix-c95-stdlib-clamp-collision:main       # shapes 0.3.0
git -C <game-clone>     push origin fix-c95-stdlib-floor_mod-collision:main    # time 0.2.1

# S2 — ship: package + tag + GitHub release + index append + re-sign (local key) + push
LOFT_REGISTRY_SIGNER=file scripts/registry_maintain.sh --yes
#   internally: gh release create <pkg>-v<ver> …   and   git push (registry main)

# S3 — verify the publish from a clean cache
cargo run --bin loft -- install shapes@0.3.0
cargo run --bin loft -- install time@0.2.1
```

`gh pr create` stays **out** of this list — opening PRs remains your explicit,
separate call every time. If you later want unattended CI publishing, the @C96
key-absent tier upgrades from "submission PR" to a scoped, revocable delegated key;
until then the above is the whole surface.

## Implementation delta (small, on existing scripts)

1. **DONE — `loft ship` verb** (`src/main.rs::run_ship_command`): locates
   `scripts/registry_maintain.sh`; on a key-present machine defaults the local file
   signer + `--yes` (autonomous), on a key-absent one runs review-only and points at
   the submission path. `LOFT_REGISTRY_SIGNER` / `--dry-run` / `--yes` respected as given.
2. **DONE — CAS-retry push** (`scripts/registry-sign.sh`): the sign+push now retries a
   lost race by rebasing the signed commit onto the fetched tip (clean when the
   concurrent change didn't touch `index.json`, so the signature stays valid), and
   aborts loudly on a real `index.json` conflict (two signers → the single-writer
   invariant is violated) rather than pushing a bad index.
3. **DONE — `submissions/` drain + inline gate** (`registry_maintain.sh`): the ship run
   peeks `submissions/` (a key-absent contributor stages `submissions/<name>-<ver>.json`
   via a PR that never touches `index.json`), then for each runs the **`vet-lib` gate** —
   a PASS is folded into the index + the staging file `git rm`ed (committed atomically
   with the index + `.sig` by the sign step); native-code NEEDS-REVIEW / FAIL is reported
   and left. Own libs also get the **V2/V3 parity gate** (`loft --interpret --tests`) in
   the pre-flight, so a lib a language change broke is excluded before it can be signed.
4. **DONE (adjacent) — reusable library CI** (`.github/workflows/library-ci-reusable.yml`
   + `scripts/deploy-library-ci.sh`) and the freeze-gate **`revalidate-libs.yml`**.
5. **DONE — docs**: [REGISTRY_SUBMIT.md](../../REGISTRY_SUBMIT.md) § "4 (recommended) — stage
   a `submissions/` file" documents the `submissions/<name>-<version>.json` format + the
   vet-and-fold flow.

## See also

- [PKG_REGISTRY.md](../../PKG_REGISTRY.md) · [REGISTRY_SUBMIT.md](../../REGISTRY_SUBMIT.md) · [COMPATIBILITY.md](../../COMPATIBILITY.md) · the publish runbook (loft-ship skill, `references/publish.md`)
- The trigger: adding stdlib `floor_mod` (@C94) broke shipped `shapes`/`time` (@C95) — the case for V3 + V4 + @C97.
