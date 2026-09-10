#!/usr/bin/env bash
# work-issues — the open issues that are actually PICK-UP work.
#
# "Work" is the residue after two exclusions, both of which mean *no agent work
# remains right now*:
#
#   * `fixed-pending-merge` — the fix has landed on a branch; only the merge is left,
#     and the `Fixes #N` trailer closes it automatically (LABELS.md § Lifecycle).
#   * `status:planned`      — the fix path is a PLAN and progress is measured there;
#     no short-term fix will be taken, and the label names the plan.
#
# Everything else is somebody's next task.  That is the whole point: it makes
# *"is there work?"* a query rather than a judgement, so the answer cannot be
# improved by closing or relabelling an issue — the two exits a board-shaped
# goal otherwise leaves open.
#
# A REPORT, never a gate: exit 0 whether the list is empty or not.  `--count`
# prints just the number, for a goal or a prompt that wants the scalar.  Exit 2 is
# reserved for "could not ask" — a failed query must never read as an empty board.
#
# Usage:  scripts/work-issues.sh [--count] [--repo owner/name] [--label L]
#
#   --label L   restrict to issues carrying L (e.g. `bug`), repeatable
set -u

repo=""; count_only=0; labels=()
while [ $# -gt 0 ]; do
    case "$1" in
        --count) count_only=1 ;;
        --repo) shift; repo="$1" ;;
        --label) shift; labels+=("$1") ;;
        -h|--help) sed -n '2,22p' "$0" | sed 's/^# \?//'; exit 0 ;;
        *) echo "work-issues: unknown argument $1" >&2; exit 2 ;;
    esac
    shift
done

# ⚠ `gh issue list --label` on a label that does not EXIST succeeds and returns an
# empty list, so a typo reads as a clear board — the likelier half of the false clean,
# since a label name is typed far more often than a repo name.  Check each one first.
if [ ${#labels[@]} -gt 0 ]; then
    known="$(gh label list --limit 300 --json name --jq '.[].name' ${repo:+--repo "$repo"} 2>&1)" || {
        echo "work-issues: cannot ask the tracker for its labels — $known" >&2; exit 2; }
    for l in "${labels[@]}"; do
        printf '%s\n' "$known" | grep -Fxq -- "$l" || {
            echo "work-issues: no such label '$l' — an unknown label would list nothing and read as a clear board" >&2
            exit 2; }
    done
fi

args=(issue list --state open --limit 200 --json number,title,labels)
[ -z "$repo" ] || args+=(--repo "$repo")
for l in ${labels+"${labels[@]}"}; do args+=(--label "$l"); done

# One jq: drop the two "no work remains" labels, then render severity first so the
# list reads in the order it should be picked up in.
jq_prog='
  [ .[]
    | select([.labels[].name] as $l
             | ($l | index("fixed-pending-merge")) == null
               and ($l | index("status:planned")) == null)
  ]
  | sort_by([.labels[].name] | map(select(startswith("sev:"))) | .[0] // "sev:zzz")
  | .[]
  | "#\(.number)  \([.labels[].name] | map(select(startswith("sev:") or startswith("area:"))) | join(" "))  \(.title)"
'

# ⚠ A query that FAILED must not read as "no work".  An unreachable tracker, a bad
# `--repo`, or a label that does not exist would otherwise print the same reassuring
# "none" as a genuinely clear board — the false clean this repo treats as a defect in
# the instrument (TESTING.md § How a guard reads green while the defect stands).  So
# the empty LIST is exit 0 and failing to ASK is exit 2.
if ! out="$(gh "${args[@]}" --jq "$jq_prog" 2>&1)"; then
    echo "work-issues: cannot ask the tracker — $out" >&2
    exit 2
fi

n=$(printf '%s' "$out" | grep -c . || true)

if [ "$count_only" = 1 ]; then echo "$n"; exit 0; fi

if [ "$n" = 0 ]; then
    echo "work issues: none — every open issue either carries a fix or names its plan."
    exit 0
fi

echo "work issues: $n (open, minus fixed-pending-merge and status:planned)"
echo
printf '%s\n' "$out"
