// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I77 — Registry / manifest / lockfile resolution

//! Trust-root public keys for the loft package registry.
//!
//! PKG.REG R3.5.  See
//! [PKG_REGISTRY_TRUST.md § Index signing](../doc/claude/PKG_REGISTRY_TRUST.md#index-signing--indexjsonsig)
//! for the threat model + key-rotation procedure.
//!
//! Each entry is a 32-byte Ed25519 public key (raw, NOT PEM/DER).
//! The corresponding private key is held by the loft maintainers and
//! used by the `loft-lang/registry` CI workflow to sign
//! `index.json` → `index.json.sig`.  Clients verify the signature
//! before trusting any data in the index.
//!
//! Multiple keys can be embedded simultaneously so key rotation is
//! seamless: during the transition window the index is signed by
//! BOTH the old and new keys, and clients accept either.  Once the
//! ecosystem has upgraded past the rotation point, the old key is
//! removed in a subsequent loft release.
//!
//! ## Current trust roots
//!
//! The list is **bootstrapped** (REGISTRY_BOOTSTRAP.md § Step 1.5) and holds
//! several INDEPENDENT keys: a client accepts a signature from ANY of them, so
//! signers are added or revoked without rotating the others.  Two kinds:
//!
//! - **YubiKey on-card keys** — non-extractable, high-assurance; used for
//!   release signing where the hardware token is to hand.
//! - **Per-laptop software signers** — one unique Ed25519 key per maintainer
//!   laptop (`~/.loft/trust-root/registry-signing-key.bin` on each), so a
//!   laptop can sign/release remotely WITHOUT YubiKey access.  Each is labelled
//!   by its machine below.
//!
//! Add a signer: generate its keypair (`loft-keygen generate`), append the
//! 32-byte public key here, ship a loft release — future binaries trust it
//! automatically.  Revoke a lost/retired laptop: delete just its entry and ship
//! a release; the others keep signing.  The private halves NEVER enter this
//! file or the repo.
//!
//! When this list is empty (a fresh tree before bootstrap), clients invoking
//! signature-verified flows MUST pass `--allow-unsigned` explicitly — a missing
//! trust root is a configuration error, not a "trust everything" signal.

/// 32-byte Ed25519 public keys embedded at compile time, one entry per
/// trusted signer (YubiKey or per-laptop software key).  Format:
/// `[key_bytes; 32]` per entry; a signature verifies if it matches ANY entry.
///
/// `&'static [[u8; 32]]` keeps the type explicit so adding a key is
/// just appending a new 32-byte literal — no allocations, no parsing,
/// trivially zero-runtime-cost.
pub const TRUSTED_PUBLIC_KEYS: &[[u8; 32]] = &[
    // Trust root bootstrapped 2026-06-13 (REGISTRY_BOOTSTRAP.md § Step 1.5) —
    // THREE INDEPENDENT keys: the client accepts a signature from ANY of them,
    // and a lost device's key is REVOKED by deleting just its entry here and
    // shipping a release (the other two keep signing — no rotation).
    //
    // K_laptop — software daily-signer (~/.loft/trust-root/registry-signing-key.bin)
    [
        0xa8, 0x8b, 0xb7, 0xc5, 0x45, 0x46, 0x8c, 0xd5, 0x44, 0x79, 0xa3, 0x9e, 0x3b, 0x29, 0x73,
        0x91, 0xa9, 0xc6, 0x2e, 0x17, 0x79, 0xfb, 0xe9, 0xdf, 0x2f, 0x35, 0xdf, 0xf2, 0xa4, 0x51,
        0xc0, 0x75,
    ],
    // K_laptop_tuxedo — per-laptop software daily-signer on the `tuxedo` machine
    // (~/.loft/trust-root/registry-signing-key.bin there).  Lets that laptop sign
    // releases remotely without YubiKey access; revoke by deleting this entry +
    // shipping a release (the other signers keep working — no rotation).
    [
        0x48, 0xdc, 0xff, 0xef, 0x19, 0x01, 0xef, 0xe4, 0xa5, 0x42, 0x6a, 0x8f, 0xd5, 0x38, 0xfc,
        0x76, 0xd5, 0xe5, 0x3f, 0xb2, 0x64, 0x17, 0xbe, 0xdf, 0x7e, 0x0e, 0xf9, 0xd3, 0x26, 0x6e,
        0x9d, 0x50,
    ],
    // K_yubiA — on-card, non-extractable (YubiKey serial 37327945, PIV slot 9C)
    [
        0x25, 0x6c, 0x53, 0x90, 0x26, 0x54, 0x65, 0xc5, 0x7d, 0x1f, 0x96, 0x46, 0xfe, 0x58, 0x1c,
        0x14, 0xbd, 0xdd, 0xb1, 0x62, 0x03, 0x5a, 0x60, 0x45, 0xb3, 0x5a, 0x94, 0x23, 0x6b, 0x3d,
        0x52, 0x8d,
    ],
    // K_yubiB — on-card, non-extractable (second YubiKey, PIV slot 9C)
    [
        0xbd, 0xf9, 0x91, 0xd2, 0x02, 0x86, 0x73, 0x36, 0xbf, 0x9a, 0x70, 0x20, 0x29, 0x28, 0x65,
        0x04, 0x5f, 0x62, 0xc9, 0x75, 0x1b, 0x7f, 0x09, 0xf3, 0x48, 0x7e, 0x7e, 0x66, 0xd2, 0xfa,
        0xc0, 0x17,
    ],
];

/// True when the binary embeds at least one trust root and can verify
/// registry signatures.  When false, all signature-requiring code
/// paths refuse to run unless the caller explicitly opted out via
/// `--allow-unsigned`.
#[must_use]
pub fn has_trust_root() -> bool {
    !TRUSTED_PUBLIC_KEYS.is_empty()
}
