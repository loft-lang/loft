<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Registry bootstrap — first-time setup for `loft-lang/registry`

PKG.REG R3 + R3.5.  This is the procedure to bring the file-based
registry online.  Run by a loft maintainer when ready to publish
the first package.

Detailed design lives in [PKG_REGISTRY.md](PKG_REGISTRY.md); this
doc is the runbook.

---

## Two bootstrap paths

The registry needs a 32-byte Ed25519 secret signing key.  Where
that key *lives* is a separate question from getting the registry
running.  Two paths, deliberately:

| Path | When to use | Key storage | Public release impact |
|---|---|---|---|
| **Interim (K_tmp)** | Early ecosystem — exercising publish + install + signature verification end-to-end before hardware backup is ready.  Maintainer is the only consumer of signed indexes. | `~/.loft/trust-root/registry-signing-key.bin` on the trusted laptop only.  No hardware backup, no off-site copy. | **Do NOT ship a public loft release with K_tmp embedded.**  Only the maintainer's local build trusts it; blast radius is one machine. |
| **Final (K_real)** | First public release that ships an embedded trust-root key. | Full 3-2-1 backup (§ Step 1.5): laptop + 2 hardware tokens + sealed paper.  Off-site copies. | Loft releases embed `K_real` in `TRUSTED_PUBLIC_KEYS`; users worldwide trust signatures made by `K_real`. |

**Going from interim → final** is a [REGISTRY_RECOVERY.md
Scenario C](REGISTRY_RECOVERY.md#scenario-c--key-compromised-stolen--exfiltrated) key-rotation event:
generate `K_real`, embed in `TRUSTED_PUBLIC_KEYS` removing
`K_tmp`, re-sign every signed index/asset with `K_real`, ship the
public release.  That procedure is documented as "compromised key
response" but it's the same mechanic — and running it deliberately
as a planned rotation **dogfoods the recovery path** before you
need it in anger.

Steps 1, 2, 3, 4 below cover both paths identically.  Step 1.5
(long-haul storage) is the divergence point — interim skips it,
final mandates it.

---

## Prerequisites

- GitHub admin access to the `loft-lang` org.
- `ed25519-dalek` 2.x via `cargo install` or a small standalone
  script — same crate the loft binary uses.
- **For the final path**: with the three-key topology (§ Step 1.5) you
  do **not** need a dedicated air-gapped machine.  The two YubiKey keys
  are generated **on-card** (§ Step 1b) — the private key is born in the
  token's RNG and never touches a host, so an ordinary (even networked)
  laptop can drive it.  `K_laptop` is a deliberately *revocable* software
  key that lives on your encrypted dev laptop anyway.  The old "MUST be
  air-gapped" rule was for the single-key model, where that one key was
  irreplaceable; three independent, individually-revocable keys relax it
  to "generate the hardware keys on-card; treat the laptop key as
  revocable."  (Interim `K_tmp`: generate on the dev laptop — single-host
  by design.)

---

## Step 1 — Generate K_laptop (the software daily-signer)

`K_laptop` is the key you sign with day-to-day.  In the three-key
topology (§ Step 1.5) it lives on your encrypted dev laptop and is
revocable, so generate it **right there** — no air-gapped machine
needed; the high-assurance keys are the on-card YubiKey ones in
Step 1b.  (Interim `K_tmp`: same command, same place.)  Linux or
macOS — the keygen reads `/dev/urandom`:

```sh
# Build the keygen from the loft repo (ships with the binary as of
# the PKG.REG MVP).  The `registry` feature is on by default.
git clone https://github.com/loft-lang/loft.git
cd loft
cargo build --release --bin loft-keygen
./target/release/loft-keygen generate
```

Outputs in cwd:

- `registry-signing-key.bin` — 32-byte raw private key, chmod 600.
  **Never uploaded to any service.**  Stored:
  - On a hardware token (YubiKey, etc.) — preferred.
  - In an offline password manager backup.
  - One additional offline copy on a fresh USB stick locked in a
    safe.
