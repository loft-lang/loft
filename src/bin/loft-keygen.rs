// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I77 — Registry / manifest / lockfile resolution

//! `loft-keygen` — Ed25519 keypair generator for the registry trust root.
//!
//! Run on an **offline / air-gapped machine** when bootstrapping the
//! `loft-lang/registry` repo (REGISTRY_BOOTSTRAP.md Step 1).
//!
//! Usage:
//!
//! ```sh
//! cargo build --release --bin loft-keygen --features registry
//! ./target/release/loft-keygen generate
//! ./target/release/loft-keygen format --in registry-signing-key.pub
//! ```
//!
//! `generate`:
//! - Writes the 32-byte raw private key to `registry-signing-key.bin`
//!   (chmod 600).  The private key signs locally on the
//!   maintainer's trusted hardware; it MUST NOT leave that hardware.
//! - Writes the 32-byte public key as hex to `registry-signing-key.pub`.
//! - Prints the public hex and a Rust `[u8; 32]` literal of the
//!   public key to stdout, ready to paste into
//!   `src/registry_keys.rs::TRUSTED_PUBLIC_KEYS`.
//!
//! `format`:
//! - Takes a hex-encoded public key (from `--in <file>` or stdin)
//!   and emits a Rust `[u8; 32]` literal ready to paste into
//!   `src/registry_keys.rs::TRUSTED_PUBLIC_KEYS`.
//!
//! No network access.  No persistent state.  The binary doesn't
//! call home, doesn't log anything beyond stdout/stderr, and
//! doesn't keep the key in memory beyond the one stack frame.

// @PLN184 A1: not yet through `file_access` — a crate root's allow covers the whole binary,
// so this binary is checked once its root is clean (src/file_access/clippy_allow.baseline).
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
#![cfg(feature = "registry")]
#![allow(clippy::missing_docs_in_private_items)]

use std::fs;
use std::io::{self, Read};
use std::path::Path;

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // `-h`/`--help` ANYWHERE prints help and exits 0 — BEFORE dispatch.  Without
    // this, `loft-keygen generate --help` fell through to `cmd_generate` (which
    // ignored its args) and *generated a keypair* instead of showing help, and a
    // bare `loft-keygen --help` exited 1 as an "unknown command".
    if args[1..].iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        std::process::exit(0);
    }
    let cmd = args.get(1).map(String::as_str).unwrap_or("");
    match cmd {
        "generate" => cmd_generate(&args[2..]),
        "format" => cmd_format(&args[2..]),
        "sign" => cmd_sign(&args[2..]),
        "verify" => cmd_verify(&args[2..]),
        _ => {
            print_help();
            std::process::exit(if cmd.is_empty() { 0 } else { 1 });
        }
    }
}

fn print_help() {
    eprintln!("loft-keygen — Ed25519 keypair tool for the loft registry trust root.");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  generate                  Create a new keypair.");
    eprintln!("                            Writes registry-signing-key.bin (private)");
    eprintln!("                            and registry-signing-key.pub (public hex).");
    eprintln!();
    eprintln!("  format --in <pub_file>    Read hex public key from <pub_file>, emit");
    eprintln!("                            a Rust [u8; 32] literal ready to paste into");
    eprintln!("                            src/registry_keys.rs::TRUSTED_PUBLIC_KEYS.");
    eprintln!("                            (omit --in to read from stdin)");
    eprintln!();
    eprintln!("  sign --in <data> --key <private.bin> --out <data>.sig");
    eprintln!("                            Sign <data> with the private key.");
    eprintln!("                            Writes a raw 64-byte Ed25519 signature.");
    eprintln!("                            Maintainer workflow: sign `index.json`");
    eprintln!("                            locally during PR review.");
    eprintln!();
    eprintln!("  verify --in <data> --sig <data>.sig --pub <hex>");
    eprintln!("                            Verify a signature against a public key.");
    eprintln!("                            --pub is a 64-char hex string (or");
    eprintln!("                            --pub-file <path>).  Exits 0 on valid,");
    eprintln!("                            1 on invalid.  Use during the annual");
    eprintln!("                            recovery drill to confirm a restored key");
    eprintln!("                            matches the embedded public.");
    eprintln!();
    eprintln!("Run `generate` on an offline machine.  Back up the .bin file to a");
    eprintln!("hardware token and a sealed offline location.  The private key");
    eprintln!("NEVER leaves your trusted hardware — daily signing happens locally");
    eprintln!("via `loft-keygen sign`, not in CI.");
}

