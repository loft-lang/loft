<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN78 — loft binary distribution + self-update

**Status — DONE 2026-07-31.**  Filed 2026-05-31 as a five-phase, 10–15 work-day
slot; delivered in one pass, because the *package* registry had already built
almost every mechanism the *binary* distribution needed.  The re-audit that found
that, and the design that followed from it, are in **[design.md](design.md)** —
kept as the historical record, not as live reference.

## What shipped

A user who will not compile loft can install it, update it, and check what they
have:

```
sh install.sh                 # bootstrap: one artifact, one sha256, hand off to loft
loft self-update              # resolve, download, verify against the signature, replace
loft self-update --from <dir> # a bundle they already have — always available
loft verify-self              # is this installation the release it claims to be?
```

The invariant it was built on held: **the toolchain is one more signed artifact in
the index we already have**, so `self-update` routes through the same
signature-verified loader `loft install` uses rather than a second copy of it.
Both asymmetries the design named up front turned out to be the real work —
bootstrap has no verifier, and self-replacement mutates the running program.

## Where the reference content lives now

| Topic | Home |
|---|---|
| Release procedure, the hash chain, per-release verification | [RELEASE_PUBLISH.md](../../RELEASE_PUBLISH.md) § Tag & publish and § The bundles and the registry entry |
| Submitting the toolchain entry, the registry-side gates | [REGISTRY_SUBMIT.md § The toolchain entry](../../REGISTRY_SUBMIT.md) |
| What `verify-self` proves, and what it does not | `src/verify_self.rs` module docs |
| Why the entry's fields are what they are | `scripts/gen-toolchain-entry.py` |

## What outlived the plan, and what now enforces it

Three things are not done.  None is plan work, and each has a home that fails
loudly rather than a tracker row that rots:

1. **The registry entry is not published.**  It cannot be for v2026.7.2 —
   published assets are immutable, so that release can never gain the source
   archive the entry names.  The first entry is for the next release.  *Enforced
   by:* the `previous release reached the registry` CI job, which reddens the PR
   that bumps `Cargo.toml` if the last release never reached the signed index.
2. **`self-update` is unverified on Windows** against a genuinely running
   `loft.exe` — the one platform-divergent step in the chain, and the one thing no
   test can reach.  *Enforced by:* a per-release checklist item in
   [RELEASE_PUBLISH.md § The bundles and the registry entry](../../RELEASE_PUBLISH.md#the-bundles-and-the-registry-entry).
3. **Reproducible builds** (the plan's step 7) were sequenced last on purpose so
   they could never block a user-visible installer, and closing this does not make
   them urgent.  *Homed at:* [RELEASE_PUBLISH.md § Reproducible builds](../../RELEASE_PUBLISH.md#reproducible-builds).

A note on the original Goal's `curl -sSL https://loft-lang.org/install.sh | sh`:
the script ships and works, and once this branch is on `main` it is fetchable at
`raw.githubusercontent.com/loft-lang/loft/main/scripts/install.sh`.  A vanity URL
at `loft-lang.org` is a DNS/hosting decision, not distribution work, and nothing
here waits on it.

## What the plan predicted badly, and why it is worth remembering

The 2026-05-31 sizing was wrong in one specific way that recurs: it estimated the
work against **the world at filing time**.  Between filing and building, the
library registry shipped signing, multi-root key rotation, per-target
`BinaryEntry`, sha256-verified downloads and an advisory feed — five of the plan's
own phases, arrived at sideways.  The re-audit table at the top of
[design.md](design.md) is the artifact of noticing; without it this would have
been built a second time beside the first.
