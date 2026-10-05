#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Regenerate the pure derived artefacts on a runner (.github/workflows/derive.yml) for this
# branch, wait for it, and fast-forward this checkout onto the bot's commit.
#
#   scripts/derive-ci.sh            # the current branch
#   scripts/derive-ci.sh <branch>   # another branch; this checkout is left alone
#
# The branch must be pushed: the runner regenerates from what is on origin.  The fast-forward
# is `git merge --ff-only` on a CLEAN tree only — never a pull — and a checkout that has
# committed since is told to rebase instead (CLAUDE.md § Git safety).  JOINING.md § Derived
# artefacts on a runner.
set -euo pipefail

here=$(git rev-parse --abbrev-ref HEAD)
branch=${1:-$here}
[ "$branch" != main ] || { echo "main takes changes only through a PR" >&2; exit 2; }

local_sha=$(git rev-parse "$branch")
git fetch -q origin "$branch"
if [ "$(git rev-parse "origin/$branch")" != "$local_sha" ]; then
  echo "$branch is not what origin has — push it first, so the runner sees this tree" >&2
  exit 2
fi

since=$(date -u +%Y-%m-%dT%H:%M:%SZ)
gh workflow run derive.yml --ref "$branch" >/dev/null
run=""
for _ in $(seq 1 30); do
  run=$(gh run list --workflow derive.yml --branch "$branch" --event workflow_dispatch \
        --created ">=$since" --limit 1 --json databaseId -q '.[0].databaseId // empty')
  [ -n "$run" ] && break
  sleep 4
done
[ -n "$run" ] || { echo "the dispatched run did not appear" >&2; exit 1; }
echo "derive run $run on $branch"
gh run watch "$run" --exit-status >/dev/null || {
  echo "derive run $run failed: gh run view $run --log-failed" >&2; exit 1; }

git fetch -q origin "$branch"
if [ "$(git rev-parse "origin/$branch")" = "$local_sha" ]; then
  echo "every pure derived artefact already matched"
  exit 0
fi
git log --oneline -1 "origin/$branch"
if [ "$branch" != "$here" ]; then
  exit 0
fi
if [ -n "$(git status --porcelain --untracked-files=no)" ] || [ "$(git rev-parse HEAD)" != "$local_sha" ]; then
  echo "this checkout changed meanwhile: commit, then \`git rebase origin/$branch\`" >&2
  exit 1
fi
git merge -q --ff-only "origin/$branch"
echo "fast-forwarded onto the regenerated artefacts"
