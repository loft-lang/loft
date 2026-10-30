// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C — the TLS client a unit links

//! The TLS of the C runtime, client and server (@PLN203 C5): `src/c/tls/loft_tls.c` over BearSSL's engine,
//! vendored under `src/c/tls/bearssl/` from the same upstream the crypto library vendors its
//! primitives from (BearSSL v0.6-39-g7bea48e, MIT).  The subset is the files `loft_tls.c`'s
//! client and server profiles link on any host — the union of what the linker pulls in with
//! the 64-bit multiply, AES-NI and SSE2 code and without them (read off a link map against the
//! whole upstream tree), plus the POWER8 variants — and the guard
//! `the_tls_sources_are_the_directory` keeps this list and the directory one set.
//! `tls/loft_ssh.c`, the SSH client of the `net` component (`src/c/loft_ssh.h`), is built into
//! the same archive: it uses only primitives this subset already carries, and Ed25519 of its own.
//!
//! The client is not part of a unit's translation unit: BearSSL is a library of `static` helpers
//! across files, and compiling it into every unit that fetches a URL would cost seconds a build.
//! It is built once per compiler into a static archive under `~/.loft/c-tls/` (honouring
//! `LOFT_HOME`, as the other user caches do), by the archiver a `[c] shim` uses, and a unit that
//! fetches over a URL names it on a `link:` line ([`unit_lines`]).

use std::sync::OnceLock;

