#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# registry-signer-lib.sh — signing with the registry key on the YubiKey, sourced by
# scripts/registry-sign.sh (a publish) and scripts/registry-heartbeat.sh (a re-stamp).
# Not run on its own.

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
