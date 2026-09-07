#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Drift detection for doc/claude/.  Catches the patterns that
# routinely rot in plan / reference docs:
#
#   1. Broken plan links — markdown links of the form
#      `[...](path/to/plan)` where the resolved path doesn't exist.
#      Plans move between current/future/deferred/finished and
#      links don't always follow.
#   2. Time-projection language (multi-week, 2-3 weeks, etc.).
#   3. Stale "is current" claims about retired features (text_code,
#      Type::Long, .loftc, forwarding_smoke.rs).
#
# Reports findings; does NOT fix.  Exit 0 if clean, 1 if drift
# found that's likely real (broken paths, stale claims).  Time
# projections are warnings only.
#
# Usage:
#   scripts/check_doc_drift.sh                 # all checks, verbose
#   scripts/check_doc_drift.sh -q              # all checks, summary only
#   scripts/check_doc_drift.sh paths           # only path drift
#   scripts/check_doc_drift.sh time            # only time projections
#   scripts/check_doc_drift.sh stale           # only stale claims
#   scripts/check_doc_drift.sh roadmap         # only ROADMAP/disk cross-check
#   scripts/check_doc_drift.sh refs            # only finished/deferred refs
#   scripts/check_doc_drift.sh examples        # only worked-example tag resolution
#   EXAMPLES_REPO_ROOT=../loft-libs-x \
#     scripts/check_doc_drift.sh examples-progress   # rollout REPORT: ready to PR?
#   scripts/check_doc_drift.sh features-progress    # monthly AID: @F catalogue gaps
#   scripts/check_doc_drift.sh libraries-progress   # monthly AID: library-review gaps
#
# Exit code: 0 = clean (or only time-projection warnings), 1 = drift.

set -u

cd "$(dirname "$0")/.."
# This checkout — the gate's OWN tree.  Recorded because the gate is repo-agnostic: it
# runs against a library checkout that, in library CI, CONTAINS this one (`path:
# loft-src`).  Anything scanning "the repo under test" has to be able to exclude it.
LOFT_ROOT="$PWD"

# ---- Cross-repo tier: are the worked-example checks GATING or ADVISORY? ----
#
# `examples` + `examples-index` are the only two checks that span repositories: a
# library's CI checks loft out as `loft-src` and runs THIS script against the library.
# That makes the gate's rules arrive from whatever loft `main` happens to be, and it
# bites in both directions.  Measured 2026-08-21: the `exindex` check landed here
# 2026-08-18 and reddened loft-libs-game's next PR for a file it never touched (its
# last green run was 2026-08-17); and in the other direction, switching a LIBRARY
# CHECKOUT's branch turned loft's own run red with two dangling tags — a failure with
# no bad commit in either repo.
#
# So they gate INSIDE loft, which owns the generator and the feature-doc citations and
# where no cross-repo coupling exists, and ADVISE in a library repo.  That follows the
# repo's own diagnostic rule (CLAUDE.md): a diagnostic gates if and only if ignoring it
# can produce a WRONG RESULT.  A dangling doc citation is a broken link — it cannot.
#
# ⚠ The scanner SELFTESTS stay hard everywhere: a scanner that no longer follows its
# documented rules is loft's bug whichever repo happens to run it.
#
# `EXAMPLES_GATE=hard|advisory` overrides, for testing and for a repo that wants the
# strict behaviour back.
EXAMPLES_FOREIGN=0
if [ -n "${EXAMPLES_REPO_ROOT:-}" ]; then
  _err_p=$(cd "$EXAMPLES_REPO_ROOT" 2>/dev/null && pwd -P) || _err_p=""
  _loft_p=$(cd "$LOFT_ROOT" 2>/dev/null && pwd -P) || _loft_p="$LOFT_ROOT"
  if [ -n "$_err_p" ] && [ "$_err_p" != "$_loft_p" ]; then EXAMPLES_FOREIGN=1; fi
fi
case "${EXAMPLES_GATE:-}" in
  hard)     EXAMPLES_FOREIGN=0 ;;
  advisory) EXAMPLES_FOREIGN=1 ;;
esac

# PREFLIGHT — "would my PR report anything on tags?", answered locally with a real exit
# code.  Advisory CI is the right default and it costs you a pass/fail you can act on
# before pushing, so keep one command that gives it back.
#
# ⚠ It gates the CITATION faults (dangling / duplicate / unregistered — the things CI
# would surface) WITHOUT re-demanding a committed `examples-index.tsv`.  Those are two
# different questions and only the first is a defect: the index is generated in CI now, so
# a preflight that insisted on the file would fail for the exact state this is supposed to
# be.  Hence a flag of its own rather than reusing EXAMPLES_GATE=hard.
EXAMPLES_CITE_GATES=1
[ "${EXAMPLES_FOREIGN:-0}" -eq 1 ] && EXAMPLES_CITE_GATES=0
[ "${EXAMPLES_PREFLIGHT:-0}" -eq 1 ] && EXAMPLES_CITE_GATES=1

QUIET=0
if [ "${1:-}" = "-q" ] || [ "${1:-}" = "--quiet" ]; then
  QUIET=1
  shift
fi
CHECK="${1:-all}"
DRIFT=0
HITS_PATHS=0
HITS_TIME=0
HITS_STALE=0
HITS_ROADMAP=0
HITS_REFS=0
HITS_LIBS=0
HITS_EXAMPLES=0
HITS_EXAMPLES_WARN=0
HITS_EXINDEX=0
HITS_VALIDATOR=0
HITS_VALIDATOR_WARN=0

red()    { [ $QUIET -eq 0 ] && printf '\033[31m%s\033[0m\n' "$*"; }
yellow() { [ $QUIET -eq 0 ] && printf '\033[33m%s\033[0m\n' "$*"; }
green()  { [ $QUIET -eq 0 ] && printf '\033[32m%s\033[0m\n' "$*"; }
# Is `$1` a git checkout?  Asked of git, not the filesystem: a linked worktree's `.git` is a
# FILE, so a `-d "$1/.git"` test reads a worktree as "not a checkout" and every step that
# archives a ref falls back to something else in silence.
is_checkout() { git -C "$1" rev-parse --is-inside-work-tree >/dev/null 2>&1; }
say()    { [ $QUIET -eq 0 ] && echo "$@"; }

# ---- Check 1: broken markdown links to plans ----
check_paths() {
  say "=== Broken plan links ==="
  local hits=0
  # Match markdown links [...](url) where url contains plans/<NN>-<slug>.
  # Resolve the url relative to the containing file.
  #
  # Both loops read from a temp file / here-string rather than a `< <(...)`
  # process substitution: macOS's stock bash 3.2 corrupts its heap and dies
  # (SIGBUS/SIGKILL) on hundreds of nested process substitutions, so a local
  # `make ci` on a Mac never got past this check.  Same output on Linux.
  local links
  links=$(mktemp)
  grep -rn -E '\]\([^)]*(lib_plans|plans)/[^)]*[0-9]+-[a-z0-9-]+' \
       doc/claude/ CLAUDE.md --include='*.md' 2>/dev/null \
     | grep -v 'check_doc_drift.sh' > "$links"
  while IFS= read -r line; do
    file="${line%%:*}"
    rest="${line#*:}"
    lineno="${rest%%:*}"
    text="${rest#*:}"
    # Match markdown-link targets: ](path) — explicit ] before ( so we
    # don't capture surrounding prose.  Multiple links per line OK.
    while read -r target; do
      # Strip trailing fragment (#section) and query.
      clean="${target%%#*}"
      clean="${clean%%\?*}"
      [ -z "$clean" ] && continue
      # Skip external URLs — a cross-repo link like
      # https://github.com/.../plans/future/06-foo is NOT a local path
      # (e.g. dryopea's plans linked from a loft lib_plan).
      case "$clean" in
        http://*|https://*|mailto:*) continue ;;
      esac
      # Skip non-plan targets in the same line.
      case "$clean" in
        *plans/*[0-9]-*) ;;
        *) continue ;;
      esac
      # Resolve relative to the file's directory.  `[ -e ]` resolves an
      # embedded `..` itself, so no `realpath` is needed — which also keeps
      # this portable, as BSD `realpath` rejects `-m --relative-to`.
      # `${file%/*}` (not `dirname`, a subshell) — but for a top-level file
      # with no `/` that leaves the name unchanged, so map that to `.`.
      case "$file" in */*) dir="${file%/*}" ;; *) dir="." ;; esac
      check_path="$dir/$clean"
      if [ ! -e "$check_path" ]; then
        red "  $file:$lineno → $clean (resolved: $check_path)"
        hits=$((hits + 1))
      fi
    done <<< "$(printf '%s\n' "$text" | grep -oE '\]\([^)]*\)' | sed -E 's/^\]\(//; s/\)$//')"
  done < "$links"
  rm -f "$links"
  HITS_PATHS=$hits
  if [ $hits -eq 0 ]; then
    green "  clean"
  else
    red "  $hits broken plan links"
    DRIFT=1
  fi
}

