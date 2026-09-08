// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)
//! File I/O: reading and writing database content to/from files.

use crate::database::{Parts, Stores};
use crate::keys::DbRef;
use crate::store::Store;
use crate::vector;
#[cfg(not(host_fs))]
use std::collections::BTreeMap;
#[cfg(not(host_fs))]
use std::io::Write as _;

enum Format {
    TextFile = 1,
    _LittleEndian = 2,
    _BigEndian = 3,
    Directory = 4,
    NotExists = 5,
}

/// The state of a `File` handle for a path that is not there: no position, no
/// size, `NotExists`.
///
/// Size stays 0 rather than the null every other "absent" answer uses, because
/// `File.size` is declared `not null` — `format == NotExists` is where a handle
/// says there is no file.  It matters that this is written down once: the
/// containment refusal used to skip filling the struct at all, and a handle
/// then carried whatever its fields happened to start as (loft#708).
fn fill_absent(store: &mut Store, file: &DbRef) {
    store.set_long(file.rec, file.pos, 0); // size
    store.set_long(file.rec, file.pos + 8, i64::MIN); // current
    store.set_long(file.rec, file.pos + 16, i64::MIN); // next
    store.set_byte(file.rec, file.pos + 32, 0, Format::NotExists as i32);
}

#[cfg(not(host_fs))]
fn fill_file(path: &std::path::Path, store: &mut Store, file: &DbRef) -> bool {
    store.set_long(file.rec, file.pos + 8, i64::MIN); // current
    store.set_long(file.rec, file.pos + 16, i64::MIN); // next
    if let Ok(data) = path.metadata() {
        store.set_long(file.rec, file.pos, i64::MIN); // no size
        let tp = if data.is_dir() {
            Format::Directory
        } else if data.is_file() {
            store.set_long(file.rec, file.pos, data.len() as i64); // write size
            Format::TextFile
        } else {
            Format::NotExists
        };
        store.set_byte(file.rec, file.pos + 32, 0, tp as i32);
        true
    } else {
        fill_absent(store, file);
        false
    }
}

