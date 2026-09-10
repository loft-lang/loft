#!/usr/bin/env bash
# registry-sign.sh — SEE exactly what you are about to sign into the loft
# registry index, then sign it.  The human review IS the trust root
# (REGISTRY_BOOTSTRAP.md § Step 4 — "Why laptop signing"): a signature is only
# as trustworthy as the maintainer's look at what changed.
#
# For every added/changed package version it shows the LIBRARY RELEASE it points
# at (repo + tag), optionally that release's notes, and DOWNLOADS the tarball to
# confirm its sha256 actually matches the index — then asks before signing.
#
#   scripts/registry-sign.sh [opts]      # run in / pointed at a loft-lang/registry checkout
#     --registry-dir DIR  registry checkout (default: $PWD; if that isn't a
#                         registry, the live loft-lang/registry is cloned)
#     --pr N              clone the registry + check out registry PR #N, then sign
#     --key FILE          Ed25519 private key file (the local-key path)
#                         (default: $LOFT_REGISTRY_KEY or
#                          ~/.loft/trust-root/registry-signing-key.bin)
#     --yubikey           force ON-CARD signing only — fail (don't fall back to a
#                         file key) if the card doesn't sign.  (or LOFT_REGISTRY_SIGNER=yubikey)
#     --key FILE-only     LOFT_REGISTRY_SIGNER=file skips the card and uses --key.
#     --since REF         diff index.json against this git ref (default: auto —
#                         HEAD if you have uncommitted edits, else HEAD~1)
#     --notes             also print each release's full notes (gh release body)
#     --no-download       skip the tarball sha256 re-download check
#     --no-push           sign + commit locally but do NOT push (keeps the clone)
#     --message MSG       commit message (default: "sign: …"; registry_maintain
#                         passes "publish: <libs>")
#     --expect P@V        sign ONLY this — repeatable.  The diff must introduce
#                         exactly the named versions, remove nothing, and leave
#                         every other package untouched, or the run REFUSES.
#                         This is the agent-safe confirmation: it replaces the
#                         typed 'yes' with a check, and it is stricter than the
#                         prompt it replaces (a 'yes' verifies nothing).
#     --expect-yank P@V   sign a YANK and only that — repeatable.  The diff must
#                         add exactly the named versions to those packages'
#                         `yanked` arrays, add or change NO version, un-yank
#                         nothing, and leave every other field of every package
#                         alone.  A yank adds no version, so plain --expect can
#                         never describe one: it refuses for "asked for but
#                         ABSENT from the diff" while the yank itself reads as
#                         untouchable metadata drift.  The two combine.
#     --expect-meta P     sign a METADATA correction to these packages and only
#                         that — repeatable.  The diff must change nothing but
#                         their description / homepage / categories, add or remove
#                         NO version anywhere, change no `yanked` array, and leave
#                         every other package alone.  Same bargain as
#                         --expect-yank and for the same reason: a metadata fix
#                         adds no version, so plain --expect can never describe
#                         one — it refuses for "asked for but ABSENT from the
#                         diff" while the correction itself reads as untouchable
#                         drift.  Without it the only route left is --yes, which
#                         asserts nothing about what is signed and is refused
#                         off a terminal anyway.  All three combine.
#     --yes               skip the confirm prompt (scripted use).  Prefer
#                         --expect: --yes asserts nothing about WHAT is signed.
#
# If a run fails, re-run THIS script against the same checkout
# (`--registry-dir <that dir>`) rather than the whole registry_maintain.sh cycle:
# a refusal leaves the staged index in place and unsigned, so the retry is seconds
# rather than the ~8 minutes a full re-clone + `compat check --full` costs.
#
# Signing — DEFAULT is YubiKey-first with a local-key fallback (both end at the
# same trust gate):
#   1. Try the YubiKey ON-CARD (PIV slot 9C, Ed25519) for LOFT_YUBIKEY_TIMEOUT
#      (default 10s) — its PIN + touch are the human-presence proof; the private key
#      never leaves the card.  Default call is `pkcs11-tool --mechanism EDDSA`
#      (module auto-found or LOFT_YUBIKEY_PKCS11_MODULE; PIV 9C → id 02 or
#      LOFT_YUBIKEY_PIV_ID), or override the whole step with LOFT_YUBIKEY_SIGN_CMD
#      (gets $LOFT_SIG_IN / $LOFT_SIG_OUT).
#   2. If the card is absent / the tool is missing / it times out, FALL BACK to the
#      local key file (`--key`) when present.
#   --yubikey disables the fallback; LOFT_REGISTRY_SIGNER=file disables the card.
# `--expect P@V` is the third route and the one to reach for from a script or an
# agent: no prompt, no touch, and the signature is BOUND to the versions named on
# the command line — anything else in the diff refuses the run.  It answers the
# doctrine's actual concern (PKG_REGISTRY.md § Why laptop signing: "CI signing
# would sign whatever lands in main — no human gate") by making "whatever lands"
# impossible, while the key still never leaves this machine.
# `--yes` skips the confirmation.  The TRUST GATE always runs: whatever was signed
# must verify under a key in src/registry_keys.rs::TRUSTED_PUBLIC_KEYS or it is
# NOT committed/pushed — so a wrong key/module fails safe.
#
# On confirm it signs, commits index.json + index.json.sig together (so HEAD's
# index always matches its signature — #377), pushes, and (for an auto-clone)
# deletes the temp checkout.  A failed push keeps the clone so the signed commit
# is never lost.
#
# Refuses to sign (non-zero exit) on a sha256 mismatch, invalid JSON, or a "no"
# at the prompt.  Needs: python3, gh (for notes), and target/release/loft-keygen.
set -euo pipefail