# ---- Check 2: time-projection language ----
check_time() {
  say "=== Time projections ==="
  local hits=0
  local patterns=(
    'weeks? of focused'
    '[0-9]+-[0-9]+ weeks'
    'multi-week'
    'next [0-9]+ months'
    # A time unit must follow, or this fires on "expected to take the identical
    # spelling" — a warning that is wrong is one people learn to skip past.
    'expected to take [^.]*(hour|day|week|month|session)'
    'Estimated cost.*hours'
    'Estimated cost.*sessions'
  )
  for pat in "${patterns[@]}"; do
    while IFS= read -r match; do
      case "$match" in
        *plans/finished/*|*CHANGELOG*|*plans/_LIFECYCLE.md*|*scripts/check_doc_drift.sh*)
          continue
          ;;
      esac
      yellow "  $match"
      hits=$((hits + 1))
    done < <(grep -rn -E "$pat" doc/claude/ CLAUDE.md --include='*.md' 2>/dev/null)
  done
  HITS_TIME=$hits
  if [ $hits -eq 0 ]; then
    green "  clean"
  else
    yellow "  $hits time projections (consider effort letters XS/S/M/MH/H/VH/L)"
    # Time projections are warnings, not errors.
  fi
}

# ---- Check 3: stale claims about retired features ----
check_stale() {
  say "=== Stale 'is current' claims about retired features ==="
  local hits=0
  # Tighter patterns: only Rust-code-block or definition-shape mentions
  # (excludes prose mentions in "removed/retired" context).
  local stale_patterns=(
    'pub.*text_code:.*Vec<u8>'
    'text_code: \*const Vec<u8>'
    'pub Long,?\s*//'
    'src/generation/ops/forwarding_smoke\.rs'
    '\.loftc.*current|byte_code_with_cache'
  )
  for pat in "${stale_patterns[@]}"; do
    while IFS= read -r match; do
      file="${match%%:*}"
      case "$file" in
        */CHANGELOG*|*/plans/finished/*|*/plans/deferred/*|*scripts/check_doc_drift.sh)
          continue
          ;;
      esac
      line_text="${match#*:}"
      line_text="${line_text#*:}"
      if echo "$line_text" | grep -qiE 'removed|retired|no longer|previous|legacy|former|was '; then
        continue
      fi
      red "  $match"
      hits=$((hits + 1))
    done < <(grep -rn -E "$pat" doc/claude/ CLAUDE.md --include='*.md' 2>/dev/null)
  done
  HITS_STALE=$hits
  if [ $hits -eq 0 ]; then
    green "  clean"
  else
    red "  $hits potentially stale claims"
    DRIFT=1
  fi
}

# ---- Check 4: ROADMAP plan-state cross-check ----
# Rules:
#   - Active plans (plans/<NN>, plans/future/<NN>) MAY appear on ROADMAP, but
#     absence is NOT drift — plan tracking lives in the issues (loft-lang/plans),
#     not a hand-maintained ROADMAP table.  Only a dual / wrong-bucket citation
#     of whatever IS on ROADMAP is flagged.
#   - Deferred plans (plans/deferred/<NN>) MUST NOT appear on ROADMAP.
#     Their home is DEFERRED.md (trigger index).
#   - Finished plans (plans/finished/<NN>) MUST NOT appear on ROADMAP as
#     action items.  Closure lives in CHANGELOG + git history.
#     Parenthetical historical mentions are tolerated.
check_roadmap() {
  say "=== ROADMAP plan-state cross-check ==="
  local hits=0
  local roadmap=doc/claude/ROADMAP.md
  if [ ! -f "$roadmap" ]; then
    yellow "  $roadmap missing — skipping"
    return
  fi

  for tracker in plans lib_plans; do
    for bucket in '' future deferred finished; do
      bucket_dir="doc/claude/$tracker${bucket:+/$bucket}"
      [ -d "$bucket_dir" ] || continue
      for plan_dir in "$bucket_dir"/[0-9]*/; do
        [ -d "$plan_dir" ] || continue
        slug=$(basename "$plan_dir")
        if [ -z "$bucket" ]; then
          canonical="$tracker/$slug"
        else
          canonical="$tracker/$bucket/$slug"
        fi

        case "$bucket" in
          finished|deferred)
            # MUST NOT appear on ROADMAP as action item.
            # Tolerate parenthetical / "Shipped 2026-XX" historical mentions.
            offending=$(grep -nE "$tracker/$bucket/$slug" "$roadmap" \
                        | grep -vE 'Shipped [0-9]|\(plans/(finished|deferred)/|^[0-9]+:#' )
            # Also flag short-form citation in active-bucket position
            # (e.g. plans/28-const-store when plan is in deferred/).
            short=$(grep -nE "(^|[^/])$tracker/$slug([^/0-9a-z-]|$)" "$roadmap")
            if [ -n "$offending" ] || [ -n "$short" ]; then
              red "  $canonical → $bucket plan still on ROADMAP:"
              { [ -n "$offending" ] && echo "$offending"; [ -n "$short" ] && echo "$short"; } \
                | sort -u | sed 's/^/    /' | head -4
              hits=$((hits + 1))
            fi
            ;;
          *)
            # Active plan (current or future).  Tracking lives in the issues now
            # (loft-lang/plans), NOT a hand-maintained ROADMAP — so being absent
            # from ROADMAP is FINE, not drift.  Only flag a plan cited at MULTIPLE
            # / wrong buckets (a real inconsistency in whatever IS on ROADMAP).
            if grep -qE "$canonical" "$roadmap"; then
              other_buckets=""
              for other_b in '' future deferred finished; do
                [ "$other_b" = "$bucket" ] && continue
                if [ -z "$other_b" ]; then
                  other_path="$tracker/$slug"
                else
                  other_path="$tracker/$other_b/$slug"
                fi
                [ "$other_path" = "$canonical" ] && continue
                # Match boundary: not followed by alphanum/dash so we don't
                # match plans/14-tuple as a substring of plans/14-tuple-foo.
                if grep -qE "$other_path([^/0-9a-zA-Z-]|/|$)" "$roadmap"; then
                  other_buckets="$other_buckets $other_path"
                fi
              done
              if [ -n "$other_buckets" ]; then
                red "  $canonical → cited at canonical AND wrong path(s):$other_buckets"
                hits=$((hits + 1))
              fi
            fi
            ;;
        esac
      done
    done
  done

  HITS_ROADMAP=$hits
  if [ $hits -eq 0 ]; then
    green "  clean"
  else
    red "  $hits ROADMAP/disk drift items"
    DRIFT=1
  fi
}

# ---- Check 5: closed/deferred plan references from normal docs ----
# Closure rule: when a plan ships, reference content moves OUT to
# doc/claude/<NAME>.md.  Other docs link to the reference home, NOT
# the closed plan.  Same for deferred plans (their content is
# either reference-extracted or part of design-content kept in the
# plan README; non-plan docs shouldn't deep-link into deferred plan
# files for ongoing reference).
#
# Allowed: links FROM other plan READMEs (cross-arc references) and
# FROM the closed/deferred plan's siblings.  Flagged: links from
# normal reference docs (doc/claude/<NAME>.md, CLAUDE.md, lib/<name>/*.md).
check_refs() {
  say "=== finished/deferred plan refs from normal docs ==="
  local hits=0
  # Find every markdown link target containing plans/finished/ or plans/deferred/.
  # Tolerance pattern (closure narrative + status annotations + design-pointer phrases).
  local tol_pat='closed by|closure record|shipped (via|by)|historical|retrospective|moved to .*finished|moved to .*deferred|\(closed [0-9]|\(active\)|\(shipped|\(deferred|preserved at|originally documented|design lives|design content|Phase A \+ D|fixture catalogue|spec captured in|catalogue from|deferred follow-ups|trigger conditions'
  # Temp file / here-string, not `< <(...)`: bash 3.2 (macOS) crashes on
  # heavy process substitution — see check_paths.
  local refs
  refs=$(mktemp)
  grep -rn -E '\]\([^)]*plans/(finished|deferred)/' \
       doc/claude/ CLAUDE.md lib/ --include='*.md' 2>/dev/null \
     | grep -v 'check_doc_drift.sh' > "$refs"
  while IFS= read -r line; do
    file="${line%%:*}"
    rest="${line#*:}"
    lineno="${rest%%:*}"
    text="${rest#*:}"
    # Skip:
    # - Links FROM plan READMEs themselves (cross-arc references are fine).
    # - Links FROM presentations/ (not real documentation; archaeology).
    case "$file" in
      */plans/*|*/lib_plans/*|*/presentations/*) continue ;;
    esac
    # Build context window: 3 lines back + current + 1 forward.  Markdown
    # paragraphs wrap heavily so a closure-narrative phrase can sit several
    # lines above the link.
    context=$(awk -v n="$lineno" 'NR>=n-3 && NR<=n+1' "$file" 2>/dev/null)
    while read -r target; do
      clean="${target%%#*}"
      clean="${clean%%\?*}"
      [ -z "$clean" ] && continue
      case "$clean" in
        *plans/finished/*|*plans/deferred/*) ;;
        *) continue ;;
      esac
      # Tolerate explicit closure-narrative + status-annotation patterns.
      if echo "$context" | grep -qiE "$tol_pat"; then
        continue
      fi
      # Deep-link to a phase file (not just the README) is always drift —
      # reference content should have moved out per the closure rule.
      case "$clean" in
        *plans/finished/*/*.md|*plans/deferred/*/*.md)
          case "$clean" in
            */README.md|*/README.md\#*) continue ;;
          esac
          red "  $file:$lineno → $clean (deep-link to phase file)"
          hits=$((hits + 1))
          continue
          ;;
      esac
      red "  $file:$lineno → $clean"
      hits=$((hits + 1))
    done <<< "$(printf '%s\n' "$text" | grep -oE '\]\([^)]*\)' | sed -E 's/^\]\(//; s/\)$//')"
  done < "$refs"
  rm -f "$refs"

  HITS_REFS=$hits
  if [ $hits -eq 0 ]; then
    green "  clean"
  else
    red "  $hits suspect refs (normal doc → finished/deferred plan)"
    red "  Expected: link to the reference home (doc/claude/<NAME>.md) or to the canonical lib doc."
    red "  Tolerated: explicit closure-narrative ('closed by', 'shipped via', 'historical', etc.)"
    DRIFT=1
  fi
}

