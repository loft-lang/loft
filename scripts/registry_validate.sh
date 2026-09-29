#!/usr/bin/env bash
# Validate one REGISTRY package against the current loft + rustc toolchain.
#
# Installs the package from the live registry (the real tarball + sha256 +
# dependency resolution a user gets), builds its native crate if it ships one,
# and runs its own test suite on BOTH backends.  This validates the PUBLISHED
# artifact — not the source repo, whose CI already gates pushes — so it
# catches "the toolchain moved and the released library rotted" (the class
# behind loft-lang/loft-libs-core#14: registry random 0.2.1 was dead on the
# 2026.6 toolchain and no CI anywhere could see it).
#
# The version validated is resolved from the INDEX, refreshed first — never from
# whatever happens to sit in ~/.loft/registry.  Both halves are load-bearing
# (loft#1027): run straight after publishing 0.1.1, this script installed
# nothing (the un-refreshed index still ended at 0.1.0), then picked the highest
# LOCAL directory, validated 0.1.0, and printed `OK` — a green light about the
# release it had not looked at.  Reading the local cache also made the verdict
# depend on which versions this machine had downloaded before.
#
# Usage:  scripts/registry_validate.sh <package>[@<version>]
#           hex_way          validate the index's newest stable release
#           hex_way@0.1.1    validate exactly that release
# Env:    LOFT               loft binary to validate against (default: loft on PATH)
#         LOFT_REGISTRY_URL  respected by loft itself (defaults to the live index)
#         LOFT_HOME          registry cache root (default: $HOME)
#         LOFT_TIMEOUT       per-run watchdog seconds (default 300)
#
# Exit codes: 0 = package is healthy; 1 = any step failed.
set -u

ARG="${1:?usage: registry_validate.sh <package>[@<version>]}"
PKG="${ARG%%@*}"
WANT_VER="${ARG#*@}"
[ "$WANT_VER" = "$ARG" ] && WANT_VER=""
LOFT="${LOFT:-loft}"
LOFT_CACHE="${LOFT_HOME:-$HOME}/.loft/registry"
export LOFT_TIMEOUT="${LOFT_TIMEOUT:-300}"

fail() { echo "registry-validate: $PKG: FAIL — $*" >&2; exit 1; }
note() { echo "registry-validate: $PKG: $*"; }

