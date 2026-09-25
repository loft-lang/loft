#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Comment-quality detector for src/ (and lib/**/src/) — the runnable
# Check behind doc/claude/DOC_QUALITY.md.  It flags three waste patterns,
# so doc quality is *evaluated, not asserted*:
#
#   1. History stamps — plan tags, phase/cluster/arc refs, bare dates in
#      comments.  git blame already owns this.
#   2. Change-narration — comments that describe a past edit ("removed",
#      "used to misroute", "previously inlined") instead of the code as
#      it is now.
#   3. Incident-subject — comments organised around the BUG rather than
#      the contract ("panicked", "silently wrong", "never fired", "the
#      hole").  A different axis from 1 and 2: such a comment is often
#      present-tense and stamp-free and still answers a question nobody
#      has any more.  See DOC_QUALITY.md § B2 — the fix is to CONVERT the
#      story into the rule it contains, not to delete it.
#
# ADVISORY by design: it never fails CI and never edits.  A flagged line
# is a REVIEW HINT, not a verdict (a live `#NNN` / doc pointer is a
# keeper — see DOC_QUALITY.md rule 2).  Exit is always 0.
#
# This is CODE-only.  Docs legitimately carry plan tags and dates
# (changelogs, plans, GOALS.md) — they are linted by check_doc_drift.sh,
# not here.
#
# Baseline ratchet — adopt on a legacy tree with no big-bang cleanup:
#   T0   scripts/lint_comments.sh --baseline   # accept today's flagged lines
#                                               # into scripts/.lint_comments_baseline
#   CI   scripts/lint_comments.sh --check       # advisory: lists only NEW
#                                               # flagged lines (not in baseline)
#   fix  scripts/lint_comments.sh --prune        # after a cleanup pass, drop the
#                                                # now-fixed lines from the baseline
# The baseline is keyed by file + comment text (not line number), so it
# survives reformatting and code moving around.  The baseline's shrinking
# size is the cleanup timeline.
#
# Cleanup aid — see the biggest offenders first:
#   scripts/lint_comments.sh top [N]    # files ranked by flagged count (default 20)
#
# Report modes:
#   scripts/lint_comments.sh            # full report + biggest offenders
#   scripts/lint_comments.sh -c         # counts only (the thermometer)
#   scripts/lint_comments.sh tags       # only history stamps
#   scripts/lint_comments.sh history    # only change-narration
#   scripts/lint_comments.sh incident   # only incident-subject (§ B2)

set -u
cd "$(dirname "$0")/.."

BASELINE="scripts/.lint_comments_baseline"

FILES=$(find src lib -name '*.rs' 2>/dev/null | grep -E '/src/|^src/')
if [ -z "$FILES" ]; then echo "no source files found"; exit 0; fi

TAGS_RE='(@PLAN|@P[0-9]|plan-[0-9]|phase [0-9]|cluster [0-9]|arc [A-Z]|[0-9]{4}-[0-9]{2}-[0-9]{2}|[0-9]{4}-[0-9]{2})'
HIST_RE='\b(removed|no longer|used to [a-z]+|previously [a-z]+ed|formerly|changed from)\b'
# The incident as the comment's SUBJECT (§ B2).
#
# Deliberately a strong UNDER-approximation, and the reason is precision: this axis is
# semantic ("is the bug the subject?"), not lexical, so the obvious failure vocabulary
# is mostly innocent in this codebase.  Measured before narrowing: `SIGSEGV` is what
# `crash_report.rs` installs a handler for, `the hole` is Robin Hood hashing and the
# lexer's unclosed brace, `silently dropped` describes a live spoof-check, `never
# reported` is a contract statement, and `loft#885's hoisted reads` names a mechanism
# by its issue — a POINTER, which rule 2 explicitly keeps.  A noisy thermometer gets
# ignored, so only phrasings that are almost never innocent are kept.
#
# The rest of this axis is a REVIEW question, not a grep: run the deletion test in
# DOC_QUALITY.md § B2 on the comment in front of you.
INCIDENT_RE='\b(before the fix|regressed|answered (wrong|the FALLBACK)|(was|were) silently (wrong|lost|excluded|misrouted)|the bug was)\b'

# The licence header is not a history stamp.  `Copyright (c) 2022-2025` matches the
# bare-date half of TAGS_RE on every file that carries one, and the flag is unactionable
# by construction — the year range is the licence, not a note about when a line was
# written.  Excluding it is the same precision argument the INCIDENT_RE block above
# makes: a thermometer that reports what nobody can act on gets ignored.
LICENCE_RE='(Copyright \(c\)|SPDX-License-Identifier)'

# Emit "path:lineno:content" for every flagged comment line (both patterns).
collect_raw() {
  { grep -rnE '^[[:space:]]*///?' $FILES 2>/dev/null | grep -E "$TAGS_RE"
    grep -rnE '^[[:space:]]*///?' $FILES 2>/dev/null | grep -Ei "$HIST_RE"
    grep -rnE '^[[:space:]]*///?' $FILES 2>/dev/null | grep -Ei "$INCIDENT_RE"
  } | grep -vE "$LICENCE_RE" | sort -u
}

