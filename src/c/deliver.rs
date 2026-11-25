// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C
//! `deliver(tag, value)` in C — the loopback host of `Stores::deliver_loopback`: the value's
//! bytes as `read_via_descriptor` reads them through its layout, printed as hex.  The emitter
//! writes one `loft_dlv{tp}` per type the value reaches; a store-internal kind (a keyed
//! collection, a stored reference, a child record) answers its refusal when it is reached, as
//! the descriptor walk does.
//!
//! On a page (`LOFT_BROWSER`) `deliver` and `expose` hand the value to the page instead, as
//! `Stores::deliver_browser` does: the `loft_host_deliver` / `loft_host_expose` import gets the
//! store's base address in linear memory, the value's `(rec, pos)` and its layout descriptor as
//! JSON, which `doc/loft-deliver.js` reads the value through.  The descriptor is the unit's
//! (`loft_dlvd{tp}`, written here at compile time); only its `flat` map is the run's: every keyed
//! collection the value holds is materialised to the element array its walk yields
//! (`loft_dlk{tp}`, the twin of `Stores::collect_keyed`), and the map names each one's record.
use super::{Cx, body::c_string};
use crate::database::{Iterated, LayoutDesc, LayoutNode, Parts, Stores};
use std::collections::BTreeSet;
use std::fmt::Write as _;

/// Does `id` hold a keyed collection a delivery flattens — itself, or a field or an element
/// below it?  `seen` cuts a recursive type.
fn holds_keyed(desc: &LayoutDesc, id: u16, seen: &mut BTreeSet<u16>) -> bool {
    if !seen.insert(id) {
        return false;
    }
    match desc.nodes.get(&id) {
        Some(LayoutNode::Iterated(it)) => {
            !matches!(it, Iterated::Sorted { .. } | Iterated::Ordered { .. })
        }
        Some(LayoutNode::Record(fields) | LayoutNode::EnumValue(_, fields)) => fields
            .iter()
            .filter(|f| f.is_data())
            .any(|f| holds_keyed(desc, f.content, seen)),
        Some(LayoutNode::Vector(e) | LayoutNode::Array(e)) => holds_keyed(desc, *e, seen),
        _ => false,
    }
}

