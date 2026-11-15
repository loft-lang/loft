#!/bin/sh
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Builds the loft bytecode runner from this seed with a C compiler alone: no cargo, no rustc, no
# loft (@PLN91, doc/claude/plans/91-self-host/rust-free-build.md, level 1).  The seed is the
# runner's C unit (the C runtime inlined), the crt shim it calls and the compiled
# standard-library bodies that shim includes; `make seed` in the loft repository writes it.
#
#     sh build.sh                 # ./run_image_run
#     CC=clang CFLAGS=-O2 OUT=/usr/local/bin/loft-run sh build.sh
#
# The runner runs a program's run image: `run_image_run prog.run`.
set -eu
if [ "${1:-}" = "--help" ] || [ "${1:-}" = "-h" ]; then
  echo "usage: [CC=cc] [CFLAGS=-O3] [LTO=-flto] [OUT=run_image_run] sh build.sh"
  echo "builds the loft bytecode runner from this seed with a C compiler alone"
  exit 0
fi
cd "$(dirname "$0")"
CC=${CC:-cc}
CFLAGS=${CFLAGS:--O3}
# The unit and the shim are compiled together under link-time optimisation, as `loft --ship`
# builds them: the shim's slot readers inline into the dispatch loop (without it the runner
# measured 1.7x the instructions).  `LTO=` builds a compiler without it.
# GCC runs the link-time step in parallel with `-flto=auto`; other compilers take `-flto`.
if [ -z "${LTO+set}" ]; then
  if "$CC" -v 2>&1 | grep -q "gcc version"; then LTO=-flto=auto; else LTO=-flto; fi
fi
OUT=${OUT:-run_image_run}
# The program exports its symbols, so a program's own `#c` function is found by name in it;
# `dlopen` is in libdl on Linux and in libc elsewhere.  On Windows (MinGW gcc under MSYS2 or Git
# Bash) the threads are the CRT's, sockets Winsock's, and the main thread gets a Linux one's 8 MiB
# stack, as `loft --native-c` links (`platform::c_system_link_libs`, `c_main_stack_args`); MSVC's
# `cl` is build.cmd's.
THREADS=-pthread
EXPORT=-rdynamic
case "$(uname -s 2>/dev/null || echo unknown)" in
  Linux*) LIBS="-ldl -lm" ;;
  Darwin* | FreeBSD* | NetBSD* | OpenBSD* | DragonFly*) LIBS="-lm" ;;
  MINGW* | MSYS* | CYGWIN*)
    THREADS=
    EXPORT=-Wl,--export-all-symbols
    LIBS="-lm -lws2_32 -lcrypt32 -ladvapi32 -Wl,--stack,8388608"
    case "$OUT" in *.exe) ;; *) OUT="$OUT.exe" ;; esac
    ;;
  *)
    echo "build.sh: no link flags known for $(uname -s 2>/dev/null || echo this host);" \
         "set them by hand (MSVC: build.cmd): \$CC \$CFLAGS \$LTO gen/run_image_run.c and crt.c together, -DLOFT_BIND_LIBS," \
         "symbols exported, the dynamic loader and libm linked" >&2
    exit 2
    ;;
esac
# shellcheck disable=SC2086  # the flag variables are word lists
$CC -std=c99 -w $THREADS $CFLAGS $LTO -I gen -DLOFT_BIND_LIBS gen/run_image_run.c selfhost/crt/shim/crt.c \
  -o "$OUT" $EXPORT $LIBS