impl Stores {
    /// # Panics
    /// If `tp` refers to a type that is not implemented for file reading.
    #[allow(clippy::too_many_lines)]
    pub fn read_data(&self, r: &DbRef, tp: u16, little_endian: bool, data: &mut Vec<u8>) {
        let store = &self.allocations[r.store_nr as usize];
        match tp {
            0 => {
                // integer (i64 post-2c)
                let v = store.get_int(r.rec, r.pos);
                (if little_endian {
                    v.to_le_bytes()
                } else {
                    v.to_be_bytes()
                })
                .iter()
                .for_each(|&x| data.push(x));
            }
            6 => {
                // character (u32)
                let v = store.get_u32_raw(r.rec, r.pos);
                (if little_endian {
                    v.to_le_bytes()
                } else {
                    v.to_be_bytes()
                })
                .iter()
                .for_each(|&x| data.push(x));
            }
            1 => {
                // long
                let v = store.get_long(r.rec, r.pos);
                (if little_endian {
                    v.to_le_bytes()
                } else {
                    v.to_be_bytes()
                })
                .iter()
                .for_each(|&x| data.push(x));
            }
            2 => {
                // single
                let v = store.get_single(r.rec, r.pos);
                (if little_endian {
                    v.to_le_bytes()
                } else {
                    v.to_be_bytes()
                })
                .iter()
                .for_each(|&x| data.push(x));
            }
            3 => {
                // float
                let v = store.get_float(r.rec, r.pos);
                (if little_endian {
                    v.to_le_bytes()
                } else {
                    v.to_be_bytes()
                })
                .iter()
                .for_each(|&x| data.push(x));
            }
            4 => {
                // boolean
                let v = store.get_byte(r.rec, r.pos, 0) as u8;
                data.push(v);
            }
            5 => {
                // text
                let v = store.get_str(store.get_u32_raw(r.rec, r.pos));
                v.as_bytes().iter().for_each(|&x| data.push(x));
            }
            _ => match self.types[tp as usize].parts.clone() {
                Parts::Struct(s) | Parts::EnumValue(_, s) => {
                    for f in &s {
                        if f.name == "enum" || f.position == u16::MAX {
                            continue;
                        }
                        let field_r = DbRef {
                            store_nr: r.store_nr,
                            rec: r.rec,
                            pos: r.pos + u32::from(f.position),
                        };
                        self.read_data(&field_r, f.content, little_endian, data);
                    }
                }
                Parts::Enum(_) => {
                    data.push(store.get_byte(r.rec, r.pos, 0) as u8);
                }
                Parts::Byte(_, _) => {
                    data.push(store.get_byte(r.rec, r.pos, 0) as u8);
                }
                Parts::Short(_, _) => {
                    let v = store.get_short(r.rec, r.pos, 0) as i16;
                    (if little_endian {
                        v.to_le_bytes()
                    } else {
                        v.to_be_bytes()
                    })
                    .iter()
                    .for_each(|&x| data.push(x));
                }
                Parts::ShortRaw(from, _) => {
                    let v = store.get_i16_raw(r.rec, r.pos, from) as i16;
                    (if little_endian {
                        v.to_le_bytes()
                    } else {
                        v.to_be_bytes()
                    })
                    .iter()
                    .for_each(|&x| data.push(x));
                }
                Parts::IntRaw(_, _) => {
                    let v = store.get_u32_raw(r.rec, r.pos);
                    (if little_endian {
                        v.to_le_bytes()
                    } else {
                        v.to_be_bytes()
                    })
                    .iter()
                    .for_each(|&x| data.push(x));
                }
                Parts::Int(_, _) => {
                    let v = store.get_i32_raw(r.rec, r.pos);
                    (if little_endian {
                        v.to_le_bytes()
                    } else {
                        v.to_be_bytes()
                    })
                    .iter()
                    .for_each(|&x| data.push(x));
                }
                Parts::Vector(elem_tp) => {
                    let v_rec = {
                        let store = &self.allocations[r.store_nr as usize];
                        store.get_u32_raw(r.rec, r.pos)
                    };
                    let length = if v_rec == 0 {
                        0u32
                    } else {
                        let store = &self.allocations[r.store_nr as usize];
                        store.get_u32_raw(v_rec, 4)
                    };
                    let elem_size = u32::from(self.size(elem_tp));
                    let store_nr = r.store_nr;
                    for i in 0..length {
                        let elem = DbRef {
                            store_nr,
                            rec: v_rec,
                            pos: 8 + elem_size * i,
                        };
                        self.read_data(&elem, elem_tp, little_endian, data);
                    }
                }
                Parts::Array(elem_tp) => {
                    let store_nr = r.store_nr;
                    let store = &self.allocations[store_nr as usize];
                    let v_rec = store.get_u32_raw(r.rec, r.pos);
                    let length = if v_rec == 0 {
                        0u32
                    } else {
                        store.get_u32_raw(v_rec, 4)
                    };
                    // Collect elm_recs before the mutable borrow in read_data.
                    let elm_recs: Vec<u32> = (0..length)
                        .map(|i| store.get_u32_raw(v_rec, 8 + 4 * i))
                        .collect();
                    for elm_rec in elm_recs {
                        let elem = DbRef {
                            store_nr,
                            rec: elm_rec,
                            pos: 8,
                        };
                        self.read_data(&elem, elem_tp, little_endian, data);
                    }
                }
                Parts::Sorted(_, _)
                | Parts::Ordered(_, _)
                | Parts::Hash(_, _)
                | Parts::Index(_, _, _)
                | Parts::Radix(_, _)
                | Parts::Trie(_, _) => panic!(
                    "binary I/O not supported for type '{}': it contains a collection field \
                     with store-internal references that cannot be serialized",
                    self.types[tp as usize].name
                ),
                // Plan-06 phase 4d.C step 2: stored DbRef pointer is
                // store-internal — same restriction as collection fields,
                // can't be serialised across machines.
                Parts::DbRef => panic!(
                    "binary I/O not supported for type '{}': stored DbRef \
                     pointers are store-internal and cannot be serialised",
                    self.types[tp as usize].name
                ),
                // P213: child-record pointer is store-internal; closures
                // can't be serialised across machines either.
                Parts::ChildRec(_) => panic!(
                    "binary I/O not supported for type '{}': child-record \
                     pointers (closures) are store-internal and cannot be \
                     serialised",
                    self.types[tp as usize].name
                ),
                Parts::Base => unreachable!(
                    "Parts::Base should never appear as a field type in read_data \
                     (type: {})",
                    self.types[tp as usize].name
                ),
            },
        }
    }