- `registry-signing-key.pub` — 64-char hex public key.  Public;
  goes into source.

The same invocation prints to stdout (for copy/paste):

- The public key formatted as a Rust `[u8; 32]` literal — paste
  into `src/registry_keys.rs::TRUSTED_PUBLIC_KEYS` (Step 2).

### Step 1b — Generate the two YubiKey keys on-card (final path)

Step 1 produced **K_laptop**.  Now the two standby keys, generated
**on the tokens** so their private halves never exist on any host —
this is what removes the air-gap requirement, and why a lost YubiKey
is *revoked, not recovered* (you can't extract an on-card key to back
it up, and in this model you don't want to).

Needs a YubiKey whose **PIV applet does Ed25519 — firmware 5.7+**.
Check first (`ykman` = Yubico's CLI, `pip install yubikey-manager` or
your package manager):

```sh
ykman info          # firmware version — needs 5.7.0 or newer
ykman piv info      # PIV applet state
```

Generate on each token (PIV slot `9c` = "digital signature"),
exporting ONLY the public key (confirm flags against your
`ykman piv keys generate --help` — syntax shifts across versions):

```sh
# token A, then token B:
ykman piv keys generate --algorithm ED25519 9c K_yubiA.pub
ykman piv keys generate --algorithm ED25519 9c K_yubiB.pub
```

`ykman` writes the public key as **PEM**, but `loft-keygen format`
wants the raw 64-char **hex** — for Ed25519 the raw 32 bytes are the
DER SPKI's last 32 bytes, so convert then format:

```sh
openssl pkey -pubin -in K_yubiA.pub -outform DER | tail -c 32 \
  | xxd -p -c 64 > K_yubiA.hex
loft-keygen format --in K_yubiA.hex      # → the [u8; 32] literal for Step 2
```

Signing with an on-card key later is the external PKCS#11 /
`yubico-piv-tool` step (§ Step 1.5 "in-place hardware signing"); loft
`verify`s the raw 64-byte signature unchanged.

**If a token is older than 5.7** (no PIV Ed25519): either use it as
encrypted *backup storage* of a software key (`loft-keygen generate`
→ import to the token), or pause here — the OpenPGP applet's Ed25519
emits non-raw signatures loft won't accept as-is, so that route needs
a small loft-side adaptation first.

### Step 1.5 — Trust-root topology: three INDEPENDENT keys (final path)

**Final path only.**  For the interim path (K_tmp single-laptop),
skip to Step 2 — the daily working copy is the only copy by
design, and the eventual rotation to K_real retires K_tmp anyway.

`TRUSTED_PUBLIC_KEYS` is a *slice*: the client accepts a signature
from **any** embedded key.  Use that to make a lost device a
non-event — generate **three independent keypairs**, hold each on a
different device, and embed all three public keys:

| Key | Held on | Role |
|---|---|---|
| **K_laptop** | `~/.loft/trust-root/registry-signing-key.bin`, chmod 600, inside a FileVault/LUKS-encrypted home | Day-to-day signer (software, fast). |
| **K_yubiA** | YubiKey A (PIV slot), kept at home/office | Standby — promote it if K_laptop is lost/compromised. |
| **K_yubiB** | YubiKey B, kept at a *different* physical site | Standby — insurance against fire/theft at K_yubiA's site. |

**Why three distinct keys instead of three copies of one?**  Because
**revocation replaces recovery.**  Lose a YubiKey, or have the laptop
compromised → drop just that key's public entry from
`TRUSTED_PUBLIC_KEYS` in the next loft release; the other two keep
signing — no rotation, no re-signing, no user-visible break (contrast
[REGISTRY_RECOVERY.md Scenario C](REGISTRY_RECOVERY.md#scenario-c--key-compromised-stolen--exfiltrated),
the disruptive single-key rotation).  A lost independent key is
*revoked, not recovered*, so you do **not** need a 3-2-1 backup of
each one.  (Optional belt-and-suspenders: a sealed paper copy of
**K_laptop only** — base64, 44 chars — so a plain disk failure doesn't
force a YubiKey promotion.)

**How a YubiKey key actually signs — know this before relying on it.**
`loft-keygen` signs with a **software key file only** (`sign --key
<file.bin>`); it has **no** YubiKey / PIV / PKCS#11 driver (its sole
crypto dep is `ed25519-dalek`).  So K_yubiA/B reach a signature two
ways:

- **Backup storage (simplest).**  The PIV slot holds a *copy* of the
  32-byte key; to sign you re-import it to a machine and
  `loft-keygen sign`.  The token protects it at rest; signing is
  software.  Fine for standby keys you touch rarely.
- **In-place hardware signing (most secure, extra setup).**  The key
  is generated *on* the token and never leaves it; you sign with an
  **external** tool (`yubico-piv-tool` / a PKCS#11 module).  loft's
  `verify` is generic Ed25519, so it accepts the raw 64-byte signature
  against the embedded public key with **no loft-keygen change**.
  Requires a YubiKey whose PIV applet supports Ed25519 (firmware
  5.7+) — confirm your tokens first.

Day to day you sign with **K_laptop**.  The YubiKey keys are there so
that the day a key is lost or compromised, revocation is a one-line
release change — which is exactly when the one-time re-import (or
PKCS#11 setup) for a standby key is worth it.

**Do NOT put the key in**:

- Cloud storage (Dropbox, iCloud, Google Drive), even encrypted —
  signals to a third party that a high-value secret exists.
- Cloud-synced password managers (1Password / Bitwarden cloud
  vault) — same reason.  Local-only vaults are fine.
- A git repo (even private), an email to yourself, any chat app
  (those backups end up cloud-synced too).

**Do test the backup once a year.**  Pick a YubiKey or the paper
copy at random, restore to a clean test VM, sign a dummy
`index.json`, verify the signature with the public key.  See
[REGISTRY_RECOVERY.md § Annual recovery drill](REGISTRY_RECOVERY.md#annual-recovery-drill)
for the checklist.

If something goes wrong later (laptop dies, key compromised, key
silently corrupted), follow [REGISTRY_RECOVERY.md](REGISTRY_RECOVERY.md)
— it has step-by-step runbooks for every scenario plus the
multi-key rotation mechanism that keeps users from breaking.

---

## Step 2 — Embed the public keys in the loft binary

Edit `src/registry_keys.rs::TRUSTED_PUBLIC_KEYS` (this repo).  For the
three-key topology add **all three** public keys as 32-byte literals —
the client trusts a signature from any of them:

```rust
pub const TRUSTED_PUBLIC_KEYS: &[[u8; 32]] = &[
    // K_laptop
    [ 0x01, 0x23, /* ...32 bytes from registry-signing-key.pub... */ ],
    // K_yubiA
    [ /* ...32 bytes from K_yubiA.pub... */ ],
    // K_yubiB
    [ /* ...32 bytes from K_yubiB.pub... */ ],
];
```

(`loft-keygen generate` printed K_laptop's literal on stdout; re-emit
any `.pub` with `loft-keygen format --in <file>.pub`.)

Open a PR in the loft repo, get review, merge.  The next loft minor
release ships with all three embedded keys.  **Revoking** a key later
is the reverse: delete its entry, ship a release — the other two keep
working.

---

## Step 3 — Bootstrap the `loft-lang/registry` repo

1. Create `loft-lang/registry` on GitHub (public, MIT or CC0 — it's
   metadata).
2. Initial layout:

   ```text
   loft-lang/registry/
   ├── README.md                 (from doc/claude/registry_ci_template/registry_README.md)
   ├── index.json                (initial seed: {"schema_version": 1, "updated": "...", "packages": {}})
   ├── index.json.sig            (added with each merged PR — see step 5)
   ├── schema/
   │   └── index-v1.json         (JSON Schema; optional, for editor tooling)
   ├── tools/
   │   └── validate.py           (PR validation — schema lint + sha256 + reproducible-build)
   └── .github/
       └── workflows/
           └── pr-validate.yml   (runs validate.py on every PR)
   ```

3. Seed `index.json` from `doc/claude/registry_sample.json` in the
   loft repo (drop the `_comment` field; set `packages` to `{}`
   while no real package is ready yet).
4. Configure branch protection on `main`: required PR review,
   required CI passing, signed commits.
5. **Signing happens locally — not in CI.**  When a package
   author opens a PR adding a version row, the maintainer
   reviews + merges as usual, then signs the new `index.json`
   on their trusted laptop and commits the `.sig` file in a
   follow-up commit (or as part of the merge — see Step 4).
   No GitHub Secrets needed; the private key never leaves
   maintainer-controlled hardware.

   **Why no CI signing?**  CI signing would require storing
   the private key as a GitHub Actions secret — a third-party
   trust dependency we don't need.  Publishes are rare enough
   (weekly at most for an early ecosystem) that one manual
   command per merge is the right trade.  Detail in
   [PKG_REGISTRY.md § Why laptop signing](PKG_REGISTRY.md#index-signing--indexjsonsig).

---

## Step 4 — First publish

Per PKG_REGISTRY.md § Publishing flow.

**Package author side**:

1. `cd loft-crypto && git tag v0.1.0 && git push --tags`
2. `loft package`  (produces `crypto-0.1.0.tar.gz` + sha256 + size).
3. `gh release create v0.1.0 crypto-0.1.0.tar.gz`
4. Open PR against `loft-lang/registry`:

   ```diff
    "packages": {
   -  }
   +  "crypto": {
   +    "description": "SHA-256, HMAC, base64.",
   +    "homepage": "https://github.com/loft-lang/loft-crypto",
   +    "categories": ["crypto"],
   +    "yanked": [],
   +    "versions": {
   +      "0.1.0": {
   +        "url": "https://github.com/loft-lang/loft-crypto/releases/download/v0.1.0/crypto-0.1.0.tar.gz",
   +        "sha256": "43ebf109b0206bd00cab03209b1da081ba9f5caa416aa077ccb1dda65da67cfa",
   +        "size": 5717,
   +        "loft": ">=0.8",
   +        "deps": {},
   +        "published": "2026-05-24T08:00:00Z"
   +      }
   +    }
   +  }
    }
   ```

5. CI runs `validate.py` against the PR: schema lint, sha256 verify
   (downloads the release tarball, hashes it, compares), and the
   reproducible-build re-check (clones the source, runs
   `loft package`, compares).

**Maintainer side** (after CI passes):

6. **Merge the package PR** on GitHub once you're happy (CI's
   `pr-validate.yml` already checked sha256/size; your eyeball on the
   release is the human trust root).

7. **Review + sign + publish the signature in one command —
   `scripts/registry-sign.sh`:**

   ```sh
   scripts/registry-sign.sh
   ```

   It clones the merged registry, shows the `index.json` diff and — for
   every added/changed version — the LIBRARY RELEASE it points at (repo
   + tag + notes), **downloads each tarball to confirm its sha256
   matches**, then on your `y` signs `index.json`, commits
   `index.json.sig`, pushes, and deletes the temp clone.  It REFUSES to
   sign on any sha256 mismatch, invalid JSON, or a "no" — *don't sign
   what you haven't looked at.*

   Flags: `--pr N` (preview/sign a PR before merging), `--notes` (full
   release bodies), `--no-push` (commit locally only), `--key` /
   `--registry-dir`.  The underlying primitive, if ever needed directly:

   ```sh
   loft-keygen sign --in index.json \
       --key ~/.loft/trust-root/registry-signing-key.bin --out index.json.sig
   ```

Now `loft install crypto` works for everyone.

**Why is signing maintainer-local?**  Two reasons:

- The private key never leaves your trusted laptop.  No GitHub
  Secret to leak, no third-party trust dependency.
- A human reviewing + signing the merged commit IS the audit
  trail.  Versus CI signing, where "the key signs whatever's
  in main" — no human gate.

Cost: ~30 seconds per merge.  For an ecosystem with weekly
publishes, that's negligible.

---

## Step 5 — First key-rotation drill (R10.5)

Before a key is lost/compromised for real, run a drill.  Signing
never runs in CI (see Step 3 § "Why no CI signing" above) — there is
no `sign-and-commit.yml` and no GitHub secret to touch; the model is
`src/registry_keys.rs::TRUSTED_PUBLIC_KEYS` holding **N independent
keys** verified with OR semantics, and re-signs always run locally
via `scripts/registry-sign.sh`:

1. Generate a new keypair (Step 1).
2. Embed the new public key **alongside** the existing ones in
   `TRUSTED_PUBLIC_KEYS` (a one-line slice append, per
   [PKG_REGISTRY.md § Multi-maintainer support](PKG_REGISTRY.md#multi-maintainer-support)).
   Ship a loft release.
3. Re-sign `index.json` locally with the new key:
   `scripts/registry-sign.sh --key <new-key>.bin`.  It stages
   `index.json` + `index.json.sig` together, then trust-gates the
   result against the just-updated `TRUSTED_PUBLIC_KEYS` before
   pushing — a wrong/untrusted key refuses to push rather than
   shipping a signature every `loft install` would reject.
4. If retiring an old key (not just adding a new signer), wait for
   ecosystem adoption of the loft release that trusts the new key
   before dropping the old one — 3 months is a safe window.
5. **Retire the drill's throwaway key by revocation**: delete its
   entry from `TRUSTED_PUBLIC_KEYS` and ship a release.  This is
   revocation, not rotation — the *other* keys were already signing
   independently, so removing one doesn't require an immediate
   re-sign; the next ordinary publish carries the updated trust set.
6. Old private key: securely destroyed (overwrite + physical
   destruction of any backup media).

Document any issues that emerged during the drill in this file under
"Lessons learned" so the real rotation (when needed) is smoother.

---

## Trust-root recovery

If the private key is lost or compromised:

See [REGISTRY_RECOVERY.md](REGISTRY_RECOVERY.md) for the full
step-by-step runbook covering:

- **Scenario A** — laptop died, backup recoverable.  ~30 min
  drill; no user-visible impact.
- **Scenario B** — laptop died, no recoverable backup (catastrophic
  loss of all working copies).  Forces a multi-key rotation;
  6-month transition window; still zero user-visible impact if
  handled before users hit `loft update`.
- **Scenario C** — key is COMPROMISED (exfiltrated / stolen).
  Same-day distrust; emergency point release with the bad key
  removed; communicate aggressively.

Quick summary so the bootstrap doc reads complete on its own:

* **Lost (no compromise):** generate a new key, embed alongside
  the old one, ship a loft minor release.  Old key continues
  signing indexes during the transition window so existing
  installs keep working.  Drop the old key in a subsequent
  release.
* **Compromised:** generate a new key.  Emergency loft point
  release embedding ONLY the new key — the compromised key is
  distrusted immediately.  Existing `loft.lock` pins keep working
  (the lockfile records each tarball's sha256, which the
  compromised key can't forge); only fresh installs need the
  binary upgrade.  Communicate via every loft user-facing channel
  + a CVE if appropriate.

The architectural reason both scenarios are recoverable: the
client verifies BOTH the index signature AND each tarball's
sha256.  `loft.lock` records the sha256 — once a build is
locked, no signing-key event can corrupt it.  Only NEW installs
and `loft update` consult the index, and those are gated on the
current signature.

---

## What this doc replaces

Before this MVP, the registry was an "open work" item with no
concrete bootstrap path.  This runbook closes the gap: a maintainer
with admin access can stand the registry up in an afternoon, ship a
loft release with the embedded key, and be ready for the first
publish.
