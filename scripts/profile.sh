#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Sampling profiler for loft — "which line is this run spending its time in?"
#
#   scripts/profile.sh -- --interpret prog.loft             # profile a loft run
#   scripts/profile.sh --mem -- --interpret prog.loft       # where the HEAP went
#   scripts/profile.sh --paths -- --interpret prog.loft     # + paths to allocations
#   scripts/profile.sh --engine -- --check prog.loft        # profile LOFT ITSELF
#   scripts/profile.sh --annotate -- --check prog.loft      # + source-line view
#   scripts/profile.sh --calls -- --check prog.loft         # + who called the hot fn
#   scripts/profile.sh --no-cache -- --check prog.loft      # compile, don't reload
#   scripts/profile.sh --no-warm -- prog.loft               # don't pre-build (native)
#   scripts/profile.sh --keep -- …                          # keep perf.data
#
# Everything after `--` is passed to the loft binary unchanged.
#
# TWO PROFILERS, and picking the wrong one answers a question you did not ask.
# `perf` measures the ENGINE — loft's own Rust — and for a `--native` run that is
# also your program, because your functions were compiled into it as `n_<yours>`.
# For an `--interpret` run it is not: a loft call creates no machine frame, so
# perf's stack is the interpreter's own, identical for every program ever run.  So
# an interpreted program is measured by loft's own sampler over its own call stack
# (@PLN140 arc B), and `--engine` forces perf when the interpreter itself is what
# you are asking about.  With neither flag the script picks, and says which.
#
# Four defaults here are the difference between a useful profile and a
# misleading one, so they are not options:
#
#   SELF TIME, not inclusive.  loft's hot paths are recursive tree walkers
#   (`scopes::scan`, `use_analysis::collect_defs`).  Inclusive time attributes
#   ~100% to the walker at the root and names nothing; self time names the
#   function actually burning the cycles.  Pass --calls when you then want to
#   know who calls it.
#
#   FRAME-POINTER unwinding, not DWARF.  `--call-graph=dwarf` copies stack
#   memory per sample; against a walker that recurses hundreds deep that is slow
#   AND silently truncates the chains you came for.  The profiling profile is
#   built with `-Cforce-frame-pointers=yes` so `fp` unwinding is exact and cheap.
#
#   The `profiling` cargo profile, not `release`.  Same optimisation, plus the
#   line tables `perf annotate` needs.  Release stays byte-identical (RELEASE.md
#   pins its sha; `make speed` measures it).
#
#   THE BUILD IS NOT THE RUN.  loft's default backend is the COMPILER: `loft
#   prog.loft` generates Rust, shells out to rustc, and runs the binary it built.
#   perf follows forks, so recording that command records the BUILD — rustc, LLVM
#   and lld take the whole top of the profile, and the few samples that ARE the
#   program come back as bare hex because the binary is stripped.  A native run is
#   therefore built once unprofiled (`--no-warm` opts out) and profiled with
#   `--native-debug`, and what is left of rustc is reported.
set -uo pipefail
cd "$(dirname "$0")/.."

ANNOTATE=0; CALLS=0; KEEP=0; NOCACHE=0; WARM=1; FREQ=${LOFT_PROFILE_FREQ:-499}
MEM=0; PATHS=0; PICK=auto; OPS=${LOFT_PROFILE_OPS:-1024}
while [ $# -gt 0 ]; do
  case "$1" in
    --annotate) ANNOTATE=1; shift;;
    --calls)    CALLS=1; shift;;
    --keep)     KEEP=1; shift;;
    --no-cache) NOCACHE=1; shift;;
    --no-warm)  WARM=0; shift;;
    --mem)      MEM=1; shift;;
    --paths)    MEM=1; PATHS=1; shift;;
    --engine)   PICK=engine; shift;;
    --program)  PICK=program; shift;;
    --freq)     FREQ="$2"; shift 2;;
    --ops)      OPS="$2"; shift 2;;
    --)         shift; break;;
    # Anchored on the text, not a line range: adding a usage line must not
    # silently truncate --help.
    -h|--help)  sed -n '/^# Sampling profiler/,/^# Everything after/p' "$0" | sed 's/^# \?//'; exit 0;;
    *) echo "profile.sh: unknown option '$1' (did you forget '--' before the loft args?)" >&2; exit 2;;
  esac
