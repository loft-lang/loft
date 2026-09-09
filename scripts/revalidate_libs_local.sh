#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# revalidate-libs, on this box, from a work branch.
#
# `.github/workflows/revalidate-libs.yml` compiles every PUBLISHED library against
# this loft, and it triggers on `pull_request` and `push` to `main` ONLY — never on
# a work branch.  A language change that retro-breaks a shipped library is therefore
# invisible for as long as the branch stays unmerged, which on a bundled branch is
# days.  On 2026-08-19 that was nine libraries losing their entire public surface to
# one resolution rule, green on every branch gate for a full day
# (DEVELOPMENT.md § Opening a PR is the owner's call).
#
# Running ONE library's suite is the advice that incident produced, and it is not the
# gate: the gate is the whole registry.  This script is that, locally.
#
# What it does, per published package at its LATEST non-yanked version:
#   1. reads the matrix from ../loft-registry/index.json through
#      `scripts/revalidate_matrix.py` — the same index AND the same policy the
#      workflow's `discover` job uses, so which packages are checked, at which
#      version, and which are deliberately skipped cannot differ between them,
#   2. extracts the release TAG's tree with `git archive` from the sibling clone,
#   3. runs `loft --interpret --tests tests` against the loft you built,
#   4. on failure re-classifies exactly as the workflow does — recompile every
#      `.loft` with `--dump`; still compiles => environment/native-deps (this box
#      lacks ALSA, a display, a network); fails to compile => a genuine language
#      break, which is what the freeze forbids.
#
# READ-ONLY on the sibling repos, deliberately.  `git archive <tag> | tar -x` into a
# scratch directory writes nothing to their tree or their `.git`, which matters
# because those repos are usually somebody's live working tree — a `git checkout`
# there lands in another agent's uncommitted work, and `loft test` INSIDE a package
# rebuilds `native-auto/` and writes `.loft/` caches (CLAUDE.md § Dogfood loop).
#
# Usage:  scripts/revalidate_libs_local.sh [--self-test] [package ...]
#
#   --self-test   inject a compile break and a runtime break into one package and
#                 assert the two are reported DIFFERENTLY.  A sweep that reports
#                 "all green" is worth nothing until its harness is shown able to
#                 go red, and to distinguish the two classes it claims to.
#
# Exit status is 1 if any package COMPILE-BREAKS; a runtime/env failure is reported
# and does not fail the run (the workflow makes the same call, for the same reason).
set -uo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
siblings="$(dirname "$root")"
registry="$siblings/loft-registry/index.json"
loft="$root/target/release/loft"
work="${TMPDIR:-/tmp}/loft-revalidate-$$"
here="$root/scripts"
warn="$work/warn"
self_test=0
want=()
for a in "$@"; do
  case "$a" in
    --self-test) self_test=1 ;;
    -h|--help) sed -n '5,45p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) want+=("$a") ;;
  esac
done

[ -x "$loft" ] || { echo "no release binary at $loft — run: cargo build --release --bin loft" >&2; exit 2; }
[ -f "$registry" ] || { echo "no registry index at $registry — clone loft-lang/loft-registry beside this checkout" >&2; exit 2; }
mkdir -p "$work" "$warn"
trap 'rm -rf "$work"' EXIT

# The matrix is only as current as the sibling CLONE of the registry, and a clone that
# lags publishes a green run about the versions it happens to hold.  Say which index this
# is, and say so LOUDLY when the checkout is behind what it has already fetched — a
# package published after this commit is not skipped, it is absent, and an absent package
# is invisible to every count below.
reg_clone="$(dirname "$registry")"
echo "registry index: $(git -C "$reg_clone" log --oneline -1 2>/dev/null || echo 'not a git checkout') \
($(git -C "$reg_clone" log -1 --format=%cs 2>/dev/null || echo 'unknown date'))"
if upstream="$(git -C "$reg_clone" rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null)" \
   && behind="$(git -C "$reg_clone" rev-list --count "HEAD..$upstream" 2>/dev/null)" \
   && [ "${behind:-0}" -gt 0 ]; then
  echo "  ⚠ $behind commit(s) behind $upstream — run: git -C $reg_clone pull" >&2