/// The sources, by the flat name each is written under: the `.c` files are compiled, the headers
/// are found beside them by their `#include "…"`.
const SOURCES: &[(&str, &str)] = &[
    ("loft_tls.c", include_str!("tls/loft_tls.c")),
    ("loft_ssh.c", include_str!("tls/loft_ssh.c")),
    // The vendored BearSSL subset, one entry per file of `tls/bearssl/`.
    ("aes_ct.c", include_str!("tls/bearssl/aes_ct.c")),
    ("aes_ct64.c", include_str!("tls/bearssl/aes_ct64.c")),
    (
        "aes_ct64_cbcdec.c",
        include_str!("tls/bearssl/aes_ct64_cbcdec.c"),
    ),
    (
        "aes_ct64_cbcenc.c",
        include_str!("tls/bearssl/aes_ct64_cbcenc.c"),
    ),
    ("aes_ct64_ctr.c", include_str!("tls/bearssl/aes_ct64_ctr.c")),
    ("aes_ct64_dec.c", include_str!("tls/bearssl/aes_ct64_dec.c")),
    ("aes_ct64_enc.c", include_str!("tls/bearssl/aes_ct64_enc.c")),
    (
        "aes_ct_cbcdec.c",
        include_str!("tls/bearssl/aes_ct_cbcdec.c"),
    ),
    (
        "aes_ct_cbcenc.c",
        include_str!("tls/bearssl/aes_ct_cbcenc.c"),
    ),
    ("aes_ct_ctr.c", include_str!("tls/bearssl/aes_ct_ctr.c")),
    ("aes_ct_dec.c", include_str!("tls/bearssl/aes_ct_dec.c")),
    ("aes_ct_enc.c", include_str!("tls/bearssl/aes_ct_enc.c")),
    ("aes_pwr8.c", include_str!("tls/bearssl/aes_pwr8.c")),
    (
        "aes_pwr8_cbcdec.c",
        include_str!("tls/bearssl/aes_pwr8_cbcdec.c"),
    ),
    (
        "aes_pwr8_cbcenc.c",
        include_str!("tls/bearssl/aes_pwr8_cbcenc.c"),
    ),
    ("aes_pwr8_ctr.c", include_str!("tls/bearssl/aes_pwr8_ctr.c")),
    ("aes_x86ni.c", include_str!("tls/bearssl/aes_x86ni.c")),
    (
        "aes_x86ni_cbcdec.c",
        include_str!("tls/bearssl/aes_x86ni_cbcdec.c"),
    ),
    (
        "aes_x86ni_cbcenc.c",
        include_str!("tls/bearssl/aes_x86ni_cbcenc.c"),
    ),
    (
        "aes_x86ni_ctr.c",
        include_str!("tls/bearssl/aes_x86ni_ctr.c"),
    ),
    ("bearssl.h", include_str!("tls/bearssl/bearssl.h")),
    ("bearssl_aead.h", include_str!("tls/bearssl/bearssl_aead.h")),
    (
        "bearssl_block.h",
        include_str!("tls/bearssl/bearssl_block.h"),
    ),
    ("bearssl_ec.h", include_str!("tls/bearssl/bearssl_ec.h")),
    ("bearssl_hash.h", include_str!("tls/bearssl/bearssl_hash.h")),
    ("bearssl_hmac.h", include_str!("tls/bearssl/bearssl_hmac.h")),
    ("bearssl_kdf.h", include_str!("tls/bearssl/bearssl_kdf.h")),
    ("bearssl_pem.h", include_str!("tls/bearssl/bearssl_pem.h")),
    ("bearssl_prf.h", include_str!("tls/bearssl/bearssl_prf.h")),
    ("bearssl_rand.h", include_str!("tls/bearssl/bearssl_rand.h")),
    ("bearssl_rsa.h", include_str!("tls/bearssl/bearssl_rsa.h")),
    ("bearssl_ssl.h", include_str!("tls/bearssl/bearssl_ssl.h")),
    ("bearssl_x509.h", include_str!("tls/bearssl/bearssl_x509.h")),
    ("ccopy.c", include_str!("tls/bearssl/ccopy.c")),
    ("chacha20_ct.c", include_str!("tls/bearssl/chacha20_ct.c")),
    (
        "chacha20_sse2.c",
        include_str!("tls/bearssl/chacha20_sse2.c"),
    ),
    ("config.h", include_str!("tls/bearssl/config.h")),
    ("dec32be.c", include_str!("tls/bearssl/dec32be.c")),
    ("dec32le.c", include_str!("tls/bearssl/dec32le.c")),
    ("dec64be.c", include_str!("tls/bearssl/dec64be.c")),
    ("ec_all_m31.c", include_str!("tls/bearssl/ec_all_m31.c")),
    (
        "ec_c25519_m31.c",
        include_str!("tls/bearssl/ec_c25519_m31.c"),
    ),
    (
        "ec_c25519_m64.c",
        include_str!("tls/bearssl/ec_c25519_m64.c"),
    ),
    ("ec_default.c", include_str!("tls/bearssl/ec_default.c")),
    ("ec_p256_m31.c", include_str!("tls/bearssl/ec_p256_m31.c")),
    ("ec_p256_m64.c", include_str!("tls/bearssl/ec_p256_m64.c")),
    ("ec_prime_i31.c", include_str!("tls/bearssl/ec_prime_i31.c")),
    ("ec_pubkey.c", include_str!("tls/bearssl/ec_pubkey.c")),
    ("ec_secp256r1.c", include_str!("tls/bearssl/ec_secp256r1.c")),
    ("ec_secp384r1.c", include_str!("tls/bearssl/ec_secp384r1.c")),
    ("ec_secp521r1.c", include_str!("tls/bearssl/ec_secp521r1.c")),
    ("ecdsa_atr.c", include_str!("tls/bearssl/ecdsa_atr.c")),
    (
        "ecdsa_default_sign_asn1.c",
        include_str!("tls/bearssl/ecdsa_default_sign_asn1.c"),
    ),
    (
        "ecdsa_i31_bits.c",
        include_str!("tls/bearssl/ecdsa_i31_bits.c"),
    ),
    (
        "ecdsa_i31_sign_asn1.c",
        include_str!("tls/bearssl/ecdsa_i31_sign_asn1.c"),
    ),
    (
        "ecdsa_i31_sign_raw.c",
        include_str!("tls/bearssl/ecdsa_i31_sign_raw.c"),
    ),
    (
        "ecdsa_i31_vrfy_asn1.c",
        include_str!("tls/bearssl/ecdsa_i31_vrfy_asn1.c"),
    ),
    (
        "ecdsa_i31_vrfy_raw.c",
        include_str!("tls/bearssl/ecdsa_i31_vrfy_raw.c"),
    ),
    ("ecdsa_rta.c", include_str!("tls/bearssl/ecdsa_rta.c")),
    ("enc32be.c", include_str!("tls/bearssl/enc32be.c")),
    ("enc32le.c", include_str!("tls/bearssl/enc32le.c")),
    ("enc64be.c", include_str!("tls/bearssl/enc64be.c")),
    ("ghash_ctmul.c", include_str!("tls/bearssl/ghash_ctmul.c")),
    (
        "ghash_ctmul64.c",
        include_str!("tls/bearssl/ghash_ctmul64.c"),
    ),
    ("ghash_pclmul.c", include_str!("tls/bearssl/ghash_pclmul.c")),
    ("ghash_pwr8.c", include_str!("tls/bearssl/ghash_pwr8.c")),
    ("hmac.c", include_str!("tls/bearssl/hmac.c")),
    ("hmac_ct.c", include_str!("tls/bearssl/hmac_ct.c")),
    ("hmac_drbg.c", include_str!("tls/bearssl/hmac_drbg.c")),
    ("i31_add.c", include_str!("tls/bearssl/i31_add.c")),
    ("i31_bitlen.c", include_str!("tls/bearssl/i31_bitlen.c")),
    ("i31_decmod.c", include_str!("tls/bearssl/i31_decmod.c")),
    ("i31_decode.c", include_str!("tls/bearssl/i31_decode.c")),
    ("i31_decred.c", include_str!("tls/bearssl/i31_decred.c")),
    ("i31_encode.c", include_str!("tls/bearssl/i31_encode.c")),
    ("i31_fmont.c", include_str!("tls/bearssl/i31_fmont.c")),
    ("i31_iszero.c", include_str!("tls/bearssl/i31_iszero.c")),
    ("i31_modpow.c", include_str!("tls/bearssl/i31_modpow.c")),
    ("i31_modpow2.c", include_str!("tls/bearssl/i31_modpow2.c")),
    ("i31_montmul.c", include_str!("tls/bearssl/i31_montmul.c")),
    ("i31_mulacc.c", include_str!("tls/bearssl/i31_mulacc.c")),
    ("i31_muladd.c", include_str!("tls/bearssl/i31_muladd.c")),
    ("i31_ninv31.c", include_str!("tls/bearssl/i31_ninv31.c")),
    ("i31_reduce.c", include_str!("tls/bearssl/i31_reduce.c")),
    ("i31_rshift.c", include_str!("tls/bearssl/i31_rshift.c")),
    ("i31_sub.c", include_str!("tls/bearssl/i31_sub.c")),
    ("i31_tmont.c", include_str!("tls/bearssl/i31_tmont.c")),
    ("i32_div32.c", include_str!("tls/bearssl/i32_div32.c")),
    ("i62_modpow2.c", include_str!("tls/bearssl/i62_modpow2.c")),
    ("inner.h", include_str!("tls/bearssl/inner.h")),
    ("multihash.c", include_str!("tls/bearssl/multihash.c")),
    ("pemdec.c", include_str!("tls/bearssl/pemdec.c")),
    (
        "poly1305_ctmul.c",
        include_str!("tls/bearssl/poly1305_ctmul.c"),
    ),
    (
        "poly1305_ctmulq.c",
        include_str!("tls/bearssl/poly1305_ctmulq.c"),
    ),
    ("prf.c", include_str!("tls/bearssl/prf.c")),
    ("prf_sha256.c", include_str!("tls/bearssl/prf_sha256.c")),
    ("prf_sha384.c", include_str!("tls/bearssl/prf_sha384.c")),
    (
        "rsa_default_modulus.c",
        include_str!("tls/bearssl/rsa_default_modulus.c"),
    ),
    (
        "rsa_default_pkcs1_sign.c",
        include_str!("tls/bearssl/rsa_default_pkcs1_sign.c"),
    ),
    (
        "rsa_default_pkcs1_vrfy.c",
        include_str!("tls/bearssl/rsa_default_pkcs1_vrfy.c"),
    ),
    (
        "rsa_default_priv.c",
        include_str!("tls/bearssl/rsa_default_priv.c"),
    ),
    (
        "rsa_i31_modulus.c",
        include_str!("tls/bearssl/rsa_i31_modulus.c"),
    ),
    (
        "rsa_i31_pkcs1_sign.c",
        include_str!("tls/bearssl/rsa_i31_pkcs1_sign.c"),
    ),
    (
        "rsa_i31_pkcs1_vrfy.c",
        include_str!("tls/bearssl/rsa_i31_pkcs1_vrfy.c"),
    ),
    ("rsa_i31_priv.c", include_str!("tls/bearssl/rsa_i31_priv.c")),
    ("rsa_i31_pub.c", include_str!("tls/bearssl/rsa_i31_pub.c")),
    (
        "rsa_i62_pkcs1_sign.c",
        include_str!("tls/bearssl/rsa_i62_pkcs1_sign.c"),
    ),
    (
        "rsa_i62_pkcs1_vrfy.c",
        include_str!("tls/bearssl/rsa_i62_pkcs1_vrfy.c"),
    ),
    ("rsa_i62_priv.c", include_str!("tls/bearssl/rsa_i62_priv.c")),
    ("rsa_i62_pub.c", include_str!("tls/bearssl/rsa_i62_pub.c")),
    (
        "rsa_pkcs1_sig_pad.c",
        include_str!("tls/bearssl/rsa_pkcs1_sig_pad.c"),
    ),
    (
        "rsa_pkcs1_sig_unpad.c",
        include_str!("tls/bearssl/rsa_pkcs1_sig_unpad.c"),
    ),
    (
        "rsa_ssl_decrypt.c",
        include_str!("tls/bearssl/rsa_ssl_decrypt.c"),
    ),
    ("sha1.c", include_str!("tls/bearssl/sha1.c")),
    ("sha2big.c", include_str!("tls/bearssl/sha2big.c")),
    ("sha2small.c", include_str!("tls/bearssl/sha2small.c")),
    ("skey_decoder.c", include_str!("tls/bearssl/skey_decoder.c")),
    ("ssl_client.c", include_str!("tls/bearssl/ssl_client.c")),
    ("ssl_engine.c", include_str!("tls/bearssl/ssl_engine.c")),
    (
        "ssl_engine_default_aescbc.c",
        include_str!("tls/bearssl/ssl_engine_default_aescbc.c"),
    ),
    (
        "ssl_engine_default_aesgcm.c",
        include_str!("tls/bearssl/ssl_engine_default_aesgcm.c"),
    ),
    (
        "ssl_engine_default_chapol.c",
        include_str!("tls/bearssl/ssl_engine_default_chapol.c"),
    ),
    (
        "ssl_engine_default_ec.c",
        include_str!("tls/bearssl/ssl_engine_default_ec.c"),
    ),
    (
        "ssl_engine_default_ecdsa.c",
        include_str!("tls/bearssl/ssl_engine_default_ecdsa.c"),
    ),
    (
        "ssl_engine_default_rsavrfy.c",
        include_str!("tls/bearssl/ssl_engine_default_rsavrfy.c"),
    ),
    ("ssl_hashes.c", include_str!("tls/bearssl/ssl_hashes.c")),
    (
        "ssl_hs_client.c",
        include_str!("tls/bearssl/ssl_hs_client.c"),
    ),
    (
        "ssl_hs_server.c",
        include_str!("tls/bearssl/ssl_hs_server.c"),
    ),
    ("ssl_io.c", include_str!("tls/bearssl/ssl_io.c")),
    ("ssl_rec_cbc.c", include_str!("tls/bearssl/ssl_rec_cbc.c")),
    (
        "ssl_rec_chapol.c",
        include_str!("tls/bearssl/ssl_rec_chapol.c"),
    ),
    ("ssl_rec_gcm.c", include_str!("tls/bearssl/ssl_rec_gcm.c")),
    (
        "ssl_scert_single_ec.c",
        include_str!("tls/bearssl/ssl_scert_single_ec.c"),
    ),
    (
        "ssl_scert_single_rsa.c",
        include_str!("tls/bearssl/ssl_scert_single_rsa.c"),
    ),
    ("ssl_server.c", include_str!("tls/bearssl/ssl_server.c")),
    ("sysrng.c", include_str!("tls/bearssl/sysrng.c")),
    ("x509_decoder.c", include_str!("tls/bearssl/x509_decoder.c")),
    ("x509_minimal.c", include_str!("tls/bearssl/x509_minimal.c")),
];