REG_DIR="$PWD"; REG_GIVEN=0; PR=""; SINCE=""; NOTES=0; DOWNLOAD=1; YES=0; PUSH=1; MSG=""; EXPECT=""; EXPECT_YANK=""; EXPECT_META=""
KEY="${LOFT_REGISTRY_KEY:-$HOME/.loft/trust-root/registry-signing-key.bin}"
YUBIKEY=0; [ "${LOFT_REGISTRY_SIGNER:-}" = yubikey ] && YUBIKEY=1  # set -e safe: file/unset/other => 0 (local-key path)
while [ $# -gt 0 ]; do
    case "$1" in
        --registry-dir) REG_DIR="$2"; REG_GIVEN=1; shift;;
        --pr)           PR="$2"; shift;;
        --key)          KEY="$2"; shift;;
        --yubikey)      YUBIKEY=1;;
        --since)        SINCE="$2"; shift;;
        --notes)        NOTES=1;;
        --no-download)  DOWNLOAD=0;;
        --no-push)      PUSH=0;;
        --message)      MSG="$2"; shift;;
        --expect)       EXPECT="${EXPECT:+$EXPECT }$2"; shift;;
        --expect-yank)  EXPECT_YANK="${EXPECT_YANK:+$EXPECT_YANK }$2"; shift;;
        --expect-meta)  EXPECT_META="${EXPECT_META:+$EXPECT_META }$2"; shift;;
        --yes)          YES=1;;
        -h|--help)      sed -n '2,33p' "$0"; exit 0;;
        *) echo "unknown argument: $1" >&2; exit 2;;
    esac
    shift
done

# `--yes` means "a human decided and is not here to type it".  The second half of
# that is only true at a terminal: with no TTY on stdin the flag asserts a human
# who cannot be present, which is exactly the shape a script or an agent has.
# Refuse there and point at `--expect`, which makes the same claim CHECKABLE.
# (An interactive `--yes` still works, and `registry_maintain.sh` inherits this
# session's stdin, so its own `--yes` passes from a terminal and refuses from a
# pipe just the same.)
if [ "$YES" = 1 ] && [ ! -t 0 ]; then
    echo "!! --yes with no terminal on stdin: nothing can confirm this." >&2
    echo "   Use --expect <pkg>@<ver> — it binds the signature to what you named" >&2
    echo "   and refuses anything else in the diff, which is what --yes cannot do." >&2
    echo "   A yank is --expect-yank <pkg>@<ver>; a description / homepage / category" >&2
    echo "   correction that adds no version is --expect-meta <pkg>." >&2
    exit 2
fi

here=$(cd "$(dirname "$0")/.." && pwd)
KG="$here/target/release/loft-keygen"
if [ ! -x "$KG" ]; then
    echo "building loft-keygen (release) ..." >&2
    (cd "$here" && cargo build --release --bin loft-keygen --features registry >/dev/null)
fi

# Try to sign <in> → <out> ON-CARD with the YubiKey (PIV slot 9C, Ed25519), bounded
# to LOFT_YUBIKEY_TIMEOUT (default 10s) for the PIN + touch.  The private key never
# leaves the card.  Returns 0 on success (sig written), NON-zero on absent card /
# missing tool / timeout / failure — so the caller falls back to the local key.
# The trust gate below re-verifies whatever is produced, so a wrong module/slot/key
# fails SAFE (no push).  Override the whole step with LOFT_YUBIKEY_SIGN_CMD (gets
# $LOFT_SIG_IN / $LOFT_SIG_OUT); else a default pkcs11-tool EDDSA call — module
# auto-found (override LOFT_YUBIKEY_PKCS11_MODULE), PIV 9C → id 02 (override
# LOFT_YUBIKEY_PIV_ID).
# Path to the PKCS#11 module (LOFT_YUBIKEY_PKCS11_MODULE, else common locations).
# Robust to Homebrew prefixes, versioned dylib names (libykcs11.2.dylib), and a
# pkcs11/ subdir.  Prefers Yubico's ykcs11 (best for YubiKey PIV Ed25519) over
# OpenSC's opensc-pkcs11.
yubikey_module() {
    [ -n "${LOFT_YUBIKEY_PKCS11_MODULE:-}" ] && { echo "$LOFT_YUBIKEY_PKCS11_MODULE"; return; }
    local d m bp
    local dirs=(); bp=$(brew --prefix 2>/dev/null) && [ -n "$bp" ] && dirs+=("$bp/lib")
    dirs+=(/opt/homebrew/lib /usr/local/lib /usr/lib/x86_64-linux-gnu /usr/lib)
    # ykcs11 first (Yubico), then opensc; allow version suffixes + a pkcs11/ subdir.
    for d in "${dirs[@]}"; do
        for m in "$d"/libykcs11*.dylib "$d"/libykcs11*.so \
                 "$d"/opensc-pkcs11*.so "$d"/opensc-pkcs11*.dylib \
                 "$d"/pkcs11/libykcs11*.* "$d"/pkcs11/opensc-pkcs11*.*; do
            [ -e "$m" ] && { echo "$m"; return; }
        done
    done
}
# Is on-card signing available here?  Decided BEFORE prompting, so an absent card
# falls straight to the local key instead of making you wait.
yubikey_available() {
    [ -n "${LOFT_YUBIKEY_SIGN_CMD:-}" ] && return 0
    command -v pkcs11-tool >/dev/null 2>&1 && [ -n "$(yubikey_module)" ]
}

