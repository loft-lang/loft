# Copyright (c) 2022-2025 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# ==== What can this Makefile do for you? ================================
#
# If you just want to try things:
#
#   make play       Launch Brick Buster natively (full OpenGL window).
#                   Checks prerequisites first; fails fast with a clear
#                   message if any native library is missing.
#
#   make game       Build the Brick Buster arcade game into one HTML file
#                   (doc/brick-buster.html). Double-click to play.
#                   Works even from a half-broken checkout.
#
#   make gallery    Build the Graphics Gallery (24 demos) for the browser
#                   and verify every asset loads. Run `make serve` after.
#
#   make serve      Start a local web server on http://localhost:8000/
#                   so you can open the Playground and Gallery.
#
#   make help       Print this overview again.
#
# If you are working on loft itself:
#
#   make all        Format source + build the native binary.
#   make test       Full test suite (fmt + clippy + tests). ~1-2 minutes.
#   make quick      Same tests without the clippy/fmt gate. Faster iteration.
#   make iter TEST=<filter> [TFILE=<binary>] [PROFILE=release]
#                   Run only tests matching <filter>.  Defaults to the
#                   dev profile — incremental rebuild ~2-3s vs ~25s
#                   for release (the project's Cargo.toml turns off
#                   debug-assertions on the loft package, so dev runs
#                   loft programs at near-release speed).  Add
#                   PROFILE=release when you specifically need
#                   release behaviour or want to share cache with
#                   `make test` / `make ci`.
#   make ci         Mirror of .github/workflows/ci.yml — fmt, clippy
#                   (deny warnings), build --all-targets, build
#                   --no-default-features, nextest run --profile ci.
#                   Runs the SAME gates the remote runner uses, in the
#                   same order, so a green `make ci` predicts a green
#                   PR.  Logs to result.txt.  Does NOT run the
#                   GL / packages suites — those live in their own
#                   targets (test-packages, test-gl-smoke, test-gl-golden)
#                   and are NOT gated by the remote.
#   make profile ARGS="--interpret p.loft"
#                   Where a run spends its time, down to the source LINE, and
#                   with --mem where its HEAP went.  It picks the instrument:
#                   an interpreted program is measured by loft's own sampler
#                   over its own call stack (perf's stack is the interpreter's,
#                   the same for every program ever run), a --native run and a
#                   --check by perf.  PROFILE_FLAGS="--engine" forces perf when
#                   loft ITSELF is the question; "--mem" for allocation hot
#                   spots by loft line, "--paths" to add the call paths that
#                   reached them; "--annotate" for source lines, "--calls" for
#                   who calls the hot function, "--no-cache" to profile a
#                   COMPILE rather than a startup-cache reload, "--no-warm" to
#                   skip the native pre-build.  Release is untouched.  See
#                   PERFORMANCE.md § Profiling a run.
#   make profile-corpus
#                   Run the instruments over bench/ and check each against the
#                   hot spot known in advance (bench/profile_oracle.tsv).  A
#                   failing row is a regression in the PROFILER — that half is
#                   a gate.  The share drift it prints beside it never is.
#   make ci-full    `ci` + the development-only suites (test-packages,
#                   test-gl-smoke, test-gl-golden).  What we used to
#                   call `make ci` before the slim-down.
#   make ship       Fast local pre-push gate: fmt + both clippy variants
#                   + release tests, streamed to the terminal, chained
#                   with && so `make ship && git push` stops on first
#                   failure.
#   make clean      Nuke build artifacts.
#
# More specialised:
#
#   make wasm            Build the wasm-pack bundle that drives the gallery.
#   make wasm-html-test  WASM-runtime safety gate (tests/html_wasm.rs).
#                        Rebuilds the wasm rlib in the --html shape first
#                        so the gate is deterministic regardless of whether
#                        `make wasm` last stomped the rlib with the
#                        wasm-bindgen variant.
#   make install         System-wide install to /usr/local (sudo if not writable).
#   make install-user    Sudo-free install to ~/.local (PREFIX=$$HOME/.local).
#   make install-user-fast  As install-user, but native-only (no wasm) — fast
#                        reinstall for the after-each-PR-merge loop.
#   make test-gl-golden  Pixel-compare the smoke-test screenshot (Xvfb).
#   make fill            Regenerate src/fill.rs from default/*.loft annotations.
#   make pdf             Rebuild the printable reference PDF.
#
# Every target above is defined as a real rule later in this file.  Scroll
# down to any name to see exactly what it does.
#
# Housekeeping (the box, not the build):
#
#   make sweep-scratch   Reclaim loft's temp scratch: dead-process native artefacts,
#                        aged test caches, agent sessions older than two weeks.
#                        Run when `df` says so — TESTING.md § Scratch hygiene.
#   make sweep-target    Drop cargo artefacts no build in two weeks has used.
#
# =========================================================================

# Cache clean/release rebuilds with sccache when it is installed.  Exported
# so every recipe shell inherits it; a no-op when sccache is absent (CI,
# other developers) so it never becomes a hard dependency.  Composes with
# the mold linker from .cargo/config.toml (sccache caches the compile, mold
# links).  CARGO_INCREMENTAL=0 because sccache cannot cache incremental
# units — these targets are clean/release builds, not the interactive edit
# loop, so there is no cost.
ifneq ($(shell command -v sccache 2>/dev/null),)
# Only when NOT root.  sccache binds its server per-user on 127.0.0.1:4226;
# a root build (e.g. under `sudo make install`) cannot reach the invoking
# user's server and dies with "failed to read response header".  The build
# never runs as root anyway — `install` drops it back to the user via
# AS_USER below — so disabling the wrapper for the root parent is harmless.
ifneq ($(shell id -u),0)
export RUSTC_WRAPPER := sccache
export CARGO_INCREMENTAL := 0
endif
endif

# macOS: cc-rs invocations (notably `ring`'s build script) need the SDK path
# on -isysroot.  When `xcode-select -p` points at the bare Command Line Tools,
# plain `cc` does not auto-detect it and the build fails with
# `'TargetConditionals.h' file not found`.  Probe via `xcrun` and export so
# every recipe shell inherits it.  Honours a user-set SDKROOT; the whole
# block is skipped on non-Darwin (Linux/Windows), so other platforms build
# unchanged.
ifeq ($(shell uname -s),Darwin)
ifeq ($(origin SDKROOT),undefined)
SDKROOT := $(shell xcrun --sdk macosx --show-sdk-path 2>/dev/null)
endif
export SDKROOT
endif

# `make install` writes the binary to /usr/local (needs root) but the compile
# must NOT run as root: sccache is per-user (see above) and a root build would
# leave target/ owned by root, breaking the next ordinary `make`.  When invoked
# as `sudo make install` we are root with SUDO_USER set — AS_USER drops the
# build back to that user; the file-copy steps in `install` stay root.  Empty
# for a normal `make install`, where the build already runs as the user and the
# copy steps escalate with their own `sudo`.
AS_USER :=
ifeq ($(shell id -u),0)
AS_USER := $(if $(SUDO_USER),sudo -u $(SUDO_USER) -H,)
endif

# Install prefix.  Default is system-wide /usr/local; override with a user-writable
# prefix for a sudo-free install:  make install PREFIX=$$HOME/.local  (or the
# `install-user` shortcut).  No code change is needed — the runtime finds its stdlib
# and rlibs RELATIVE to the binary: a loft at <PREFIX>/bin/loft searches
# <PREFIX>/share/loft and .../deps (src/cache.rs, src/native_lib.rs).
PREFIX ?= /usr/local

# Root is needed only when PREFIX is not writable by the invoking user — computed,
# never assumed.  Walk up to the first existing ancestor and test it: a user prefix
# (~/.local) is writable → no sudo; a user-owned Homebrew /usr/local → no sudo; a
# root-owned /usr/local → sudo; and under `sudo make install` we are already root, so
# the ancestor is writable and nothing extra runs.  Empty = run the copies directly.
SUDO := $(shell d="$(PREFIX)"; while [ -n "$$d" ] && [ "$$d" != / ] && [ ! -e "$$d" ]; do d=$$(dirname "$$d"); done; if [ -w "$$d" ]; then echo ""; else echo "sudo"; fi)

# Native-only install: skip the wasm + html-mt browser runtimes — the slow part of
# the build — for a FAST reinstall.  The result runs loft programs and builds library
# cdylibs (all a dev needs after a PR merge); only `--html` / wasm export needs the
# skipped runtimes.  Set by `make install-native` / `install-user-fast`.
NATIVE_ONLY ?=

.PHONY: check-wasm-threads check-no-threading par-gates gate ci-miri all check-targets doctor install install-user install-native install-user-fast install-artifacts install-artifacts-native install-wasm-artifacts uninstall uninstall-user debug test quick profile clean clean-wasm fill ci ship run-tests clippy memory last meld generate gtest pdf bench test-native test-wasm test-html-render loft-test wasm-assets test-packages test-package-native-tests test-gl-headless test-gl-smoke test-gl-golden update-gl-golden serve wasm gallery game crystal-editor play native-editor editor-dist help rebuild-native-cdylibs view-build view-refresh view index index-install-hook hooks libcatalogue features-fetch features-gen features-check surface-gen surface-check api-compat check-contract-goldens contract-labels-test

# Print the overview at the top of this file.  Useful when you land on a
# fresh checkout and want to know what buttons are available without
# reading a 300-line Makefile.
help:
	@sed -n '/^# ==== What can this Makefile do for you/,/^# ====/p' Makefile \
	  | sed 's/^# \{0,1\}//'

all:
	@rustfmt src/*.rs --edition 2024
	@RUSTFLAGS=-g cargo build --release

check-targets:
	@if ! command -v rustup >/dev/null 2>&1; then \
		echo "WARNING: rustup not found; can't verify cross-compile targets."; \
		echo "If the build fails with E0463, install wasm32-wasip2 and"; \
		echo "wasm32-unknown-unknown manually for your toolchain."; \
		exit 0; \
	fi
	@for target in wasm32-wasip2 wasm32-unknown-unknown; do \
		if ! rustup target list --installed | grep -q "^$$target$$"; then \
			echo "Installing missing rustup target: $$target"; \
			rustup target add "$$target" || { \
				echo "ERROR: failed to install $$target."; \
				echo "Install manually:  rustup target add $$target"; \
				exit 1; \
			}; \
		fi; \
	done

# doctor: report the status of every external tool the wasm / --html / native
# pipelines depend on, and print environment-specific install commands for
# whatever is missing, so a fresh environment can be set up to actually work.
# Diagnostic only — never installs or fails the build.  See
# doc/claude/WASM.md § Build Toolchain Dependencies for what each tool is for.
doctor:
	@bash scripts/doctor.sh

# Compile every artifact that lands in /usr/local.  Runs as the unprivileged
# user (see AS_USER) — never root — so sccache and target/ ownership stay
# correct even under `sudo make install`.
#
# The three builds, in recipe order:
#   1. wasm32-wasip2 lib.
#   2. wasm32-unknown-unknown lib — W1.1, the browser target for `--html` export.
#   3. the host lib into an ISOLATED target dir, so deps/ contains exactly one
#      copy of each crate — no binary-only duplicates (e.g. libloading) that
#      cause StableCrateId collisions during native compilation.
#
# Why build 3 carries the feature list it does:
#   - `native-extensions` is REQUIRED: the `--native` codegen unconditionally
#     emits `loft::native_call::{enter,build_store}` to marshal a heap value
#     (`vector<u8>`, struct) across the cdylib FFI (src/generation/mod.rs), and
#     that module is `#[cfg(feature = "native-extensions")]` (src/lib.rs).  Drop
#     it here and the installed `libloft.rlib` lacks `native_call`, so every
#     `--native` run of a library with a `vector`/struct-returning (or -taking)
#     `#native` fn fails to compile with `E0433: cannot find native_call in loft`.
#   - `registry` + `remote-store` are BOTH in the crate's `default` features, so
#     omitting them here made the installed native runtime diverge from a normal
#     build: `store_load_url*` (registry-gated `load_url`,
#     src/database/allocation.rs) and the sibling remote/URL store loaders
#     (remote-store-gated) then `--interpret`-accept but `--native`-REJECT with
#     `no method load_url` on the stable toolchain (reported by a consumer on
#     2026.7.1).  Both pull only `ureq` (+ archive/crypto for registry) and are
#     dead-stripped from any `--native` binary that doesn't call them, so
#     bundling them is free for programs that don't fetch stores.
#
# The closing prune is the E0514 guard — cargo's deps/ accumulates a
# `libloft_ffi-<hash>.rlib` per rustc version across builds; after a `rustup
# update` the OLD one lingers beside the freshly-built one.  The install `cp`s
# ALL of them into the shared deps/, and because `cp` flattens every file's
# mtime to install-time the mtime-based loft_ffi resolver
# (native_lib::loft_ffi_for_libloft) can then pick the STALE one — so every
# `--native` build fails `E0514: crate loft_ffi compiled by an incompatible
# version of rustc`.  Keep only the newest (the just-built, current-rustc)
# loft_ffi rlib so exactly one, matching, copy is installed.
install-artifacts: check-targets all wasm-html-mt-lib
	@cargo build --release --target wasm32-wasip2 --lib --no-default-features --features random
	@cargo build --release --target wasm32-unknown-unknown --lib --no-default-features --features random
	@cargo build --release --lib --no-default-features --features mmap,random,threading,native-extensions,registry,remote-store --target-dir target/install-lib
	@stale=$$(ls -t target/install-lib/release/deps/libloft_ffi-*.rlib 2>/dev/null | tail -n +2); \
	 if [ -n "$$stale" ]; then echo "  pruning stale loft_ffi rlib(s): $$stale"; rm -f $$stale; fi

# Native-only artifacts: the same install-lib rlib, but WITHOUT the two wasm target
# builds, the wasm-target check, or the html-mt build-std runtime.  `all` builds the
# native binary + lib; this adds the feature-complete install-lib rlib the copies
# need.  Same stale-loft_ffi prune as the full target.
install-artifacts-native: all
	@cargo build --release --lib --no-default-features --features mmap,random,threading,native-extensions,registry,remote-store --target-dir target/install-lib
	@stale=$$(ls -t target/install-lib/release/deps/libloft_ffi-*.rlib 2>/dev/null | tail -n +2); \
	 if [ -n "$$stale" ]; then echo "  pruning stale loft_ffi rlib(s): $$stale"; rm -f $$stale; fi

install:
	@if [ -n "$(SUDO)" ]; then \
		sudo true || { \
			echo "ERROR: writing $(PREFIX) needs root — it is not writable by you."; \
			echo "Re-run with sudo, or install to a user-writable prefix (no sudo):"; \
			echo "    make install-user           # => \$$HOME/.local"; \
			echo "    make install PREFIX=DIR     # => any writable DIR"; \
			exit 1; }; \
	else \
		echo "install: writing $(PREFIX) as $$(id -un) — no sudo needed"; \
	fi
	@$(AS_USER) $(MAKE) --no-print-directory $(if $(NATIVE_ONLY),install-artifacts-native,install-artifacts)
	@$(AS_USER) $(MAKE) --no-print-directory rebuild-native-cdylibs
	@$(SUDO) install -d $(PREFIX)/share/loft/deps
	@$(SUDO) rm -rf $(PREFIX)/share/loft/default
	@$(SUDO) cp -r default $(PREFIX)/share/loft/
	@$(SUDO) install -m 644 target/install-lib/release/libloft.rlib $(PREFIX)/share/loft/
	@$(SUDO) rm -f $(PREFIX)/share/loft/deps/*.rlib $(PREFIX)/share/loft/deps/*.so $(PREFIX)/share/loft/deps/*.dylib
	@$(SUDO) cp target/install-lib/release/deps/*.rlib $(PREFIX)/share/loft/deps/
	@if ls target/install-lib/release/deps/*.so >/dev/null 2>&1; then \
		$(SUDO) cp target/install-lib/release/deps/*.so $(PREFIX)/share/loft/deps/ || { \
			echo "ERROR: failed to install dependency .so files (rights?)."; exit 1; }; \
	fi
	@# Proc-macro deps (e.g. displaydoc) are host dylibs — `.dylib` on macOS,
	@# `.so` on Linux (copied above).  A cdylib's `extern crate loft;` needs them,
	@# so omitting the macOS `.dylib` broke every library native build with
	@# E0463 "can't find crate for displaydoc" (see plans/user-local-install).
	@if ls target/install-lib/release/deps/*.dylib >/dev/null 2>&1; then \
		$(SUDO) cp target/install-lib/release/deps/*.dylib $(PREFIX)/share/loft/deps/ || { \
			echo "ERROR: failed to install dependency .dylib files (rights?)."; exit 1; }; \
	fi
	@if [ -n "$(NATIVE_ONLY)" ]; then \
		echo "install: native-only — skipping wasm + html-mt runtimes ('--html'/wasm export unavailable)"; \
	else \
		$(MAKE) --no-print-directory install-wasm-artifacts PREFIX="$(PREFIX)"; \
	fi
	@$(SUDO) chmod -R a+rX $(PREFIX)/share/loft
	@$(SUDO) install -d $(PREFIX)/bin
	@$(SUDO) install -m 755 target/release/loft $(PREFIX)/bin/loft
	@smoke="$${TMPDIR:-/tmp}/loft-install-smoke.loft"; \
	printf 'fn main() {\n    println("loft install smoke ok")\n}\n' > "$$smoke"; \
	if ! $(PREFIX)/bin/loft --interpret "$$smoke" >/dev/null 2>"$$smoke.err"; then \
		echo "ERROR: 'make install' left a broken binary<->stdlib pair —"; \
		echo "the installed loft cannot run the installed stdlib:"; \
		sed 's/^/    /' "$$smoke.err"; \
		echo "Fix: rebuild + reinstall as one unit:  make all && make install"; \
		rm -f "$$smoke" "$$smoke.err"; exit 1; \
	fi; \
	rm -f "$$smoke" "$$smoke.err"; \
	echo "install: post-install smoke OK (installed loft runs the installed stdlib)"
	@# loft#693 — the smoke above proves binary<->STDLIB.  It cannot prove
	@# binary<->RLIB: generated cdylib source calls loft runtime methods, so a
	@# libloft.rlib older than the binary fails to compile and the program cannot
	@# start at all.  That shipped once (a new borrowed-capture helper the installed
	@# rlib lacked) and surfaced to the consumer as "library X failed to build
	@# native".  Building one throwaway cdylib against the INSTALLED rlib closes it.
	@# The capture below is a PROJECTION on purpose — a borrowed capture, the newest
	@# emission path — and any drift in the shared type-registration replay fails here.
	@sdir="$${TMPDIR:-/tmp}/loft-install-smoke-lib"; \
	rm -rf "$$sdir"; mkdir -p "$$sdir/libs/smokelib/src"; \
	printf '[package]\nname = "smokelib"\nversion = "0.1.0"\nloft = ">=0.8"\n[library]\nentry = "src/smokelib.loft"\n' > "$$sdir/libs/smokelib/loft.toml"; \
	printf 'pub fn smoke_sum(v: vector<integer>) -> integer {\n  t = 0;\n  for e in v { t += e; }\n  t\n}\n' > "$$sdir/libs/smokelib/src/smokelib.loft"; \
	printf 'use smokelib;\nstruct SmIn { items: vector<integer> }\nstruct SmOut { inner: SmIn }\nfn smk() -> SmOut { SmOut { inner: SmIn { items: [1, 2, 3] } } }\nfn app(f: fn(float) -> float, x: float) -> float { f(x) }\nfn main() {\n  o = smk();\n  w = o.inner;\n  g = fn(a: float) -> float { a + (len(w.items) as float) };\n  println("smoke {app(g, 1.0)} {smoke_sum([1, 2, 3])}");\n}\n' > "$$sdir/main.loft"; \
	if ! $(PREFIX)/bin/loft --interpret --lib "$$sdir/libs" "$$sdir/main.loft" >/dev/null 2>"$$sdir/err"; then \
		echo "ERROR: 'make install' left a broken binary<->rlib pair —"; \
		echo "the installed loft cannot build a library cdylib against the installed libloft.rlib:"; \
		sed 's/^/    /' "$$sdir/err"; \
		echo "Fix: rebuild + reinstall as one unit:  make all && make install"; \
		rm -rf "$$sdir"; exit 1; \
	fi; \
	rm -rf "$$sdir"; \
	echo "install: cdylib smoke OK (a library builds against the installed rlib)"
	@# PATH check — the freshly installed loft must be the one the shell resolves.
	@# Two ways it isn't: $(PREFIX)/bin not on PATH, or an older loft earlier on PATH
	@# shadowing it.  We report both, with the OS/shell-appropriate rc file, and edit
	@# nothing ourselves.
	@bindir="$(PREFIX)/bin"; \
	on_path=0; case ":$$PATH:" in *":$$bindir:"*) on_path=1 ;; esac; \
	resolved=$$(command -v loft 2>/dev/null || true); \
	if [ "$$on_path" = 1 ] && [ "$$resolved" = "$$bindir/loft" ]; then \
		echo "install: PATH OK — 'loft' resolves to the just-installed $$bindir/loft"; \
	else \
		case "$$SHELL" in \
			*/zsh)  rc="~/.zprofile" ;; \
			*/bash) if [ "$$(uname -s)" = Darwin ]; then rc="~/.bash_profile"; else rc="~/.bashrc"; fi ;; \
			*)      rc="~/.profile" ;; \
		esac; \
		if [ "$$on_path" = 0 ]; then \
			echo "NOTE: $$bindir is not on your PATH — the loft just installed will not be found."; \
		else \
			echo "NOTE: 'loft' resolves to $${resolved:-<none>}, not the just-installed"; \
			echo "      $$bindir/loft — an earlier PATH entry is shadowing it."; \
		fi; \
		echo "      Add to $$rc, then restart your shell (or 'hash -r'):"; \
		echo "          export PATH=\"$$bindir:\$$PATH\""; \
		if [ "$$(uname -s)" = Darwin ]; then \
			echo "      (macOS system-wide alternative: a file under /etc/paths.d/)"; \
		fi; \
	fi

