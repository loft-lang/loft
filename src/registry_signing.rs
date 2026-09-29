// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I77 — Registry / manifest / lockfile resolution

//! Ed25519 signature verification for the registry index.
//!
//! PKG.REG R3.5.  See
//! [PKG_REGISTRY_TRUST.md § Index signing](../doc/claude/PKG_REGISTRY_TRUST.md#index-signing--indexjsonsig)
//! for the threat model.  Short version: HTTPS to the registry host
//! is insufficient on its own — an attacker who controls the registry
//! repo (or successfully MITMs HTTPS) can swap `index.json` with
//! matching tarball sha256s.  Signing the index file with a key NOT
//! under the registry host's control closes that gap.
//!
//! The signature format is the **raw 64-byte Ed25519 signature** over
//! the bytes of `index.json` (no encoding wrapper).  Clients receive
//! it as `index.json.sig` from the same URL prefix as `index.json`.

#![cfg(feature = "registry")]

use ed25519_dalek::{SIGNATURE_LENGTH, Signature, VerifyingKey};

use crate::registry_keys::TRUSTED_PUBLIC_KEYS;

/// Result of a verification attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyResult {
    /// At least one trust-root key produced a valid signature.
    Valid,
    /// All embedded trust roots rejected the signature.  The index
    /// is unsigned or tampered.
    Invalid,
    /// This binary has no trust roots embedded yet
    /// (`TRUSTED_PUBLIC_KEYS` is empty).  Callers decide whether to
    /// proceed unsigned via an explicit `--allow-unsigned` flag.
    NoTrustRoot,
    /// The signature payload itself is malformed (wrong length,
    /// non-Ed25519 shape).  Distinguished from `Invalid` so error
    /// messages can be precise ("corrupt signature" vs "wrong
    /// signer").
    MalformedSignature,
}

/// Verify `index_bytes` against `sig_bytes` using every embedded
/// trust-root key.  Returns `Valid` when ANY key accepts the
/// signature.  Multiple trusted keys is the rotation mechanism —
/// during a transition window the index is signed by both old and
/// new keys.
///
/// Caller is responsible for translating `NoTrustRoot` to the
/// appropriate behaviour (refuse install unless `--allow-unsigned`).
#[must_use]
pub fn verify_index(index_bytes: &[u8], sig_bytes: &[u8]) -> VerifyResult {
    verify_index_with_keys(TRUSTED_PUBLIC_KEYS, index_bytes, sig_bytes)
}

/// Core of [`verify_index`], parameterised on the trust-root key set so the
/// `NoTrustRoot` (empty-set) and `Valid` branches stay testable independent of
/// the compile-time `TRUSTED_PUBLIC_KEYS` — which is non-empty in a bootstrapped
/// binary, so a test can no longer reach the empty case through it.
fn verify_index_with_keys(keys: &[[u8; 32]], index_bytes: &[u8], sig_bytes: &[u8]) -> VerifyResult {
    if sig_bytes.len() != SIGNATURE_LENGTH {
        return VerifyResult::MalformedSignature;
    }
    let Ok(sig_array): Result<[u8; SIGNATURE_LENGTH], _> = sig_bytes.try_into() else {
        return VerifyResult::MalformedSignature;
    };
    let signature = Signature::from_bytes(&sig_array);

    if keys.is_empty() {
        return VerifyResult::NoTrustRoot;
    }

    for pk_bytes in keys {
        let Ok(verifying_key) = VerifyingKey::from_bytes(pk_bytes) else {
            continue;
        };
        if verifying_key.verify_strict(index_bytes, &signature).is_ok() {
            return VerifyResult::Valid;
        }
    }
    VerifyResult::Invalid
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn malformed_signature_detected() {
        let result = verify_index(b"any bytes", &[1, 2, 3]); // not 64 bytes
        assert!(matches!(result, VerifyResult::MalformedSignature));
    }

    #[test]
    fn empty_trust_roots_yield_no_trust_root() {
        // A bootstrapped binary's TRUSTED_PUBLIC_KEYS is non-empty, so exercise
        // the NoTrustRoot branch through the parameterised core with an explicit
        // empty key set.
        let sig = [0u8; SIGNATURE_LENGTH];
        let result = verify_index_with_keys(&[], b"any bytes", &sig);
        assert!(matches!(result, VerifyResult::NoTrustRoot));
    }

    #[test]
    fn non_empty_trust_root_verifies_a_matching_signature() {
        // The Valid branch via the parameterised core: a signature made by an
        // embedded key verifies (what `verify_index` does in production).
        let signing = SigningKey::from_bytes(&[0x42u8; 32]);
        let pk = signing.verifying_key().to_bytes();
        let msg = b"index payload";
        let sig = signing.sign(msg).to_bytes();
        let result = verify_index_with_keys(&[pk], msg, &sig);
        assert!(matches!(result, VerifyResult::Valid));
    }

    /// Round-trip test against a hard-coded keypair seed.  Validates
    /// the cryptographic core that the public `verify_index` wraps
    /// around — when `TRUSTED_PUBLIC_KEYS` gets its first real entry,
    /// this same code path runs against the embedded key.
    #[test]
    fn signs_and_verifies_against_known_key() {
        // Deterministic seed so the test is reproducible — production
        // keys are generated with a real CSPRNG offline.
        let seed = [0x42u8; 32];
        let signing = SigningKey::from_bytes(&seed);
        let verifying = signing.verifying_key();
        let message = b"test index payload";
        let sig: Signature = signing.sign(message);

        // Inner-path verify (what the production trust root does
        // once a key is embedded).
        assert!(verifying.verify_strict(message, &sig).is_ok());

        // Bad signature: tampered byte → reject.
        let mut bad = sig.to_bytes();
        bad[0] ^= 0xff;
        let bad_sig = Signature::from_bytes(&bad);
        assert!(verifying.verify_strict(message, &bad_sig).is_err());

        // Tampered message → reject.
        assert!(verifying.verify_strict(b"tampered", &sig).is_err());
    }
}
