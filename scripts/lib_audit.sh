#!/usr/bin/env bash
# lib_audit.sh — @PLN20 library health audit.
#
# Reports the true health of every loft-lang/loft-libs-* package against the
# CURRENT loft (this working tree), across four questions:
#
#   Q1-Q3  branch & PR hygiene — every branch is merged (deletable), tracked by
#          an open PR, or UNTRACKED unknown work (a gate failure).
#   Q4     correctness — interpreter + native tests vs current loft.
#   Q5     registry currency — wraps check_registry_coverage.sh.
#   +      advisory @C86 write-through lint (scripts/c86_lint.py) — REPORT-only,
#          never gates; runs for every package incl. those with no tests/.
#
# THE INVARIANT: the report never says OK for something broken, never FAIL for
# something fine, and never silently omits a target.  A verification tool that
# lies is worse than none.
#
# THE GATE — the audit fully succeeds (exit 0) iff ALL hold:
#   - every intended target was reached (no partial run),
#   - every correctness verdict is OK (no FAIL / DEGRADED / BLIND),
#   - every branch is RESOLVED: merged, or tracked by an open PR
#     (UNTRACKED unknown work, or a state we could not determine, FAILS),
#   Registry findings + advisory lints + EMPTY/SKIP are REPORTED, not gated
#   (publishing needs a signing key the audit never holds).
#
# SAFETY ASYMMETRY: a false "merged" deletes unmerged work; a false "not-merged"
# only leaves clutter.  Branch deletion is therefore KEEP-by-default and only
# happens under --prune, re-verified at delete time, logging the deleted SHA
# (recoverable from the merged-PR ref).
#
# Usage:
#   scripts/lib_audit.sh                       # dry-run, fresh clones, interp+native
#   scripts/lib_audit.sh --local               # use ~/workspace/loft-libs-* clones (fast)
#   scripts/lib_audit.sh --src net=/path/net   # explicit clone path for one repo
#   scripts/lib_audit.sh --repo "net core"     # limit to these repos
#   scripts/lib_audit.sh --prune               # actually delete the merged branches
#   scripts/lib_audit.sh --no-native           # interpreter tier only (fast)
#   scripts/lib_audit.sh --full                # + cargo-test / --deps / advisory lints
#   scripts/lib_audit.sh --clean-cache         # nuke ~/.loft/build-cache before native
#
# Exit: 0 = green; 1 = a gate failure exists; 2 = could not run (env/network).
#
# Builds on (see @PLN20 § Current tooling): doctor.sh (env precondition),
# check_registry_coverage.sh (Q5), and supersedes verify_external_libs.sh.
set -uo pipefail

ORG=loft-lang
ROOT="$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)"
LOFT="$ROOT/target/release/loft"
CACHE="$ROOT/target/lib-audit"

DRY=1; LOCAL=0; RUN_NATIVE=1; FULL=0; CLEAN_CACHE=0
REPOS=""; declare -A SRC=()
while [ $# -gt 0 ]; do
  case "$1" in
    --prune)        DRY=0 ;;
    --dry-run)      DRY=1 ;;
    --local)        LOCAL=1 ;;
    --src)          SRC["${2%%=*}"]="${2#*=}"; shift ;;
    --repo)         REPOS="$2"; shift ;;
    --no-native)    RUN_NATIVE=0 ;;
    --full)         FULL=1 ;;
    --clean-cache)  CLEAN_CACHE=1 ;;
    -h|--help)      sed -n '2,40p' "$0"; exit 0 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
  shift
done

# ── output helpers ──────────────────────────────────────────────────────────
c_red=$'\e[31m'; c_grn=$'\e[32m'; c_yel=$'\e[33m'; c_dim=$'\e[2m'; c_off=$'\e[0m'
[ -t 1 ] || { c_red=; c_grn=; c_yel=; c_dim=; c_off=; }
say()  { printf '%s\n' "$*"; }
head() { printf '\n%s== %s ==%s\n' "$c_dim" "$*" "$c_off"; }