# The loft commit under test — stamped into every failure so a red night names
# the exact loft that broke a package.  Read from the checkout the BINARY was built
# in when `$LOFT` is a path (a `target/release/loft` of another worktree), and from
# the working directory only for a `loft` found on PATH: asked of the working
# directory alone, a run against another build named this checkout's commit.
loft_path="$(command -v "$LOFT" 2>/dev/null || true)"
LOFT_SHA=""
case "$LOFT" in
  */*) [ -n "$loft_path" ] && LOFT_SHA="$(git -C "$(dirname "$(readlink -f "$loft_path")")" rev-parse --short HEAD 2>/dev/null || true)" ;;
esac
[ -n "$LOFT_SHA" ] || LOFT_SHA="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
PKG_VER="?"   # filled once the installed version dir is known (step 2)

# On a test failure the raw CI log buries the assertion under cargo/rustc noise.
# Surface the salient lines three ways — a GHA ::error annotation (top of the
# run), a collapsible ::group, and the job Step Summary — each stamped with the
# package version + loft commit, so the cause is visible without log-diving.
emit_failure() {
    local backend="$1" logfile="$2" salient assertln
    salient="$(grep -avE '^[[:space:]]*(Compiling|Downloaded|Updating|Finished|Building|Fresh|Locking|Downloading) ' "$logfile" \
               | grep -aviE 'Node\.js 20|being deprecated|deprecation-of-node' | tail -40)"
    assertln="$(grep -aiE 'assertion failed|Error:|panic|^ *FAIL ' "$logfile" | head -1 | tr -d '[:cntrl:]')"
    {
        echo "::error title=registry-validate $PKG ($backend)::${assertln:-tests failed against loft $LOFT_SHA}"
        echo "::group::registry-validate $PKG $PKG_VER — $backend failure detail (loft $LOFT_SHA)"
        echo "$salient"
        echo "::endgroup::"
    } >&2
    if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
        {
            echo "### ❌ \`$PKG\` $PKG_VER — $backend tests failed"
            echo "_loft \`$LOFT_SHA\` · rustc \`$(rustc --version 2>/dev/null | awk '{print $2}')\`_"
            echo
            echo '```'
            echo "$salient"
            echo '```'
        } >> "$GITHUB_STEP_SUMMARY"
    fi
}

# Run one backend's test suite with output captured; on failure show the full
# output (raw log) then the distilled diagnostic block.
run_backend() {
    local backend="$1"
    local log="$SCRATCH/test-$backend.log"
    note "tests: --$backend"
    if (cd "$PKG_DIR" && LOFT_ERRORS=pretty "$LOFT" --"$backend" test) >"$log" 2>&1; then
        grep -aiE 'test result|passed' "$log" | tail -1
        return 0
    fi
    cat "$log" >&2
    emit_failure "$backend" "$log"
    return 1
}

command -v "$LOFT" >/dev/null 2>&1 || fail "loft binary '$LOFT' not found"
note "toolchain: $("$LOFT" --version 2>/dev/null | head -1) | loft@$LOFT_SHA | $(rustc --version 2>/dev/null)"

# Work in a scratch project dir so the install's .loft/api stubs and lockfile
# never land in a real tree.
SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
cd "$SCRATCH" || fail "cannot enter scratch dir"

# 1. Resolve the version to validate FROM THE INDEX, refreshed first.  Deciding
#    this before installing is what keeps the answer about the registry rather
#    than about this machine's download history.  `api --registry` lists one line
#    per package — `<name> <latest stable> — <description>` — so the newest
#    non-yanked, non-prerelease release is field 2 of the row whose field 1 is an
#    exact name match.
CATALOG="$("$LOFT" api --registry --refresh 2>&1)" || fail "cannot read the registry index: $CATALOG"
INDEX_VER="$(printf '%s\n' "$CATALOG" | awk -v p="$PKG" '$1==p {print $2; exit}')"
[ -n "$INDEX_VER" ] || fail "\`$PKG\` is not in the registry index (${LOFT_REGISTRY_URL:-the default registry})"

PKG_VER="${WANT_VER:-$INDEX_VER}"
# Which release is under test, said once and said plainly.  Validating an older
# one on purpose is legitimate; validating it by ACCIDENT was the bug, so the two
# cases must not read alike.
if [ "$PKG_VER" = "$INDEX_VER" ]; then
    note "validating $PKG_VER — the index's newest stable"
else
    note "validating $PKG_VER as asked — NOT the newest; the index's newest stable is $INDEX_VER"
fi

# 2. Install exactly that version — exercises index fetch, tarball download,
#    sha256 verification, and dependency resolution.  Pinned, so no later step
#    has to guess what was resolved.
note "loft install $PKG@$PKG_VER"
"$LOFT" install "$PKG@$PKG_VER" || fail "loft install $PKG@$PKG_VER failed"

# 3. Locate that exact artifact.  Not "the highest directory present": a
#    mismatch here means the install resolved something other than what was
#    asked for, which is a finding, not a version to fall back to.
PKG_DIR="$LOFT_CACHE/$PKG-$PKG_VER"
[ -d "$PKG_DIR" ] || fail "install reported success but $PKG_DIR does not exist — resolved a different version?"
note "validating $PKG_DIR (version $PKG_VER)"

# 4. Build the native crate against current rustc, if the package ships one.
#    (loft can rebuild these on demand, but an explicit build gives the
#    failure its own step and a readable rustc error.)
if [ -d "$PKG_DIR/native" ]; then
    note "building native crate"
    (cd "$PKG_DIR/native" && cargo build --release) || fail "native crate build failed"
fi

# 5. Run the package's own tests on both backends.  Functional gate only —
#    no LOFT_DENY_WARNINGS: a released artifact must WORK on the new
#    toolchain; warning-cleanliness is the source repo CI's job.
if [ -d "$PKG_DIR/tests" ]; then
    run_backend interpret || fail "interpreter tests failed — see the ::group / Step Summary above"
    run_backend native    || fail "native tests failed — see the ::group / Step Summary above"
else
    # No tests in the tarball — still prove it loads: compile a bare `use`.
    note "no tests/ shipped — smoke-compiling 'use $PKG;'"
    printf 'use %s;\nfn main() { }\n' "$PKG" > smoke.loft
    "$LOFT" --interpret smoke.loft || fail "smoke program failed to run"
fi

# The verdict names the version, because a bare `OK` is what let a validation of
# the PREVIOUS release read as a pass for the one just published (loft#1027).
note "OK — $PKG $PKG_VER"