# Normalise raw lines to baseline keys: "path<TAB>trimmed-comment-text".
# Dropping the line number lets the baseline survive line moves.
emit_keys() {
  collect_raw | awk '{
    content=$0; sub(/^[^:]*:[0-9]+:/,"",content);
    gsub(/^[ \t]*\/\/\/?/,"",content);
    gsub(/[ \t]+/," ",content); sub(/^ /,"",content); sub(/ $/,"",content);
    path=$0; sub(/:.*/,"",path);
    print path "\t" content;
  }' | sort -u
}

cmd="${1:-report}"

case "$cmd" in
--baseline)
  emit_keys > "$BASELINE"
  echo "Wrote $(wc -l < "$BASELINE") baseline entries to $BASELINE"
  echo "(these flagged lines are now accepted; --check reports only NEW ones)"
  ;;

--check)
  if [ ! -f "$BASELINE" ]; then
    echo "No $BASELINE yet — run: scripts/lint_comments.sh --baseline"
    exit 0
  fi
  new=$(collect_raw | awk -v basef="$BASELINE" '
    BEGIN{ while((getline l < basef) > 0) seen[l]=1 }
    {
      content=$0; sub(/^[^:]*:[0-9]+:/,"",content);
      gsub(/^[ \t]*\/\/\/?/,"",content);
      gsub(/[ \t]+/," ",content); sub(/^ /,"",content); sub(/ $/,"",content);
      path=$0; sub(/:.*/,"",path);
      if (!((path "\t" content) in seen)) print $0;
    }')
  if [ -z "$new" ]; then
    echo "No new flagged comments beyond the baseline. ✓"
    exit 0
  fi
  n=$(printf '%s\n' "$new" | grep -c .)
  echo "== $n NEW flagged comment line(s) not in baseline (advisory) =="
  printf '%s\n' "$new"
  # Surface in CI as a warning annotation, but never fail the build.
  if [ -n "${GITHUB_ACTIONS:-}" ]; then
    echo "::warning::$n new comment-quality flag(s) — see DOC_QUALITY.md (advisory, not blocking)"
  fi
  exit 0
  ;;

--prune)
  if [ ! -f "$BASELINE" ]; then echo "No $BASELINE to prune."; exit 0; fi
  before=$(wc -l < "$BASELINE")
  cur=$(mktemp); emit_keys > "$cur"
  tmp=$(mktemp)
  # Keep only baseline entries that still match a currently-flagged line.
  awk 'NR==FNR{live[$0]=1; next} ($0 in live)' "$cur" "$BASELINE" > "$tmp"
  mv "$tmp" "$BASELINE"; rm -f "$cur"
  after=$(wc -l < "$BASELINE")
  echo "Pruned $((before - after)) now-fixed entries; $after remain in $BASELINE"
  ;;

top)
  N="${2:-20}"
  echo "== Biggest offenders — files by flagged comment lines (top $N) =="
  collect_raw | sed 's/:.*//' | sort | uniq -c | sort -rn | head -"$N"
  echo "(total flagged lines: $(collect_raw | grep -c .))"
  ;;

*)
  # Report modes: report (default) | -c/--counts | tags | history | incident
  tag_n=$(grep -rnE '^[[:space:]]*///?' $FILES 2>/dev/null | grep -vE "$LICENCE_RE" | grep -Ec "$TAGS_RE")
  hist_n=$(grep -rnE '^[[:space:]]*///?' $FILES 2>/dev/null | grep -vE "$LICENCE_RE" | grep -Eic "$HIST_RE")
  inc_n=$(grep -rnE '^[[:space:]]*///?' $FILES 2>/dev/null | grep -vE "$LICENCE_RE" | grep -Eic "$INCIDENT_RE")

  if [ "$cmd" != "-c" ] && [ "$cmd" != "--counts" ]; then
    if [ "$cmd" = "report" ] || [ "$cmd" = "tags" ]; then
      echo "== History stamps (plan tag / phase / date) — review, strip the stamp =="
      grep -rnE '^[[:space:]]*///?' $FILES 2>/dev/null | grep -vE "$LICENCE_RE" | grep -E "$TAGS_RE" || echo "  (none)"
      echo
    fi
    if [ "$cmd" = "report" ] || [ "$cmd" = "history" ]; then
      echo "== Change-narration (describes a past edit, not the present code) =="
      echo "   (note: 'used to <verb>' can be innocent — 'used to size the gutter')"
      grep -rnE '^[[:space:]]*///?' $FILES 2>/dev/null | grep -vE "$LICENCE_RE" | grep -Ei "$HIST_RE" || echo "  (none)"
      echo
    fi
    if [ "$cmd" = "report" ] || [ "$cmd" = "incident" ]; then
      echo "== Incident-subject (documents the bug, not the contract) =="
      echo "   (DOC_QUALITY.md § B2 — CONVERT the story into its rule, do not delete it)"
      grep -rnE '^[[:space:]]*///?' $FILES 2>/dev/null | grep -vE "$LICENCE_RE" | grep -Ei "$INCIDENT_RE" || echo "  (none)"
      echo
    fi
    if [ "$cmd" = "report" ]; then
      echo "== Biggest offenders (files by flagged comment lines) =="
      collect_raw | sed 's/:.*//' | sort | uniq -c | sort -rn | head -15
      echo "   (full ranking: scripts/lint_comments.sh top)"
      echo
    fi
  fi

  echo "history-stamp comment lines : $tag_n"
  echo "change-narration comment lines : $hist_n"
  echo "incident-subject comment lines : $inc_n"
  echo "(advisory thermometer — never fails CI)"
  ;;
esac

exit 0