# Accumulators.  GATE_FAILS is the single source of the exit code; REPORTS are
# surfaced but never gate.  INTENDED/REACHED enforce "no partial run" (C2).
declare -a GATE_FAILS=() REPORTS=() PRUNABLE=() ROWS=()
declare -a INTENDED=() REACHED=() C86_FINDINGS=()
gate()   { GATE_FAILS+=("$1"); }
report() { REPORTS+=("$1"); }

# ── preconditions ─────────────────────────────────────────────────────────────
command -v git >/dev/null  || { echo "missing: git" >&2; exit 2; }
command -v gh  >/dev/null  || { echo "missing: gh"  >&2; exit 2; }
gh auth status >/dev/null 2>&1 || { echo "gh not authenticated — cannot determine branch/PR state, so cannot claim 'no unknown work'." >&2; exit 2; }
# Native toolchain precondition (the SKIP-ENV gate; doctor.sh has the full probe).
NATIVE_ENV_OK=1
if [ "$RUN_NATIVE" = 1 ]; then
  if ! command -v rustc >/dev/null || ! command -v cargo >/dev/null; then
    NATIVE_ENV_OK=0
    report "native tier: SKIP-ENV (no rustc/cargo — see scripts/doctor.sh)"
  fi
fi

# ── build current loft + stamp provenance ─────────────────────────────────────
head "building current loft from $(git -C "$ROOT" rev-parse --abbrev-ref HEAD)"
if ! cargo build --release --bin loft --manifest-path "$ROOT/Cargo.toml" 2>&1 | tail -2; then
  echo "FATAL: loft build failed" >&2; exit 2
fi
LOFT_SHA=$(git -C "$ROOT" rev-parse --short HEAD)
git -C "$ROOT" diff --quiet 2>/dev/null || LOFT_SHA="$LOFT_SHA-dirty"
[ "$CLEAN_CACHE" = 1 ] && rm -rf "$HOME/.loft/build-cache"

# ── discover target repos ─────────────────────────────────────────────────────
if [ -z "$REPOS" ]; then
  REPOS=$(gh api "orgs/$ORG/repos" --paginate --jq '.[].name' 2>/dev/null \
            | sed -n 's/^loft-libs-//p' | sort) \
    || { echo "could not list $ORG repos (network) — cannot enumerate the audit's targets." >&2; exit 2; }
fi
[ -n "$REPOS" ] || { echo "no loft-libs-* repos found." >&2; exit 2; }

# ── per-repo clone resolution (--src > --local > fresh) ───────────────────────
resolve_clone() {  # repo  -> echoes a path with all origin/* refs, or "" on failure
  local repo="$1" path=""
  if [ -n "${SRC[$repo]:-}" ]; then path="${SRC[$repo]}"
  elif [ "$LOCAL" = 1 ] && [ -d "$HOME/workspace/loft-libs-$repo/.git" ]; then path="$HOME/workspace/loft-libs-$repo"
  else
    path="$CACHE/$repo"
    if [ ! -d "$path/.git" ]; then
      rm -rf "$path"; mkdir -p "$CACHE"
      git clone --quiet --filter=blob:none "https://github.com/$ORG/loft-libs-$repo.git" "$path" 2>/dev/null || { echo ""; return; }
    fi
  fi
  git -C "$path" fetch --quiet --prune origin 2>/dev/null || { echo ""; return; }
  echo "$path"
}

default_branch() {  # clone -> echoes default branch (origin/HEAD), fallback main
  local d; d=$(git -C "$1" symbolic-ref --quiet refs/remotes/origin/HEAD 2>/dev/null | sed 's@^refs/remotes/origin/@@')
  [ -z "$d" ] && { git -C "$1" remote set-head origin --auto >/dev/null 2>&1; d=$(git -C "$1" symbolic-ref --quiet refs/remotes/origin/HEAD 2>/dev/null | sed 's@^refs/remotes/origin/@@'); }
  echo "${d:-main}"
}