    /// Return the number of bytes that `read_data` will append for the given type.
    /// Returns 0 for types whose binary size is variable (text) or unsupported (collections).
    fn binary_size(&self, tp: u16) -> usize {
        match tp {
            2 | 6 => 4,     // single, character
            0 | 1 | 3 => 8, // integer (post-2c i64), long, float
            4 => 1,         // boolean
            5 => 0,         // text: variable length
            _ => match &self.types[tp as usize].parts {
                Parts::Struct(s) | Parts::EnumValue(_, s) => s
                    .iter()
                    .filter(|f| f.name != "enum" && f.position != u16::MAX)
                    .map(|f| self.binary_size(f.content))
                    .sum(),
                Parts::Enum(_) | Parts::Byte(_, _) => 1,
                Parts::Short(_, _) | Parts::ShortRaw(_, _) => 2,
                Parts::Int(_, _) | Parts::IntRaw(_, _) => 4,
                _ => 0,
            },
        }
    }

    /// # Panics
    /// If `data` does not contain enough bytes for the given type.
    #[allow(clippy::too_many_lines)]
    pub fn write_data(&mut self, r: &DbRef, tp: u16, little_endian: bool, data: &[u8]) {
        let store = &mut self.allocations[r.store_nr as usize];
        match tp {
            0 => {
                // integer (i64 post-2c)
                let d = data[0..8].try_into().unwrap();
                let v = if little_endian {
                    i64::from_le_bytes(d)
                } else {
                    i64::from_be_bytes(d)
                };
                store.set_int(r.rec, r.pos, v);
            }
            6 => {
                let d = data[0..4].try_into().unwrap();
                let v = if little_endian {
                    u32::from_le_bytes(d)
                } else {
                    u32::from_be_bytes(d)
                };
                store.set_u32_raw(r.rec, r.pos, v);
            }
            1 => {
                // long
                let d = data[0..8].try_into().unwrap();
                let v = if little_endian {
                    i64::from_le_bytes(d)
                } else {
                    i64::from_be_bytes(d)
                };
                store.set_long(r.rec, r.pos, v);
            }
            2 => {
                // single
                let d = data[0..4].try_into().unwrap();
                let v = if little_endian {
                    f32::from_le_bytes(d)
                } else {
                    f32::from_be_bytes(d)
                };
                store.set_single(r.rec, r.pos, v);
            }
            3 => {
                // float
                let d = data[0..8].try_into().unwrap();
                let v = if little_endian {
                    f64::from_le_bytes(d)
                } else {
                    f64::from_be_bytes(d)
                };
                store.set_float(r.rec, r.pos, v);
            }
            4 => {
                // boolean
                let v = data[0];
                store.set_byte(r.rec, r.pos, 0, i32::from(v));
            }
            5 => {
                // text
                let v = unsafe {
                    let mut v = Vec::new();
                    v.extend_from_slice(data);
                    String::from_utf8_unchecked(v)
                };
                let s = store.set_str(v.as_str());
                store.set_u32_raw(r.rec, r.pos, s);
            }
            _ => match self.types[tp as usize].parts.clone() {
                Parts::Struct(s) | Parts::EnumValue(_, s) => {
                    let mut offset = 0usize;
                    for f in &s {
                        if f.name == "enum" || f.position == u16::MAX {
                            continue;
                        }
                        let field_r = DbRef {
                            store_nr: r.store_nr,
                            rec: r.rec,
                            pos: r.pos + u32::from(f.position),
                        };
                        self.write_data(&field_r, f.content, little_endian, &data[offset..]);
                        offset += self.binary_size(f.content);
                    }
                }
                Parts::Enum(_) | Parts::Byte(_, _) => {
                    store.set_byte(r.rec, r.pos, 0, i32::from(data[0]));
                }
                Parts::Short(_, _) => {
                    let d: [u8; 2] = data[0..2].try_into().unwrap();
                    let v = if little_endian {
                        i32::from(i16::from_le_bytes(d))
                    } else {
                        i32::from(i16::from_be_bytes(d))
                    };
                    store.set_short(r.rec, r.pos, 0, v);
                }
                Parts::ShortRaw(from, _) => {
                    let d: [u8; 2] = data[0..2].try_into().unwrap();
                    let v = if little_endian {
                        i32::from(i16::from_le_bytes(d))
                    } else {
                        i32::from(i16::from_be_bytes(d))
                    };
                    store.set_i16_raw(r.rec, r.pos, from, v);
                }
                Parts::Int(_, _) => {
                    let d: [u8; 4] = data[0..4].try_into().unwrap();
                    let v = if little_endian {
                        i32::from_le_bytes(d)
                    } else {
                        i32::from_be_bytes(d)
                    };
                    store.set_i32_raw(r.rec, r.pos, v);
                }
                Parts::IntRaw(_, _) => {
                    let d: [u8; 4] = data[0..4].try_into().unwrap();
                    let v = if little_endian {
                        u32::from_le_bytes(d)
                    } else {
                        u32::from_be_bytes(d)
                    };
                    store.set_u32_raw(r.rec, r.pos, v);
                }
                Parts::Vector(elem_tp) => {
                    let elem_size = u32::from(self.size(elem_tp));
                    if elem_size == 0 {
                        return;
                    }
                    let n_elems = data.len() / elem_size as usize;
                    for i in 0..n_elems {
                        let elem_ref = vector::vector_append(r, elem_size, &mut self.allocations);
                        let slice = &data[i * elem_size as usize..(i + 1) * elem_size as usize];
                        self.write_data(&elem_ref, elem_tp, little_endian, slice);
                        vector::vector_finish(r, &mut self.allocations);
                    }
                }
                Parts::Array(_)
                | Parts::Sorted(_, _)
                | Parts::Ordered(_, _)
                | Parts::Hash(_, _)
                | Parts::Index(_, _, _)
                | Parts::Radix(_, _)
                | Parts::Trie(_, _) => panic!(
                    "binary I/O not supported for type '{}': it contains a collection field \
                     with store-internal references that cannot be serialized",
                    self.types[tp as usize].name
                ),
                // Plan-06 phase 4d.C step 2: see read_data's DbRef arm.
                Parts::DbRef => panic!(
                    "binary I/O not supported for type '{}': stored DbRef \
                     pointers are store-internal and cannot be serialised",
                    self.types[tp as usize].name
                ),
                // P213: see read_data's ChildRec arm.
                Parts::ChildRec(_) => panic!(
                    "binary I/O not supported for type '{}': child-record \
                     pointers (closures) are store-internal and cannot be \
                     serialised",
                    self.types[tp as usize].name
                ),
                Parts::Base => unreachable!(
                    "Parts::Base should never appear as a field type in write_data \
                     (type: {})",
                    self.types[tp as usize].name
                ),
            },
        }
    }

