// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN38 phase 00 — durable-store on-disk format detection.
//!
//! Exercises `Store::detect_format` and `Store::validate_integrity` against
//! synthetic fixtures.  These tests do NOT open the store as a `Store`;
//! they only check the format-introspection API.  See
//! `tests/store_durable_tier1.rs` for the open_durable round-trip.

#![cfg(feature = "mmap")]

use loft::file_access as fa;
use loft::store::{CorruptReason, Store, StoreFormat, StoreIntegrity};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Per-test scratch directory under the system temp.  Each test name gets
/// its own subdir so parallel `cargo test` runs don't collide.
fn scratch(test_name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("loft-store-durable-tests")
        .join(test_name);
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Layout of a hand-built sidecar fixture.  Mirrors the format in
/// `src/store.rs` so the tests don't depend on internal helpers.
fn build_sidecar(
    signature: &[u8; 8],
    tier_id: u16,
    flags: u16,
    last_clean_ns: u64,
    payload_len: u64,
    payload_crc: u32,
    override_header_crc: Option<u32>,
) -> [u8; 40] {
    let mut buf = [0u8; 40];
    buf[..8].copy_from_slice(signature);
    buf[8..10].copy_from_slice(&tier_id.to_le_bytes());
    buf[10..12].copy_from_slice(&flags.to_le_bytes());
    let mut hdr = crc32fast::Hasher::new();
    hdr.update(&buf[..12]);
    let header_crc = override_header_crc.unwrap_or_else(|| hdr.finalize());
    buf[12..16].copy_from_slice(&header_crc.to_le_bytes());
    buf[16..24].copy_from_slice(&last_clean_ns.to_le_bytes());
    buf[24..32].copy_from_slice(&payload_len.to_le_bytes());
    buf[32..36].copy_from_slice(&payload_crc.to_le_bytes());
    // bytes[36..40] reserved (zero)
    buf
}

/// Write a synthetic main file with deterministic contents.
fn write_main_file(path: &Path, content: &[u8]) {
    let mut f = fa::create(path).expect("create main file");
    f.write_all(content).expect("write main file");
    f.sync_all().expect("fsync main file");
}

fn payload_crc(content: &[u8]) -> u32 {
    let mut h = crc32fast::Hasher::new();
    h.update(content);
    h.finalize()
}

// ── detect_format ────────────────────────────────────────────────────────────

#[test]
fn detect_format_no_sidecar_is_legacy() {
    let dir = scratch("detect_format_no_sidecar_is_legacy");
    let main = dir.join("data.store");
    write_main_file(&main, b"any bytes; sidecar absent");
    assert_eq!(Store::detect_format(&main).unwrap(), StoreFormat::Legacy);
}

#[test]
fn detect_format_valid_sidecar_is_durable_tier_1() {
    let dir = scratch("detect_format_valid_sidecar_is_durable_tier_1");
    let main = dir.join("data.store");
    let content = b"payload of length 32 bytes -- ok";
    write_main_file(&main, content);
    let sidecar = build_sidecar(
        b"DStoreV1",
        1,
        0,
        12345,
        content.len() as u64,
        payload_crc(content),
        None,
    );
    fa::write(dir.join("data.store.dmeta"), sidecar).unwrap();
    assert_eq!(
        Store::detect_format(&main).unwrap(),
        StoreFormat::Durable(1)
    );
}

// ── validate_integrity ───────────────────────────────────────────────────────

#[test]
fn validate_clean_when_everything_matches() {
    let dir = scratch("validate_clean_when_everything_matches");
    let main = dir.join("data.store");
    let content = b"x".repeat(128);
    write_main_file(&main, &content);
    let sidecar = build_sidecar(
        b"DStoreV1",
        1,
        0,
        99,
        content.len() as u64,
        payload_crc(&content),
        None,
    );
    fa::write(dir.join("data.store.dmeta"), sidecar).unwrap();
    assert_eq!(
        Store::validate_integrity(&main).unwrap(),
        StoreIntegrity::Clean
    );
}

#[test]
fn validate_tail_marker_missing_when_no_sidecar() {
    let dir = scratch("validate_tail_marker_missing_when_no_sidecar");
    let main = dir.join("data.store");
    write_main_file(&main, b"some bytes");
    assert_eq!(
        Store::validate_integrity(&main).unwrap(),
        StoreIntegrity::Corrupt(CorruptReason::TailMarkerMissing)
    );
}

#[test]
fn validate_signature_mismatch_when_wrong_signature_bytes() {
    let dir = scratch("validate_signature_mismatch_when_wrong_signature_bytes");
    let main = dir.join("data.store");
    let content = b"valid main file bytes";
    write_main_file(&main, content);
    // Build a sidecar with a wrong signature; header_crc has to be over the
    // bytes-as-written so this exercises SignatureMismatch (which is checked
    // before HeaderCrcMismatch).
    let sidecar = build_sidecar(
        b"WrongSig",
        1,
        0,
        0,
        content.len() as u64,
        payload_crc(content),
        None,
    );
    fa::write(dir.join("data.store.dmeta"), sidecar).unwrap();
    assert_eq!(
        Store::validate_integrity(&main).unwrap(),
        StoreIntegrity::Corrupt(CorruptReason::SignatureMismatch)
    );
}

#[test]
fn validate_header_crc_mismatch_when_header_crc_field_corrupted() {
    let dir = scratch("validate_header_crc_mismatch_when_header_crc_field_corrupted");
    let main = dir.join("data.store");
    let content = b"main file bytes";
    write_main_file(&main, content);
    // Override the header_crc field with a wrong value.
    let sidecar = build_sidecar(
        b"DStoreV1",
        1,
        0,
        0,
        content.len() as u64,
        payload_crc(content),
        Some(0xDEAD_BEEF),
    );
    fa::write(dir.join("data.store.dmeta"), sidecar).unwrap();
    assert_eq!(
        Store::validate_integrity(&main).unwrap(),
        StoreIntegrity::Corrupt(CorruptReason::HeaderCrcMismatch)
    );
}

#[test]
fn validate_truncated_file_when_main_shorter_than_sidecar_claims() {
    let dir = scratch("validate_truncated_file_when_main_shorter_than_sidecar_claims");
    let main = dir.join("data.store");
    let content = b"only 32 bytes of actual content!";
    write_main_file(&main, content);
    // Sidecar claims the main file is 9999 bytes long.  CRC field is
    // arbitrary — the length check fires first.
    let sidecar = build_sidecar(b"DStoreV1", 1, 0, 0, 9999, 0, None);
    fa::write(dir.join("data.store.dmeta"), sidecar).unwrap();
    assert_eq!(
        Store::validate_integrity(&main).unwrap(),
        StoreIntegrity::Corrupt(CorruptReason::TruncatedFile)
    );
}

#[test]
fn validate_tail_crc_mismatch_when_payload_byte_flipped() {
    let dir = scratch("validate_tail_crc_mismatch_when_payload_byte_flipped");
    let main = dir.join("data.store");
    let mut content = b"original content of length 32!!".to_vec();
    write_main_file(&main, &content);
    // Sidecar reflects the ORIGINAL CRC.
    let original_crc = payload_crc(&content);
    let sidecar = build_sidecar(
        b"DStoreV1",
        1,
        0,
        0,
        content.len() as u64,
        original_crc,
        None,
    );
    fa::write(dir.join("data.store.dmeta"), sidecar).unwrap();
    // Now flip one byte in the main file (simulates bitrot / torn write).
    content[10] ^= 0xFF;
    write_main_file(&main, &content);
    assert_eq!(
        Store::validate_integrity(&main).unwrap(),
        StoreIntegrity::Corrupt(CorruptReason::TailCrcMismatch)
    );
}

#[test]
fn validate_truncated_when_main_missing_but_sidecar_present() {
    let dir = scratch("validate_truncated_when_main_missing_but_sidecar_present");
    let main = dir.join("data.store");
    // Don't write main file at all.
    let sidecar = build_sidecar(b"DStoreV1", 1, 0, 0, 128, 0, None);
    fa::write(dir.join("data.store.dmeta"), sidecar).unwrap();
    assert_eq!(
        Store::validate_integrity(&main).unwrap(),
        StoreIntegrity::Corrupt(CorruptReason::TruncatedFile)
    );
}

#[test]
fn validate_signature_mismatch_when_tier_id_zero() {
    // A sidecar whose tier_id is 0 ("none") is structurally suspect — it
    // advertises that no real durability tier owns this store.  The
    // validate path surfaces this as SignatureMismatch (the closest
    // semantic match — the sidecar isn't a real durable-store identifier).
    let dir = scratch("validate_signature_mismatch_when_tier_id_zero");
    let main = dir.join("data.store");
    let content = b"some bytes";
    write_main_file(&main, content);
    let sidecar = build_sidecar(
        b"DStoreV1",
        0, // tier_id = none
        0,
        0,
        content.len() as u64,
        payload_crc(content),
        None,
    );
    fa::write(dir.join("data.store.dmeta"), sidecar).unwrap();
    assert_eq!(
        Store::validate_integrity(&main).unwrap(),
        StoreIntegrity::Corrupt(CorruptReason::SignatureMismatch)
    );
}
