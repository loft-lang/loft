// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The one TLS client configuration every HTTPS request loft makes uses.
//!
//! It trusts what the machine trusts: the bundled Mozilla roots (`webpki-roots`) PLUS the
//! platform's store, read by `rustls-native-certs` — the system bundle, or `SSL_CERT_FILE` /
//! `SSL_CERT_DIR` when set, as curl and git read them.  A TLS-inspecting proxy whose CA is
//! installed there is therefore trusted here too.  Certificates are always validated; there
//! is no switch that turns that off.
//!
//! ureq's own default trusts the bundled roots only, and its `native-certs` feature REPLACES
//! them with the platform's rather than adding to them, so the store is built here and every
//! agent is made by [`agent_builder`].

use std::sync::{Arc, OnceLock};

/// The shared client config, built once per process: the platform store is read from disk.
fn client_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            };
            let native = rustls_native_certs::load_native_certs();
            let (_, unusable) = roots.add_parsable_certificates(native.certs);
            // A source that could not be read, or a certificate in it that rustls cannot use,
            // is skipped: the bundled roots still stand, and one bad entry in a system bundle
            // must not stop every download.  Said once, because a proxy CA that silently fails
            // to load is exactly what someone debugging an UnknownIssuer needs to see.
            for e in &native.errors {
                eprintln!("loft: a certificate source could not be read and is skipped: {e}");
            }
            if unusable > 0 {
                eprintln!(
                    "loft: {unusable} certificate(s) in the machine's trust store could not be \
                     used and are skipped"
                );
            }
            Arc::new(
                rustls::ClientConfig::builder_with_provider(
                    rustls::crypto::ring::default_provider().into(),
                )
                .with_protocol_versions(&[&rustls::version::TLS12, &rustls::version::TLS13])
                .expect("the ring provider supports TLS 1.2 and 1.3")
                .with_root_certificates(roots)
                .with_no_client_auth(),
            )
        })
        .clone()
}

/// An agent builder carrying the shared TLS config.  Every HTTPS client in loft starts here
/// and adds its own timeouts.
#[must_use]
pub(crate) fn agent_builder() -> ureq::AgentBuilder {
    ureq::AgentBuilder::new().tls_config(client_config())
}