yubikey_sign() {  # <in> <out>  → 0 = signed on-card, non-zero = not signed (fall back)
    local in="$1" out="$2" t="${LOFT_YUBIKEY_TIMEOUT:-}"
    local TO=""   # UNBOUNDED by default (take your time); LOFT_YUBIKEY_TIMEOUT=<sec> to bound it
    if [ -n "$t" ]; then
        command -v timeout  >/dev/null 2>&1 && TO="timeout $t"
        [ -z "$TO" ] && command -v gtimeout >/dev/null 2>&1 && TO="gtimeout $t"
    fi
    rm -f "$out"
    if [ -n "${LOFT_YUBIKEY_SIGN_CMD:-}" ]; then
        LOFT_SIG_IN="$in" LOFT_SIG_OUT="$out" $TO bash -c "$LOFT_YUBIKEY_SIGN_CMD" \
            || { echo "  YubiKey: LOFT_YUBIKEY_SIGN_CMD failed" >&2; return 1; }
        [ -s "$out" ] || { echo "  YubiKey: produced no signature" >&2; return 1; }
        return 0
    fi
    local module; module="$(yubikey_module)"
    local pin_arg=(); [ -n "${LOFT_YUBIKEY_PIN:-}" ] && pin_arg=(--pin "$LOFT_YUBIKEY_PIN")
    echo "  YubiKey: signing on-card via $module (PIV 9C / id ${LOFT_YUBIKEY_PIV_ID:-02})." >&2
    echo "           Enter PIN if asked, then TOUCH the key — take your time." >&2
    # NOTE: output is shown (PIN prompt + errors visible — do NOT suppress it).
    $TO pkcs11-tool --module "$module" --sign --mechanism EDDSA \
        --id "${LOFT_YUBIKEY_PIV_ID:-02}" --login ${pin_arg[@]+"${pin_arg[@]}"} \
        --input-file "$in" --output-file "$out" \
        || { echo "  YubiKey: pkcs11-tool sign failed — check the id/PIN/slot, or set LOFT_YUBIKEY_SIGN_CMD" >&2; return 1; }
    [ -s "$out" ] || { echo "  YubiKey: empty signature" >&2; return 1; }
    return 0
}

# No explicit checkout and the cwd isn't a registry → clone the live registry, so
# the routine runs standalone (inspection, or signing a PR via --pr).
CLONED=0; SIGNED=0; PUSHED=0
if [ "$REG_GIVEN" != 1 ] && [ ! -f "$REG_DIR/index.json" ]; then
    REG_DIR=$(mktemp -d -t loft-registry.XXXXXX)
    echo "cloning loft-lang/registry → $REG_DIR ..." >&2
    # SSH first: it pushes with your key and never prompts, so the later
    # commit+push succeeds unattended.  GitHub no longer accepts an HTTPS
    # password at the git push prompt, so an HTTPS clone needs a working
    # credential helper — fall back to it (gh, then plain HTTPS) only when SSH
    # is unavailable.
    git clone -q git@github.com:loft-lang/registry.git "$REG_DIR" 2>/dev/null \
        || gh repo clone loft-lang/registry "$REG_DIR" -- -q 2>/dev/null \
        || git clone -q https://github.com/loft-lang/registry "$REG_DIR"
    CLONED=1
    if [ -n "$PR" ]; then
        echo "checking out registry PR #$PR ..." >&2
        git -C "$REG_DIR" fetch -q origin "pull/$PR/head:pr-$PR"
        git -C "$REG_DIR" checkout -q "pr-$PR"
    fi
fi
cleanup() {
    rm -f "${PREV:-}"
    # Delete an auto-clone when nothing was signed, or it was signed AND pushed.
    # Keep it only when there's an unpushed signed commit (push failed / --no-push).
    if [ "$CLONED" = 1 ]; then
        if [ "$SIGNED" != 1 ] || [ "$PUSHED" = 1 ]; then
            rm -rf "$REG_DIR"
        else
            echo "kept temp clone (unpushed signed commit): $REG_DIR" >&2
        fi
    fi
}
trap cleanup EXIT

INDEX="$REG_DIR/index.json"
[ -f "$INDEX" ] || { echo "no index.json in $REG_DIR — use --registry-dir" >&2; exit 2; }
# No early key-file requirement: the default tries the YubiKey first, so a missing
# file key is fine when the card can sign.  The sign block errors if NEITHER a card
# signature nor a local key is available.

# Shape gate: refuse to even look at a non-JSON index.
python3 -c 'import json,sys; json.load(open(sys.argv[1]))' "$INDEX" \
    || { echo "index.json is NOT valid JSON — refusing." >&2; exit 1; }

# Schema gate: refuse to SIGN an index the registry's own PR validation rejects.
# `registry_schema_gate.sh` runs gate 1 out of the checkout being signed, so the
# rules are the ones `pr-validate.yml` will apply rather than a second copy of
# them.  This is the chokepoint rather than the door: an index that fails gate 1
# reaches `main` only through a signature, and once there it reddens every later
# submission PR on a check nobody but the key holder can clear.
"$(dirname "$0")/registry_schema_gate.sh" "$REG_DIR" \
    || { echo "index.json fails the registry's own schema gate (above) — refusing to sign." >&2; exit 1; }
echo "schema   : gate 1 (tools/validate.py) passes"

# Auto-pick the diff base: uncommitted edits → compare to HEAD; otherwise the
# change you're signing is the last commit, so compare to HEAD~1.
if [ -z "$SINCE" ]; then
    if git -C "$REG_DIR" diff --quiet HEAD -- index.json 2>/dev/null; then
        SINCE=HEAD~1
    else
        SINCE=HEAD
    fi
fi
git -C "$REG_DIR" rev-parse "$SINCE" >/dev/null 2>&1 || SINCE=""   # no such ref / no git