# ---- Check 7: lib/<name>/ hygiene (warn-only) ----
# Every lib/<name>/ should have:
#   - loft.toml (package manifest — essential for the package format)
#   - src/<name>.loft (the conventional entry file)
#   - a description comment in the entry file beyond the standard
#     Copyright + SPDX-License-Identifier headers (the "what is this
#     library" docstring that gives a reader 5-second orientation)
#
# Notably we do NOT require README.md.  README is downstream-consumer
# documentation — useful when `loft install <pkg>` (PKG.REG) starts
# surfacing it to users who can't see source, but premature today
# while every consumer reads the source tree directly.  The header
# docstring already gives a reader the "what is this" answer; adding
# a one-line README that points at src/ is busywork without
# additional information.  Re-add the README check when PKG.REG ships.
check_libs() {
  say "=== lib/ hygiene ==="
  local hits=0
  [ -d lib ] || { say "  no lib/ — skipping"; return; }
  for d in lib/*/; do
    [ -d "$d" ] || continue
    name=$(basename "$d")
    missing=""
    [ -f "$d/loft.toml" ] || missing="$missing loft.toml"
    entry="$d/src/$name.loft"
    if [ ! -f "$entry" ]; then
      missing="$missing src/$name.loft"
    elif ! has_header_docstring "$entry"; then
      missing="$missing src/$name.loft:header-docstring"
    fi
    if [ -n "$missing" ]; then
      yellow "  $name: missing$missing"
      hits=$((hits + 1))
    fi
  done
  HITS_LIBS=$hits
  if [ $hits -eq 0 ]; then
    green "  clean"
  else
    yellow "  $hits libraries with missing hygiene files"
    # Warning-only: doesn't set DRIFT.
  fi
}

# Returns 0 (true) when the file has at least one `//` comment line
# beyond the standard Copyright + SPDX-License-Identifier headers,
# scanning the first 10 non-blank lines.  This is the "what does
# this library do" description that orients a reader in 5 seconds.
#
# Examples that pass:
#   // Graphics library — 2D pixel canvas with drawing primitives.
#   // --- HTTP Server + WebSocket ---
#   // crypto — hashing, HMAC, base64, JWT.
#
# Examples that fail (Copyright/SPDX only):
#   // Copyright (c) 2026 Jurjen Stellingwerff
#   // SPDX-License-Identifier: LGPL-3.0-or-later
#   (blank, then code)
has_header_docstring() {
  local file="$1"
  local count
  # Take first 10 non-blank lines, keep `//` comment lines that are
  # NOT Copyright + NOT SPDX-License-Identifier.  At least one such
  # line means a real description is present.
  count=$(awk '
    /^[[:space:]]*$/ { next }
    seen >= 10       { exit }
    { seen++ }
    /^\/\// && !/Copyright/ && !/SPDX-License-Identifier/ { print; matched++ }
    END { exit (matched > 0 ? 0 : 1) }
  ' "$file" 2>/dev/null | wc -l)
  [ "$count" -gt 0 ]
}

# Emit `tag<TAB>relpath:line<TAB>fn` for every `// @AAA-### … <newline> fn` in a
# tree.  A tag binds to the `fn` that FOLLOWS it in the same comment block (a blank
# line breaks the block); an `Example:` line is a citation, never a definition.  The
# `fn` may be ANY function — a `test_*`, or a real function in a first-class
# application's own source — so a worked example can be a live use, not only a test.
#
# The FIRST tag in a block defines it.  An example's prose routinely names a sibling
# example ("the failure path, see @ARG-004"), and letting a later mention win read
# that block as defining the sibling — which surfaced as a `dangling` on the block's
# own tag AND a `duplicate` on the one it mentioned, both pointing away from the
# actual mistake.
#
# An `Example:` line makes its whole block a CITATION: it cancels any pending tag
# (dryopea's `crossref` fixture pins that half — a file may name a tag in prose and
# then cite it without claiming to define it), and nothing AFTER it in the block may
# define either.  The second half is what was missing.  A citation is prose, and its
# continuation lines routinely name a second tag — "@GFX-005: the choice is read off
# the pixels, so one alpha-0 pixel (@GFX-001) makes the file RGBA" — so the line
# under the citation was read as defining the tag it merely mentioned, reported as a
# `duplicate` against the real definition somewhere else entirely.  Same misdirection
# the first-tag rule removed on the definition side, arriving from the citation side.
examples_defs_in_tree() {
  local root="$1"
  [ -d "$root" ] || return 0
  # The gate's own checkout is never part of the repo under test.  Library CI checks loft
  # out INSIDE the workspace (`path: loft-src`) and points EXAMPLES_REPO_ROOT at that same
  # workspace, so an unfiltered walk indexes loft's OWN @STD/@GIT/@LEX/@ACR/@EHK tags as
  # the library's — and every library's `examples-index.tsv` then reads `stale` forever,
  # naming rows (`loft-src/tests/scripts/945-…`) no library could ever commit.  Skipping by
  # PATH rather than by the name `loft-src` keeps it true wherever the checkout is nested.
  local abs_root; abs_root=$(cd "$root" 2>/dev/null && pwd) || return 0
  local skip=""
  if [ "${LOFT_ROOT:-}" != "$abs_root" ]; then
    case "${LOFT_ROOT:-}" in "$abs_root"/*) skip="./${LOFT_ROOT#"$abs_root"/}" ;; esac
  fi
  ( cd "$root" 2>/dev/null || exit 0
    # `-type f`: `-name '*.loft'` also matches a DIRECTORY named `.loft` — the
    # build cache `loft test` leaves beside a package — and awk exits fatally on
    # "Is a directory", losing every later file in that xargs batch behind the
    # 2>/dev/null.  The index then silently drops rows the resolver still sees.
    if [ -n "$skip" ]; then
      find . -type f -name '*.loft' -not -path './.*' -not -path './target/*' \
        -not -path "$skip/*" -print0 2>/dev/null
    else
      find . -type f -name '*.loft' -not -path './.*' -not -path './target/*' -print0 2>/dev/null
    fi \
    | xargs -0 awk '
        FNR==1 { f=FILENAME; sub(/^\.\//,"",f); p=""; cited=0 }
        /^[[:space:]]*\/\/.*Example:/ { p=""; cited=1; next }
        /^[[:space:]]*\/\// && match($0, /@[A-Z][A-Z][A-Z]-[0-9][0-9][0-9]/) {
          if (p=="" && cited==0) { p=substr($0,RSTART,RLENGTH); pl=FNR } next }
        /^[[:space:]]*\/\// { next }
        /^[[:space:]]*$/ { p=""; cited=0; next }
        /^[[:space:]]*(pub )?fn / {
          if (p!="") { n=$0; sub(/^[[:space:]]*(pub )?fn /,"",n); sub(/\(.*$/,"",n);
                       printf "%s\t%s:%d\t%s\n", p, f, pl, n }
          p=""; cited=0; next }
        { p=""; cited=0 }
      ' 2>/dev/null )
}

# ---- Self-test: the def scanner's four rules, pinned (@PLN141) ----
# `examples_defs_in_tree` decides what a tag MEANS, and its rules are subtle enough
# that two of them have already been got wrong in a way whose fault report points
# somewhere else entirely (a `duplicate` naming the innocent definition, a `dangling`
# naming the block that was right).  The real trees cannot pin them: they are the
# tree the rules were tuned against, so a scanner change that breaks a rule nobody
# happens to exercise stays green.  These fixtures exercise each rule on purpose.
#
# Built in a temp dir rather than committed under the repo: the scanner walks every
# `*.loft` in a tree, so committed fixtures would inject their fake `@TST` tags into
# loft's own tag index and `examples-index.tsv`.
check_examples_selftest() {
  say "=== Worked-example def scanner: the rules still hold ==="
  local d; d=$(mktemp -d) || { red "  selftest: cannot create a temp dir"; DRIFT=1; return; }
  mkdir -p "$d/src"

  # 1. A tag above a fn defines it.
  printf '// @TST-001 — the plain case.\nfn plain_def() { }\n' > "$d/src/a.loft"
  # 2. The FIRST tag in a block wins; a sibling named later is only a mention.
  #    The mention must be on a LATER LINE — `match()` only ever sees the first tag
  #    on a line, so a same-line sibling passes whatever the rule is, and pins nothing.
  printf '// @TST-002 — the real one.\n// See @TST-003 for the failure path.\nfn first_wins() { }\n' \
    > "$d/src/b.loft"
  # 3. A blank line breaks the block, so nothing is defined.
  printf '// @TST-004 — orphaned by the blank line below.\n\nfn not_defined() { }\n' > "$d/src/c.loft"
  # 4. An `Example:` line makes the block a CITATION: it cancels a tag named in
  #    earlier prose, and a tag named in its own continuation defines nothing.
  printf '// @TST-005 lives elsewhere; this file only cites it.\n//\n// Example: @TST-005\nfn cites_only() { }\n' \
    > "$d/src/e.loft"
  printf '// Example: @TST-006 — the choice is read off the pixels, so one\n// alpha-0 pixel (@TST-007) makes the whole file RGBA.\nfn cite_continuation() { }\n' \
    > "$d/src/f.loft"
  # 5. The gate's OWN checkout, when it sits INSIDE the scanned tree, defines nothing
  #    for that tree.  This is the library-CI shape (`path: loft-src` under the same
  #    workspace EXAMPLES_REPO_ROOT points at), and getting it wrong is invisible in
  #    this repo — loft scanning itself has nothing to exclude — while making EVERY
  #    library's examples-index.tsv permanently `stale` against rows naming loft's own
  #    tests.  Pinned as an ABSENCE, so a scanner that stops filtering fails here.
  mkdir -p "$d/loft-src/tests"
  printf '// @TST-009 — the gate own tree; never the library under test.\nfn gate_own_tree() { }\n' \
    > "$d/loft-src/tests/x.loft"

  local want got
  want=$(printf '@TST-001\tsrc/a.loft:1\tplain_def\n@TST-002\tsrc/b.loft:1\tfirst_wins\n')
  got=$(LOFT_ROOT="$d/loft-src" examples_defs_in_tree "$d" | sort)
  rm -rf "$d"
  if [ "$got" = "$want" ]; then
    green "  ok — 6 fixtures, 5 rules (defines / first-tag-wins / blank-breaks / citation-block / own-checkout)"
  else
    red "  selftest FAILED — the scanner no longer follows its documented rules:"
    diff <(printf '%s\n' "$want") <(printf '%s\n' "$got") | sed 's/^/      /'
    HITS_EXAMPLES=$((HITS_EXAMPLES + 1)); DRIFT=1
  fi
}

# ---- Self-test: the CITATION scanner's rules, pinned (@PLN141 C2) ----
# The def scanner above decides what a tag MEANS; this decides what COUNTS as citing
# one, and C2 gave it a second source in a different language (markdown, from a feature
# issue body).  Two of the four rules below are ABSENCES — a scanner that gets keener
# reads the documentation OF the convention as a use of it, which is silent drift in
# the direction that always looks like more coverage.
# @PLN141 Phase C follow-up — the online fallback stays a fallback.
#
# HERMETIC ON PURPOSE: it points the fetch at a host that cannot exist, so it asserts the
# one property a network test could never assert reliably — that a run which CANNOT fetch
# still warns instead of failing.  A gate that goes red when a DNS lookup fails stops
# being run, and that failure mode only appears on the machine that has no network, which
# is never the machine the test was written on.
check_examples_online_selftest() {
  say "=== Worked-example online fallback: a failed fetch is not a finding ==="
  local d out rc; d=$(mktemp -d) || { red "  online selftest: cannot create a temp dir"; DRIFT=1; return; }
  mkdir -p "$d/src"
  # A citation to a REGISTERED acronym whose repo has no local checkout: the only path
  # that reaches the online fallback at all.
  printf '// Example: @RGX-901\nfn cites_a_foreign_tag() { }\n' > "$d/src/a.loft"
  # `.invalid` is reserved by RFC 2606 and can never resolve, so the fetch fails on every
  # machine and no packet reaches anyone's server.  A resolver that answers anyway (a
  # captive portal) still leaves the test sound: whatever it returns is not a tag index,
  # and the shape guard rejects it.
  printf 'RGX\tno-such-repo-for-a-selftest\thttps://example.invalid/nope\tmain\n' > "$d/reg.tsv"
  out=$( EXAMPLES_ONLINE=1 EXAMPLES_CITE_GATES=1 EXAMPLES_REGISTRY="$d/reg.tsv" \
         EXAMPLES_REPO_ROOT="$d" EXAMPLES_CITE_ROOTS=src bash "$0" examples 2>&1 ); rc=$?
  rm -rf "$d"
  if printf '%s' "$out" | grep -q "unvalidated: @RGX-901" && \
     ! printf '%s' "$out" | grep -q "dangling: @RGX-901" && [ $rc -eq 0 ]; then
    green "  ok — an unreachable index warns, and the gate stays green"
  else
    red "  online selftest FAILED — an unreachable index must warn, never fail the gate"
    printf '%s\n' "$out" | sed 's/^/      /' | head -6
    red "      exit was $rc (want 0)"
    DRIFT=1
  fi
}

check_examples_cite_selftest() {
  say "=== Worked-example citation scanner: both sources, and what each ignores ==="
  local d; d=$(mktemp -d) || { red "  cite selftest: cannot create a temp dir"; DRIFT=1; return; }
  mkdir -p "$d/src" "$d/features"

  # 1. The CODE source: a `// Example:` line in a .loft cites.
  printf '// Example: @TST-101\nfn cites_from_code() { }\n' > "$d/src/a.loft"
  # 2. The CATALOGUE source: a markdown `Example:` line in a feature doc cites, with or
  #    without the usual emphasis/list markers around it.
  printf '# @F1 — a feature\n\n## Example\n\nExample: @TST-102\n' > "$d/features/F1.md"
  printf '# @F2\n\n- **Example:** @TST-103\n' > "$d/features/F2.md"
  # 3. ABSENCE — `## Example` is a HEADING, not a citation, and 81 feature docs carry
  #    one.  If the colon stopped being required every one of them would start citing
  #    whatever tag happened to appear under it.
  printf '# @F3\n\n## Example\n\n@TST-104 is only mentioned here.\n' > "$d/features/F3.md"
  # 4. ABSENCE — the code form shown INSIDE a feature doc is documentation of the
  #    convention, not a use of it.  A feature explaining worked examples will contain
  #    exactly this line, and it must stay inert.
  printf '# @F4\n\nWrite it like this:\n\n```loft\n// Example: @TST-105\nfn demo() { }\n```\n' \
    > "$d/features/F4.md"

  local want got
  want=$(printf '@TST-101\n@TST-102\n@TST-103\n')
  got=$(examples_cited_in_tree "$d" "src" "features")
  # 5. INERT — with no feature-docs directory, the code source stands alone.
  local inert; inert=$(examples_cited_in_tree "$d" "src" "no_such_dir")
  rm -rf "$d"
  if [ "$got" = "$want" ] && [ "$inert" = "@TST-101" ]; then
    green "  ok — 5 rules (code / markdown / heading-is-not / fenced-form-is-not / inert)"
  else
    red "  cite selftest FAILED — the citation scanner no longer follows its rules:"
    diff <(printf '%s\n' "$want") <(printf '%s\n' "$got") | sed 's/^/      /'
    [ "$inert" = "@TST-101" ] || red "      with no feature docs it returned: $inert"
    HITS_EXAMPLES=$((HITS_EXAMPLES + 1)); DRIFT=1
  fi
}

# Every worked-example tag a repo CITES, one per line, sorted.  Two sources, and they
# are deliberately different in SHAPE because they live in different kinds of text:
#
#   code       `// Example: @AAA-###` in a `.loft` under $2 — a function documenting
#              its own correct use, beside the code it is about.
#   catalogue  `Example: @AAA-###` in a feature doc under $3 — MARKDOWN, because the
#              canonical text is a loft-lang/features ISSUE BODY and `doc/features/`
#              is its generated shadow (@PLN141 C2).  A feature's ```loft fence is the
#              minimal compiles-and-runs snippet; this points at a REAL use, and the
#              two are complementary rather than alternatives.
#
# `//` is deliberately NOT a markdown line-marker here.  A feature doc explaining the
# convention shows the code form inside a fence, and a scanner that accepted it would
# read the documentation OF the mechanism as a use of it — the same class of mistake
# the def scanner's citation rule exists for.
#
# The catalogue source is inert where its directory is absent, which is every library:
# the same opt-in ratchet as the rest of the mechanism.
examples_cited_in_tree() {
  local root="$1" cite_roots="$2" feature_docs="$3"
  local tag_re='@[A-Z][A-Z][A-Z]-[0-9][0-9][0-9]'
  {
    ( cd "$root" 2>/dev/null && grep -rhE "//[[:space:]]*Example:" $cite_roots --include='*.loft' 2>/dev/null )
    if [ -n "$feature_docs" ] && [ -d "$root/$feature_docs" ]; then
      ( cd "$root" 2>/dev/null && grep -rhE '^[[:space:]]*([*_>-]+[[:space:]]*)*Example:' \
          "$feature_docs" --include='*.md' 2>/dev/null )
    fi
  } | grep -oE "$tag_re" | sort -u
}

# ---- Check: worked-example tags resolve to a real test/function (@PLN141) ----
# A stdlib / library function documents its correct use by CITING a demonstrator:
# `// Example: @AAA-###`, where the tag names a `fn` carrying `// @AAA-###` above it
# — a test, OR a real function in a first-class application.  The acronym names the
# repo that owns the tag (scripts/example_repos.tsv), so a citation can point ACROSS
# repos and still be validated + linked:
#
#   ok           the tag resolves to a fn; a cross-repo hit prints its git link.
#   dangling     cited, but no fn in the owning repo carries it → drift.
#   duplicate    one tag on two fns in this repo → ambiguous → drift.
#   unregistered the acronym isn't in the registry → drift (add its repo + url).
#   unvalidated  a cross-repo tag this run could not check at all → WARNING only
#                (the link is still emitted).  A local checkout is validated OFFLINE and
#                stays preferred: it can see unmerged refs, which is what tells a PENDING
#                MERGE from a real dangling citation, and a published index cannot.
#
# With no checkout, `EXAMPLES_ONLINE=1` reads the owning repo's own published
# `examples-index.tsv` and validates against that — the case a missing sibling used to
# leave entirely unchecked.  Opt-in, because the default gate is hermetic and stays that
# way.  A fetch that FAILS is not a finding (no network, no `curl`, a repo that has not
# adopted the convention).  Neither is ABSENCE from a fetched index: the only fetchable
# one is a committed `examples-index.tsv`, and LIBRARY_AUTHORING.md is retiring that file
# because a copy committed beside the source cannot be regenerated where it sits.  So the
# index may CONFIRM a tag and never REFUTE one — this path can turn `unvalidated` into
# `ok`, and can never turn it red.  Guarded by `check_examples_online_selftest`.
#
# The `@AAA-###` shape (three letters, hyphen, three digits) is distinct from the
# repo's other tag families (@F1, @P259, @PLN141) — none has that hyphen.
check_examples() {
  say "=== Worked-example tags resolve to a test/function ==="
  local tag_re='@[A-Z][A-Z][A-Z]-[0-9][0-9][0-9]'
  # The registry + gate logic are loft-anchored (this script cd'd to the loft root at
  # startup), but the CITATIONS and the local repo's defs come from EXAMPLES_REPO_ROOT:
  # `.` (loft) for loft's own run, the library checkout when library-ci-reusable.yml
  # runs this same gate against a loft-libs-* repo.  Defaults reproduce loft's self-check
  # byte-for-byte; the library CI sets REPO_ROOT + CITE_ROOTS to the package under test.
  local registry="${EXAMPLES_REGISTRY:-scripts/example_repos.tsv}"
  local repo_root="${EXAMPLES_REPO_ROOT:-.}"
  # ⚠ The default `default lib` is LOFT's own layout.  In a library repo those directories
  # do not exist, so an unset CITE_ROOTS scans nothing and the check passes VACUOUSLY —
  # which is the worst outcome for a local preflight, because it looks like a pass.  So a
  # foreign repo defaults to its package dirs (`*/src`, `*/tests`), falling back to the
  # whole tree.  CI still sets CITE_ROOTS explicitly to the package under test.
  local cite_roots="${EXAMPLES_CITE_ROOTS:-}"
  if [ -z "$cite_roots" ]; then
    if [ "${EXAMPLES_FOREIGN:-0}" -eq 1 ]; then
      cite_roots=$(cd "$repo_root" 2>/dev/null && \
        for d in */src */tests src tests; do [ -d "$d" ] && printf '%s ' "$d"; done)
      [ -n "$cite_roots" ] || cite_roots="."
    else
      cite_roots="default lib"
    fi
  fi
  # The loft repo hosting this gate is always available in place at `.` (the script cd'd
  # to the loft root at startup) — even in a library CI, where loft is checked out as
  # loft-src rather than a sibling ../loft.  So loft's OWN acronyms (STD/GIT/LEX/…)
  # resolve there for any repo-under-test.  Its registry name is `loft` by convention.
  local host_repo="${EXAMPLES_HOST_REPO:-loft}"
  local self_name; self_name=$(basename "$(cd "$repo_root" 2>/dev/null && pwd)")
  # Feature-catalogue citations (@PLN141 C2): present only in loft, inert elsewhere.
  local feature_docs="${EXAMPLES_FEATURE_DOCS:-doc/features}"
  local cited cache
  cited=$(mktemp); cache=$(mktemp -d)
  # Citations under the repo-under-test (loft: default/ + lib/ + doc/features/).
  examples_cited_in_tree "$repo_root" "$cite_roots" "$feature_docs" > "$cited"
  # Cache a repo's defs by (checkout path, branch), so each is scanned at most once.
  #
  # Resolve against the BRANCH the registry names, not the working tree.  A sibling
  # checkout sits on whatever branch its own agent is working on, so indexing the tree made
  # this gate's verdict depend on that: a tag present only on a feature branch read as
  # RESOLVED while the `blob/<branch>` link printed beside it 404s, and the same tag read as
  # dangling once that checkout moved.  A false GREEN is the worse half.
  #
  # `git archive` is read-only — a sibling repo is never ours to check out, stash or
  # worktree — and the export reuses `examples_defs_in_tree` unchanged, so the def rules
  # (defines / first-tag-wins / blank-breaks / citation-block) have exactly one home.
  # Falls back to the working tree, loudly, when the ref is absent.
  _defs() {
    local root="$1" br="${2:-}" key cf ref exp
    key=$(printf '%s@%s' "$root" "$br" | tr '/.@' '___'); cf="$cache/$key"
    if [ ! -f "$cf" ]; then
      ref=""
      if [ -n "$br" ] && is_checkout "$root"; then
        for cand in "origin/$br" "$br"; do
          if git -C "$root" rev-parse --verify -q "$cand^{commit}" >/dev/null 2>&1; then
            ref="$cand"; break
          fi
        done
      fi
      if [ -n "$ref" ]; then
        exp="$cache/x_$key"; mkdir -p "$exp"
        if git -C "$root" archive "$ref" 2>/dev/null | tar -x -C "$exp" 2>/dev/null; then
          examples_defs_in_tree "$exp" > "$cf" 2>/dev/null
        else
          examples_defs_in_tree "$root" > "$cf" 2>/dev/null
        fi
      else
        # To stderr: stdout of this function IS the cache path its caller reads.
        [ -n "$br" ] && yellow "  note: $root has no ref '$br' — indexing its working tree instead" >&2
        examples_defs_in_tree "$root" > "$cf" 2>/dev/null
      fi
    fi
    printf '%s' "$cf"
  }
  # The owning repo's PUBLISHED tag index, fetched once per (repo, branch) — @PLN141
  # Phase C follow-up.  Prints the cache path when the index is in hand, nothing when it
  # is not.
  #
  # Only reached when there is NO local checkout: a checkout is validated OFFLINE and
  # stays preferred, because it can see unmerged refs (the `pending` answer) and a
  # published index cannot.  This is the case a missing sibling used to leave entirely
  # unchecked.
  #
  # A FETCH FAILURE IS NOT A FINDING.  No network, no `curl`, a 404 on a repo that has
  # not adopted the convention — none of those say anything about the citation, so each
  # falls back to the same `unvalidated` warning as before.  Only a REACHABLE index that
  # does not carry the tag is evidence, and that is what becomes a `dangling`.  A doc
  # gate that goes red when a DNS lookup fails stops being run.
  _online_defs() {
    local repo="$1" br="$2" url="$3" key cf
    [ "${EXAMPLES_ONLINE:-0}" -eq 1 ] || return 1
    command -v curl >/dev/null 2>&1 || return 1
    key=$(printf 'online_%s@%s' "$repo" "$br" | tr '/.@:' '____'); cf="$cache/$key"
    if [ ! -f "$cf" ]; then
      # `examples-index.tsv` is the repo's own generated index — the same file
      # `write_examples_index` produces here — so this reads what that repo published
      # rather than re-deriving it from source we would have to clone.
      local raw="${url%/}"; raw="${raw%.git}"
      raw="${raw/#https:\/\/github.com\//https://raw.githubusercontent.com/}/$br/examples-index.tsv"
      curl -fsSL --max-time 10 "$raw" -o "$cf.try" 2>/dev/null || { : > "$cf.fail"; return 1; }
      # A repo that serves an error page rather than a 404 must not read as an index.
      if ! grep -qE "^@[A-Z]{3}-[0-9]{3}\s" "$cf.try" 2>/dev/null; then
        rm -f "$cf.try"; : > "$cf.fail"; return 1
      fi
      mv "$cf.try" "$cf"
    fi
    [ -f "$cf" ] || return 1
    printf '%s' "$cf"
  }
  local n_cited; n_cited=$(grep -cvE '^$' "$cited")
  local hits=0 t acr row repo url branch lpath def link
  while IFS= read -r t; do
    [ -z "$t" ] && continue
    acr=${t#@}; acr=${acr%%-*}
    row=$(awk -F'\t' -v a="$acr" '$1!~/^#/ && $1==a {print; exit}' "$registry" 2>/dev/null)
    if [ -z "$row" ]; then
      red "  unregistered: $t — acronym '$acr' not in $registry (add its repo + git url)"
      hits=$((hits + 1)); continue
    fi
    repo=$(printf '%s' "$row" | cut -f2)
    url=$(printf '%s'  "$row" | cut -f3)
    branch=$(printf '%s' "$row" | cut -f4)
    # Resolve the owning repo to a checkout: the repo-under-test itself (in place at
    # repo_root), the loft host repo (always in place at `.`, even as loft-src in a
    # library CI), or a foreign sibling checkout (../<repo>).
    lpath="../$repo"
    if [ "$self_name" = "$repo" ]; then lpath="$repo_root"
    elif [ "$repo" = "$host_repo" ]; then lpath="."
    fi
    if [ ! -d "$lpath" ]; then
      # No checkout: try the repo's PUBLISHED index before giving up on the tag.
      local oidx odef
      if oidx=$(_online_defs "$repo" "$branch" "$url"); then
        odef=$(awk -F'\t' -v tg="$t" '$1==tg{print $2; exit}' "$oidx")
        if [ -n "$odef" ]; then
          link="$url/blob/$branch/$(printf '%s' "$odef" | sed 's/:\([0-9][0-9]*\)$/#L\1/')"
          say "  ok  $t -> $link  (validated online against $repo@$branch)"
          continue
        fi
        # ABSENCE IN THE INDEX IS NOT EVIDENCE, and this is the whole design.  The only
        # fetchable index is a `examples-index.tsv` a library COMMITTED, and
        # LIBRARY_AUTHORING.md is retiring exactly that file — CI builds it now, and a
        # leftover committed copy "can only rot: you cannot regenerate it where it sits".
        # A repo that stops regenerating it would start reporting tags it really carries
        # as missing, and loft's gate would go red for something no loft file says — the
        # failure this check's own header warns about.  So the index may CONFIRM a tag
        # and never REFUTE one.
        yellow "  unvalidated: $t — $repo@$branch's published index does not list it; it may be stale (CI builds that file now). Clone ../$repo to settle it. link: $url"
        HITS_EXAMPLES_WARN=$((HITS_EXAMPLES_WARN + 1)); continue
      fi
      if [ "${EXAMPLES_ONLINE:-0}" -eq 1 ]; then
        yellow "  unvalidated: $t — no checkout ../$repo and $repo@$branch published no readable index. link: $url"
      else
        yellow "  unvalidated: $t — no sibling checkout ../$repo; clone it, or set EXAMPLES_ONLINE=1 to check its published index. link: $url"
      fi
      HITS_EXAMPLES_WARN=$((HITS_EXAMPLES_WARN + 1)); continue
    fi
    def=$(awk -F'\t' -v tg="$t" '$1==tg{print $2; exit}' "$(_defs "$lpath" "$branch")")
    if [ -z "$def" ]; then
      # Absent from the branch the registry names is not one condition but two, and they
      # want different answers.  A tag carried on some OTHER ref of the owning repo is a
      # PENDING MERGE — a cross-repo timing fact, nothing here can fix it, and failing the
      # gate on it blocks this repo on another repo's merge schedule.  A tag carried
      # nowhere is a genuine dangling citation and stays an error.
      pending=""
      if is_checkout "$lpath"; then
        pending=$(git -C "$lpath" grep -l -- "$t" \
                    $(git -C "$lpath" for-each-ref --format='%(refname)' \
                        refs/heads refs/remotes 2>/dev/null) 2>/dev/null \
                  | head -1 | cut -d: -f1)
      fi
      if [ -n "$pending" ]; then
        yellow "  pending: $t is on '$pending' in $repo, not yet on '$branch' — merge it there"
        HITS_EXAMPLES_WARN=$((HITS_EXAMPLES_WARN + 1)); continue
      fi
      red "  dangling: $t is cited but no fn carries it in $repo (on any ref)"
      ( cd "$repo_root" 2>/dev/null && grep -rnE "Example:.*$t" $cite_roots --include='*.loft' 2>/dev/null ) | sed 's/^/      /'
      [ -d "$repo_root/$feature_docs" ] && \
        ( cd "$repo_root" 2>/dev/null && grep -rnE "Example:.*$t" "$feature_docs" --include='*.md' 2>/dev/null ) | sed 's/^/      /'
      hits=$((hits + 1)); continue
    fi
    if [ "$lpath" = "$repo_root" ]; then
      say "  ok  $t -> $def"
    else
      link="$url/blob/$branch/$(printf '%s' "$def" | sed 's/:\([0-9][0-9]*\)$/#L\1/')"
      say "  ok  $t -> $link  (validated against $repo)"
    fi
  done < "$cited"
  # DUPLICATE — one tag on two fns in THIS repo (each foreign repo owns its own).
  local d
  for d in $(cut -f1 "$(_defs "$repo_root")" | sort | uniq -d); do
    red "  duplicate: $d tags more than one fn in this repo"
    hits=$((hits + 1))
  done
  rm -rf "$cited" "$cache"
  HITS_EXAMPLES=$hits
  if [ $hits -gt 0 ]; then
    [ $EXAMPLES_CITE_GATES -eq 1 ] && DRIFT=1
  elif [ "${HITS_EXAMPLES_WARN:-0}" -eq 0 ]; then
    if [ "$n_cited" -eq 0 ]; then
      # A check that examined nothing is not a pass — say which roots were scanned, so a
      # vacuous run reads as vacuous instead of green.
      yellow "  ok — but 0 citations were found (scanned: $cite_roots)"
    else
      green "  ok — $n_cited citation(s) resolve to a test/function"
    fi
  fi
}

# ---- Worked-example index: where each tag lives (@PLN141) ----
# examples-index.tsv lists every `// @AAA-###`-tagged fn in a repo with its file:line and
# git blob link, so a READER can find where a tag resolves without a checkout.
#
# ⚠ Nothing machine-reads it.  This comment used to claim "or loft's cross-repo `idx`";
# measured 2026-08-21, `scripts/idx` does not open the file and never has, and neither does
# `check_examples`, which resolves cross-repo tags through a local CHECKOUT.  So its only
# automated purpose was being checked for freshness — a file that exists so a check can
# verify the file.
#
# WHERE IT LIVES follows the same axis as whether the check gates: does the repo OWN the
# generator?
#   * loft — yes.  `make examples-index`, the pre-commit hook keeps it current, and a
#     committed copy is greppable offline, which the agent development model relies on
#     (BUS_FACTOR.md).  Committed, and verified current (fail-on-diff).
#   * a library repo — no.  The generator is in loft, so the file cannot be regenerated
#     where it lives; it can only rot, and a "regenerate it" message there names a command
#     the maintainer does not have.  So CI GENERATES it per run and publishes it, and
#     nothing is committed.  A derived file that is never committed cannot be stale.
#
# Line numbers churn, so it lives WITH the code it indexes, not in the central acronym
# registry (which stays one stable row per acronym).
EXAMPLES_INDEX_FILE="${EXAMPLES_INDEX_FILE:-examples-index.tsv}"

# Emit the index body: `tag <TAB> file:line <TAB> fn <TAB> blob_url`, one row per def,
# sorted by tag.  The blob link comes from the acronym's registry row (url + branch).
_examples_index_body() {
  local root="$1" reg="$2" tag rest fn acr row url branch link
  examples_defs_in_tree "$root" | sort | while IFS=$'\t' read -r tag rest fn; do
    acr=${tag#@}; acr=${acr%%-*}
    row=$(awk -F'\t' -v a="$acr" '$1!~/^#/ && $1==a {print; exit}' "$reg" 2>/dev/null)
    url=$(printf '%s' "$row" | cut -f3); branch=$(printf '%s' "$row" | cut -f4)
    if [ -n "$url" ]; then
      link="$url/blob/$branch/$(printf '%s' "$rest" | sed 's/:\([0-9][0-9]*\)$/#L\1/')"
    else
      link="-"   # acronym not registered — path still recorded, no link
    fi
    printf '%s\t%s\t%s\t%s\n' "$tag" "$rest" "$fn" "$link"
  done
}

_examples_index_full() {
  local root="$1" reg="$2"
  printf '# Generated by scripts/check_doc_drift.sh (@PLN141) — DO NOT EDIT.\n'
  # The two homes differ, so the header must not advertise a cure the reader cannot run:
  # loft commits this file and regenerates it with `make`; a library repo commits nothing
  # and CI rebuilds it every run.
  if [ "${EXAMPLES_FOREIGN:-0}" -eq 1 ]; then
    printf '# Built by CI from the loft checkout that owns the generator; NOT committed here.\n'
  else
    printf '# Regenerate: make examples-index    Verified in CI: check_doc_drift.sh examples-index\n'
  fi
  printf '# Every worked-example tag defined in this repo: where it lives + its git blob link.\n'
  printf '# tag\tfile:line\tfn\tblob_url\n'
  _examples_index_body "$root" "$reg"
}

# Emit the index to STDOUT — what CI uses in a repo that does not commit one.
emit_examples_index() {
  local root="${EXAMPLES_REPO_ROOT:-.}" reg="${EXAMPLES_REGISTRY:-scripts/example_repos.tsv}"
  _examples_index_full "$root" "$reg"
}

write_examples_index() {
  local root="${EXAMPLES_REPO_ROOT:-.}" reg="${EXAMPLES_REGISTRY:-scripts/example_repos.tsv}"
  _examples_index_full "$root" "$reg" > "$root/$EXAMPLES_INDEX_FILE"
  say "wrote $root/$EXAMPLES_INDEX_FILE ($(grep -cvE '^#' "$root/$EXAMPLES_INDEX_FILE") tag(s))"
}

# The index check, and it asks a DIFFERENT question depending on who owns the generator.
#
#   * loft (native run) — the committed copy must exist and be current (fail-on-diff, the
#     features-check pattern).  We own the generator, so "regenerate it" is a real cure.
#   * a library repo (cross-repo run) — nothing is required and nothing is verified: CI
#     generates the index per run and publishes it.  A derived file that is never committed
#     cannot be stale, which retires the whole failure mode rather than downgrading it.
check_examples_index() {
  local root="${EXAMPLES_REPO_ROOT:-.}" reg="${EXAMPLES_REGISTRY:-scripts/example_repos.tsv}"
  local f="$root/$EXAMPLES_INDEX_FILE" tmp; tmp=$(mktemp)
  _examples_index_full "$root" "$reg" > "$tmp"
  local defines_tags=0; grep -qE '^@' "$tmp" && defines_tags=1
  local n; n=$(grep -cE '^@' "$tmp")

  if [ "${EXAMPLES_FOREIGN:-0}" -eq 1 ]; then
    say "=== Worked-example index (generated here, not committed) ==="
    if [ $defines_tags -eq 0 ]; then
      green "  ok — no worked-example tags defined; no index to build"
    else
      green "  ok — generated $n tag(s); CI publishes it, this repo commits nothing"
      # ⚠ A leftover committed copy is not an error — it is just unmaintainable here, since
      # the generator lives in loft.  Say so once, as a warning, rather than diffing it:
      # reporting it "stale" would be the exact message this change exists to delete.
      if [ -f "$f" ]; then
        yellow "  note: $EXAMPLES_INDEX_FILE is committed but no longer needed — CI generates it"
        yellow "        now, and it cannot be regenerated here.  Safe to delete."
        HITS_EXAMPLES_WARN=$((HITS_EXAMPLES_WARN + 1))
      fi
    fi
    rm -f "$tmp"; return
  fi

  say "=== Worked-example index (examples-index.tsv) is current ==="
  if [ ! -f "$f" ]; then
    if [ $defines_tags -eq 1 ]; then
      red "  missing: $EXAMPLES_INDEX_FILE — run 'make examples-index' (repo defines worked-example tags)"
      HITS_EXINDEX=1; DRIFT=1
    else
      green "  ok — no worked-example tags defined; no index needed"
    fi
    rm -f "$tmp"; return
  fi
  if diff -q "$f" "$tmp" >/dev/null 2>&1; then
    green "  ok — $EXAMPLES_INDEX_FILE lists $(grep -cE '^@' "$f") tag(s), all current"
  else
    red "  stale: $EXAMPLES_INDEX_FILE is out of date — run 'make examples-index'"
    [ $QUIET -eq 0 ] && diff "$f" "$tmp" | sed 's/^/      /' | head -20
    HITS_EXINDEX=1; DRIFT=1
  fi
  rm -f "$tmp"
}

# ---- Registry validator: the template and the deployed copy are ONE file ----
# `doc/claude/registry_ci_template/validate.py` is the file a registry deploys at
# `tools/validate.py`, and its own docstring says so.  The two drifted for eleven
# weeks in BOTH directions with neither a superset (loft#1052): the deployment grew
# a docs gate, a `yanked` type-check and chunk-repo homepages; the template grew a
# trigger-uniqueness gate and an `api` re-derive.  Deploying the template would then
# have REMOVED three live checks, and the producer in `registry_maintain.sh` was
# written against the template's weaker rules — which is how every package published
# after 2026-06-19 went in unmergeable.
#
# So the invariant is byte-identity, not "roughly the same": one file, two homes.
# Validated OFFLINE against a local registry checkout and WARN-only when none is
# present, the same convention `check_examples` uses for a cross-repo tag.
check_validator() {
  say "=== Registry validator template matches the deployed copy ==="
  local tpl="doc/claude/registry_ci_template/validate.py"
  if [ ! -f "$tpl" ]; then
    green "  ok — no validator template in this repo"; return
  fi
  local reg=""
  for cand in "${LOFT_REGISTRY_DIR:-}" ../loft-registry ../registry; do
    [ -n "$cand" ] && [ -f "$cand/tools/validate.py" ] && { reg="$cand"; break; }
  done
  if [ -z "$reg" ]; then
    yellow "  unvalidated: no local loft-lang/registry checkout (set LOFT_REGISTRY_DIR) — cannot compare"
    HITS_VALIDATOR_WARN=1; return
  fi
  if cmp -s "$tpl" "$reg/tools/validate.py"; then
    green "  ok — template is byte-identical to $reg/tools/validate.py"
  else
    red "  DRIFT: $tpl differs from $reg/tools/validate.py (loft#1052)"
    red "         they are ONE file with two homes — deploying a drifted template removes live gates"
    [ $QUIET -eq 0 ] && diff "$tpl" "$reg/tools/validate.py" | sed 's/^/      /' | head -20
    HITS_VALIDATOR=1; DRIFT=1
  fi
}

# ---- Rollout progress: is a library repo ready to PR? (@PLN141) ----
# The worked-example rollout lands ONE branch per library repo, opened as a PR when
# every package in that repo has a VERDICT — either it carries tags, or it is recorded
# in `examples-exempt.tsv` (repo root, hand-written: `package <TAB> exempt|deferred
# <TAB> reason`).  `exempt` = no function here teaches more from a call site than from
# its signature.  `deferred` = one does, but not in this pass; the reason names what
# unblocks it, and the monthly by-hand review (LIBRARY_DOC_REVIEW.md) is where it comes
# back.  Silence is neither: an unlisted, untagged package is TODO and holds the PR.
#
# This is a REPORT, not a gate.  It is deliberately NOT part of `all` and NOT run by
# library CI, and it always exits 0: a half-adopted repo must stay green, because the
# opt-in ratchet — one library adopting the convention cannot redden its neighbours —
# is what lets the rollout proceed one package at a time.
EXAMPLES_EXEMPT_FILE="${EXAMPLES_EXEMPT_FILE:-examples-exempt.tsv}"

check_examples_progress() {
  local root="${EXAMPLES_REPO_ROOT:-.}" reg="${EXAMPLES_REGISTRY:-scripts/example_repos.tsv}"
  local name; name=$(basename "$(cd "$root" 2>/dev/null && pwd)")
  say "=== Worked-example rollout progress ($name) ==="
  local idx exempt d pkg tags row verdict reason
  local n_tagged=0 n_exempt=0 n_deferred=0 n_todo=0
  idx=$(mktemp); _examples_index_body "$root" "$reg" > "$idx"
  exempt="$root/$EXAMPLES_EXEMPT_FILE"
  for d in "$root"/*/; do
    [ -f "$d/loft.toml" ] || continue          # a package is a dir with a manifest
    pkg=$(basename "$d")
    tags=$(awk -F'\t' -v p="$pkg/" 'index($2, p) == 1 {printf "%s ", $1}' "$idx")
    if [ -n "$tags" ]; then
      green "  tagged    $pkg — ${tags% }"
      n_tagged=$((n_tagged + 1)); continue
    fi
    row=$(awk -F'\t' -v p="$pkg" '$1!~/^#/ && $1==p {print; exit}' "$exempt" 2>/dev/null)
    if [ -z "$row" ]; then
      red "  TODO      $pkg — no tags and no verdict in $EXAMPLES_EXEMPT_FILE"
      n_todo=$((n_todo + 1)); continue
    fi
    verdict=$(printf '%s' "$row" | cut -f2); reason=$(printf '%s' "$row" | cut -f3)
    case "$verdict" in
      exempt)   say    "  exempt    $pkg — $reason"; n_exempt=$((n_exempt + 1)) ;;
      deferred) yellow "  deferred  $pkg — $reason"; n_deferred=$((n_deferred + 1)) ;;
      *)        red    "  TODO      $pkg — unknown verdict '$verdict' (exempt|deferred)"
                n_todo=$((n_todo + 1)) ;;
    esac
  done
  rm -f "$idx"
  say ""
  if [ $((n_tagged + n_exempt + n_deferred + n_todo)) -eq 0 ]; then
    yellow "  no packages found under $root (is EXAMPLES_REPO_ROOT a library repo?)"
  elif [ $n_todo -eq 0 ]; then
    green "READY TO PR — $n_tagged tagged, $n_exempt exempt, $n_deferred deferred, 0 todo"
  else
    yellow "NOT READY — $n_todo package(s) still owe a verdict ($n_tagged tagged, $n_exempt exempt, $n_deferred deferred)"
  fi
}

FEATURES_SNAPSHOT="${FEATURES_SNAPSHOT:-index/features.json}"
FEATURES_EXEMPT_FILE="${FEATURES_EXEMPT_FILE:-features-exempt.tsv}"

# Feature-catalogue review AID — "what is left to check this time".
#
# ⚠ THIS VERIFIES NOTHING ABOUT QUALITY, and it must not be read as if it did. Whether
# an entry is SELF-EXPLANATORY, whether its example demonstrates what the entry
# PROMISES, and whether either has gone stale against code that moved are judgements no
# program makes — they stay an AGENT task (@PLN141 C2 is that task, one entry at a
# time). What a program can do is bound the reading: say what is structurally missing,
# and say which entries the month actually touched, so the pass reads ten entries
# instead of eighty-two.
#
# Never a gate, and exit is always the script's usual: a monthly aid that could block a
# release would be routed around within one cycle.
#
# Two sections:
#
#   MISSING  — structural only. A feature has a written body, a generated page, and
#              either a runnable `tests/docs/features/F<N>.loft` or a row in
#              $FEATURES_EXEMPT_FILE saying why a runnable file cannot show it. Anything
#              else is named here. "Written" is keyed on the @PLN92 Pass-1 marker, NOT on
#              the section headings: @I (infra) entries are written in an
#              infra-appropriate shape (`## What it does`), and a check keyed on the
#              feature headings mis-reads all 35 of them as unwritten — it did, on the
#              first version of this function.
#
#   TO RE-READ — the worklist, with `--since <ref>`. An entry is worth re-reading when
#              the SOURCE that cites it moved: entries are referenced bare as `@F<N>` in
#              src/, default/, tests/ and doc/, so a diff of the citing files since last
#              cycle's watermark is the highest-signal bound available. The twin of
#              `doc-review.sh --since`, which does the same for library docs off changed
#              `pub fn` signatures.
check_features_progress() {
  local snap="$FEATURES_SNAPSHOT" exempt="$FEATURES_EXEMPT_FILE"
  say "=== Feature catalogue — review aid (structure only; quality is an agent task) ==="
  if [ ! -f "$snap" ]; then
    yellow "  $snap missing — run: make features-fetch"; return
  fi
  if ! command -v jq >/dev/null 2>&1; then
    yellow "  jq not installed — cannot read $snap"; return
  fi
  local n_ok=0 n_stub=0 n_todo=0 n_exempt=0 n_deferred=0 n_infra=0 num kind body tag
  say "  -- missing (structural) --"
  while IFS=$'\t' read -r num kind body; do
    if [ "$kind" = "infra" ]; then n_infra=$((n_infra + 1)); continue; fi
    tag="F$num"
    case "$body" in
      *"TBD (Pass 2/3)"*)
        red "    STUB      $tag — never written past @PLN92 Pass 1, so no page is generated"
        n_stub=$((n_stub + 1)); continue ;;
    esac
    if [ ! -f "doc/features/$tag.md" ]; then
      red "    NO PAGE   $tag — body is written but no doc/features/$tag.md was generated"
      n_stub=$((n_stub + 1)); continue
    fi
    if [ -f "tests/docs/features/$tag.loft" ]; then n_ok=$((n_ok + 1)); continue; fi
    local row verdict
    row=$(awk -F'\t' -v t="$tag" '$1!~/^#/ && $1==t {print; exit}' "$exempt" 2>/dev/null)
    if [ -z "$row" ]; then
      red "    NO EXAMPLE $tag — and no verdict in $exempt"
      n_todo=$((n_todo + 1)); continue
    fi
    verdict=$(printf '%s' "$row" | cut -f2)
    case "$verdict" in
      exempt)   n_exempt=$((n_exempt + 1)) ;;
      deferred) yellow "    deferred  $tag — a runnable example is wanted and unwritten"
                n_deferred=$((n_deferred + 1)) ;;
      *)        red "    NO EXAMPLE $tag — unknown verdict '$verdict' (exempt|deferred)"
                n_todo=$((n_todo + 1)) ;;
    esac
  done < <(jq -r '.[] | [(.number|tostring), .kind, ((.body // "") | gsub("[\n\t]"; " "))] | @tsv' "$snap")
  [ $((n_stub + n_todo + n_deferred)) -eq 0 ] && green "    (nothing missing)"
  say ""
  say "  $n_ok with a runnable example · $n_exempt exempt · $n_deferred deferred · \
$n_stub unwritten · $n_todo owe a verdict · $n_infra infra entries (no example expected)"

  if [ -n "${FEATURES_SINCE:-}" ]; then
    say ""
    say "  -- to re-read since ${FEATURES_SINCE} (its citing source moved) --"
    local t n_touch=0 tags
    # The CITING LINES, not the changed files.  A month touches most of src/, so
    # "some file that mentions @F7 changed" selects nearly the whole catalogue and the
    # worklist stops bounding anything.  A line that cites `@F7` being ADDED or REWRITTEN
    # is the sharp signal — the same narrowing `doc-review.sh` gets from diffing `pub fn`
    # signatures rather than whole files.
    # Scope: src/ default/ tests/ — the IMPLEMENTATION and its proofs. `doc/` is
    # deliberately out: a changelog paragraph citing @F109 does not mean @F109's entry
    # drifted, and including it added 8 entries of pure noise to a month's worklist.
    # Measured over 2026-08: src+default 26, +tests 39, +doc 47 — and `-a` on both greps
    # because a binary file in the diff silently TRUNCATES the tag list otherwise (the
    # first version of this reported 3 and looked wonderfully sharp; it had stopped at
    # the first binary match).
    tags=$(git diff --unified=0 "${FEATURES_SINCE}"..HEAD -- src default tests 2>/dev/null \
           | grep -aE '^\+[^+]' | grep -aohE '@F[0-9]+' | sort -u -t F -k2 -n)
    if [ -z "$tags" ]; then
      say "    (no line citing a @F entry changed since ${FEATURES_SINCE})"
    else
      for t in $tags; do
        yellow "    $t — $(jq -r --arg n "${t#@F}" '.[] | select((.number|tostring)==$n) | .title' "$snap" 2>/dev/null | cut -c1-84)"
        n_touch=$((n_touch + 1))
      done
      say ""
      say "    $n_touch entr(y|ies) to re-read. For each: does the entry still describe what the"
      say "    code does, and is its example still the clearest demonstration of it?"
    fi
  else
    say ""
    say "  (pass FEATURES_SINCE=<last cycle's watermark> for the to-re-read worklist)"
  fi
}

# ---- Library review aid: what is left to check this cycle? (@PLN141) ----
LIBRARIES_UNRELEASED="${LIBRARIES_UNRELEASED:-doc/claude/unreleased-snapshot.json}"
LIBRARIES_REGISTRY="${LIBRARIES_REGISTRY:-doc/claude/registry-index-snapshot.json}"
LIBRARIES_REVIEW_DOC="${LIBRARIES_REVIEW_DOC:-doc/claude/LIBRARY_DOC_REVIEW.md}"
LIBRARIES_SIBLINGS="${LIBRARIES_SIBLINGS:-..}"

# The watermark table of LIBRARY_DOC_REVIEW.md, as data: key <TAB> reviewed <TAB> commit.
#
# That table is the ONE home for "reviewed through" — the reviewer edits it by hand in
# step 6 of the protocol, so a second machine-readable copy would be a second list of
# the same fact and would drift the moment someone updated only the prose.  This parses
# the prose instead, which is why column 1 must hold EXACTLY ONE library per row, spelled
# the way the aid keys it (a path for an in-tree tree, the package name for a published
# one).  A row that does not match anything is not silently dropped — it is reported as a
# STALE ROW.  That is how all six unmatched rows of the 2026-08 table surfaced: two
# libraries that had moved out to another repo (`lib/html`, `lib/markdown`), two spelled
# differently from the tree (`stdlib default`, `lib/lexer`), and two that were prose
# standing in for a list (a four-library cell, and "registered libs (…)" naming six of
# thirty-four).
_libraries_watermarks() {
  awk -F'|' '
    /^\| *library *\| *reviewed through *\|/ { t = 1; next }
    t && /^\|[ :-]*-[ :-]*\|/               { next }
    t && /^\|/ {
      k = $2; r = $3; c = $4
      gsub(/`/, "", k); gsub(/`/, "", r); gsub(/`/, "", c)
      gsub(/^[ \t]+|[ \t]+$/, "", k); sub(/\/$/, "", k)
      gsub(/^[ \t]+|[ \t]+$/, "", r); gsub(/^[ \t]+|[ \t]+$/, "", c)
      if (k != "") print k "\t" r "\t" c
      next
    }
    t { t = 0 }
  ' "$LIBRARIES_REVIEW_DOC" 2>/dev/null
}

# The trees this repo reviews itself: the stdlib, each packaged lib, each single-file lib.
# A file with no `pub fn` has no public surface to review and is not a library — that is
# what keeps `lib/docs.loft` and `lib/logger.loft` out of the population without a list.
_libraries_in_tree() {
  local d f
  [ -d default ] && echo "default"
  for d in lib/*/; do [ -f "$d/loft.toml" ] && echo "${d%/}"; done
  for f in lib/*.loft; do [ -f "$f" ] && echo "$f"; done
}

# Worked-example citations carried by a library's OWN source.
#
# NOT `examples-index.tsv`: that indexes where each tag is DEFINED (the test or consumer
# call site), and for `default/` every @STD tag is defined under `tests/scripts/`, for
# `lib/git` under `tools/`.  Keying on the index therefore reported the three best-covered
# in-tree libraries as carrying no examples at all.  The citation is the half that lives
# beside the `pub fn`, so it is the half that answers "does this library cite examples?".
_libraries_citations() {
  grep -rhoa '// Example: @[A-Z][A-Z][A-Z]-[0-9][0-9][0-9]' "$1" --include='*.loft' 2>/dev/null | wc -l
}

# Print one grouped line of the missing report, wrapping a long member list under a hanging
# indent so a 14-package monorepo stays one readable entry.
_libraries_group_line() {
  local line
  while IFS= read -r line; do say "$line"; done < <(
    printf '      %-20s' "$1"
    printf '%s\n' "$2" | fold -s -w 66 | sed '1s/^/ /; 2,$s/^/                           /')
}

# Library-distribution review AID — "what is left to check this cycle".
#
# ⚠ THIS VERIFIES NOTHING ABOUT QUALITY.  Whether a `///` doc still describes what the
# function DOES, and whether a cited example is still the CLEAREST demonstration of it,
# are the two failures LIBRARY_DOC_REVIEW.md exists for, and no program makes either
# judgement.  What a program can do is bound the reading: say what is structurally
# missing, and say which libraries actually moved since they were last read.  Measured on
# the 2026-08 watermarks: one library to re-read out of a population of forty-two.
#
# The twin of `features-progress`, which asks the same two questions of the @F catalogue.
# Never a gate, always exit 0: a monthly aid that could block a release would be routed
# around within one cycle, and libraries are deliberately OFF the release axis anyway
# (RELEASE.md § What forces a release).
#
# Two sections:
#
#   MISSING     — structural only.  A library owes a watermark row (so the next pass has
#                 a baseline) and a worked-example verdict — either its source CITES an
#                 example, or its repo's `examples-exempt.tsv` says why it does not.
#                 Anything else is named.  A row naming a library that is neither in-tree
#                 nor published is a STALE ROW: the table is wrong, not the library.
#
#   TO RE-READ  — the worklist.  A library is worth re-reading when its source moved since
#                 the commit its watermark records.  Per-library watermarks, not one global
#                 `SINCE`: libraries publish on their own cadence, so a single ref is
#                 meaningless across thirty-four packages in eight repos.  Where the repo
#                 is checked out the entry also carries the commit count and how many
#                 `pub fn` lines changed — the same narrowing `doc-review.sh --since` gets
#                 from diffing signatures rather than whole files.
#
# Reads two LOCAL builds (`make libcatalogue`) rather than committed data, deliberately:
# @PLN112 made the catalogue a local build so it cannot go stale, and the published half
# of this report is only as true as its inputs.  Missing snapshots degrade to the in-tree
# trees and say so, rather than reporting a distribution of nothing.
check_libraries_progress() {
  local unrel="$LIBRARIES_UNRELEASED" reg="$LIBRARIES_REGISTRY" wm_file="$LIBRARIES_REVIEW_DOC"
  say "=== Library distribution — review aid (structure only; quality is an agent task) ==="
  if ! command -v jq >/dev/null 2>&1; then
    yellow "  jq not installed — cannot read $unrel"; return
  fi
  local have_pub=1
  if [ ! -f "$unrel" ] || [ ! -f "$reg" ]; then
    yellow "  $unrel / $reg missing — run: make libcatalogue"
    yellow "  (in-tree trees only; the published distribution needs those snapshots)"
    have_pub=0
  fi
  # SAY HOW OLD THE WORKLIST IS.  The package list comes from a local snapshot, and a
  # snapshot omits every package that landed after it was built — which is exactly the set
  # most likely to owe a review, because a new package has never had one.  Silent, that
  # reads as "these libraries owe nothing" rather than "I did not look at them": a
  # five-day-old snapshot hid four packages that each owed a verdict, and the report
  # above them was confident and complete-looking (@PLN141).
  if [ $have_pub -eq 1 ]; then
    # `stat` twice, GNU then BSD: `date -r FILE` is the file's mtime on GNU and reads
    # FILE as EPOCH SECONDS on macOS, so it answers a wrong age there rather than
    # failing — the same shape of silent wrongness this warning exists to prevent.
    local snap_epoch snap_days
    snap_epoch=$(stat -c %Y "$unrel" 2>/dev/null || stat -f %m "$unrel" 2>/dev/null || echo 0)
    snap_days=$(( ( $(date +%s) - snap_epoch ) / 86400 ))
    [ "$snap_epoch" -gt 0 ] 2>/dev/null || snap_days=0
    if [ "$snap_days" -ge 2 ]; then
      yellow "  ⚠ the package list is a snapshot built ${snap_days} day(s) ago — anything published"
      yellow "    or merged since is NOT on this worklist.  Refresh: make libcatalogue"
    else
      say "  (package list from a snapshot built ${snap_days} day(s) ago — make libcatalogue to refresh)"
    fi
  fi

  local wm pop; wm=$(mktemp); pop=$(mktemp)
  _libraries_watermarks > "$wm"
  # key <TAB> kind <TAB> group <TAB> dir <TAB> sha <TAB> pub-fns <TAB> undocumented
  local k n sha
  while read -r k; do
    [ -n "$k" ] || continue
    if [ -d "$k" ]; then n=$(grep -rha '^[[:space:]]*pub fn ' "$k" --include='*.loft' 2>/dev/null | wc -l)
    else                 n=$(grep -cae '^[[:space:]]*pub fn ' "$k" 2>/dev/null); fi
    [ "${n:-0}" -gt 0 ] || continue
    sha=$(git log -1 --format=%H -- "$k" 2>/dev/null)
    printf '%s\tin-tree\tin-tree\t%s\t%s\t%s\t-\t-\n' "$k" "$k" "${sha:--}" "$n"
  done < <(_libraries_in_tree) >> "$pop"
  if [ $have_pub -eq 1 ]; then
    jq -r --slurpfile r "$reg" '
      to_entries[] | . as $p |
      (($r[0].packages[$p.key].homepage // "")
        | capture("github\\.com/[^/]+/(?<repo>[^/]+)/tree/[^/]+/(?<dir>.+)$")
        // {repo: "?", dir: $p.key}) as $loc |
      ([$p.value.api[] | select((.doc // "") == "")]) as $u |
      [ $p.key, "published", $loc.repo, $loc.dir, ($p.value.sha // "-"),
        ($p.value.api | length),
        ($u | length),
        ([$u[] | select((.sig | test("^[[:space:]]*pub[[:space:]]+fn\\b")) | not)] | length) ] | @tsv' \
      "$unrel" >> "$pop"
  fi

  local g_new g_todo g_stale reread
  g_new=$(mktemp); g_todo=$(mktemp); g_stale=$(mktemp); reread=$(mktemp)
  local n_rev=0 n_new=0 n_stale=0 n_exempt=0 n_deferred=0 n_todo=0 n_blindpath=0
  local n_api_pub=0 n_api_tree=0 n_undoc=0 n_undoc_ty=0
  local kind group dir cur api undoc row wmrev wmcom tree cites verdict reason

  while IFS=$'\t' read -r k kind group dir cur api undoc undoc_ty; do
    if [ "$kind" = "published" ]; then n_api_pub=$((n_api_pub + api)); n_undoc=$((n_undoc + undoc))
      n_undoc_ty=$((n_undoc_ty + undoc_ty))
    else                               n_api_tree=$((n_api_tree + api)); fi
    row=$(awk -F'\t' -v k="$k" '$1 == k {print; exit}' "$wm")
    wmrev=$(printf '%s' "$row" | cut -f2); wmcom=$(printf '%s' "$row" | cut -f3)
    case "${row:+set}${wmrev}" in
      ""|"set—"|"set-") printf '%s\t%s\n' "$group" "$k" >> "$g_new"; n_new=$((n_new + 1)) ;;
      *) n_rev=$((n_rev + 1))
         printf '%s\t%s\t%s\t%s\t%s\n' "$k" "$kind" "$group" "$dir" "$wmcom|$cur" >> "$reread" ;;
    esac
    # Worked-example verdict — needs the library's own source, so it needs a checkout.
    if [ "$kind" = "in-tree" ]; then tree="$dir"; row="$EXAMPLES_EXEMPT_FILE"
    else tree="$LIBRARIES_SIBLINGS/$group/$dir"; row="$LIBRARIES_SIBLINGS/$group/$EXAMPLES_EXEMPT_FILE"; fi
    if [ ! -e "$tree" ]; then n_blindpath=$((n_blindpath + 1)); continue; fi
    cites=$(_libraries_citations "$tree")
    [ "${cites:-0}" -gt 0 ] && continue
    row=$(awk -F'\t' -v p="$dir" '$1 !~ /^#/ && $1 == p {print; exit}' "$row" 2>/dev/null)
    if [ -z "$row" ]; then
      printf '%s\t%s\n' "$group" "$k" >> "$g_todo"; n_todo=$((n_todo + 1)); continue
    fi
    verdict=$(printf '%s' "$row" | cut -f2); reason=$(printf '%s' "$row" | cut -f3)
    case "$verdict" in
      exempt)   n_exempt=$((n_exempt + 1)) ;;
      deferred) printf '%s\t%s — %s\n' "$group" "$k" "${reason:0:56}" >> "$g_todo"
                n_deferred=$((n_deferred + 1)) ;;
      *)        printf "%s\t%s — BAD VERDICT '%s' (exempt|deferred)\n" "$group" "$k" "$verdict" >> "$g_todo"
                n_todo=$((n_todo + 1)) ;;
    esac
  done < "$pop"
  while IFS=$'\t' read -r k wmrev wmcom; do
    awk -F'\t' -v k="$k" '$1 == k {f = 1} END {exit !f}' "$pop" && continue
    printf '%s\n' "$k" >> "$g_stale"; n_stale=$((n_stale + 1))
  done < "$wm"

  say "  -- missing (structural) --"
  local grp
  if [ -s "$g_new" ]; then
    yellow "    never reviewed — no row in $wm_file"
    while read -r grp; do
      _libraries_group_line "$grp" "$(awk -F'\t' -v g="$grp" '$1 == g {printf "%s ", $2}' "$g_new")"
    done < <(cut -f1 "$g_new" | awk '!seen[$0]++')
  fi
  if [ -s "$g_todo" ]; then
    yellow "    owe a worked-example verdict — no \`// Example:\` citation, no $EXAMPLES_EXEMPT_FILE row"
    while IFS=$'\t' read -r grp k; do say "      $k"; done < "$g_todo"
  fi
  if [ -s "$g_stale" ]; then
    red "    STALE ROW — names a library that is neither in-tree nor published; fix $wm_file"
    while IFS= read -r k; do say "      $k"; done < "$g_stale"
  fi
  [ $((n_new + n_todo + n_deferred + n_stale)) -eq 0 ] && green "    (nothing missing)"

  say ""
  say "  $n_rev reviewed · $n_new never reviewed · $n_exempt exempt · $n_deferred deferred · \
$n_todo owe a verdict · $n_stale stale row(s)"
  # SPLIT THE UNDOCUMENTED COUNT BY KIND.  Read as one number it says 2.5% and reads as
  # fine; split, it says the functions are at 0.2% and the TYPES at 22% — a blind spot two
  # orders of magnitude wide, on the items a reader meets first.  Measured 2026-09: 33 of
  # the 34 undocumented types are named in another public signature, `stage::Stage` in 111
  # of them.  The single figure is what hid that, so it is not printed alone.
  if [ $have_pub -eq 1 ]; then
    say "  public surface: $n_api_pub published pub items ($n_undoc undocumented — \
$((n_undoc - n_undoc_ty)) fn, $n_undoc_ty type) · $n_api_tree in-tree pub fns"
  fi
  [ $n_blindpath -gt 0 ] && yellow "  $n_blindpath library(ies) not checked out under \
$LIBRARIES_SIBLINGS/ — their verdict is unknown, not absent"

  say ""
  say "  -- to re-read (its source moved since its watermark) --"
  local n_moved=0 n_blind=0 commits sigs repo_dir
  while IFS=$'\t' read -r k kind group dir row; do
    wmcom="${row%|*}"; cur="${row#*|}"
    if [ "$kind" = "in-tree" ]; then repo_dir="."; else repo_dir="$LIBRARIES_SIBLINGS/$group"; fi
    case "$wmcom" in
      ""|"—"|"-"|"(bootstrap)")
        yellow "    $(printf '%-22s' "$k") reviewed, but no commit recorded — nothing to diff against"
        n_blind=$((n_blind + 1)); continue ;;
    esac
    if [ ! -d "$repo_dir" ] || ! git -C "$repo_dir" cat-file -e "$wmcom^{commit}" 2>/dev/null; then
      yellow "    $(printf '%-22s' "$k") watermark $wmcom unreachable — no checkout to diff"
      n_blind=$((n_blind + 1)); continue
    fi
    [ "$cur" = "-" ] && cur=$(git -C "$repo_dir" rev-parse HEAD 2>/dev/null)
    commits=$(git -C "$repo_dir" rev-list --count "$wmcom..$cur" -- "$dir" 2>/dev/null)
    [ "${commits:-0}" -eq 0 ] && continue
    # -a on the grep: a binary file in the diff otherwise TRUNCATES the scan and the
    # count reads wonderfully low (the same trap features-progress hit).
    sigs=$(git -C "$repo_dir" diff --unified=0 "$wmcom..$cur" -- "$dir" 2>/dev/null \
           | grep -acE '^[+-][^+-].*pub fn ')
    yellow "    $(printf '%-22s' "$k") ${wmcom:0:8} → ${cur:0:8} — $commits commit(s), \
${sigs:-0} pub fn line(s) changed"
    n_moved=$((n_moved + 1))
  done < "$reread"
  if [ $((n_moved + n_blind)) -eq 0 ]; then
    green "    (nothing reviewed has moved)"
  else
    say ""
    say "    $n_moved to re-read, $n_blind with no usable baseline.  For each: does the doc still"
    say "    describe what the code does, and is its example still the clearest demonstration?"
  fi
  rm -f "$wm" "$pop" "$g_new" "$g_todo" "$g_stale" "$reread"
}

# Sep between sections (verbose mode only).
sep() { [ $QUIET -eq 0 ] && echo; }

case "$CHECK" in
  paths)   check_paths ;;
  time)    check_time ;;
  stale)   check_stale ;;
  roadmap) check_roadmap ;;
  refs)    check_refs ;;
  libs)    check_libs ;;
  examples) check_examples ;;
  examples-selftest) check_examples_selftest; sep; check_examples_cite_selftest; sep; check_examples_online_selftest ;;
  examples-index) check_examples_index ;;
  emit-examples-index) emit_examples_index; exit 0 ;;
  examples-preflight)
    # Everything a library PR's tag checks would report, with a REAL exit code.
    EXAMPLES_PREFLIGHT=1; EXAMPLES_CITE_GATES=1
    check_examples; sep; check_examples_index ;;
  validator) check_validator ;;
  write-examples-index) write_examples_index; exit 0 ;;
  examples-progress) check_examples_progress; exit 0 ;;   # a REPORT — never in `all`
  features-progress) check_features_progress; exit 0 ;;   # a REVIEW AID — never in `all`, never a gate
  libraries-progress) check_libraries_progress; exit 0 ;; # a REVIEW AID — never in `all`, never a gate
  all)
    check_paths
    sep
    check_time
    sep
    check_stale
    sep
    check_roadmap
    sep
    check_refs
    sep
    check_libs
    sep
    check_examples_selftest
    sep
    check_examples_cite_selftest
    sep
    check_examples
    sep
    check_examples_index
    sep
    check_validator
    ;;
  *)
    echo "Usage: $0 [-q|--quiet] [all|paths|time|stale|roadmap|refs|libs|examples|examples-selftest|examples-index|emit-examples-index|examples-preflight|validator|write-examples-index|examples-progress|features-progress|libraries-progress]" >&2
    exit 2
    ;;
esac

# One-line summary (always printed; even in quiet mode this is the only output).
if [ "${EXAMPLES_CITE_GATES:-1}" -eq 0 ]; then
  # Cross-repo run: the worked-example checks ADVISE rather than gate (see the tier note
  # at the top).  They still print in full and still show in the summary.
  total=$((HITS_PATHS + HITS_STALE + HITS_ROADMAP + HITS_REFS + HITS_VALIDATOR))
  warns=$((HITS_TIME + HITS_LIBS + HITS_EXAMPLES + HITS_EXAMPLES_WARN + HITS_EXINDEX + HITS_VALIDATOR_WARN))
  tier_note=" [worked-example checks advisory: cross-repo — 'examples-preflight' to gate them here]"
else
  total=$((HITS_PATHS + HITS_STALE + HITS_ROADMAP + HITS_REFS + HITS_EXAMPLES + HITS_EXINDEX + HITS_VALIDATOR))
  warns=$((HITS_TIME + HITS_LIBS + HITS_EXAMPLES_WARN + HITS_VALIDATOR_WARN))
  if [ "${EXAMPLES_PREFLIGHT:-0}" -eq 1 ]; then
    tier_note=" [preflight: citation faults GATE, the index is not required]"
  else
    tier_note=""
  fi
fi
summary="paths=$HITS_PATHS time=$HITS_TIME stale=$HITS_STALE roadmap=$HITS_ROADMAP refs=$HITS_REFS libs=$HITS_LIBS examples=$HITS_EXAMPLES/w$HITS_EXAMPLES_WARN exindex=$HITS_EXINDEX validator=$HITS_VALIDATOR/w$HITS_VALIDATOR_WARN$tier_note"
if [ $DRIFT -eq 0 ] && [ $warns -eq 0 ]; then
  printf '\033[32mclean\033[0m (%s)\n' "$summary"
  exit 0
elif [ $DRIFT -eq 0 ]; then
  # Only warn-level findings — exit 0 but flag in summary.
  printf '\033[33mclean+warn\033[0m (%s) — %d warning(s)\n' "$summary" "$warns"
  exit 0
else
  printf '\033[31mDRIFT\033[0m (%s) — %d action items\n' "$summary" "$total"
  exit 1
fi
