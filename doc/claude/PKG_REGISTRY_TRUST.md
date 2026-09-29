
# Registry trust — signing, reproducibility, nightly validation

How a published package is trusted: the signed index, reproducible builds, and validation against `loft@main`.  Split out of [PKG_REGISTRY.md](PKG_REGISTRY.md).

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
  [REGISTRY_BOOTSTRAP.md § Step 1.5](REGISTRY_BOOTSTRAP.md#step-15--trust-root-topology-three-independent-keys-final-path).
  First public loft release embeds `K_real` in
  `TRUSTED_PUBLIC_KEYS`.

**Going from interim → final** is mechanically identical to the
"Compromised key" path: generate `K_real`, embed it in
`TRUSTED_PUBLIC_KEYS` removing `K_tmp`, re-sign every signed
index/asset with `K_real`, ship the public loft release.  The
recovery runbook for this is
[REGISTRY_RECOVERY.md § Scenario C](REGISTRY_RECOVERY.md#scenario-c--key-compromised-stolen--exfiltrated).

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
| `apt-get update` (separate index refresh) | Cached index with 1h TTL + `--refresh` flag | [§ Local cache layout](PKG_REGISTRY.md#local-cache-layout) |
| Mirrors via `sources.list` | `LOFT_REGISTRY_URL` env var | [§ Decoupled lifecycle](PKG_REGISTRY.md#decoupled-lifecycle--debian-style-the-registry-is-a-repo) |

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
