<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 142 — Sudo-free user-local install of loft

## Status — DONE, 2026-08-17

All three phases plus both Phase-3 tails shipped.  `make install-user` installs to
`~/.local` with no sudo, passes the stdlib + cdylib smokes, and verifies that the
freshly installed binary is the one the shell resolves; `make install-user-fast`
does the same without the wasm/html-mt runtimes.

**How to use it lives in [README § Getting started](../../../../README.md) (users) and
[DEVELOPMENT.md § Local CI gate](../../DEVELOPMENT.md#local-ci-gate) (the
after-merge reinstall loop); the mechanism lives in the `Makefile` targets
themselves** (`install` / `install-artifacts` / `install-user` / `install-native` /
`install-user-fast`, with the proc-macro rule commented at the copy site).  This
file is the closure record.

| phase | shipped |
|---|---|
| **1** proc-macro `.dylib` deps | `install-artifacts` copies `deps/*.dylib` beside `*.rlib`/`*.so`, and the deps refresh deletes them too so a reinstall can't leave a stale one (*install: copy proc-macro .dylib deps so library native builds work on macOS*) |
| **2** `PREFIX` + conditional sudo | `PREFIX ?= /usr/local`; `SUDO` is **computed** from prefix writability (walk to the first existing ancestor, `test -w`) rather than assumed — a user prefix or a user-owned Homebrew `/usr/local` needs none, a root-owned prefix still escalates (*install: sudo-free user-local install via PREFIX + per-OS PATH check*) |
| **3** `install-user` + PATH check | `install-user` / `uninstall-user`; after the copy, `command -v loft` must equal `$(PREFIX)/bin/loft` — reporting *absent from PATH* and *shadowed by an earlier entry* separately, with the rc line for the resolved shell/OS, and editing nothing |
| **3a** native-only fast path | `NATIVE_ONLY=1` → `make install-native` (any prefix) / `install-user-fast` (`~/.local`): no wasm target builds, no html-mt, no `check-targets`, and the wasm copy step extracted to `install-wasm-artifacts` and skipped (*install: native-only fast path (install-user-fast) + README note*) |
| **3b** doc | README § Getting started; the per-OS PATH line is emitted by the target, not duplicated in prose |

No runtime code changed: loft already resolves its stdlib + rlibs **relative to the
binary** (`src/cache.rs` `loft_rlib_path`, `src/native_lib.rs` `rlib_search_dirs`),
so `<prefix>/bin/loft` finds `<prefix>/share/loft` for any prefix.  That is what
made the whole plan a Makefile change, and the cdylib smoke passing from `~/.local`
is what proved it end to end.

## What it found

**The motivating failure was a real macOS-only install break, not a convenience
gap.**  `make install` copied `deps/*.rlib` and `deps/*.so` but never `*.dylib`.
Proc-macro crates build to a *host* dylib — `.dylib` on macOS, `.so` on Linux — so
on macOS the installed `share/loft/deps/` held 107 `.rlib` and zero dylibs, and
every consumer library's cdylib (`extern crate loft;`) died at the post-install
smoke with `error[E0463]: can't find crate for displaydoc`.  Linux never saw it,
because its proc-macro artifact is the `.so` that was already copied.  A
platform-conditional artifact extension is the kind of thing an install rule gets
wrong silently on the platform its author doesn't use.

Three open questions closed with the build:

- **Writability detection** — "creatable by me" counts as writable: walk to the
  first *existing* ancestor and test that, since a first-time `~/.local/share`
  doesn't exist yet and `install -d` will make it.
- **Symlink vs copy** — copy.  A symlinked binary with a copied `share/loft` can
  drift out of pair, and the binary↔rlib smoke exists precisely to guard that pair.
- **`PREFIX` for wasm/html-mt** — not a prefix question at all; `NATIVE_ONLY=1`
  skips them, and the full `install-user` still ships them.

**One leak found while validating the fast path:** `NATIVE_ONLY` initially stopped
at the install targets, so `rebuild-native-cdylibs` still ran its two wasm rlib
rebuilds after any source change — the "fast" path re-entering the work it was
built to skip, via a shared dependency. The gate now passes down into it, and is
inert for every other caller.

## See also

- [README § Getting started](../../../../README.md) — the user-facing install.
- `Makefile` — `install`, `install-artifacts`, `install-user`, `install-native`,
  `install-user-fast`, `uninstall-user`.
- loft#693 — the binary↔rlib smoke this plan keeps green.