done
if [ $# -eq 0 ]; then
  echo "profile.sh: nothing to profile — pass loft's arguments after '--'" >&2
  echo "  e.g. scripts/profile.sh -- --interpret prog.loft" >&2
  exit 2
fi

OUT="${LOFT_PROFILE_DIR:-${TMPDIR:-/tmp}}/loft-profile"
mkdir -p "$OUT"
DATA="$OUT/perf.data"

# ── is this a native run, i.e. one that rustc has to build first? ─────────────
#
# Every loft subcommand is the FIRST positional token (`src/main.rs`: "every
# subcommand is the FIRST positional, never a later one"), so a plain program run
# is exactly "the first positional is a .loft path" — minus the modes that stop
# before rustc is ever reached.  The value-taking options are stepped over so
# their argument is not mistaken for the script.
#
# Getting this wrong is not silent: the rustc-share guard after recording says so.

# Set before the warm-up, not just before recording: the warm-up only warms the
# right build if it runs in the same environment as the run it is warming for.
if [ "$NOCACHE" = 1 ]; then export LOFT_NO_CACHE=1; fi

NATIVE=1
RUNS_PROGRAM=1     # does this invocation actually EXECUTE the user's loft code?
TESTS=0            # a test run: interprets by default, and IS loft code (loft#860)
skip_value=0
first_positional=""
for a in "$@"; do
  if [ "$skip_value" = 1 ]; then skip_value=0; continue; fi
  case "$a" in
    --path|--project|--lib|--log-conf|--timeout)                 skip_value=1;;
    --interpret|--repl)                                          NATIVE=0;;
    --tests)                                                     TESTS=1;;
    --check|--dump)                                              NATIVE=0; RUNS_PROGRAM=0;;
    --native-emit*|--native-wasm*|--html*|--native-android*)      NATIVE=0; RUNS_PROGRAM=0;;
    -*) ;;
    *)  [ -z "$first_positional" ] && first_positional="$a";;
  esac
done
case "$first_positional" in *.loft) ;; *) NATIVE=0; RUNS_PROGRAM=0;; esac

# loft#860 — a test run executes loft code under the interpreter, so the loft-level
# sampler is the right instrument for it, exactly as for `--interpret prog.loft`.
# It is not caught by the rule above: `loft test` has no `.loft` positional at all,
# and `loft --tests p.loft` has one but reaches it through a runner that defaults to
# the interpreter whatever the backend flags say.  Without this, `make profile
# ARGS="test"` recorded perf over the loft binary and answered with loft's own Rust
# — a profile of the harness, offered as a profile of the suite.
[ "$first_positional" = test ] && TESTS=1
if [ "$TESTS" = 1 ]; then
  RUNS_PROGRAM=1
  # An EXPLICIT --native is the one case a test run is not interpreted.
  case " $* " in *" --native "*) NATIVE=1;; *) NATIVE=0;; esac
fi

# ── which instrument answers the question that was asked? ─────────────────────
#
# `--check` never runs the program, so there is no program to profile: the only
# thing burning cycles is loft's own front end.  A `--native` run compiled the
# program into the binary perf is sampling, so perf names it.  An interpreted
# program is the one case where perf's stack is structurally the wrong stack, so
# that is where loft's own sampler goes.
if [ "$PICK" = auto ]; then
  if [ "$RUNS_PROGRAM" = 1 ] && [ "$NATIVE" = 0 ]; then PICK=program; else PICK=engine; fi
