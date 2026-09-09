#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Run a guard against the tree it was written to catch, and say which CHANNEL saw the
# difference.
#
#   scripts/falsify.sh tests/scripts/<guard>.loft <control-ref>
#
# A guard that passes on the build it was written for proves nothing, and the ways that
# happens are not exotic — four turned up in one afternoon (QUALITY.md § B6m): the wrong
# ENTRY POINT (a `main`-less guard under `--interpret` runs no assertion; a `main`-ful one
# under `--tests` runs the helpers), a success marker the error report ECHOES, a leak gate
# that is monotone so an over-free reads as an improvement, and a cell whose shape never
# reaches the code path it was written for.
#
# So this does not ask "does it pass now".  It builds `<control-ref>`, runs the guard THERE
# and HERE through the entry point the corpus runner would pick, and compares six channels
# separately — exit code, assertion failures, leaked stores, panic, stack-store free refusals
# (`BUG (#306)`), and the guard's own `@EXPECT_ERROR` declarations.  The verdict names the
# channel that moved, which is the fact a bare pass/fail hides.
#
# A guard need only move ONE channel on ONE backend.  A backend-divergence guard cannot move
# both by construction, so an inert side is reported as expected rather than counted against
# it; what the gate is for is a guard that moves nothing anywhere (loft#1224).
#
# The control build is cached per ref under the scratch root, so a second guard against the
# same ref costs nothing.
#
# ⚠ ONE CHANNEL IS BLIND, and it is blind for the corpus's usual guard shape.  The leak column
# is read off the run's stderr ("stores not freed at program exit"), which only a `main`-ful
# run under `--interpret` prints: `--tests` does not leak-check at all (the corpus leak gate
# lives in `tests/wrap.rs`, which this does not run).  So a `main`-less guard — the standard
# form — reports `leak none` on BOTH trees whatever it leaks, and a guard written to catch a
# LEAK is therefore recorded INERT, i.e. mislabelled a lock.  Measured 2026-08-27 on
# `a-nullable-return-joins-its-branch-arms.loft`, whose leaking cell `make ci` failed on while
# this reported `0|0|none|none|0` for both trees (QUALITY.md B6p).  Until `--tests` grows a leak
# check, score a leak guard by giving it a `main` and running it under `--interpret`.
#
# ⚠ A SECOND CHANNEL IS BLIND, for the mirror-image reason.  `expect_channel` counts only
# `@EXPECT_ERROR` / `@EXPECT_FAIL`, and `entry_modes` routes only those two to `--tests`.  A
# guard whose subject is a WARNING declares `@EXPECT_WARNING` and, if it also has a `main`
# (which a leak or value guard wants), gets a direct run — where nothing matches warnings at
# all.  Both trees then read `expect -` and the guard is reported INERT however loudly it
# fires.  Measured 2026-09-06 on `a-payload-binding-warns-when-its-subject-is-given-another-\
# variant.loft`: INERT here, 0 -> 2 reports by hand.  Until this grows a warning channel,
# score such a guard by hand — run it on both builds and count the reports.
set -uo pipefail