# Copy the wasm (wasm32-wasip2, wasm32-unknown-unknown) + html-mt browser runtimes
# into the install prefix.  Split out of `install` so a native-only install can skip
# it; called with the same PREFIX/SUDO the parent resolved.
#
# The html-mt dependency copy reads BOTH cargo layouts.  `-Z build-std` lands its
# rlibs under the per-unit `build/<crate>/<hash>/out/` tree cargo adopted on
# 2026-07-29, where `release/deps/` does not exist at all — so `cp .../deps/*.rlib`
# failed with `cannot stat` and took the whole install down.  This step runs BEFORE
# the binary is installed, so the failure is at least honest: it leaves the user on
# their previous loft rather than pairing a new binary with stale runtimes.
# `src/native_utils.rs::dep_search_dirs` already reads both layouts for the LINK;
# this is the same fact on the install side.  The two blocks above still assume
# `deps/` and are correct today: they are ordinary cargo builds, not build-std ones.
#
# loft's OWN rlib is excluded, for the reason `assemble_atomics_sysroot` gives when
# it skips the same file: the link names it with `--extern` and `-L dependency=`, and
# a second copy on the search path is how rustc comes to report multiple candidates
# for one crate.  Under the per-unit layout it is worse than redundant — two build
# dirs each hold a `libloft.rlib`, so the copy aborts on the duplicate basename.
install-wasm-artifacts:
	@$(SUDO) install -d $(PREFIX)/share/loft/wasm32-wasip2/deps
	@$(SUDO) install -m 644 target/wasm32-wasip2/release/libloft.rlib $(PREFIX)/share/loft/wasm32-wasip2/
	@$(SUDO) rm -f $(PREFIX)/share/loft/wasm32-wasip2/deps/*.rlib
	@$(SUDO) cp target/wasm32-wasip2/release/deps/*.rlib $(PREFIX)/share/loft/wasm32-wasip2/deps/
	@$(SUDO) install -d $(PREFIX)/share/loft/wasm32-unknown-unknown/deps
	@$(SUDO) install -m 644 target/wasm32-unknown-unknown/release/libloft.rlib $(PREFIX)/share/loft/wasm32-unknown-unknown/
	@$(SUDO) rm -f $(PREFIX)/share/loft/wasm32-unknown-unknown/deps/*.rlib
	@$(SUDO) cp target/wasm32-unknown-unknown/release/deps/*.rlib $(PREFIX)/share/loft/wasm32-unknown-unknown/deps/
	@if [ -f target/loft/html-mt/wasm32-unknown-unknown/release/libloft.rlib ]; then \
	  echo "install: shipping the threaded browser runtime (html-mt)"; \
	  $(SUDO) install -d $(PREFIX)/share/loft/html-mt/wasm32-unknown-unknown/deps; \
	  $(SUDO) install -m 644 target/loft/html-mt/wasm32-unknown-unknown/release/libloft.rlib \
	    $(PREFIX)/share/loft/html-mt/wasm32-unknown-unknown/; \
	  $(SUDO) rm -f $(PREFIX)/share/loft/html-mt/wasm32-unknown-unknown/deps/*.rlib; \
	  set -- $$(ls target/loft/html-mt/wasm32-unknown-unknown/release/deps/*.rlib 2>/dev/null); \
	  [ $$# -gt 0 ] || set -- $$(ls target/loft/html-mt/wasm32-unknown-unknown/release/build/*/*/out/*.rlib 2>/dev/null \
	    | grep -v '/libloft[.-]'); \
	  [ $$# -gt 0 ] || { echo "install: html-mt built but no dependency rlibs found in either cargo layout" >&2; exit 1; }; \
	  $(SUDO) cp "$$@" $(PREFIX)/share/loft/html-mt/wasm32-unknown-unknown/deps/; \
	else \
	  echo "install: no threaded browser runtime built — 'loft --html --threads' will report it"; \
	fi

# Sudo-free install into the user's home prefix — reinstall as often as you like
# (e.g. after each PR merge) with no root.  Just `make install` with PREFIX pointed
# at ~/.local, which is user-writable so $(SUDO) resolves empty.
install-user:
	@$(MAKE) --no-print-directory install PREFIX=$(HOME)/.local

# Fast reinstalls: skip the wasm + html-mt runtimes.  `install-native` respects
# PREFIX (system by default); `install-user-fast` is the sudo-free ~/.local variant —
# the one to run after each PR merge when you only need to run + build loft locally.
install-native:
	@$(MAKE) --no-print-directory install NATIVE_ONLY=1

install-user-fast:
	@$(MAKE) --no-print-directory install PREFIX=$(HOME)/.local NATIVE_ONLY=1

uninstall:
	$(SUDO) rm -f $(PREFIX)/bin/loft
	$(SUDO) rm -rf $(PREFIX)/share/loft

uninstall-user:
	@$(MAKE) --no-print-directory uninstall PREFIX=$(HOME)/.local

debug:
	RUSTFLAGS=-g RUST_BACKTRACE=1 cargo build -v
	sudo ln -f -s ${PWD}/target/debug/loft /usr/local/bin/loft

# Rebuild every derived artefact the test suite depends on.  Covers
# four classes of stale artefact that each cascade into misleading
# test failures (rustc 1.94→1.96 update on 2026-05-29 surfaced #2):
#   0. The top-level release rlib `target/release/libloft.rlib` —
#      linked by the native code-gen pipeline (P200/P244 tests) —
#      AND the release `target/release/loft` binary.  `--bins` is
#      load-bearing (#304): tests that spawn the release binary by
#      path (tests/viewer_markdown.rs) get a stale-source binary
#      otherwise — `make ci` runs its tests in the debug profile, so
#      nothing else in that chain ever rebuilds the release bin, and
#      the fresh rlib + stale bin pair made the binary build (or
#      validate) native cdylibs against a loft that is not its own.
#      Cargo's fingerprint detects rustc version bumps, so this is
#      a no-op when fresh and a forced rebuild after a toolchain
#      update.
#   1. Sibling cdylibs under lib/*/native/  (loaded via
#      extensions::load_all, linked via --native)
#   2. Test fixture cdylibs under tests/lib/*/native/
#   3. The wasm32-unknown-unknown rlib the html_wasm suite links
#      AND the wasm32-wasip2 rlib the wasm_library_suite uses
#      (only when the target dir already exists, so `make test`
#      doesn't impose the wasm targets on developers who never
#      touch --html / wasip2)
# Cargo is incremental; each step is ~free on a clean tree.
rebuild-native-cdylibs:
	@cargo build --release --lib --bins -q || { \
	  echo "FAIL: top-level libloft.rlib + loft binary rebuild"; exit 1; \
	}
	@for d in lib/*/native tests/lib/*/native; do \
	  [ -f "$$d/Cargo.toml" ] || continue; \
	  (cd "$$d" && cargo build --release -q) || { \
	    echo "FAIL: rebuild $$d"; exit 1; \
	  }; \
	done
	@# Gate on the target's std being INSTALLED, not on the output dir
	@# (which persists across toolchain switches — a fresh `rustup default`
	@# drops the wasm std but leaves target/wasm32-*/, so the old `[ -d … ]`
	@# heuristic hard-failed with a buried E0463).  Installed → rebuild;
	@# stale artefact but std gone → warn with the fix and SKIP; neither → skip.
	@if [ -n "$(NATIVE_ONLY)" ]; then :; \
	elif rustup target list --installed 2>/dev/null | grep -qx wasm32-unknown-unknown; then \
	  cargo build --release --target wasm32-unknown-unknown \
	    --lib --no-default-features --features random -q || { \
	    echo "FAIL: wasm32-unknown-unknown rlib rebuild"; exit 1; \
	  }; \
	elif [ -d target/wasm32-unknown-unknown ]; then \
	  echo "WARN: wasm32-unknown-unknown std not installed — skipping wasm rlib refresh"; \
	  echo "      (stale target/wasm32-unknown-unknown/ present; run: rustup target add wasm32-unknown-unknown)"; \
	fi
	@if [ -n "$(NATIVE_ONLY)" ]; then :; \
	elif rustup target list --installed 2>/dev/null | grep -qx wasm32-wasip2; then \
	  cargo build --release --target wasm32-wasip2 \
	    --lib --no-default-features --features random -q || { \
	    echo "FAIL: wasm32-wasip2 rlib rebuild"; exit 1; \
	  }; \
	elif [ -d target/wasm32-wasip2 ]; then \
	  echo "WARN: wasm32-wasip2 std not installed — skipping wasm rlib refresh"; \
	  echo "      (stale target/wasm32-wasip2/ present; run: rustup target add wasm32-wasip2)"; \
	fi

# Disk-backed scratch for native/wasm test compiles — keeps rustc/rust-lld link
# intermediates AND the loft native binary cache off the /tmp tmpfs (a ~7.5G RAM
# disk on some boxes), which parallel compiles otherwise exhaust → rust-lld dies
# with SIGBUS, surfacing as flaky, unrelated failures.  TMPDIR also redirects
# every std::env::temp_dir() user (cross_mode / exit_codes / html_wasm), and
# scratch_dir() falls back to TMPDIR when LOFT_TMPDIR is unset.  Mirrors the same
# redirect in scripts/find_problems.sh.  MUST be OUTSIDE the repo: a
# `target/`-relative TMPDIR breaks the package/registry tests (they build
# fixtures in temp_dir then package/extract — anything under `target/` is
# excluded, so loft.toml goes missing).  /var/tmp is disk-backed.
TEST_SCRATCH := /var/tmp/loft-test-scratch-$(shell printf '%s' "$(CURDIR)" | cksum | cut -d' ' -f1)
TEST_ENV := TMPDIR=$(TEST_SCRATCH) LOFT_TMPDIR=$(TEST_SCRATCH)

# How many loft gates are live on this box right now, counting this checkout's own
# claim — the divisor the CI throttle shares its parallelism by.
#
# `ci-guard` already DETECTS a sibling gate and it is already mutual (every run scans
# `../*/` for a live claim).  What it did was warn, and a warning does not stop two
# gates asking the box for 2 x $(nproc) rustc processes at once.  On 24 threads and
# 61 GiB that is the shape that reaches the memory ceiling: rustc peaks over a GiB on
# the big crates, and `user@1000.service` is `ManagedOOMMemoryPressure=kill` at 90%
# (`/etc/systemd/oomd.conf.d/50-relax.conf`), so the kill lands on whatever the slice
# is running — an agent session as readily as the compile that caused it.
#
# Counted, not assumed: a claim whose pid is dead does not count, the same liveness
# test `ci-guard` uses, so a killed run cannot throttle every later one.
# ⚠ Deduplicated by REALPATH, and that is not a detail: `../*/` matches this checkout
# too, so a naive loop counts our own claim twice and halves the box for a gate that is
# alone on it.  Measured while writing this — no claims answered 1, one live claim
# answered 3.  `ci-guard`'s own sibling loop skips self the same way.
# `nproc` is Linux; macOS spells it `sysctl -n hw.ncpu` — a bare $(nproc) made the
# recipe die with `nproc: command not found` on every Mac (same 2808e183 throttle).
CI_NPROC = $$( nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4 )
# The case pattern carries a leading `(` on purpose: macOS's /bin/sh is bash 3.2, which
# cannot parse a pattern's bare `)` inside `$( )` command substitution — without it,
# every `make ci` on a Mac died at the recipe with `syntax error near ';;'` (the
# optional open-paren is POSIX and is what rebalances 3.2's parser).
CI_LIVE_GATES = $$( n=0; seen=""; for f in .ci-running ../*/.ci-running; do [ -f "$$f" ] || continue; d=$$(cd "$$(dirname "$$f")" 2>/dev/null && pwd -P) || continue; case " $$seen " in (*" $$d "*) continue;; esac; seen="$$seen $$d"; kill -0 "$$(cat "$$f" 2>/dev/null)" 2>/dev/null && n=$$((n+1)); done; [ $$n -lt 1 ] && n=1; echo $$n )


# Speed REPORT for the slow tests — never a gate.  `speed` measures the tests
# that carry a `// @speed` annotation, one at a time (parallel wall-clock is
# mostly contention), best of two runs, and prints what drifted.  `speed-discover`
# is the wide parallel pass that finds which tests deserve an annotation.
# Nothing here fails: correctness fails a build, speed is what you read.
.PHONY: speed profile profile-corpus speed-discover speed-bless sweep-scratch sweep-target

sweep-scratch:  ## Reclaim loft's scratch: dead-process native artefacts, aged test caches, old sessions
	@# What loft writes to a temp dir and what removes it — TESTING.md § Scratch hygiene.
	@# Safe by construction: only loft's own names, only dead pids or aged entries, and a
	@# sibling checkout's gate scratch is never touched (each checkout has its own).
	@scripts/sweep_scratch.sh --sessions $(TEST_SCRATCH) "$${TMPDIR:-$$HOME/.cache/tmp}"
	@df -h / | tail -1

sweep-target:  ## Drop cargo artefacts no build in two weeks has used (stale-hash test binaries)
	@# `target/debug/deps` keeps every test binary of every dependency hash it ever built
	@# (76-110 GB per checkout measured); cargo-sweep removes the ones no recent build
	@# touched.  Installs the tool on first use, exactly as `make ci` does for nextest.
	@(cargo sweep --version >/dev/null 2>&1 || cargo install cargo-sweep --locked) && cargo sweep --time 14
	@du -sh target 2>/dev/null
profile:  ## Sampling profile of a loft run: make profile ARGS="--interpret p.loft"
	@scripts/profile.sh $(PROFILE_FLAGS) -- $(ARGS)
profile-corpus:  ## Check the profilers against bench/profile_oracle.tsv, then report drift
	@scripts/profile_corpus.sh $(PROFILE_FLAGS)
speed:  ## Report how the slow tests' speed has drifted (never fails)
	python3 scripts/test_speed.py run
speed-discover:  ## Find tests slow enough to deserve a @speed annotation
	python3 scripts/test_speed.py discover
speed-bless:  ## Write the measured numbers back into the tests
	python3 scripts/test_speed.py run --bless

test: clippy rebuild-native-cdylibs
	-rm -f tests/generated/*
	-rm -f tests/dumps/*.txt
	mkdir -p $(TEST_SCRATCH)
	# --release: the loft bytecode interpreter is ~1800x slower in debug
	# mode (debug Rust running an interpreter loop). Release mode keeps
	# the full test suite under a minute instead of 30+ minutes.
	$(TEST_ENV) RUST_BACKTRACE=1 cargo test --release -- --nocapture --test-threads=1 >> result.txt 2>&1

quick: rebuild-native-cdylibs
	mkdir -p $(TEST_SCRATCH)
	$(TEST_ENV) RUST_BACKTRACE=1 cargo test --release -- --nocapture --test-threads=1 > result.txt 2>&1

# make iter TEST=<filter> [TFILE=<test_binary>] [PROFILE=release]
#
# Fast single-test iteration loop.  Defaults to the dev profile (which is
# specially tuned in Cargo.toml: opt-level=1 plus
# debug-assertions=false on the loft package).  Measured here:
#
#   incremental rebuild after touching one file:
#     dev profile     ~2.4s
#     release profile ~26.8s
#
# That's ~11x faster wall-time on the inner debug loop.  Pass
# `PROFILE=release` if you specifically need release behaviour
# (e.g. timing-sensitive parallel tests, or to share cache with
# `make test` / `make ci`).
#
# Cleans tests/dumps/ and tests/generated/ first — these are written
# by every test run and pin codegen output for the LAST run; if you
# alternate profiles or run a different subset, stale fixtures can
# trigger bogus errors (e.g. `attempt to add with overflow` from
# u16::MAX placeholder positions).  `make test` / `make ci` already
# clean them; this matches that behaviour.
#
# TFILE optionally restricts to one test binary — e.g. TFILE=issues
# skips every other test crate's rebuild + run.
#
# Examples:
#   make iter TEST=p197                        # dev profile, all p197*
#   make iter TEST=p194 TFILE=issues           # dev profile, only issues.rs
#   make iter TEST=p197 PROFILE=release        # release profile
#   make iter TEST=introspect TFILE=exit_codes # exit_codes.rs only
iter:
	@if [ -z "$(TEST)" ]; then \
		echo "Usage: make iter TEST=<filter> [TFILE=<test_binary>] [PROFILE=release]"; \
		echo "Examples:"; \
		echo "  make iter TEST=p197"; \
		echo "  make iter TEST=p194 TFILE=issues"; \
		exit 1; \
	fi
	@-rm -f tests/generated/* tests/dumps/*.txt 2>/dev/null
	@TFILE_ARG=$$([ -n "$(TFILE)" ] && echo "--test $(TFILE)" || echo ""); \
	PROFILE_ARG=$$([ "$(PROFILE)" = "release" ] && echo "--release" || echo ""); \
	RUST_BACKTRACE=1 cargo test $$PROFILE_ARG $$TFILE_ARG -- $(TEST) --nocapture


# wasm: build the browser bundle (loft.js + loft_bg.wasm under doc/pkg/)
# via wasm-pack.  Uses the `wasm` feature → pulls in wasm-bindgen → the
# resulting wasm has __wbindgen_placeholder__ imports that wasm-bindgen-cli
# replaces during post-processing.  This is the wasm-pack pipeline; it
# is NOT compatible with the `loft --html` pipeline (see `make game` /
# `make wasm-html-test`), which links a no-wasm-feature rlib instead.
#
# CAVEAT: this overwrites target/wasm32-unknown-unknown/release/libloft.rlib
# with the wasm-bindgen variant.  Re-run `make game` (or
# `make wasm-html-test`) afterwards if you need the --html variant back.
wasm:
	$$HOME/.cargo/bin/wasm-pack build --target web --out-dir doc/pkg --release -- --features wasm --no-default-features
	@./scripts/wasm_bundle_stamp.sh > doc/pkg-src.stamp
	@echo "  wrote doc/pkg-src.stamp ($$(cut -c1-12 doc/pkg-src.stamp)…)"

# @PLN117 — the THREADED gallery bundle: par() over real Web Worker threads.
# Same shape as `make wasm` but with the wasm-threads recipe (see `wasm-mt` for
# the full-flag-set rationale), output to doc/pkg-mt so it does NOT clobber the
# committed single-threaded doc/pkg (which stays the default — no nightly /
# build-std burden on gallery CI).  To deploy a threaded gallery: build this,
# copy doc/pkg-mt over ./pkg on a COOP/COEP host; the playground/gallery loaders
# start loft's pool automatically when crossOriginIsolated.  Needs the same
# nightly + rust-src toolchain as `wasm-mt`.
gallery-mt:
	RUSTFLAGS='$(WASM_MT_RUSTFLAGS)' RUSTC=$(NIGHTLY_RUSTC) \
	rustup run nightly \
	$$HOME/.cargo/bin/wasm-pack build --target web --out-dir doc/pkg-mt --release \
	-- --no-default-features --features wasm-threads -Z build-std=panic_abort,std
	@cp doc/loft-thread.js doc/pkg-mt/
	@echo "Built doc/pkg-mt/ (threaded gallery). Deploy as ./pkg on a COOP/COEP host; serve with 'make serve'."

# gallery: verify-and-rebuild the web gallery end-to-end so it can
# recover from a partially-broken state.  Use this when the browser
# reports errors like "Failed to grow table" (wasm/JS glue mismatch),
# "404 on pkg/loft_bg.wasm" (out-of-tree build), or just after an
# upstream change that invalidates the generated pkg.
#
# Steps (each fails loudly, no silent skips):
#   1. Clean doc/pkg/ entirely so a partial cache cannot hide staleness.
#   2. Check wasm-pack is installed; abort with an actionable message
#      ("cargo install wasm-pack") if not.
#   3. Rebuild the wasm bundle via `make wasm`.
#   4. Verify every file the gallery imports actually exists at the
#      expected path AND is non-empty.
#   5. Verify loft.js and loft_bg.wasm were generated in the SAME run
#      (timestamps within 120s) — a mismatch is the most common source
#      of "failed to grow table" runtime errors.
#   6. Start a transient http.server on a fixed ephemeral port,
#      HEAD-request every asset the gallery loads, fail on non-200.
#   7. Print a one-line "gallery ready" summary with the URL.
#
# After a successful run, `make serve` will work for local browsing.
gallery:
	@echo "  [1/7] cleaning doc/pkg ..."
	@rm -rf doc/pkg
	@echo "  [2/7] checking wasm-pack ..."
	@if [ ! -x "$$HOME/.cargo/bin/wasm-pack" ] && ! command -v wasm-pack >/dev/null 2>&1; then \
		echo "    FAIL: wasm-pack not installed."; \
		echo "    install with: cargo install wasm-pack"; \
		exit 1; \
	fi
	@echo "  [3/7] building wasm bundle ..."
	@$(MAKE) wasm >/tmp/loft_gallery_wasm.log 2>&1 || { \
		echo "    FAIL: wasm-pack build failed — see /tmp/loft_gallery_wasm.log"; \
		tail -20 /tmp/loft_gallery_wasm.log; \
		exit 1; \
	}
	@echo "  [4/7] checking required gallery files ..."
	@missing=0; \
	for f in doc/gallery.html doc/gallery-run.html doc/gallery-examples.js doc/loft-gl.js \
	         doc/pkg/loft.js doc/pkg/loft_bg.wasm doc/pkg/loft.d.ts; do \
		if [ ! -s "$$f" ]; then \
			echo "    FAIL: $$f is missing or empty"; \
			missing=$$((missing + 1)); \
		fi; \
	done; \
	if [ $$missing -gt 0 ]; then exit 1; fi
	@echo "  [5/7] checking wasm/js glue are from the same build ..."
	@js_mtime=$$(stat -c %Y doc/pkg/loft.js); \
	wasm_mtime=$$(stat -c %Y doc/pkg/loft_bg.wasm); \
	delta=$$((wasm_mtime - js_mtime)); \
	delta=$${delta#-}; \
	if [ $$delta -gt 120 ]; then \
		echo "    FAIL: loft.js and loft_bg.wasm timestamps differ by $$delta s"; \
		echo "    One or both is stale — rerun 'make gallery'."; \
		exit 1; \
	fi
	@echo "  [6/7] starting transient http.server and probing assets ..."
	@port=18765; \
	cd doc && python3 -m http.server $$port --bind 127.0.0.1 \
	  >/tmp/loft_gallery_server.log 2>&1 & \
	echo $$! > /tmp/loft_gallery_server.pid; \
	# Give the server a moment to bind the port. \
	for _ in 1 2 3 4 5 6 7 8 9 10; do \
		sleep 0.3; \
		if curl -s -o /dev/null "http://127.0.0.1:$$port/gallery.html"; then break; fi; \
	done; \
	failed=0; \
	for path in /gallery.html /gallery-run.html /gallery-examples.js /loft-gl.js \
	            /pkg/loft.js /pkg/loft_bg.wasm /pkg/loft.d.ts; do \
		code=$$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$$port$$path"); \
		if [ "$$code" != "200" ]; then \
			echo "    FAIL: http://127.0.0.1:$$port$$path returned $$code"; \
			failed=$$((failed + 1)); \
		fi; \
	done; \
	kill $$(cat /tmp/loft_gallery_server.pid) 2>/dev/null || true; \
	wait $$(cat /tmp/loft_gallery_server.pid) 2>/dev/null || true; \
	rm -f /tmp/loft_gallery_server.pid /tmp/loft_gallery_server.log; \
	if [ $$failed -gt 0 ]; then exit 1; fi
	@# Inject no-cache meta + content-hash version on every local asset
	@# reference so post-deploy browsers fetch fresh.  See
	@# scripts/cache_bust_html.py for rationale.
	@python3 scripts/cache_bust_html.py >/dev/null
	@echo "  [7/7] gallery ready — run 'make serve' and open http://localhost:8000/gallery.html"

# @PLN117 — COOP/COEP so a threaded gallery bundle (`make gallery-mt`) gets
# crossOriginIsolated === true and par() runs on Web Workers.  Harmless for the
# default single-threaded bundle (the gallery is self-contained / same-origin).
# Reuses the cross-origin-isolated static server built for the threaded-wasm
# harness.  Without these headers the gallery still runs — par() just sequential.
serve:
	@echo "Playground: http://localhost:8000/playground.html"
	@echo "Gallery:    http://localhost:8000/gallery.html"
	@echo "(COOP/COEP on — a threaded gallery from 'make gallery-mt' runs par() on Web Workers)"
	python3 tests/wasm/coi-server.py 8000 doc

# ── Branch review viewer (plan-35) ─────────────────────────────
# Serves a branch-aware doc + code review dashboard.
# See doc/claude/plans/35-branch-review-viewer/.
#
# Phase 01 ships INTERPRETER MODE (target/release/loft --interpret).
# Native mode is blocked by P262 (text-call inline-arg codegen quirk)
# + a separate lib/web duplicate-native-fn issue when lib/server is
# pulled transitively.  Phase 07 closeout revisits frozen-binary
# packaging once those blockers close.
#
# view-build: ensure host loft is built; record build provenance.
# view:       refresh state, then run script via loft --interpret.
view-build:
	@echo "  [1/2] building host loft binary ..."
	@cargo build --release -q --lib --bin loft 2>/tmp/loft_view_host.log || { \
	    echo "    FAIL: host cargo build — see /tmp/loft_view_host.log"; \
	    tail -20 /tmp/loft_view_host.log; exit 1; }
	@echo "  [2/2] recording build provenance ..."
	@host_sha=$$(git rev-parse --short HEAD 2>/dev/null || echo "unknown"); \
	{ echo "loft-view phase 01 — interp-mode build"; \
	  echo "Built $$(date -u +%Y-%m-%dT%H:%M:%SZ) against loft commit $$host_sha"; \
	  echo ""; \
	  echo "Native compilation blocked by:"; \
	  echo "  P262 — text-returning calls passed inline get extra & wrap"; \
	  echo "  (lib/web duplicate native fn defs in --native compile)"; \
	  echo ""; \
	  echo "Runs via 'loft --interpret' until those blockers close."; \
	} > tools/viewer/BUILD_NOTES.md
	@echo "loft-view ready: tools/viewer/src/main.loft (interp-mode)"
	@echo "  See tools/viewer/BUILD_NOTES.md for native-mode blockers."

# @PLN119 arc F — the state dump is loft now, not bash.  It replaced
# `tools/viewer/refresh.sh`, which existed only because loft could not call
# `git`; `lib/git` is that call as a typed library, so the viewer's state is
# produced by the same language the viewer is written in — and `jq` is no longer
# needed to run the dashboard.
view-refresh:
	@if [ ! -x target/release/loft ]; then \
	    echo "host loft binary missing; run: make view-build"; \
	    exit 1; \
	fi
	@./target/release/loft --interpret --lib lib tools/viewer/refresh.loft

# ── Tracker-tag indexer (plan-37) ───────────────────────────────
# Scans the repo for @P-id / @PLAN-id references, writes
# index/tags.json.  See doc/claude/plans/37-tracker-index/.
# CLI query wrapper (`scripts/idx`) lands in plan-37 phase 01.
index:  ## Refresh index/tags.json via the loft scanner
	@# @PLAN37 phase 07 sub-commits A.5→J: scan.loft is now the
	@# sole canonical scanner; the legacy bash scan.sh + the
	@# `index-bash` / `index-loft` fallback targets were removed
	@# in sub-commit J after sub-commit H (cutover) soaked one
	@# CI cycle on main.  scan.loft writes the JSON-object form
	@# of tags.json to stdout (via `LOFT_INDEX_BUCKETED=1`) and
	@# the summary stats line to stderr — the shell redirect
	@# captures one, the terminal shows the other.  The
	@# `--no-warnings` flag (@P282 close) keeps stdout free of
	@# the loft compiler's warning preamble.
	@if [ ! -x target/release/loft ]; then \
	    echo "host loft binary missing; run: cargo build --release"; exit 1; \
	fi
	@mkdir -p index
	@# `--native-release` (rather than bare `--native`) instructs rustc
	@# to emit `-O` (opt-level=2) and the loft codegen to emit only
	@# reachable functions.  For a hot loop like scan.loft the
	@# difference is dramatic: scan loop 1.7s → 165ms, total 5s → 1.3s
	@# warm-cache.  Cold compile costs ~6s but the per-source cache
	@# (tools/indexer/src/.loft/cache/) survives across runs, so the
	@# everyday `make index` invocation is the warm path.
	@LOFT_INDEX_BUCKETED=1 ./target/release/loft --native-release --no-warnings --lib lib/ \
	    tools/indexer/src/scan.loft > index/tags.json

index-install-hook:
	@./tools/indexer/install-hook.sh

# Point git at the repo's checked-in hooks.  Currently one: `commit-msg` reminds you
# that an issue referenced with `Refs #N` (or a bare `#N`) is NOT labelled
# fixed-pending-merge by the push workflow, which reads `Fixes|Closes|Resolves #N`.
hooks:
	@git config core.hooksPath .githooks
	@echo "core.hooksPath = .githooks">&2

# ── @I81 · @PLN92 feature catalogue sync (strand 3) — sync tooling ──
# The `loft-lang/features` issues are the canonical, self-contained docs; these
# targets keep the in-project shadow one-way: the mirror (doc/features/) that
# agents grep + scan.loft indexes, and the runnable examples (tests/docs/features/)
# that CI runs cross-backend.  See doc/claude/plans/92-feature-catalogue/.
FEATURES_REPO ?= loft-lang/features

surface-gen:  ## Regenerate index/target_surface.json (which builtins exist per target)
	@python3 scripts/gen_target_surface.py

surface-check:  ## Drift guard: fail if the committed per-target surface is stale
	@python3 scripts/gen_target_surface.py --check

features-fetch:  ## Refresh index/features.json from the loft-lang/features tracker (network; gh + jq)
	@gh issue list -R $(FEATURES_REPO) --state all --limit 200 \
	    --json number,title,labels,body \
	  | jq 'sort_by(.number) | map({number, title, kind: ((.labels|map(.name)|map(select(startswith("kind:")))|.[0]) // "kind:unknown" | sub("^kind:";"")), body})' \
	  > index/features.json
	@echo "index/features.json: $$(jq length index/features.json) issues"

features-gen:  ## Regenerate the shadow (doc/features/ + tests/docs/features/) from the snapshot
	@if [ ! -x target/release/loft ]; then \
	    echo "host loft binary missing; run: cargo build --release --bin loft"; exit 1; \
	fi
	@rm -rf doc/features tests/docs/features
	@./target/release/loft --interpret tools/features/gen.loft

# The scope must name EVERY file `features-gen` writes.  It listed the two directories
# and not `tests/docs/33-features.loft` — the published chapter — so a hand-edit to the
# page that ships in all four release bundles was regenerated away in the checking tree
# and reported as "in sync", while the committed copy kept whatever it had been given.
# The chapter's own text promises this guard catches exactly that.
features-check: features-gen  ## Drift guard: fail if the committed shadow is stale vs the snapshot
	@out=$$(git status --porcelain -- doc/features tests/docs/features tests/docs/33-features.loft); \
	if [ -n "$$out" ]; then \
	    echo "ERROR: the generated catalogue drifted from index/features.json."; \
	    echo "Run 'make features-gen' and commit the result. Offending paths:"; \
	    echo "$$out"; \
	    exit 1; \
	fi
	@echo "features shadow in sync with index/features.json."

examples-index:  ## Regenerate examples-index.tsv (worked-example tag -> file:line -> blob link)
	@bash scripts/check_doc_drift.sh write-examples-index

# The tag checks ADVISE in a library repo (their rules come from loft, so a gate there
# reddens a PR for a change loft made).  This is how you get a real pass/fail back before
# pushing: it gates the citation faults CI would report — dangling / duplicate /
# unregistered — without demanding an `examples-index.tsv`, which libraries no longer
# commit.  REPO=. checks loft itself.
# The LOCAL fast gate — same tests, ~1.9x quicker, by widening the two serial groups
# that exist for the 3-core CI runner.  See `.config/nextest.toml [profile.fast]` for the
# measurement and, more importantly, for what it does NOT prove: those groups close a
# starvation window, so re-run a networked failure under `--profile ci` before believing
# it.  `make ci` is unchanged and stays the pre-push gate.
test-fast:  ## The suite under the local `fast` profile (~1.9x quicker than `make ci`'s)
	@bash scripts/box-claim.sh cargo nextest run --profile fast

examples-preflight:  ## Would a PR report anything on worked-example tags? (REPO=../loft-libs-x)
	@EXAMPLES_REPO_ROOT=$(REPO) bash scripts/check_doc_drift.sh examples-preflight

# REPO defaults to this repo; point it at a library checkout to drive that repo's
# rollout: make examples-progress REPO=../loft-libs-graphics
REPO ?= .
.PHONY: test-fast examples-index examples-preflight examples-progress features-review libraries-review bug-review release-checklist release-gate reference-review skills-review clippy-review
examples-progress:  ## Worked-example rollout REPORT: which packages still owe a verdict (never a gate)
	@EXAMPLES_REPO_ROOT=$(REPO) bash scripts/check_doc_drift.sh examples-progress

# Monthly release aid, NOT a gate and NOT in CI (CI_BUDGET.md's 20-minute rule).
# Answers the only two questions a program can answer about the catalogue — what is
# structurally missing, and which entries the cycle actually touched — so the agent's
# read is bounded to those instead of all 82.  Whether an entry is self-explanatory and
# whether its example still demonstrates it are judgements, and stay an agent task.
#   make features-review                     # what is missing
#   make features-review SINCE=<watermark>   # + what to re-read this cycle
features-review:  ## Feature-catalogue review aid: what is missing + what to re-read (SINCE=<ref>)
	@FEATURES_SINCE=$(SINCE) bash scripts/check_doc_drift.sh features-progress

# The LIBRARY half of the same monthly pass (LIBRARY_DOC_REVIEW.md).  Same two questions,
# same non-gate status; the difference is where the baseline comes from.  Libraries are
# deliberately OFF the release axis (RELEASE.md § What forces a release), so they move on
# their own cadence and one global SINCE would be meaningless across thirty-four packages
# in eight repos -- each library carries its OWN watermark in LIBRARY_DOC_REVIEW.md's
# table, and the aid diffs each against that.  Reads the local catalogue snapshots, so:
#   make libcatalogue && make libraries-review
libraries-review:  ## Library review aid: which libraries owe a review + which ones moved
	@bash scripts/check_doc_drift.sh libraries-progress

# The THIRD monthly pass (BUG_REVIEW.md): the two above ask whether the docs still
# describe the code; this one asks whether the month's bugs share a cause worth
# collapsing.  Same non-gate status, same monthly beat, and equally not in CI -- it
# needs the network (gh) and a month of evidence, neither of which belongs in a
# 20-minute PR budget (CI_BUDGET.md).  It REPORTS four things and judges none: the
# population, each mechanism class's share over time, whether keystones already landed
# actually moved their class, and which IR variants hand-written walkers omit.  Which
# rising class is worth one generalization is the judgement, and stays an agent task.
#   make bug-review                       # fetch from gh and report
#   make bug-review ARGS="--bands 6"      # finer slicing on a busy cycle
bug-review:  ## Monthly bug-review aid: which mechanism classes are still producing bugs
	@python3 scripts/bug-review.py $(ARGS)

# The per-release checklist: what a HUMAN still has to do, with everything the machine
# can decide already decided.  RELEASE.md holds the prose and three partial lists; this
# is the single worked-through list, generated so it cannot drift from the repo.
# Automatic items are MEASURED on every run and cannot be ticked (a gate you can tick is
# a gate that gets ticked); manual items carry the exact command and what counts as a
# pass, and are the only ones `--done` accepts.  Items for work this release did not
# touch (the VS Code pass, the native-debug gate) stay hidden.
# A REPORT plus local state — never a gate, and it never tags or publishes anything.
#   make release-checklist                            # the list for Cargo.toml's version
#   make release-checklist ARGS="--fetch"             # refresh origin/main + tags first
#   make release-checklist ARGS="--done M-install-sh --note 'ran on the NUC'"
# `|| true`: the script exits 1 while an automatic check is FAILING, which is the
# answer a caller wants ("is this release ready?") and the wrong thing for make to
# render as a broken target.  A report says what it found; it does not stop the build.
release-checklist:  ## Per-release checklist: what CI proved, and what is left for a human
	@python3 scripts/release-checklist.py $(ARGS) || true

# The liveness census (@PLN156): are the gates themselves still live?  Suppressions
# justified by CLOSED issues, gate workflows that quietly stopped firing, checklist
# items never run in any recorded cycle.  A REPORT, never a gate — the per-release
# reader is the M-liveness checklist item; `ARGS="--no-network"` for the offline half.
.PHONY: release-liveness
release-liveness:  ## Census: stale suppressions, gates that stopped firing, steps never run
	@python3 scripts/release-liveness.py $(ARGS)

# Every nightly, run deliberately against THIS commit in one CI run that ends in one
# verdict — the release evidence RELEASE.md § The nightlies asks for, on demand instead
# of on GitHub's schedule (whose 03:00 daily has started anywhere from 03:34 to 14:45
# UTC, on whatever `main` was at that moment).  Dispatches `release-gate.yml` on the
# current branch, which must be PUSHED with HEAD at its tip — a dispatch runs the commit
# GitHub holds, and `release-checklist` accepts only a run for HEAD's sha — then waits
# (~60–90 min).  Exit status is the verdict.  Never tags, drafts or publishes.
#   make release-gate
#   make release-gate ARGS=--no-wait      # dispatch and return
release-gate:  ## Run every nightly against this commit in one CI run (the release evidence)
	@bash scripts/release-gate.sh $(ARGS)

# The pass that validates what the reference PROMISES — the half `A-pdf*` cannot reach.
# Those checks establish the document is whole, current and correctly versioned; all
# three stay green on a chapter describing behaviour the language dropped two releases
# ago.  Continuous by design (watermark per chapter, like `libraries-review`): read a
# chapter the week its source moves, and the tag-day list is short by construction.
#   make reference-review                                   # what owes a read
#   make reference-review ARGS=--verbose                    # + the commits behind each
#   make reference-review ARGS="--done tests/docs/07-vector.loft"
reference-review:  ## Which reference chapters owe a human read (and which have MOVED)
	@python3 scripts/reference-review.py $(ARGS)

# The same pass for the agent skills (.claude/skills/): a skill is loaded INSTEAD of
# the canonical doc it paraphrases, and those docs move daily while the skill is only
# edited when someone notices.  Mechanical half is outright (cited paths, make targets
# and LOFT_* switches must resolve); the content/usability/conciseness read is by hand,
# per skill, watermarked so it happens the week a skill's sources move — SKILLS_REVIEW.md.
#   make skills-review                                # what owes a read
#   make skills-review ARGS=--verbose                 # + the commits behind each
#   make skills-review ARGS="--done loft-test"        # record one as validated
skills-review:  ## Which agent skills owe a human read (and which moved under their sources)
	@python3 scripts/skills-review.py $(ARGS)

# RELEASE.md § 8, measured instead of grepped.  Every `#[allow(clippy::…)]` under
# src/ becomes an `#[expect]` in a throwaway worktree and clippy runs the way CI
# runs it, so the compiler itself names each suppression nothing fulfils — the
# function that shrank under the line limit, the parameter that was removed —
# beside whether anything on or above the line says why it is there.  A REPORT,
# never a gate, and never a cleanup: the checkout is not edited.  Builds under
# target/clippy-review, so it neither touches nor waits on a running gate.
#   make clippy-review                        # CI's three clippy legs, ~1 min warm
#   make clippy-review ARGS="--legs all"      # + debug-assertions ON + wasm32: what CI never lints
clippy-review:  ## Which clippy suppressions are dead, and which are live but unexplained
	@python3 scripts/clippy-review.py $(ARGS)

# `doc/claude/plans/**/probes/` holds ~860 executable `.loft` files that no suite
# reaches — the residue of finished investigations, still compiling and running
# long after their plan closed.  loft#1113 (a months-old SIGSEGV) surfaced only
# because an unrelated change happened to walk that directory and run one.
# A REPORT, never a gate: some of these probes fault ON PURPOSE, and the sweep
# scores crash channels only — it cannot say whether a probe computed the right
# answer, because these files carry no expected values.
#   make doc-probes                       # sweep doc/ on the release binary
#   make doc-probes ARGS="--jobs 12"      # or --dir <subdir>, --tsv <out>
doc-probes:  ## Run every checked-in .loft under doc/ and report the hard faults
	@cargo build --release --bin loft
	@./scripts/doc_probe_sweep.sh $(ARGS)

api-compat:  ## @PLN102 — check bundled api-surface baselines are still a drop-in (CI: red, non-blocking)
	@cargo build --release --bin loft
	@rc=0; for base in tests/fixtures/api_compat/*.api-baseline; do \
	    src="$${base%.api-baseline}.loft"; \
	    echo "== api-compat: $$src =="; \
	    target/release/loft api-surface --check "$$base" "$$src" || rc=1; \
	done; \
	if [ $$rc -ne 0 ]; then \
	    echo "NOT a drop-in. On an INTENTIONAL change, regenerate: loft api-surface <lib> --emit-baseline > <lib>.api-baseline"; \
	fi; \
	exit $$rc

check-contract-goldens:  ## @PLN102 flip-gate Gate 1+2 — a frozen golden (layout/behaviour) changed ⇒ CONTRACT_VERSION bump (inert at 0; CI: red, non-blocking)
	@scripts/check_contract_goldens.sh --self-test
	@scripts/check_contract_goldens.sh "$${BASE_REF:-origin/main}"

view: view-refresh
	@if [ ! -f tools/viewer/src/main.loft ]; then \
	    echo "loft-view source missing; expected tools/viewer/src/main.loft"; \
	    exit 1; \
	fi
	@if [ ! -x target/release/loft ]; then \
	    echo "host loft binary missing; run: make view-build"; \
	    exit 1; \
	fi
	# Default to --native-release (rustc -O).  Bare --native runs
	# unoptimised generated Rust — for an HTTP server that handles
	# repeated requests, the per-request cost difference is large
	# (10× on hot loops; see PERFORMANCE.md § Open work).  Cold
	# compile is ~6s; cached binary survives across restarts via
	# tools/viewer/src/.loft/cache/.
	# @P274 closed 2026-05-14 (use-after-free in
	# patch_hoisted_returns Pass 2 + text-concat type-dispatch in
	# parse_append_text).
	# `LOFT_VIEW_INTERP=1 make view` falls back to the interpreter
	# (useful when bisecting a native-only regression).
	@if [ -n "$$LOFT_VIEW_INTERP" ]; then \
	    ./target/release/loft --interpret --lib lib/ tools/viewer/src/main.loft; \
	else \
	    ./target/release/loft --native-release --lib lib/ tools/viewer/src/main.loft; \
	fi

# game: rebuild the efficient browser build of Brick Buster from any
# state — clean rebuild of the wasm32-unknown-unknown rlibs + host
# binary + `loft --html`, then publish the resulting self-contained
# HTML to doc/brick-buster.html.  Use when the check-in looks broken
# or after an upstream change that invalidates either the wasm rlib
# or the host tooling.
#
# Steps (each fails loudly, no silent skips):
#   1. Rebuild the host binary so --html and its native_utils helpers
#      are current.
#   2. Ensure the wasm32-unknown-unknown target is installed.
#   3. Rebuild the wasm32-unknown-unknown libloft.rlib + deps via
#      `make wasm-assets`; this is the ingredient `--html` links
#      against and is the single most common source of "W1.1"
#      compile failures.
#   4. Verify libloft.rlib exists for both wasm32 and the host
#      (proc-macros need the host deps dir).
#   5. Run `loft --html doc/brick-buster.html ...25-brick-buster.loft`.
#   6. Sanity-check the output HTML: doctype + loft_start + > 5kB.
#   7. Print the file:// URL so the user can click through.
# ── Brick Buster's sprite pack ─────────────────────────────────────
#
# The atlas is drawn ONCE, into a content pack the game reads, rather
# than by 180 lines of fill_rect on every launch (@PLN146 F4).  Both
# `make game` and `make play` need the pack on disk first.
#
# `loft build` has no assets-only mode — it would compile-check the
# whole game to reach the asset phase — so this runs the script
# directly.  That makes the script path a SECOND spelling of the
# `run` in tools/brick-buster/loft.toml's `[[build.asset]]`, and
# `doc_hygiene::brick_buster_pack_step_matches_its_manifest` pins the
# two together so they cannot drift apart silently.
.PHONY: brick-buster-pack
brick-buster-pack:
	@echo "  drawing Brick Buster's sprite pack ..."
	@./target/release/loft --interpret tools/brick-buster/pack_atlas.loft \
	    >/tmp/loft_bb_pack.log 2>&1 || { \
	    echo "    FAIL: sprite pack — see /tmp/loft_bb_pack.log"; \
	    tail -20 /tmp/loft_bb_pack.log; exit 1; }
	@test -s tools/brick-buster/assets/bb.blobs.store || { \
	    echo "    FAIL: pack_atlas.loft produced no assets/bb.blobs.store"; exit 1; }

game:
	@echo "  [1/7] building host binary + libloft.rlib ..."
	@# `--bin loft` alone does not always produce the top-level
	@# libloft.rlib that step 4 requires for proc-macro lookup;
	@# building both explicitly guarantees both artefacts exist.
	@cargo build --release -q --lib --bin loft || { echo "    FAIL: host cargo build"; exit 1; }
	@echo "  [2/7] checking wasm32-unknown-unknown target ..."
	@rustup target list --installed 2>/dev/null | grep -q wasm32-unknown-unknown || { \
	    echo "    FAIL: rustup target not installed"; \
	    echo "    install with: rustup target add wasm32-unknown-unknown"; \
	    exit 1; }
	@echo "  [3/7] rebuilding wasm32-unknown-unknown rlibs ..."
	@cargo build --release -q --target wasm32-unknown-unknown --lib --no-default-features --features random \
	    >/tmp/loft_game_wasm.log 2>&1 || { \
	    echo "    FAIL: wasm rlib build — see /tmp/loft_game_wasm.log"; \
	    tail -20 /tmp/loft_game_wasm.log; exit 1; }
	@echo "  [4/7] verifying libloft.rlib for both targets ..."
	@test -f target/wasm32-unknown-unknown/release/libloft.rlib || { \
	    echo "    FAIL: target/wasm32-unknown-unknown/release/libloft.rlib missing"; exit 1; }
	@test -f target/release/libloft.rlib || { \
	    echo "    FAIL: target/release/libloft.rlib missing (needed for proc-macros)"; exit 1; }
	@echo "  [5/7] compiling Brick Buster to self-contained HTML ..."
	@$(MAKE) --no-print-directory brick-buster-pack
	@./target/release/loft --html doc/brick-buster.html \
	    --path "$$(pwd)/" --lib "$$(pwd)/lib/" \
	    tools/brick-buster/25-brick-buster.loft \
	    >/tmp/loft_game_html.log 2>&1 || { \
	    echo "    FAIL: --html compilation — see /tmp/loft_game_html.log"; \
	    tail -30 /tmp/loft_game_html.log; exit 1; }
	@echo "  [6/7] sanity-checking HTML output ..."
	@test -f doc/brick-buster.html || { echo "    FAIL: doc/brick-buster.html not created"; exit 1; }
	@size=$$(stat -c %s doc/brick-buster.html 2>/dev/null || stat -f %z doc/brick-buster.html); \
	if [ $$size -lt 5000 ]; then \
	    echo "    FAIL: doc/brick-buster.html is only $$size bytes (expected > 5000)"; exit 1; \
	fi; \
	grep -q "<!DOCTYPE html>" doc/brick-buster.html || { echo "    FAIL: missing DOCTYPE"; exit 1; }; \
	grep -q "loft_start" doc/brick-buster.html || { echo "    FAIL: missing loft_start entry"; exit 1; }
	@# Integrity gate (@P337): a size/DOCTYPE check is NOT enough — the two
	@# ways this bundle silently breaks are (a) a STOMPED rlib (wasm-bindgen
	@# placeholder imports → won't instantiate) and (b) MISSING wasm-opt
	@# (no asyncify → render loop hangs the tab).  Both pass the size check.
	@# This instantiates the embedded wasm with stub imports and asserts the
	@# import/export shape, so a broken bundle fails the build loudly instead
	@# of shipping.  Requires node (already a dev dependency for wasm tests).
	@if command -v node >/dev/null 2>&1; then \
	    node tools/check_html_bundle.mjs doc/brick-buster.html || exit 1; \
	else \
	    echo "    WARN: node not found — skipping bundle integrity check (install node to enable)"; \
	fi
	@# Inject no-cache meta + content-hash version on every local
	@# asset reference so post-deploy browsers fetch fresh.  See
	@# scripts/cache_bust_html.py for rationale (GitHub Pages doesn't
	@# let us set custom HTTP headers; the only lever is the HTML).
	@python3 scripts/cache_bust_html.py >/dev/null
	@echo "  [7/7] Brick Buster ready."
	@echo ""
	@echo "    Open in your browser:"
	@echo "      file://$$(pwd)/doc/brick-buster.html"
	@echo ""
	@echo "    Or serve locally:"
	@echo "      make serve  →  http://localhost:8000/brick-buster.html"

# crystal-editor: build the stand-alone audience crystal editor
# (tools/audience-demo/crystal_editor.loft) to a self-contained browser
# page doc/crystal-editor.html — the GitHub Pages demo.  Mirrors `make
# game`: host binary + wasm rlib, then `loft --html` (which bundles the
# WebGL backend lib/graphics/js/loft-gl.js), then cache-bust.
crystal-editor:
	@echo "  [1/5] building host binary + libloft.rlib ..."
	@cargo build --release -q --lib --bin loft || { echo "    FAIL: host cargo build"; exit 1; }
	@echo "  [2/5] checking wasm32-unknown-unknown target ..."
	@rustup target list --installed 2>/dev/null | grep -q wasm32-unknown-unknown || { \
	    echo "    FAIL: rustup target not installed"; \
	    echo "    install with: rustup target add wasm32-unknown-unknown"; \
	    exit 1; }
	@echo "  [3/5] rebuilding wasm32-unknown-unknown rlibs ..."
	@cargo build --release -q --target wasm32-unknown-unknown --lib --no-default-features --features random \
	    >/tmp/loft_crystal_wasm.log 2>&1 || { \
	    echo "    FAIL: wasm rlib build — see /tmp/loft_crystal_wasm.log"; \
	    tail -20 /tmp/loft_crystal_wasm.log; exit 1; }
	@echo "  [4/5] compiling Crystal Editor to self-contained HTML ..."
	@./target/release/loft --html doc/crystal-editor.html \
	    --path "$$(pwd)/" --lib "$$(pwd)/lib/" \
	    tools/audience-demo/crystal_editor.loft \
	    >/tmp/loft_crystal_html.log 2>&1 || { \
	    echo "    FAIL: --html compilation — see /tmp/loft_crystal_html.log"; \
	    tail -30 /tmp/loft_crystal_html.log; exit 1; }
	@test -f doc/crystal-editor.html || { echo "    FAIL: doc/crystal-editor.html not created"; exit 1; }
	@size=$$(stat -c %s doc/crystal-editor.html 2>/dev/null || stat -f %z doc/crystal-editor.html); \
	if [ $$size -lt 5000 ]; then \
	    echo "    FAIL: doc/crystal-editor.html is only $$size bytes (expected > 5000)"; exit 1; \
	fi; \
	grep -q "<!DOCTYPE html>" doc/crystal-editor.html || { echo "    FAIL: missing DOCTYPE"; exit 1; }; \
	grep -q "loft_start" doc/crystal-editor.html || { echo "    FAIL: missing loft_start entry"; exit 1; }
	@python3 scripts/cache_bust_html.py >/dev/null
	@echo "  [5/5] Crystal Editor ready."
	@echo ""
	@echo "    Open in your browser:"
	@echo "      file://$$(pwd)/doc/crystal-editor.html"
	@echo ""
	@echo "    Or serve locally:"
	@echo "      make serve  →  http://localhost:8000/crystal-editor.html"

# play: validate everything needed for a native OpenGL run of Brick
# Buster, then launch the game.  Prerequisites checked in order so
# the first missing item fails fast with an actionable message.
play:
	@echo "  [1/5] checking loft binary + libloft.rlib ..."
	@# Build both --lib and --bin so libloft.rlib (consumed by the
	@# /tmp/loft_native.rs compile in step 5) matches the current
	@# transitive dep set.  Building only --bin can leave a stale
	@# libloft.rlib referencing an older rand_core and rustc fails
	@# with E0460 "found possibly newer version of crate `rand_core`".
	@cargo build --release -q --lib --bin loft 2>/tmp/loft_play_host.log || { \
	    echo "    FAIL: host cargo build — see /tmp/loft_play_host.log"; \
	    tail -20 /tmp/loft_play_host.log; exit 1; }
	@echo "  [2/5] checking system GL libraries ..."
	@if command -v pkg-config >/dev/null 2>&1; then \
	    pkg-config --exists gl || { \
	        echo "    FAIL: OpenGL development headers not found"; \
	        echo "    install:  apt install libgl1-mesa-dev  (debian/ubuntu)"; \
	        echo "              dnf install mesa-libGL-devel  (fedora)"; \
	        echo "              brew install mesa             (macos)"; \
	        exit 1; }; \
	else \
	    echo "    note: pkg-config not found; trusting rustc to link GL"; \
	fi
	@echo "  [3/5] building native graphics cdylib ..."
	@# Try once; on E0460 (stale incremental rmeta) auto-clean and retry
	@# once before surfacing the failure.  This is a pure cargo caching
	@# artefact that otherwise presents as "can't find crate / found
	@# possibly newer version of crate X" and blocks the first-time run
	@# on any checkout where the subcrate was built against different
	@# deps than the current workspace.
	@cd lib/graphics/native && cargo build --release -q 2>/tmp/loft_play_graphics.log || { \
	    if grep -qE 'E0460|E0463' /tmp/loft_play_graphics.log; then \
	        echo "    stale incremental build detected, running cargo clean + retry ..."; \
	        cd lib/graphics/native && cargo clean -q >/dev/null 2>&1; \
	        cd lib/graphics/native && cargo build --release -q 2>>/tmp/loft_play_graphics.log || { \
	            echo "    FAIL: lib/graphics/native build after clean — see /tmp/loft_play_graphics.log"; \
	            tail -30 /tmp/loft_play_graphics.log; exit 1; }; \
	    else \
	        echo "    FAIL: lib/graphics/native build — see /tmp/loft_play_graphics.log"; \
	        tail -30 /tmp/loft_play_graphics.log; \
	        echo ""; \
	        echo "    Common causes:"; \
	        echo "      - missing X11 / Wayland dev headers (libx11-dev, libwayland-dev)"; \
	        echo "      - missing GLFW system dependency"; \
	        exit 1; \
	    fi; }
	@test -f lib/graphics/native/target/release/libloft_graphics_native.so \
	    -o -f lib/graphics/native/target/release/libloft_graphics_native.dylib \
	    -o -f lib/graphics/native/target/release/loft_graphics_native.dll || { \
	    echo "    FAIL: native graphics cdylib missing after build"; exit 1; }
	@echo "  [4/5] checking display available ..."
	@if [ -z "$$DISPLAY" ] && [ -z "$$WAYLAND_DISPLAY" ]; then \
	    echo "    FAIL: no \$$DISPLAY or \$$WAYLAND_DISPLAY set"; \
	    echo "    headless? prefix the command with 'xvfb-run -a' or run on a desktop session"; \
	    exit 1; \
	fi
	@echo "  [5/5] launching Brick Buster ..."
	@echo ""
	@echo "    Controls: ←/→ or A/D to move, Space to launch, Esc to quit"
	@echo ""
	@# --native-release (rustc -O) for the game frame loop; bare
	@# --native runs unoptimised generated Rust and burns frame
	@# budget on call-ABI / null-sentinel bookkeeping the optimiser
	@# normally elides.  Cold compile is ~6s; cached binary survives
	@# across runs.
	@$(MAKE) --no-print-directory brick-buster-pack
	@./target/release/loft --native-release \
	    --path "$$(pwd)/" --lib "$$(pwd)/lib/" \
	    tools/brick-buster/25-brick-buster.loft

# ── Native Moros editor ────────────────────────────────────────────
#
# make native-editor  — build + run from source.  Same checklist as
#                       `make play` (host loft + graphics cdylib +
#                       display server).  Opens a 1024x768 window
#                       with the 7x7 starter map.
#
# make editor-dist    — package a relocatable `dist/moros-editor/`
#                       directory: the optimised binary, the
#                       loft_graphics_native.so, any fonts / assets,
#                       and `rpath=$$ORIGIN` so the binary finds the
#                       .so without LD_LIBRARY_PATH.  A user can copy
#                       the entire `dist/moros-editor/` dir anywhere
#                       and run `./moros-editor` without having loft
#                       or the graphics cdylib installed.
native-editor:
	@echo "  [1/3] building loft binary + cdylib ..."
	@cargo build --release -q --lib --bin loft 2>/tmp/loft_editor_host.log || { \
	    echo "    FAIL: host cargo build — see /tmp/loft_editor_host.log"; \
	    tail -20 /tmp/loft_editor_host.log; exit 1; }
	@cd lib/graphics/native && cargo build --release -q 2>/tmp/loft_editor_graphics.log || { \
	    echo "    FAIL: lib/graphics/native — see /tmp/loft_editor_graphics.log"; \
	    tail -20 /tmp/loft_editor_graphics.log; exit 1; }
	@echo "  [2/3] checking display available ..."
	@if [ -z "$$DISPLAY" ] && [ -z "$$WAYLAND_DISPLAY" ]; then \
	    echo "    FAIL: no \$$DISPLAY / \$$WAYLAND_DISPLAY set"; \
	    echo "    run on a desktop session or prefix with 'xvfb-run -a'"; \
	    exit 1; \
	fi
	@echo "  [3/3] launching Moros editor ..."
	@echo "    Controls: WASD move / Arrows camera / 1-6 tools / Ctrl-Z undo"
	@echo "              Left-click paint / F5 save / F9 load / F11 fullscreen / Esc quit"
	@# --native-release for the editor's UI / paint frame loop —
	@# same rationale as `make play`.  Cached binary survives across
	@# runs via lib/graphics/examples/.loft/cache/.
	@./target/release/loft --native-release \
	    --path "$$(pwd)/" --lib "$$(pwd)/lib/" \
	    lib/graphics/examples/moros_editor.loft

editor-dist:
	@echo "  [1/5] building loft binary + cdylib (release-optimised) ..."
	@cargo build --release -q --lib --bin loft 2>/tmp/loft_dist_host.log || { \
	    echo "    FAIL: host cargo build — see /tmp/loft_dist_host.log"; \
	    tail -20 /tmp/loft_dist_host.log; exit 1; }
	@cd lib/graphics/native && cargo build --release -q 2>/tmp/loft_dist_graphics.log || { \
	    echo "    FAIL: graphics cdylib — see /tmp/loft_dist_graphics.log"; \
	    tail -20 /tmp/loft_dist_graphics.log; exit 1; }
	@echo "  [2/5] compiling native_editor.loft with --native-release ..."
	@./target/release/loft --native-release \
	    --path "$$(pwd)/" --lib "$$(pwd)/lib/" \
	    --native-emit /tmp/moros_editor_dist.rs \
	    lib/graphics/examples/moros_editor.loft >/dev/null
	@# Find the cached binary — `loft --native` caches into
	@# <script_dir>/.loft/cache/<stem>-<hash>.  The hash changes
	@# with source, so a glob finds the freshest file.
	@CACHED=$$(ls -t lib/graphics/examples/.loft/cache/moros_editor-* 2>/dev/null | head -1); \
	if [ -z "$$CACHED" ]; then \
	    echo "    FAIL: loft --native-release did not produce a cached binary"; \
	    echo "    looked in lib/graphics/examples/.loft/cache/"; \
	    exit 1; \
	fi; \
	echo "    cached binary: $$CACHED"; \
	echo "  [3/5] assembling dist/moros-editor/ ..."; \
	rm -rf dist/moros-editor; \
	mkdir -p dist/moros-editor/assets; \
	cp "$$CACHED" dist/moros-editor/moros-editor; \
	cp lib/graphics/native/target/release/libloft_graphics_native.so \
	    dist/moros-editor/ 2>/dev/null || \
	cp lib/graphics/native/target/release/libloft_graphics_native.dylib \
	    dist/moros-editor/ 2>/dev/null || \
	cp lib/graphics/native/target/release/loft_graphics_native.dll \
	    dist/moros-editor/ 2>/dev/null || { \
	    echo "    FAIL: graphics cdylib artefact missing"; exit 1; }; \
	cp lib/graphics/examples/DejaVuSans-Bold.ttf \
	    dist/moros-editor/assets/; \
	echo "  [4/5] patching rpath = \$$ORIGIN (Linux / macOS) ..."; \
	if command -v patchelf >/dev/null 2>&1; then \
	    patchelf --set-rpath '$$ORIGIN' dist/moros-editor/moros-editor \
	        && echo "    patchelf ok"; \
	elif command -v install_name_tool >/dev/null 2>&1; then \
	    install_name_tool -add_rpath @executable_path \
	        dist/moros-editor/moros-editor 2>/dev/null \
	        && echo "    install_name_tool ok"; \
	else \
	    echo "    note: neither patchelf nor install_name_tool available"; \
	    echo "    binary will require LD_LIBRARY_PATH=./dist/moros-editor at runtime"; \
	fi
	@echo "  [5/5] distributable layout:"
	@find dist/moros-editor -maxdepth 2 -type f -printf "    %p  (%s bytes)\n" 2>/dev/null || \
	    find dist/moros-editor -maxdepth 2 -type f -exec ls -l {} \; | awk '{printf "    %s  (%s bytes)\n", $$9, $$5}'
	@echo ""
	@echo "  Distributable:  dist/moros-editor/"
	@echo "  Run:            ./dist/moros-editor/moros-editor"

# wasm-html-test: run the WASM-runtime safety gate (tests/html_wasm.rs).
#
# Why this target exists:  `make wasm` (wasm-pack pipeline) and
# `loft --html` (Brick Buster pipeline) both write to the same path
# `target/wasm32-unknown-unknown/release/libloft.rlib` but with
# incompatible feature sets.  After `make wasm` the rlib pulls in
# wasm-bindgen and the html_wasm tests fail to instantiate (`Import #1
# module="__wbindgen_placeholder__": module is not an object or
# function`).  This target rebuilds the rlib in the --html shape
# (no `wasm` feature) before invoking the test, so the gate is
# deterministic regardless of what was built last.
#
# RELEASE.md § Safety gate cites this as the WASM-runtime gate.
wasm-html-test:
	@echo "  [1/3] checking wasm32-unknown-unknown target ..."
	@rustup target list --installed 2>/dev/null | grep -q wasm32-unknown-unknown || { \
	    echo "    FAIL: rustup target not installed"; \
	    echo "    install with: rustup target add wasm32-unknown-unknown"; \
	    exit 1; }
	@echo "  [2/3] rebuilding wasm rlib for --html (no wasm feature) ..."
	@cargo build --release -q --target wasm32-unknown-unknown --lib --no-default-features --features random || { \
	    echo "    FAIL: wasm rlib build"; exit 1; }
	@cargo build --release -q --lib --bin loft || { \
	    echo "    FAIL: host binary + libloft.rlib build"; exit 1; }
	@echo "  [3/3] running html_wasm safety gate ..."
	@cargo test --release --test html_wasm

# Browser-side WebGL rendering gate.  Builds doc/brick-buster.html
# (only browser-deployed --html artefact today), loads it in headless
# Chrome + SwiftShader, fails on any JS console.error / exception.
#
# Catches shader-compile regressions that `wasm-html-test` cannot
# (compileShader errors are JS-side; the WASM `loft_start` returns
# cleanly even when every frame fails to draw).  Skips cleanly when
# google-chrome / node / wasm32 toolchain are not installed.
#
# Wired into `cargo test --release` automatically (so `make ship` /
# `make ci` pick it up); this target is for ad-hoc invocation.
test-html-render:
	@# Build the HTML artefact BEFORE the cargo test invocation.
	@# tests/html_render.rs intentionally does NOT auto-build via
	@# `make game` — that would invoke `cargo build` mid-`cargo test`
	@# and race the rustc invocations in tests/native.rs over
	@# target/release/deps/.  Calling `make game` from this target
	@# keeps the build outside the test process.
	@$(MAKE) game >/dev/null
	@cargo test --release --test html_render

clean:
	-rm -rf result.txt tests/dumps/*.txt tests/generated/* pkg target/* perf.data perf.data.old profiler.svg

# Nuke only the wasm32 build trees + the browser bundle.  Run when a
# rustc bump leaves a stale rlib that masks real wasm regressions
# (pre-existing failures silently pass until a full rebuild).  Keep
# this OUT of `make ci` — the fast gate stays fast.  Use before
# `make wasm-html-test` or `make gallery` when you suspect staleness.
clean-wasm:
	-rm -rf target/wasm32-unknown-unknown target/wasm32-wasip2 doc/pkg

# @PLN117 — threaded browser bundle: par() / par_fold over real Web Worker
# threads on loft's own pool (rayon on a SharedArrayBuffer + wasm atomics; the
# same runtime `loft --html` links, see src/wasm_threads.rs).
# Needs the nightly toolchain WITH rust-src (build-std rebuilds std with
# atomics) plus wasm-pack.  --target web is MANDATORY — the worker bootstrap
# imports the generated glue as a module; --target nodejs cannot drive it (and
# node has no Web Worker).  A page must `await init()` then
# `startLoftWorkers(wasm, n, {memory, mainJS})` from loft-thread.js before any
# par; on a host without cross-origin isolation it skips the pool and par()
# falls back to sequential (never breaks — verified in-browser).
# Prove it: python3 tests/wasm/coi-server.py 8799 tests/wasm & then load
# /par-thread-proof.html in a cross-origin-isolated browser.  Design + arcs:
# doc/claude/plans/117-browser-multithreading/.
#
# The link-arg set below is what wasm-bindgen's thread transform requires and
# this toolchain's rustc does NOT auto-emit from +atomics alone: shared +
# imported memory with a maximum, plus lld's synthesized TLS / heap-base
# globals kept as exports.  Drop any one and the bundle silently builds with a
# NON-shared memory — workers then die at runtime with "Memory could not be
# cloned".  (max-memory 1 GiB = 16384 wasm pages.)
WASM_MT_RUSTFLAGS = -C target-feature=+atomics,+bulk-memory,+mutable-globals \
  -C link-arg=--shared-memory -C link-arg=--max-memory=1073741824 \
  -C link-arg=--import-memory -C link-arg=--export=__heap_base \
  -C link-arg=--export=__wasm_init_tls -C link-arg=--export=__tls_size \
  -C link-arg=--export=__tls_align -C link-arg=--export=__tls_base \
  -C link-arg=--export=__stack_pointer

# `-Z build-std` reads the std SOURCE from the sysroot of the rustc cargo spawns
# — NOT the rustc `rustup run nightly` selected for cargo itself.  On a box whose
# PATH carries a toolchain's real bin dir (e.g. `.rustup/toolchains/stable/bin`)
# instead of the `~/.cargo/bin` rustup proxies, that bare `rustc` resolves to
# STABLE, whose rust-src omits `library/Cargo.lock` (build-std is nightly-only),
# and every build-std recipe dies with "Cargo.lock does not exist".  Pinning
# RUSTC to nightly's own rustc closes the leak regardless of PATH shape.
# `$$(...)` runs in the recipe shell, so it costs nothing until a target uses it.
NIGHTLY_RUSTC = "$$(rustup run nightly rustc --print sysroot)/bin/rustc"

# @PLN117 — prove `par` still computes the right answers in a build WITHOUT the
# threading feature (the shape `make wasm` ships to the browser).  CI only
# `cargo check`s that configuration, which is how a par that returned garbage
# there stayed invisible: the family's native entries were feature-gated out, so
# the interpreter called functions that were not registered.  This runs the
# threading scripts on both builds and diffs — a par must mean the same thing
# with and without threads, only slower.
check-no-threading:
	@CARGO_TARGET_DIR=target-nothread cargo build --release -q --no-default-features --features "mmap random"
	@cargo build --release -q
	@fail=0; for f in tests/scripts/22-threading.loft tests/scripts/22b-par-fold.loft \
	                 tests/scripts/22c-par-sources.loft tests/scripts/22d-par-narrow.loft; do \
	  ./target/release/loft --interpret $$f 2>/dev/null > /tmp/loft-thr.out; \
	  ./target-nothread/release/loft --path $$(pwd)/ --interpret $$f 2>/dev/null > /tmp/loft-nothr.out; \
	  if diff -q /tmp/loft-thr.out /tmp/loft-nothr.out >/dev/null; then echo "  ok   $$f"; \
	  else echo "  FAIL $$f — par differs with and without the threading feature"; fail=1; fi; done; \
	rm -f /tmp/loft-thr.out /tmp/loft-nothr.out; \
	if [ $$fail -eq 0 ]; then echo "PASS: par is identical with and without threads"; fi; exit $$fail

# @PLN117 — every in-browser threading gate, in one command.  Each measures a
# claim rather than assuming it: that `par` really dispatches across Web Worker
# threads, that the shared-memory model still holds, that it scales, that the UI
# stays responsive, and that a `loft --html` page does all of that too — each
# against the value the interpreter produces.  Needs a headless chromium; the
# bundle gates additionally need `make wasm-mt` (they SKIP without it).  The
# runner keeps going after a failing gate and prints one table at the end;
# `scripts/par_gates.sh --ci` is the same run with a SKIP promoted to a failure,
# which is what .github/workflows/browser-threads.yml runs nightly.
par-gates:
	@scripts/par_gates.sh

# @PLN117 — type-check loft's OWN browser thread pool (src/wasm_threads.rs).  It
# is browser-only by nature, so the host `cargo clippy --all-features` never sees
# it; this is where it gets compiled.  Same recipe the `--html` threaded build
# uses, minus the link step.
# #619 — BUILD the threaded browser runtime (the `html-mt` shape) so `make
# install` can ship it.  Without this the installed loft has only the plain
# `wasm32-unknown-unknown` shape, so `--html --threads` finds no threaded
# runtime and falls back to a single-threaded page — the half of #619 that the
# silent-fallback diagnostic reports but could not fix.
#
# NIGHTLY-CONDITIONAL by necessity: only `-Z build-std` can produce a std
# compiled with wasm atomics, and that is nightly-only.  A stable-only box skips
# it with a message and installs everything else; `--html --threads` then says
# exactly what is missing (the #619 diagnostic) instead of silently degrading.
wasm-html-mt-lib:
	@if ! rustup run nightly rustc -V >/dev/null 2>&1; then \
	  echo "skip: threaded browser runtime needs the nightly toolchain (rustup toolchain install nightly)"; \
	  echo "      installing without it — 'loft --html --threads' will report the missing runtime."; \
	elif ! rustup run nightly rustc --print sysroot 2>/dev/null | xargs -I{} test -d {}/lib/rustlib/src/rust; then \
	  echo "skip: threaded browser runtime needs rust-src (rustup component add rust-src --toolchain nightly)"; \
	  echo "      installing without it — 'loft --html --threads' will report the missing runtime."; \
	else \
	  echo "building the threaded browser runtime (html-mt) — build-std, one-time"; \
	  RUSTFLAGS='$(WASM_MT_RUSTFLAGS)' RUSTC=$(NIGHTLY_RUSTC) \
	  rustup run nightly cargo build --release --lib --target wasm32-unknown-unknown \
	    --no-default-features --features "random wasm-native-threads" \
	    --target-dir target/loft/html-mt -Zbuild-std=panic_abort,std \
	  || { echo "FAIL: threaded browser runtime build"; exit 1; }; \
	fi

check-wasm-threads:
	RUSTFLAGS='-Ctarget-feature=+atomics,+bulk-memory,+mutable-globals' RUSTC=$(NIGHTLY_RUSTC) \
	rustup run nightly cargo check --lib --target wasm32-unknown-unknown \
	--no-default-features --features "random wasm-native-threads" \
	--target-dir target/loft/html-mt -Zbuild-std=panic_abort,std

wasm-mt:
	RUSTFLAGS='$(WASM_MT_RUSTFLAGS)' RUSTC=$(NIGHTLY_RUSTC) \
	rustup run nightly \
	$$HOME/.cargo/bin/wasm-pack build --target web --out-dir tests/wasm/pkg-mt --release \
	-- --no-default-features --features wasm-threads -Z build-std=panic_abort,std
	@cp doc/loft-thread.js tests/wasm/pkg-mt/
	@echo "Built tests/wasm/pkg-mt/ (--target web, threaded, shared memory)."

fill:
	@cargo build --release -q
	@echo "Regenerating src/fill.rs from default/*.loft ..."
	@cargo test --test issues regen_fill_rs -- --ignored --nocapture > /dev/null 2>&1
	@echo "Done. Review with: git diff src/fill.rs"

# List all currently-open P-issues from PROBLEMS.md.  Source of
# truth is the Quick-Reference table; this extracts rows whose
# severity column contains the word "open" or marks the issue
# `(partial)`.  Mirror of the `🔴 Currently Open (fast index)`
# section at the top of the doc — that section uses
# `(partial)` for half-closed issues like P229 (Linux fixed,
# Windows still flaky), and the table writes that as
# `(a) Closed; (b) Open (Windows)` in the severity cell.  The
# regex matches "open" as a whole word so it catches both
# styles without false-positive on `(closed)`.
# `(observation-only)` rows (transient bugs filed once but
# not reproducible) are intentionally omitted — they're a
# watch list, not actionable items.
# `tests/doc_hygiene.rs::problems_open_index_matches_quickref`
# asserts the fast-index and table stay in sync.
problems:
	@awk -F'|' '\
	  /^## Open Issues — Quick Reference/ {flag=1; next} \
	  /^## / && flag {exit} \
	  flag && /^\| [0-9]+ \|/ { \
	    pid = $$2; gsub(/ /, "", pid); \
	    sev = $$4; gsub(/^ +| +$$/, "", sev); \
	    if (tolower(sev) ~ /(^| )open( |[)]|$$)|[(]partial/) { \
	      desc = $$3; gsub(/^ +/, "", desc); \
	      pos = index(desc, "."); \
	      if (pos > 0 && pos < 200) desc = substr(desc, 1, pos); \
	      else if (length(desc) > 180) desc = substr(desc, 1, 180) "..."; \
	      printf "P%-4s  %-65s  %s\n", pid, substr(sev, 1, 65), desc; \
	    } \
	  }' doc/claude/PROBLEMS.md

test-packages:
	@cargo build --release -q
	@failed=0; total=0; \
	for pkg in lib/*/; do \
		if [ ! -f "$$pkg/loft.toml" ]; then continue; fi; \
		if [ ! -d "$$pkg/tests" ]; then continue; fi; \
		pkg_name=$$(basename "$$pkg"); \
		if [ -f "$$pkg/.allow_warnings" ]; then \
			deny_env=""; \
		else \
			deny_env="LOFT_DENY_WARNINGS=1"; \
		fi; \
		for f in "$$pkg"/tests/*.loft; do \
			[ -f "$$f" ] || continue; \
			total=$$((total + 1)); \
			printf "  %-50s" "$$pkg_name/$$(basename $$f)"; \
			out=$$(cd "$$pkg" && env $$deny_env ../../target/release/loft test "$$(basename $$f .loft)" 2>&1); \
			code=$$?; \
			if [ $$code -ne 0 ] || echo "$$out" | grep -q "^Error:\|panicked"; then \
				echo "FAILED"; \
				echo "$$out" | grep -A2 "^Error:\|panicked\|--deny-warnings" | head -8; \
				failed=$$((failed + 1)); \
			else \
				echo "ok"; \
			fi; \
		done; \
	done; \
	echo "$$total package tests, $$failed failed"; \
	if [ $$failed -gt 0 ]; then exit 1; fi

# Phase 6t Tier 2 — Rust integration tests living inside each library's
# `native/tests/`.  These travel with the library when it extracts to a
# chunk repo (the library-ci.yml.example template runs `cargo test
# --release` for any package that ships `native/tests/*.rs`).  In the
# monorepo, this target runs the equivalent so coverage doesn't lapse
# while the library still lives here.
test-package-native-tests:
	@failed=0; total=0; \
	for pkg in lib/*/native; do \
		[ -d "$$pkg/tests" ] || continue; \
		pkg_name=$$(basename $$(dirname "$$pkg")); \
		printf "  %-50s" "$$pkg_name/native: cargo test"; \
		total=$$((total + 1)); \
		out=$$(cd "$$pkg" && cargo test --release 2>&1); \
		code=$$?; \
		if [ $$code -ne 0 ]; then \
			echo "FAILED"; \
			echo "$$out" | grep -E "FAILED|panicked|^---- " | head -20; \
			failed=$$((failed + 1)); \
		else \
			echo "ok"; \
		fi; \
	done; \
	echo "$$total native test crates, $$failed failed"; \
	if [ $$failed -gt 0 ]; then exit 1; fi

# Headless GL example tests — tiered:
#
#   test-gl-smoke    : 3 representative examples, ~20s. Wired into `make ci-full`.
#                      Catches catastrophic regressions (window creation,
#                      Painter2D draw path, scene-graph render path).
#   test-gl-headless : full set (14 today, 26 once P120 lands), ~90-180s.
#                      Run on demand: `make test-gl-headless`. Catches
#                      finer-grained regressions.
#
# Both run lib/graphics/examples/*.loft under Xvfb with the Mesa software
# rasterizer for ~5 seconds each, looking for panics. They catch the
# "appears fixed but isn't" failure mode where a unit-level regression
# test passes but the real GL example panics in actual usage (see
# PROBLEMS.md #120).
#
# An example "passes" if it exits with code 0 (clean exit), 124 (our
# 5-second timeout fired — expected for examples with `for _ in 0..1000000`
# game loops), or 143 (SIGTERM). Anything else is a failure, and any
# `panicked` line in stderr is also a failure regardless of exit code.

# Smoke set — one custom example designed for fast, broad coverage of
# the most-likely-to-regress paths in a single ~5s run. Adding more
# coverage to the smoke set should be done by editing 00-smoke.loft,
# not by adding more files here.
GL_SMOKE := 00-smoke

# Examples currently broken by P120 (Delete on locked store in copy_record).
# P120 fixed — const-param store lock now released at function exit.
# All 27 GL examples pass headless.  Keep variable for future skip needs.
GL_HEADLESS_SKIP :=

# Internal helper: run one loft example under Xvfb. Used by both targets.
# $1 = path to .loft file. Returns 0 on success, sets failed counter via stderr.
define gl_headless_run_one
	name=$$(basename "$(1)" .loft); \
	printf "  %-30s " "$$name"; \
	out=$$(timeout 5 xvfb-run -a -s "-screen 0 800x600x24" \
		./target/release/loft --interpret \
			--path $$(pwd)/ --lib $$(pwd)/lib/ \
			"$(1)" 2>&1); \
	code=$$?; \
	if echo "$$out" | grep -q "panicked"; then \
		echo "FAILED (panic)"; \
		echo "$$out" | grep -A2 "panicked" | head -5; \
		failed=$$((failed + 1)); \
	elif [ $$code -eq 0 ] || [ $$code -eq 124 ] || [ $$code -eq 143 ]; then \
		echo "ok"; \
	else \
		echo "FAILED (exit $$code)"; \
		echo "$$out" | tail -3; \
		failed=$$((failed + 1)); \
	fi
endef

test-gl-smoke:
	@cargo build --release -q
	@if ! command -v xvfb-run >/dev/null 2>&1; then \
		echo "  test-gl-smoke: SKIPPED (xvfb-run not installed; apt-get install xvfb)"; \
		exit 0; \
	fi
	@failed=0; total=0; \
	for name in $(GL_SMOKE); do \
		f="lib/graphics/examples/$$name.loft"; \
		[ -f "$$f" ] || { echo "MISSING: $$f"; failed=$$((failed + 1)); continue; }; \
		total=$$((total + 1)); \
		$(call gl_headless_run_one,$$f); \
	done; \
	echo "$$total smoke-tested, $$failed failed"; \
	if [ $$failed -gt 0 ]; then exit 1; fi

# test-gl-golden: render the smoke test under Xvfb and compare the
# resulting screenshot pixel-for-pixel against tests/golden/00-smoke.png.
# Mesa swrast is deterministic, so any non-zero difference indicates a
# real rendering regression — colour swap, missing texture, layout drift,
# font path failure, etc. The bug found today (gl_load_font sentinel
# mismatch hiding all text textures) would have been caught here on the
# first run after the bug was introduced.
#
# Tolerance: 1% per-pixel fuzz, 0 absolute differences allowed. Adjust
# the AE threshold if anti-aliasing on different platforms produces a
# small but bounded difference.
#
# To accept a deliberate visual change, run `make update-gl-golden`.
test-gl-golden:
	@cargo build --release -q
	@if ! command -v xvfb-run >/dev/null 2>&1; then \
		echo "  test-gl-golden: SKIPPED (xvfb-run not installed)"; \
		exit 0; \
	fi
	@if ! command -v compare >/dev/null 2>&1; then \
		echo "  test-gl-golden: SKIPPED (ImageMagick compare not installed)"; \
		exit 0; \
	fi
	@if [ ! -f tests/golden/00-smoke.png ]; then \
		echo "  test-gl-golden: FAIL — tests/golden/00-smoke.png missing."; \
		echo "  Run 'make update-gl-golden' to create it."; \
		exit 1; \
	fi
	@mkdir -p /tmp/loft_test_render
	@printf "  %-30s " "00-smoke.png vs golden"
	@xvfb-run -a -s "-screen 0 400x300x24" \
		tests/scripts/snap_smoke.sh /tmp/loft_test_render/00-smoke.png \
		>/tmp/loft_golden.log 2>&1; \
	rc=$$?; \
	if [ $$rc -ne 0 ]; then \
		echo "FAIL (snapshot)"; \
		cat /tmp/loft_golden.log; \
		exit 1; \
	fi; \
	diff_count=$$(compare -metric AE -fuzz 1% \
		tests/golden/00-smoke.png \
		/tmp/loft_test_render/00-smoke.png \
		/tmp/loft_test_render/00-smoke-diff.png 2>&1); \
	if [ "$$diff_count" = "0" ]; then \
		echo "ok (0 px differ)"; \
	else \
		echo "FAIL ($$diff_count px differ)"; \
		echo "  Diff written to /tmp/loft_test_render/00-smoke-diff.png"; \
		echo "  If the change is intentional, run: make update-gl-golden"; \
		exit 1; \
	fi

# Regenerate tests/golden/00-smoke.png from the current build. Use after
# an intentional visual change to the smoke test or to a renderer code
# path that affects it.
update-gl-golden:
	@cargo build --release -q
	@if ! command -v xvfb-run >/dev/null 2>&1; then \
		echo "  update-gl-golden: requires xvfb-run"; exit 1; \
	fi
	@mkdir -p tests/golden
	@xvfb-run -a -s "-screen 0 400x300x24" \
		tests/scripts/snap_smoke.sh tests/golden/00-smoke.png
	@echo "  Updated tests/golden/00-smoke.png"
	@echo "  Inspect with: xdg-open tests/golden/00-smoke.png"

test-gl-headless:
	@cargo build --release -q
	@if ! command -v xvfb-run >/dev/null 2>&1; then \
		echo "  test-gl-headless: SKIPPED (xvfb-run not installed; apt-get install xvfb)"; \
		exit 0; \
	fi
	@failed=0; total=0; skipped=0; \
	skip_pattern="$$(echo "$(GL_HEADLESS_SKIP)" | tr ' ' '|')"; \
	for f in lib/graphics/examples/*.loft; do \
		[ -f "$$f" ] || continue; \
		name=$$(basename "$$f" .loft); \
		if echo "$$name" | grep -qE "^($$skip_pattern)$$"; then \
			printf "  %-30s SKIP (PROBLEMS.md P120)\n" "$$name"; \
			skipped=$$((skipped + 1)); \
			continue; \
		fi; \
		total=$$((total + 1)); \
		$(call gl_headless_run_one,$$f); \
	done; \
	echo "$$total tested, $$skipped skipped, $$failed failed"; \
	if [ $$failed -gt 0 ]; then exit 1; fi

ci-miri:  ## @PLAN53: run the loft interpreter under Miri (hard-UB gate). SLOW (~15 min/test).
	@# Mirror of .github/workflows/miri.yml.  Catches alignment / OOB / UAF /
	@# uninitialised / leak UB the homegrown stack_align_guard can't see.
	@# Needs nightly + miri:  rustup toolchain install nightly --component miri
	@# -Zmiri-disable-stacked-borrows gates the HARD memory UB, not the aliasing
	@# model (loft's store layer aliases distinct records by design).  Runs on the
	@# aligned interpreter (the hard-UB-clean configuration).  Add validated tests
	@# to the curated list below as the lever closes more clusters.
	cargo +nightly miri setup
	LOFT_ALIGN=1 LOFT_SLOT_V2=drive \
		MIRIFLAGS='-Zmiri-disable-isolation -Zmiri-disable-stacked-borrows' \
		cargo +nightly miri test --test issues -- --exact \
		p213_struct_field_basic_int

.PHONY: o-proxy-check
o-proxy-check:  ## @FR-O-Proxy: does every free on the empty-deps proxy consult the override?
	@# An empty dep list does not mean "owner" — it means nothing recorded a dep, which is
	@# also true of a borrow nobody populated (loft#723).  Gated by
	@# `doc_hygiene::o_proxy_frees_consult_the_override`, which runs this same script.
	@python3 scripts/o_proxy_check.py

.PHONY: ir-schema-check ir-schema-regen
falsify:  ## Run a guard against the build it was written to catch: GUARD=<file> REF=<commit>
	@# A guard that passes on the build it was written for proves nothing, and the ways
	@# that happens are not exotic — the wrong ENTRY POINT, a success marker the error
	@# report echoes, a leak gate that is monotone, a cell that never reaches its code
	@# path.  This builds REF, runs GUARD on both trees through the entry point the
	@# corpus runner would pick, and compares four channels apart so the verdict names
	@# WHICH one moved.  Paste its line into the guard; `doc_hygiene::
	@# every_new_guard_records_its_control` requires one on every new file.
	@if [ -z "$(GUARD)" ] || [ -z "$(REF)" ]; then \
		echo "usage: make falsify GUARD=tests/scripts/<file>.loft REF=<commit-before-the-fix>"; \
		exit 2; \
	fi
	@./scripts/falsify.sh "$(GUARD)" "$(REF)"

ir-schema-check:  ## Is src/ir_schema_gen.rs still what tools/ir_schema/ir.loft generates?
	@# The generated file IS the store layout (record sizes, field offsets, the Node
	@# discriminants data_store.rs bakes into DISC_*).  It drifted once already — Key
	@# gained a `start` field, the generated file was updated, ir.loft was not — and a
	@# wrong layout is a wrong byte offset, not a build error.  Gated by
	@# `doc_hygiene::ir_schema_gen_matches_its_loft_source`, which runs this same script.
	@scripts/ir_schema_check.sh

ir-schema-regen:  ## Rewrite src/ir_schema_gen.rs from tools/ir_schema/ir.loft
	@scripts/ir_schema_check.sh --fix
	@echo "now run: cargo test --lib baked_layout_mirrors_loft_schema"

.PHONY: check-rlib
check-rlib:  ## One-second pre-flight: is target/release/libloft.rlib present and current?
	@# The native path (`build_shared_cdylib`) links `libloft.rlib`, and NOTHING in an
	@# ordinary edit loop rebuilds it: `cargo build --bin loft` refreshes the binary and
	@# leaves the library rlib alone, `cargo build --release --lib` is the only thing that
	@# touches it.  So a session that iterates on the compiler with `--bin loft` drifts,
	@# and the drift is invisible until a gate runs.
	@#
	@# It costs a full cycle when it lands, which is why this check exists at all.  It does
	@# not fail like a compile error: it surfaces ~9 minutes in as a handful of native tests
	@# failing for what look like unrelated reasons — `libloft.rlib not found for this
	@# build`, a cdylib mtime that did not advance, a `native_scripts` sweep going red — each
	@# naming a file that is present when you go and look.
	@#
	@# This target REPORTS; it is not wired into `ci`.  `make ci` builds all three itself
	@# (beside the wasm builds it already ran), because a gate that refuses on a condition
	@# it could satisfy is friction on every run after every edit, and friction is what
	@# gets a check switched off.  Reach for this before a BARE `cargo test --release`,
	@# which builds no rlib of its own and is where the drift actually bites.
	@# Keyed on the SOURCES, not on `deps/`.  A from-source tree usually has no
	@# `deps/libloft-<hash>.rlib` at all — only the bare uplifted one — so comparing the
	@# two finds nothing and reports "current" on a tree that is anything but.  What is
	@# always true is that a `.rs` newer than an rlib means that rlib predates the code
	@# the rest of the gate is about to test.
	@#
	@# THREE rlibs, because there are three link targets and each has its own suite:
	@# the native one (`--native`, the cdylib tests), the browser one (`--html`), and
	@# wasip2 (the wasm library suite).  They drift independently — refreshing the
	@# native rlib does nothing for `--html`, which is how a green re-run still went red
	@# on `moros_editor_html_smoke` and `wasm_library_suite` alone.  A target directory
	@# that does not exist is SKIPPED, not failed: not every checkout installs the wasm
	@# targets, and a check that fails on their absence would just be turned off.
	@fail=0; \
	for spec in \
	    "target/release/libloft.rlib|cargo build --release --lib|native (--native, cdylib tests)" \
	    "target/wasm32-unknown-unknown/release/libloft.rlib|cargo build --release --target wasm32-unknown-unknown --lib --no-default-features --features random|browser (--html)" \
	    "target/wasm32-wasip2/release/libloft.rlib|cargo build --release --target wasm32-wasip2 --lib --no-default-features --features random|wasip2 (wasm library suite)"; do \
	    rlib=$${spec%%|*}; rest=$${spec#*|}; cure=$${rest%%|*}; what=$${rest#*|}; \
	    dir=$$(dirname "$$(dirname "$$rlib")"); \
	    if [ ! -d "$$dir" ]; then continue; fi; \
	    if [ ! -f "$$rlib" ]; then \
	        echo "make: $$rlib is MISSING — the $$what tests cannot link."; \
	        echo "  Run: $$cure"; fail=1; continue; \
	    fi; \
	    newer=$$(find src Cargo.toml -newer "$$rlib" -print -quit 2>/dev/null); \
	    if [ -n "$$newer" ]; then \
	        echo "make: $$rlib is STALE — $$newer is newer than it."; \
	        echo "  The $$what tests link it, so they would test an outdated library"; \
	        echo "  and fail ~9 minutes in for reasons that look unrelated to the edit."; \
	        echo "  Run: $$cure"; fail=1; \
	    fi; \
	done; \
	if [ $$fail -ne 0 ]; then exit 1; fi; \
	echo "libloft.rlib: present and current (native + wasm)"

.PHONY: ci-guard
ci-guard:
	@# REFUSE to start while another gate is running in this tree, BEFORE the
	@# truncation below — because two concurrent runs do not merely interleave,
	@# they FAKE FAILURES in each other and both reports become fiction:
	@#
	@#   * they share `target/`, so the second run's `cargo build` replaces the
	@#     debug rlib the first is linking tests against.  The symptom is
	@#     `error: extern location for loft does not exist:
	@#     target/debug/deps/libloft.rlib` across ~20 native tests — a red gate
	@#     naming a file that is present when you look;
	@#   * they share `result.txt`, so the second truncates the first's report
	@#     mid-write and the surviving text belongs to neither run.
	@#
	@# Both were read as real failures before this guard existed, and the cost
	@# is a full cycle each time — the gate is ~10 minutes.
	@#
	@# Keyed on LIVENESS, not on the file existing: the pid is make's own
	@# ($$PPID from a recipe shell), so a run killed with ^C or an OOM leaves a
	@# stale file that the next `kill -0` steps straight over.  A lock that
	@# outlives its holder is worse than no lock — it fails runs that should
	@# pass, and gets deleted by hand until nobody trusts it.
	@# ⚠ That is what the liveness test is FOR, and for a while it did not do it: the
	@# `kill -0` gated only the ancestor walk below, so a DEAD claim fell straight through
	@# to the refusal with the pid still set.  A killed run then blocked every later gate
	@# in the tree until someone removed the file by hand — the exact failure this comment
	@# promises it prevents.  Clearing the claim is what makes the promise true.
	@# ⚠ A claim held by one of OUR OWN ANCESTORS is not a competing run — it is the
	@# wrapper that launched us.  `scripts/box-claim.sh make ci` writes the claim, then
	@# `make ci` refused itself and exited 1; the failure was then invisible, because the
	@# stale `result.txt` from an earlier run still said ALL GATES PASSED and that is what
	@# a summary grep reads.  Measured 2026-08-21 — it produced a false green on a real
	@# cherry-pick.  So walk the pid chain: a claim from an ancestor is ours.
	@claim=$$(cat .ci-running 2>/dev/null); \
	if [ -n "$$claim" ] && ! kill -0 "$$claim" 2>/dev/null; then claim=""; fi; \
	if [ -n "$$claim" ]; then \
	    p=$$PPID; mine=0; \
	    while [ "$$p" -gt 1 ]; do \
	        [ "$$p" = "$$claim" ] && { mine=1; break; }; \
	        p=$$(ps -o ppid= -p $$p 2>/dev/null | tr -d ' '); \
	        [ -n "$$p" ] || p=1; \
	    done; \
	    [ "$$mine" = "1" ] && claim=""; \
	fi; \
	if [ -n "$$claim" ]; then \
	    echo "make ci: REFUSED — a gate is already running in this tree (make pid $$claim)."; \
	    echo "  Two runs share target/ and result.txt; the second deletes the rlib the first"; \
	    echo "  links against and truncates its report, so BOTH results would be fiction."; \
	    echo "  Wait for it to finish, or stop it first."; \
	    exit 1; \
	fi
	@# A SIBLING CHECKOUT is a different question, and gets a warning rather than a
	@# refusal.  Two loft checkouts on one box do NOT share target/ or result.txt, so
	@# neither result is fiction — but they share the 24 threads, and that is enough to
	@# matter twice: a timing measurement in either tree becomes worthless, and the
	@# 300s slow-timeout starts firing on tests that pass standalone in seconds
	@# (`instancing_bridge_draws_every_instance` is the known one, 300s under load vs
	@# 3.6s alone).  Measured 2026-08-21: two agents collided twice in one afternoon,
	@# reaching load 66 on 24 threads, and both had guessed the box was free from
	@# `pgrep` — silence read as evidence.
	@#
	@# ⚠ This can only see runs that CLAIM.  `make ci` writes `.ci-running`; a bare
	@# `cargo nextest run` writes nothing and stays invisible, which is exactly how the
	@# second collision happened.  Claim the tree by hand for a long ad-hoc run:
	@#     echo $$$$ > .ci-running        # and rm it when done
	@for d in ../*/; do \
	    [ "$$(cd "$$d" 2>/dev/null && pwd -P)" = "$$(pwd -P)" ] && continue; \
	    [ -f "$$d/.ci-running" ] || continue; \
	    kill -0 "$$(cat "$$d/.ci-running" 2>/dev/null)" 2>/dev/null || continue; \
	    echo "make ci: WARNING — a gate is also running in $$d (pid $$(cat "$$d/.ci-running"))."; \
	    echo "  Not refused: separate target/ and result.txt, so neither result is fiction."; \
	    echo "  But you are sharing $(CI_NPROC) threads — expect a slower run, and treat any"; \
	    echo "  300s slow-timeout as 'the machine was busy' until it reproduces alone."; \
	done

ci: ci-guard
	@echo $$PPID > .ci-running
	@# Fresh header FIRST so result.txt can never be mistaken for a stale
	@# run.  rebuild-native-cdylibs is invoked INSIDE the chain below (not as
	@# an order-only prerequisite) so its output — and any failure — lands in
	@# result.txt; as a prerequisite it ran before this truncation and a
	@# prereq failure (e.g. a missing wasm target after a toolchain switch)
	@# left the OLD result.txt in place, masking the real cause.
	@printf '== make ci | %s | %s ==\n' "$$(rustc --version 2>/dev/null)" "$$(date -u +%FT%TZ)" > result.txt
	-rm -rf tests/generated
	-rm -f /tmp/loft_native_*
	# Some tests (e.g. fill_rs_up_to_date, n2..n10) write into tests/generated
	# directly via generate_code_to without first calling create_dir_all.
	# Recreate the directory so these tests don't fail with NotFound when
	# parallel test scheduling lets them race the helpers that *do* create it.
	mkdir -p tests/generated
	# Mirror of .github/workflows/ci.yml in invocation order so a green
	# local `make ci` predicts a green PR:
	#   1. Format     job → cargo fmt -- --check
	#   2. Clippy     job → cargo clippy -- -D warnings    (no --release,
	#                       no --tests, no --no-default-features — that
	#                       matches the remote runner exactly)
	#                  +  → cargo clippy --all-targets --all-features
	#                       -- -D warnings (added 2026-05-29 to catch
	#                       wasm-feature-only defects that the default
	#                       gate misses: the bin/lib module-cfg mismatch
	#                       on wasm_gl, latent silent-no-op in
	#                       parallel.rs, pedantic-lint regressions in
	#                       src/wasm.rs after rustc updates)
	#   3. Doc hygiene job → scripts/check_doc_drift.sh (blocking since
	#                       2026-05-18 — promoted from non-blocking after
	#                       repeated PR-212 cycles where ignored drift
	#                       surfaced as downstream test failures)
	#   3b. cache warm     → loft#1238: build the native artifacts this run is
	#                       about to need, ONCE, before the parallel section.
	#                       `native_artifact_cache_key` folds in a content hash
	#                       of the loft build, so the rebuild above invalidates
	#                       every cached cdylib and loft's own wasm runtime
	#                       rlib — and the FIRST test to want each pays the
	#                       full rebuild while the rest queue on the global
	#                       build lock.  Measured: 25.6s for the wasm rlib,
	#                       63s for the `random` cdylib on a loaded box,
	#                       against a 60s per-test budget that blew twice.
	#                       Run with the RELEASE binary on purpose: the cdylib
	#                       key is profile-independent, but the wasm-rlib
	#                       fingerprint is a content hash of the binary, and
	#                       `html_asyncify` drives `target/release/loft`.
	#                       0.3-0.5s once warm; never fails the gate.
	#   4. Test       job → cargo build --all-targets,
	#                       cargo build --release --target wasm32-wasip2/
	#                         wasm32-unknown-unknown --lib (added
	#                         2026-06-12: the wasm targets are a separate
	#                         COMPILE GATE — a cfg'd fn whose call sites
	#                         aren't cfg'd breaks them while every native
	#                         gate stays green, hidden behind stale rlibs
	#                         until something rebuilds one; bit twice in
	#                         the @PLN18 arc.  ~seconds when warm),
	#                       cargo build --no-default-features
	#                         (to an isolated --target-dir: this strips
	#                          `native-extensions` from libloft.rlib, and
	#                          sharing target/debug/deps/ let it stomp the
	#                          rlib the native tests link → intermittent
	#                          `E0433: cannot find native_call`),
	#                       cargo nextest run --profile ci
	#
	# Drift from what GH runs is the most common cause of "passed local,
	# failed remote" — keep this list short and IDENTICAL to ci.yml.
	# The `Browser build + probe` (gallery) job is intentionally not
	# mirrored here: it requires wasm-pack + node + a clean network and
	# is heavy enough that local devs run `make gallery` separately when
	# touching the wasm bundle.  Other dev-only suites (test-packages,
	# test-gl-smoke, test-gl-golden) live in `make ci-full`.
	mkdir -p $(TEST_SCRATCH) && \
	{ scripts/sweep_scratch.sh $(TEST_SCRATCH) >> result.txt 2>&1 || true; } && \
	export $(TEST_ENV) && \
	{ gates=$(CI_LIVE_GATES); jobs=$$(( $(CI_NPROC) / $${gates:-1} )); if [ $$jobs -lt 2 ]; then jobs=2; fi; \
	  export CARGO_BUILD_JOBS=$$jobs NEXTEST_TEST_THREADS=$$jobs; } && \
	{ [ "$${gates:-1}" -gt 1 ] && echo "make ci: THROTTLED to $$jobs of $(CI_NPROC) threads — $$gates gates live on this box" || echo "make ci: $$jobs of $(CI_NPROC) threads (sole gate)"; } | tee -a result.txt && \
	$(MAKE) rebuild-native-cdylibs >> result.txt 2>&1 && \
	cargo fmt -- --check >> result.txt 2>&1 && \
	cargo clippy -- -D warnings >> result.txt 2>&1 && \
	cargo clippy --all-targets --all-features -- -D warnings >> result.txt 2>&1 && \
	scripts/check_doc_drift.sh >> result.txt 2>&1 && \
	$(MAKE) --no-print-directory label-guard-test >> result.txt 2>&1 && \
	python3 scripts/contract_labels.py --self-test >> result.txt 2>&1 && \
	python3 scripts/revalidate_matrix.py --self-test >> result.txt 2>&1 && \
	cargo build --all-targets >> result.txt 2>&1 && \
	cargo build --release --lib >> result.txt 2>&1 && \
	cargo build --no-default-features --target-dir target/nodefault >> result.txt 2>&1 && \
	cargo build --release --target wasm32-wasip2 --lib --no-default-features --features random >> result.txt 2>&1 && \
	cargo build --release --target wasm32-unknown-unknown --lib --no-default-features --features random >> result.txt 2>&1 && \
	python3 scripts/gen_target_surface.py --check >> result.txt 2>&1 && \
	(cargo nextest --version >/dev/null 2>&1 || cargo install cargo-nextest --locked) >> result.txt 2>&1 && \
	./target/release/loft cache warm --from tests >> result.txt 2>&1 && \
	{ gates=$(CI_LIVE_GATES); jobs=$$(( $(CI_NPROC) / $${gates:-1} )); if [ $$jobs -lt 2 ]; then jobs=2; fi; export NEXTEST_TEST_THREADS=$$jobs; } && \
	echo "make ci: tests on $$jobs thread(s), $$gates gate(s) live" >> result.txt && \
	cargo nextest run --profile ci >> result.txt 2>&1 && \
	echo 'CI-RESULT: ALL GATES PASSED' >> result.txt || \
	{ echo 'CI-RESULT: FAILED — see the last failing command above in result.txt' >> result.txt; rm -f .ci-running; exit 1; }
	@# Tidiness only — the guard above tests whether the recorded pid is ALIVE,
	@# so a run that dies without reaching either branch blocks nothing.
	@rm -f .ci-running

# Reliable gate control — `ci` itself is unchanged; these wrap it so completion can be
# DETECTED rather than guessed.  `grep CI-RESULT result.txt` is not a completion signal:
# a run that dies writes no verdict, and a previous run's verdict is still in the file
# while the next one compiles.  `ci-status` re-reads the recorded pid, so a vanished run
# reports DIED instead of RUNNING forever.  See scripts/ci-run.sh.
ci-bg:  ## Start the full gate detached (refuses if one is already running)
	@./scripts/ci-run.sh start

ci-status:  ## PASSED / FAILED / RUNNING <n>s / DIED / NOT-STARTED
	@./scripts/ci-run.sh status

ci-wait:  ## Block until the running gate reaches a verdict, then print it
	@./scripts/ci-run.sh wait

# The agent-facing form: launch with the Bash tool's `run_in_background`, which re-invokes
# the agent when the command EXITS.  So this is a one-shot notification that costs nothing
# while it waits and leaves the session free — the reason not to use a foreground `ci-wait`,
# which blocks the agent and the user with it.  Ends on DIED as well as PASSED/FAILED.
ci-notify:  ## Exit (once) when the gate reaches any verdict — for background launch
	@./scripts/ci-run.sh notify

# Local-only superset of `ci`: same gates plus the development suites
# that are NOT in .github/workflows/ci.yml — package smoke tests and
# the GL/golden visual regression.  Useful before merging an
# infrastructure change that could affect them; not required for the
# remote PR gate.
ci-full: ci
	$(MAKE) test-packages >> result.txt 2>&1 && \
	$(MAKE) test-gl-smoke >> result.txt 2>&1 && \
	$(MAKE) test-gl-golden >> result.txt 2>&1

# Doc hygiene checks — non-blocking.
#
# Runs scripts/check_doc_drift.sh:
#   - paths   : every markdown link to a plan resolves
#   - stale   : retired-feature claims (Type::Long, text_code, .loftc, …)
#   - roadmap : every active plan on ROADMAP at canonical path; no
#               finished/deferred crept in as action items
#   - refs    : no normal docs deep-link into closed/deferred plans
#               (closure-rule enforcement)
#   - time    : (warn) calendar-time projections that should be effort letters
#   - libs    : (warn) lib/<name>/ has loft.toml + README.md
#
# Exit code:
#   0 = clean OR only warnings (time/libs)
#   1 = real drift (paths/stale/roadmap/refs)
#
# Intentionally NOT a `ci` dependency.  Doc drift is a soft failure:
# bad links + closure-rule violations don't break the binary.  Run
# locally before opening a PR; the user makes the call.  If you want
# to gate a PR on doc cleanliness, run `make doc-check` and check
# the exit code yourself, OR wire it into a separate non-blocking
# GitHub Actions job.
doc-check:
	@scripts/check_doc_drift.sh

doc-check-quiet:
	@scripts/check_doc_drift.sh -q

# Regenerate the agent-facing installable-library catalogue from the LIVE
# registry: doc/claude/LIBRARIES.md plus the committed snapshot CI generates
# from (doc/claude/registry-index-snapshot.json).  Run after any registry
# change so an agent in this repo always sees what's already published — and
# never reimplements a registered library.  CI runs the same generator in
# --check mode against the snapshot and fails on drift, so commit both files
# this writes.
libcatalogue:
	@python3 scripts/refresh-unreleased.py   # @PLN112 — origin/main `unreleased` tier (sha-cached)
	@python3 scripts/refresh-loft-release.py # @PLN112 — loft itself (version + binary sha, tag-cached)
	@python3 scripts/refresh-applications.py # @PLN112 — apps: first-party self-described (repo topic+toml) + community issues
	@python3 scripts/gen-library-catalogue.py --refresh-snapshot

# QUALITY Tier 4 #12 — the single pre-push gate.
#
# `ci` optimises for the full automated pipeline: it logs to result.txt,
# runs the GL + packages suites, and only prints on failure.  That's
# great for a remote runner but awkward at the terminal — if you forget
# one of fmt / clippy-default / clippy-ndf / release tests before `git
# push`, the remote CI surfaces it minutes later and the branch sits in
# a partial state.
#
# `ship` is the fast local equivalent: the same four invariants that
# block any push, streamed to the terminal, chained with `&&` so the
# first failure stops the chain and exits non-zero.  The intended
# workflow is `make ship && git push` — if `ship` fails the push
# doesn't happen.
ship:
	cargo fmt --all -- --check && \
	cargo clippy --all-targets --all-features -- -D warnings && \
	cargo clippy --no-default-features --all-targets -- -D warnings && \
	scripts/check_doc_drift.sh && \
	cargo test --release

# The cheap CI gates ONLY (~2-3 min warm): exactly the commands the Format /
# Clippy / Doc-hygiene jobs run, nothing else.  A local pass here removes the
# failure modes that cost a full remote round each ("3-4 runs to get a PR
# green"): fmt drift, a clippy lint the narrower local variants miss
# (`--all-features` is what the CI job adds), a doc-drift ref, and the
# repo-shape guard tests (ship recipe, doc links — seconds to run, and a
# Makefile/doc edit that trips one costs a whole matrix round remotely).
# Use before every push; `make ship` remains the full pre-push gate with tests.
gate:
	cargo fmt --all -- --check && \
	cargo clippy --all-targets --all-features -- -D warnings && \
	scripts/check_doc_drift.sh -q && \
	cargo nextest run --release --test doc_hygiene

run-tests: rebuild-native-cdylibs
	cargo test --release > result.txt 2>&1

clippy:
	cargo fmt -- --check > result.txt 2>&1
	cargo clippy --tests -- -D warnings >> result.txt 2>&1
	cargo check --no-default-features >> result.txt 2>&1

memory:
	cargo test --test vectors -- --nocapture 2>&1 | valgrind --tool=memcheck

last:
	cargo test --package dryopea --test wrap last --release -- --nocapture

meld:
	rustfmt tests/generated/text.rs --edition 2024
	cmp -s tests/generated/text.rs src/text.rs; if [ $$? -eq 1 ]; then meld tests/generated/text.rs src/text.rs; fi
	rustfmt tests/generated/fill.rs --edition 2024
	cmp -s tests/generated/fill.rs src/fill.rs; if [ $$? -eq 1 ]; then meld tests/generated/fill.rs src/fill.rs; fi

generate:
	# cd tests/generated && rustfmt *.rs --edition 2024
	# TODO: target path 'generated/tests/' not present; update when generated workspace is added
	meld tests/generated/ generated/tests/

gtest:
	# TODO: 'generated/' workspace not present; update path when added
	cd generated && cargo clippy --tests -- -W clippy::all -W clippy::cognitive_complexity > result.txt 2>&1
	cd generated && rustfmt tests/*.rs --edition 2024 >> result.txt 2>&1
	cd generated && cargo test -- --nocapture --test-threads=1 >>result.txt 2>&1

bench:
	cargo build --release -q
	bash bench/run_bench.sh --warmup

.PHONY: doc doc-packages
# The whole doc site, the way the release builds it (@PLN149).
#
# `gendoc` alone is not that: tiers 1 and 3 — the library guide and the source browser —
# render the EXTRACTED package under `~/.loft/registry/`, which the registry index does
# not carry.  On a box whose cache is empty every page still generates, and 42 of them
# say "not on this build box" instead of showing the source.  Filling the cache first is
# what makes a local build and the published site the same artefact.
doc: doc-packages  ## Build doc/*.html the way the release does (cache first, then gendoc)
	cargo run --bin gendoc

doc-packages:  ## Fetch every published package the doc build renders (no-op once cached)
	@cargo build --release --bin loft
	@scripts/fetch-doc-packages.sh target/release/loft

# Typst stamps a creation date into the PDF, so an unchanged document still produces a
# different file on every build and a COMMITTED pdf churns in every diff.  Pinning
# SOURCE_DATE_EPOCH to the source's last commit makes the output depend on the content
# alone: rebuild without editing and git reports nothing.
pdf: doc
	SOURCE_DATE_EPOCH=$$(git log -1 --format=%ct -- doc/loft-reference.typ) \
	  typst compile doc/loft-reference.typ doc/loft-reference.pdf

# Print one design document as its own PDF.  The Markdown stays the single source;
# `scripts/md2typ.py` renders it, so the two cannot drift.
#   make pdf-doc DOC=doc/claude/WEB_STACK.md OUT=doc/web-stack
DOC ?= doc/claude/WEB_STACK.md
OUT ?= doc/web-stack
pdf-doc:
	python3 scripts/md2typ.py $(DOC) $(OUT).typ
	SOURCE_DATE_EPOCH=$$(git log -1 --format=%ct -- $(DOC)) \
	  typst compile $(OUT).typ $(OUT).pdf

test-native:
	@cargo build --release -q
	@failed=0; \
	for f in tests/docs/*.loft; do \
		printf "  %-45s" "$$f"; \
		out=$$(./target/release/loft --native "$$f" 2>&1); \
		code=$$?; \
		if [ $$code -ne 0 ] || echo "$$out" | grep -q "^Error:\|panicked"; then \
			echo "FAILED"; \
			echo "$$out" | grep -A2 "^Error:\|panicked" | head -5; \
			failed=$$((failed + 1)); \
		else \
			echo "ok"; \
		fi; \
	done; \
	if [ $$failed -gt 0 ]; then \
		echo "$$failed file(s) failed"; \
		exit 1; \
	else \
		echo "All native tests passed."; \
	fi

wasm-assets:
	node tests/wasm/gen-assets.mjs

test-wasm:
	@cargo build --release -q
	@WASMTIME=$$(which wasmtime 2>/dev/null); \
	if [ -z "$$WASMTIME" ] && [ -x "$$HOME/.cargo/bin/wasmtime" ]; then WASMTIME="$$HOME/.cargo/bin/wasmtime"; fi; \
	if [ -z "$$WASMTIME" ] && [ -x "$$HOME/.wasmtime/bin/wasmtime" ]; then WASMTIME="$$HOME/.wasmtime/bin/wasmtime"; fi; \
	if [ -n "$$WASMTIME" ]; then echo "Running wasm tests with wasmtime"; else echo "wasmtime not found — compile-only (install via: cargo install wasmtime-cli)"; fi; \
	failed=0; \
	for f in tests/docs/*.loft tests/scripts/*.loft; do \
		printf "  %-45s" "$$f"; \
		wasm=$$(mktemp /tmp/loft_wasm_XXXXXX.wasm); \
		out=$$(./target/release/loft --native-wasm "$$wasm" "$$f" 2>&1); \
		code=$$?; \
		if [ $$code -ne 0 ]; then \
			rm -f "$$wasm"; \
			echo "FAILED (compile)"; \
			echo "$$out" | head -5; \
			failed=$$((failed + 1)); \
		elif [ -n "$$WASMTIME" ]; then \
			run_out=$$($$WASMTIME --dir . "$$wasm" 2>&1); \
			run_code=$$?; \
			rm -f "$$wasm"; \
			if [ $$run_code -ne 0 ] || echo "$$run_out" | grep -q "^Error:\|panicked"; then \
				echo "FAILED (run)"; \
				echo "$$run_out" | grep -A2 "^Error:\|panicked" | head -5; \
				failed=$$((failed + 1)); \
			else \
				echo "ok"; \
			fi; \
		else \
			rm -f "$$wasm"; \
			echo "ok (compiled)"; \
		fi; \
	done; \
	if [ $$failed -gt 0 ]; then \
		echo "$$failed file(s) failed"; \
		exit 1; \
	else \
		echo "All wasm tests passed."; \
	fi

loft-test:
	@cargo build --bin loft --release -q
	@failed=0; \
	for f in tests/docs/*.loft; do \
		printf "  %-45s" "$$f"; \
		out=$$(./target/release/loft "$$f" 2>&1); \
		code=$$?; \
		if [ $$code -ne 0 ] || echo "$$out" | grep -q "^Error:\|panicked"; then \
			echo "FAILED"; \
			echo "$$out" | grep -A2 "^Error:\|panicked" | head -5; \
			failed=$$((failed + 1)); \
		else \
			echo "ok"; \
		fi; \
	done; \
	if [ $$failed -gt 0 ]; then \
		echo "$$failed file(s) failed"; \
		exit 1; \
	else \
		echo "All loft tests passed."; \
	fi

# T1.1 — link rot in user-facing Markdown (root + doc/, excluding the
# agent-facing doc/claude/ which has its own drift checker).
#   make linkcheck            relative links only — offline, fast, deterministic
#   make linkcheck-external   also HEAD every http(s) link
# Deliberately NOT part of `make ci`: an external-link check makes the build
# depend on other people's uptime, buying flakiness for a class of rot that
# moves slowly.  Nightly is its home; there a red run is information, not a
# blocked merge.
# The label guard's body parser, held to real issue-body shapes.
#
# `.github/workflows/label-guard.yml` turns what a filer wrote into `sev:` /
# `wa:` / `area:` labels — including for a reporter who CANNOT set labels, since
# GitHub restricts that to triage permission.  Every failure mode of that parsing
# is silent (no label applied, which is what an unanswered form looks like), and
# the workflow only runs on an issue event, so nothing else would catch a
# regression until someone filed a bug and got no labels.
#
# Skips where node is absent, like the bundle integrity check above; CI's
# `Doc hygiene` job always runs it.
.PHONY: label-guard-test
label-guard-test:
	@if command -v node >/dev/null 2>&1; then \
	    node tools/label_guard_selftest.mjs || exit 1; \
	else \
	    echo "  WARN: node not found — skipping label-guard selftest"; \
	fi

# The `Contract:` trailer parser, same argument one axis over: the push workflow
# applies the `contract:` label off it, so a regex that stopped matching would
# apply no label — which is exactly what a fix nobody judged looks like, and the
# monthly ratio would report convergence it never measured.
.PHONY: contract-labels-test
contract-labels-test:  ## the `Contract:` trailer parse behind the push workflow's contract: label
	@python3 scripts/contract_labels.py --self-test

# The revalidate-libs matrix, same argument one gate over: the policy decides which
# published packages the freeze gate looks at, and BOTH readers — the workflow and
# the local script — take it from one file now.  A matrix that quietly returns every
# package looks identical to a correct one on a registry with nothing to exclude,
# which is the state the index is usually in; the self-test gives each rule an input
# it has to act on.  Its zero being wrong is what made the local gate read
# `1 COMPILE-BREAK` on an unchanged tree (loft#1315).
.PHONY: revalidate-matrix-test
revalidate-matrix-test:  ## the revalidate-libs matrix policy shared by the workflow and the local gate
	@python3 scripts/revalidate_matrix.py --self-test

.PHONY: linkcheck linkcheck-external
linkcheck:
	scripts/linkcheck.sh

linkcheck-external:
	scripts/linkcheck.sh --external
