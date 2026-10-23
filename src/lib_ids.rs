// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: the keys a library artifact names its types and functions by

//! @PLN199 G0b-2 — the names a library artifact gives the types and functions its code refers
//! to, so the code does not depend on the numbers one program happened to give them.
//!
//! A type id and a function number are positions in ONE program's tables: a second program
//! that loads another library first numbers the same library's types and functions
//! differently.  A library cdylib therefore carries, beside its code, the KEY of every id that
//! code names ([`TypeNames::key`], [`fn_key`]); the host answers each key with its own number
//! when it loads the artifact ([`host_type_id`], [`host_fn_id`]), and the code reads the
//! numbers from that table.  Both sides compute a key from their own `Data` and `Stores` with
//! the same function, so a key means one type in both.
//!
//! A key is made of names that do not move between programs: a source's
//! [`Data::source_label`] (`std`, `main`, a library's name) with a definition's name, and for a
//! type without a definition of its own — a vector, a keyed collection, a tuple record — the
//! keys of what it is made of.

use crate::data::{Data, DefType};
use crate::database::{BASE_TYPES, Parts, Stores};
use std::collections::HashMap;
use std::fmt::Write as _;

/// The keys of one program's types.
pub struct TypeNames<'a> {
    data: &'a Data,
    stores: &'a Stores,
    /// The definition that declares each named type (a struct, an enum, a data-enum variant).
    declared: HashMap<u16, u32>,
}

impl<'a> TypeNames<'a> {
    #[must_use]
    pub fn new(data: &'a Data, stores: &'a Stores) -> Self {
        let mut declared: HashMap<u16, u32> = HashMap::new();
        for d in 0..data.definitions() {
            let def = data.def(d);
            let tp = def.known_type();
            if tp == u16::MAX
                || !matches!(
                    def.def_type(),
                    DefType::Struct | DefType::Enum | DefType::EnumValue
                )
                // A type spelled from other types (`__tuple<…>`, `main_vector<…>`,
                // `__nullable<…>`, a generic instance) is minted by whichever source named it
                // first, so its definition's source is the program's; its parts name it.  A
                // type variable's placeholder (`T#4`) is numbered by the generics before it.
                || def.name().contains('<')
                || def.name().contains('#')
            {
                continue;
            }
            // Two definitions of one type (an enum and its variants): the type's own, then
            // the first by name — an order both sides share, where definition numbers are not.
            let named = |n: u32| {
                let def = data.def(n);
                (
                    def.def_type() == DefType::EnumValue,
                    data.source_label(def.source),
                    def.name().to_string(),
                )
            };
            declared
                .entry(tp)
                .and_modify(|e| {
                    if named(d) < named(*e) {
                        *e = d;
                    }
                })
                .or_insert(d);
        }
        TypeNames {
            data,
            stores,
            declared,
        }
    }

    /// The key of type `tp`.
    #[must_use]
    pub fn key(&self, tp: u16) -> String {
        let mut out = String::new();
        self.write_key(&mut out, tp, &mut Vec::new());
        out
    }

    /// `path` holds the types being keyed around this one: a type that contains itself (a
    /// tree's `kids: vector<Tree<…>>`) is named by how far up the path it is (`^2`).
    fn write_key(&self, out: &mut String, tp: u16, path: &mut Vec<u16>) {
        if tp < BASE_TYPES {
            let _ = write!(out, "{tp}");
            return;
        }
        if let Some(up) = path.iter().rev().position(|&t| t == tp) {
            let _ = write!(out, "^{up}");
            return;
        }
        if let Some(&d) = self.declared.get(&tp) {
            let def = self.data.def(d);
            let _ = write!(
                out,
                "{}::{}",
                self.data.source_label(def.source),
                def.name()
            );
            return;
        }
        let Some(row) = self.stores.types.get(usize::from(tp)) else {
            let _ = write!(out, "?{tp}");
            return;
        };
        path.push(tp);
        let sub = |out: &mut String, path: &mut Vec<u16>, label: &str, c: u16, keys: &str| {
            let _ = write!(out, "{label}<");
            self.write_key(out, c, path);
            let _ = write!(out, ">{keys}");
        };
        match &row.parts {
            Parts::Vector(c) => sub(out, path, "vector", *c, ""),
            Parts::Array(c) => sub(out, path, "array", *c, ""),
            Parts::ChildRec(c) => sub(out, path, "child", *c, ""),
            Parts::Sorted(c, k) => sub(out, path, "sorted", *c, &format!("{k:?}")),
            Parts::Ordered(c, k) => sub(out, path, "ordered", *c, &format!("{k:?}")),
            Parts::Index(c, k) => sub(out, path, "index", *c, &format!("{k:?}")),
            Parts::Hash(c, k) => sub(out, path, "hash", *c, &format!("{k:?}")),
            Parts::Radix(c, k) => sub(out, path, "radix", *c, &format!("{k:?}")),
            Parts::Trie(c, k) => sub(out, path, "trie", *c, &format!("[{k}]")),
            // A type variable's placeholder is numbered by the generics parsed before it
            // (`__typevar_T#4`); every placeholder is the same empty record.
            Parts::Struct(_) if row.name.starts_with("__typevar_") => out.push_str("typevar"),
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                // The stem names the kind (`main_vector`, `__tuple`, a type variable); the
                // element names inside its `<…>` are the program's spelling, so the fields
                // stand for them.
                let stem = row.name.split('<').next().unwrap_or("");
                if let Parts::EnumValue(v, _) = &row.parts {
                    let _ = write!(out, "{stem}#{v}{{");
                } else {
                    let _ = write!(out, "{stem}{{");
                }
                for (i, f) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    let _ = write!(out, "{}:", f.name);
                    self.write_key(out, f.content, path);
                }
                out.push('}');
            }
            Parts::Enum(values) => {
                // A variant's type is named by its key; a plain value has none.
                out.push_str("enum{");
                for (i, (vt, name)) in values.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    let _ = write!(out, "{name}:");
                    if *vt == u16::MAX {
                        out.push('-');
                    } else {
                        self.write_key(out, *vt, path);
                    }
                }
                out.push('}');
            }
            other => {
                let _ = write!(out, "{other:?}");
            }
        }
        path.pop();
    }
}