/// The archive, built on the first call of the process and reused by every later one; the
/// reason as text when it cannot be built.
pub(crate) fn archive() -> Result<std::path::PathBuf, String> {
    static BUILT: OnceLock<Result<std::path::PathBuf, String>> = OnceLock::new();
    BUILT.get_or_init(build).clone()
}

/// The lines a unit that fetches over a URL carries: `LOFT_HTTPS` and the archive's `link:`
/// line, or — where the archive cannot be built — `LOFT_TLS_UNAVAILABLE` with the reason, which
/// an `https://` request then answers with.
pub(crate) fn unit_lines() -> String {
    match archive() {
        Ok(path) => format!("#define LOFT_HTTPS 1\n/* link: {} */\n", path.display()),
        Err(why) => {
            let first = why.lines().next().unwrap_or_default();
            format!(
                "#define LOFT_TLS_UNAVAILABLE {}\n",
                super::body::c_string(&format!("the TLS client did not build: {first}"))
            )
        }
    }
}

/// Where the sources are written: one directory per source set, so a loft carrying other
/// sources never reads an archive built from these.
fn source_dir() -> std::path::PathBuf {
    let parts: Vec<&[u8]> = SOURCES
        .iter()
        .flat_map(|(name, text)| [name.as_bytes(), text.as_bytes()])
        .collect();
    let key = crate::cache::stable_hash(&parts);
    std::env::var_os("LOFT_HOME")
        .map(std::path::PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join(".loft")
        .join("c-tls")
        .join(format!("{key:016x}"))
}

fn build() -> Result<std::path::PathBuf, String> {
    let dir = source_dir();
    crate::file_access::create_dir_all(&dir).map_err(|e| format!("cannot create {e}"))?;
    // Each file is written beside its final name and renamed into place, so a concurrent build
    // reads a whole file or none; a file already there is the same text (the directory is
    // keyed by the sources).
    for (name, text) in SOURCES {
        let path = dir.join(name);
        if crate::file_access::exists(&path) {
            continue;
        }
        let stage = dir.join(format!(".{name}.{}", std::process::id()));
        crate::file_access::write(&stage, text)
            .map_err(|e| format!("cannot write {}: {e}", stage.display()))?;
        crate::file_access::rename(&stage, &path)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    let compiled: Vec<String> = SOURCES
        .iter()
        .filter(|(name, _)| crate::file_access::has_extension(*name, "c"))
        .map(|(name, _)| (*name).to_string())
        .collect();
    crate::c_shim::cc_archive(&dir.to_string_lossy(), &compiled)
}

#[cfg(test)]
mod tests {
    use super::SOURCES;

    /// Every vendored file is carried and nothing else: a file added to `tls/bearssl/` without
    /// its entry would be missing from the archive, and only a link would say so.
    #[test]
    fn the_tls_sources_are_the_directory() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/c/tls/bearssl");
        let mut on_disk: Vec<String> = crate::file_access::read_dir(dir)
            .expect("the vendored BearSSL directory")
            .iter()
            .filter_map(crate::file_access::file_name)
            .filter(|n| {
                crate::file_access::has_extension(n.as_str(), "c")
                    || crate::file_access::has_extension(n.as_str(), "h")
            })
            .collect();
        on_disk.sort();
        let mut listed: Vec<String> = SOURCES
            .iter()
            .map(|(n, _)| (*n).to_string())
            .filter(|n| n != "loft_tls.c" && n != "loft_ssh.c")
            .collect();
        listed.sort();
        assert_eq!(listed, on_disk);
    }
}