# ── Q1-Q3 : the 3-signal merged test (KEEP by default) ────────────────────────
classify_branch() {  # repo clone def br -> "VERDICT|detail"
  local repo="$1" clone="$2" def="$3" br="$4"
  local prm pro
  prm=$(gh pr list -R "$ORG/loft-libs-$repo" --head "$br" --state merged --json number --jq '.[0].number' 2>/dev/null) || { echo "INDET|gh merged-query failed"; return; }
  pro=$(gh pr list -R "$ORG/loft-libs-$repo" --head "$br" --state open   --json number --jq '.[0].number' 2>/dev/null) || { echo "INDET|gh open-query failed"; return; }
  local anc=0; git -C "$clone" merge-base --is-ancestor "origin/$br" "origin/$def" 2>/dev/null && anc=1
  local ce=0 mt; mt=$(git -C "$clone" merge-tree --write-tree "origin/$def" "origin/$br" 2>/dev/null) \
    && [ "$mt" = "$(git -C "$clone" rev-parse "origin/$def^{tree}")" ] && ce=1
  if [ -n "$prm" ] || [ "$anc" = 1 ] || [ "$ce" = 1 ]; then
    local why=""; [ -n "$prm" ] && why="PR#$prm "; [ "$anc" = 1 ] && why+="ancestor "; [ "$ce" = 1 ] && why+="content-in-main"
    echo "MERGED|${why}"
  elif [ -n "$pro" ]; then echo "TRACKED|open PR#$pro"
  else echo "UNTRACKED|unmerged, no PR"; fi
}

# ── Q4 : per-package correctness, with honest verdicts ────────────────────────
# OK / FAIL / DEGRADED (native silently ran interp) / EMPTY (0 tests) / SKIP-ENV
run_tests() {  # mode("interpret"|"native") pkgdir -> "VERDICT|note"
  local mode="$1" pkg="$2" out code
  out=$(cd "$pkg" && "$LOFT" "--$mode" test 2>&1); code=$?
  if [ "$code" -ne 0 ]; then
    # SKIP-ENV ONLY for a genuinely-absent toolchain — never for a real compile
    # error.  The markers are narrow on purpose: 'can'\''t find crate FOR' is the
    # E0463 toolchain phrasing, not 'in crate X' (a normal E0425), so a library
    # build-script error stays a FAIL and keeps gating (it must not hide here).
    if echo "$out" | grep -qiE "rustc.*command not found|can't find crate for|error: linker|LNK1181|libloft\.rlib"; then
      echo "SKIP-ENV|toolchain absent"; return
    fi
    echo "FAIL|$(echo "$out" | grep -m1 -iE 'error\[|error:|panic|fail' | sed 's/^ *//' | cut -c1-70)"; return
  fi
  if echo "$out" | grep -qiE 'no test|0 (tests|passed)|nothing to run'; then echo "EMPTY|no tests ran"; return; fi
  # DEGRADED: exit-0 native run that warned about a dependency dropping to interp.
  if [ "$mode" = native ] && echo "$out" | grep -qiE 'fall(ing|s|en)? ?back|interpret(ing)? instead|native .* (failed|disabled|skipped)|could not (build|compile) .* native'; then
    echo "DEGRADED|native fell back to interpreter"; return
  fi
  echo "OK|"
}