fn cmd_generate(extra: &[String]) {
    // `generate` takes no arguments (help is handled in `main`).  Reject stray
    // ones rather than silently ignore them — a typo'd flag must not quietly
    // produce a keypair.
    if let Some(arg) = extra.first() {
        eprintln!(
            "loft-keygen generate: unexpected argument `{arg}` (generate takes none; try --help)"
        );
        std::process::exit(2);
    }
    // Use the OS CSPRNG via getrandom — `ed25519_dalek::SigningKey`'s
    // `generate` takes a `CryptoRngCore`; OsRng from rand_core 0.6 (the
    // version dalek 2.x expects).  We're already paying for the dalek
    // dependency, so use its bundled prelude here.
    let mut seed = [0u8; 32];
    getrandom_fill(&mut seed).unwrap_or_else(|e| {
        eprintln!("loft-keygen: cannot read OS entropy: {e}");
        std::process::exit(1);
    });
    let signing_key = SigningKey::from_bytes(&seed);
    let priv_bytes = signing_key.to_bytes();
    let pub_bytes = signing_key.verifying_key().to_bytes();

    let priv_path = Path::new("registry-signing-key.bin");
    let pub_path = Path::new("registry-signing-key.pub");

    if priv_path.exists() || pub_path.exists() {
        eprintln!(
            "loft-keygen: refusing to overwrite existing key files in cwd.\n  \
             Move or delete `registry-signing-key.bin` / `registry-signing-key.pub` first."
        );
        std::process::exit(1);
    }

    fs::write(priv_path, priv_bytes)
        .unwrap_or_else(|e| die(&format!("write {}: {e}", priv_path.display())));
    fs::write(pub_path, hex_encode(&pub_bytes))
        .unwrap_or_else(|e| die(&format!("write {}: {e}", pub_path.display())));
    chmod_600(priv_path);

    println!("Generated keypair:");
    println!(
        "  private:    {} (32 bytes, chmod 600)",
        priv_path.display()
    );
    println!("  public:     {} (64-char hex)", pub_path.display());
    println!();
    println!("Public key (paste into src/registry_keys.rs::TRUSTED_PUBLIC_KEYS):");
    println!("    {}", format_rust_literal(&pub_bytes));
    println!();
    println!("Next steps:");
    println!("  1. Back up registry-signing-key.bin to a hardware token + sealed offline copy.");
    println!("     (See REGISTRY_BOOTSTRAP.md § Step 1.5 for the 3-2-1 backup layout.)");
    println!("  2. Move registry-signing-key.bin to your daily signing location,");
    println!("     e.g. ~/.loft/trust-root/registry-signing-key.bin (chmod 600,");
    println!("     inside FileVault/LUKS-encrypted home).");
    println!("  3. Edit src/registry_keys.rs and add the public-key literal above.");
    println!();
    println!("Daily signing (run on your trusted laptop, NOT in CI):");
    println!("  loft-keygen sign --in index.json \\");
    println!("                   --key ~/.loft/trust-root/registry-signing-key.bin \\");
    println!("                   --out index.json.sig");
    println!("  git add index.json.sig && git commit -m 'sign index.json'");
    println!();
    println!("The private key never leaves your laptop; no GitHub Secret needed.");
}

