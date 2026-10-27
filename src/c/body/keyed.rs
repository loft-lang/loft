// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! The keyed collections — hash, sorted, index — as C: filing a new record, a lookup by key,
//! a walk over a key range, a removal, a clear and a length.  The runtime half is
//! `loft_keyed.h`; a key part's value is a `loft_kv`, its description a `loft_key`.

use super::{
    Coll, E, FnWriter, K, as_bool, as_char, as_enum, as_float, as_int, as_ref, as_single, as_view,
};
use crate::c::{CType, Keyed};
use crate::data::Value;
use crate::database::Parts;
use crate::keys::Key;
use std::fmt::Write as _;

/// The C description of key parts: a `loft_key` array literal.
fn key_array(keys: &[Key]) -> String {
    if keys.is_empty() {
        return "0".into();
    }
    let parts: Vec<String> = keys
        .iter()
        .map(|k| format!("{{ {}, {}, {} }}", k.type_nr, k.position, k.start))
        .collect();
    format!("(const loft_key[]){{ {} }}", parts.join(", "))
}

/// One key part's value as a `loft_kv`, for a part of kind `type_nr`.
fn key_value(e: E, type_nr: i8) -> Result<String, String> {
    Ok(match type_nr.unsigned_abs() {
        3 => format!("loft_kv_single({})", as_single(e)?),
        4 => format!("loft_kv_float({})", as_float(e)?),
        6 => format!("loft_kv_text({})", as_view(e)?),
        _ => match e.kind {
            K::Bool | K::Cond => format!("loft_kv_int((int64_t) {})", as_bool(e)?),
            K::Enum => format!("loft_kv_int((int64_t) {})", as_enum(e)?),
            K::Char => format!("loft_kv_int((int64_t) {})", as_char(e)?),
            _ => format!("loft_kv_int({})", as_int(e)?),
        },
    })
}

/// `n` key values as a `loft_kv` array literal, or the null pointer for none.
fn value_array(values: &[String]) -> String {
    if values.is_empty() {
        "0".into()
    } else {
        format!("(loft_kv[]){{ {} }}", values.join(", "))
    }
}

