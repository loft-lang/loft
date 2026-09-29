// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @C91 — `==` compares content for every type (`@FR-E-Eq`).  The Rust-level guards of the
//! build (@PLN175 § C91): the identity census the flips are measured by.

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// The census trace (`LOFT_TRACE_EQ_IDENTITY=1`) of a `--check` over `src`, one line per site.
fn identity_sites(name: &str, src: &str) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!("loft-c91-census-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join(name);
    std::fs::write(&path, src).expect("write probe");
    let out = Command::new(loft_bin())
        .arg("--check")
        .arg(&path)
        .env("LOFT_TRACE_EQ_IDENTITY", "1")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("run loft --check");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "the probe must compile: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter_map(|l| l.strip_prefix("[eq-identity] "))
        .map(|l| l.rsplit_once('/').map_or(l, |(_, tail)| tail).to_string())
        .collect()
}

/// The census names each `==` / `!=` that compares identity — by line, kind and operand
/// types — and is silent where the answer does not depend on identity: a `null` test, a
/// scalar, a `value struct` compared by content.  A trace that went silent would report a
/// flip as changing nothing; one that also fired on `x == null` would send that site to be
/// converted to `&a == &b`, which is not what it asks.
#[test]
fn the_census_names_identity_compares_and_nothing_else() {
    let sites = identity_sites(
        "census.loft",
        "struct P { x: integer }
value struct V { x: integer }
enum S { Circle { r: integer }, Square { s: integer } }
struct E { k: integer, v: integer }
fn main() {
  a = P { x: 1 };
  b = P { x: 1 };
  n: P? = null;
  m = P { x: 2 };
  v = V { x: 1 };
  w = V { x: 1 };
  c = Circle { r: 1 };
  d = Circle { r: 1 };
  h: hash<E[k]> = [];
  g: hash<E[k]> = [];
  assert(!(a == b), \"struct\");
  assert(a != b, \"struct ne\");
  assert(n == null, \"null test\");
  assert(!(m == null), \"null test on a non-optional struct\");
  assert(v == w, \"value struct\");
  assert(1 == 1, \"scalar\");
  assert(!(c == d), \"struct-enum\");
  assert(!(h == g), \"collection\");
}
",
    );
    assert_eq!(
        sites,
        vec![
            "census.loft:16  struct  P == P",
            "census.loft:17  struct  P != P",
            "census.loft:22  struct-enum  Circle == Circle",
            "census.loft:23  collection  hash<E> == hash<E>",
        ],
        "the identity census"
    );
}

