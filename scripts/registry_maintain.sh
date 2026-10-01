#!/usr/bin/env bash
# One-command registry maintenance: see what needs publishing, OK once,
# sign once — and the registry is current again.
#
#   scripts/registry_maintain.sh [--only P[,P...]] [--dry-run] [--yes] [--key <file>] [--registry-dir <dir>]
#
# The script gathers BOTH populations into one worklist:
#
#   own libs     — every `loft-lang/loft-libs-*` library that is missing from
#                  or newer than the registry index.  These the script can fix
#                  end-to-end: `loft package` → tag + GitHub release →
#                  `loft publish` → index entry.
#   foreign libs — third-party state the maintainer can only review:
#                  open submission PRs on `loft-lang/registry` (merged when
#                  their validation CI is green), and registry packages whose
#                  upstream repo has newer release tags than the index
#                  (informational — publishing is the author's move).
#
# Then ONE confirmation, the publishes + green-PR merges run, the index is
# updated, and the maintainer signs once (`loft-keygen sign`, laptop-only key —
# PKG_REGISTRY.md § Why laptop signing).  Without a key file the script stops
# after staging the commit and prints the two remaining commands.
#
# To review-then-sign a SINGLE registry PR (shows the diff + each library
# release + notes, re-checks every tarball sha256, then signs/commits/pushes),
# use `scripts/registry-sign.sh --pr <N>` instead — the per-PR signing path.
#
#   --only P[,P...]  publish EXACTLY these own libraries and nothing else: the
#                    worklist is filtered to them (before the pre-flight, so no
#                    time is spent on libraries nobody asked for), foreign PRs and
#                    staged submissions are reported but NOT acted on, and the
#                    signer is handed `--expect <name>@<ver>` for each one — so a
#                    run that somehow changed anything else REFUSES instead of
#                    signing it.  A name with nothing to publish is an error.
#                    This is the agent-safe form; it needs no `--yes`.
#   --dry-run        gather + print the worklist, change nothing
#   --yes            skip the confirmation prompt
#   --key <file>     Ed25519 private key (default: $LOFT_REGISTRY_KEY, else
#                    ~/.loft/trust-root/registry-signing-key.bin)
#   --registry-dir   existing loft-lang/registry checkout (default: temp clone)
set -euo pipefail

ORG=loft-lang
DRY=0
YES=0
KEY="${LOFT_REGISTRY_KEY:-$HOME/.loft/trust-root/registry-signing-key.bin}"
REG_DIR="${LOFT_REGISTRY_DIR:-}"
ONLY=""
while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) DRY=1 ;;
        --yes) YES=1 ;;
        --key) KEY="$2"; shift ;;
        --only) ONLY="$2"; shift ;;
        --registry-dir) REG_DIR="$2"; shift ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
    shift
done

here=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

for dep in git gh python3 curl; do
    command -v "$dep" > /dev/null || { echo "missing dependency: $dep" >&2; exit 2; }
done
gh auth status > /dev/null 2>&1 || { echo "gh is not authenticated (gh auth login)" >&2; exit 2; }

# loft + loft-keygen binaries (release build; the registry feature is default).
LOFT="$here/target/release/loft"
KEYGEN="$here/target/release/loft-keygen"
if [ ! -x "$LOFT" ] || [ ! -x "$KEYGEN" ]; then
    echo "building loft + loft-keygen (release) ..."
    (cd "$here" && cargo build --release --bin loft --bin loft-keygen --features registry)
fi