fi

# The matrix: package -> latest installable version, its repo, its tag, its subpath.
#
# Read from `scripts/revalidate_matrix.py`, which the WORKFLOW reads too.  This file used
# to compute it inline, and "exactly as the workflow does" then drifted on four separate
# questions — the compiler's own package, the known-broken map, the `subpath` default, and
# whether yanked versions count.  The one that showed was the first: `loft` is the
# COMPILER, its `tests/` holds fixtures that are deliberately not standalone programs, and
# the re-classifier read them as a language break — so this script printed
# `1 COMPILE-BREAK` and exited 1 on a tree with nothing wrong in it, against the binary the
# package was published with (loft#1315).  A gate whose zero is not zero cannot measure the
# thing it was built for.  The exclusions are printed, for the same reason: a skip nobody
# sees is indistinguishable from a package that passed.
matrix="$work/legs.tsv"
python3 "$root/scripts/revalidate_matrix.py" "$registry" > "$matrix" || {
  echo "cannot read the registry matrix from $registry" >&2; exit 2; }

# Extract one package's release tree into $work and echo its package directory.
extract() {                       # extract <repo> <tag> <sub> <dest-name>
  local repo="$1" tag="$2" sub="$3" dest="$4"
  local clone="$siblings/$(basename "$repo")"
  git -C "$clone" rev-parse --verify -q "refs/tags/$tag" >/dev/null 2>&1 || return 1
  rm -rf "${work:?}/$dest"; mkdir -p "$work/$dest"
  git -C "$clone" archive "$tag" | tar -x -C "$work/$dest" 2>/dev/null || return 1
  [ -d "$work/$dest/$sub" ] && echo "$work/$dest/$sub" || echo "$work/$dest"
}

# The workflow's re-classification: 0 = every .loft still compiles, 1 = a real break.
still_compiles() {                # still_compiles <pkg-dir> <log>
  local p="$1" log="$2" rc=0 n=0 f
  # `-type f` is load-bearing.  `loft --dump` WRITES a `tests/.loft` cache directory
  # beside the file it compiles, and the glob `*.loft` matches that NAME.  `find`
  # streams through the process substitution, so the directory this very loop creates
  # can be handed straight back to it; `--dump` then fails on a directory and a
  # runtime failure is reported as a compile break.  Reachable on any package with two
  # or more test files, and the shipped gate had the same line (fixed with it).
  while IFS= read -r f; do
    n=$((n + 1))
    (cd "$p" && "$loft" --dump "$f") >/dev/null 2>>"$log" || rc=1
  done < <(cd "$p" && find tests -type f -name '*.loft' 2>/dev/null)
  [ "$n" -gt 0 ] || return 1      # nothing to reclassify — fail conservatively
  return $rc
}

