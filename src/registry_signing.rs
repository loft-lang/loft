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
//! The signature format is the **raw 64-byte Ed25519 signature**, in two forms during a
//! transition:
//!
//! - `index.json.sig2` / `advisories.json.sig2` — over the document's bytes PREFIXED with its
//!   purpose ([`Purpose::domain`]: `loft-registry-index-v1\0` or
//!   `loft-registry-advisories-v1\0`), so a signature made for one document cannot be read as
//!   the other's, nor as any future document's the same keys sign;
//! - `index.json.sig` / `advisories.json.sig` — over the raw bytes, as every loft before the
//!   prefix reads it.
//!
//! The registry publishes both until the clients that read only the raw form are gone; a
//! client verifies the prefixed one when it is there, and the raw one only while
//! [`RAW_SIGNATURE_ACCEPTED`] holds.  Inside a client the two travel as ONE value through the
//! code that fetches, caches and checks them ([`join_signatures`] / [`split_signatures`]); only
//! the file readers and writers know there are two files.

#![cfg(feature = "registry")]

/// What a registry signature vouches for: the prefix of the message the keys sign.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Index,
    Advisories,
}

impl Purpose {
    /// The bytes signed before the document's own: its purpose, a version, a NUL.
    #[must_use]
    pub fn domain(self) -> &'static [u8] {
        match self {
            Purpose::Index => b"loft-registry-index-v1\0",
            Purpose::Advisories => b"loft-registry-advisories-v1\0",
        }
    }
}

/// The message a prefixed signature (`.sig2`) is made over: the purpose, then the document.
#[must_use]
pub fn signed_message(purpose: Purpose, document: &[u8]) -> Vec<u8> {
    let mut m = purpose.domain().to_vec();
    m.extend_from_slice(document);
    m
}

/// Whether an index or feed carrying only the RAW signature (no `.sig2`) is still accepted:
/// the transition clause.  True while the registry publishes both forms and older clients
/// read only the raw one; set to false in the first loft release after every registry
/// document carries a `.sig2` and the release before it shipped this reader.
pub const RAW_SIGNATURE_ACCEPTED: bool = true;

/// The marker that opens a joined pair in memory: never the start of a raw signature in
/// practice and, more to the point, never 64 bytes long with it.
const PAIR_MARK: &[u8] = b"LOFTSIG2";

/// The raw (`.sig`) and prefixed (`.sig2`) signatures as one value: the raw one alone when
/// there is no prefixed one, so an older cache or bundle reads as it always did.
#[must_use]
pub fn join_signatures(raw: &[u8], prefixed: &[u8]) -> Vec<u8> {
    if prefixed.is_empty() {
        return raw.to_vec();
    }
    let mut v = PAIR_MARK.to_vec();
    v.extend_from_slice(prefixed);
    v.extend_from_slice(raw);
    v
}

/// The two signatures [`join_signatures`] made: `(raw, prefixed)`, the second empty when the
/// value is a raw signature alone.
#[must_use]
pub fn split_signatures(sig: &[u8]) -> (&[u8], &[u8]) {
    match sig.strip_prefix(PAIR_MARK) {
        Some(rest) if rest.len() >= SIGNATURE_LENGTH => {
            let (prefixed, raw) = rest.split_at(SIGNATURE_LENGTH);
            (raw, prefixed)
        }
        _ => (sig, &[]),
    }
}

/// The verdict on a registry document and its signatures ([`join_signatures`]): the prefixed
/// signature decides whenever it is there — a valid raw one beside a wrong prefixed one is a
/// wrong signature — and the raw one only while [`RAW_SIGNATURE_ACCEPTED`] holds; after that,
/// a document with only a raw signature reads as unsigned.
#[must_use]
pub fn verify_document(purpose: Purpose, document: &[u8], sig: &[u8]) -> VerifyResult {
    verify_document_with_keys(
        TRUSTED_PUBLIC_KEYS,
        purpose,
        document,
        sig,
        RAW_SIGNATURE_ACCEPTED,
    )
}

fn verify_document_with_keys(
    keys: &[[u8; 32]],
    purpose: Purpose,
    document: &[u8],
    sig: &[u8],
    raw_accepted: bool,
) -> VerifyResult {
    let (raw, prefixed) = split_signatures(sig);
    if !prefixed.is_empty() {
        return verify_index_with_keys(keys, &signed_message(purpose, document), prefixed);
    }
    if raw_accepted || raw.is_empty() {
        return verify_index_with_keys(keys, document, raw);
    }
    // Past the transition a raw signature alone is not a signature this client reads: it
    // answers as a malformed one, which every caller refuses even with `--allow-unsigned`.
    verify_index_with_keys(keys, document, &[])
}

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

    /// The contract of the transition: the prefixed signature decides when it is there,
    /// the raw one while the transition lasts, and a signature made for one purpose never
    /// verifies a document of the other.
    #[test]
    fn a_prefixed_signature_binds_its_purpose() {
        let signing = SigningKey::from_bytes(&[0x42u8; 32]);
        let keys = [signing.verifying_key().to_bytes()];
        let doc = b"{\"schema_version\": 1}";
        let raw = signing.sign(doc).to_bytes();
        let for_index = signing
            .sign(&signed_message(Purpose::Index, doc))
            .to_bytes();
        let both = join_signatures(&raw, &for_index);
        assert_eq!(split_signatures(&both), (&raw[..], &for_index[..]));
        assert_eq!(
            split_signatures(&raw),
            (&raw[..], &[][..]),
            "a raw signature alone"
        );
        let v = |p, s: &[u8], raw_ok| verify_document_with_keys(&keys, p, doc, s, raw_ok);
        assert_eq!(v(Purpose::Index, &both, true), VerifyResult::Valid);
        assert_eq!(v(Purpose::Index, &both, false), VerifyResult::Valid);
        // The index's prefixed signature is no signature of the same bytes as a feed.
        assert_eq!(v(Purpose::Advisories, &both, true), VerifyResult::Invalid);
        // A wrong prefixed signature beside a right raw one is wrong, not a fallback.
        let mut bad = for_index;
        bad[0] ^= 1;
        assert_eq!(
            v(Purpose::Index, &join_signatures(&raw, &bad), true),
            VerifyResult::Invalid
        );
        // The raw signature alone: accepted during the transition, unsigned after it.
        assert_eq!(v(Purpose::Index, &raw, true), VerifyResult::Valid);
        assert_eq!(
            v(Purpose::Index, &raw, false),
            VerifyResult::MalformedSignature
        );
    }

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