usage() {
  echo "usage: scripts/falsify.sh <guard.loft> <control-ref>" >&2
  echo "       scripts/falsify.sh <guard.loft> --patch <file>  # control = HEAD + the patch" >&2
  echo "       scripts/falsify.sh --bulk <listfile>   # <guard>TAB<control-ref> per line" >&2
  exit 2
}
BULK=""
REF=""
PATCHFILE=""
PATCH_ABS=""
if [ "${1:-}" = "--bulk" ]; then
  [ $# -eq 2 ] || usage
  BULK="$2"; [ -f "$BULK" ] || { echo "no such list: $BULK" >&2; exit 2; }
elif [ "${2:-}" = "--patch" ]; then
  # The control as a PATCH rather than a ref, for a guard whose control commit no longer
  # exists anywhere.  The patch reintroduces the defect on top of HEAD, so the receipt carries
  # the defect itself instead of a pointer to a build that once had it — nothing outside the
  # file has to survive for it to be re-run.
  [ $# -eq 3 ] || usage
  GUARD="$1"; PATCHFILE="$3"
  [ -f "$GUARD" ] || { echo "no such guard: $GUARD" >&2; exit 2; }
  [ -f "$PATCHFILE" ] || { echo "no such patch: $PATCHFILE" >&2; exit 2; }
  PATCH_ABS="$(cd "$(dirname "$PATCHFILE")" && pwd)/$(basename "$PATCHFILE")"
else
  [ $# -eq 2 ] || usage
  GUARD="$1"; REF="$2"
  [ -f "$GUARD" ] || { echo "no such guard: $GUARD" >&2; exit 2; }
fi

ROOT=$(git rev-parse --show-toplevel)
CACHE="${LOFT_FALSIFY_CACHE:-${TMPDIR:-/tmp}/loft-falsify}"

# Resolve a control ref to a commit, reaching into `refs/pull/*/head` when the clone does not
# already hold it.
#
# A control names the build a guard was written to catch, which is a commit on the branch that
# fixed it — and a squash-merge keeps none of those: the PR lands as ONE commit, so `main` never
# held the original and no branch points at it afterwards.  The object is usually still on the
# remote under the PR's own head, a namespace `git clone` and `git fetch` both skip by default,
# so the ref is missing HERE rather than missing everywhere.  Fetch that namespace once, on the
# first miss only, and retry.
#
# ⚠ This RECOVERS a control; it does not make one durable.  `refs/pull/*` is a GitHub
# convention with no retention contract, so a mirror, a host migration or a policy change drops
# every control that depends on it — and the receipts it cannot reach are already unreachable
# for everyone, not just here.  Measured 2026-09-09 across the 359 guards on `main` that carry a
# control sha: 11 resolve from `main`, 25 from another branch, 199 ONLY from a PR ref, and 124
# from no public ref at all.  So this answers about two thirds of the corpus and cannot answer
# the rest; a receipt whose control no longer exists needs a different FORM rather than a better
# lookup — TESTING.md § What a falsification receipt is worth.
#
# Resolvability is also a property of the CHECKOUT, not of the guard: two clones disagree about
# whether the same receipt is checkable, because each keeps whichever loose objects its own gc
# spared.  Nothing in the file says which side you are on, which is why a miss here reports the
# ref rather than failing silently.
PR_REFS_FETCHED=0
RESOLVED_SHA=""
# The answer comes back in RESOLVED_SHA rather than on stdout so that the once-only fetch latch
# actually latches.  Read through `sha=$(resolve_control …)` the whole function runs in a
# SUBSHELL, where `PR_REFS_FETCHED=1` dies with it — every unresolvable control in a bulk sweep
# then re-fetches the entire PR namespace, once per ref, and the latch reads as working because
# a single call cannot show it failing.
resolve_control() {   # <ref>; sets RESOLVED_SHA; non-zero when the control is unreachable
  local ref="$1"
  # `git rev-parse --verify --quiet` is the form that stays SILENT on a miss.  Plain
  # `git rev-parse "$ref^{commit}"` exits non-zero but still ECHOES its own argument, so a
  # caller testing the output for emptiness reads a missing object as a resolved one.
  if RESOLVED_SHA=$(git rev-parse --verify --quiet "${ref}^{commit}"); then return 0; fi
  if [ "$PR_REFS_FETCHED" -eq 0 ]; then
    PR_REFS_FETCHED=1
    echo "control $ref is not in this clone — fetching refs/pull/*/head …" >&2
    git fetch origin 'refs/pull/*/head:refs/pull/*/head' --quiet >/dev/null 2>&1 </dev/null || true
  fi
  RESOLVED_SHA=$(git rev-parse --verify --quiet "${ref}^{commit}") || { RESOLVED_SHA=""; return 1; }
  echo "control $ref recovered from a PR ref — it is on no branch, so this receipt depends on" >&2
  echo "  GitHub retaining refs/pull/*; it is not durable.  TESTING.md § falsification receipt." >&2
  return 0
}
if [ -n "$PATCHFILE" ]; then
  # Cache the control build against the patch's CONTENT, so editing the patch rebuilds and
  # re-running an unchanged one does not.
  SHA="patch-$(git hash-object "$PATCHFILE" | cut -c1-12)"
  WT="$CACHE/$SHA"; TGT="$CACHE/$SHA-target"
elif [ -z "$BULK" ]; then
  resolve_control "$REF" || {
    echo "unknown ref: $REF" >&2
    echo "  The control is on no branch of this remote and under no refs/pull/*/head, so NO" >&2
    echo "  clone can build it — this receipt records a falsification nobody can re-run." >&2
    echo "  Re-falsify the guard against a control that still exists, or record the" >&2
    echo "  reintroducing patch instead of a ref: TESTING.md § falsification receipt." >&2
    exit 2; }
  SHA=$(git rev-parse --short "$RESOLVED_SHA")
  WT="$CACHE/$SHA"; TGT="$CACHE/$SHA-target"
fi

# ⚠ TEMPORARY, and it should come out: every `--path` below carries a TRAILING SLASH because
# `run_tests` builds the stdlib directory as `default_dir.to_string() + "default"` — a join
# with no separator, so `--path /tree` looks for `/treedefault` and says "cannot load default
# library".  That exit 1 reads as a difference and scored every `main`-less guard as falsified
# by the TREE rather than by the invocation; the first sweep over the corpus lost a quarter of
# its verdicts to it.
#
# This is a caller compensating for a contract defect, which is the kind of thing that outlives
# the memory of why it is here.  It is loft#1112 — delete the slashes when that lands.
#
# The corpus runner (`tests/wrap.rs::run_test`) runs `main` when the file HAS one and every
# zero-parameter function otherwise.  Picking the wrong one is the failure this tool exists
# to stop, so it is derived from the file rather than passed in.
entry_modes() { # <guard> ; sets MODE_I / MODE_N
  # An ANNOTATION-SCORED guard is run THROUGH THE SUITE, whatever its entry point, because the
  # suite is the only thing that peels the file the way its annotations are written to be read.
  #
  # loft#1224 ran these as a plain program instead, reasoning that a direct run PRINTS the
  # diagnostic while `--tests` consumes it, so only the direct run's output carries the thing
  # being compared.  The premise is true and the conclusion does not follow, because a direct
  # run does not see the whole FILE: `Parser::parse` runs pass 2 only when pass 1 finished
  # clean, so ONE pass-1 refusal silences every pass-2 diagnostic in the file, and a mixed guard
  # scored `expect 1/5` with all five cells matching (loft#1253).  The suite has peeled that
  # since loft#1242 — it attributes each error to its enclosing function, blanks that cell and
  # re-parses, checking the UNION of every round.
  #
  # And `--tests` is COMPARABLE after all, on the channel that was thought unusable: a file
  # whose declared errors all occur exits 0, one with an unmatched declaration exits 1.
  # Measured on both guard shapes — the mixed one reads 0 -> 0 (genuinely INERT, which the
  # direct run reported as a misleading `1/5` on both trees) and an all-pass-2 one reads 1 -> 0.
  if grep -qE '@EXPECT_ERROR|@EXPECT_FAIL' "$1"; then
    MODE_I=(--tests); MODE_N=(--tests --native)
  elif grep -qE '^[[:space:]]*fn main[[:space:]]*\(' "$1"; then
    MODE_I=(--interpret); MODE_N=(--native)
  else
    MODE_I=(--tests); MODE_N=(--tests --native)
  fi
}
[ -n "$BULK" ] || entry_modes "$GUARD"

build() { # <dir> <target-dir> -> path to binary
  ( cd "$1" && cargo build --bin loft --target-dir "$2" >/dev/null 2>&1 ) || return 1
  echo "$2/debug/loft"
}

# Six channels, read apart.  A guard scored on one of them can be silent on the others,
# and which one moved is the thing worth printing.
#
# The REFUSAL channel is here because `tests/wrap.rs` Part A2 already fails a corpus file on
# it and this script did not read it — so a guard for an ownership defect that moves only
# that channel scored INERT while `make ci` would have failed on the control.  Two
# consecutive rule-led walks (@FR-L-Null, @FR-O-Proxy) had to measure it by hand.  A
# `BUG (#306)` line means a whole-store free aimed at the eval-stack store that only the
# allocator's guard stopped, and the guard keeps the store alive — which is exactly why
# values, exit code and the leak report can all stay put while it fires.
#
# The EXPECT channel is the same lesson one guard-kind over: an annotation-scored file's
# channel is the diagnostic it declared, and reading only the five above scored it INERT
# whatever it did (loft#1224).

# What the SUITE made of a guard's own `@EXPECT_ERROR` / `@EXPECT_FAIL` declarations —
# "<matched>/<declared>" when it accepted them all, "FAIL/<declared>" when it did not, or "-"
# when the file declares none.
#
# Read off the suite's verdict rather than counted here, and that is the whole of loft#1253's
# fix.  Counting matches in a DIRECT run's output looks equivalent and is not: one pass-1
# refusal silences every pass-2 diagnostic in the file, so a mixed guard scored `1/5` with all
# five cells matching — a number not merely incomplete but readable as its own opposite, which
# sends a reviewer to repair four cells that were never broken.  The suite peels (loft#1242) and
# already knows the answer; asking it is both correct and less code than re-deriving it.
#
# Deliberately NOT a partial count on failure.  The suite reports the file, not the cell, so a
# fraction here would be a guess in exactly the position where a guessed fraction did the
# damage.  `FAIL/6 -> 6/6` says what moved without inventing which cells did.
expect_channel() { # <guard-path> <output> -> "<matched>/<declared>" | "FAIL/<declared>" | "-"
  local file="$1" out="$2" declared matched
  declared=$(sed -n 's/.*@EXPECT_\(ERROR\|FAIL\)://p' "$file" | grep -c .)
  [ "$declared" -eq 0 ] && { echo "-"; return; }
  # `error` / `errors` — the suite pluralises the noun, so a guard declaring exactly ONE
  # expectation prints "1 expected error:" and a plural-only pattern never matched it.  Every
  # single-cell guard therefore scored `FAIL/1` on both trees while the suite ran it green: a
  # column that reads as an unmatched declaration, in the one place a reviewer looks to find
  # out whether the guard is live.  24 of the corpus's guards declare exactly one.
  matched=$(echo "$out" | sed -n 's/.*(\([0-9]\{1,\}\) expected errors\{0,1\}:.*/\1/p' | head -1)
  if [ -n "$matched" ]; then echo "$matched/$declared"; else echo "FAIL/$declared"; fi
}

# Does a signature read as a PASSING run?  One home, asked by the single-guard path and the
# bulk sweep, because they had already drifted: the bulk one compared against the literal
# `0|0|none|none|0` and `signature` has produced SIX fields since loft#1224 added `expect`, so
# every guard in every sweep read `here-not-clean` — including a guard measured clean by the
# single path one line of shell away (loft#1253).  A hand-spelled shape of another function's
# return value is a restated predicate; this is the same class as the one loft#1250 closed.
is_clean() { # <signature> -> 0 when the run passed
  case "$1" in
    0\|0\|none\|none\|0\|FAIL/*) return 1 ;;
    0\|0\|none\|none\|0\|*) return 0 ;;
    *) return 1 ;;
  esac
}

signature() { # <binary> <tree> <guard-path> <extra-args…> ; "exit|asserts|leak|panic|refusals|expect"
  local bin="$1" tree="$2" file="$3"; shift 3
  local out rc
  # `timeout` as well as `LOFT_TIMEOUT`, and the outer one is not redundant: an OLD control
  # running a NEW guard can hang somewhere loft's own watchdog does not reach, and a bulk
  # sweep then stops silently on one file.  Measured — a control run sat for ten minutes
  # against a 180 s `LOFT_TIMEOUT`.  A run the outer bound kills scores `exit 124`, which is
  # a difference like any other and says plainly which side could not finish.
  local lim="${LOFT_FALSIFY_TIMEOUT:-180}"
  # Run IN the tree being scored, not merely with `--path` pointing at it.  A `use <lib>`
  # resolves `lib/` relative to the process CWD, so with both sides run from the checkout
  # the control read THIS tree's libraries and every guard whose subject is a `.loft`
  # library scored INERT — measured on the loft#1259 parser guard, which fails outright
  # against the pre-fix `lib/parser.loft` and reported "the control and this tree answer
  # the same".  A guard is scored against a tree by running it there.
  # The arena instruments pass through when the caller armed them.  A defect that only an
  # instrument can see — a stale work-ref reclaiming, in place, a store number another
  # record has since taken, which `LOFT_POISON=1` turns into a garbage read and plain mode
  # hides behind the allocator's reuse order — is scored on the channel that sees it, and
  # the guard's `@falsified-at` line says which one was armed.
  out=$(cd "$tree" && timeout -k 5 "$((lim + 20))" env LOFT_NATIVE_LEAK_CHECK=1 LOFT_TIMEOUT="$lim" \
        ${LOFT_POISON:+LOFT_POISON="$LOFT_POISON"} \
        ${LOFT_STRICT_STORES:+LOFT_STRICT_STORES="$LOFT_STRICT_STORES"} \
        "$bin" "$@" "$file" 2>&1); rc=$?
  local asserts leak panic refusals
  asserts=$(echo "$out" | grep -c "assertion failed")
  leak=$(echo "$out" | grep -oE "stores not freed at program exit: .*" | head -1 | sed 's/.*exit: //')
  panic=$(echo "$out" | grep -oE "panicked at [^:]*" | head -1)
  refusals=$(echo "$out" | grep -c "BUG (#306)")
  echo "$rc|$asserts|${leak:-none}|${panic:-none}|$refusals|$(expect_channel "$file" "$out")"
}

mkdir -p "$CACHE"

# ── bulk ─────────────────────────────────────────────────────────────────────────────────
# Retrofitting the corpus: one control build per REF rather than per guard, into a SHARED
# target dir so the dependency crates are compiled once (measured 61 s cold, 8.7 s warm).
# Interpret only — the native run costs a rustc invocation per file and the question here is
# "did this guard ever fail", which one backend answers.
if [ -n "$BULK" ]; then
  HERE=$(build "$ROOT" "$CACHE/head-target") || { echo "this tree does not build" >&2; exit 1; }
  SHARED="$CACHE/shared-target"
  # Read the ref list on FD 3, not stdin.  `git worktree add` and `cargo build` both read
  # stdin, and inside a `… | while read` loop they swallow the rest of the list — the first
  # sweep stopped silently after 51 of 186 refs, in order, with an exit status of 0.
  while read -r ref <&3; do
    [ -n "$ref" ] || continue
    # A control the clone cannot resolve is reported APART from a worktree that would not
    # create.  "The receipt names a build nobody has" and "the build is here but unusable" call
    # for different repairs — re-falsify against a live control versus fix the tree — and one
    # status for both hid the first behind the second.
    resolve_control "$ref" || {
      awk -F'\t' -v r="$ref" '$2==r {printf "%s\t%s\tno-such-ref\t\n", $1, r}' "$BULK"; continue; }
    rsha="$RESOLVED_SHA"
    wt="$CACHE/wt-$ref"
    if [ ! -d "$wt" ]; then
      git worktree add --detach "$wt" "$rsha" >/dev/null 2>&1 </dev/null || {
        awk -F'\t' -v r="$ref" '$2==r {printf "%s\t%s\tno-worktree\t\n", $1, r}' "$BULK"; continue; }
    fi
    if ! ( cd "$wt" && cargo build --bin loft --target-dir "$SHARED" >/dev/null 2>&1 </dev/null ); then
      awk -F'\t' -v r="$ref" '$2==r {printf "%s\t%s\tno-build\t\n", $1, r}' "$BULK"
      git worktree remove --force "$wt" >/dev/null 2>&1
      continue
    fi
    while read -r g <&4; do
      # An annotation-scored file used to be skipped here, because run as a plain program its
      # PASSING answer is a refusal and its exit code is 1 on both trees.  `entry_modes` runs it
      # through the suite now (loft#1253), where a passing file exits 0 and an unmatched
      # declaration exits 1 — so it is scoreable like any other and the sweep no longer has a
      # blind category.
      entry_modes "$ROOT/$g"
      c=$(signature "$SHARED/debug/loft" "$wt" "$ROOT/$g" --path "$wt/" "${MODE_I[@]}")
      h=$(signature "$HERE" "$ROOT" "$ROOT/$g" --path "$ROOT/" "${MODE_I[@]}")
      if ! is_clean "$h"; then
        printf '%s\t%s\there-not-clean\t%s\n' "$g" "$ref" "$h"
      elif [ "$c" = "$h" ]; then
        printf '%s\t%s\tINERT\t%s\n' "$g" "$ref" "$c"
      else
        ch=""
        for i in 1 2 3 4 5 6; do
          cf=$(echo "$c" | cut -d'|' -f$i); hf=$(echo "$h" | cut -d'|' -f$i)
          [ "$cf" = "$hf" ] && continue
          case $i in
            1) d="exit $cf -> $hf";; 2) d="$cf assertion failures -> $hf";;
            3) d="leaked $cf -> clean";; 4) d="panicked -> clean";;
            5) d="$cf stack-store free refusal(s) (BUG #306) -> $hf";;
            6) d="expectations $cf -> $hf";;
          esac
          [ -n "$ch" ] && ch="$ch, "; ch="$ch$d"
        done
        printf '%s\t%s\tfalsified\t%s\n' "$g" "$ref" "$ch"
      fi
    done 4< <(awk -F'\t' -v r="$ref" '$2==r {print $1}' "$BULK")
    git worktree remove --force "$wt" >/dev/null 2>&1
  done 3< <(cut -f2 "$BULK" | sort -u)
  exit 0
fi
# ─────────────────────────────────────────────────────────────────────────────────────────

if [ ! -x "$TGT/debug/loft" ]; then
  if [ -n "$PATCHFILE" ]; then
    [ -d "$WT" ] || {
      git worktree add --detach "$WT" HEAD >/dev/null 2>&1 || {
        echo "cannot create a worktree at HEAD" >&2; exit 1; }
      # A patch receipt is checked before it is trusted.  One records the fix as it was, so it
      # applies to the tree it was cut against and drifts out as that tree moves — refusing here
      # is the receipt telling you it has gone stale, which a dangling sha never gets to do.
      ( cd "$WT" && git apply "$PATCH_ABS" ) || {
        echo "the recorded patch no longer applies to HEAD: $PATCHFILE" >&2
        echo "  The receipt still RECORDS the defect, but it can no longer be re-run here." >&2
        echo "  Re-derive it against a tree it applies to, or score the guard by hand." >&2
        git worktree remove --force "$WT" >/dev/null 2>&1; exit 3; }
    }
    echo "building the control from $PATCHFILE (cached at $TGT) …" >&2
  else
    [ -d "$WT" ] || git worktree add --detach "$WT" "$SHA" >/dev/null 2>&1 || {
      echo "cannot create a worktree at $SHA" >&2; exit 1; }
    echo "building the control at $SHA (cached at $TGT) …" >&2
  fi
  build "$WT" "$TGT" >/dev/null || { echo "the control does not build" >&2; exit 1; }
fi
CONTROL="$TGT/debug/loft"
# The control build is a full debug `target/` (~2 GB: the native leg links `libloft.rlib` and
# its dependency rlibs, so the binary alone is not enough).  Kept per ref and never pruned,
# these reached 364 GB and filled the disk (2026-09-05).  Keep the LOFT_FALSIFY_KEEP most
# recently USED controls (default 4) — this one is stamped now — and remove the rest, their
# worktrees with them.
touch "$TGT" "$WT" 2>/dev/null
keep="${LOFT_FALSIFY_KEEP:-4}"
for old in $(ls -td "$CACHE"/*-target 2>/dev/null | tail -n +$((keep + 1))); do
  case "$old" in */head-target|*/shared-target) continue;; esac
  sha=${old##*/}; sha=${sha%-target}
  git -C "$ROOT" worktree remove --force "$CACHE/$sha" >/dev/null 2>&1 || rm -rf "$CACHE/$sha"
  rm -rf "$old"
done
git -C "$ROOT" worktree prune >/dev/null 2>&1
# A separate target dir on purpose: the main one may be mid-`make ci`, and cargo's build
# lock is per target dir — building into it stalls a gate that is already running.
HERE=$(build "$ROOT" "$CACHE/head-target") || { echo "this tree does not build" >&2; exit 1; }

# loft#1224 — the verdict is an OR across backends, not an AND, and it names the inert side.
#
# A guard for a BACKEND DIVERGENCE can only move one channel by construction: if both backends
# moved it would not be a divergence.  Scoring `fail=1` on any inert backend therefore reported
# NOT FALSIFIED for every such guard — measured on the native-only loft#1217 and loft#1222,
# where native went 1 -> 0 and interpret was correctly identical on both trees.  What the gate
# is for is a guard that moves NOTHING, so that is what it now reports; a backend that stays put
# while its sibling moves is named rather than counted as a failure.
falsified_any=0
notclean=0
INERT_SIDES=""
CHANNELS=""
printf '%-12s %-10s %-46s %s\n' backend tree "exit|asserts|leak|panic|refusals|expect" verdict
for pair in "interpret ${MODE_I[*]}" "native ${MODE_N[*]}"; do
  name=${pair%% *}; args=${pair#* }
  # shellcheck disable=SC2086
  c=$(signature "$CONTROL" "$WT" "$ROOT/$GUARD" --path "$WT/" $args)
  # `--path` for BOTH sides: the binary is built into its own target dir and has no
  # `default/` beside it, so without this it cannot load the stdlib and exits 1 — which
  # reads as a difference and would score every guard as falsified for the wrong reason.
  # shellcheck disable=SC2086
  h=$(signature "$HERE" "$ROOT" "$ROOT/$GUARD" --path "$ROOT/" $args)
  # loft#1224 — "clean" means the guard PASSES, and for an annotation-scored file passing is a
  # refusal: it exits 1 and prints the message it declared.  Judging it by exit code alone
  # reported THIS TREE IS NOT CLEAN for a guard that was working exactly as written.  So a file
  # that declares expectations is clean when it produced all of them, and every other file is
  # clean when it exits 0 with nothing leaked, asserted, panicked or refused.
  # An annotation-scored file needs no special case any more.  Under `--tests` its passing
  # answer is an ORDINARY pass — exit 0, nothing leaked, asserted, panicked or refused — because
  # the suite consumes the declared diagnostics instead of letting them fail the run.  loft#1224
  # needed the special case only because the file was run as a plain program, where a passing
  # refusal guard exits 1; loft#1253 moved it onto the suite and the exception went with it.
  clean_here="ok"
  is_clean "$h" || clean_here="NOT-CLEAN"
  if [ "$c" = "$h" ]; then
    verdict="INERT — the control and this tree answer the same"
    [ -n "$INERT_SIDES" ] && INERT_SIDES="$INERT_SIDES, "
    INERT_SIDES="$INERT_SIDES$name"
  elif [ "$clean_here" != "ok" ]; then
    verdict="THIS TREE IS NOT CLEAN"
    notclean=1
  else
    verdict="falsified"
    falsified_any=1
    # Name the channel that moved, so the recorded line says what was measured rather
    # than only that something was.
    for i in 1 2 3 4 5 6; do
      cf=$(echo "$c" | cut -d'|' -f$i); hf=$(echo "$h" | cut -d'|' -f$i)
      [ "$cf" = "$hf" ] && continue
      case $i in
        1) d="exit $cf -> $hf";;
        2) d="$cf assertion failures -> $hf";;
        3) d="leaked $cf -> clean";;
        4) d="panicked -> clean";;
        5) d="$cf stack-store free refusal(s) (BUG #306) -> $hf";;
        6) d="expectations $cf -> $hf (the suite's verdict, not a count of matches)";;
      esac
      [ -n "$CHANNELS" ] && CHANNELS="$CHANNELS, "
      CHANNELS="$CHANNELS$name $d"
    done
  fi
  printf '%-12s %-10s %-46s %s\n' "$name" control "$c" ""
  printf '%-12s %-10s %-46s %s\n' "$name" here "$h" "$verdict"
