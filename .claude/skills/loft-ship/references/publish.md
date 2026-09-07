# The registry publish runbook

Read this before submitting a library to the registry. Full reference:
[REGISTRY_SUBMIT.md](../../../../doc/claude/REGISTRY_SUBMIT.md) (author-facing 5-step flow),
[PKG_REGISTRY.md](../../../../doc/claude/PKG_REGISTRY.md) (the file-based registry design),
[REGISTRY_RECOVERY.md](../../../../doc/claude/REGISTRY_RECOVERY.md) (trust-root incidents).

## How the registry works (the one fact that explains the foot-gun)

The registry (`loft-lang/registry`) is a static **`index.json`** plus a detached signature
**`index.json.sig`** (Ed25519, 64-byte raw), signed by the maintainer trust-root key
(`~/.loft/trust-root/registry-signing-key.bin` / `.pub`). `loft install` downloads `index.json`
(via the raw-GitHub CDN), **verifies the signature**, then fetches the package and checks its
**sha256 + size** against the index entry. So the index and its signature must always move
together — an index the signature doesn't match makes the whole registry unusable.

## The routine (preferred) — `loft package` → tag → release → publish → touch-sign

Don't hand-run the steps below unless you're debugging — the correctness-critical parts (the
index `sha256`/`size` matching the release tarball, the **atomic re-sign**, the fixture pin) are
exactly where a hand publish goes wrong. Use the routines:

- **Cut the GitHub release** (the easy half, author-triggered): bump `version` in `loft.toml`,
  merge to `main`, then push tag `<pkg>-v<version>`. A release-on-tag workflow (or `release.sh`
  locally) runs `loft package` + `gh release create` + uploads the tarball. *The tag is the only
  thing you do by hand — everything correctness-sensitive is downstream.*
- **Publish + sign** (the hard half, runs on the machine that holds the key):
  - own libs, all at once: `scripts/registry_maintain.sh` — publishes every lib that is
    missing/newer than the index, then signs (it signs **through** `registry-sign.sh`).
  - one registry PR: `scripts/registry-sign.sh --pr N` — shows the diff, re-checks every
    tarball `sha256`, then signs.