# `--expect` is a claim about what CHANGED, so it is meaningless without something
# to have changed FROM.  With no diff base every version reads as new, and the
# check would either refuse everything or — worse — be satisfied by an index that
# happens to hold only the expected package.  Refuse instead of guessing.
if [ -n "$EXPECT$EXPECT_YANK" ] && [ -z "$SINCE" ]; then
    echo "!! --expect needs a diff base, and this checkout has no usable git history." >&2
    echo "   Pass --since <ref>, or sign without --expect." >&2
    exit 2
fi

echo "================  WHAT YOU ARE SIGNING  ================"
echo "registry : $REG_DIR"
echo "index    : $(wc -c < "$INDEX") bytes   sha256 $(sha256sum "$INDEX" | cut -d' ' -f1)"
echo "key      : $KEY"
echo "diff base: ${SINCE:-<none — treating every version as new>}"
echo

echo "----  1. raw change to index.json  ----"
if [ -n "$SINCE" ]; then
    git -C "$REG_DIR" --no-pager diff "$SINCE" -- index.json || true
else
    echo "  (no git history to diff against)"
fi
echo

echo "----  2. library releases + 3. tarball integrity  ----"
PREV=$(mktemp)   # removed by cleanup() on exit
if [ -n "$SINCE" ]; then
    git -C "$REG_DIR" show "$SINCE:index.json" > "$PREV" 2>/dev/null || printf '{}' > "$PREV"
else
    printf '{}' > "$PREV"
fi

set +e
NOTES="$NOTES" DOWNLOAD="$DOWNLOAD" EXPECT="$EXPECT" EXPECT_YANK="$EXPECT_YANK" EXPECT_META="$EXPECT_META" python3 - "$PREV" "$INDEX" <<'PY'
import json, sys, os, re, hashlib, shutil, tempfile, time, urllib.request, urllib.error, subprocess
def load(p):
    try:
        with open(p) as f: return json.load(f)
    except Exception:
        return {}

# ---- tarball fetch (loft#887) -------------------------------------------------
# The re-download below is the trust-root backstop, so it must not be the thing
# that fails.  Two rules:
#
#   * A dropped connection is RETRIED.  It says nothing about the artifact, and
#     the sha256 comparison still decides correctness, so a retry can only turn a
#     non-answer into an answer — it weakens no check.
#   * An ABSENT asset (HTTP 404/410) is terminal and reported as such.  Retrying
#     cannot make a missing file appear, and the old wording ("does the release
#     exist with the named asset?") sent the reader to verify a release that was
#     never in doubt.
#
# `curl` goes first when present: GitHub's release-asset redirect chain
# (objects.githubusercontent.com) drops most of `urllib`'s connections on some
# hosts while `curl` copes with the same URL at the same moment — 1/4 vs 4/4 when
# loft#887 measured it.  `urllib` remains the fallback so the script keeps working
# with python3 alone.
FETCH_ATTEMPTS = 4

def _fetch_curl(url):
    """One `curl` attempt → `(data, reason, absent)`."""
    fd, tmp = tempfile.mkstemp(prefix="loft-registry-sign-")
    os.close(fd)
    try:
        out = subprocess.run(
            ["curl", "-sSL", "--max-time", "180", "-o", tmp, "-w", "%{http_code}", url],
            capture_output=True, text=True, timeout=200)
        code = (out.stdout or "").strip()
        if out.returncode != 0:
            why = (out.stderr.strip().splitlines() or [f"exit {out.returncode}"])[-1]
            return None, why, False
        if code in ("404", "410"):
            return None, f"HTTP {code}", True
        if code != "200":
            return None, f"HTTP {code}", False
        with open(tmp, "rb") as f:
            return f.read(), None, False
    except Exception as e:
        return None, f"{type(e).__name__}: {e}", False
    finally:
        try: os.unlink(tmp)
        except OSError: pass

def _fetch_urllib(url):
    """One `urllib` attempt → `(data, reason, absent)`."""
    try:
        return urllib.request.urlopen(url, timeout=60).read(), None, False
    except urllib.error.HTTPError as e:
        return None, f"HTTP {e.code} {e.reason}", e.code in (404, 410)
    except Exception as e:
        return None, f"{type(e).__name__}: {e}", False

CLIENTS = ([("curl", _fetch_curl)] if shutil.which("curl") else []) + [("urllib", _fetch_urllib)]

def fetch(url):
    """Download `url` → `(data, None, False)`, or `(None, reason, absent)`.

    `absent` True means the asset is genuinely not there (HTTP 404/410); False
    means every attempt failed in transport, which is a network problem and not
    a reason to doubt the release.
    """
    tried = []   # DISTINCT reasons — four identical DNS failures say one thing, not four
    for attempt in range(1, FETCH_ATTEMPTS + 1):
        why = []
        for client, fetcher in CLIENTS:
            data, reason, absent = fetcher(url)
            if data is not None:
                return data, None, False
            if absent:
                return None, reason, True
            why.append(f"{client}: {reason}")
        for w in why:
            if w not in tried:
                tried.append(w)
        print(f"        VERIFY   : attempt {attempt}/{FETCH_ATTEMPTS} failed — {'; '.join(why)}"
              f"{' — retrying' if attempt < FETCH_ATTEMPTS else ''}")
        if attempt < FETCH_ATTEMPTS:
            time.sleep(2 ** (attempt - 1))
    return None, "; ".join(tried), False
# ------------------------------------------------------------------------------

prev, cur = load(sys.argv[1]), load(sys.argv[2])
show_notes = os.environ.get("NOTES") == "1"
download   = os.environ.get("DOWNLOAD", "1") == "1"
pp = (prev.get("packages") or {}) if isinstance(prev, dict) else {}
cp = cur.get("packages") or {}
rel_re = re.compile(r"github\.com/([^/]+)/([^/]+)/releases/download/([^/]+)/")

