#!/usr/bin/env bash
# @PLN102 arc-E flip-gate — the CI drift gates: a frozen-golden change ⇒ CONTRACT_VERSION must bump.
#
# Two frozen surfaces make up the persistence/behaviour contract:
#   - LAYOUT  (tests/golden/layout/corpus.txt)   — the byte layout of stored data (Gate 1).
#     A change misreads a store persisted under the old layout.
#   - BEHAVIOUR (tests/golden/behavior/corpus.out) — the observable output of valid programs (Gate 2).
#     A changed VALUE on a valid program is a silent semantics break.
#
# POST-FREEZE a change to either golden may land only ALONGSIDE a CONTRACT_VERSION bump
# (a declared, epoch-style break). INERT while CONTRACT_VERSION == 0 (pre-freeze the
# language is still settling, so both are free — you just re-bless). The gate arms itself
# at the 0 -> 1 flip; nothing here changes on flip day (flip-gate.md step 4).
#
# Non-blocking by design (like api-compat): it INFORMS ("this changed the frozen contract
# without declaring it"); the doctrine is a conscious decision, not prevention.
#
# Usage:  scripts/check_contract_goldens.sh [BASE_REF]   (default BASE_REF = origin/main)
#         scripts/check_contract_goldens.sh --self-test  (prove the decision table)
set -euo pipefail

GOLDENS=("tests/golden/layout/corpus.txt" "tests/golden/behavior/corpus.out")
MANIFEST="src/manifest.rs"

# Pure decision function — the whole policy, so --self-test can exercise it without git.
#   args: <contract> <golden_changed 0|1> <contract_bumped 0|1>
#   echoes a verdict word (ok|inert|break) and returns 0 for pass, 1 for a break.
decide() {
  local contract="$1" golden_changed="$2" contract_bumped="$3"
  if [ "$contract" -eq 0 ]; then echo "inert"; return 0; fi         # pre-freeze: free
  if [ "$golden_changed" -eq 0 ]; then echo "ok"; return 0; fi      # nothing moved
  if [ "$contract_bumped" -eq 1 ]; then echo "ok"; return 0; fi     # declared break
  echo "break"; return 1                                            # silent break
}

if [ "${1:-}" = "--self-test" ]; then
  fail=0
  check() { # <expect_verdict> <expect_rc> <contract> <golden_changed> <bump>
    local got rc; got=$(decide "$3" "$4" "$5") && rc=0 || rc=$?
    if [ "$got" != "$1" ] || [ "$rc" -ne "$2" ]; then
      echo "SELF-TEST FAIL: decide($3,$4,$5) = ($got,$rc), expected ($1,$2)"; fail=1
    fi
  }
  check inert 0 0 1 0   # contract 0 → always inert, even a golden change
  check inert 0 0 0 0
  check ok    0 1 0 0   # post-flip, golden unchanged → ok
  check ok    0 1 1 1   # post-flip, golden changed WITH a bump → ok
  check break 1 1 1 0   # post-flip, golden changed, NO bump → break
  check break 1 2 1 0   # any contract >= 1 behaves the same
  if [ "$fail" -eq 0 ]; then echo "check_contract_goldens self-test: PASS"; exit 0; fi
  echo "check_contract_goldens self-test: FAIL"; exit 1
fi

BASE="${1:-origin/main}"

# `sed -n`, not `grep -oP`: BSD grep (macOS) has no `-P`, and this gate was red there.
contract=$(sed -nE 's/^pub const CONTRACT_VERSION: u32 = ([0-9]+);.*/\1/p' "$MANIFEST" | head -1)
[ -n "$contract" ] || exit 1

# Did any frozen golden move vs the base?  (A real change always rewrites the golden;
# an additive corpus entry also rewrites it — conservative/fail-closed, flags for a human.)
golden_changed=0
moved=()
for g in "${GOLDENS[@]}"; do
  if ! git diff --quiet "$BASE" -- "$g"; then golden_changed=1; moved+=("$g"); fi
done

# Did CONTRACT_VERSION change in this diff?
if git diff "$BASE" -- "$MANIFEST" | grep -qE '^\+[^+].*CONTRACT_VERSION: u32 ='; then
  contract_bumped=1
else
  contract_bumped=0
fi

verdict=$(decide "$contract" "$golden_changed" "$contract_bumped") && rc=0 || rc=$?
case "$verdict" in
  inert) echo "contract-goldens gate: INERT (CONTRACT_VERSION=$contract, pre-freeze)"; ;;
  ok)    echo "contract-goldens gate: ok (contract=$contract, golden_changed=$golden_changed, bumped=$contract_bumped)"; ;;
  break)
    cat >&2 <<MSG
contract-goldens gate: FAIL — a frozen contract golden changed but CONTRACT_VERSION did not bump.

Changed: ${moved[*]}
A change to the layout golden is a silent PERSISTENCE break; a change to the behaviour
golden is a silent SEMANTICS break. Post-freeze (CONTRACT_VERSION=$contract) that may land
only as a DECLARED break. Either:
  - bump CONTRACT_VERSION in $MANIFEST (and, for a layout change, set LAYOUT_CONTRACT to
    the new value in tests/layout_golden.rs), or
  - if the change is contract-neutral, revert the golden re-bless.
MSG
    ;;
esac
exit "$rc"