impl Cx<'_> {
    /// A page's delivery of type `tp`: its descriptor (`loft_dlvd{tp}`) and the walker that
    /// flattens its keyed collections (`loft_dlk{tp}`, or `0` when it holds none), registered
    /// for the unit.  The key tables the walkers name are registered here, while the unit's
    /// tables are still open.
    pub(crate) fn deliver_page(&self, tp: u16) -> (String, String) {
        let mut desc = self.stores.layout_descriptor(&[tp]);
        let holds = |d: &LayoutDesc, id: u16| holds_keyed(d, id, &mut BTreeSet::new());
        let flat_ids: Vec<u16> = desc
            .nodes
            .keys()
            .copied()
            .filter(|id| holds(&desc, *id))
            .collect();
        for &id in &flat_ids {
            let body = self.flatten_body(&desc, id, &|c| holds(&desc, c));
            self.deliver_flats.borrow_mut().entry(id).or_insert(body);
        }
        let collect = if flat_ids.contains(&tp) {
            format!("loft_dlk{tp}")
        } else {
            "0".into()
        };
        Stores::rewrite_iterated(&mut desc);
        // `LayoutDesc::to_delivery_json` without its `flat` map, which the run appends.
        let mut json = desc.to_json();
        json.pop();
        self.deliver_descs.borrow_mut().entry(tp).or_insert(json);
        (format!("loft_dlvd{tp}"), collect)
    }

    /// The flatten walker of node `id` (`Stores::collect_keyed`): a keyed collection is
    /// materialised and named in the `flat` map under its `(rec, pos)`; a record descends into
    /// its fields, a vector or an array into its elements, each only where a keyed collection
    /// can be found below.
    fn flatten_body(&self, desc: &LayoutDesc, id: u16, holds: &dyn Fn(u16) -> bool) -> String {
        let elem_walk = |e: u16, at: &str| {
            format!(
                "    uint32_t v = loft_u32(cx, loft_store_of(cx, at), at.rec, at.pos);\n    uint32_t n = v ? loft_u32(cx, loft_store_of(cx, at), v, 4) : 0;\n    for (uint32_t i = 0; i < n; i++) {{\n        loft_ref x = {{ at.store_nr, {at}, 0 }};\n        loft_dlk{e}(cx, f, x);\n    }}\n"
            )
        };
        match desc.nodes.get(&id) {
            Some(LayoutNode::Iterated(Iterated::Hash { .. })) => {
                let (keys, n) = self.keys_of(id);
                format!(
                    "    loft_dlv_flat_add(cx, f, at, loft_hash_sorted(cx, at, {keys}, {n}));\n"
                )
            }
            Some(LayoutNode::Iterated(Iterated::Radix { .. } | Iterated::Trie { .. })) => {
                "    loft_dlv_flat_add(cx, f, at, loft_radix_sorted(cx, at));\n".into()
            }
            Some(LayoutNode::Iterated(Iterated::Index { .. })) => {
                "    loft_dlv_flat_add(cx, f, at, loft_dlv_index(cx, at));\n".into()
            }
            Some(LayoutNode::Record(fields) | LayoutNode::EnumValue(_, fields)) => {
                let mut s = String::new();
                for fl in fields.iter().filter(|fl| fl.is_data() && holds(fl.content)) {
                    let _ = writeln!(
                        s,
                        "    loft_dlk{}(cx, f, loft_field(cx, at, {}));",
                        fl.content, fl.position
                    );
                }
                s
            }
            Some(LayoutNode::Vector(e)) => {
                let size = desc.sizes.get(e).copied().unwrap_or(0);
                elem_walk(*e, &format!("v, 8 + {size} * i"))
            }
            Some(LayoutNode::Array(e)) => {
                elem_walk(*e, "loft_u32(cx, loft_store_of(cx, at), v, 8 + 4 * i), 8")
            }
            _ => String::new(),
        }
    }

    /// The descriptors and flatten walkers a page's deliveries use, for the browser build only.
    fn deliver_page_code(&self) -> String {
        let descs = self.deliver_descs.borrow();
        let flats = self.deliver_flats.borrow();
        if descs.is_empty() {
            return String::new();
        }
        let mut out = String::from("#if defined(LOFT_BROWSER)\n");
        for (tp, json) in descs.iter() {
            let _ = writeln!(
                out,
                "static const char loft_dlvd{tp}[] = {};",
                c_string(json)
            );
        }
        for tp in flats.keys() {
            let _ = writeln!(
                out,
                "static void loft_dlk{tp}(loft_cx *cx, loft_dlv_flat *f, loft_ref at);"
            );
        }
        for (tp, body) in flats.iter() {
            let _ = writeln!(
                out,
                "static void loft_dlk{tp}(loft_cx *cx, loft_dlv_flat *f, loft_ref at) {{\n    (void) f;\n{body}}}"
            );
        }
        // Off a page the loopback host reads neither.
        out.push_str("#else\n");
        for tp in descs.keys() {
            let _ = writeln!(out, "#define loft_dlvd{tp} 0");
        }
        for tp in flats.keys() {
            let _ = writeln!(out, "#define loft_dlk{tp} 0");
        }
        out.push_str("#endif\n");
        out
    }

    /// The C walker of type `tp` and of every type it reaches, registered for the unit.
    pub(crate) fn deliver_type(&self, tp: u16) -> String {
        if self.deliver_types.borrow_mut().insert(tp)
            && let Some(t) = self.stores.types.get(tp as usize)
        {
            match &t.parts {
                Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                    for f in fields.iter().filter(|f| Self::is_data(f)) {
                        self.deliver_type(f.content);
                    }
                }
                Parts::Vector(e) | Parts::Array(e) => {
                    self.deliver_type(*e);
                }
                _ => {}
            }
        }
        format!("loft_dlv{tp}")
    }

    /// `LayoutField::is_data`: the enum discriminant, an absent field and a `#` synthetic are
    /// not data.
    fn is_data(f: &crate::database::Field) -> bool {
        f.name != "enum" && f.position != u16::MAX && !f.name.starts_with('#')
    }

    /// Every registered walker.
    pub(crate) fn deliver_code(&self) -> String {
        let types = self.deliver_types.borrow();
        let mut out = String::new();
        for &tp in types.iter() {
            let _ = writeln!(
                out,
                "static const char *loft_dlv{tp}(loft_cx *cx, loft_text *o, loft_ref r);"
            );
        }
        for &tp in types.iter() {
            let _ = writeln!(
                out,
                "static const char *loft_dlv{tp}(loft_cx *cx, loft_text *o, loft_ref r) {{"
            );
            out.push_str(&self.deliver_body(tp));
            out.push_str("}\n");
        }
        out.push_str(&self.deliver_page_code());
        out
    }

    fn deliver_body(&self, tp: u16) -> String {
        let stores = self.stores;
        let Some(t) = stores.types.get(tp as usize) else {
            return format!(
                "    (void) o; (void) r;\n    return {};\n",
                c_string(&format!("type {tp} not in descriptor"))
            );
        };
        let raw = |n: u32| format!("    loft_dlv_raw(cx, o, r, {n});\n    return 0;\n");
        match &t.parts {
            // `BaseKind::from_type_id`: a long and an integer are 8 bytes, the rest their own.
            Parts::Base => match tp {
                2 | 6 => raw(4),
                4 => raw(1),
                5 => "    loft_dlv_text(cx, o, r);\n    return 0;\n".into(),
                _ => raw(8),
            },
            Parts::Byte(..) | Parts::Enum(_) => raw(1),
            Parts::Short(..) => "    loft_dlv_short(cx, o, r, 0, 1);\n    return 0;\n".into(),
            Parts::ShortRaw(start, _) => {
                format!("    loft_dlv_short(cx, o, r, {start}, 0);\n    return 0;\n")
            }
            Parts::Int(..) | Parts::IntRaw(..) => raw(4),
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                let mut s = String::from("    const char *e;\n");
                for f in fields.iter().filter(|f| Self::is_data(f)) {
                    let _ = writeln!(
                        s,
                        "    if ((e = loft_dlv{}(cx, o, loft_field(cx, r, {}))) != 0) return e;",
                        f.content, f.position
                    );
                }
                s.push_str("    (void) o; (void) r;\n    return 0;\n");
                s
            }
            Parts::Vector(e) => format!(
                "    uint32_t v = loft_u32(cx, loft_store_of(cx, r), r.rec, r.pos), n = v ? loft_u32(cx, loft_store_of(cx, r), v, 4) : 0;\n    for (uint32_t i = 0; i < n; i++) {{\n        loft_ref x = {{ r.store_nr, v, 8 + {} * i, 0 }};\n        const char *e = loft_dlv{e}(cx, o, x);\n        if (e) return e;\n    }}\n    return 0;\n",
                stores.size(*e)
            ),
            Parts::Array(e) => format!(
                "    uint32_t v = loft_u32(cx, loft_store_of(cx, r), r.rec, r.pos), n = v ? loft_u32(cx, loft_store_of(cx, r), v, 4) : 0;\n    for (uint32_t i = 0; i < n; i++) {{\n        loft_ref x = {{ r.store_nr, loft_u32(cx, loft_store_of(cx, r), v, 8 + 4 * i), 8, 0 }};\n        const char *e = loft_dlv{e}(cx, o, x);\n        if (e) return e;\n    }}\n    return 0;\n"
            ),
            _ => format!(
                "    (void) o; (void) r;\n    return {};\n",
                c_string(&format!(
                    "type {tp} ({}) is a store-internal kind — not in the serializable subset \
                     (cursor-walked in a later phase)",
                    t.name
                ))
            ),
        }
    }
}