changes = []
for name in sorted(cp):
    for ver, meta in sorted((cp[name].get("versions") or {}).items()):
        old = (pp.get(name, {}).get("versions") or {}).get(ver)
        if old != meta:
            changes.append((name, ver, meta, old is None))

if not changes:
    print("  (no added or changed versions vs the diff base)")

# ---- --expect: bind the signature to the versions the maintainer NAMED --------
#
# The typed 'yes' this replaces asserts that a human looked; it checks nothing.
# This checks — and it catches the thing the looking demonstrably missed: a run
# that published four packages ALSO rewrote four unrelated descriptions, losing
# `ssh`'s "Native-only", and every gate stayed green because none of them read a
# description (§ The first-`description` gotcha in the publish runbook).
#
# So the check is deliberately wider than "is my package here": the diff must
# introduce exactly the named versions, remove nothing, and leave every other
# package byte-for-byte alone.
# `--expect-yank` is the same bargain for the OTHER maintenance write.  A yank
# adds no version, so it lands entirely in `yanked` — which the check above reads
# as untouchable drift, and which no `--expect` argument can name.  Without a
# bound form the only route left is `--yes`, and `--yes` asserts nothing about
# what is signed, which is exactly what this whole block exists to replace.
expect = set((os.environ.get("EXPECT") or "").split())
expect_yank = set((os.environ.get("EXPECT_YANK") or "").split())
# `--expect-meta` is the THIRD bound form, for the write the other two cannot name: a
# description / homepage / category correction adds no version and touches no `yanked`
# array, so `--expect` refuses it as "ABSENT from the diff" while the correction itself
# is read as untouchable drift.  Measured: `hex_grid`'s index description called a
# pointy-top ODD-R OFFSET package "axial" for twelve days after its manifest was fixed,
# and every route to correcting it alone was closed — `--yes` is refused off a terminal
# by design, which left bundling it into an unrelated publish as the only path, which is
# exactly what the scope check exists to prevent.
expect_meta = set((os.environ.get("EXPECT_META") or "").split())
if expect or expect_yank or expect_meta:
    got = {f"{n}@{v}" for n, v, _, _ in changes}
    removed_pkgs = sorted(set(pp) - set(cp))
    removed_vers = sorted(
        f"{n}@{v}" for n in set(pp) & set(cp)
        for v in (pp[n].get("versions") or {}) if v not in (cp[n].get("versions") or {}))
    # Yanks the diff actually makes, and un-yanks — a version LEAVING a `yanked`
    # array withdraws a warning consumers are already relying on, so it is never
    # implicit and has no `--expect` spelling at all.
    yank_got = {f"{n}@{v}" for n in set(pp) & set(cp)
                for v in set(cp[n].get("yanked") or []) - set(pp[n].get("yanked") or [])}
    unyanked = sorted(f"{n}@{v}" for n in set(pp) & set(cp)
                      for v in set(pp[n].get("yanked") or []) - set(cp[n].get("yanked") or []))
    # A package named in --expect-yank is allowed to differ in `yanked` and in
    # nothing else; every other package still has to match byte-for-byte.
    yank_pkgs = {s.split("@")[0] for s in expect_yank}

    def meta(d, name):
        skip = ("versions", "yanked") if name in yank_pkgs else ("versions",)
        return {k: x for k, x in d[name].items() if k not in skip}

    # Metadata drift is the finding EXCEPT on a package `--expect-meta` names, where it is
    # the point.  Such a package still has to be byte-identical in `versions` and `yanked`
    # — those are the two fields with consumer-visible consequences, and a metadata flag
    # must never become a way to smuggle one past the scope check.
    drift = sorted(n for n in set(pp) & set(cp)
                   if n not in expect_meta and meta(pp, n) != meta(cp, n))
    # A named package that did NOT change is a claim about a diff that is not there —
    # the same phantom the yank path refuses, and worth refusing for the same reason:
    # it means the operator is signing something other than what they described.
    meta_absent = sorted(n for n in expect_meta
                         if n not in set(pp) & set(cp) or meta(pp, n) == meta(cp, n))
    # `versions` and `yanked` on a --expect-meta package: metadata only means metadata.
    meta_overreach = sorted(
        n for n in expect_meta if n in set(pp) & set(cp)
        and ((pp[n].get("versions") or {}) != (cp[n].get("versions") or {})
             or (pp[n].get("yanked") or []) != (cp[n].get("yanked") or [])))
    # A yank of a version the index does not list is a typo, not a yank: it would
    # sign a marker pointing at nothing (PKG_REGISTRY.md § Yanking — the `web`
    # 0.2.2 loss is what that promise is made of).
    phantom = sorted(s for s in expect_yank
                     if s.split("@")[0] not in cp
                     or s.split("@", 1)[1] not in (cp[s.split("@")[0]].get("versions") or {}))
    problems = []
    if got - expect:
        problems.append(("not asked for", sorted(got - expect)))
    if expect - got:
        problems.append(("asked for but ABSENT from the diff", sorted(expect - got)))
    if yank_got - expect_yank:
        problems.append(("YANKED but not asked for", sorted(yank_got - expect_yank)))
    if expect_yank - yank_got:
        problems.append(("yank asked for but ABSENT from the diff",
                         sorted(expect_yank - yank_got)))
    if unyanked:
        problems.append(("versions UN-yanked (a warning withdrawn)", unyanked))
    if phantom:
        problems.append(("yank names a version the index does not list", phantom))
    if removed_pkgs:
        problems.append(("packages REMOVED", removed_pkgs))
    if removed_vers:
        problems.append(("versions REMOVED", removed_vers))
    if drift:
        problems.append(("other packages' metadata changed (description / homepage / "
                         "categories / yanked)", drift))
    if meta_absent:
        problems.append(("--expect-meta names a package whose metadata did NOT change",
                         meta_absent))
    if meta_overreach:
        problems.append(("--expect-meta package also changed versions or yanked "
                         "(metadata only means metadata)", meta_overreach))
    print("----  scope  ----")
    if problems:
        print("!!  --expect MISMATCH — NOT signing.")
        asked = ", ".join(sorted(expect) + [f"yank {y}" for y in sorted(expect_yank)]
                          + [f"meta {m}" for m in sorted(expect_meta)])
        print(f"    asked to sign: {asked or '(nothing)'}")
        for label, items in problems:
            print(f"      {label}:")
            for it in items:
                print(f"        - {it}")
        print()
        print("    Nothing was signed.  Either the index carries more than the publish")
        print("    you asked for, or --expect names the wrong version.")
        sys.exit(1)
    asked = ", ".join(sorted(expect) + [f"yank {y}" for y in sorted(expect_yank)]
                      + [f"metadata of {m}" for m in sorted(expect_meta)])
    print(f"  exactly {asked} — nothing else added, removed or altered")
    # A metadata correction has no tarball and no release, so sections 2 and 3 below print
    # nothing for it.  Say what actually changed here or the run renders as a signature over
    # an empty diff, which is the one thing a reviewer of this output must not conclude.
    for m in sorted(expect_meta):
        for k in sorted(set(meta(pp, m)) | set(meta(cp, m))):
            was, now = meta(pp, m).get(k), meta(cp, m).get(k)
            if was != now:
                print(f"    {m}.{k}:")
                print(f"      was: {was!r}")
                print(f"      now: {now!r}")
    print()
