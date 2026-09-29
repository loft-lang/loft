<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Registry trust-root recovery runbook

When the signing key for `loft-lang/registry` is lost, corrupted,
or compromised, **follow the appropriate scenario below**.  Each
section is a checklist designed to be executed under stress — no
improvisation required.

The companion doc [REGISTRY_BOOTSTRAP.md](REGISTRY_BOOTSTRAP.md)
covers the initial setup; THIS doc covers everything that comes
after.

---

## Pre-flight — verify your safety net works

Run [§ Annual recovery drill](#annual-recovery-drill) **before**
you need to.  The first time you actually use a backup should not
be at 2am when your laptop just bricked.

Specifically, confirm:

- You can locate every backup copy without consulting a stranger.
  (Family members know where the YubiKey-in-the-safe is, the
  paper backup's location, etc.)
- The YubiKey PIV slot still holds the key.
- The paper backup is readable (no smudges / fade).
- You remember the YubiKey PIN.
- The base64 on the paper actually decodes to the right 32 bytes.

---

## Architectural backstop — why all scenarios are recoverable

Before the runbooks: **two layers of verification** protect users:

1. **Index signature**: `index.json.sig` proves the index hasn't
   been tampered with.  Verified by the loft client against
   `TRUSTED_PUBLIC_KEYS`.
2. **Per-tarball sha256**: every install records the tarball's
   sha256 in `loft.lock`.  Subsequent builds verify the sha256
   independently of the index signature.

**The lockfile is the firewall.**  Once a user has installed a
package, their build is reproducible forever — no signing-key
event (loss, compromise, key rotation) can corrupt the bytes
they already pinned.  Only **new** installs and `loft update`
consult the current index, and those are gated on the current
signature.

This means:
- Scenario A/B (lost key): existing installs keep working;
  worst case is a delay in shipping new versions.
- Scenario C (compromised): existing installs keep working; only
  fresh-install / update users need to upgrade the loft binary.

---

## Scenario A — Laptop died, backup recoverable

**Symptoms**: working machine is gone (hardware failure, theft,
upgrade) but at least one backup (YubiKey or paper) is intact.

**Impact on users**: zero.  No public signal needed.

**Drill**: should complete in 30-60 minutes.

### Checklist

1. [ ] Acquire a clean replacement machine (Linux or macOS).
2. [ ] Verify the machine is uncompromised — fresh OS install,
       full-disk encryption enabled, no unverified software.
3. [ ] Retrieve a backup copy (YubiKey A by preference; paper
       fallback if both YubiKeys are gone).
4. [ ] Re-import the key:

   **From YubiKey (PIV slot):**
   ```sh
   # The exact command depends on how you stored it — yubico-piv-tool
   # or ykman.  Example with yubico-piv-tool:
   yubico-piv-tool --action read-object \
                   --object-id <your-slot-id> \
                   --output registry-signing-key.bin
   chmod 600 registry-signing-key.bin
   ```

   **From paper base64:**
   ```sh
   # Transcribe the 44-char base64 (carefully — one typo and verification fails).
   echo "<paste-base64-here>" | base64 -d > registry-signing-key.bin
   chmod 600 registry-signing-key.bin
   stat -c '%s' registry-signing-key.bin  # must be 32
   ```

5. [ ] **Verify the restored key matches the embedded public key.**
       This catches transcription errors before anything else
       depends on the key:

   ```sh
   # Build the loft-keygen helper from the loft source tree.
   cargo build --release --bin loft-keygen
   # Sign a known test message with the restored private key, then
   # verify it against the matching public key embedded in
   # src/registry_keys.rs::TRUSTED_PUBLIC_KEYS with `loft-keygen verify`.
   echo -n "test message" > /tmp/test.bin
   ./target/release/loft-keygen sign \
       --in  /tmp/test.bin \
       --key registry-signing-key.bin \
       --out /tmp/test.bin.sig
   ./target/release/loft-keygen verify \
       --in  /tmp/test.bin \
       --sig /tmp/test.bin.sig \
       --pub "<hex for this key, from src/registry_keys.rs>"
   ```

   Expect `OK — signature valid`.

6. [ ] Move `registry-signing-key.bin` into your new daily-use
       location (e.g. `~/.loft/trust-root/`, chmod 600, inside
       encrypted home).
7. [ ] If you used the paper backup, generate fresh hardware-token
       copies on the new machine — Scenario A's backup chain is
       now down a copy and you should restore the 3-2-1 rule
       before the next failure.
8. [ ] Re-test the end-to-end signing flow from the new machine:

   ```sh
   echo "drill payload" > /tmp/drill.bin
   loft-keygen sign \
       --in  /tmp/drill.bin \
       --key ~/.loft/trust-root/registry-signing-key.bin \
       --out /tmp/drill.bin.sig
   loft-keygen verify \
       --in     /tmp/drill.bin \
       --sig    /tmp/drill.bin.sig \
       --pub-file ~/.loft/trust-root/registry-signing-key.pub
   ```

   Expect "OK — signature valid".  Confirms the restored key
   signs + the corresponding public still verifies.

9. [ ] Done — no rotation needed.  The public key in client
       binaries is unchanged.  Existing signed `index.json.sig`
       files remain valid; future merges sign on this machine
       instead of the previous one.

---

## Scenario B — Laptop died AND backups unrecoverable

**Symptoms**: all working copies of the private key are gone.
No way to sign new `index.json` versions.  Existing signed
indexes remain valid.

This is the **worst non-compromise case**.  The procedure is a
**graceful key rotation** with a transition window.

**Impact on users**: zero, if handled within ~6 months.  Beyond
that, users running stale loft binaries may see "signature
invalid" on fresh installs until they upgrade.

### Checklist

1. [ ] Don't panic.  The compromised-key emergency procedure
       (Scenario C) is NOT needed — no attacker has the key.
2. [ ] **Generate a new keypair** on a clean machine:

   ```sh
   cargo build --release --bin loft-keygen --features registry
   ./target/release/loft-keygen generate
   ```

   Same hygiene as the original bootstrap: chmod 600 the .bin,
   back up to TWO hardware tokens and a paper sealed copy
   before doing anything else.

3. [ ] **Embed the new public key in `src/registry_keys.rs::TRUSTED_PUBLIC_KEYS`**:

   ```rust
   pub const TRUSTED_PUBLIC_KEYS: &[[u8; 32]] = &[
       // Existing original key — keep here.
       [ /* ... */ ],
       // NEW key (added 2026-MM-DD; rotation from lost-backup
       // scenario).
       [ /* paste new key here */ ],
   ];
   ```

   **Important**: the old key STAYS.  The slice supports any
   number of trusted keys; verification is OR across all
   entries.  Existing signed `index.json.sig` files remain
   valid throughout the transition.

4. [ ] **Ship a loft minor release** with the updated
       `TRUSTED_PUBLIC_KEYS`.  Tag as e.g. `v0.8.7`.  Document
       in `CHANGELOG.md`:

   > **Trust-root rotation** (2026-MM-DD): added a second
   > registry signing key alongside the original.  No user
   > action required — the loft client accepts indexes signed
   > by either key during the transition.  See
   > REGISTRY_RECOVERY.md § Scenario B for context.

5. [ ] **Re-sign `index.json` locally with the new key**, via
       `scripts/registry-sign.sh` — NOT a raw `loft-keygen sign` +
       `git add index.json.sig` (that stages only the `.sig` and
       skips the trust-gate; #377 was exactly this failure mode:
       an uncommitted/changed `index.json` sat alongside a
       committed `.sig` that verified against content that never
       landed):

   ```sh
   scripts/registry-sign.sh --registry-dir loft-lang/registry \
       --key <new-key>.bin \
       --message "sign: re-sign index.json with new trust root (Scenario B rotation)"
   ```

   `registry-sign.sh` shows the diff, re-checks each tarball's
   sha256, signs, then verifies the new signature against
   `src/registry_keys.rs::TRUSTED_PUBLIC_KEYS` (refusing to
   commit/push if it doesn't match a trusted key), stages
   `index.json` + `index.json.sig` **together** in one commit, and
   pushes.  Clients fetching the new `.sig` verify it against
   either the old or the new key in `TRUSTED_PUBLIC_KEYS` — both
   pass during the transition.

   > **Note — the current trust root is multiple independent keys,
   > not one.**  Since the 2026-06-14 bootstrap,
   > `TRUSTED_PUBLIC_KEYS` holds 4 independent signers (2 software
   > laptop keys + 2 on-card YubiKeys; see PKG_REGISTRY.md § R3.5),
   > not a single rotating trust-root key.  When only ONE signer's
   > key is lost (this scenario) and the others are intact, the
   > simpler fix is usually **revocation**: delete that key's entry
   > from `TRUSTED_PUBLIC_KEYS` and ship a release — the surviving
   > keys keep signing, so there is no add-new-key / wait / drop-old
   > transition to run.  The rotation below is still the right shape
   > when you want to add a brand-new signer, or when every existing
   > key needs replacing at once.
7. [ ] **Wait for ecosystem adoption.**  Six months is a safe
       transition window.  Watch `loft --version` distribution
       via your own analytics if available; otherwise just
       calendar a check-in.
8. [ ] **Drop the old key** in a subsequent loft release
       (e.g. `v0.9.0`).  Document the removal in `CHANGELOG.md`.
       The old `index.json.sig` files signed by the old key
       become un-verifiable from this release forward — but
       any new sign by the registry CI now uses only the new
       key, so this only affects users who haven't re-fetched
       the index since the rotation (rare after 6 months).

### What goes wrong if you DON'T add the new key alongside the old

Tempting shortcut: just swap the key.  Don't.  Reason: every
loft binary currently in the wild has the OLD public key
embedded.  If `index.json.sig` is suddenly signed by the new
key with no overlap window:

- Users get "signature invalid" on `loft install`.
- Users must upgrade loft binary IMMEDIATELY to recover.
- The ecosystem hates you.

The two-key transition costs one extra loft release and a
6-month wait, in exchange for zero user impact.  Always pay it.

---

## Scenario C — Key compromised (stolen / exfiltrated)

**Symptoms**: the private key is suspected to be in adversary
hands.  Sources of suspicion:

- Laptop was stolen (and you can't be SURE the disk encryption
  held).
- Forensic evidence of malware that could exfiltrate `~/.loft/`.
- A backup copy went missing from a previously-secure location.
- A backup copy was accidentally pushed to a public location
  (e.g. a paste, a screen-share recording, a cloud sync).

**Also use this procedure for planned key rotations** —
notably the **interim `K_tmp` → permanent `K_real`** transition
documented in [REGISTRY_BOOTSTRAP.md § Two bootstrap paths](REGISTRY_BOOTSTRAP.md#two-bootstrap-paths)
and [PKG_REGISTRY.md § Two-stage bootstrap](PKG_REGISTRY_TRUST.md#two-stage-bootstrap--interim-k_tmp--permanent-k_real).
Same mechanic (embed new key, remove old, re-sign artefacts,
ship release), different urgency.  Running the procedure as a
planned rotation dogfoods the recovery path before it's needed
in anger — skip the "emergency loft patch release" + CVE
communication steps below, treat the rotation as a normal
minor release.

**Impact**: an attacker can sign forged `index.json` files.
They can NOT forge tarballs already in user `loft.lock`s
(per-tarball sha256 is the firewall).  But they could redirect
new installs to malicious tarballs.

**Treat this as a security incident.**  Aggressive timeline
(hours, not weeks).

### Checklist — immediate (within 1 hour of suspicion)

1. [ ] **Stop using the compromised key for ANY signing.**
       Move `~/.loft/trust-root/registry-signing-key.bin` to
       `~/.loft/trust-root/COMPROMISED-<date>.bin` (don't
       delete yet — you may need it for forensics).  Now the
       key isn't on the daily path.
2. [ ] **Triage**: how confident are you that the key is
       compromised?
   - HIGH confidence (key seen in logs, laptop stolen and FDE
     status unknown): proceed with Scenario C.
   - LOW confidence (laptop stolen but FDE was working, no
     evidence of access): consider treating as Scenario B
     (rotation, but not emergency).  The cost difference is
     ~1 loft minor release vs ~1 loft patch release.
3. [ ] **Generate a new keypair** on a CLEAN machine (don't
       use the suspected-compromised machine even for
       generation — it could have a keylogger).

   ```sh
   cargo build --release --bin loft-keygen --features registry
   ./target/release/loft-keygen generate
   ```

   Storage hygiene per [REGISTRY_BOOTSTRAP.md § Step 1.5](REGISTRY_BOOTSTRAP.md#step-15--trust-root-topology-three-independent-keys-final-path).

### Checklist — same day

4. [ ] **Embed the new public key, REMOVE the old one** in
       `src/registry_keys.rs::TRUSTED_PUBLIC_KEYS`.  Unlike
       Scenario B, the old key is **gone immediately**:

   ```rust
   pub const TRUSTED_PUBLIC_KEYS: &[[u8; 32]] = &[
       // OLD key REMOVED 2026-MM-DD — see CVE-YYYY-NNNN.
       // NEW key only.
       [ /* paste new key here */ ],
   ];
   ```

5. [ ] **Ship an emergency loft patch release** with this
       change.  Bump only the patch version (e.g. `v0.8.5` →
       `v0.8.5.1` or `v0.8.6`).  Document loudly in
       `CHANGELOG.md`:

   > **SECURITY: emergency trust-root rotation** (CVE-YYYY-NNNN).
   > The previous registry signing key was compromised.  This
   > release REMOVES that key and adds a new one.  Users on
   > previous versions: upgrade IMMEDIATELY before running
   > `loft install` or `loft update`.  Existing `loft.lock`-
   > pinned builds remain safe — the lockfile records each
   > tarball's sha256, which the compromised key cannot forge.

6. [ ] **Re-sign `index.json` locally with the new key**, via
       `scripts/registry-sign.sh` — NOT a raw `loft-keygen sign` +
       `git add index.json.sig` (that skips the trust-gate and can
       leave `index.json` uncommitted while a signed `.sig` ships,
       see #377):

   ```sh
   scripts/registry-sign.sh --registry-dir loft-lang/registry \
       --key <new-key>.bin \
       --message "sign: EMERGENCY re-sign with new trust root (CVE-YYYY-NNNN)"
   ```

   `registry-sign.sh` signs, verifies the result against
   `src/registry_keys.rs::TRUSTED_PUBLIC_KEYS` (refusing to push
   otherwise), stages `index.json` + `index.json.sig` together, and
   pushes.  The new `.sig` file invalidates anything signed by the
   compromised key.  Clients on the emergency loft patch release
   verify against ONLY the new key, so any forged index from the
   attacker (signed by the compromised key) is rejected.

   > **Note — independent-key model.**  With `TRUSTED_PUBLIC_KEYS`
   > holding 4 independent signers (PKG_REGISTRY.md § R3.5), a
   > single compromised key can be handled as pure **revocation** —
   > drop that entry, ship a release, done — without necessarily
   > minting a replacement in the same emergency release; add a new
   > signer separately once you've re-established a clean device.
8. [ ] **Audit `index.json` history** in the registry repo:

   ```sh
   cd loft-lang/registry
   git log --since="<compromise window start>" --until=now \
           --pretty=format:"%h %ai %s" \
           index.json
   ```

   For every commit in the window:
   - Confirm the commit author + reviewer match expectations.
   - Re-download each tarball URL added in that window.
   - Verify the sha256 matches what the index claims.
   - Verify the tarball matches the repo source (run the
     reproducible-build re-check from
     `tools/validate.py` locally).
   - Any suspicious row → yank immediately.

9. [ ] **Communicate**:
   - Banner at the top of `loft-lang/loft`'s README.
   - GitHub Discussion / Issue pinned on `loft-lang/loft` AND
     `loft-lang/registry`.
   - CVE filing through GitHub's CNA flow if any user-facing
     impact is possible.
   - Post-mortem to the loft-lang mailing list / Slack /
     whatever channel exists.

### Checklist — week 1

10. [ ] **Post-mortem**: how did the key get compromised?  Was
        it a malware infection, a misconfigured CI log, a
        physical theft, a phishing attack on the maintainer?
        Write it up; update REGISTRY_BOOTSTRAP.md § Step 1.5
        with any new safeguards.
11. [ ] **Verify ecosystem health**: any reports of installs
        producing wrong artefacts in the compromise window?
        Cross-check against the audit from step 8.
12. [ ] **Securely destroy the compromised key**: even if the
        attacker has it, destroying your local copies prevents
        accidental re-use.  Overwrite the .bin file (e.g.
        `dd if=/dev/urandom of=registry-signing-key.bin bs=32
        count=1` then delete), physically destroy any paper
        backups, wipe any YubiKey holding the old key.

### What the architecture protects you from

The compromised key lets an attacker sign forged indexes, but
to actually trick a user they need ALSO:

- Get the forged index to the user.  The default URL is
  `raw.githubusercontent.com/loft-lang/registry/main/index.json`
  — they need to either compromise GitHub or convince the user
  to point `LOFT_REGISTRY_URL` at a malicious mirror.
- Convince the user to install (not just rebuild) — existing
  `loft.lock` pins remain safe.
- The malicious tarball must hash to whatever sha256 the
  forged index claims.  They can pick the hash freely, but
  the user's verification still runs.

So the realistic attack window is: **new installs of compromised
versions, during the time between key exfil and emergency
release**.  Aggressive timeline + lockfile firewall keeps the
blast radius small.

---

## Annual recovery drill

Calendar this.  Pick a date (e.g. first Monday of every year)
and run through Scenario A end-to-end while everything is
still healthy.  Catches problems while they're fixable.

### Checklist (30-60 min)

1. [ ] Locate each backup copy.  Time yourself.  If you spent
       more than 5 minutes finding any of them, fix the
       location / documentation.
2. [ ] Verify the YubiKey is responsive.  Plug it in, confirm
       it shows up to the system.
3. [ ] Recall the YubiKey PIN.  Don't reset it just to test —
       if you got the PIN wrong now, you'd have gotten it
       wrong in a real recovery.
4. [ ] Recover the key on a test VM (clean Linux/macOS, no
       persistent state).  Use the most-recently-stored
       backup; rotate which one you test each year.
5. [ ] Sign a dummy `index.json` (containing the string
       `recovery-drill-YYYY-MM-DD`) with the restored key.
6. [ ] Verify the signature with the public key embedded in
       the current loft release binary.  This is the most
       important step — it confirms the FULL pipeline.
7. [ ] Discard the test VM.
8. [ ] If anything failed, fix it now.  Common failures:
   - YubiKey PIV slot configuration drift (firmware update
     migrated the key).
   - Paper backup faded or moisture-damaged (re-print).
   - You forgot which encrypted volume holds the key.
   - Family member who knew the safe combination moved
     houses.
9. [ ] Document the drill in a CHANGELOG entry in this repo:

   ```markdown
   ### 2026-MM-DD — annual trust-root recovery drill
   Tested: paper backup → restored to fresh VM → signed +
   verified dummy index.  No issues.  Next drill: 2027-MM-DD.
   ```

   Or note any issues + their fix dates.

### Why the drill matters

The two killer failure modes for any backup strategy:

1. **The backup was never actually a backup.**  You put a file
   somewhere and assumed it would work; you never tested.
   Common case: paper backups with errors in transcription,
   YubiKey slots that needed a firmware update mid-storage.
2. **You forget which copy is which / where it is.**  Six
   months from the bootstrap, you can't remember which
   physical address holds the second YubiKey.

The drill kills both.

---

## Quick reference — decision tree

```
Is the private key in adversary hands?
├── YES (compromise) → Scenario C — same-day emergency rotation
└── NO
    ├── Can you recover a backup?
    │   ├── YES → Scenario A — restore + continue (no user impact)
    │   └── NO  → Scenario B — multi-key rotation (no user impact, 6mo window)
    └── Annual practice run → Annual recovery drill
```

---

## See also

- [REGISTRY_BOOTSTRAP.md](REGISTRY_BOOTSTRAP.md) — initial setup
  (key generation, storage planning, registry repo creation).
- [PKG_REGISTRY.md](PKG_REGISTRY.md) — the design doc; § Index
  signing has the architectural rationale; § Path 1 server
  features describes how the migration to a real server would
  preserve the current trust model.
- [`src/registry_keys.rs`](../../src/registry_keys.rs) — the
  embedded trust roots in code.
- [`src/registry_signing.rs`](../../src/registry_signing.rs) —
  the verify implementation.
