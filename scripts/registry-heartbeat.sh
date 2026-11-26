#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# registry-heartbeat.sh — re-sign the registry's index.json and advisories.json with their
# `updated` set to now, and NOTHING else changed.
#
# A signature says who wrote a document, not when it was read, so an old signed advisory feed
# served again looks as good as the current one.  The registry is re-signed at least weekly,
# so a signed `updated` older than that is not the current document: the PR check
# (scripts/check-registry-heartbeat.py, the `registry-heartbeat` job of ci.yml) goes red at
# 7 days, and the PR routine runs this script before a PR is opened (DEVELOPMENT.md § Opening
# a PR).  PKG_REGISTRY_TRUST.md § The heartbeat has the reasoning.
#
# Signing stays on the maintainer's machine (PKG_REGISTRY_TRUST.md § Why laptop signing), by
# the same signer as a publish: the YubiKey when one is present (four touches — `.sig` and
# `.sig2` of each document), else the key file (LOFT_REGISTRY_SIGNER=file forces it).  There
# is no review prompt, because what it signs is checked mechanically instead: the change is
# exactly the `updated` stamp of the two documents, or the run refuses.  Any other change in
# the checkout is refused too: it is a publish, which goes through scripts/registry-sign.sh
# and its review.  The signatures must verify under a client-trusted key before anything is
# committed.
#
#   scripts/registry-heartbeat.sh [opts]
#     --registry-dir DIR   a clean registry checkout (default: a fresh clone)
#     --key FILE           the Ed25519 key file, when no YubiKey signs (default:
#                          $LOFT_REGISTRY_KEY or ~/.loft/trust-root/registry-signing-key.bin)
#     --min-age HOURS      do nothing when both documents were signed less than HOURS ago
#                          (default 20: one heartbeat a day, however many PRs)
#     --no-push            sign and commit, but do not push
#
# Exit: 0 signed and pushed, or nothing to do · 1 refused · 2 usage / no signer ·
# 3 the signatures verify under no client-trusted key · 4 the YubiKey did not sign
# (nothing committed on 3 or 4).
set -euo pipefail

REG_DIR=""; MIN_AGE=20; PUSH=1
KEY="${LOFT_REGISTRY_KEY:-$HOME/.loft/trust-root/registry-signing-key.bin}"
while [ $# -gt 0 ]; do
    case "$1" in
        --registry-dir) REG_DIR="$2"; shift;;
        --key)          KEY="$2"; shift;;
        --min-age)      MIN_AGE="$2"; shift;;
        --no-push)      PUSH=0;;
        -h|--help)      sed -n '2,34p' "$0"; exit 0;;
        *) echo "registry-heartbeat: unknown argument: $1" >&2; exit 2;;
    esac
    shift
done
here=$(cd "$(dirname "$0")/.." && pwd)
# shellcheck source=registry-signer-lib.sh
. "$(dirname "$0")/registry-signer-lib.sh"
USE_CARD=0
[ "${LOFT_REGISTRY_SIGNER:-}" != file ] && yubikey_available && USE_CARD=1
if [ "$USE_CARD" = 0 ] && [ ! -f "$KEY" ]; then
    echo "registry-heartbeat: no YubiKey signer and no key file at $KEY" >&2
    exit 2
fi
KG="$here/target/release/loft-keygen"
if [ ! -x "$KG" ]; then
    echo "building loft-keygen (release) ..." >&2
    (cd "$here" && cargo build --release --bin loft-keygen --features registry >/dev/null)
fi

CLONED=0
if [ -z "$REG_DIR" ]; then
    REG_DIR=$(mktemp -d -t loft-registry-heartbeat.XXXXXX)
    git clone -q git@github.com:loft-lang/registry.git "$REG_DIR" 2>/dev/null \
        || gh repo clone loft-lang/registry "$REG_DIR" -- -q 2>/dev/null \
        || git clone -q https://github.com/loft-lang/registry "$REG_DIR"
    CLONED=1
fi
WORK=$(mktemp -d -t loft-heartbeat-msg.XXXXXX)
cleanup() {
    rm -rf "$WORK"
    [ "$CLONED" = 1 ] && [ "${KEEP:-0}" != 1 ] && rm -rf "$REG_DIR"
    return 0
}
trap cleanup EXIT

# A clean checkout at the tip of main: a heartbeat signs what is published, nothing else.
git -C "$REG_DIR" fetch -q origin main
if [ -n "$(git -C "$REG_DIR" status --porcelain)" ]; then
    echo "registry-heartbeat: $REG_DIR has uncommitted changes — a publish goes through" >&2
    echo "  scripts/registry-sign.sh; refusing." >&2
    exit 1
fi
if [ "$(git -C "$REG_DIR" rev-parse HEAD)" != "$(git -C "$REG_DIR" rev-parse origin/main)" ]; then
    git -C "$REG_DIR" checkout -q --detach origin/main
fi

DOCS="index.json advisories.json"
# Put the checkout back as it was found (it was clean): on every refusal after the restamp.
restore() {
    git -C "$REG_DIR" checkout -q -- .
    git -C "$REG_DIR" clean -fq
}
NOW=$(date -u +%Y-%m-%dT%H:%M:%SZ)