# ── per-repo pass ─────────────────────────────────────────────────────────────
for repo in $REPOS; do
  head "loft-libs-$repo"
  clone=$(resolve_clone "$repo")
  if [ -z "$clone" ]; then
    INTENDED+=("loft-libs-$repo"); gate "loft-libs-$repo: could not clone/fetch (target unreached)"
    say "  ${c_red}UNREACHED${c_off} — clone/fetch failed"; continue
  fi
  INTENDED+=("loft-libs-$repo"); REACHED+=("loft-libs-$repo")
  def=$(default_branch "$clone")
  repo_sha=$(git -C "$clone" rev-parse --short "origin/$def")
  say "  ${c_dim}clone=$clone  default=$def@$repo_sha${c_off}"

  # Q1-Q3 branches
  for br in $(git -C "$clone" for-each-ref --format='%(refname:strip=3)' refs/remotes/origin | grep -vxE "$def|HEAD"); do
    IFS='|' read -r verdict detail <<<"$(classify_branch "$repo" "$clone" "$def" "$br")"
    case "$verdict" in
      MERGED)
        PRUNABLE+=("$repo/$br")
        if [ "$DRY" = 0 ]; then
          # re-verify at delete time, then delete; log the recoverable SHA.
          IFS='|' read -r v2 _ <<<"$(classify_branch "$repo" "$clone" "$def" "$br")"
          sha=$(git -C "$clone" rev-parse "origin/$br")
          if [ "$v2" = MERGED ]; then
            if git -C "$clone" push origin --delete "$br" >/dev/null 2>&1; then
              say "  ${c_grn}DELETED${c_off} $br ${c_dim}($detail; was $sha — restore: git push origin $sha:refs/heads/$br)${c_off}"
            else say "  ${c_yel}delete failed${c_off} $br"; fi
          else say "  ${c_yel}skip delete${c_off} $br (state changed since scan: $v2)"; fi
        else
          say "  ${c_grn}MERGED${c_off}    $br ${c_dim}($detail) — prunable${c_off}"
        fi ;;
      TRACKED)   say "  ${c_yel}TRACKED${c_off}   $br ${c_dim}($detail)${c_off}"; report "loft-libs-$repo/$br: in review ($detail)" ;;
      UNTRACKED) say "  ${c_red}UNTRACKED${c_off} $br ${c_dim}($detail)${c_off}"; gate "loft-libs-$repo/$br: UNKNOWN work — unmerged, no PR" ;;
      INDET)     say "  ${c_red}INDETERMINATE${c_off} $br ${c_dim}($detail)${c_off}"; gate "loft-libs-$repo/$br: branch state indeterminate ($detail)" ;;
    esac
  done

  # Q4 correctness — in a throwaway worktree at origin/$def (never disturbs the
  # working checkout, and tests the exact ship ref).
  wt="$CACHE/wt-$repo"; rm -rf "$wt"
  if ! git -C "$clone" worktree add -q --detach "$wt" "origin/$def" 2>/dev/null; then
    gate "loft-libs-$repo: could not create worktree (correctness unreached)"; say "  ${c_red}worktree failed${c_off}"; continue
  fi
  for toml in "$wt"/*/loft.toml; do
    [ -e "$toml" ] || continue
    p=$(dirname "$toml"); name=$(basename "$p")
    # @C86 write-through lint — REPORT-only, never gates.  Runs BEFORE the no-tests
    # skip so EMPTY-suite packages (their only @C86 guard) are still scanned.
    if command -v python3 >/dev/null 2>&1; then
      while IFS= read -r cl; do
        [ -n "$cl" ] && C86_FINDINGS+=("$repo/${cl#"$wt"/}")
      done < <(python3 "$ROOT/scripts/c86_lint.py" "$p" 2>/dev/null)
    fi
    [ -d "$p/tests" ] || { ROWS+=("$repo/$name|interp -|native -|no tests/"); continue; }
    # INTENDED is claimed at DISCOVERY; REACHED only once a verdict actually exists.
    # These used to be pushed together on one line, which made the completeness check
    # below vacuous — a package was "reached" before its tests ran, so the A3 blind
    # spot the check exists to catch could not be detected.
    INTENDED+=("loft-libs-$repo/$name")
    IFS='|' read -r iv inote <<<"$(run_tests interpret "$p")"
    nv="-"; nnote=""
    if [ "$RUN_NATIVE" = 1 ] && [ "$NATIVE_ENV_OK" = 1 ]; then
      IFS='|' read -r nv nnote <<<"$(run_tests native "$p")"
    elif [ "$RUN_NATIVE" = 1 ]; then nv="SKIP-ENV"; fi
    # A3 · BLIND — the package was FOUND but produced no verdict (run_tests died, the
    # subshell was killed, a timeout).  The design calls this "designed to be
    # impossible"; the point of emitting it is that "impossible" must still be
    # OBSERVABLE if it happens, because the alternative is an empty verdict reading as
    # a silent pass — the exact false-clean this taxonomy exists to prevent.
    if [ -z "$iv" ]; then
      iv="BLIND"; inote="found but unrun — no verdict produced"
    fi
    REACHED+=("loft-libs-$repo/$name")
    note="$inote"; [ -n "$nnote" ] && [ "$nnote" != "$inote" ] && note="${note:+$note; }$nnote"
    ROWS+=("$repo/$name|interp $iv|native $nv|$note")
    case "$iv" in
      FAIL)  gate "loft-libs-$repo/$name [interpret]: FAIL — $inote" ;;
      BLIND) gate "loft-libs-$repo/$name [interpret]: BLIND — $inote" ;;
    esac
    case "$nv" in
      FAIL)     gate "loft-libs-$repo/$name [native]: FAIL — $nnote" ;;
      DEGRADED) gate "loft-libs-$repo/$name [native]: DEGRADED — $nnote" ;;
      BLIND)    gate "loft-libs-$repo/$name [native]: BLIND — $nnote" ;;
    esac
  done
  git -C "$clone" worktree remove --force "$wt" 2>/dev/null
done

# ── Q5 registry currency (REPORT, never gates) ────────────────────────────────
head "Q5 — registry currency"
if reg=$("$ROOT/scripts/check_registry_coverage.sh" 2>&1); then
  echo "$reg" | grep -E 'missing|stale|orphan' | while read -r l; do say "  $l"; done
  echo "$reg" | grep -qE 'missing|stale' && report "registry: bumped-but-unpublished versions exist (run registry_maintain.sh)"
  echo "$reg" | grep -qE 'missing|stale|orphan' || say "  registry current"
else
  report "registry: coverage check unavailable (network) — currency indeterminate"
fi

# ── report ────────────────────────────────────────────────────────────────────
head "correctness matrix  (loft @ $LOFT_SHA)"
[ ${#ROWS[@]} -eq 0 ] && say "  (none)"
for r in "${ROWS[@]}"; do IFS='|' read -r a b c d <<<"$r"; printf '  %-26s %-14s %-16s %s\n' "$a" "$b" "$c" "${c_dim}$d${c_off}"; done

[ ${#PRUNABLE[@]} -gt 0 ] && { head "prunable (merged) branches"; for x in "${PRUNABLE[@]}"; do say "  $x"; done; [ "$DRY" = 1 ] && say "  ${c_dim}(re-run with --prune to delete)${c_off}"; }

# @C86 write-through lint — advisory (REPORT), never gates.  A one-liner also lands
# in "reported (non-gating)" via report() so it shows in the summary tally.
[ ${#C86_FINDINGS[@]} -gt 0 ] && {
  head "C86 write-through lint (advisory — plain field bind may need &)"
  for x in "${C86_FINDINGS[@]}"; do say "  ${c_yel}?${c_off} $x"; done
  report "C86 lint: ${#C86_FINDINGS[@]} write-through bind(s) may need \`&\` — see § C86 write-through lint (not gated)"
}

[ ${#REPORTS[@]} -gt 0 ] && { head "reported (non-gating)"; for x in "${REPORTS[@]}"; do say "  - $x"; done; }

# Completeness (C2): any intended target not reached is itself a failure.
for t in "${INTENDED[@]}"; do
  printf '%s\n' "${REACHED[@]}" | grep -qxF "$t" || gate "$t: intended but NOT reached"
done

head "verdict  (loft @ $LOFT_SHA, $(date -u +%Y-%m-%dT%H:%MZ))"
if [ ${#GATE_FAILS[@]} -eq 0 ]; then
  say "  ${c_grn}GREEN${c_off} — every target reached, every check OK, every branch resolved."
  exit 0
else
  say "  ${c_red}NOT GREEN — ${#GATE_FAILS[@]} gate failure(s):${c_off}"
  for x in "${GATE_FAILS[@]}"; do say "    ${c_red}✗${c_off} $x"; done
  exit 1
fi