    // Native + `wasm32-wasip2` (WASI filesystem via `--dir` preopens).
    // @P334 fix (2026-05-29): widen the active-impl cfg from
    // `not(target_arch = "wasm32")` to also include `target_os = "wasi"`
    // — std::fs works under wasip2 (proven by OpReadFile/OpWriteFile
    // at codegen_runtime.rs which were already feature-gated, not
    // target-gated).  Without this, get_file's wasm-unknown stub
    // short-circuited every exists()/file() under wasip2 → the
    // world_load assert in lib/hex_world failed → `wasm unreachable`.
    #[cfg(not(host_fs))]
    pub fn get_file(&mut self, file: &DbRef) -> bool {
        if file.rec == 0 {
            return false;
        }
        // #255 / @PLN9: re-home a relative path against the program anchor.
        // Resolve read-only first (drops the borrow), then re-borrow mutably
        // for fill_file.  The original path stays in the struct's `.path` field.
        let raw = {
            let store = self.store(file);
            store
                .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                .to_owned()
        };
        let resolved = self.resolve_path(&raw);
        let store = self.store_mut(file);
        let path = std::path::Path::new(&resolved);
        fill_file(path, store, file)
    }

    /// FS-E: ask the JS host for the metadata.  Both browser shells arrive
    /// here — the wasm-bindgen bundle and, since loft#851, `--html`.
    #[cfg(host_fs)]
    pub fn get_file(&mut self, file: &DbRef) -> bool {
        if file.rec == 0 {
            return false;
        }
        let file_path = {
            let store = self.store_mut(file);
            store
                .get_str(store.get_u32_raw(file.rec, file.pos + 24))
                .to_owned()
        };
        let store = self.store_mut(file);
        store.set_long(file.rec, file.pos + 8, i64::MIN);
        store.set_long(file.rec, file.pos + 16, i64::MIN);
        if crate::wasm::host_fs_is_dir(&file_path) {
            store.set_long(file.rec, file.pos, i64::MIN);
            store.set_byte(file.rec, file.pos + 32, 0, Format::Directory as i32);
            true
        } else if crate::wasm::host_fs_is_file(&file_path) {
            let sz = crate::wasm::host_fs_file_size(&file_path);
            store.set_long(file.rec, file.pos, sz.max(0));
            store.set_byte(file.rec, file.pos + 32, 0, Format::TextFile as i32);
            true
        } else if crate::wasm_assets::asset_exists(&file_path) {
            // @P321(c): a PNG `--html` auto-discovered beside the entry file is
            // not in the host filesystem — JS owns its decoded bytes — but it
            // must still report as `TextFile` (size 0) or `file().png()` never
            // reaches the imaging bridge.  Asked AFTER the host so a page that
            // writes over the name gets its own bytes back, which is the same
            // delta-wins-over-base rule the page filesystem uses throughout.
            // No-op (`false`) on the wasm-bindgen bundle, which has no asset table.
            store.set_long(file.rec, file.pos, 0);
            store.set_byte(file.rec, file.pos + 32, 0, Format::TextFile as i32);
            true
        } else {
            fill_absent(store, file);
            false
        }
    }