fi
if [ "$MEM" = 1 ] && [ "$PICK" = engine ] && [ "$NATIVE" = 1 ]; then
  # @PLN140 open question 3, DECLINED rather than left implicit.  The allocation
  # site is `alloc_pc`, republished per op by the interpreter's dispatch loop.  A
  # native binary has no dispatch loop, so every store it allocates carries site 0
  # and the report would be one row saying "line 0" — a table with the shape of an
  # answer and none of the content.  Memory attribution is an --interpret
  # instrument; the same program interpreted allocates from the same lines.
  echo "profile.sh: --mem needs the interpreter." >&2
  echo "  The allocation site is the interpreter's bytecode position; a native binary" >&2
  echo "  has none, so every site would read 'line 0'.  Re-run the same program with" >&2
  echo "  --interpret — it allocates from the same loft lines." >&2
  exit 2
fi

# ── the loft-level instruments: no perf, no build-vs-run problem ──────────────
if [ "$PICK" = program ]; then
  echo "── building (release) ──" >&2
  cargo build --release --bin loft >&2 || exit 1
  RBIN=target/release/loft
  envs=()
  if [ "$MEM" = 1 ]; then envs+=(LOFT_ALLOC_SITES=1); fi
  if [ "$PATHS" = 1 ]; then envs+=(LOFT_ALLOC_PATHS=16); fi
  # CPU sampling is the default; --mem alone means the question was about memory.
  if [ "$MEM" = 0 ]; then envs+=("LOFT_PROFILE=$OPS"); fi
  echo "── running (loft's own sampler over its own call stack) ──" >&2
  env "${envs[@]}" "$RBIN" "$@"
  rc=$?
  echo
  echo "(--engine profiles loft ITSELF with perf; --mem for the heap, --paths for" >&2
  echo " the call paths that reached each allocation)" >&2
  exit $rc
fi

command -v perf >/dev/null 2>&1 || {
  echo "profile.sh: perf is not installed." >&2
  echo "  Debian/Ubuntu: sudo apt-get install linux-tools-common linux-tools-\$(uname -r)" >&2
  exit 1
}

# perf needs permission to sample. Report the exact fix rather than the kernel's
# bare 'Permission denied', which names neither the knob nor the value.
PARANOID=$(cat /proc/sys/kernel/perf_event_paranoid 2>/dev/null || echo 4)
if [ "$PARANOID" -gt 2 ] 2>/dev/null; then
  echo "profile.sh: perf_event_paranoid is $PARANOID — sampling a user process needs <= 2." >&2
  echo "  One-time, persists across reboots:" >&2
  echo "    echo 'kernel.perf_event_paranoid = 2' | sudo tee /etc/sysctl.d/99-perf.conf" >&2
  echo "    sudo sysctl --system" >&2
  echo "  Or for this boot only:  sudo sysctl -w kernel.perf_event_paranoid=2" >&2
  exit 1
fi

echo "── building (profiling profile: release + line tables, frame pointers) ──" >&2
RUSTFLAGS="${RUSTFLAGS:-} -Cforce-frame-pointers=yes" \
  cargo build --profile profiling --bin loft >&2 || exit 1
BIN=target/profiling/loft

if [ "$NATIVE" = 1 ]; then
  # `--native-debug` is the lever that survives the binary cache, and the only
  # one: the cache key hashes THIS FLAG (src/main.rs), not the environment, so
  # `LOFT_NATIVE_KEEP_SYMBOLS=1` on its own hands back an already-cached STRIPPED
  # binary and changes nothing.  The flag keeps symbols, emits DWARF line tables,
  # and preserves the generated .rs that `--annotate` needs to show a source line.
  # It adds debug info without touching optimisation, so the run profiled is the
  # run that was asked for.  It goes in FRONT: loft forwards every flag AFTER the
  # script path to the program, so appending it would hand it to the program.
  has_nd=0
  for a in "$@"; do [ "$a" = "--native-debug" ] && has_nd=1; done
  [ "$has_nd" = 0 ] && set -- --native-debug "$@"

  if [ -n "${LOFT_NATIVE_NO_CACHE:-}" ]; then
    echo "note: LOFT_NATIVE_NO_CACHE is set, so the build cannot be cached —" >&2
    echo "      rustc will compile inside the sampled window." >&2
  elif [ "$WARM" = 1 ]; then
    # One unprofiled run to get rustc's work into the binary cache, so recording
    # covers the program rather than the compiler.  It really does run the
    # program, side effects and all — hence the announcement, and --no-warm.
    echo "── warming (building the native binary; your program runs once, unprofiled) ──" >&2
    WARMLOG="$OUT/warm.log"
    if ! "$BIN" "$@" >/dev/null 2>"$WARMLOG"; then
      echo "note: the warm-up run exited non-zero. Its last stderr lines:" >&2
      tail -5 "$WARMLOG" >&2
    fi
    rm -f "$WARMLOG"
  fi