# Restamp each document: only its top-level `updated` changes, in place, so the bytes around
# it — and every other field — are exactly what was signed before.  Refuses a document whose
# first `updated` is not its top-level one, or a restamp that changed anything else.
rc=0
python3 - "$REG_DIR" "$NOW" "$MIN_AGE" $DOCS <<'PY' || rc=$?
import datetime, json, re, sys
reg, now, min_age, docs = sys.argv[1], sys.argv[2], float(sys.argv[3]), sys.argv[4:]
stamp = re.compile(r'("updated"\s*:\s*")([^"]*)(")')
fmt = "%Y-%m-%dT%H:%M:%SZ"
ages, texts = [], {}
for name in docs:
    path = f"{reg}/{name}"
    text = open(path, encoding="utf-8").read()
    top = json.loads(text)
    m = stamp.search(text)
    if not m or top.get("updated") != m.group(2):
        sys.exit(f"registry-heartbeat: {name}'s first `updated` is not its top-level one; refusing")
    then = datetime.datetime.strptime(m.group(2), fmt).replace(tzinfo=datetime.timezone.utc)
    ages.append((datetime.datetime.now(datetime.timezone.utc) - then).total_seconds() / 3600)
    new = text[: m.start(2)] + now + text[m.end(2):]
    after = json.loads(new)
    top["updated"] = now
    if after != top:
        sys.exit(f"registry-heartbeat: restamping {name} changed more than `updated`; refusing")
    texts[name] = new
if max(ages) < min_age:
    print(f"registry-heartbeat: signed {max(ages):.0f} h ago at most — nothing to do")
    sys.exit(10)
for name, new in texts.items():
    open(f"{reg}/{name}", "w", encoding="utf-8", newline="").write(new)
PY
[ "$rc" = 10 ] && exit 0
[ "$rc" = 0 ] || exit 1

# The one diff a heartbeat may make: the `updated` line of each document.
changed=$(git -C "$REG_DIR" diff --numstat | awk '{print $1+$2, $3}')
while read -r lines file; do
    [ -z "$file" ] && continue
    if [ "$lines" != 2 ]; then
        echo "registry-heartbeat: $file changed $lines lines, not its \`updated\` line; refusing" >&2
        restore
        exit 1
    fi
done <<<"$changed"

# Sign: the raw signature over the document, and the purpose-prefixed `.sig2` over its message
# (src/registry_signing.rs), as scripts/registry-sign.sh does for the index.
sign() {  # <in> <out>
    if [ "$USE_CARD" = 1 ]; then
        yubikey_sign "$1" "$2" || {
            echo "registry-heartbeat: the YubiKey did not sign — nothing committed" >&2
            restore
            exit 4
        }
    else
        "$KG" sign --in "$1" --key "$KEY" --out "$2" >/dev/null
    fi
}
[ "$USE_CARD" = 1 ] && echo "registry-heartbeat: signing on the YubiKey — touch it four times" >&2
for doc in $DOCS; do
    purpose=index; [ "$doc" = advisories.json ] && purpose=advisories
    sign "$REG_DIR/$doc" "$REG_DIR/$doc.sig"
    "$KG" message --purpose "$purpose" --in "$REG_DIR/$doc" --out "$WORK/$doc.msg"
    sign "$WORK/$doc.msg" "$REG_DIR/$doc.sig2"
done

# Trust gate: every signature verifies under ONE key clients trust
# (src/registry_keys.rs::TRUSTED_PUBLIC_KEYS), or nothing is committed.
trusted_hex=$(grep -oE '0x[0-9A-Fa-f]{2}' "$here/src/registry_keys.rs" | sed 's/0x//' | tr -d '\n')
ok=0; i=0
while [ "$i" -lt "${#trusted_hex}" ]; do
    pub="${trusted_hex:$i:64}"; all=1
    for doc in $DOCS; do
        "$KG" verify --in "$REG_DIR/$doc" --sig "$REG_DIR/$doc.sig" --pub "$pub" >/dev/null 2>&1 \
            && "$KG" verify --in "$WORK/$doc.msg" --sig "$REG_DIR/$doc.sig2" --pub "$pub" >/dev/null 2>&1 \
            || all=0
    done
    [ "$all" = 1 ] && { ok=1; break; }
    i=$((i + 64))
done
if [ "$ok" != 1 ]; then
    echo "registry-heartbeat: the signatures verify under no client-trusted key — not committing" >&2
    restore
    exit 3
fi

files=""
for doc in $DOCS; do files="$files $doc $doc.sig $doc.sig2"; done
# shellcheck disable=SC2086
git -C "$REG_DIR" add $files
name=$(git -C "$REG_DIR" config user.name || gh api user --jq .login 2>/dev/null || echo loft-registry-signer)
email=$(git -C "$REG_DIR" config user.email || echo "$name@users.noreply.github.com")
git -C "$REG_DIR" -c user.name="$name" -c user.email="$email" \
    commit -q -m "heartbeat: index.json and advisories.json re-signed at $NOW"
if [ "$PUSH" != 1 ]; then
    KEEP=1
    echo "registry-heartbeat: committed (not pushed) at $REG_DIR"
    exit 0
fi
# A publish that lands first makes this push non-fast-forward: the heartbeat is then
# redundant for the documents that publish re-signed, so it is dropped and the next run
# restamps on top of the new tip.
if GIT_TERMINAL_PROMPT=0 git -C "$REG_DIR" -c credential.helper='!gh auth git-credential' \
    push -q origin HEAD:main; then
    echo "registry-heartbeat: re-signed at $NOW ($(git -C "$REG_DIR" rev-parse --short HEAD))"
else
    echo "registry-heartbeat: the push was refused (a concurrent publish?); run it again" >&2
    exit 1
fi