failures = []  # (name, ver, reason) — collected so the end-of-run summary names each
for name, ver, meta, is_new in changes:
    url, sha, size = meta.get("url", ""), meta.get("sha256", ""), meta.get("size")
    print(f"  [{'NEW' if is_new else 'CHANGED'}]  {name} {ver}")
    print(f"        url      : {url}")
    print(f"        sha256   : {sha}")
    print(f"        size     : {size} bytes")
    biny = meta.get("binaries") or {}
    if biny:
        print(f"        binaries : {', '.join(sorted(biny))}")
    m = rel_re.search(url)
    if m:
        owner, repo, tag = m.group(1), m.group(2), m.group(3)
        print(f"        release  : {owner}/{repo} @ {tag}")
        try:
            out = subprocess.run(
                ["gh", "release", "view", tag, "--repo", f"{owner}/{repo}",
                 "--json", "name,publishedAt,body"],
                capture_output=True, text=True, timeout=30)
            if out.returncode == 0:
                rel = json.loads(out.stdout)
                print(f"        published: {rel.get('publishedAt','?')}  \"{rel.get('name') or tag}\"")
                if show_notes and (rel.get("body") or "").strip():
                    for line in rel["body"].rstrip().splitlines():
                        print(f"           | {line}")
            else:
                err = (out.stderr.strip().splitlines() or ["not found"])[0]
                print(f"        notes    : (gh: {err})")
        except Exception as e:
            print(f"        notes    : (unavailable: {e})")
    if download and url and sha:
        data, why, absent = fetch(url)
        if data is None:
            print(f"        VERIFY   : download FAILED: {why}")
            if absent:
                failures.append((name, ver,
                    f"release asset ABSENT ({why}) — the server answered, and answered that the "
                    "named file is not on that release.\n"
                    "             Cause: the release was never cut, the tag differs, or the asset "
                    "failed to upload.\n"
                    "             Fix: upload the asset to the release (or correct `url` in "
                    "index.json), then re-run.\n"
                    f"             url: {url}"))
            else:
                failures.append((name, ver,
                    f"download failed in TRANSPORT after {FETCH_ATTEMPTS} attempts — no HTTP 404 was "
                    "seen, so the release and its asset are NOT in question; the connection is.\n"
                    f"             Attempts: {why}\n"
                    "             Fix: re-run `scripts/registry-sign.sh --registry-dir <this "
                    "checkout>`.  The staged index is\n"
                    "             untouched and unsigned, so retrying the sign alone costs seconds "
                    "instead of a full\n"
                    "             registry_maintain.sh cycle.\n"
                    f"             url: {url}"))
        else:
            got = hashlib.sha256(data).hexdigest()
            if got == sha:
                print(f"        VERIFY   : sha256 MATCH ({len(data)} bytes downloaded)")
                if size is not None and len(data) != size:
                    print(f"        VERIFY   : size MISMATCH !!  downloaded {len(data)} != declared {size}")
                    failures.append((name, ver,
                        f"size mismatch: downloaded {len(data)} bytes, index declares {size}"))
            else:
                print(f"        VERIFY   : sha256 MISMATCH !!  declared {sha[:16]}… got {got[:16]}…")
                failures.append((name, ver,
                    "sha256 mismatch — the uploaded release tarball does not match the index entry "
                    f"(index says {sha[:16]}…, the download hashes to {got[:16]}…).\n"
                    "             Cause: the release artifact is stale, or the lib's source on main "
                    "moved past its released tag.\n"
                    "             Fix: re-cut the release so its bytes match `loft package` at the tag "
                    "(bump the version + re-release if main has changed).\n"
                    f"             url: {url}"))
    print()

