<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 20 — Library health audit — the worklist

Tracker: [@PLN20](https://github.com/loft-lang/plans/issues/20) · `subject:libs`

## The done bar (per repo)

A library repo is **DONE** only when **all three** hold:

1. **Zero open branches** — every branch either merged (auto-deleted) or deleted as
   merged/superseded/abandoned; no stranded work.
2. **`main` green** — `library-ci` passes on `main` against the **current** loft.
3. **Zero warnings** — `main` compiles + tests against current loft with **no**
   deprecation/lint warnings (`not null`, `&`-on-param, len/size strict-index, steer).

Detect stranded branches any time with `scripts/lib-branch-audit.sh`; the nightly
`lib-branch-report` workflow surfaces them automatically.

## Status matrix (2026-07-18)

| Repo | main CI | open branches | warnings | DONE |
|---|---|---|---|---|
| **assets** | ✅ green | 0 | ? verify | 🟡 verify-warnings |
| **core** | ✅ green | 4 (cbor, crypto-v0.3.0, fix/cbor-narrowing, tuxedo-work) | ? | ☐ |
| **docs** | — (no ci run) | 1 (markdown flip #1) | ? | ☐ |
| **game** | ❌ red | 1 real + 1 held (flip #2) + fix-c95 (delete) | `not null`? | ☐ |
| **graphics** | ❌ red | 6 PRs + fix-c95 (delete) | ? | ☐ |
| **net** | ❌ red | ssh #10 + web flip #11 | ? | ☐ |
| **world** | ❌ red | hex_terrain #7, hex_world #8, nullflow (delete) | `not null`? | ☐ |

The four red repos (game/graphics/net/world) are red primarily because their `main`
still uses **byte-intent `len()`**, which miscomputes now that the len/size flip
(@PLN110) is live on loft `main`. The fix is the **held flip PRs** — and their hold
reason ("until @PLN110 ships") is now **satisfied** (it shipped in loft #587), so on
current loft `size()`=bytes is correct and they can merge. (Publishing new *registry
versions* is still gated on the contract 0→1 release; merging the branch to green `main`
is not.)

## Per-repo tasks

### assets — 🟡 nearly done
- [x] main green, zero branches.
- [ ] Verify `main` builds warning-free against current loft (glb/mesh3d).

### core — ☐ (main green; branch cleanup)
- [ ] **DELETE** `crypto-v0.3.0` — superseded (registry + main at crypto 0.3.5).
- [ ] **DELETE** `fix/cbor-loft-2026.6.0-narrowing` — superseded (main at cbor 0.1.1; PR #19 closed).
- [ ] **`cbor`** — big divergent branch (cbor C1/C2/A3 + crypto Ed25519…). crypto 0.3.5 + cbor 0.1.1 are on main, so most is published; **confirm the `not null` retirement (C2c) is the only live delta**, then merge that or delete.
- [ ] **`tuxedo-work`** (PR #20) — the const/cbor commits are superseded (const landed, cbor conflicts); only **arguments 0.2.0 (getopt_long)** is live+valuable. Rebase to drop the superseded commits and land arguments cleanly, or close #20 and re-PR arguments alone.
- [ ] Verify main warning-free (esp. `not null` on cbor/crypto struct fields).

### docs — ☐
- [ ] **markdown flip #1** — un-hold + merge (flip is live on loft main; `size()`=bytes now correct). 25 byte-intent `len→size` sites.
- [ ] Confirm `library-ci` runs + main green + warning-free.

### game — ☐
- [ ] **DELETE** `fix-c95-stdlib-floor_mod-collision` — merged.
- [ ] **time flip #2** — un-hold + merge → fixes the red main (`time` byte-offset parse).
- [ ] Re-run main CI → green; verify no `not null`/other warnings (input const-harden already landed via #3).

### graphics — ☐ (the messiest — 6 PRs)
- [ ] **DELETE** `fix-c95-stdlib-clamp-collision` — merged.
- [ ] **#13 reconcile-graphics-0.3.0** — main is behind the released 0.3.0/imaging-0.2.0; merge to reconcile.
- [ ] **#10 canvas `&self.data`** (@C86) — review + merge.
- [ ] **#12 graphics 0.4.0** input-event queue (also drops unnecessary `&`, C3) — review + merge.
- [ ] **#7 CI native packages**, **#2 gl_load_font path (#255)** — review + merge.
- [ ] **glb flip #14** — un-hold + merge.
- [ ] Re-run main CI → green; verify warning-free.

### net — ☐
- [ ] **ssh #10** — merge (ssh 0.1.0 is registered but its source isn't on main — reconcile).
- [ ] **web flip #11** — un-hold + merge → fixes the red main (`web` pack byte-count).
- [ ] Re-run main CI → green; verify warning-free.

### world — ☐
- [ ] **DELETE** `tuxedo-nullflow-compat` — superseded (main already discharges the sqrt; PR #9 closed).
- [ ] **hex_terrain #7** — likely stale (main already carries the `&self` @C86 fix); confirm + close-or-merge.
- [ ] **hex_world #8** (`not null` retirement C2a + `&` alias) — review + merge.
- [ ] Re-run main CI → green; verify warning-free (`not null` on hex_terrain).

## Cross-cutting themes

- **The held len/size-flip PRs** (docs #1, game #2, graphics #14, net #11) are the main
  blocker for green `main` on four repos. Their hold is satisfied (flip is on loft main);
  merge them to green the libs. Registry *publishing* stays gated on the contract flip.
- **The `not null` retirement** is a coordinated uptake, half-done: PR'd on world (#8) and
  net (ssh #10 carries C2d), stranded on core `cbor` (C2c). Land all so no lib emits the
  deprecation warning.
- **Delete list** (confirmed merged/superseded, no value): core `crypto-v0.3.0`, core
  `fix/cbor-narrowing`, game `fix-c95-floor_mod`, graphics `fix-c95-clamp`, world
  `tuxedo-nullflow-compat`.