fi

echo "── recording (perf, ${FREQ}Hz) ──" >&2
# `cpu-clock:u` — a USER-SPACE software clock: it needs only `perf_event_paranoid <= 2`
# (level 1 is what kernel frames need, and self time in loft's own Rust never wants
# them), and it exists on every box, including VMs without hardware counters.
perf record -e cpu-clock:u -F "$FREQ" --call-graph=fp -o "$DATA" -- "$BIN" "$@" >/dev/null
rc=$?
if [ ! -s "$DATA" ]; then
  echo "profile.sh: perf wrote no samples (exit $rc) — the run may have been too short to sample." >&2
  exit 1
fi

# A percentage computed from a handful of samples is noise wearing a number's
# clothes — at 50 samples a 2% row is one sample, and `--annotate` will annotate
# whatever won the coin toss (it has picked a 1-sample `getenv` from libc).  The
# count is printed rather than the rows filtered, because the fix is a longer run
# or a higher --freq, not a quieter report.
REPORT=$(perf report -i "$DATA" --stdio --no-children -g none --percent-limit 1 2>/dev/null)
SAMPLES=$(printf '%s\n' "$REPORT" | sed -n 's/^# Samples: *\([0-9KMG.]*\).*/\1/p' | head -1)

echo
echo "════ self time — ${SAMPLES:-?} samples (where the cycles actually burn) ════"
SELF=$(printf '%s\n' "$REPORT" | grep -vE '^#|^$' | head -20)
echo "$SELF"