done

echo
if [ $notclean -eq 1 ]; then
  echo "NOT falsified.  This tree does not pass the guard, so nothing here says whether the"
  echo "guard can CATCH anything — fix the tree first, then re-run."
  exit 1
elif [ $falsified_any -eq 1 ]; then
  # An inert backend beside a moved one is expected for a backend-divergence guard, so say so
  # in the recorded line rather than withholding the verdict (loft#1224).
  [ -n "$INERT_SIDES" ] && CHANNELS="$CHANNELS; $INERT_SIDES INERT (expected for a
  backend-divergence guard — only one side can move)"
  echo "Paste this into $GUARD:"
  if [ -n "$PATCHFILE" ]; then
    echo "// @falsified-by: $PATCHFILE — $CHANNELS"
  else
    echo "// @falsified-at: $SHA — $CHANNELS"
    # A ref receipt starts decaying the moment its PR merges: the squash keeps no branch
    # pointing at the control, and the count of unreachable ones grows on its own as merged
    # branches are pruned — 124 to 133 over a few hours of one afternoon.  The patch that would
    # rescue it is derivable exactly ONCE for free, here, where the control is still resolvable
    # and the diff applies by construction; later it is hand-reconstruction, and for two thirds
    # of the corpus it is already too late.  So the durable form is offered at the only moment
    # it costs nothing.
    pdir="$ROOT/tests/falsified"
    pbase=$(basename "$GUARD" .loft)
    mkdir -p "$pdir"
    git -C "$ROOT" diff -R "$SHA" -- src/ default/ > "$pdir/$pbase.patch" 2>/dev/null || true
    plines=$(wc -l < "$pdir/$pbase.patch" 2>/dev/null || echo 0)
    # A patch receipt is only a receipt while it isolates ONE defect.  Run against the commit
    # just before the fix — the documented use — the diff IS the fix and runs to a few dozen
    # lines; run against a DISTANT control it becomes the whole source difference since, which
    # reintroduces everything fixed in between and cannot say which one the guard caught.  A
    # measured 47079-line "receipt" is the shape of that mistake.  The recorded receipts run
    # 17 to 203 lines, so a bound well above them separates the two uses without tuning.
    if [ "${plines:-0}" -eq 0 ]; then
      rm -f "$pdir/$pbase.patch"
      echo
      echo "note: no durable patch written — the fix touches nothing under src/ or default/,"
      echo "      so the control cannot be reconstructed from a source diff alone."
    elif [ "$plines" -gt 800 ]; then
      rm -f "$pdir/$pbase.patch"
      echo
      echo "note: no durable patch written — $SHA is $plines source lines from this tree, so a"
      echo "      diff against it reintroduces everything fixed in between, not this defect."
      echo "      Re-run against the commit immediately before the fix to get one."
    else
      echo
      echo "…and the receipt that does not decay — written to tests/falsified/$pbase.patch."
      echo "Score it with: scripts/falsify.sh $GUARD --patch tests/falsified/$pbase.patch"
      echo "// @falsified-by: tests/falsified/$pbase.patch — $CHANNELS"
    fi
  fi
  exit 0
else
  echo "NOT falsified.  A guard that answers the same on the build it was written for is"
  echo "measuring something other than the defect — check the ENTRY POINT above first."
  exit 1
fi