/// The key of function `d_nr`: its source's label and its name, unique within a source.
#[must_use]
pub fn fn_key(data: &Data, d_nr: u32) -> String {
    let def = data.def(d_nr);
    format!("{}::{}", data.source_label(def.source), def.name())
}

/// The host's side of the lookup: every type and function of the program that loaded a
/// library artifact, by key.  Built once, on the first key an artifact asks for.
pub struct HostIds<'a> {
    data: &'a Data,
    stores: &'a Stores,
    built: std::cell::OnceCell<(HashMap<String, u16>, HashMap<String, u32>)>,
    /// The first key asked for that this program has no answer to: what a refusal names.
    missing: std::cell::RefCell<Option<String>>,
}

impl<'a> HostIds<'a> {
    #[must_use]
    pub fn new(data: &'a Data, stores: &'a Stores) -> Self {
        HostIds {
            data,
            stores,
            built: std::cell::OnceCell::new(),
            missing: std::cell::RefCell::new(None),
        }
    }

    fn tables(&self) -> &(HashMap<String, u16>, HashMap<String, u32>) {
        self.built.get_or_init(|| {
            let names = TypeNames::new(self.data, self.stores);
            let trace = std::env::var_os("LOFT_TRACE_LIB_IDS").is_some();
            let mut types = HashMap::new();
            for tp in 0..u16::try_from(self.stores.types.len()).unwrap_or(u16::MAX) {
                let key = names.key(tp);
                if trace {
                    eprintln!("[lib-ids] type {tp} = {key}");
                }
                types.entry(key).or_insert(tp);
            }
            let mut fns = HashMap::new();
            for d in 0..self.data.definitions() {
                fns.entry(fn_key(self.data, d)).or_insert(d);
            }
            (types, fns)
        })
    }

    /// The host's id of the type named `key`, or `u16::MAX`.
    #[must_use]
    pub fn type_id(&self, key: &str) -> u16 {
        let id = self.tables().0.get(key).copied();
        self.note_missing(id.is_none(), key);
        id.unwrap_or(u16::MAX)
    }

    /// The host's number of the function named `key`, or `u32::MAX`.
    #[must_use]
    pub fn fn_id(&self, key: &str) -> u32 {
        let id = self.tables().1.get(key).copied();
        self.note_missing(id.is_none(), key);
        id.unwrap_or(u32::MAX)
    }

    fn note_missing(&self, missing: bool, key: &str) {
        if missing {
            self.missing
                .borrow_mut()
                .get_or_insert_with(|| key.to_string());
        }
    }

    /// The first key an artifact named that this program lacks (taken: a second artifact's
    /// refusal names its own).
    pub fn take_missing(&self) -> Option<String> {
        self.missing.borrow_mut().take()
    }
}

/// [`HostTable::type_id`](crate::native_lib::HostTable::type_id): `ctx` is the [`HostIds`]
/// the host passed to the artifact's `loft_shared_init`.
///
/// # Safety
/// `ctx` points at a live `HostIds`, and `key` at `len` bytes of UTF-8.
pub unsafe extern "C" fn host_type_id(
    ctx: *const std::ffi::c_void,
    key: *const u8,
    len: usize,
) -> u16 {
    let ids = unsafe { &*ctx.cast::<HostIds<'_>>() };
    let key = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(key, len)) };
    ids.type_id(key)
}

/// [`HostTable::fn_id`](crate::native_lib::HostTable::fn_id), as [`host_type_id`].
///
/// # Safety
/// As [`host_type_id`].
pub unsafe extern "C" fn host_fn_id(
    ctx: *const std::ffi::c_void,
    key: *const u8,
    len: usize,
) -> u32 {
    let ids = unsafe { &*ctx.cast::<HostIds<'_>>() };
    let key = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(key, len)) };
    ids.fn_id(key)
}