if [ "$self_test" = 1 ]; then
  # The matrix policy first: which packages this run looks at is decided before any of
  # them is compiled, so a wrong answer there is invisible to every check below — it was
  # a permanently-red summary line rather than a wrong verdict on any one package
  # (loft#1315).  Shared with the workflow, and self-tested where it lives.
  python3 "$root/scripts/revalidate_matrix.py" --self-test || {
    echo "self-test FAILED: the shared matrix policy does not hold"; exit 1; }
  read -r n v repo tag sub < <(head -1 "$matrix")
  p="$(extract "$repo" "$tag" "$sub" "selftest")" || { echo "self-test: cannot extract $n@$tag" >&2; exit 2; }
  victim="$(cd "$p" && find tests -type f -name '*.loft' | head -1)"
  [ -n "$victim" ] || { echo "self-test: $n has no test .loft" >&2; exit 2; }
  cp "$p/$victim" "$work/victim.bak"

  printf '\nfn ~~~broken~~~( {\n' >> "$p/$victim"
  if (cd "$p" && LOFT_TIMEOUT=120 "$loft" --interpret --tests tests) >/dev/null 2>&1; then
    echo "self-test FAILED: a syntax error did not fail the suite"; exit 1; fi
  still_compiles "$p" /dev/null && { echo "self-test FAILED: a compile break read as env/deps"; exit 1; }
  echo "self-test: compile break     -> reported as COMPILE-BREAK      ok"

  cp "$work/victim.bak" "$p/$victim"
  printf '\nfn test_injected_runtime_failure() { assert(1 == 2, "injected"); }\n' >> "$p/$victim"
  if (cd "$p" && LOFT_TIMEOUT=120 "$loft" --interpret --tests tests) >/dev/null 2>&1; then
    echo "self-test FAILED: a false assertion did not fail the suite"; exit 1; fi
  still_compiles "$p" /dev/null || { echo "self-test FAILED: a runtime break read as a compile break"; exit 1; }
  echo "self-test: runtime break     -> reported as runtime/env         ok"
  echo "self-test: the two classes are distinguished, on $n@$tag"
  exit 0
fi

# A package named on the command line that the matrix does not carry means the index does
# not know it — almost always a clone that predates its publish.  Reporting `0 pass, 0
# COMPILE-BREAK` and exiting 0 there says "green" about a package never looked at, which is
# the one answer a gate must never give.
for wnt in ${want[@]+"${want[@]}"}; do
  cut -f1 "$matrix" | grep -qxF "$wnt" || {
    echo "package '$wnt' is not in this registry index — it is not published, or this \
clone predates its publish (see the index line above)" >&2
    exit 2
  }
done

