#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# The PLATFORM C toolchain a loft build compiles C with, as one line — in ONE place
# because two scripts must ask it identically: `make-release.sh` records it in the
# bundle's BUILD-INFO, and `repro-verify.sh` compares its own against that record.
#
# ## Why a rebuild depends on it
#
# loft is not only Rust.  `ring` (reached through rustls → ureq) compiles C and
# assembly through the `cc` crate, with whatever C compiler the build host provides:
# `cl.exe` from the Visual Studio toolset on Windows, Apple clang on macOS,
# `musl-gcc` for the musl target.  rustc's version, `Cargo.lock` and the remapped
# paths pin the Rust side; nothing pinned this one.
#
# Measured 2026-09-15 on one `windows-2025-vs2026` runner, one source, one rustc
# (1.98.1), one build root: the default toolset (14.51) linked `.text 0xbd1586`,
# `-vcvars_ver=14.44` linked `.text 0xbd1656` with a different `.rdata` and `.pdata`,
# and the published v2026.9.0 binary — built on an older image — has `.text 0xbd1706`.
# A hosted runner's toolset changes with its image, so the weekly verification
# compared a rebuild against a binary made by a compiler that no longer exists there,
# and reported the difference as the SOURCE's.
#
# ## Usage
#   . scripts/repro-toolchain.sh
#   repro_c_toolchain <target-triple>      # prints e.g. "msvc 14.51.36231 winsdk 10.0.26100.0"

repro_c_toolchain() {
  case "$1" in
    *-pc-windows-msvc)
      local tools="${VCToolsVersion:-}" sdk="${WindowsSDKVersion:-}" vswhere inst default_txt
      vswhere="/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
      # Outside a developer prompt, `cc` takes the newest Visual Studio instance's DEFAULT
      # toolset — the version named in its Microsoft.VCToolsVersion.default.txt.
      if [ -z "$tools" ] && [ -x "$vswhere" ]; then
        inst=$("$vswhere" -latest -products '*' -property installationPath 2>/dev/null | tr -d '\r')
        if [ -n "$inst" ]; then
          default_txt="$(cygpath -u "$inst" 2>/dev/null || echo "$inst")/VC/Auxiliary/Build/Microsoft.VCToolsVersion.default.txt"
          [ -f "$default_txt" ] && tools=$(tr -d '\r\n ' < "$default_txt")
        fi
      fi
      sdk="${sdk%\\}"
      [ -n "$sdk" ] || sdk=$(ls "/c/Program Files (x86)/Windows Kits/10/Lib/" 2>/dev/null | sort -V | tail -1)
      echo "msvc ${tools:-unknown} winsdk ${sdk:-unknown}"
      ;;
    *-apple-darwin)
      local clang sdkv
      clang=$(cc --version 2>/dev/null | head -1 | sed 's/[[:space:]]*$//')
      sdkv=$(xcrun --show-sdk-version 2>/dev/null)
      echo "${clang:-cc unknown} / macOS SDK ${sdkv:-unknown}"
      ;;
    *-linux-musl)
      local c
      for c in x86_64-linux-musl-gcc musl-gcc; do
        if command -v "$c" >/dev/null 2>&1; then
          echo "$c $("$c" --version 2>/dev/null | head -1 | sed 's/[[:space:]]*$//')"
          return
        fi
      done
      echo "musl-gcc unknown"
      ;;
    *)
      local generic
      generic=$(cc --version 2>/dev/null | head -1 | sed 's/[[:space:]]*$//')
      echo "${generic:-cc unknown}"
      ;;
  esac
}