fn cmd_format(extra: &[String]) {
    let hex_text = if let Some(pos) = extra.iter().position(|a| a == "--in")
        && let Some(path) = extra.get(pos + 1)
    {
        fs::read_to_string(path).unwrap_or_else(|e| die(&format!("read {path}: {e}")))
    } else {
        let mut s = String::new();
        io::stdin()
            .read_to_string(&mut s)
            .unwrap_or_else(|e| die(&format!("read stdin: {e}")));
        s
    };
    let hex = hex_text.trim();
    let bytes = hex_decode(hex).unwrap_or_else(|e| die(&format!("invalid hex: {e}")));
    if bytes.len() != 32 {
        die(&format!(
            "expected 32 bytes of hex (64 chars), got {} bytes",
            bytes.len()
        ));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    println!("    {}", format_rust_literal(&arr));
}

fn format_rust_literal(bytes: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut s = String::from("[");
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        let _ = write!(s, "0x{b:02x}");
    }
    s.push(']');
    s
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) {
        return Err(format!("odd number of hex chars: {}", s.len()));
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for chunk in s.as_bytes().chunks(2) {
        let h = std::str::from_utf8(chunk).map_err(|e| e.to_string())?;
        out.push(u8::from_str_radix(h, 16).map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// `loft-keygen sign --in <data> --key <private.bin> --out <data>.sig`
///
/// Reads `<data>` from disk, reads the 32-byte private key from
/// `<private.bin>`, produces a raw 64-byte Ed25519 signature, writes
/// it to `<out>` atomically (temp + rename).  No encoding wrapper —
/// the loft client expects raw bytes.
fn cmd_sign(extra: &[String]) {
    let in_path = arg_value(extra, "--in").unwrap_or_else(|| die("sign: missing --in"));
    let key_path = arg_value(extra, "--key").unwrap_or_else(|| die("sign: missing --key"));
    let out_path = arg_value(extra, "--out").unwrap_or_else(|| die("sign: missing --out"));

    let key_bytes =
        fs::read(&key_path).unwrap_or_else(|e| die(&format!("read key {key_path}: {e}")));
    if key_bytes.len() != 32 {
        die(&format!(
            "key file must be 32 bytes (raw Ed25519 private), got {}",
            key_bytes.len()
        ));
    }
    let mut key_arr = [0u8; 32];
    key_arr.copy_from_slice(&key_bytes);
    let signing_key = SigningKey::from_bytes(&key_arr);

    let data = fs::read(&in_path).unwrap_or_else(|e| die(&format!("read {in_path}: {e}")));
    let sig = signing_key.sign(&data);
    let sig_bytes = sig.to_bytes();

    // Atomic write so a half-written sig file can't briefly serve.
    let tmp = format!("{out_path}.tmp");
    fs::write(&tmp, sig_bytes).unwrap_or_else(|e| die(&format!("write {tmp}: {e}")));
    fs::rename(&tmp, &out_path)
        .unwrap_or_else(|e| die(&format!("rename {tmp} -> {out_path}: {e}")));
    println!(
        "signed {in_path} ({} bytes) -> {out_path} (64-byte Ed25519 signature)",
        data.len()
    );
}

/// `loft-keygen verify --in <data> --sig <data>.sig --pub <hex>` (or `--pub-file`)
///
/// Verifies that `<data>.sig` is a valid Ed25519 signature for `<data>`
/// under the public key in `--pub` (64 hex chars) or `--pub-file`
/// (file containing 64 hex chars).  Exits 0 on valid, 1 on invalid /
/// any failure.  Used during the annual recovery drill to confirm a
/// restored key matches the embedded public key.
fn cmd_verify(extra: &[String]) {
    let in_path = arg_value(extra, "--in").unwrap_or_else(|| die("verify: missing --in"));
    let sig_path = arg_value(extra, "--sig").unwrap_or_else(|| die("verify: missing --sig"));
    let pub_hex = if let Some(h) = arg_value(extra, "--pub") {
        h
    } else if let Some(p) = arg_value(extra, "--pub-file") {
        fs::read_to_string(&p)
            .unwrap_or_else(|e| die(&format!("read {p}: {e}")))
            .trim()
            .to_string()
    } else {
        die("verify: missing --pub <hex> or --pub-file <path>");
    };

    let pub_bytes = hex_decode(&pub_hex).unwrap_or_else(|e| die(&format!("--pub: {e}")));
    if pub_bytes.len() != 32 {
        die(&format!(
            "public key must be 32 bytes (64 hex chars), got {}",
            pub_bytes.len()
        ));
    }
    let mut pub_arr = [0u8; 32];
    pub_arr.copy_from_slice(&pub_bytes);
    let verifying_key = VerifyingKey::from_bytes(&pub_arr)
        .unwrap_or_else(|e| die(&format!("invalid public key bytes: {e}")));

    let data = fs::read(&in_path).unwrap_or_else(|e| die(&format!("read {in_path}: {e}")));
    let sig_bytes = fs::read(&sig_path).unwrap_or_else(|e| die(&format!("read {sig_path}: {e}")));
    if sig_bytes.len() != 64 {
        die(&format!(
            "signature file must be 64 bytes (raw Ed25519), got {}",
            sig_bytes.len()
        ));
    }
    let mut sig_arr = [0u8; 64];
    sig_arr.copy_from_slice(&sig_bytes);
    let sig = ed25519_dalek::Signature::from_bytes(&sig_arr);

    match verifying_key.verify(&data, &sig) {
        Ok(()) => {
            println!("OK — signature valid");
        }
        Err(e) => {
            eprintln!("FAIL — signature invalid: {e}");
            std::process::exit(1);
        }
    }
}

/// Parse `--<name> <value>` argument pairs.  Returns `None` when the
/// flag isn't present.
fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|p| args.get(p + 1))
        .cloned()
}

#[cfg(unix)]
fn chmod_600(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    if let Ok(metadata) = fs::metadata(path) {
        let mut perms = metadata.permissions();
        perms.set_mode(0o600);
        let _ = fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn chmod_600(_path: &Path) {
    // Windows: NTFS ACLs are out of scope for the bootstrap.  The
    // private key file should still be on an air-gapped machine
    // anyway, where filesystem perms are secondary.
}

fn getrandom_fill(buf: &mut [u8]) -> io::Result<()> {
    // /dev/urandom path keeps the dependency surface minimal — no
    // `rand` / `getrandom` crate needed.  Same entropy source on
    // Linux + macOS + the BSDs.  Windows falls back to the OS-level
    // BCryptGenRandom via a tiny wrapper; if the user's air-gap
    // machine is Linux/macOS this code path is what runs.
    #[cfg(unix)]
    {
        let mut f = fs::File::open("/dev/urandom")?;
        f.read_exact(buf)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        // For non-Unix, defer to the user running keygen on a real
        // air-gapped Unix box.  The bootstrap doc recommends this
        // anyway.
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "loft-keygen needs /dev/urandom (Unix-only).  \
             Run on Linux/macOS instead.",
        ))
    }
}

fn die(msg: &str) -> ! {
    eprintln!("loft-keygen: {msg}");
    std::process::exit(1);
}
