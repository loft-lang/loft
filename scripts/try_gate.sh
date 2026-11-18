#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# The try-gate (@PLN91): ask the testing side to gate a change BEFORE it is promoted.
#
#   scripts/try_gate.sh            # hand over the work in this checkout, committed or not, and go on
#   scripts/try_gate.sh --wait     # stay until the gate has run (its line only; the analysis is a message)
#   scripts/try_gate.sh --base REV # on another base than the last promoted tip
#
# It posts ONE comment on the mailbox issue (loft#2560): the change as a `git bundle` on the
# base tip, in text.  Nothing is pushed to any branch.  This is a HAND-OFF, not a tool to wait
# on.  A CLEAN result (nothing that agreed is lost) is acknowledged automatically as a comment
# `result <commit>`: safe to promote.  One with ISSUES is answered only when the testing side
# has analysed what is wrong — what stopped agreeing and why, each differ at its line.
#
# Uncommitted work is tried as it stands (tracked files; an untracked file is named and left
# out — `git add` it first).  The base must be a tip the tester has gated: by default the tip of
# the promoted line this checkout was made from.
set -euo pipefail

ISSUE=2560
REPO=loft-lang/loft
LINE=${LOFT_TRY_LINE:-origin/tuxedo-pln204-int32}   # the promoted line
wait=0
base=""
while [ $# -gt 0 ]; do
  case "$1" in
    --wait) wait=1 ;;
    --no-wait) wait=0 ;;
    --base) base=$2; shift ;;
    -h|--help) sed -n '5,20p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "try_gate: unknown argument $1 (see --help)" >&2; exit 2 ;;
  esac
  shift
done

git fetch -q origin "${LINE#origin/}" 2>/dev/null || true
[ -n "$base" ] || base=$(git merge-base HEAD "$LINE")
base=$(git rev-parse "$base^{commit}")

# The change: HEAD, or HEAD with the uncommitted tracked work on top (a commit object of its
# own, made without touching the branch, the index or the working tree).
head=$(git rev-parse HEAD)
if ! git diff --quiet HEAD; then
  head=$(git stash create "try-gate: uncommitted work on $(git rev-parse --short HEAD)")
  echo "try_gate: including the uncommitted changes of $(git diff --name-only HEAD | wc -l) tracked file(s)"
fi
untracked=$(git ls-files --others --exclude-standard -- '*.loft' '*.rs' '*.py' | head -5)
[ -z "$untracked" ] || echo "try_gate: NOT included (untracked — \`git add\` to include): $(echo $untracked)" >&2
if [ "$head" = "$base" ]; then
  echo "try_gate: nothing to try — this checkout is the base tip $(git rev-parse --short "$base")" >&2; exit 2
fi

# A bundle is made from a NAME, not a bare commit id: a local ref of its own, removed again.
git update-ref refs/try-gate/send "$head"
trap 'git update-ref -d refs/try-gate/send 2>/dev/null || true' EXIT
payload=$(git bundle create - "$base..refs/try-gate/send" 2>/dev/null | base64 -w0)
size=${#payload}
if [ "$size" -gt 60000 ]; then
  echo "try_gate: the change is $size characters as a bundle; a comment holds about 65000 — split it, or promote and read the cycle" >&2; exit 2
fi

short=$(git rev-parse --short "$head")
body=$(printf 'try %s on %s\n\n%s file(s) changed: %s\n\n```bundle\n%s\n```\n' \
  "$head" "$base" "$(git diff --name-only "$base" "$head" | wc -l)" \
  "$(git diff --name-only "$base" "$head" | head -8 | tr '\n' ' ')" "$payload")
url=$(gh issue comment "$ISSUE" --repo "$REPO" --body "$body")
echo "try_gate: handed over $short on $(git rev-parse --short "$base") ($size characters) — $url"
echo "try_gate: carry on; a clean result is acknowledged on the issue in about ten minutes, one with issues comes back analysed"
[ "$wait" = 1 ] || exit 0

echo "try_gate: waiting for the answer (ten minutes when clean, longer when it needs analysis; Ctrl-C leaves the request in place)…"
for _ in $(seq 1 120); do
  sleep 30
  answer=$(gh api "repos/$REPO/issues/$ISSUE/comments?per_page=30&sort=created&direction=desc" \
    --jq "map(select(.body | startswith(\"result $head\"))) | .[0].body // empty" 2>/dev/null || true)
  if [ -n "$answer" ]; then
    echo "$answer"
    exit 0
  fi
done
echo "try_gate: no answer within an hour; it will appear on loft#$ISSUE" >&2
exit 1