if failures:
    n = len(failures)
    print("══════════════════════════════════════════════════════════════════════")
    print("!!  INTEGRITY CHECK FAILED — NOT signing.")
    print(f"    {n} of {len(changes)} new/changed entr{'y' if n == 1 else 'ies'} "
          "failed verification:")
    print()
    for name, ver, reason in failures:
        print(f"  ✗  {name} {ver}")
        print(f"        {reason}")
    print()
    print("    Nothing was signed.  Fix the flagged artifact(s) above, then re-run.")
    print("    (Passing requires each tarball to byte-match `loft package` at its tag —")
    print("     the registry's gate-3 reproducible-build invariant.)")
    sys.exit(1)
sys.exit(0)
PY
rc=$?
set -e
if [ "$rc" -ne 0 ]; then
    # The verification block above prints a named, per-package failure summary.
    echo "registry-sign: refusing to sign — integrity check failed (see the summary above)." >&2
    exit 1
fi

echo "----  4. current signature  ----"
PUB="${KEY%.bin}.pub"
SIG="$REG_DIR/index.json.sig"
if [ -f "$SIG" ] && [ -f "$PUB" ]; then
    if "$KG" verify --in "$INDEX" --sig "$SIG" --pub "$(cat "$PUB")" >/dev/null 2>&1; then
        echo "  current index.json.sig is already VALID under this key (re-sign = no-op)"
    else
        echo "  current index.json.sig does NOT match this key/content (will be replaced)"
    fi
else
    echo "  (no current signature, or no .pub beside the key)"
fi
echo

# Signer policy.  If a YubiKey is available, CONFIRM-BY-TOUCH: go straight to the
# on-card sign — the PIN + touch ARE the confirmation (no typing), and the wait is
# UNBOUNDED (take your time; LOFT_YUBIKEY_TIMEOUT=<sec> to bound it).  Only when the
# card is ABSENT do we fall back to the local key (with a typed 'yes').  --yubikey
# forces on-card (no fallback); LOFT_REGISTRY_SIGNER=file forces the local key.
KEY_ONLY=0; [ "${LOFT_REGISTRY_SIGNER:-}" = file ] && KEY_ONLY=1
USE_CARD=0
[ "$KEY_ONLY" != 1 ] && { [ "$YUBIKEY" = 1 ] || yubikey_available; } && USE_CARD=1
if [ "$KEY_ONLY" != 1 ] && [ "$USE_CARD" = 0 ]; then
    echo "  note: YubiKey signing unavailable here — need pkcs11-tool + a PKCS#11 module" >&2
    echo "        (macOS: brew install opensc yubico-piv-tool), or set LOFT_YUBIKEY_SIGN_CMD." >&2
    echo "        Falling through to the local key." >&2
fi

SIGNED_VIA=""
if [ "$USE_CARD" = 1 ]; then
    [ "$PUSH" = 1 ] && verb="Sign, commit & push" || verb="Sign & commit (no push)"
    echo "$verb this index — review the diff above, then CONFIRM BY TOUCHING YOUR YUBIKEY."
    if yubikey_sign "$INDEX" "$SIG"; then
        SIGNED_VIA="YubiKey (on-card)"
    elif [ "$YUBIKEY" = 1 ]; then
        echo "!! --yubikey: card sign failed and fallback is disabled." >&2; exit 4
    else
        echo "  card sign didn't complete — falling back to the local key." >&2
    fi
fi
if [ -z "$SIGNED_VIA" ]; then
    if [ -n "$EXPECT$EXPECT_YANK$EXPECT_META" ]; then
        # The review block above already refused anything but the named versions,
        # so the decision this would prompt for has been made and CHECKED.  Not a
        # skipped confirmation — a mechanical one.  `--expect-meta` belongs here for
        # exactly the same reason: its scope check is stricter than the prompt, since a
        # typed 'yes' verifies nothing while that check refuses a smuggled version, a
        # changed `yanked` array, and a named package that did not actually change.
        echo "  --expect satisfied: signing exactly ${EXPECT:-}${EXPECT:+ }${EXPECT_YANK:+yank }${EXPECT_YANK:-}${EXPECT_YANK:+ }${EXPECT_META:+metadata of }${EXPECT_META:-} with $(basename "$KEY")."
    elif [ "$YES" != 1 ]; then
        printf "  sign with the local key %s? type 'yes': " "$(basename "$KEY")"
        read -r ans
        case "$ans" in yes|YES) ;; *) echo "aborted — NOT signed."; exit 1;; esac
    fi
    [ -f "$KEY" ] || { echo "!! no card signature and no local key at $KEY — nothing to sign with." >&2; exit 2; }
    "$KG" sign --in "$INDEX" --key "$KEY" --out "$SIG"
    SIGNED_VIA="local key $(basename "$KEY")"
fi
SIGNED=1
echo "signed: $SIG  (via $SIGNED_VIA)"

# Trust gate — the signature must verify under a key that CLIENTS trust
# (src/registry_keys.rs::TRUSTED_PUBLIC_KEYS), not merely under the key we
# signed with.  Signing with an untrusted key yields a sig that every
# `loft install` rejects ("registry index signature INVALID").  The old check
# verified only against ${KEY}.pub AND was skipped entirely when that file was
# absent — so a wrong/untrusted key shipped silently (broke the live index
# 2026-06-28).  Refuse to commit/push unless a trusted key validates it.
KEYS_RS="$here/src/registry_keys.rs"
if [ -f "$KEYS_RS" ]; then
    trusted_hex=$(grep -oE '0x[0-9A-Fa-f]{2}' "$KEYS_RS" | sed 's/0x//' | tr -d '\n')
    ok=0; i=0
    while [ "$i" -lt "${#trusted_hex}" ]; do
        if "$KG" verify --in "$INDEX" --sig "$SIG" --pub "${trusted_hex:$i:64}" >/dev/null 2>&1; then ok=1; break; fi
        i=$((i + 64))
    done
    if [ "$ok" != 1 ]; then
        echo "!! registry-sign: the new signature verifies under NONE of the trusted" >&2
        echo "   keys in $KEYS_RS — every 'loft install' would reject this index." >&2
        echo "   Signing key: $KEY is not a registry trust-root key." >&2
        echo "   NOT committing/pushing.  Sign with a trusted key (set LOFT_REGISTRY_KEY" >&2
        echo "   / --key), or add this key's public to TRUSTED_PUBLIC_KEYS and ship a" >&2
        echo "   new client release first." >&2
        exit 3
    fi
    echo "  trust gate OK — signature verifies under a client-trusted key"