# A native profile can still be a profile of the BUILD — the warm-up was skipped,
# the binary cache was disabled, or the source changed between the two runs.  A
# top full of rustc/LLVM/lld reads exactly like a hot program (plausible symbols,
# real percentages), so the share is measured and named rather than left to be
# recognised.  `--sort comm` is what separates them: rustc's own threads are
# `rustc`, LLVM's codegen units `opt cgu.N`, the linker `rust-lld`.
BUILD_PCT=$(perf report -i "$DATA" --stdio --no-children -g none --sort comm --percent-limit 0 \
  2>/dev/null | awk '
    /^[[:space:]]*[0-9]/ {
      pct = $1; sub("%", "", pct)
      comm = $0; sub("^[[:space:]]*[0-9.]+%[[:space:]]+", "", comm)
      if (comm ~ /^(rustc|rust-lld|ld|cc|cc1|cc1plus|collect2|opt cgu)/) s += pct
    }
    END { printf "%.0f", s + 0 }')
if [ "${BUILD_PCT:-0}" -ge 20 ] 2>/dev/null; then
  echo
  echo "⚠  ${BUILD_PCT}% of this profile is the BUILD (rustc / LLVM / lld), not your program." >&2
  if [ "$RUNS_PROGRAM" = 0 ] && [ "$NATIVE" = 0 ]; then
    # `--check` alone is not a front-end-only run: the DEFAULT backend is the
    # compiler, so `check_only` falls through the native pipeline and rustc
    # compiles the program it will not run.  Naming the missing flag beats
    # repeating the generic advice, which does not apply here.
    echo "   \`--check\` still goes through the native backend — the default backend IS the" >&2
    echo "   compiler, so rustc builds a binary it then does not run. Add --interpret" >&2
    echo "   (\`-- --interpret --check p.loft\`) to profile loft's front end alone." >&2
  else
    echo "   The native binary was compiled inside the sampled window. Drop --no-warm," >&2
    echo "   or unset LOFT_NATIVE_NO_CACHE, so the build is cached before recording." >&2
  fi
elif [ "$NATIVE" = 1 ]; then
  echo
  # The program's command name is not fixed — a cache hit runs `<stem>-<hash>`,
  # a fresh compile runs `loft_native_bin_<pid>` — so it is described by what it
  # is not, rather than by a name that would be wrong half the time.
  echo "(the 'loft' rows are the front end — parse, IR, codegen, cache lookup;"
  echo " the other command is your compiled program, its fns named n_<yours>)"
fi

# A profile of a CACHE HIT looks like a profile of a compile, and reads as one:
# same command, same output, a tenth of the time, and a flat graph that blames
# the store loader.  loft answers a second run of an unchanged file from the
# startup cache, so this is the normal outcome of profiling the same file twice
# — which is how it wastes an afternoon rather than announcing itself.
if echo "$SELF" | grep -qE "startup_cache|ir_read::open_bundle|warm_load"; then
  echo
  echo "⚠  this run was answered from the STARTUP CACHE, not compiled." >&2
  echo "   You are profiling the cache loader. Re-run with --no-cache, or point" >&2
  echo "   at a file whose content has changed since the last run." >&2
fi

if [ "$CALLS" = 1 ]; then
  echo
  echo "════ callers of the hot functions ════"
  perf report -i "$DATA" --stdio --no-children -g graph,0.5,caller --percent-limit 3 2>/dev/null |
    grep -vE '^#|^$' | head -60
fi

if [ "$ANNOTATE" = 1 ]; then
  HOT=$(printf '%s\n' "$SELF" | head -1 | sed 's/.*\] //')
  # The last honesty guard, and the one that cost the most to learn: `--annotate`
  # takes whichever symbol came out on top, and on a short run that is a coin toss
  # — during testing it annotated a ONE-SAMPLE `getenv` from libc and printed forty
  # lines of disassembly about it, which reads exactly like a finding.  So the
  # winner's own sample count is reconstructed from its share and refused when it
  # is too small to mean anything.  Printing "too few samples" is the useful
  # output here; the disassembly would not have been.
  HOT_PCT=$(printf '%s\n' "$SELF" | head -1 | sed -n 's/^[[:space:]]*\([0-9.]*\)%.*/\1/p')
  # `$SAMPLES` is perf's own count, which it abbreviates (`42K`) — so the suffix is
  # expanded here rather than silently parsed as 42.
  HOT_SAMPLES=$(awk -v p="${HOT_PCT:-0}" -v s="${SAMPLES:-0}" 'BEGIN {
      mult = 1; n = s
      if (n ~ /K$/) { mult = 1000;       sub("K", "", n) }
      else if (n ~ /M$/) { mult = 1000000;    sub("M", "", n) }
      else if (n ~ /G$/) { mult = 1000000000; sub("G", "", n) }
      printf "%d", p * n * mult / 100
    }')
  echo
  if [ "${HOT_SAMPLES:-0}" -lt 50 ] 2>/dev/null; then
    echo "⚠  not annotating $HOT: its ${HOT_PCT}% rests on about ${HOT_SAMPLES} samples." >&2
    echo "   At that count the top symbol is whichever won the toss — the annotation" >&2
    echo "   would be precise about noise. Raise --freq, or profile a longer run." >&2
  else
    echo "════ source lines in: $HOT (~${HOT_SAMPLES} samples) ════"
    perf annotate -i "$DATA" --stdio -l --percent-limit 1 "$HOT" 2>/dev/null | head -40
  fi
fi

echo
if [ "$KEEP" = 1 ]; then
  echo "perf.data kept at $DATA — 'perf report -i $DATA' for the interactive view."
else
  rm -f "$DATA"
  echo "(--keep retains perf.data for 'perf report'; --annotate for source lines; --calls for callers; --no-cache to compile rather than reload; --no-warm to skip the native pre-build)"
fi
