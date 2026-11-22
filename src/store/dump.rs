// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@PLN206 P4c` — `LOFT_STORE_DUMP=<file>`: every store's final content — written when the store
//! is freed, and for a store still live at the end of the run, at the end — for
//! `scripts/store_diff.py` to set beside the C runtime's (`loft_store_dump`, the same format).  One line per store — `S <name> <known type> <size in words>` — then one per claimed
//! block in position order — `R <record> <words> <bytes in hex>` — the block whole, its header
//! word included.  A store is named as `LOFT_ALLOC_TRACE` names it, so the script pairs the two
//! runtimes' stores by what happened to them rather than by slot.  Run under
//! `LOFT_POISON_CLAIM`, a byte nothing wrote reads the same on both.

use super::Store;
use std::fmt::Write as _;

impl Store {
    /// The store's name in a dump: its `LOFT_ALLOC_TRACE` name where the build writes one.
    fn dump_name(&self) -> String {
        #[cfg(feature = "op-census")]
        {
            self.trace_name()
        }
        #[cfg(not(feature = "op-census"))]
        {
            self.alloc_serial.to_string()
        }
    }

    /// Every claimed block of this store with its bytes, one `R` line each — what
    /// `LOFT_HOSTED_VERIFY` compares (`state/hosted_verify.rs`).
    pub(crate) fn claimed_image(&self) -> String {
        let mut out = String::new();
        self.dump_into(&mut out);
        out.split_once('\n')
            .map_or(String::new(), |(_, rest)| rest.to_string())
    }

    /// This store's lines: its header, then each claimed block's bytes.
    fn dump_into(&self, out: &mut String) {
        let _ = writeln!(
            out,
            "S {} {} {} {}",
            self.dump_name(),
            self.known_type,
            self.size,
            self.store_nr
        );
        for rec in self.claims.positions() {
            let words = self.read::<i32>(rec, 0).max(1) as u32;
            let end = (rec + words).min(self.size);
            let _ = write!(out, "R {rec} {words} ");
            for w in rec..end {
                for b in 0..8 {
                    // SAFETY: `w < size`, so the byte lies inside this store's buffer.
                    let byte = unsafe { *self.ptr.add(w as usize * 8 + b) };
                    let _ = write!(out, "{byte:02x}");
                }
            }
            out.push('\n');
        }
    }
}

/// The dump file, opened at the first store written; `None` when `LOFT_STORE_DUMP` is not set.
fn dump_file() -> Option<&'static std::sync::Mutex<crate::file_access::File>> {
    static FILE: std::sync::OnceLock<Option<std::sync::Mutex<crate::file_access::File>>> =
        std::sync::OnceLock::new();
    FILE.get_or_init(|| {
        let path = std::env::var_os("LOFT_STORE_DUMP")?;
        crate::file_access::create(std::path::Path::new(&path))
            .ok()
            .map(std::sync::Mutex::new)
    })
    .as_ref()
}

fn write_out(text: &str) {
    use std::io::Write as _;
    if let Some(f) = dump_file()
        && let Ok(mut f) = f.lock()
    {
        let _ = f.write_all(text.as_bytes());
    }
}

/// A store as it is freed: its final content (`Stores::free_named`, before the free poisons it).
pub(crate) fn on_free(store: &Store) {
    if dump_file().is_none() || store.stack_buffer {
        return;
    }
    let mut out = String::new();
    store.dump_into(&mut out);
    write_out(&out);
}

/// Every store still live at the end of the run.
pub(crate) fn write_if_asked(stores: &[Store]) {
    if dump_file().is_none() {
        return;
    }
    let mut out = String::new();
    for s in stores.iter().filter(|s| s.live && !s.stack_buffer) {
        s.dump_into(&mut out);
    }
    write_out(&out);
}