else
    echo "  WARNING: $KEYS_RS not found — skipping trust-set verification" >&2
    [ -f "$PUB" ] && "$KG" verify --in "$INDEX" --sig "$SIG" --pub "$(cat "$PUB")"
fi

# Automated git: stage index.json AND its signature together, commit, push.
# Ed25519 is deterministic, so re-signing identical content yields the same bytes
# → nothing to commit.  #377: staging only index.json.sig while a new/dirty
# index.json sat uncommitted silently published a sig/index mismatch — the
# committed .sig verified against content that never landed, and the new version
# was never published.  Staging BOTH keeps HEAD self-consistent: its committed
# index.json always matches its committed index.json.sig.
git -C "$REG_DIR" add index.json index.json.sig
if git -C "$REG_DIR" diff --cached --quiet; then
    echo "index.json.sig unchanged — nothing to commit."
    PUSHED=1   # nothing outstanding → safe to clean up the clone
elif [ "$PUSH" = 1 ]; then
    # The throwaway registry clone inherits NO identity when the maintainer has
    # only a per-repo (not global) git config, so a plain `git commit` aborts with
    # "Author identity unknown".  Resolve one — existing config → gh login → a
    # stable registry-signer fallback — and pass it inline.  Trust comes from the
    # Ed25519 signature, not the git author, so a derived/bot identity is fine; we
    # just need a valid one so the signing commit lands.
    sign_name=$(git -C "$REG_DIR" config user.name || true)
    sign_email=$(git -C "$REG_DIR" config user.email || true)
    if [ -z "$sign_name" ] || [ -z "$sign_email" ]; then
        gh_login=$(gh api user --jq '.login' 2>/dev/null || true)
        sign_name="${sign_name:-${gh_login:-loft-registry-signer}}"
        sign_email="${sign_email:-${gh_login:+$gh_login@users.noreply.github.com}}"
        sign_email="${sign_email:-loft-registry-signer@users.noreply.github.com}"
    fi
    git -C "$REG_DIR" -c user.name="$sign_name" -c user.email="$sign_email" \
        commit -q -m "${MSG:-sign: commit index.json + regenerate index.json.sig}"
    # Push reusing the gh login as git's credential helper: no username/password
    # prompt on an HTTPS remote (the registry is often cloned over HTTPS), and
    # harmless on an SSH remote (which uses your key).  `gh release create` etc.
    # work because gh uses API auth; plain `git push` uses git's credential
    # system, which without a helper falls back to prompting — and GitHub no
    # longer accepts a password there.  GIT_TERMINAL_PROMPT=0 makes a genuinely
    # missing credential fail fast instead of hanging on an interactive prompt.
    # CAS push (C96 single-writer): a concurrent push to the registry makes ours
    # non-fast-forward.  Retry by rebasing our signed commit onto the fetched tip —
    # a concurrent change that did NOT touch index.json rebases cleanly (our signed
    # index is unchanged, its signature still valid) and we re-push.  A genuine
    # index.json conflict means TWO signers raced (the single-writer invariant is
    # violated): abort and report rather than ever push a bad/unsigned index.  Only
    # a lost race is retried; a real failure (auth) breaks out immediately.
    push_ok=0
    for attempt in 1 2 3 4 5; do
        if GIT_TERMINAL_PROMPT=0 git -C "$REG_DIR" \
            -c credential.helper='!gh auth git-credential' push -q 2>/tmp/_rs_push.$$; then
            echo "committed + pushed: $(git -C "$REG_DIR" rev-parse --short HEAD) → $(git -C "$REG_DIR" remote get-url origin 2>/dev/null)"
            PUSHED=1; push_ok=1; break
        fi
        grep -qiE 'fetch first|non-fast-forward|rejected|behind' /tmp/_rs_push.$$ || break
        git -C "$REG_DIR" fetch -q origin || break
        if git -C "$REG_DIR" rebase -q '@{u}' 2>/tmp/_rs_reb.$$; then
            echo "   (concurrent push; rebased our signed commit onto the new tip — retry $attempt)" >&2
            continue
        fi
        git -C "$REG_DIR" rebase --abort 2>/dev/null || true
        echo "!! index.json changed underneath us — two signers raced (single-writer invariant violated)." >&2
        echo "   signed commit kept at $REG_DIR; reconcile the index by hand and re-run." >&2
        push_ok=2; break
    done
    if [ "$push_ok" = 0 ]; then
        echo "!! push FAILED: $(cat /tmp/_rs_push.$$ 2>/dev/null)" >&2
        echo "   signed commit kept at $REG_DIR — push it manually." >&2
        echo "   (auth? run 'gh auth setup-git' once, or use an SSH remote, then re-run.)" >&2
    fi
    rm -f "/tmp/_rs_push.$$" "/tmp/_rs_reb.$$"
else
    git -C "$REG_DIR" commit -q -m "${MSG:-sign: commit index.json + regenerate index.json.sig}"
    echo "committed (--no-push): $(git -C "$REG_DIR" rev-parse --short HEAD) at $REG_DIR — push when ready."
fi
