// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! The keyed collections — hash, sorted, index — as C: filing a new record, a lookup by key,
//! a walk over a key range, a removal, a clear and a length.  The runtime half is
//! `loft_keyed.h`; a key part's value is a `loft_kv`, its description a `loft_key`.

use super::{
    E, FnWriter, K, as_bool, as_char, as_enum, as_float, as_int, as_ref, as_single, as_view,
};
use crate::c::{CType, Keyed};
use crate::data::Value;
use crate::keys::Key;

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
    /// its end; a hash or an index gets a record of its own.
    pub(super) fn keyed_new(&mut self, c: &str, tp: u16, ind: usize) -> Result<E, String> {
        let (kind, elem, size) = self.keyed_of(tp)?;
        let (image, n) = self.cx.default_image(elem);
        let t = self.temp(CType::Ref);
        let make = if kind == Keyed::Sorted {
            "loft_sorted_new"
        } else {
            "loft_entry_new"
        };
        self.line(ind, &format!("{t} = {make}({c}, {size});"));
        self.line(ind, &format!("loft_fill({t}, {image}, {n});"));
        Ok(E::new(t, K::Ref))
    }

    /// `OpFinishRecord` in a keyed collection: the record goes to its key's place
    /// (`@FR-Col-Insert`: an equal key's element is replaced).
    pub(super) fn keyed_finish(&mut self, c: &str, rec: &str, tp: u16) -> Result<E, String> {
        let (kind, _, size) = self.keyed_of(tp)?;
        let (keys, n) = self.cx.keys_of(tp);
        Ok(E::new(
            match kind {
                Keyed::Sorted => format!("loft_sorted_finish({c}, {size}, {keys}, {n})"),
                Keyed::Hash => format!("(void) loft_hash_finish({c}, {rec}, {keys}, {n})"),
                Keyed::Index | Keyed::Ordered => {
                    format!("(void) loft_index_finish({c}, {rec}, {keys}, {n})")
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
        let mut rows = Vec::new();
        for &(pos, tp) in members {
            let (kind, _, _) = self.keyed_of(tp)?;
            let (keys, n) = self.cx.keys_of(tp);
            let kind = match kind {
                Keyed::Array => "LOFT_M_ARRAY",
                Keyed::Hash => "LOFT_M_HASH",
                Keyed::Index | Keyed::Ordered => "LOFT_M_INDEX",
                Keyed::Sorted => return Err("a sorted member of a linked group".into()),
            };
            rows.push(format!("{{ {pos}, {kind}, {keys}, {n} }}"));
        }
        Ok(E::new(
            format!(
                "loft_group_finish({parent}, {rec}, (const loft_member[]){{ {} }}, {})",
                rows.join(", "),
                rows.len()
            ),
            K::Void,
        ))
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
            ("t_4hash_len", [h]) => {
                let h = self.expr(h, ind)?;
                Ok(Some(E::new(
                    format!("loft_hash_len({})", as_ref(h)?),
                    K::Int,
                )))
            }
            _ => Ok(None),
        }
    }

    /// The keyed operators, or `None` for another.
    pub(super) fn keyed_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
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
                        1 | 3 => format!("loft_seq_step(&{state}, {data}, 0, {reverse})"),
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
                    // A view member lets its records go; the member that holds them frees.
                    self.keyed_of(tp & !crate::database::CLEAR_KEYED_VIEW)?;
                    return Ok(Some(E::new(
                        format!("loft_keyed_clear({})", as_ref(dest)?),
                        K::Void,
                    )));
                }
                self.cx.copy_type(tp)?;
                Ok(Some(E::new(
                    format!("loft_remove_claims(loft_types, {tp}, {})", as_ref(dest)?),
                    K::Void,
                )))
            }
            ("OpLengthIndex", [r]) => {
                let r = self.expr(r, ind)?;
                Ok(Some(E::new(
                    format!("loft_vec_len({})", as_ref(r)?),
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
            ("OpSetKeyed", [coll, value, tp, _, vals @ ..]) => {
                self.set_keyed(coll, value, tp, vals, ind).map(Some)
            }
            ("OpRemove", [Value::Var(state), data, on, tp]) => {
                let on = Self::literal(on, "walk kind")?;
                let tp = u16::try_from(Self::literal(tp, "type")?)
                    .map_err(|_| "a type number out of range".to_string())?;
                let size = match on & 63 {
                    1 | 3 => 0,
                    2 => self.cx.stores.size(tp),
                    // A vector walk: its state is the index; forward, the successor moves into
                    // the removed slot, so the index steps back to visit it (State::remove).
                    0 if !self.cx.stores.is_linked(tp) => {
                        let size = self.cx.stores.size(tp);
                        let state = self.var(*state);
                        let data = self.expr(data, ind)?;
                        let back = if on & 64 != 0 { "" } else { " - 1" };
                        return Ok(Some(E::new(
                            format!(
                                "{{ (void) loft_vec_remove({}, {size}, {state}); {state} = {state}{back}; }}",
                                as_ref(data)?
                            ),
                            K::Void,
                        )));
                    }
                    k => return Err(format!("a removal in a walk of kind {k}")),
                };
                let state = self.var(*state);
                let data = self.expr(data, ind)?;
                Ok(Some(E::new(
                    format!(
                        "loft_seq_remove(&{state}, {}, {size}, {})",
                        as_ref(data)?,
                        i32::from(on & 64 != 0)
                    ),
                    K::Void,
                )))
            }
            ("OpHashRemove", [data, rec, tp]) => self.keyed_remove(data, rec, tp, ind).map(Some),
            _ => Ok(None),
        }
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
            Keyed::Index | Keyed::Ordered => "LOFT_KEYED_INDEX",
            Keyed::Array => return Err("a keyed write into an array".into()),
        };
        Ok(E::new(
            format!(
                "loft_set_keyed(loft_types, {elem}, {coll}, {value}, {kind}, {keys}, {nk}, {kv}, {n}, {})",
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
        Ok(E::new(
            match kind {
                Keyed::Hash => format!("loft_hash_find({data}, {keys}, {nk}, {kv}, {n})"),
                Keyed::Sorted => format!("loft_seq_find({data}, {size}, {keys}, {nk}, {kv}, {n})"),
                Keyed::Index | Keyed::Ordered => {
                    format!("loft_seq_find({data}, 0, {keys}, {nk}, {kv}, {n})")
                }
                Keyed::Array => return Err("a keyed lookup in an array".into()),
            },
            K::Ref,
        ))
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
        Ok(E::new(
            match on & 63 {
                1 | 3 => format!(
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
        // A view member only unlinks (`CLEAR_KEYED_VIEW`); with no record freed in C, that is
        // the removal itself.
        let raw = raw & !crate::database::CLEAR_KEYED_VIEW;
        let (kind, _, size) = self.keyed_of(raw)?;
        let (keys, nk) = self.cx.keys_of(raw);
        let (data, rec) = self.two("OpHashRemove", &[data.clone(), rec.clone()], ind)?;
        let (data, rec) = (as_ref(data)?, as_ref(rec)?);
        Ok(E::new(
            match kind {
                Keyed::Hash => format!("loft_hash_remove({data}, {rec}, {keys}, {nk})"),
                Keyed::Sorted => format!("loft_sorted_remove({data}, {rec}, {size})"),
                Keyed::Index | Keyed::Ordered | Keyed::Array => {
                    format!("loft_index_remove({data}, {rec})")
                }
            },
            K::Void,
        ))
    }
}