    #[cfg(not(host_fs))]
    pub fn get_dir(&mut self, file_path: &str, result: &DbRef) -> bool {
        // #255 / @PLN9: re-home a relative dir path against the program anchor.
        let resolved = self.resolve_path(file_path);
        let path = std::path::Path::new(&resolved);
        if let Ok(iter) = std::fs::read_dir(path) {
            let vector = DbRef {
                store_nr: result.store_nr,
                rec: result.rec,
                pos: result.pos,
            };
            let mut res = BTreeMap::new();
            for entry in iter.flatten() {
                if let Some(name) = entry.path().to_str() {
                    // Normalise to forward slashes so loft paths are consistent on
                    // all platforms (Windows returns backslash-separated paths).
                    // Through the shared helper, so a Unix filename that legitimately
                    // contains a backslash is not split into a fake two-segment path —
                    // this listing is data a loft program reads back.
                    res.insert(crate::portable_path::portable_str(name), entry);
                }
                // A non-UTF-8 name degrades that ENTRY (skipped), never the
                // listing: aborting here returned a silently truncated vector.
            }
            for (name, entry) in res {
                let elm = vector::vector_append(&vector, 33, &mut self.allocations);
                let store = self.store_mut(result);
                let name_pos = store.set_str(&name) as i32;
                store.set_u32_raw(elm.rec, elm.pos + 24, name_pos as u32);
                store.set_i32_raw(elm.rec, elm.pos + 28, i32::MIN);
                // Initialize current and next to null (i64::MIN) so they're not shown
                store.set_long(elm.rec, elm.pos + 8, i64::MIN);
                store.set_long(elm.rec, elm.pos + 16, i64::MIN);
                vector::vector_finish(&vector, &mut self.allocations);
                let store = self.store_mut(result);
                // An unstat-able entry (e.g. a dangling symlink) is already a
                // well-formed `NotExists` element by `fill_file`'s own error
                // path; keep listing — a mid-loop abort silently dropped every
                // entry sorting after it.
                fill_file(&entry.path(), store, &elm);
            }
        }
        true
    }