# Pre-flight: say which packages CANNOT be checked, before spending the run finding out.
# Every SKIP is decided by two facts a `git rev-parse` answers in milliseconds — is the
# sibling clone there, and does it carry the release tag — while learning it from the run
# itself costs a full registry pass first.  A prerequisite is worth naming before it bites,
# not in the summary afterwards.
missing_clone="" missing_tag=""
while IFS=$'\t' read -r n v repo tag sub; do
  if [ ${#want[@]} -gt 0 ]; then
    printf '%s\n' "${want[@]}" | grep -qxF "$n" || continue
  fi
  clone="$siblings/$(basename "$repo")"
  if [ ! -d "$clone/.git" ]; then
    missing_clone="$missing_clone$repo\t$n\n"
  elif ! git -C "$clone" rev-parse --verify -q "refs/tags/$tag" >/dev/null 2>&1; then
    missing_tag="$missing_tag$(basename "$repo")\t$n@$tag\n"
  fi
done < "$matrix"
if [ -n "$missing_clone" ] || [ -n "$missing_tag" ]; then
  echo
  echo "PRE-FLIGHT — these will SKIP, and a SKIP is not a pass:"
  if [ -n "$missing_clone" ]; then
    # One line per REPO, listing the packages it covers — `loft-libs-docs` carries two, and
    # repeating the same clone command under each reads as two problems.
    printf "$missing_clone" | sort -u | awk -F'\t' '
      { pkgs[$1] = ($1 in pkgs) ? pkgs[$1] ", " $2 : $2 }
      END { for (r in pkgs) print r "\t" pkgs[r] }' | sort |
    while IFS=$'\t' read -r repo pkgs; do
      printf '  %-34s covers %s\n      git clone git@github.com:%s.git %s\n' \
        "$(basename "$repo") (not cloned)" "$pkgs" "$repo" "$siblings/$(basename "$repo")"
    done
  fi
  if [ -n "$missing_tag" ]; then
    printf "$missing_tag" | sort -u | while IFS=$'\t' read -r clone what; do
      printf '  %-34s needs %s — git -C %s fetch --tags\n' \
        "$clone (tag absent)" "$what" "$siblings/$clone"
    done
  fi
  echo
fi

printf '%-18s %-9s %s\n' PACKAGE VERSION VERDICT
breaks=0 passed=0 skipped=0 envfail=0
while IFS=$'\t' read -r n v repo tag sub; do
  if [ ${#want[@]} -gt 0 ]; then
    printf '%s\n' "${want[@]}" | grep -qxF "$n" || continue
  fi
  if ! p="$(extract "$repo" "$tag" "$sub" "$n")"; then
    printf '%-18s %-9s %s\n' "$n" "$v" "SKIP (tag $tag not in $(basename "$repo"))"
    skipped=$((skipped + 1)); continue
  fi
  if [ ! -d "$p/tests" ]; then
    printf '%-18s %-9s %s\n' "$n" "$v" "SKIP (no tests/)"; skipped=$((skipped + 1)); continue
  fi
  log="$work/$n.log"
  if (cd "$p" && LOFT_TIMEOUT=240 "$loft" --interpret --tests tests) >"$log" 2>&1; then
    # The WARNING reading, off the log the run already produced, so it costs nothing.
    # CI has had this since the dashboard existed; this gate had not, which meant the
    # same sweep answered two different questions depending on where it ran.
    python3 "$here/lib_warning_scan.py" scan --from-log "$log" --root "$p" \
      --name "$n" --ref "$v" --label published --json "$warn/$n-published.json" \
      >/dev/null 2>&1 || true
    printf '%-18s %-9s %s\n' "$n" "$v" "PASS"; passed=$((passed + 1))
  elif still_compiles "$p" "$log"; then
    printf '%-18s %-9s %s\n' "$n" "$v" "runtime-fail, COMPILES (env/native-deps — VERIFY)"
    envfail=$((envfail + 1))
    grep -aiE 'FAIL |assertion|Error:|panic|expected|got ' "$log" | tail -5 | sed 's/^/    /'
  else
    printf '%-18s %-9s %s\n' "$n" "$v" "*** COMPILE-BREAK ***"
    breaks=$((breaks + 1))
    tail -12 "$log" | sed 's/^/    /'
  fi
done < "$matrix"

# The warning half.  Separate from the compile/test verdict above ON PURPOSE: warnings do
# not break a shipped artifact and must never fail it (COMPATIBILITY.md), but a library that
# was clean and now warns will fail its OWN CI on its next PR, for an author who did not
# touch the code.  The ratchet is what makes that visible at the change that caused it —
# see `lib_warning_scan.py` for the eight days of green runs that earned it.
echo
ratchet_rc=0
if [ -n "$(ls -A "$warn" 2>/dev/null)" ]; then
  # PARTIAL means "this run did not read every package", and there are two ways to be
  # partial: something SKIPPED (a clone or a tag missing), or you NAMED a subset.  Missing
  # the second reported the whole rest of the registry as `cleaned` on a three-package run
  # — a re-pin request that would have thrown the baseline away.
  partial=""
  { [ "$skipped" -gt 0 ] || [ ${#want[@]} -gt 0 ]; } && partial="--partial"
  python3 "$here/lib_warning_scan.py" collect "$warn" ${partial:+$partial} || ratchet_rc=$?
fi

echo
echo "$passed pass, $envfail runtime/env, $skipped skipped, $breaks COMPILE-BREAK"
[ "$skipped" -gt 0 ] && echo "a SKIP is not a pass — clone the missing repo beside this one, or fetch its tags"
[ "$breaks" -eq 0 ] || echo "a published library no longer compiles against this loft — the freeze forbids that"
# You NAMED these packages, so a skip is a non-answer about the thing you asked for — the
# same reason a named package missing from the index exits 2.  The whole-registry run keeps
# tolerating skips, because taking the box as it finds it is what that pass is for.
if [ ${#want[@]} -gt 0 ] && [ "$skipped" -gt 0 ]; then
  echo "asked about ${#want[@]} package(s) and could not check $skipped of them" >&2
  exit 2
fi
[ "$ratchet_rc" -eq 0 ] || echo "a library that was warning-clean now warns — see the ratchet above"
exit $(( (breaks > 0) || ratchet_rc != 0 ))