# sha256 of a file via python3 (already a hard dep; avoids sha256sum-vs-shasum
# portability differences across the laptops that sign).
sha256_of() { python3 -c 'import hashlib,sys;print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$1"; }

# ── gather ────────────────────────────────────────────────────────────────────

echo "fetching registry index ..."
curl -fsSL "https://raw.githubusercontent.com/$ORG/registry/main/index.json" -o "$tmp/index.json"

echo "cloning family repos ..."
repos=$(gh api "orgs/$ORG/repos?per_page=100" --jq '.[].name' | grep '^loft-libs-' || true)
: > "$tmp/own.tsv" # <name>\t<version>\t<repo>\t<libdir>
for r in $repos; do
    git clone --quiet --depth 1 "https://github.com/$ORG/$r.git" "$tmp/$r"
    for t in "$tmp/$r"/*/loft.toml; do
        [ -f "$t" ] || continue
        name=$(sed -n 's/^name *= *"\(.*\)"/\1/p' "$t" | head -1)
        ver=$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$t" | head -1)
        [ -n "$name" ] && printf '%s\t%s\t%s\t%s\n' "$name" "$ver" "$r" "$(dirname "$t")" >> "$tmp/own.tsv"
    done
done

# Own-lib worklist: missing/stale vs the index (same rule as
# check_registry_coverage.sh, but keeping the libdir for the publish step).
python3 - "$tmp" <<'EOF'
import json, sys

tmp = sys.argv[1]
index = json.load(open(f"{tmp}/index.json"))["packages"]


def semver(v):
    return tuple(int(p) if p.isdigit() else 0 for p in v.split("."))


work = []
for line in open(f"{tmp}/own.tsv"):
    name, ver, repo, libdir = line.rstrip("\n").split("\t")
    if name not in index:
        work.append((name, ver, repo, libdir, "missing"))
    elif semver(ver) > semver(max(index[name]["versions"], key=semver)):
        work.append((name, ver, repo, libdir, "stale"))
with open(f"{tmp}/work.tsv", "w") as f:
    for row in sorted(work):
        f.write("\t".join(row) + "\n")

# Foreign packages: index entries whose release URL lives outside the org.
foreign = []
for name, pkg in sorted(index.items()):
    newest = max(pkg["versions"], key=semver)
    url = pkg["versions"][newest].get("url", "")
    parts = url.removeprefix("https://github.com/").split("/")
    if len(parts) >= 2 and parts[0] != "loft-lang":
        foreign.append(f"{name}\t{newest}\t{parts[0]}/{parts[1]}")
open(f"{tmp}/foreign.tsv", "w").write("\n".join(foreign) + ("\n" if foreign else ""))
EOF

# ── --only: this run publishes exactly these, and nothing else ────────────────
# Applied HERE, before the pre-flight below, for two reasons.  The pre-flight runs
# each worklist lib's whole suite and downloads its release tarball, so filtering
# after it would spend minutes on libraries nobody asked for — and a lib blocked
# there prints a scary failure for work that was never in scope.
#
# A name that is not in the worklist is an ERROR, not a quiet no-op: `--only
# drawing` on a registry that already has drawing means one of us is wrong about
# what needs publishing, and finishing silently is the answer that hides it.
if [ -n "$ONLY" ]; then
    : > "$tmp/only_work.tsv"
    miss=""
    for want in ${ONLY//,/ }; do
        hits=$(awk -F'\t' -v n="$want" '$1 == n' "$tmp/work.tsv")
        if [ -z "$hits" ]; then miss="$miss $want"; else printf '%s\n' "$hits" >> "$tmp/only_work.tsv"; fi
    done
    if [ -n "$miss" ]; then
        echo "!! --only names $(echo "$miss" | wc -w) package(s) with nothing to publish:$miss" >&2
        echo "   Each is either absent from $ORG/loft-libs-*, or its loft.toml version is" >&2
        echo "   already the newest in the registry index.  Bump the version, or drop the name." >&2
        exit 2
    fi
    sort -u "$tmp/only_work.tsv" > "$tmp/work.tsv"
    echo "--only: $(wc -l < "$tmp/work.tsv") own lib(s); foreign PRs and staged submissions are"
    echo "        reported below but NOT acted on — they would change the index this run is"
    echo "        about to bind its signature to."
fi


# Foreign upstream drift: does the author's repo have a newer release tag than
# the registry?  Informational only — the author publishes, not us.
: > "$tmp/foreign_drift.txt"
while IFS=$'\t' read -r name newest frepo; do
    latest=$(gh api "repos/$frepo/releases?per_page=100" \
        --jq ".[].tag_name | select(startswith(\"$name-v\"))" 2> /dev/null |
        sed "s/^$name-v//" | sort -V | tail -1 || true)
    if [ -n "$latest" ] && [ "$(printf '%s\n%s\n' "$newest" "$latest" | sort -V | tail -1)" != "$newest" ]; then
        echo "  $name: registry $newest, upstream $frepo has $latest — nudge the author" >> "$tmp/foreign_drift.txt"
    fi
done < "$tmp/foreign.tsv"

# Foreign submissions: open PRs on the registry repo + their CI verdict.
gh pr list -R "$ORG/registry" --state open \
    --json number,title,author,statusCheckRollup \
    --jq '.[] | [.number, .author.login, ([.statusCheckRollup[]?.conclusion] | if length == 0 then "pending" elif all(. == "SUCCESS") then "green" else "red" end), .title] | @tsv' \
    > "$tmp/prs.tsv" 2> /dev/null || : > "$tmp/prs.tsv"

# Branch hygiene: we already enumerate every family repo above, so surface their
# stray branches here too — unmerged with no open PR (orphaned work to PR-or-delete)
# and merged with no PR (prunable).  We shallow-clone only main, so this lists
# branches via the API rather than checking them out.  Informational — never
# blocks the publish/sign; it just nudges cleanup during a maintenance run.
: > "$tmp/orphan_branches.txt"
: > "$tmp/prunable_branches.txt"
for r in $repos; do
    pr_heads=$(gh pr list -R "$ORG/$r" --state open --json headRefName --jq '.[].headRefName' 2> /dev/null || true)
    while read -r br; do
        [ -n "$br" ] && [ "$br" != "main" ] || continue
        printf '%s\n' "$pr_heads" | grep -qxF "$br" && continue   # has an open PR — in review
        read -r ahead last <<< "$(gh api "repos/$ORG/$r/compare/main...$br" \
            --jq '"\(.ahead_by) \(.commits[-1].commit.committer.date // "?")"' 2> /dev/null || echo '0 ?')"
        # A squash-merged branch is "ahead" (its commits aren't individually on
        # main) yet its work DID land — detect that via a merged PR so it's
        # classed prunable, not orphaned.
        merged=$(gh pr list -R "$ORG/$r" --state merged --head "$br" --json number --jq 'length' 2> /dev/null || echo 0)
        if [ "${ahead:-0}" = 0 ] || [ "${merged:-0}" != 0 ]; then
            echo "  $r/$br" >> "$tmp/prunable_branches.txt"
        else
            echo "  $r/$br ($ahead ahead, last ${last%%T*})" >> "$tmp/orphan_branches.txt"
        fi
    done < <(gh api "repos/$ORG/$r/branches?per_page=100" --jq '.[].name' 2> /dev/null || true)
done

# ── pre-flight: stale-release gate (runs BEFORE the worklist/prompt + in dry-run) ─
# A worklist lib whose source on main moved past its `<name>-v<ver>` tag with no
# version bump packages to bytes the EXISTING release tarball lacks — that fails
# the sign-time integrity check.  Catch it here, before anything is shown or
# confirmed, so the dry-run + the publish prompt reflect only what can actually be
# signed, and each blocked lib is reported with what is wrong + how to fix it.
# Only libs whose `<name>-v<ver>` release ALREADY exists can be stale; a missing
# release is created from main by the publish loop, so it always matches.
: > "$tmp/blocked.txt"
: > "$tmp/work_ok.tsv"
while IFS=$'\t' read -r name ver repo libdir _why; do
    [ -n "$name" ] || continue
    tag="$name-v$ver"
    # V2/V3 gate — the lib must parse/type/test against THIS loft before it can be
    # published + signed (the same gate library-ci + revalidate-libs run; catches a
    # lib a language change broke, e.g. C95, before it reaches the signed index).
    if [ -d "$libdir/tests" ] \
       && ! ( cd "$libdir" && LOFT_TIMEOUT=180 "$LOFT" --interpret --tests tests ) > "$tmp/gate_$name.log" 2>&1; then
        {
          echo "  ✗ $name $ver — GATE FAILED: does not pass its tests against this loft."
          echo "      $(grep -iE 'error|fail' "$tmp/gate_$name.log" | head -1)"
          echo "      Fix the library (or reconsider the language change — pre-freeze the stdlib may only grow additively), then re-run."
        } >> "$tmp/blocked.txt"
        continue
    fi
    if ! gh release view "$tag" -R "$ORG/$repo" > /dev/null 2>&1; then
        printf '%s\t%s\t%s\t%s\t%s\n' "$name" "$ver" "$repo" "$libdir" "$_why" >> "$tmp/work_ok.tsv"
        continue
    fi
    # Build main's tarball + fetch the published one; compare.  On any error
    # (can't package / can't download) keep the lib — the publish loop reports it.
    #
    # `--tarball-only`: this comparison is mechanical — it asks whether main's
    # bytes match the release, which is true or false regardless of what the
    # package declares.  Without it a library that has not yet entered the
    # compatibility contract would report as a STALE RELEASE, which is a
    # different and untrue thing.
    if "$LOFT" package --tarball-only "$libdir" > /dev/null 2>&1 && [ -f "$libdir/$name-$ver.tar.gz" ] \
       && gh release download "$tag" -R "$ORG/$repo" --pattern "$name-$ver.tar.gz" \
              --dir "$tmp/dl_$name" --clobber 2>/dev/null \
       && [ -f "$tmp/dl_$name/$name-$ver.tar.gz" ] \
       && [ "$(sha256_of "$libdir/$name-$ver.tar.gz")" != "$(sha256_of "$tmp/dl_$name/$name-$ver.tar.gz")" ]; then
        {
          echo "  ✗ $name $ver — STALE RELEASE: published $name-$ver.tar.gz ≠ \`loft package\` of main."
          echo "      Why: main has commits past tag $tag with no version bump, so its source no"
          echo "           longer matches the $tag release artifact (would fail the sign-time check)."
          echo "      Fix: bump $name in $repo's loft.toml + commit, then re-run (cuts a new"
          echo "           tag/release); OR if main is releasable unchanged, delete the stale $tag"
          echo "           release so it is re-cut from current main."
        } >> "$tmp/blocked.txt"
    else
        printf '%s\t%s\t%s\t%s\t%s\n' "$name" "$ver" "$repo" "$libdir" "$_why" >> "$tmp/work_ok.tsv"
    fi
done < "$tmp/work.tsv"
mv "$tmp/work_ok.tsv" "$tmp/work.tsv"

# ── the worklist ──────────────────────────────────────────────────────────────

n_own=$(wc -l < "$tmp/work.tsv")
n_prs=$(wc -l < "$tmp/prs.tsv")
n_blocked=$(grep -c '✗' "$tmp/blocked.txt" 2>/dev/null || true); n_blocked=${n_blocked:-0}
# C96 key-absent path: versions STAGED under submissions/ (via a PR that does NOT
# touch index.json).  Peek at the count before cloning so a submissions-only run
# still proceeds; the drain (post-clone) vets + folds each one.
n_subs=$(gh api "repos/$ORG/registry/contents/submissions" \
    --jq '[.[] | select(.name | endswith(".json"))] | length' 2>/dev/null \
    | grep -E '^[0-9]+$' | head -1 || true)   # submissions/ absent (404) → empty
n_subs=${n_subs:-0}
echo
echo "== own libs to publish ($n_own) =="
awk -F'\t' '{ printf "  %s %s (%s, %s)\n", $1, $2, $3, $5 }' "$tmp/work.tsv"
if [ "$n_blocked" != 0 ]; then
    echo "== blocked: stale release — fix + re-run, NOT published this run ($n_blocked) =="
    cat "$tmp/blocked.txt"
fi
echo "== foreign submission PRs on $ORG/registry ($n_prs) =="
awk -F'\t' '{ printf "  #%s by %s [%s] %s\n", $1, $2, $3, $4 }' "$tmp/prs.tsv"
echo "== staged submissions/ to vet + fold ($n_subs) =="
echo "== foreign upstream drift (informational) =="
cat "$tmp/foreign_drift.txt"
if [ -s "$tmp/orphan_branches.txt" ]; then
    echo "== unmerged branches, no open PR (orphaned — PR or delete) =="
    cat "$tmp/orphan_branches.txt"
fi
if [ -s "$tmp/prunable_branches.txt" ]; then
    echo "== merged branches, no open PR (prunable) =="
    cat "$tmp/prunable_branches.txt"
fi
echo

if [ "$n_own" = 0 ] && [ "$n_prs" = 0 ] && [ "$n_subs" = 0 ]; then
    echo "nothing to do to publish — registry is current (see any branch hygiene above)."
    exit 0
fi
if [ "$DRY" = 1 ]; then
    echo "[dry-run] stopping before any change."
    exit 0
fi
if [ "$YES" != 1 ]; then
    n_green=$(awk -F'\t' '$3 == "green"' "$tmp/prs.tsv" | wc -l)
    # No prompt here — registry-sign.sh below is the single human gate (it shows the
    # diff, re-checks each tarball, asks you to confirm/sign, and trust-gates the
    # push).  A second [y/N] is redundant AND a touch-trap (an idle YubiKey touch
    # types an OTP, which a `read` would mis-read).  Use --dry-run to preview.
    echo "publishing $n_own own lib(s) + merging $n_green green PR(s) — registry-sign will review + sign next."
fi

# ── execute ───────────────────────────────────────────────────────────────────

# Merge the green foreign PRs FIRST so the registry pull below includes them.
# RELIABILITY: a green (CI-passed) PR can still have MERGE CONFLICTS if main moved
# since its CI run.  A failed `gh pr merge` must NOT abort the routine — `gh pr
# merge` squash-commits the PR's index.json change to the remote, but the .sig is
# only regenerated by the sign step at the very end; aborting here leaves the
# registry with a changed index.json and a STALE signature (= "signature invalid
# — refusing to load" for every consumer).  So skip the unmergeable PR, record
# it, and carry on to the sign step, which re-signs the resulting index.
SKIPPED_PRS=()
# `--only` binds this run's signature to the named libraries (see the sign step),
# so merging someone else's PR here would put a version in the index that the
# signer then refuses — AFTER the merge already landed on the remote, leaving a
# changed index.json with a stale signature.  Report them, act on none.
[ -n "$ONLY" ] && : > "$tmp/prs.tsv"
while IFS=$'\t' read -r num _author state _title; do
    if [ "$state" != green ]; then
        echo "skipping $ORG/registry#$num ($state — review by hand)"
        continue
    fi
    mergeable=$(gh pr view "$num" -R "$ORG/registry" --json mergeable -q .mergeable 2>/dev/null || echo UNKNOWN)
    if [ "$mergeable" = CONFLICTING ]; then
        echo "⚠ skipping $ORG/registry#$num — merge conflict (main moved since CI); rebase or close it separately"
        SKIPPED_PRS+=("#$num (conflict)")
        continue
    fi
    echo "merging $ORG/registry#$num ..."
    if ! gh pr merge "$num" -R "$ORG/registry" --squash 2>"$tmp/merge_$num.err"; then
        echo "⚠ could not merge $ORG/registry#$num: $(head -1 "$tmp/merge_$num.err" 2>/dev/null); skipping"
        SKIPPED_PRS+=("#$num (merge failed)")
        continue
    fi
done < "$tmp/prs.tsv"

if [ -z "$REG_DIR" ]; then
    REG_DIR="$tmp/registry"
    # SSH first so the sign+push step (registry-sign.sh, given this checkout)
    # pushes with your key without an HTTPS password prompt; fall back to HTTPS.
    git clone --quiet "git@github.com:$ORG/registry.git" "$REG_DIR" 2>/dev/null \
        || git clone --quiet "https://github.com/$ORG/registry.git" "$REG_DIR"
else
    git -C "$REG_DIR" pull --ff-only
fi

# A merged PR may have covered an own-lib finding (an author can submit via PR
# before this script runs) — re-filter the worklist against the post-merge
# index so those libraries are not published twice.
python3 - "$tmp" "$REG_DIR/index.json" <<'EOF'
import json, sys

tmp, index_path = sys.argv[1:3]
index = json.load(open(index_path))["packages"]


def semver(v):
    return tuple(int(p) if p.isdigit() else 0 for p in v.split("."))


keep = []
for line in open(f"{tmp}/work.tsv"):
    name, ver = line.split("\t")[:2]
    if name in index and semver(ver) <= semver(max(index[name]["versions"], key=semver)):
        print(f"  {name} {ver} covered by a just-merged PR — skipping")
        continue
    keep.append(line)
open(f"{tmp}/work.tsv", "w").writelines(keep)
EOF

# sha256 of a file via python3 (already a hard dep; avoids sha256sum-vs-shasum
# portability differences across the laptops that sign).
sha256_of() { python3 -c 'import hashlib,sys;print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$1"; }

# Publish each own lib: tarball → tag + release (created only when absent,
# so re-runs are safe) → `loft publish` emits the verified index entry.
# RELIABILITY: one own-lib that can't publish (missing release asset, package
# error) must not abort the rest — skip it, record it, keep going so the others
# publish and the index still gets signed.
published=(); EXPECTS=()
SKIPPED_LIBS=()
while IFS=$'\t' read -r name ver repo libdir _why; do
    echo "publishing $name $ver from $repo ..."
    tag="$name-v$ver"
    tarball="$libdir/$name-$ver.tar.gz"
    # The registry-admission gate, checked BEFORE the tag and GitHub release
    # exist.  A refusal after those are cut leaves an artifact nobody can
    # register, and deleting a release is exactly the move that lost web 0.2.2.
    if ! ( cd "$libdir" && "$LOFT" compat levels ) > "$tmp/levels_$name.log" 2>&1; then
        echo "  ⚠ $name $ver — no compatibility floor declared; cannot enter the registry."
        sed -n '1,10p' "$tmp/levels_$name.log" | sed 's/^/      /'
        echo "      Measure what it should say:  (cd $libdir && $LOFT compat floor --with-tests)"
        SKIPPED_LIBS+=("$name-$ver"); continue
    fi
    # Step 7 — the RELEASE gate proves the WHOLE claim, not the O(1) sample a PR pays
    # for.  A PR asks "did this change break something"; publishing asks "is everything
    # this package promises actually true", and only the second is a promise to people
    # who cannot ask the package a question.  Overrunning the budget FAILS here rather
    # than publishing a floor that was never checked past its first few releases.
    # Before the tag, for the same reason the admission gate is.
    if ! ( cd "$libdir" && "$LOFT" compat check --full ) > "$tmp/full_$name.log" 2>&1; then
        echo "  ⚠ $name $ver — its declared compatibility claim does not hold."
        sed -n '1,20p' "$tmp/full_$name.log" | sed 's/^/      /'
        SKIPPED_LIBS+=("$name-$ver"); continue
    fi
    if ! "$LOFT" package "$libdir" > "$tmp/pkg_$name.log" 2>&1 || [ ! -f "$tarball" ]; then
        echo "  ⚠ package/tarball failed — skipping $name $ver"
        sed -n '1,12p' "$tmp/pkg_$name.log" | sed 's/^/      /'
        SKIPPED_LIBS+=("$name-$ver"); continue
    fi
    if ! gh release view "$tag" -R "$ORG/$repo" > /dev/null 2>&1; then
        if ! gh release create "$tag" "$tarball" -R "$ORG/$repo" \
            --target "$(git -C "$tmp/$repo" rev-parse HEAD)" --title "$name v$ver" 2>"$tmp/rel_$name.err"; then
            echo "  ⚠ release create failed ($(head -1 "$tmp/rel_$name.err" 2>/dev/null)) — skipping $name $ver"
            SKIPPED_LIBS+=("$name-$ver"); continue
        fi
    fi
    # (Stale-release detection ran in the pre-flight gate before the prompt — a
    # lib reaching this loop either matches its release or had none yet, which the
    # create-from-main step above just produced.  registry-sign re-verifies every
    # tarball as the trust-root backstop.)
    if ! "$LOFT" publish "$libdir" > "$tmp/pub_$name.out" 2>"$tmp/pub_$name.err"; then
        echo "  ⚠ loft publish failed ($(head -1 "$tmp/pub_$name.err" 2>/dev/null)) — skipping $name $ver"; SKIPPED_LIBS+=("$name-$ver"); continue
    fi
    # The pasteable JSON entry is on stdout; `[publish]` status is on stderr (so
    # do NOT merge with 2>&1).  Strip any #-comment / [publish] / blank line so
    # only the `"<ver>": {…}` member remains — wrapping `{…}` + parsing it below
    # choked on those trailing status lines otherwise.
    grep -vE '^#|^\[publish\]|^[[:space:]]*$' "$tmp/pub_$name.out" > "$tmp/entry_$name.json"
    # Description: prefer the OFFICIAL `[package] description` in loft.toml.  When
    # present it is authoritative (refreshed on every publish below); absent, fall
    # back to the README's first real prose line — skipping HTML/license comment
    # blocks (`<!-- … -->`), markdown headings (`# …`), fenced code blocks
    # (``` … ```), and `//` / `<` lines (the old "first non-# line" grabbed `<!--`
    # as the description — see hex_world).  The README fallback only sets the
    # description on FIRST publish; it never overwrites an existing one.
    desc=$(sed -n 's/^[[:space:]]*description[[:space:]]*=[[:space:]]*"\(.*\)"/\1/p' "$tmp/$repo/$name/loft.toml" | head -1)
    desc_src=manifest
    # Categories: the `[package] categories` list groups the library in the
    # generated catalogue, and `tools/validate.py` gate 1 REJECTS an empty one.
    # There is no fallback to invent — a category is a curation decision — so a
    # package new to the index is refused below when its manifest omits it.
    # `.*$` after the closing bracket, so a trailing `# comment` does not make
    # the line read as "no categories declared" — a silent miss there would keep
    # publishing an existing package's stale list.
    cats=$(sed -n 's/^[[:space:]]*categories[[:space:]]*=[[:space:]]*\(\[[^]]*\]\).*$/\1/p' "$tmp/$repo/$name/loft.toml" | head -1)
    if [ -z "$desc" ]; then
        desc_src=readme
        desc=$(awk '
            /^[[:space:]]*<!--/ { inc=1 }
            inc { if (/-->/) inc=0; next }
            /^[[:space:]]*```/ { fence = !fence; next }
            fence { next }
            /^[[:space:]]*$/ { next }
            /^[[:space:]]*#/ { next }
            /^[[:space:]]*(\/\/|<)/ { next }
            { print; exit }
        ' "$tmp/$repo/$name/README.md" 2> /dev/null || true)
    fi
    if ! python3 - "$REG_DIR/index.json" "$name" "$ver" "$tmp/entry_$name.json" \
        "https://github.com/$ORG/$repo/tree/main/$name" "${desc:-loft library $name}" "$desc_src" \
        "${cats:-}" "$tmp/pub_$name.err" <<'EOF'
import json, sys

index_path, name, ver, entry_path, homepage, desc, desc_src, cats_raw, pub_err = sys.argv[1:10]
raw = open(entry_path).read()
try:
    entry = json.loads("{%s}" % raw)[ver]
except json.JSONDecodeError as e:
    # stdout is the machine half of `loft publish` and must parse on its own.  Say which
    # line broke rather than dying in a traceback: this ran AFTER the GitHub release was
    # cut, so the operator needs to know what to fix, not what to read.
    print(f"::error::`{name}` {ver}: `loft publish` stdout is not a JSON entry ({e}).")
    for n, line in enumerate(raw.splitlines()[:40], 1):
        print(f"    {n:>3}| {line[:120]}")
    sys.exit(1)
index = json.load(open(index_path))

# `[package] categories` off the manifest.  A package the index has never seen
# is REFUSED without one: `tools/validate.py` gate 1 rejects an empty
# `categories`, so seeding `[]` here mints an index that the registry's own PR
# validation will not accept — and it fails on somebody else's submission PR,
# far from the publish that caused it.  Measured: every package first published
# since the docs gate landed (2026-06-19) went in that way.
cats = []
if cats_raw.strip():
    try:
        cats = json.loads(cats_raw)
    except json.JSONDecodeError:
        print(f"::error::`{name}`: [package] categories is not a JSON list: {cats_raw}")
        sys.exit(1)
    if not isinstance(cats, list) or not all(isinstance(c, str) and c.strip() for c in cats):
        print(f"::error::`{name}`: [package] categories must be non-empty strings")
        sys.exit(1)
if name not in index["packages"] and not cats:
    print(
        f"::error::`{name}` is new to the index and its loft.toml declares no "
        f"[package] categories — add e.g. `categories = [\"graphics\"]` beside "
        f"`description`.  Gate 1 of the registry's own validation rejects an "
        f"empty list, so publishing without one leaves the index unmergeable."
    )
    sys.exit(1)

pkg = index["packages"].setdefault(
    name, {"description": desc, "homepage": homepage, "categories": [], "yanked": [], "versions": {}}
)
# The official manifest description is authoritative — refresh it on every publish
# so a corrected `[package] description` propagates.  A README-scraped fallback
# only seeds a brand-new package (setdefault above); it never clobbers an
# existing, possibly hand-curated, description.
if desc_src == "manifest" and desc:
    pkg["description"] = desc
# Same rule for categories, and for the same reason: the manifest is where the
# author states them, so a corrected list propagates on the next publish.  A
# manifest that declares none leaves an existing (hand-curated) list alone.
if cats:
    pkg["categories"] = cats

# `deps` is a CLAIM, and `loft publish` can only read it from `loft.toml`.  A
# multi-package repo deliberately keeps its registry deps OUT of the manifest
# (declaring them there resolves from the registry instead of the `--lib` path,
# which breaks multi-library consumption), so for those packages an empty
# `[dependencies]` means "not stated here" and pasting the entry verbatim would
# publish a version that resolves none of its dependencies.  The package's own
# previous entry is where those versions were stated, so carry them forward.
# `[publish] deps-incomplete:` on stderr names what the SOURCE actually uses;
# anything it names that is still absent afterwards is a refusal, not a note.
incomplete = []
try:
    for line in open(pub_err, encoding="utf-8", errors="replace"):
        if line.startswith("[publish] deps-incomplete:"):
            incomplete = [d.strip() for d in line.split(":", 1)[1].split(",") if d.strip()]
except OSError:
    pass


def _vkey(v):
    return [int(p) if p.isdigit() else -1 for p in v.split(".")]


if not entry.get("deps") and pkg["versions"]:
    prev = max(pkg["versions"], key=_vkey)
    carried = pkg["versions"][prev].get("deps") or {}
    if carried:
        entry["deps"] = dict(carried)
        print(
            f"  deps carried forward from {name} {prev}: "
            + ", ".join(f"{k} {v}" for k, v in sorted(carried.items()))
        )
missing = [d for d in incomplete if d not in (entry.get("deps") or {})]
if missing:
    print(
        f"::error::`{name}` {ver}: the source uses {', '.join(missing)}, and neither "
        f"loft.toml nor the previous index entry says which version(s).  Publishing this "
        f"would install a version that resolves none of them.  Add them to `[dependencies]` "
        f"or to the previous entry's `deps` first."
    )
    sys.exit(1)

pkg["versions"][ver] = entry
# Bump the top-level `updated` so it reflects this publish (REGISTRY_SUBMIT.md).
# Reuse the entry's own `published` UTC stamp — the precise moment this version
# was published, already in `YYYY-MM-DDTHH:MM:SSZ` form — falling back to now.
# Sequential publishes overwrite it, so the last (newest) one wins.
from datetime import datetime, timezone

index["updated"] = entry.get("published") or datetime.now(timezone.utc).strftime(
    "%Y-%m-%dT%H:%M:%SZ"
)
json.dump(index, open(index_path, "w"), indent=2, ensure_ascii=False)
open(index_path, "a").write("\n")
EOF
    then
        echo "  ⚠ index update failed — skipping $name $ver"; SKIPPED_LIBS+=("$name-$ver"); continue
    fi
    published+=("$name-$ver")
    # `<name>@<ver>` as well as `<name>-<ver>`: the signer keys on the first and
    # a package name may itself contain a `-`, so splitting the message form back
    # apart would be a guess.  Two spellings, one append site.
    EXPECTS+=("$name@$ver")
done < "$tmp/work.tsv"

# ── submissions/ drain (C96 key-absent path) ─────────────────────────────────
# A key-absent contributor stages a version by adding submissions/<name>-<ver>.json
# (name, version, repo, tag, subpath, entry) via a PR that does NOT touch index.json,
# so it can't race the signed index.  Here — on the key-present machine — each staged
# submission goes through the full vet-lib gate; a PASS is folded into the index and
# its staging file `git rm`ed (committed together with the index + .sig by the sign
# step, so it lands atomically); a native-code NEEDS-REVIEW or a FAIL is reported and
# the submission left in place for a human decision.
DRAINED=(); REVIEW_SUBS=(); FAILED_SUBS=()
for sub in "$REG_DIR"/submissions/*.json; do
    [ -e "$sub" ] || continue
    # Same reason as the PR merge above: folding a submission would change the
    # index this run is about to bind its signature to.
    [ -n "$ONLY" ] && { echo "  --only: leaving $(basename "$sub") staged"; continue; }
    read -r s_name s_ver s_repo s_tag s_sub < <(python3 - "$sub" <<'PYEOF'
import json, sys
d = json.load(open(sys.argv[1]))
print(d.get("name", ""), d.get("version", ""), d.get("repo", ""),
      d.get("tag", ""), d.get("subpath", d.get("name", "")))
PYEOF
)
    if [ -z "$s_name" ] || [ -z "$s_ver" ] || [ -z "$s_repo" ] || [ -z "$s_tag" ]; then
        echo "  ⚠ malformed submission $(basename "$sub") (need name/version/repo/tag) — left for review"
        FAILED_SUBS+=("$(basename "$sub")"); continue
    fi
    echo "vetting submission $s_name $s_ver ($s_repo @ $s_tag) ..."
    bash "$here/scripts/vet-lib.sh" "$s_repo" "$s_tag" "$s_sub" > "$tmp/vet_$s_name.log" 2>&1
    rc=$?
    if [ "$rc" = 3 ]; then
        echo "  ⚠ $s_name $s_ver NEEDS REVIEW (carries #native/#rust) — promote by hand after review (log: $tmp/vet_$s_name.log)"
        REVIEW_SUBS+=("$s_name-$s_ver"); continue
    fi
    if [ "$rc" != 0 ]; then
        echo "  ✗ $s_name $s_ver did NOT pass the gate — not admitted (log: $tmp/vet_$s_name.log)"
        FAILED_SUBS+=("$s_name-$s_ver"); continue
    fi
    if python3 - "$REG_DIR/index.json" "$sub" <<'PYEOF'
import json, sys
from datetime import datetime, timezone
index_path, sub_path = sys.argv[1:3]
sub = json.load(open(sub_path))
name, ver, entry = sub["name"], sub["version"], sub["entry"]
index = json.load(open(index_path))
# A submission that introduces a package must carry its own `categories`, for
# the reason the own-lib fold states: gate 1 rejects an empty list, so folding
# one in leaves the index unmergeable.  Refusing here leaves the staging file in
# place (the caller reports it), which is the recoverable outcome.
cats = sub.get("categories") or []
if not isinstance(cats, list) or not all(isinstance(c, str) and c.strip() for c in cats):
    print(f"::error::submission `{name}`: `categories` must be a list of non-empty strings")
    sys.exit(1)
if name not in index["packages"] and not cats:
    print(f"::error::submission `{name}` is new to the index and carries no `categories`")
    sys.exit(1)
pkg = index["packages"].setdefault(name, {
    "description": sub.get("description", f"loft library {name}"),
    "homepage": sub.get("homepage", ""), "categories": [], "yanked": [], "versions": {}})
if cats:
    pkg["categories"] = cats
entry.setdefault("published", datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"))
pkg["versions"][ver] = entry
index["updated"] = entry["published"]
json.dump(index, open(index_path, "w"), indent=2, ensure_ascii=False)
open(index_path, "a").write("\n")
PYEOF
    then
        git -C "$REG_DIR" rm -q "submissions/$(basename "$sub")" 2>/dev/null || rm -f "$sub"
        DRAINED+=("$s_name-$s_ver"); echo "  ✓ vetted + folded $s_name $s_ver (staging file removed)"
    else
        echo "  ⚠ index fold failed for $s_name $s_ver — submission left in place"
        FAILED_SUBS+=("$s_name-$s_ver")
    fi
done

# Summary — never drop a skip silently.
if [ "${#SKIPPED_PRS[@]}" -gt 0 ] || [ "${#SKIPPED_LIBS[@]}" -gt 0 ] || [ "$n_blocked" != 0 ]; then
    echo
    echo "⚠ skipped (resolve + re-run to include): PRs: ${SKIPPED_PRS[*]:-none} | libs: ${SKIPPED_LIBS[*]:-none}"
    if [ "$n_blocked" != 0 ]; then
        echo "  stale releases excluded in pre-flight ($n_blocked) — bump version + re-release:"
        cat "$tmp/blocked.txt"
    fi
fi
if [ "${#DRAINED[@]}" -gt 0 ] || [ "${#REVIEW_SUBS[@]}" -gt 0 ] || [ "${#FAILED_SUBS[@]}" -gt 0 ]; then
    echo
    echo "submissions: folded ${DRAINED[*]:-none} | needs-review ${REVIEW_SUBS[*]:-none} | rejected ${FAILED_SUBS[*]:-none}"
fi

echo
git -C "$REG_DIR" --no-pager diff --stat index.json || true

# ── sign + push ───────────────────────────────────────────────────────────────
# Delegate to registry-sign.sh — the SINGLE trust-root signer.  It stages
# index.json + its .sig together (#377), shows the diff + re-verifies each
# tarball's sha256, signs, runs `keygen verify`, then commits + pushes.  Keeping
# one signer means the signing-safety rules (commit both, post-sign verify, key
# default) live in exactly one place instead of drifting between two scripts.
# Re-signing here is also what fixes a `gh pr merge` that changed index.json on
# the remote without re-signing — the routine must never leave it unsigned.
sign_args=(--registry-dir "$REG_DIR" --key "$KEY" --message "publish: ${published[*]:-PR merges / re-sign}")
if [ -n "$ONLY" ]; then
    # `--only` already said what this run is for, and the loop above recorded what
    # it actually did.  Handing those to the signer as `--expect` closes the gap
    # between the two: if the index carries anything else — a version this run did
    # not publish, a removal, another package's description rewritten — it refuses
    # rather than signing it.  That is also the confirmation, so no `--yes`.
    if [ ${#EXPECTS[@]} -eq 0 ]; then
        echo "--only published nothing — not signing (the index is unchanged by this run)." >&2
        exit 0
    fi
    for e in "${EXPECTS[@]}"; do sign_args+=(--expect "$e"); done
elif [ "$YES" = 1 ]; then
    sign_args+=(--yes)
fi
"$here/scripts/registry-sign.sh" "${sign_args[@]}"

echo
echo "verifying — coverage check should now be clean:"
# Check against the index we JUST pushed (local REG_DIR copy), not the CDN-cached
# raw URL — otherwise a publish shows a spurious "stale" until the cache catches
# up (the CDN serves the pre-push index for a few minutes).
"$here/scripts/check_registry_coverage.sh" --index "$REG_DIR/index.json"