**The signing gate (why the key never enters CI).** A fully-automatic "push → signed release" is
impossible: the trust-root key must never leave the maintainer's machine (a wrong/stale signature
breaks *every* `loft install` — see the foot-gun below), so signing is **inherently local and
human-gated**. The default path is **on-card**: a YubiKey holds the Ed25519 key in PIV slot 9C
and signs over PKCS#11 (`pkcs11-tool --mechanism EDDSA`) — the private key never leaves the card,
and PIN + touch ARE the confirmation (no typed `yes`). The wait is **unbounded by default**
(`LOFT_YUBIKEY_TIMEOUT=<sec>` to bound it); the module is auto-found or set via
`LOFT_YUBIKEY_PKCS11_MODULE`, the PIV id defaults to `02` (`LOFT_YUBIKEY_PIV_ID`), a PIN can be
pre-supplied via `LOFT_YUBIKEY_PIN`, and the whole step can be replaced with
`LOFT_YUBIKEY_SIGN_CMD`. If no card is available, signing **falls back to the local key file**
(`--key`, default `~/.loft/trust-root/registry-signing-key.bin`) behind a **typed `yes`** prompt
(`LOFT_REGISTRY_SIGNER=file` forces this path; `--yubikey` disables the fallback). Either way, a
**trust gate** then verifies the new signature against
`src/registry_keys.rs::TRUSTED_PUBLIC_KEYS` before anything is committed or pushed — a signature
from a key not on that list is refused, fail-safe. (Reading an idle YubiKey touch's typed
one-time-password as the confirmation "yes" is the exact hazard this on-card design avoids —
see `registry_maintain.sh`'s note above its sign step — not a feature of it.) `--yes` skips the
local-key prompt for scripted use; the trust gate still runs regardless. So the whole release is:
**you push a tag, then you touch your key once** (or type `yes` if no card is present) — the
routine gets everything between right.

## The steps — what the routine does under the hood (and the manual fallback)

The registry package is a **source tarball** — `loft package` does **not** build per-target
artifacts (no prebuilt cdylib). Consumers build the native/wasm artifacts at install time
against their own loft, so cross-tree `loft-ffi` matching is a *consumer-side* concern, **not a
publish blocker**. (It does mean a `#native` feature only works on `--native` once the
consumer's loft itself carries the needed compiler support.)

1. **Land the code first.** The lib's PR must be merged to its repo's `main` (the package is
   built from `main`), with the new `version` in `loft.toml`.
2. **`loft package`** (in the package dir) → writes `<pkg>-<version>.tar.gz` and **prints the
   index entry** (`url`, `sha256`, `size`, `loft`). It **omits `subpath` and `deps`** — copy
   those from the package's existing entries (e.g. `"subpath": "crypto"`, `"deps": {}`).
3. **GitHub release + asset** (the tag must match the entry's `url`):
   ```
   gh release create <pkg>-v<version> --repo <org>/<repo> --target main \
     <pkg>-<version>.tar.gz --title "<pkg> v<version>" --notes "…"
   ```
4. **Registry update** — the destination depends on who's publishing:
   - **Own libs** (the maintainer, who holds the signing key): `registry_maintain.sh` clones
     `loft-lang/registry` on its default branch, adds the version under
     `packages.<pkg>.versions.<version>` (the printed entry **plus** `subpath` + `deps` + a
     `published` ISO-8601 UTC timestamp), bumps the top-level `updated`, then hands off to
     `registry-sign.sh`, which stages `index.json` + `index.json.sig` **together**, signs, and
     **commits + pushes DIRECTLY to `main` — no PR**.
   - **Foreign submissions** (an author without signing access): the current route is a
     **staging file** `submissions/<name>-<version>.json` in a PR against
     `loft-lang/registry` — it never touches `index.json`; the maintainer run vets it
     (`scripts/vet-lib.sh`) and folds + re-signs atomically (REGISTRY_SUBMIT.md § 4; the
     old direct-index-edit PR still works but is being superseded). Preview your entry
     first with `loft publish --dry-run`; the maintainer-side wrapper for the whole
     fold-and-sign run is `loft ship`.
   Either way, editing the JSON:
   - **Match the index's CURRENT unicode convention** when editing with a script: check whether
     descriptions carry raw `—` or escaped `\uXXXX` and pass the matching `ensure_ascii` to
     `json.dump`, then verify with `git diff` that ONLY your entry (+ `updated`) changed.  The
     convention has flipped once (a hard "always ensure_ascii=True" rule here rewrote every
     description line against a raw-unicode index) — the diff check is the invariant, not the flag.
     Safer still for a small edit: substitute the STRINGS in place and never re-serialise, so the
     formatting, key order and convention cannot move at all.
   - **Re-sign, then verify** (the maintainer key; this is the step that breaks all installs if
     skipped — see below). The routine above (`registry-sign.sh`) wraps these two with the
     signing-gate confirmation described above; the raw commands are:
     ```
     loft-keygen sign   --in index.json --key ~/.loft/trust-root/registry-signing-key.bin --out index.json.sig
     loft-keygen verify --in index.json --sig index.json.sig --pub "$(cat ~/.loft/trust-root/registry-signing-key.pub)"
     ```
   - **Commit `index.json` + `index.json.sig` TOGETHER** (one atomic change) — direct to `main`
     for an own lib, or as part of the PR branch for a foreign submission.

## The re-sign foot-gun (internalize this)

Editing `index.json` **without** regenerating `index.json.sig` leaves a valid-looking index
with a **stale signature**. Every `loft install` then fails signature verification — *all*
packages, not just the new one — every `loft install` fails with **"registry index
signature INVALID"** (verification runs before anything else in `src/install.rs`). This happened in @PLN84 (a `crypto` version was
merged into the index un-re-signed and broke installs until a re-sign PR fixed it). So: **every
change to `index.json` is followed immediately by a re-sign.** Treat them as one atomic edit.

## The first-`description` gotcha (a publish that succeeds and still loses data)

This one is **by design**, which is why nothing reports it.  `registry_maintain.sh` treats
a `[package] description` in `loft.toml` as authoritative and *refreshes it on every
publish*, so that correcting the manifest propagates to the catalogue.  A README-scraped
fallback deliberately does the opposite — it only seeds a brand-new package and never
clobbers an existing entry.

The hazard sits exactly in that gap: a package registered **without** a manifest
description still has one in the index, seeded from its README on first publish or written
by hand.  The day you add the field to `loft.toml`, the authoritative path takes over and
**replaces** that text.

Nothing flags it.  The publish exits 0, every sha256 matches, the signature verifies, the
coverage check is clean and the pinned installs work — none of them look at the
description.  It surfaces only in a before/after diff of `index.json`.

It cost four entries on one run: `cbor` lost its canonical-ordering / byte-identical /
native==wasm guarantees, `server` and `web` lost the crates backing them, and `ssh` lost
**"Native-only"** — the one fact a consumer needs before choosing it.

**So: before adding a `description` for the first time, read what the index already says**

```
curl -fsSL https://raw.githubusercontent.com/loft-lang/registry/main/index.json \
  | jq -r '.packages.<pkg>.description'
```

and merge rather than replace.  The existing text tends to say what the library GUARANTEES
or how it is built; a fresh one tends to say what it is FOR.  A catalogue line should carry
both.

Not every old line is worth keeping — check it against the code before preserving it.  Two
of the six on that run were wrong or empty (`game_protocol` advertised "ack/retransmit" it
has no code for; `zttext` said "loft library zttext"), and replacing those was the fix.

(This is the maintainer path only.  A foreign `submissions/` entry's `description` is
ignored when the package already exists — see REGISTRY_SUBMIT.md — so the hazard does not
arise there.)

## The CDN-staleness gotcha (don't misread it as a failed publish)

`loft install` reads `index.json` through the raw-GitHub CDN and keeps its own local
cache under `~/.loft/registry/` with roughly a **1-hour TTL** (the TTL is loft's, in
`src/install.rs` — the CDN adds its own shorter propagation on top). Right after a merge, `@latest` may still resolve to
the previous version at some edges. To verify a fresh publish, **install the exact version**
(`loft install <lib>@<version>`) rather than `@latest`; if the pinned version installs and
verifies, the publish succeeded — `@latest` will catch up as the CDN propagates. Don't conclude
the publish failed from a stale edge read.

## Verification before you call it shipped

Every item below is something the publish itself will NOT tell you.  It exits 0, matches
every sha256, verifies the signature and reports zero findings whether or not these hold —
so "the publish succeeded" is not one of the checks.

- `loft-keygen verify` passes on the re-signed index (signature matches).
- `index.json` and `index.json.sig` are in **one commit** (`git show --stat`) — a split
  leaves a valid-looking index with a stale signature and breaks *every* install.
- **Diff the index against the previous commit and read what changed.**  Confirm no
  package or version disappeared and no description was rewritten:

  ```
  git diff <prev>..<new> -- index.json | grep '^-' | grep -v '^---'
  ```

  A large deletion count is usually harmless key reordering, but it hides real loss —
  compare the parsed package/version SETS, not the diff text.
- `loft install <lib>@<version>` succeeds from a clean cache (sha256 + size check pass).
  Pin the exact version; `@latest` can read a stale CDN edge.
- **A concurrent publish survived.**  The registry has real concurrent writers (two of
  three consecutive runs hit one).  The signer rebases and re-signs, or refuses — but
  confirm the other party's entry is still in the index afterwards.
- The parity gate (in SKILL.md) is green on every target the entry claims.
