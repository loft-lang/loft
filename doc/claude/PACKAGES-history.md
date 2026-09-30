<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->

# PACKAGES-history.md — the timeline behind PACKAGES.md

The timeline behind [PACKAGES.md](PACKAGES.md): the original implementation-phase plan.

---

## Implementation phases

| Phase | Scope | Effort | Depends on |
|---|---|---|---|
| **P1** | Connect `#native` to interpreter dispatch | Medium | `extensions.rs` completion |
| **P2** | `loft install` for local packages | Medium | P1 |
| **P3** | Native codegen `--extern` for `#native` packages | Medium | P1 |
| **P4** | WASM codegen with native package wasm rlib | Medium | P3 |
| **P5** | OpenGL package: 2D canvas + PNG | Medium | P1 |
| **P6** | OpenGL package: font rendering | Small | P5 |
| **P7** | OpenGL package: GL window + shader | High | P5 + glutin |
| **P8** | WebGL variant + WASM integration | High | P4 + P7 |

P1 is the foundation — without interpreter dispatch of `#native` symbols,
nothing else works.  P5-P6 can proceed in parallel with P3-P4 since the
2D canvas and font rendering don't need GL.

## Finished rows moved from PACKAGES.md § Open work on 2026-09-30

| Item | Status |
|---|---|
| **PKG.REG** — registry MVP (`loft package`/`install`/`search`/`info`, resolve, sig-verify) | **SHIPPED** — [PKG_REGISTRY.md](PKG_REGISTRY.md) R1–R9; 13 libs published. |
| **PKG.7** — `loft.lock` reproducible builds | **SHIPPED** — `src/lockfile.rs` (= R2). |
| **PKG.SIGN** — Ed25519 trust root | **SHIPPED** — PR #371: three independent keys in `registry_keys.rs`, `scripts/registry-sign.sh` review-then-sign tool, live index signed ([REGISTRY_BOOTSTRAP.md](REGISTRY_BOOTSTRAP.md)).  Fully active once a loft **release** ships the embedded keys. |