impl FnWriter<'_> {
    /// The keyed collection a type number names, its element type and element size, or a
    /// refusal naming the kind.
    fn keyed_of(&self, tp: u16) -> Result<(Keyed, u16, u16), String> {
        let (kind, elem) = self
            .cx
            .keyed(tp)
            .ok_or_else(|| format!("a keyed operation on a {}", self.kind_word(tp)))?;
        Ok((kind, elem, self.cx.stores.size(elem)))
    }

    /// The static type of a variable read, the one shape a persisted value takes; `None` for
    /// any other value.
    pub(super) fn value_type(&self, v: &Value) -> Option<crate::data::Type> {
        match v.unspan() {
            Value::Var(var) => Some(self.vars.tp(*var).clone()),
            _ => None,
        }
    }

    /// The kind of type `tp` in a word, for a refusal.
    fn kind_word(&self, tp: u16) -> String {
        self.cx.stores.types.get(tp as usize).map_or_else(
            || "type out of range".into(),
            |t| {
                let kind = format!("{:?}", t.parts);
                kind.split(['(', ' '])
                    .next()
                    .unwrap_or("value")
                    .to_lowercase()
            },
        )
    }

    /// `OpNewRecord` in a keyed collection `c` of type `tp`: a sorted stages the element past
    /// its end; a hash takes an entry of its table (`Stores::hash_entry`); an index gets a
    /// record of its own.
    pub(super) fn keyed_new(&mut self, c: &str, tp: u16, ind: usize) -> Result<E, String> {
        let (kind, elem, size) = self.keyed_of(tp)?;
        let (image, n) = self.cx.default_image(elem);
        let t = self.temp(CType::Ref);
        let make = match kind {
            Keyed::Sorted => format!("loft_sorted_new({c}, {size})"),
            Keyed::Hash => format!(
                "loft_hash_entry({c}, {size}, {})",
                u8::from(self.cx.stores.is_linked(elem))
            ),
            _ => format!("loft_entry_new({c}, {size})"),
        };
        self.line(ind, &format!("{t} = {make};"));
        self.line(ind, &format!("loft_fill({t}, {image}, {n});"));
        Ok(E::new(t, K::Ref))
    }

    /// `OpFinishRecord` in a keyed collection: the record goes to its key's place
    /// (`@FR-Col-Insert`: an equal key's element is replaced).
    pub(super) fn keyed_finish(&mut self, c: &str, rec: &str, tp: u16) -> Result<E, String> {
        let (kind, elem, size) = self.keyed_of(tp)?;
        let (keys, n) = self.cx.keys_of(tp);
        Ok(E::new(
            match kind {
                Keyed::Sorted => format!("loft_sorted_finish({c}, {size}, {keys}, {n})"),
                Keyed::Hash => {
                    // A displaced entry releases what it owns: its type is in the table.
                    self.cx.name_types();
                    self.cx.describe_type(elem);
                    format!(
                        "(void) loft_hash_insert(loft_types, {elem}, {c}, {rec}, {keys}, {n}, 0)"
                    )
                }
                Keyed::Index | Keyed::Ordered => {
                    // A displaced record releases what it owns: its type is in the table.
                    self.cx.name_types();
                    self.cx.describe_type(elem);
                    let insert = if kind == Keyed::Index {
                        "loft_ix_insert"
                    } else {
                        "loft_ordered_insert"
                    };
                    format!("(void) {insert}(loft_types, {elem}, {c}, {rec}, {keys}, {n}, 0)")
                }
                Keyed::Spatial | Keyed::Trie => {
                    self.cx.name_types();
                    self.cx.describe_type(elem);
                    format!(
                        "(void) loft_rx_insert_record(loft_types, {elem}, {c}, {rec}, {keys}, {n}, {}, 0)",
                        u8::from(kind == Keyed::Trie)
                    )
                }
                Keyed::Array => format!("loft_array_append({c}, {rec})"),
            },
            K::Void,
        ))
    }

    /// `OpFinishRecord` in a member of a linked group: the record enters every member
    /// (`@FR-Col-Group`).
    pub(super) fn group_finish(
        &mut self,
        parent: &str,
        rec: &str,
        members: &[(u16, u16)],
    ) -> Result<E, String> {
        let (rows, n) = self.cx.member_rows(members)?;
        let (elem, held, nullable) = self.cx.group_release(members)?;
        Ok(E::new(
            format!(
                "loft_group_finish(loft_types, {elem}, {held}, {nullable}, {parent}, {rec}, {rows}, {n})"
            ),
            K::Void,
        ))
    }

    /// The operators that keep a linked group one record set (`@FR-Col-Group`): a record the
    /// vector member holds handed to the other members after an element write
    /// (`OpLinkRecord`), and a whole vector indexed into one view member (`OpIndexGroup`).
    fn group_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        match (name, args) {
            ("OpLinkRecord", [data, rec, ptp, fld]) => {
                let ptp = u16::try_from(Self::literal(ptp, "type")?)
                    .map_err(|_| "a type number out of range".to_string())?;
                let fld = u16::try_from(Self::literal(fld, "field")?)
                    .map_err(|_| "a field number out of range".to_string())?;
                self.group_parent_plain(ptp)?;
                let siblings = self.cx.group_siblings(ptp, fld);
                let (data, rec) = self.two(name, &[data.clone(), rec.clone()], ind)?;
                if siblings.is_empty() {
                    return Ok(Some(E::new("(void) 0", K::Void)));
                }
                let (rows, n) = self.cx.member_rows(&siblings)?;
                let stores = self.cx.stores;
                let mut all = siblings.clone();
                all.push((stores.field_position(ptp, fld), stores.field_type(ptp, fld)));
                let (elem, held, nullable) = self.cx.group_release(&all)?;
                Ok(Some(E::new(
                    format!(
                        "loft_group_finish(loft_types, {elem}, {held}, {nullable}, {}, {}, {rows}, {n})",
                        as_ref(data)?,
                        as_ref(rec)?
                    ),
                    K::Void,
                )))
            }
            ("OpIndexGroup", [primary, _view, tp, ptp, fld]) => {
                let tp = u16::try_from(Self::literal(tp, "type")?)
                    .map_err(|_| "a type number out of range".to_string())?;
                let ptp = u16::try_from(Self::literal(ptp, "type")?)
                    .map_err(|_| "a type number out of range".to_string())?;
                let fld = u16::try_from(Self::literal(fld, "field")?)
                    .map_err(|_| "a field number out of range".to_string())?;
                self.group_parent_plain(ptp)?;
                let stores = self.cx.stores;
                let prim = stores.field_position(ptp, fld);
                let members = self.cx.group_siblings(ptp, fld);
                let Some(&(vpos, _)) = members.iter().find(|(_, t)| *t == tp) else {
                    return Err("a group view that is no member of the group".into());
                };
                let keyed: Vec<(u16, u16)> = members
                    .iter()
                    .copied()
                    .filter(|&(_, t)| !matches!(self.keyed_of(t), Ok((Keyed::Array, _, _))))
                    .collect();
                let view = self.cx.member_row(vpos, tp)?;
                let (rows, n) = self.cx.member_rows(&keyed)?;
                let (_, _, nullable) = self.cx.group_release(&members)?;
                let primary = self.expr(primary, ind)?;
                Ok(Some(E::new(
                    format!(
                        "loft_index_group({}, {prim}, {nullable}, (const loft_member[]){{ {view} }}, {rows}, {n})",
                        as_ref(primary)?
                    ),
                    K::Void,
                )))
            }
            _ => Ok(None),
        }
    }

    /// A group whose parent is a `__nullable<S>` payload reads its members through the
    /// variant (`Stores::nullable_field_parent`), which the C does not carry yet.
    fn group_parent_plain(&self, ptp: u16) -> Result<(), String> {
        if self.cx.stores.types[ptp as usize]
            .name
            .starts_with("__nullable<")
        {
            return Err("a linked group inside a nullable record".into());
        }
        Ok(())
    }

    /// The natives over keyed collections, or `None` for another.
    pub(super) fn keyed_native(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        match (name, args) {
            ("n_hash_sorted", [h, tp]) => {
                let tp = u16::try_from(Self::literal(tp, "type")?)
                    .map_err(|_| "a type number out of range".to_string())?;
                let (keys, n) = self.cx.keys_of(tp);
                let h = self.expr(h, ind)?;
                Ok(Some(E::new(
                    format!("loft_hash_sorted({}, {keys}, {n})", as_ref(h)?),
                    K::Ref,
                )))
            }
            ("n_hash_unsorted", [h, _]) => {
                let h = self.expr(h, ind)?;
                Ok(Some(E::new(
                    format!("loft_hash_unsorted({})", as_ref(h)?),
                    K::Ref,
                )))
            }
            ("n_radix_sorted", [r, _]) => {
                let r = self.expr(r, ind)?;
                Ok(Some(E::new(
                    format!("loft_radix_sorted({})", as_ref(r)?),
                    K::Ref,
                )))
            }
            ("n_spatial_range", [r, tp, rest @ ..]) if rest.len() == 8 => {
                let tp = u16::try_from(Self::literal(tp, "type")?)
                    .map_err(|_| "a type number out of range".to_string())?;
                let (keys, n) = self.cx.keys_of(tp);
                let mut all = vec![r.clone()];
                all.extend(rest.iter().cloned());
                let mut es = self.args(&all, ind)?.into_iter();
                let r = as_ref(es.next().ok_or("a slice without its collection")?)?;
                let ints = es.map(as_int).collect::<Result<Vec<_>, _>>()?;
                Ok(Some(E::new(
                    format!("loft_spatial_range({r}, {keys}, {n}, {})", ints.join(", ")),
                    K::Ref,
                )))
            }
            ("n_trie_prefix", [r, tp, pre, limit]) => {
                let tp = u16::try_from(Self::literal(tp, "type")?)
                    .map_err(|_| "a type number out of range".to_string())?;
                let (keys, _) = self.cx.keys_of(tp);
                let mut es = self
                    .args(&[r.clone(), pre.clone(), limit.clone()], ind)?
                    .into_iter();
                let r = as_ref(es.next().ok_or("a slice without its collection")?)?;
                let pre = as_view(es.next().ok_or("a slice without its prefix")?)?;
                let limit = as_int(es.next().ok_or("a slice without its limit")?)?;
                Ok(Some(E::new(
                    format!("loft_trie_prefix({r}, {keys}, {pre}, {limit})"),
                    K::Ref,
                )))
            }
            ("t_4hash_len", [h]) => {
                let h = self.expr(h, ind)?;
                Ok(Some(E::new(
                    format!("loft_hash_len({})", as_ref(h)?),
                    K::Int,
                )))
            }
            _ => self.store_native(name, args, ind),
        }
    }

    /// The static type of the value whose store `v` names: a variable's, and for a field of a
    /// record (`OpGetField`) the record's — the field lives in the record's store, whose root
    /// a whole-store native reads.
    fn store_value_type(&self, v: &Value) -> Option<crate::data::Type> {
        match v.unspan() {
            Value::Call(d, args) if self.cx.data.def(*d).name() == "OpGetField" => {
                self.store_value_type(args.first()?)
            }
            _ => self.value_type(v),
        }
    }

    /// `store_load_url(r, url, sha)` and `store_load_url_trusted(r, url)` (`loft_net.h`): the
    /// gate reads the layout of the store's root type, as `store_load`'s does.
    fn load_url(
        &mut self,
        name: &str,
        r: &Value,
        url: &Value,
        rest: &[Value],
        ind: usize,
    ) -> Result<E, String> {
        // The gate reads the layout of the store's root type, as `store_load`'s does.
        let tp = self
            .store_value_type(r)
            .and_then(|t| self.cx.root_type(&t, self.cx.data.def(self.d).source()))
            .ok_or("a loaded value whose store type is not known here")?;
        // A store holding references is re-pointed at its slot by a walk of its type.
        if self.cx.holds_stored_refs(tp) {
            self.cx.copy_type(tp)?;
            self.cx.name_types();
        }
        let sc = self.cx.sidecar_of(tp);
        let mut all = vec![r.clone(), url.clone()];
        all.extend(rest.iter().cloned());
        let es = self.args(&all, ind)?;
        let mut es = es.into_iter();
        let r = as_ref(es.next().ok_or("a URL load without its value")?)?;
        let url = as_view(es.next().ok_or("a URL load without its URL")?)?;
        let sha = match es.next() {
            Some(e) => format!("{}, ", as_view(e)?),
            None => String::new(),
        };
        let f = if name == "n_store_load_url" {
            "loft_store_load_url"
        } else {
            "loft_store_load_url_trusted"
        };
        Ok(E::new(format!("{f}({r}, {url}, {sha}{sc})"), K::Bool))
    }

    /// The natives of a lazily bound collection (`@PLN129`, `loft_lazy.h`), or `None`.
    fn lazy_native(&mut self, name: &str, args: &[Value], ind: usize) -> Result<Option<E>, String> {
        match (name, args) {
            ("n_store_bind_lazy", [local, source]) => self.bind_lazy(local, source, ind).map(Some),
            ("n_store_lazy_error", [local]) => {
                let r = self.expr(local, ind)?;
                Ok(Some(E::new(
                    format!("loft_lazy_error({})", as_ref(r)?),
                    K::View,
                )))
            }
            ("n_store_lazy_faults", [local]) => {
                let r = self.expr(local, ind)?;
                Ok(Some(E::new(
                    format!("loft_lazy_faults({})", as_ref(r)?),
                    K::Int,
                )))
            }
            ("n_store_lazy_clear", [local]) => {
                let r = self.expr(local, ind)?;
                Ok(Some(E::new(
                    format!("loft_lazy_clear({})", as_ref(r)?),
                    K::Bool,
                )))
            }
            ("n_store_lazy_fail", [local, why]) => {
                let (r, why) = self.two(name, &[local.clone(), why.clone()], ind)?;
                Ok(Some(E::new(
                    format!("loft_lazy_fail({}, {})", as_ref(r)?, as_view(why)?),
                    K::Void,
                )))
            }
            // A database's explicit fetches (`lazy_query`, `lazy_range`), run by the runtime's
            // sqlite source.
            ("n_store_lazy_query", [local, condition]) => {
                let (r, c) = self.two(name, &[local.clone(), condition.clone()], ind)?;
                Ok(Some(E::new(
                    format!("loft_lazy_query({}, {})", as_ref(r)?, as_view(c)?),
                    K::Int,
                )))
            }
            ("n_store_lazy_range", [local, lo, hi]) => {
                let es = self.args(&[local.clone(), lo.clone(), hi.clone()], ind)?;
                let [r, lo, hi] = <[E; 3]>::try_from(es)
                    .map_err(|_| "a range fetch without its three operands".to_string())?;
                Ok(Some(E::new(
                    format!(
                        "loft_lazy_range({}, {}, {})",
                        as_ref(r)?,
                        as_int(lo)?,
                        as_int(hi)?
                    ),
                    K::Int,
                )))
            }
            _ => Ok(None),
        }
    }

    /// The natives over a whole store — its image, its verify, its report — or `None` for
    /// another.
    fn store_native(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        if let Some(e) = self.lazy_native(name, args, ind)? {
            return Ok(Some(e));
        }
        match (name, args) {
            ("n_store_persist_copy", [r, path]) => {
                // The image's sidecar is the store's root type's: the value's static type
                // names it, and the runtime writes nothing for a store minted otherwise.
                let tp = self
                    .store_value_type(r)
                    .and_then(|t| self.cx.root_type(&t, self.cx.data.def(self.d).source()))
                    .ok_or("a persisted value whose store type is not known here")?;
                // The image is a deep copy of the store's root: a type the copy cannot walk
                // is refused here, as every copy refuses it.
                self.cx.copy_type(tp)?;
                self.cx.name_types();
                let sc = self.cx.sidecar_of(tp);
                let (r, path) = self.two(name, &[r.clone(), path.clone()], ind)?;
                Ok(Some(E::new(
                    format!(
                        "(uint8_t) loft_image_write(loft_types, loft_ntypes, {}, {}, {sc})",
                        as_ref(r)?,
                        as_view(path)?
                    ),
                    K::Bool,
                )))
            }
            ("n_store_load" | "n_store_load_untrusted", [r, path]) => {
                // The gate reads the layout of the store's root type, the value's static type.
                let tp = self
                    .store_value_type(r)
                    .and_then(|t| self.cx.root_type(&t, self.cx.data.def(self.d).source()))
                    .ok_or("a loaded value whose store type is not known here")?;
                // A store holding references is re-pointed at its slot by a walk of its type.
                if self.cx.holds_stored_refs(tp) {
                    self.cx.copy_type(tp)?;
                    self.cx.name_types();
                }
                let sc = self.cx.sidecar_of(tp);
                let (r, path) = self.two(name, &[r.clone(), path.clone()], ind)?;
                Ok(Some(E::new(
                    format!(
                        "(uint8_t) loft_image_load({}, {}, {sc}, {})",
                        as_ref(r)?,
                        as_view(path)?,
                        u8::from(name == "n_store_load_untrusted")
                    ),
                    K::Bool,
                )))
            }
            ("n_store_load_url" | "n_store_load_url_trusted", [r, url, rest @ ..]) => {
                self.load_url(name, r, url, rest, ind).map(Some)
            }
            ("n_store_verify", [r]) => {
                // The walk reads the store's own type, so every type the value reaches is
                // described.
                let tp = self
                    .store_value_type(r)
                    .and_then(|t| self.cx.root_type(&t, self.cx.data.def(self.d).source()))
                    .ok_or("a verified value whose store type is not known here")?;
                self.cx.name_types();
                self.cx.describe_type(tp);
                let r = self.expr(r, ind)?;
                Ok(Some(E::new(
                    format!("loft_store_verify(loft_types, loft_ntypes, {})", as_ref(r)?),
                    K::Bool,
                )))
            }
            (
                "n_store_load_key"
                | "n_store_load_key_text"
                | "n_store_load_keys"
                | "n_store_load_keys_text"
                | "n_store_load_prefix"
                | "n_store_load_box"
                | "n_store_load_range",
                [local, ..],
            ) => self.paged_load(name, local, &args[1..], ind).map(Some),
            ("n_store_memory", []) => {
                let t = self.temp(CType::Text);
                self.line(ind, &format!("loft_text_free(&{t});"));
                // The type-name table every unit writes (`Cx::type_name_table`), by type
                // number: the name `Stores::types` gives each, as the interpreter names them.
                let names = "loft_tnames, loft_ntnames";
                self.line(ind, &format!("{t} = loft_store_memory({names});"));
                Ok(Some(self.held(t, CType::Text)))
            }
            _ => Ok(None),
        }
    }

    /// A `sqlite:` source's plan for keyed collection type `tp` (`loft_sql_plan`): the equality
    /// query `sql_query::derive_select` derives from the layout — the same one the interpreter
    /// derives at the miss — its table, each column's field, and the collection's own insert.
    /// `"0"` where no query can be derived (a field that is not a column), which the miss reports.
    fn sql_plan(&mut self, tp: u16) -> Result<String, String> {
        use crate::database::Parts;
        use crate::database::sql_query::{Mapping, QueryShape, derive_select};
        let stores = self.cx.stores;
        if tp == u16::MAX || (tp as usize) >= stores.types.len() {
            return Ok("0".to_string());
        }
        let desc = stores.layout_descriptor(&[tp]);
        let Some(sql) = derive_select(&desc, tp, &QueryShape::Equality, &Mapping::default()) else {
            return Ok("0".to_string());
        };
        // The explicit forms (`lazy_query`, `lazy_range`): a filter is the select with the
        // caller's predicate after its WHERE; a range exists only for an ordered collection.
        let filter = derive_select(
            &desc,
            tp,
            &QueryShape::Filter(String::new()),
            &Mapping::default(),
        )
        .map(|s| s.text);
        let range = derive_select(&desc, tp, &QueryShape::Range, &Mapping::default());
        let Ok((kind, elem, size)) = self.keyed_of(tp) else {
            return Ok("0".to_string());
        };
        let kind = match kind {
            Keyed::Hash => "LOFT_KEYED_HASH",
            Keyed::Sorted => "LOFT_KEYED_SORTED",
            Keyed::Index => "LOFT_KEYED_INDEX",
            Keyed::Ordered => "LOFT_KEYED_ORDERED",
            Keyed::Spatial => "LOFT_KEYED_SPATIAL",
            Keyed::Trie => "LOFT_KEYED_TRIE",
            Keyed::Array => return Ok("0".to_string()),
        };
        let Parts::Struct(fields) = &stores.types[elem as usize].parts else {
            return Ok("0".to_string());
        };
        self.cx.copy_type(elem)?;
        let (keys, nk) = self.cx.keys_of(tp);
        let table = sql
            .text
            .split(" FROM ")
            .nth(1)
            .and_then(|t| t.split(' ').next())
            .unwrap_or("");
        let cols: Vec<String> = sql
            .columns
            .iter()
            .filter_map(|c| {
                let f = fields.get(c.field)?;
                Some(format!(
                    "{{ {}, {}, {}, {} }}",
                    super::c_string(&c.column),
                    f.position,
                    f.content,
                    super::c_string(&stores.types[f.content as usize].name)
                ))
            })
            .collect();
        let name = format!("loft_sqlplan{tp}");
        if self.cx.sql_plans.borrow_mut().insert(tp) {
            let _ = writeln!(
                self.cx.gen_decls.borrow_mut(),
                "static const loft_sql_col loft_sqlcols{tp}[] = {{ {} }};\n\
                 static const loft_sql_plan {name} = {{ {}, {}, {}, loft_types, loft_sqlcols{tp}, {}, {}, {elem}, {size}, {kind}, {keys}, {nk}, {}, {}, {} }};",
                if cols.is_empty() {
                    "{ 0, 0, 0, 0 }".to_string()
                } else {
                    cols.join(", ")
                },
                super::c_string(&sql.text),
                super::c_string(table),
                super::c_string(stores.element_type_name(tp)),
                cols.len(),
                sql.params.len(),
                filter.map_or_else(|| "0".to_string(), |f| super::c_string(&f)),
                range
                    .as_ref()
                    .map_or_else(|| "0".to_string(), |r| super::c_string(&r.text)),
                range.as_ref().map_or(0, |r| r.params.len())
            );
        }
        Ok(format!("&{name}"))
    }

    /// `store_bind_lazy(local, source)` (`Stores::bind_lazy`): a kind an image cannot page — any
    /// but a hash, a trie or a spatial — is refused at the bind (`unservable_kind`); else the
    /// binding carries a fetch for one key written here from the collection's paged plan, the
    /// loader a lookup of that kind takes (`fetch_from_file`).
    fn bind_lazy(&mut self, local: &Value, source: &Value, ind: usize) -> Result<E, String> {
        use crate::database::Parts;
        let root = self
            .store_value_type(local)
            .and_then(|t| self.cx.root_type(&t, self.cx.data.def(self.d).source()))
            .filter(|tp| (*tp as usize) < self.cx.stores.types.len())
            .ok_or("a lazily bound value whose store type is not known here")?;
        let stores = self.cx.stores;
        let coll = stores.collection_type_of_store(root);
        let parts = stores.types.get(coll as usize).map(|t| &t.parts);
        // The element a miss is fetched for is the BOUND collection's own (the interpreter
        // keys its driver on the lookup's collection type): a field of a struct holding several
        // keyed collections names its own, where the store's root names no one collection.
        let field_type = match local.unspan() {
            // A field read names the field's own type as its third operand.
            Value::Call(d, a) if a.len() == 3 && self.cx.data.def(*d).name() == "OpGetField" => {
                match a[2].unspan() {
                    Value::Int(tp) => u16::try_from(*tp).ok(),
                    _ => None,
                }
            }
            _ => self
                .value_type(local)
                .and_then(|t| self.cx.root_type(&t, self.cx.data.def(self.d).source())),
        };
        let own = field_type
            .filter(|tp| (*tp as usize) < stores.types.len())
            .map(|tp| stores.collection_type_of_store(tp))
            .filter(|c| *c != u16::MAX)
            .unwrap_or(coll);
        let element = super::c_string(if own == u16::MAX {
            ""
        } else {
            stores.element_type_name(own)
        });
        let (unservable, fetch) = match parts {
            Some(Parts::Hash(..) | Parts::Trie(..) | Parts::Radix(..)) => {
                let radix = matches!(parts, Some(Parts::Radix(..)));
                let name = match parts {
                    Some(Parts::Radix(..)) => "n_store_load_box",
                    Some(Parts::Trie(..)) => "n_store_load_key_text",
                    _ => "n_store_load_key",
                };
                let plan = self.paged_plan(name, local)?;
                let k = self.cx.lazy_fetchers.get();
                self.cx.lazy_fetchers.set(k + 1);
                let PagedPlan {
                    sc,
                    refusal,
                    content,
                    keys,
                    nk,
                    kind,
                    coll_name,
                } = plan;
                // A point is the degenerate box, and it loads every record at that point.
                let load = if radix {
                    format!(
                        "n > 0 ? loft_pg_load_box_kv(loft_types, coll, path, {sc}, {refusal}, {content}, {keys}, {nk}, key, n, key, n, -1, {coll_name}) : 0"
                    )
                } else {
                    format!(
                        "n == 1 ? loft_pg_load_keys(loft_types, coll, path, {sc}, {refusal}, {content}, {keys}, {nk}, {kind}, key, 1) : 0"
                    )
                };
                let f = format!("loft_lazy_fetch{k}");
                let sig = format!(
                    "static int64_t {f}(loft_ref coll, loft_str path, const loft_kv *key, int n)"
                );
                let _ = writeln!(self.cx.gen_decls.borrow_mut(), "{sig};");
                let _ = writeln!(
                    self.cx.gen_bodies.borrow_mut(),
                    "\n{sig} {{\n    return {load};\n}}"
                );
                ("0".to_string(), f)
            }
            _ => (
                super::c_string(&format!(
                    "a lazily-bound `.store` image is read a page at a time, which only a `hash`, a \
                     `trie` or a `spatial` supports, and `{}` is none of them — read it whole with \
                     `store_load` (or `store_load_url_trusted` for a URL), which carries every kind",
                    stores.type_name(root)
                )),
                "0".to_string(),
            ),
        };
        let plan = self.sql_plan(own)?;
        let (r, source) = self.two("n_store_bind_lazy", &[local.clone(), source.clone()], ind)?;
        Ok(E::new(
            format!(
                "loft_lazy_bind_coll({}, {}, {fetch}, {element}, {unservable}, {plan})",
                as_ref(r)?,
                as_view(source)?
            ),
            K::Bool,
        ))
    }

    /// What a working-set loader can serve, asked of the Rust store once, when the program is
    /// written: the collection the local's store roots (`Stores::collection_type_of_store`;
    /// `load_range` reads the root itself), whether the loader reads that kind, and whether the
    /// entry type's every field can be copied out of the image (`Stores::is_copyable_entry`).
    /// A loader that cannot serve its collection still runs, and answers 0 after its gate and
    /// its open, saying why as the interpreter does.
    fn paged_plan(&mut self, name: &str, local: &Value) -> Result<PagedPlan, String> {
        use crate::database::Parts;
        let root = self
            .store_value_type(local)
            .and_then(|t| self.cx.root_type(&t, self.cx.data.def(self.d).source()))
            .filter(|tp| (*tp as usize) < self.cx.stores.types.len())
            .ok_or("a loaded value whose store type is not known here")?;
        let stores = self.cx.stores;
        let coll = if name == "n_store_load_range" {
            root
        } else {
            stores.collection_type_of_store(root)
        };
        let parts = stores.types.get(coll as usize).map(|t| &t.parts);
        let (want, served) = match name {
            "n_store_load_prefix" => ("a trie", matches!(parts, Some(Parts::Trie(..)))),
            "n_store_load_box" => ("a spatial index", matches!(parts, Some(Parts::Radix(..)))),
            "n_store_load_range" => (
                "a sorted collection",
                matches!(parts, Some(Parts::Sorted(..) | Parts::Ordered(..))),
            ),
            _ => (
                "a hash or a trie",
                matches!(parts, Some(Parts::Hash(..) | Parts::Trie(..))),
            ),
        };
        let content = match parts {
            Some(
                Parts::Hash(c, _)
                | Parts::Trie(c, _)
                | Parts::Radix(c, _)
                | Parts::Sorted(c, _)
                | Parts::Ordered(c, _),
            ) if served => Some(*c),
            _ => None,
        };
        let refusal = match content {
            None => Some(stores.wrong_collection_reason(root, want)),
            Some(c) if !stores.is_copyable_entry(c) => Some(stores.not_copyable_reason(c)),
            Some(_) => None,
        };
        self.cx.name_types();
        let sc = self.cx.sidecar_of(root);
        let (content, keys, nk) = match (content, &refusal) {
            (Some(c), None) => {
                self.cx.describe_type(c);
                let (keys, nk) = self.cx.keys_of(coll);
                (c, keys, nk)
            }
            _ => (0, "0".to_string(), 0),
        };
        let kind = match parts {
            Some(Parts::Trie(..)) => "LOFT_PG_TRIE",
            Some(Parts::Radix(..)) => "LOFT_PG_RADIX",
            _ => "LOFT_PG_HASH",
        };
        Ok(PagedPlan {
            sc,
            refusal: refusal.map_or_else(|| "0".to_string(), |r| super::c_string(&r)),
            content,
            keys,
            nk,
            kind,
            coll_name: super::c_string(stores.type_name(coll)),
        })
    }

    /// A working-set loader (`store_load_key` and its siblings, `loft_paged_load.h`): its
    /// collection read once here (`paged_plan`), its operands evaluated in order.
    fn paged_load(
        &mut self,
        name: &str,
        local: &Value,
        rest: &[Value],
        ind: usize,
    ) -> Result<E, String> {
        let plan = self.paged_plan(name, local)?;
        let PagedPlan {
            sc,
            refusal,
            content,
            keys,
            nk,
            kind,
            ..
        } = &plan;
        let mut all = vec![local.clone()];
        all.extend(rest.iter().cloned());
        let mut es = self.args(&all, ind)?.into_iter();
        let mut next = || es.next().ok_or(format!("{name} without its operands"));
        let local = as_ref(next()?)?;
        let path = as_view(next()?)?;
        let head = format!("loft_types, {local}, {path}, {sc}, {refusal}, {content}, {keys}, {nk}");
        Ok(match name {
            "n_store_load_key" => E::new(
                format!(
                    "(uint8_t) (loft_pg_load_keys({head}, {kind}, (loft_kv[]) {{ loft_kv_int({}) }}, 1) > 0)",
                    as_int(next()?)?
                ),
                K::Bool,
            ),
            "n_store_load_key_text" => E::new(
                format!(
                    "(uint8_t) (loft_pg_load_keys({head}, {kind}, (loft_kv[]) {{ loft_kv_text({}) }}, 1) > 0)",
                    as_view(next()?)?
                ),
                K::Bool,
            ),
            "n_store_load_keys" | "n_store_load_keys_text" => E::new(
                format!(
                    "loft_pg_load_key_list({head}, {kind}, {}, {})",
                    as_ref(next()?)?,
                    u8::from(name == "n_store_load_keys_text")
                ),
                K::Int,
            ),
            "n_store_load_prefix" => {
                let pre = as_view(next()?)?;
                E::new(
                    format!("loft_pg_load_prefix({head}, {pre}, {})", as_int(next()?)?),
                    K::Int,
                )
            }
            "n_store_load_box" => {
                let (from, till) = (as_ref(next()?)?, as_ref(next()?)?);
                E::new(
                    format!(
                        "loft_pg_load_box({head}, {from}, {till}, {}, {})",
                        as_int(next()?)?,
                        plan.coll_name
                    ),
                    K::Int,
                )
            }
            _ => {
                let lo = as_int(next()?)?;
                E::new(
                    format!(
                        "loft_pg_load_range(loft_types, {local}, {path}, {sc}, {refusal}, {content}, {keys}, {lo}, {})",
                        as_int(next()?)?
                    ),
                    K::Int,
                )
            }
        })
    }

    /// The keyed operators, or `None` for another.
    pub(super) fn keyed_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        if let Some(e) = self.group_operator(name, args, ind)? {
            return Ok(Some(e));
        }
        match (name, args) {
            ("OpGetRecord", [data, tp, _, vals @ ..]) => {
                self.get_record(data, tp, vals, ind).map(Some)
            }
            ("OpIterate", [data, on, arg, Value::Keys(keys), rest @ ..]) => {
                self.iterate(data, on, arg, keys, rest, ind).map(Some)
            }
            ("OpVectorRef" | "OpVectorRefNullable", [_, _]) => {
                let (v, i) = self.two(name, args, ind)?;
                Ok(Some(E::new(
                    format!("loft_array_get({}, {})", as_ref(v)?, as_int(i)?),
                    K::Ref,
                )))
            }
            ("OpStep", [Value::Var(state), data, on, arg]) => {
                let on = Self::literal(on, "walk kind")?;
                let arg = Self::literal(arg, "walk argument")?;
                let state = self.var(*state);
                let data = self.expr(data, ind)?;
                let data = as_ref(data)?;
                let reverse = i32::from(on & 64 != 0);
                Ok(Some(E::new(
                    match on & 63 {
                        1 => {
                            // `arg` is the index type: a step finds its record again by key.
                            let tp = u16::try_from(arg)
                                .map_err(|_| "a type number out of range".to_string())?;
                            let (keys, n) = self.cx.keys_of(tp);
                            format!("loft_ix_step(&{state}, {data}, {keys}, {n}, {reverse})")
                        }
                        3 => format!("loft_seq_step(&{state}, {data}, 0, {reverse})"),
                        2 => format!("loft_seq_step(&{state}, {data}, {arg}, {reverse})"),
                        4 => format!("loft_scratch_step(&{state}, {data}, {reverse})"),
                        k => return Err(format!("a walk of kind {k}")),
                    },
                    K::Ref,
                )))
            }
            ("OpFreeScratch", [x]) => {
                let x = self.expr(x, ind)?;
                Ok(Some(E::new(
                    format!("loft_free_scratch({})", as_ref(x)?),
                    K::Void,
                )))
            }
            ("OpLengthSpatial" | "OpLengthTrie", [r]) => {
                let r = self.expr(r, ind)?;
                Ok(Some(E::new(
                    format!("loft_rx_count({})", as_ref(r)?),
                    K::Int,
                )))
            }
            ("OpLengthHash", [h]) => {
                let h = self.expr(h, ind)?;
                Ok(Some(E::new(
                    format!("loft_hash_len({})", as_ref(h)?),
                    K::Int,
                )))
            }
            _ => self.keyed_write(name, args, ind),
        }
    }

    /// The keyed operators that write a collection, or `None` for another.
    fn keyed_write(&mut self, name: &str, args: &[Value], ind: usize) -> Result<Option<E>, String> {
        match (name, args) {
            // The record releases what it owns (`Stores::remove_claims`): a keyed collection
            // is emptied, a struct's every text and collection.
            ("OpClearKeyed", [dest, tp]) => {
                let tp = u16::try_from(Self::literal(tp, "type")?)
                    .map_err(|_| "a type number out of range".to_string())?;
                let dest = self.expr(dest, ind)?;
                if tp & crate::database::CLEAR_KEYED_VIEW != 0 {
                    // A view member releases its own storage; the member that holds the
                    // records frees them (`remove_claims_keyed`).
                    let tp = tp & !crate::database::CLEAR_KEYED_VIEW;
                    self.keyed_of(tp)?;
                    self.cx.name_types();
                    self.cx.describe_type(tp);
                    return Ok(Some(E::new(
                        format!(
                            "loft_remove_claims_mode(loft_types, {tp}, {}, 1)",
                            as_ref(dest)?
                        ),
                        K::Void,
                    )));
                }
                self.cx.copy_type(tp)?;
                Ok(Some(E::new(
                    format!("loft_remove_claims(loft_types, {tp}, {})", as_ref(dest)?),
                    K::Void,
                )))
            }
            ("OpSizeSpatial" | "OpSizeTrie", [r]) => {
                let r = self.expr(r, ind)?;
                Ok(Some(E::new(
                    format!("loft_rx_table_bytes({})", as_ref(r)?),
                    K::Int,
                )))
            }
            ("OpSizeIndex", [r]) => {
                let r = self.expr(r, ind)?;
                Ok(Some(E::new(
                    format!("loft_bt_table_bytes({})", as_ref(r)?),
                    K::Int,
                )))
            }
            ("OpLengthIndex", [r]) => {
                let r = self.expr(r, ind)?;
                Ok(Some(E::new(
                    format!("(int64_t) loft_bt_len({})", as_ref(r)?),
                    K::Int,
                )))
            }
            ("OpReplaceKeyed", [src, dest, tp]) => {
                let raw = u16::try_from(Self::literal(tp, "type")?)
                    .map_err(|_| "a type number out of range".to_string())?;
                let tp = raw & 0x7FFF;
                self.keyed_of(tp)?;
                self.cx.copy_type(tp)?;
                let (src, dest) = self.two(name, &[src.clone(), dest.clone()], ind)?;
                Ok(Some(E::new(
                    format!(
                        "loft_replace_keyed(loft_types, {tp}, {}, {}, {})",
                        as_ref(src)?,
                        as_ref(dest)?,
                        i32::from(raw & 0x8000 != 0)
                    ),
                    K::Void,
                )))
            }
            ("OpFillKeyed", [parent, src, tp, ptp, fld]) => {
                self.fill_keyed(parent, src, tp, ptp, fld, ind).map(Some)
            }
            ("OpSetKeyed", [coll, value, tp, _, vals @ ..]) => {
                self.set_keyed(coll, value, tp, vals, ind).map(Some)
            }
            ("OpRemove", [Value::Var(state), data, on, tp]) => {
                self.walk_remove(*state, data, on, tp, ind).map(Some)
            }
            ("OpHashRemove", [data, rec, tp]) => self.keyed_remove(data, rec, tp, ind).map(Some),
            ("OpReserveHash", [h, count, tp]) => self.reserve_hash(h, count, tp, ind).map(Some),
            _ => Ok(None),
        }
    }

    /// `OpRemove(state, data, on, tp)` (`e#remove` in a walk, State::remove): the element the
    /// walk visited last leaves the collection, releasing what it owns (`remove_vector_at`,
    /// `remove_during_tree_iteration`), and the walk goes on with the one after it.  `tp` is the
    /// element type, but for an index walk (on 1) the index type.
    fn walk_remove(
        &mut self,
        state: u16,
        data: &Value,
        on: &Value,
        tp: &Value,
        ind: usize,
    ) -> Result<E, String> {
        let on = Self::literal(on, "walk kind")?;
        let tp = u16::try_from(Self::literal(tp, "type")?)
            .map_err(|_| "a type number out of range".to_string())?;
        let reverse = i32::from(on & 64 != 0);
        let call = match on & 63 {
            // A vector walk: its state is the index; forward, the successor moves into the
            // removed slot, so the index steps back to visit it.
            0 if !self.cx.stores.is_linked(tp) => "loft_vec_walk_remove",
            0 => "loft_array_walk_remove",
            1 => {
                let (_, elem, _) = self.keyed_of(tp)?;
                let (keys, n) = self.cx.keys_of(tp);
                self.cx.name_types();
                self.cx.describe_type(elem);
                let state = self.var(state);
                let data = self.expr(data, ind)?;
                return Ok(E::new(
                    format!(
                        "loft_ix_walk_remove(loft_types, {elem}, &{state}, {}, {keys}, {n}, {reverse})",
                        as_ref(data)?
                    ),
                    K::Void,
                ));
            }
            2 => "loft_sorted_walk_remove",
            3 => "loft_ordered_walk_remove",
            k => return Err(format!("a removal in a walk of kind {k}")),
        };
        self.cx.name_types();
        self.cx.describe_type(tp);
        let state = self.var(state);
        let data = self.expr(data, ind)?;
        Ok(E::new(
            format!(
                "{call}(loft_types, {tp}, &{state}, {}, {reverse})",
                as_ref(data)?
            ),
            K::Void,
        ))
    }

    /// `OpReserveHash(h, count, tp)`: buckets for `count` entries (`Stores::reserve_hash`).
    fn reserve_hash(
        &mut self,
        h: &Value,
        count: &Value,
        tp: &Value,
        ind: usize,
    ) -> Result<E, String> {
        let tp = u16::try_from(Self::literal(tp, "type")?)
            .map_err(|_| "a type number out of range".to_string())?;
        self.keyed_of(tp)?;
        self.cx.name_types();
        self.cx.describe_type(tp);
        let (h, count) = self.two("OpReserveHash", &[h.clone(), count.clone()], ind)?;
        Ok(E::new(
            format!(
                "loft_hash_reserve(loft_types, {tp}, {}, {})",
                as_ref(h)?,
                as_int(count)?
            ),
            K::Void,
        ))
    }

    /// `OpFillKeyed(parent, src, tp, ptp, fld)`: `coll += vector`, each element filed into the
    /// keyed collection at the parent's field (the parent itself for field `u16::MAX`) as a deep
    /// copy under its own key; in a member of a linked group the copy enters every member
    /// (`insert_keyed_copy`'s `record_finish`).
    fn fill_keyed(
        &mut self,
        parent: &Value,
        src: &Value,
        tp: &Value,
        ptp: &Value,
        fld: &Value,
        ind: usize,
    ) -> Result<E, String> {
        let raw = u16::try_from(Self::literal(tp, "type")?)
            .map_err(|_| "a type number out of range".to_string())?;
        let tp = raw & 0x7FFF;
        let (kind, elem, _) = self.keyed_of(tp)?;
        let (place, _, coll) = self.record_collection(parent, ptp, fld, ind)?;
        let group = match coll {
            Coll::Keyed(_) => "0".to_string(),
            Coll::Group(parent, members) => {
                let (rows, n) = self.cx.member_rows(&members)?;
                let (_, held, nullable) = self.cx.group_release(&members)?;
                format!("&(const loft_group){{ {parent}, {rows}, {n}, {held}, {nullable} }}")
            }
            Coll::Vector => return Err("a fill of a vector".into()),
        };
        self.cx.copy_type(elem)?;
        let src = self.expr(src, ind)?;
        let src = as_ref(src)?;
        let (keys, nk) = self.cx.keys_of(tp);
        let kind = match kind {
            Keyed::Hash => "LOFT_KEYED_HASH",
            Keyed::Sorted => "LOFT_KEYED_SORTED",
            Keyed::Index => "LOFT_KEYED_INDEX",
            Keyed::Ordered => "LOFT_KEYED_ORDERED",
            Keyed::Array => return Err("a fill of an array".into()),
            Keyed::Spatial => "LOFT_KEYED_SPATIAL",
            Keyed::Trie => "LOFT_KEYED_TRIE",
        };
        let linked = i32::from(self.cx.stores.is_linked(elem));
        Ok(E::new(
            format!(
                "loft_fill_keyed(loft_types, {elem}, {place}, {src}, {kind}, {keys}, {nk}, {linked}, {}, {group})",
                i32::from(raw & 0x8000 != 0)
            ),
            K::Void,
        ))
    }

    /// `OpSetKeyed(coll, value, tp, n, keys…)`: `coll[keys…] = value`, a deep copy of the value
    /// filed under the subscript (`@FR-Col-Assign`).
    fn set_keyed(
        &mut self,
        coll: &Value,
        value: &Value,
        tp: &Value,
        vals: &[Value],
        ind: usize,
    ) -> Result<E, String> {
        let raw = u16::try_from(Self::literal(tp, "type")?)
            .map_err(|_| "a type number out of range".to_string())?;
        let tp = raw & 0x7FFF;
        let (kind, elem, _) = self.keyed_of(tp)?;
        self.cx.copy_type(elem)?;
        let types: Vec<i8> = self.cx.stores.types[tp as usize]
            .keys
            .iter()
            .map(|k| k.type_nr)
            .collect();
        if vals.len() > types.len() || types.len() > 8 {
            return Err("a keyed write with more values than keys".into());
        }
        let mut all = vec![coll.clone(), value.clone()];
        all.extend(vals.iter().cloned());
        let mut es = self.args(&all, ind)?.into_iter();
        let coll = as_ref(es.next().ok_or("a keyed write without its collection")?)?;
        let value = as_ref(es.next().ok_or("a keyed write without its value")?)?;
        let values = es
            .zip(&types)
            .map(|(e, t)| key_value(e, *t))
            .collect::<Result<Vec<_>, _>>()?;
        let n = values.len();
        let kv = value_array(&values);
        let (keys, nk) = self.cx.keys_of(tp);
        let kind = match kind {
            Keyed::Hash => "LOFT_KEYED_HASH",
            Keyed::Sorted => "LOFT_KEYED_SORTED",
            Keyed::Index => "LOFT_KEYED_INDEX",
            Keyed::Ordered => "LOFT_KEYED_ORDERED",
            Keyed::Array => return Err("a keyed write into an array".into()),
            Keyed::Spatial => "LOFT_KEYED_SPATIAL",
            Keyed::Trie => "LOFT_KEYED_TRIE",
        };
        Ok(E::new(
            format!(
                "loft_set_keyed(loft_types, {elem}, {coll}, {value}, {kind}, {keys}, {nk}, {kv}, {n}, {}, 0)",
                i32::from(raw & 0x8000 != 0)
            ),
            K::Void,
        ))
    }

    /// `OpGetRecord(data, tp, n, keys…)`: the element under the key, or null.
    fn get_record(
        &mut self,
        data: &Value,
        tp: &Value,
        vals: &[Value],
        ind: usize,
    ) -> Result<E, String> {
        let tp = u16::try_from(Self::literal(tp, "type")?)
            .map_err(|_| "a type number out of range".to_string())?;
        let (kind, _, size) = self.keyed_of(tp)?;
        let types: Vec<i8> = self.cx.stores.types[tp as usize]
            .keys
            .iter()
            .map(|k| k.type_nr)
            .collect();
        if vals.len() > types.len() {
            return Err("a lookup with more values than keys".into());
        }
        let mut all = vec![data.clone()];
        all.extend(vals.iter().cloned());
        let mut es = self.args(&all, ind)?.into_iter();
        let data = as_ref(es.next().ok_or("a lookup without its collection")?)?;
        let values = es
            .zip(&types)
            .map(|(e, t)| key_value(e, *t))
            .collect::<Result<Vec<_>, _>>()?;
        let n = values.len();
        let kv = value_array(&values);
        let (keys, nk) = self.cx.keys_of(tp);
        // `@FR-R-TypedKeyed`: a hash with ONE 8-byte integer key, looked up by that integer,
        // takes the typed entry — the same digest, home bucket, walk and equality, in place.
        // Two to four such keys take the compound entry (the same clause's compound form).
        let ints: Option<Vec<&str>> = (kind == Keyed::Hash
            && n == nk
            && (1..=4).contains(&n)
            && types.iter().take(n).all(|t| matches!(t, 1 | 2)))
        .then(|| {
            values
                .iter()
                .map(|v| {
                    v.strip_prefix("loft_kv_int(")
                        .and_then(|v| v.strip_suffix(')'))
                })
                .collect()
        })
        .flatten();
        // One text key takes the text entry (the same clause's text form).
        let text_find = (kind == Keyed::Hash && n == 1 && nk == 1 && types[0] == 6)
            .then(|| {
                values[0]
                    .strip_prefix("loft_kv_text(")
                    .and_then(|v| v.strip_suffix(')'))
                    .map(|key| format!("loft_h_find_text({data}, {keys}, {key})"))
            })
            .flatten();
        let typed_find = ints
            .map(|ks| match ks.as_slice() {
                [key] => format!("loft_h_find_i64({data}, {keys}, {key})"),
                ks => format!(
                    "loft_h_find_ints({data}, {keys}, {n}, (int64_t[]){{ {} }})",
                    ks.join(", ")
                ),
            })
            .or(text_find);
        let find = match kind {
            Keyed::Hash if typed_find.is_some() => typed_find.clone().unwrap_or_default(),
            Keyed::Hash => format!("loft_h_find({data}, {keys}, {kv}, {n})"),
            Keyed::Sorted => format!("loft_seq_find({data}, {size}, {keys}, {nk}, {kv}, {n})"),
            Keyed::Index => format!("loft_ix_find({data}, {keys}, {nk}, {kv}, {n})"),
            Keyed::Ordered => format!("loft_seq_find({data}, 0, {keys}, {nk}, {kv}, {n})"),
            Keyed::Spatial | Keyed::Trie => format!(
                "loft_rx_find({data}, {keys}, {nk}, {}, {kv}, {n})",
                u8::from(kind == Keyed::Trie)
            ),
            Keyed::Array => return Err("a keyed lookup in an array".into()),
        };
        // `@PLN129` — a miss on a collection bound to a source fetches the key and looks again
        // (`State::get_record`); only a unit that binds one asks.
        if self.cx.lazy.get() {
            let t = self.temp(CType::Ref);
            self.line(ind, &format!("{t} = {find};"));
            self.line(
                ind,
                &format!(
                    "if ({t}.rec == 0 && loft_lazy_bound({data}) && loft_lazy_miss({data}, {kv}, {n})) {t} = {find};"
                ),
            );
            return Ok(E::new(t, K::Ref));
        }
        Ok(E::new(find, K::Ref))
    }

    /// `OpIterate(data, on, arg, keys, n_from, from…, n_till, till…)`: the walk's first state.
    fn iterate(
        &mut self,
        data: &Value,
        on: &Value,
        arg: &Value,
        keys: &[Key],
        rest: &[Value],
        ind: usize,
    ) -> Result<E, String> {
        let on = Self::literal(on, "walk kind")?;
        let arg = Self::literal(arg, "walk argument")?;
        let nfrom = usize::try_from(Self::literal(
            rest.first().ok_or("a walk without its bounds")?,
            "bound count",
        )?)
        .map_err(|_| "a negative bound count")?;
        let till_at = 1 + nfrom;
        let ntill = usize::try_from(Self::literal(
            rest.get(till_at).ok_or("a walk without its upper bound")?,
            "bound count",
        )?)
        .map_err(|_| "a negative bound count")?;
        let from = rest
            .get(1..till_at)
            .ok_or("a walk without its lower bound")?;
        let till = rest
            .get(till_at + 1..till_at + 1 + ntill)
            .ok_or("a walk without its upper bound")?;
        if from.len() > keys.len() || till.len() > keys.len() {
            return Err("a walk bound with more values than keys".into());
        }
        let mut all = vec![data.clone()];
        all.extend(from.iter().cloned());
        all.extend(till.iter().cloned());
        let mut es = self.args(&all, ind)?.into_iter();
        let data = as_ref(es.next().ok_or("a walk without its collection")?)?;
        let mut values = Vec::new();
        for (i, e) in es.enumerate() {
            let k = if i < nfrom { i } else { i - nfrom };
            values.push(key_value(e, keys[k].type_nr)?);
        }
        let till_values = values.split_off(nfrom);
        let ex = i32::from(on & 128 == 0);
        let (nk, ka) = (keys.len(), key_array(keys));
        let (f, t) = (value_array(&values), value_array(&till_values));
        let reverse = i32::from(on & 64 != 0);
        Ok(E::new(
            match on & 63 {
                1 => format!(
                    "loft_ix_iterate({data}, {ex}, {reverse}, {ka}, {nk}, {f}, {nfrom}, {t}, {ntill})"
                ),
                3 => format!(
                    "loft_seq_iterate({data}, 0, {ex}, {ka}, {nk}, {f}, {nfrom}, {t}, {ntill})"
                ),
                2 => format!(
                    "loft_seq_iterate({data}, {arg}, {ex}, {ka}, {nk}, {f}, {nfrom}, {t}, {ntill})"
                ),
                4 if nfrom == 0 && ntill == 0 => format!("loft_scratch_iterate({data})"),
                k => return Err(format!("a walk of kind {k}")),
            },
            K::Int,
        ))
    }

    /// `OpHashRemove(data, rec, tp)`: the element leaves its keyed collection.
    fn keyed_remove(
        &mut self,
        data: &Value,
        rec: &Value,
        tp: &Value,
        ind: usize,
    ) -> Result<E, String> {
        let raw = u16::try_from(Self::literal(tp, "type")?)
            .map_err(|_| "a type number out of range".to_string())?;
        // A view member only unlinks (`CLEAR_KEYED_VIEW`); the owner releases (`remove_owned`).
        let view = raw & crate::database::CLEAR_KEYED_VIEW != 0;
        let raw = raw & !crate::database::CLEAR_KEYED_VIEW;
        let (kind, elem, size) = self.keyed_of(raw)?;
        let (keys, nk) = self.cx.keys_of(raw);
        let (data, rec) = self.two("OpHashRemove", &[data.clone(), rec.clone()], ind)?;
        let (data, rec) = (as_ref(data)?, as_ref(rec)?);
        // What the element owns is released (`remove_owned`): its type is in the table.
        self.cx.name_types();
        self.cx.describe_type(elem);
        let view = u8::from(view);
        Ok(E::new(
            match kind {
                Keyed::Hash => format!(
                    "loft_hash_remove(loft_types, {elem}, {data}, {rec}, {keys}, {nk}, {view})"
                ),
                Keyed::Sorted => {
                    format!("loft_sorted_remove_owned(loft_types, {elem}, {data}, {rec}, {size})")
                }
                Keyed::Index => format!(
                    "loft_ix_remove(loft_types, {elem}, {data}, {rec}, {keys}, {nk}, {view})"
                ),
                Keyed::Ordered | Keyed::Array => format!(
                    "loft_ordered_remove(loft_types, {elem}, {data}, {rec}, {}, {view})",
                    u8::from(kind == Keyed::Array)
                ),
                Keyed::Spatial | Keyed::Trie => format!(
                    "loft_rx_remove_owned(loft_types, {elem}, {data}, {rec}, {keys}, {nk}, {}, {view})",
                    u8::from(kind == Keyed::Trie)
                ),
            },
            K::Void,
        ))
    }
}