    #[cfg(host_fs)]
    pub fn get_dir(&mut self, file_path: &str, result: &DbRef) -> bool {
        // FS-E: list directory entries from JS host bridge.
        let mut entries = crate::wasm::host_fs_list_dir(file_path);
        entries.sort();
        let vector = DbRef {
            store_nr: result.store_nr,
            rec: result.rec,
            pos: result.pos,
        };
        for name in entries {
            let full = format!("{file_path}/{name}");
            let elm = vector::vector_append(&vector, 33, &mut self.allocations);
            let store = self.store_mut(result);
            let name_pos = store.set_str(&full) as i32;
            store.set_u32_raw(elm.rec, elm.pos + 24, name_pos as u32);
            store.set_i32_raw(elm.rec, elm.pos + 28, i32::MIN);
            store.set_long(elm.rec, elm.pos + 8, i64::MIN);
            store.set_long(elm.rec, elm.pos + 16, i64::MIN);
            // Fill metadata for this entry.
            let is_dir = crate::wasm::host_fs_is_dir(&full);
            let is_file = crate::wasm::host_fs_is_file(&full);
            let store = self.store_mut(result);
            if is_dir {
                store.set_long(elm.rec, elm.pos, i64::MIN);
                store.set_byte(elm.rec, elm.pos + 32, 0, 4); // Directory
            } else if is_file {
                let sz = crate::wasm::host_fs_file_size(&full);
                store.set_long(elm.rec, elm.pos, sz.max(0));
                store.set_byte(elm.rec, elm.pos + 32, 0, 1); // TextFile
            } else {
                store.set_byte(elm.rec, elm.pos + 32, 0, 5); // NotExists
            }
            vector::vector_finish(&vector, &mut self.allocations);
        }
        true
    }