/// What `paged_plan` found: the sidecar the gate reads, the refusal (a C string, or `0`), the
/// entry type, the key parts and the collection kind the loader is handed, and the
/// collection's name for a refusal the runtime words.
struct PagedPlan {
    sc: String,
    refusal: String,
    content: u16,
    keys: String,
    nk: usize,
    kind: &'static str,
    coll_name: String,
}

/// The members of a linked group (`@FR-Col-Group`), as the C writes them: a function's group
/// operators and a parse's record finish read the same rows.
impl crate::c::Cx<'_> {
    /// One member of a linked group as a `loft_member` row: its byte position, kind and keys.
    pub(crate) fn member_row(&self, pos: u16, tp: u16) -> Result<String, String> {
        let (kind, _) = self
            .keyed(tp)
            .ok_or("a linked group member that is not a collection")?;
        let (keys, n) = self.keys_of(tp);
        let kind = match kind {
            Keyed::Array => "LOFT_M_ARRAY",
            Keyed::Hash => "LOFT_M_HASH",
            Keyed::Index => "LOFT_M_INDEX",
            Keyed::Ordered => "LOFT_M_ORDERED",
            Keyed::Sorted => return Err("a sorted member of a linked group".into()),
            Keyed::Spatial => "LOFT_M_SPATIAL",
            Keyed::Trie => "LOFT_M_TRIE",
        };
        Ok(format!("{{ {pos}, {kind}, {keys}, {n} }}"))
    }

    /// The members of `members` as a `loft_member` array literal and its length.
    pub(crate) fn member_rows(&self, members: &[(u16, u16)]) -> Result<(String, usize), String> {
        let rows = members
            .iter()
            .map(|&(pos, tp)| self.member_row(pos, tp))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((
            format!("(const loft_member[]){{ {} }}", rows.join(", ")),
            rows.len(),
        ))
    }

    /// The record type of a group's members and whether a vector member holds its records
    /// (`settle_displaced`'s `held_by_vector`): a displaced record nothing holds is released,
    /// through the type table — and whether its records are `vector<S?>` elements, whose
    /// null ones no keyed member files.
    pub(crate) fn group_release(&self, members: &[(u16, u16)]) -> Result<(u16, i32, i32), String> {
        let mut held = 0;
        let mut elem = u16::MAX;
        let mut nullable = 0;
        for &(_, tp) in members {
            let (kind, e) = self
                .keyed(tp)
                .ok_or("a linked group member that is not a collection")?;
            elem = e;
            held |= i32::from(kind == Keyed::Array);
            nullable |= i32::from(self.stores.nullable_some_variant(e).is_some());
        }
        self.name_types();
        self.describe_type(elem);
        Ok((elem, held, nullable))
    }

    /// The members of field `fld` of struct `ptp`'s linked group beside the field itself, as
    /// (position, type).
    pub(crate) fn group_siblings(&self, ptp: u16, fld: u16) -> Vec<(u16, u16)> {
        let stores = self.stores;
        let (Parts::Struct(fields) | Parts::EnumValue(_, fields)) =
            &stores.types[ptp as usize].parts
        else {
            return Vec::new();
        };
        fields
            .get(fld as usize)
            .map(|f| {
                f.other_indexes()
                    .iter()
                    .copied()
                    .filter(|m| *m != u16::MAX)
                    .map(|m| (stores.field_position(ptp, m), stores.field_type(ptp, m)))
                    .collect()
            })
            .unwrap_or_default()
    }
}