    /**
    Read the binary data from a png image.
    # Panics
    On file system problems
    */
    #[cfg(feature = "png")]
    pub fn get_png(&mut self, file_path: &str, result: &DbRef) -> bool {
        // #255 / @PLN9: re-home against the program anchor.  Outside the project
        // reads as absent, like any other file (loft#708).
        let resolved = self.resolve_path(file_path);
        let store = self.store_mut(result);
        if let Ok((img, width, height)) = crate::png_store::read(&resolved, store) {
            if let Some(name) = std::path::Path::new(&resolved).file_name() {
                let name_pos = store.set_str(name.to_str().unwrap());
                store.set_u32_raw(result.rec, result.pos, name_pos);
                store.set_int(result.rec, result.pos + 4, i64::from(width));
                store.set_int(result.rec, result.pos + 8, i64::from(height));
                store.set_u32_raw(result.rec, result.pos + 12, img);
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    #[cfg(not(feature = "png"))]
    #[allow(clippy::unused_self)]
    pub fn get_png(&mut self, _file_path: &str, _result: &DbRef) -> bool {
        false
    }

    /// Write `v` as UTF-8 text to `file`.  Returns whether the OS write
    /// succeeded — `false` on a failed create (perm denied, bad path) or a
    /// failed `write_all` (disk full, closed handle) — so the `write` builtin can
    /// surface a `FileResult` instead of swallowing the failure (H3).
    pub fn write_file(&mut self, file: &DbRef, v: &str) -> bool {
        #[cfg(host_fs)]
        {
            // FS-E: write text content via JS host bridge (best-effort; the host
            // shim does not return a status, so a browser write reports success).
            let file_path = {
                let s = self.store_mut(file);
                s.get_str(s.get_u32_raw(file.rec, file.pos + 24)).to_owned()
            };
            crate::wasm::host_fs_write_text(&file_path, v);
            true
        }
        #[cfg(not(host_fs))]
        {
            let f_nr = self.files.len() as i32;
            // #255 / @PLN9: resolve against the program anchor before borrowing
            // the store mutably.
            let resolved = {
                let s = self.store(file);
                let raw = s.get_str(s.get_u32_raw(file.rec, file.pos + 24)).to_owned();
                self.resolve_path(&raw)
            };
            let resolved_name = resolved;
            let s = self.store_mut(file);
            let mut file_ref = s.get_i32_raw(file.rec, file.pos + 28);
            if file_ref == i32::MIN {
                match std::fs::File::create(&resolved_name) {
                    Ok(f) => {
                        s.set_i32_raw(file.rec, file.pos + 28, f_nr);
                        self.files.push(Some(f));
                        file_ref = f_nr;
                    }
                    Err(e) => {
                        // A failed create must NOT leave a dangling ref —
                        // indexing self.files with it panicked (CI-caught on
                        // a Windows runner with a transient FS refusal).
                        // Warn and skip the write (the recoverable-fault
                        // posture); the next write retries the create.
                        crate::loft_eprintln!(
                            "loft: cannot create {resolved_name} for writing: {e} — write skipped"
                        );
                        return false;
                    }
                }
            }
            if let Some(Some(f)) = self.files.get_mut(file_ref as usize) {
                f.write_all(v.as_bytes()).is_ok()
            } else {
                false
            }
        }
    }

    /// List the entry NAMES of directory `path` as a `vector<text>` (base
    /// names, not full paths — matching the `host_fs_list_dir` host shape).
    /// Entries are sorted for a deterministic order.  Returns an empty vector
    /// when the path is not a readable directory.  Drives the `list_dir`
    /// builtin on both backends via its `#rust` template.
    #[must_use]
    pub fn fs_list_dir(&mut self, path: &str) -> DbRef {
        // #255 / @PLN9: re-home a relative dir path against the program anchor.
        // Outside the project reads as absent — the same NULL a missing
        // directory gives (loft#708).
        let resolved = self.resolve_path(path);
        // @PLN102 H4 — a MISSING / non-directory path lists as NULL (the null
        // vector), distinct from an EMPTY-but-present directory (a length-0 vector).
        #[cfg(host_fs)]
        let names: Option<Vec<String>> = {
            // The host bridge can't cheaply distinguish empty-vs-missing here; treat
            // a non-directory as absent via the is-dir probe.
            if crate::wasm::host_fs_is_dir(&resolved) {
                Some(crate::wasm::host_fs_list_dir(&resolved))
            } else {
                None
            }
        };
        #[cfg(not(host_fs))]
        let names: Option<Vec<String>> = match std::fs::read_dir(std::path::Path::new(&resolved)) {
            Ok(iter) => {
                let mut v = Vec::new();
                for entry in iter.flatten() {
                    // Keep only the final path component; skip non-UTF-8 names
                    // (that ENTRY degrades, never the whole listing).
                    if let Some(name) = entry.file_name().to_str() {
                        v.push(name.to_owned());
                    }
                }
                Some(v)
            }
            Err(_) => None,
        };
        let Some(mut names) = names else {
            return DbRef::NULL;
        };
        names.sort();
        self.text_vector(&names)
    }

    /// Read the entire file `path` as a `vector<u8>`.  Returns an empty vector
    /// on a missing file / IO error / non-regular path.  Binary-exact: a
    /// non-UTF-8 blob round-trips byte-for-byte through `fs_write_bytes`.
    /// Drives the `read_bytes` builtin on both backends via its `#rust`
    /// template.
    #[must_use]
    pub fn fs_read_bytes(&mut self, path: &str) -> DbRef {
        // #255 / @PLN9: re-home against the program anchor.  Outside the
        // project reads as absent — the same NULL a missing file gives
        // (loft#708).
        let resolved = self.resolve_path(path);
        // @PLN102 H4 — a MISSING / unreadable file reads as NULL (the null vector),
        // distinct from an EMPTY-but-present file (a length-0 vector).  `None` is the
        // absent case; `Some(v)` (possibly empty) is a real read.
        // The WHOLE file, so the host's per-path cursor must not be consulted:
        // `host_fs_read_bytes` reads `n` bytes FROM the cursor, and after any
        // preceding write that cursor sits at the end — which read back zero
        // bytes from a file that had just been written (loft#851's probe cell E).
        // `host_fs_read_binary` answers the file, and `None` iff it is absent,
        // which is also the distinction the size probe was standing in for.
        #[cfg(host_fs)]
        let data: Option<Vec<u8>> = crate::wasm::host_fs_read_binary(&resolved);
        #[cfg(not(host_fs))]
        let data: Option<Vec<u8>> = std::fs::read(std::path::Path::new(&resolved)).ok();
        let Some(data) = data else { return DbRef::NULL };
        // Owning field is a 4-byte vector pointer; the inner record holds the
        // bytes one-per-element (length at offset 4, payload at offset 8).
        let vec = self.database(4);
        let count = data.len() as u32;
        let rec = vector::alloc_vector_from_bytes(self.store_mut(&vec), 1, count, &data);
        self.store_mut(&vec).set_u32_raw(vec.rec, vec.pos, rec);
        vec
    }

    /// Write `bytes` (a `vector<u8>`) to file `path`, truncating any existing
    /// content.  Returns `true` on success.  The inverse of `fs_read_bytes`;
    /// the pair round-trips a non-UTF-8 blob byte-for-byte.  Drives the
    /// `write_bytes` builtin on both backends via its `#rust` template.
    #[must_use]
    pub fn fs_write_bytes(&mut self, path: &str, bytes: DbRef) -> bool {
        // #255 / @PLN9: re-home against the program anchor.  A path `file()`
        // reports absent must not be creatable here (loft#708); `false` is the
        // documented failure answer.
        let resolved = self.resolve_path(path);
        // Read the byte payload out of the `vector<u8>` (same layout
        // `text_from_bytes_native` reads): inner record at (bytes.rec,
        // bytes.pos), live length at offset 4, payload from offset 8.
        let length = vector::length_vector(&bytes, &self.allocations);
        let data: Vec<u8> = if bytes.rec == 0 || bytes.pos == 0 || length == 0 {
            Vec::new()
        } else {
            let store = self.store_mut(&bytes);
            let vec_rec = store.get_u32_raw(bytes.rec, bytes.pos);
            if vec_rec == 0 {
                Vec::new()
            } else {
                store.buffer(vec_rec)[..length as usize].to_vec()
            }
        };
        // Truncating, so this is `write_binary` and not the cursor-relative
        // `write_bytes` — that one appends at wherever the path's cursor was
        // left, which neither replaces the old content nor starts at 0.  The
        // native arm below is `std::fs::write`, whose contract this matches.
        #[cfg(host_fs)]
        {
            crate::wasm::host_fs_write_binary(&resolved, &data) == 0
        }
        #[cfg(not(host_fs))]
        {
            std::fs::write(std::path::Path::new(&resolved), &data).is_ok()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue 59: `read_data Parts::Struct` must use `r.pos + f.position` for every
    /// field, not `r.pos` for all fields.  Without the fix both fields serialise the same
    /// value (the first field is read twice).
    #[test]
    fn read_data_struct_field_positions() {
        let mut stores = Stores::new();
        // Define struct Pair { a: integer, b: integer }
        let pair_tp = stores.structure("Pair", -1);
        stores.field(pair_tp, "a", 0); // integer
        stores.field(pair_tp, "b", 0); // integer
        stores.finish();

        // Allocate a record big enough for two 8-byte integers (post-2c).
        //   word 0: header
        //   field a at db.pos + 0  (struct-position 0)
        //   field b at db.pos + 8  (struct-position 8, post-2c integer stride)
        let db = stores.database(4);
        {
            let s = &mut stores.allocations[db.store_nr as usize];
            s.set_int(db.rec, db.pos, 10); // a = 10
            s.set_int(db.rec, db.pos + 8, 20); // b = 20 — integer is 8B post-2c
        }

        let mut data = Vec::new();
        stores.read_data(&db, pair_tp, true, &mut data);

        assert_eq!(data.len(), 16, "Pair binary size: 2 × i64 = 16 bytes");
        let a_val = i64::from_le_bytes(data[0..8].try_into().unwrap());
        let b_val = i64::from_le_bytes(data[8..16].try_into().unwrap());
        assert_eq!(a_val, 10, "field a should be 10");
        assert_eq!(b_val, 20, "field b should be 20 (was 10 before fix)");
    }

    /// Issue 59: `write_data Parts::Struct` must use `r.pos + f.position` for every
    /// field and advance the data-slice offset between fields.
    #[test]
    fn write_data_struct_field_positions() {
        let mut stores = Stores::new();
        let pair_tp = stores.structure("Pair", -1);
        stores.field(pair_tp, "a", 0);
        stores.field(pair_tp, "b", 0);
        stores.finish();

        let db = stores.database(4);
        // Write [a=77_le, b=99_le] into the struct — post-2c integer = 8B
        let bytes: Vec<u8> = {
            let mut v = 77i64.to_le_bytes().to_vec();
            v.extend_from_slice(&99i64.to_le_bytes());
            v
        };
        stores.write_data(&db, pair_tp, true, &bytes);

        let s = &stores.allocations[db.store_nr as usize];
        let a_val = s.get_int(db.rec, db.pos);
        let b_val = s.get_int(db.rec, db.pos + 8);
        assert_eq!(a_val, 77, "field a should be 77");
        assert_eq!(b_val, 99, "field b should be 99 (was 77 before fix)");
    }
}
