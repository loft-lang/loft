// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @I70 — Database subsystem: the store-schema layout descriptor.
//
// @PLN105 Phase 0 — the layout DESCRIPTOR: a self-describing, structured twin of
// the store's `Parts` schema, emitted once per type-closure so a *foreign* reader
// (the browser JS bridge, later phases) can walk any loft value in linear memory
// with no serialization and no copy — the exact type-driven walk `Stores::read_data`
// (`io.rs`) already does internally, but as data instead of Rust control flow.
//
// Phase 0 is pure loft-side (NO FFI): it proves the descriptor is a *faithful* and
// *sufficient* transcription of the layout, on two independent oracles —
//   * faithfulness — `LayoutDesc::render_dump` reproduces `Stores::layout_dump`
//     byte-for-byte (so its FNV-1a hash IS `layout_algo_hash`, the @PLN97 F9 layout
//     identity): the descriptor loses none of the layout facts the contract pins.
//   * sufficiency  — `Stores::read_via_descriptor`, driven ONLY by the descriptor,
//     reproduces `read_data`'s bytes for a live value: the descriptor carries
//     everything a reader needs to walk the bytes.
//
// The boundary `read_data` enforces by PANIC — keyed collections (hash/index/
// spatial/sorted) and stored `DbRef`/`ChildRec` pointers are store-internal and
// never structurally serialized — is preserved here as data: those become
// `Iterated` / `Ref` / `ChildRec` nodes (walked by cursor in a later phase, never
// as a byte layout), so a foreign reader only ever interprets
// {scalar, text, record, vector, enum}.

use crate::database::{Parts, Stores};
use crate::keys::DbRef;
use std::collections::BTreeMap;

/// The seven seed base types, keyed by their fixed type-id (`Stores::new` order):
/// 0 integer, 1 long, 2 single, 3 float, 4 boolean, 5 text, 6 character. These are
/// the ids `read_data` fast-paths before ever consulting `Parts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseKind {
    Integer,
    Long,
    Single,
    Float,
    Boolean,
    Text,
    Character,
}

impl BaseKind {
    fn from_type_id(tp: u16) -> Option<BaseKind> {
        Some(match tp {
            0 => BaseKind::Integer,
            1 => BaseKind::Long,
            2 => BaseKind::Single,
            3 => BaseKind::Float,
            4 => BaseKind::Boolean,
            5 => BaseKind::Text,
            6 => BaseKind::Character,
            _ => return None,
        })
    }

    /// The wire name the JS reader dispatches its scalar read on (@PLN105 Phase 2 `to_json`).
    fn wire(self) -> &'static str {
        match self {
            BaseKind::Integer => "integer",
            BaseKind::Long => "long",
            BaseKind::Single => "single",
            BaseKind::Float => "float",
            BaseKind::Boolean => "boolean",
            BaseKind::Text => "text",
            BaseKind::Character => "character",
        }
    }
}

/// One field of a `Record` / `EnumValue` node — name, byte position within the
/// record, and the type-id of the field's value (a key into [`LayoutDesc::nodes`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutField {
    pub name: String,
    pub position: u16,
    pub content: u16,
    /// @PLN127 arc D — was this field DECLARED nullable?
    ///
    /// Carried, never RENDERED: `render_dump` is the @PLN97 layout identity and
    /// nullability is not a layout fact — `text?` occupies the same bytes as
    /// `text` and spells absence with a sentinel. So this rides along for a
    /// reader that wants the declaration without moving the hash a store was
    /// written under.
    pub nullable: bool,
}

impl LayoutField {
    /// Is this a field the PROGRAM wrote, rather than bookkeeping the layout
    /// added?
    ///
    /// Three kinds of field are not data and every walk over a record has to
    /// skip all three: the struct-enum discriminant, a field with no position,
    /// and the `#`-prefixed synthetics — an `index` element carries its own
    /// red-black links (`#left_1` / `#right_1` / `#color_1`) INSIDE the record,
    /// and `#color_1` is an ordinary boolean, so a walk that filters by type
    /// alone lets it through.
    ///
    /// One home because the answer is one fact: `read_via_descriptor`, the
    /// browser delivery and @PLN129's query derivation all ask it, and a walk
    /// that disagreed with the others would not look wrong — it would emit one
    /// extra column, or one fewer value.
    #[must_use]
    pub fn is_data(&self) -> bool {
        self.name != "enum" && self.position != u16::MAX && !self.name.starts_with('#')
    }
}

/// The five keyed-collection kinds `read_data` refuses to serialize (store-internal
/// references). Delivered as `Iterated` so a foreign reader knows to walk them by
/// cursor, never as a byte layout. Key lists are kept verbatim so the descriptor
/// re-renders the exact layout-dump line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Iterated {
    Sorted {
        elem: u16,
        keys: Vec<(u16, bool)>,
    },
    Ordered {
        elem: u16,
        keys: Vec<(u16, bool)>,
    },
    Hash {
        elem: u16,
        keys: Vec<u16>,
    },
    Index {
        elem: u16,
        keys: Vec<(u16, bool)>,
        left: u16,
    },
    Radix {
        elem: u16,
        keys: Vec<u16>,
    },
    Trie {
        elem: u16,
        key: u16,
    },
}

impl Iterated {
    /// The element type-id — the type of a record yielded by a cursor over this
    /// collection (Phase 3). Shared by every kind.
    #[must_use]
    pub fn elem(&self) -> u16 {
        match self {
            Iterated::Sorted { elem, .. }
            | Iterated::Ordered { elem, .. }
            | Iterated::Hash { elem, .. }
            | Iterated::Index { elem, .. }
            | Iterated::Radix { elem, .. }
            | Iterated::Trie { elem, .. } => *elem,
        }
    }
}

/// One descriptor node — a structured mirror of one `Parts` variant, with the
/// element type carried by id (into [`LayoutDesc::nodes`]) rather than inlined, so
/// the descriptor is a flat table exactly like the store's type table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutNode {
    /// A seed base scalar or text (`Parts::Base`); the reader dispatches on `kind`.
    Base(BaseKind),
    /// A record — `Parts::Struct`. All fields kept (incl. the struct-enum `enum`
    /// discriminant field) for a faithful render; the reader skips the non-data
    /// fields exactly as `read_data` does.
    Record(Vec<LayoutField>),
    /// A struct-enum variant record — `Parts::EnumValue(tag, fields)`.
    EnumValue(u8, Vec<LayoutField>),
    /// A value enum — `Parts::Enum(variants)`; one byte discriminant. Variants kept
    /// (id + name) for the render and for later name-mapping on the JS side.
    Choices(Vec<(u16, String)>),
    /// Narrow scalars — `Parts::Byte/Short/Int/ShortRaw/IntRaw(start, nullable)`.
    Byte {
        start: i32,
        nullable: bool,
    },
    Short {
        start: i32,
        nullable: bool,
    },
    Int {
        start: i32,
        nullable: bool,
    },
    ShortRaw {
        start: i32,
        nullable: bool,
    },
    /// The 4-byte UNSIGNED encoding — `Parts::IntRaw`.  Distinct from `Int` because the
    /// same four bytes decode differently: no sign extension, and `u32::MAX` rather than
    /// `i32::MIN` is the reserved absence code.
    IntRaw {
        start: i32,
        nullable: bool,
    },
    /// Inline element vector — `Parts::Vector(elem)`.
    Vector(u16),
    /// By-reference element vector — `Parts::Array(elem)`.
    Array(u16),
    /// @PLN105 Phase 3 — a BROWSER-SYNTHETIC node: a keyed collection PRE-FLATTENED at deliver
    /// time to an element array. The array's DATA record is NOT in the node (a type node is shared
    /// by every instance of that type — e.g. every element of a `vector<Bag>`); instead the reader
    /// looks it up in the delivery's `flat` redirect map by the current `(rec, pos)`, which
    /// `deliver_browser` materialised there. `data`'s offset-4 word is the element count, offset-8
    /// onwards the elements, each read as `elem`. Never appears in a REAL type descriptor — only
    /// injected at deliver time, so the loopback/@PLN97-hash paths never see it.
    ///
    /// `stride` says what an element IS, because that now depends on which kind was flattened:
    /// **4** for a record number whose payload starts at byte 8 (`build_rec_scratch` — radix,
    /// trie, index), **8** for a `(record, offset)` PAIR (`build_ref_scratch` — a hash, whose
    /// entries are slots in a chunked arena and have no record of their own, @PLN135 arc H).
    /// Carried rather than assumed: the reader had the 4 hard-coded, and a hash delivered through
    /// it read every second word as a record number — the entries came back shifted, with a
    /// `null` where the last one should be, and only `deliver_wasm` saw it.
    FlatArray {
        elem: u16,
        stride: u32,
    },
    /// A keyed collection — walked by cursor, never structurally (see [`Iterated`]).
    Iterated(Iterated),
    /// A 12-byte stored `DbRef` pointer — `Parts::DbRef` (store-internal).
    Ref,
    /// A co-located child record pointer — `Parts::ChildRec(elem)` (store-internal).
    ChildRec(u16),
}

/// A self-describing layout descriptor for a type-closure: one [`LayoutNode`] per
/// reachable type-id, plus the names and record sizes needed to reproduce the
/// @PLN97 layout dump / hash and to render referenced type names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutDesc {
    pub nodes: BTreeMap<u16, LayoutNode>,
    pub names: BTreeMap<u16, String>,
    pub sizes: BTreeMap<u16, u16>,
}

impl LayoutDesc {
    /// The type-name of `k`, matching `Stores::layout_type_name` (`#<id>` when the
    /// id is out of range — only for a `u16::MAX` field content).
    fn name(&self, k: u16) -> String {
        self.names
            .get(&k)
            .cloned()
            .unwrap_or_else(|| format!("#{k}"))
    }

    fn size(&self, k: u16) -> u16 {
        if k == u16::MAX {
            0
        } else {
            self.sizes.get(&k).copied().unwrap_or(0)
        }
    }

    fn render_fields(&self, fields: &[LayoutField]) -> String {
        fields
            .iter()
            .map(|f| format!("{}@{}:{}", f.name, f.position, self.name(f.content)))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Render one node exactly as `Stores::render_layout_parts` does.
    fn render_parts(&self, node: &LayoutNode) -> String {
        match node {
            LayoutNode::Base(_) => "base".to_string(),
            LayoutNode::Record(fields) => format!("struct{{{}}}", self.render_fields(fields)),
            LayoutNode::Choices(vs) => {
                let inner: Vec<String> = vs.iter().map(|(_, n)| n.clone()).collect();
                format!("enum{{{}}}", inner.join(", "))
            }
            LayoutNode::EnumValue(tag, fields) => {
                format!("enumvalue[{tag}]{{{}}}", self.render_fields(fields))
            }
            LayoutNode::Byte { start, nullable } => format!("byte(start={start},null={nullable})"),
            LayoutNode::Short { start, nullable } => {
                format!("short(start={start},null={nullable})")
            }
            LayoutNode::Int { start, nullable } => format!("int4(start={start},null={nullable})"),
            LayoutNode::ShortRaw { start, nullable } => {
                format!("shortraw(start={start},null={nullable})")
            }
            LayoutNode::IntRaw { start, nullable } => {
                format!("int4raw(start={start},null={nullable})")
            }
            LayoutNode::Vector(e) => {
                format!("vector<{}>(elem_size={})", self.name(*e), self.size(*e))
            }
            LayoutNode::Array(e) => {
                format!("array<{}>(elem_size={})", self.name(*e), self.size(*e))
            }
            // Browser-synthetic; never rendered in a real layout dump.
            LayoutNode::FlatArray { elem, stride } => {
                format!("flatarray<{}>({stride})", self.name(*elem))
            }
            LayoutNode::Iterated(it) => match it {
                Iterated::Sorted { elem, keys } => {
                    format!(
                        "sorted<{}>(keys={keys:?},elem_size={})",
                        self.name(*elem),
                        self.size(*elem)
                    )
                }
                Iterated::Ordered { elem, keys } => {
                    format!(
                        "ordered<{}>(keys={keys:?},elem_size={})",
                        self.name(*elem),
                        self.size(*elem)
                    )
                }
                // Every keyed kind renders its placement token, not just the one whose
                // token is currently off baseline: `tag` renders nothing at BASELINE, so
                // three of these four are invisible today and would silently disagree
                // with `layout_dump` the moment their kind's placement moved — which is
                // exactly the divergence `descriptor_render_reproduces_layout_dump`
                // exists to catch, and it can only catch what both sides render.
                Iterated::Hash { elem, keys } => {
                    format!(
                        "hash<{}>(keys={keys:?},elem_size={}{})",
                        self.name(*elem),
                        self.size(*elem),
                        crate::placement::tag(crate::placement::HASH)
                    )
                }
                Iterated::Index { elem, keys, left } => {
                    format!(
                        "index<{}>(keys={keys:?},left={left},elem_size={}{})",
                        self.name(*elem),
                        self.size(*elem),
                        crate::placement::tag(crate::placement::INDEX)
                    )
                }
                Iterated::Radix { elem, keys } => {
                    format!(
                        "spatial<{}>(keys={keys:?},elem_size={}{})",
                        self.name(*elem),
                        self.size(*elem),
                        crate::placement::tag(crate::placement::RADIX)
                    )
                }
                Iterated::Trie { elem, key } => {
                    format!(
                        "trie<{}>(key={key},elem_size={}{})",
                        self.name(*elem),
                        self.size(*elem),
                        crate::placement::tag(crate::placement::TRIE)
                    )
                }
            },
            LayoutNode::Ref => "dbref12".to_string(),
            LayoutNode::ChildRec(c) => format!("childrec<{}>", self.name(*c)),
        }
    }

    /// Reproduce `Stores::layout_dump` from the descriptor alone — one line per type,
    /// sorted by (name, id). Equal to `layout_dump` iff the transcription is
    /// faithful; its FNV-1a hash is then exactly `layout_algo_hash` (@PLN97 F9).
    #[must_use]
    pub fn render_dump(&self) -> String {
        use std::fmt::Write as _;
        let mut ids: Vec<u16> = self.nodes.keys().copied().collect();
        ids.sort_by(|a, b| self.name(*a).cmp(&self.name(*b)).then(a.cmp(b)));
        let mut out = String::new();
        // @PLN97 F9 — pin the HOST endianness (see `Stores::layout_dump`). MUST stay
        // byte-identical with that twin; `descriptor_render_reproduces_layout_dump` enforces it.
        let _ = writeln!(
            out,
            "@endian\t{}",
            if cfg!(target_endian = "big") {
                "big"
            } else {
                "little"
            }
        );
        for id in ids {
            let _ = writeln!(
                out,
                "{}\tsize={}\t{}",
                self.name(id),
                self.size(id),
                self.render_parts(&self.nodes[&id])
            );
        }
        out
    }

    /// FNV-1a over `render_dump` — the same algorithm as `Stores::layout_algo_hash`,
    /// so an equal dump yields an equal hash (the Phase-0 integrity check).
    #[must_use]
    pub fn layout_hash(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in self.render_dump().bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    /// @PLN105 Phase 2 — serialize the descriptor to JSON for the foreign (JS) reader.
    ///
    /// The descriptor is METADATA emitted once per type-closure and memoized host-side, NOT the
    /// hot path — the value bytes are the zero-copy fast lane — so a self-describing JSON blob
    /// (trivially `JSON.parse`d, robust to schema evolution) is the right contract over a bespoke
    /// binary format. Shape: `{nodes:{<id>:node}, names:{<id>:str}, sizes:{<id>:u16}}`, one node
    /// per reachable type-id — the read-only twin the JS `read(view, desc, typeId, rec, pos)`
    /// switch (§2) dispatches on. Hand-rendered (no serde dep) so it compiles into the lean wasm
    /// build unchanged; the node `kind` tags mirror the `read_via_descriptor` match arms.
    #[must_use]
    pub fn to_json(&self) -> String {
        use std::fmt::Write as _;
        let mut s = String::from("{\"nodes\":{");
        for (i, (id, node)) in self.nodes.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            let _ = write!(s, "\"{id}\":");
            node_json(node, &mut s);
        }
        s.push_str("},\"names\":{");
        for (i, (id, name)) in self.names.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            let _ = write!(s, "\"{id}\":\"{}\"", json_escape(name));
        }
        s.push_str("},\"sizes\":{");
        for (i, (id, size)) in self.sizes.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            let _ = write!(s, "\"{id}\":{size}");
        }
        s.push_str("}}");
        s
    }

    /// @PLN105 Phase 3 — the delivery descriptor: [`to_json`] plus a `flat` REDIRECT map that gives
    /// each pre-flattened keyed collection INSTANCE its materialised data record, keyed by the
    /// `(rec, pos)` where the collection lives (`"<rec>_<pos>"`). A `FlatArray` type node is shared
    /// by every instance of its type (e.g. every element of a `vector<Bag>`), so the per-instance
    /// data cannot live in the node — the reader looks it up here by the current `(rec, pos)`.
    #[must_use]
    pub fn to_delivery_json(&self, flat: &BTreeMap<u64, u32>) -> String {
        use std::fmt::Write as _;
        let mut s = self.to_json();
        s.pop(); // drop the base object's closing '}'
        s.push_str(",\"flat\":{");
        for (i, (k, data)) in flat.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            let (rec, pos) = ((k >> 32) as u32, *k as u32);
            let _ = write!(s, "\"{rec}_{pos}\":{data}");
        }
        s.push_str("}}");
        s
    }

    fn fields_json(fields: &[LayoutField], s: &mut String) {
        use std::fmt::Write as _;
        s.push('[');
        for (i, f) in fields.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            let _ = write!(
                s,
                "{{\"name\":\"{}\",\"pos\":{},\"content\":{}}}",
                json_escape(&f.name),
                f.position,
                f.content
            );
        }
        s.push(']');
    }
}

/// Render one descriptor node as a JSON object. `kind` is the reader's dispatch tag; every
/// type-id reference (`content`/`elem`) is a key into `nodes`, mirroring the flat store type
/// table so the reader never inlines. A free function — a node renders from its own data only
/// (no `LayoutDesc` lookups, unlike `render_parts`).
fn node_json(node: &LayoutNode, s: &mut String) {
    use std::fmt::Write as _;
    match node {
        LayoutNode::Base(k) => {
            let _ = write!(s, "{{\"kind\":\"base\",\"base\":\"{}\"}}", k.wire());
        }
        LayoutNode::Record(fields) => {
            s.push_str("{\"kind\":\"record\",\"fields\":");
            LayoutDesc::fields_json(fields, s);
            s.push('}');
        }
        LayoutNode::EnumValue(tag, fields) => {
            let _ = write!(s, "{{\"kind\":\"enumvalue\",\"tag\":{tag},\"fields\":");
            LayoutDesc::fields_json(fields, s);
            s.push('}');
        }
        LayoutNode::Choices(vs) => {
            s.push_str("{\"kind\":\"enum\",\"variants\":[");
            for (disc, (id, name)) in vs.iter().enumerate() {
                if disc > 0 {
                    s.push(',');
                }
                let _ = write!(
                    s,
                    "{{\"disc\":{disc},\"id\":{id},\"name\":\"{}\"}}",
                    json_escape(name)
                );
            }
            s.push_str("]}");
        }
        LayoutNode::Byte { start, nullable } => {
            let _ = write!(
                s,
                "{{\"kind\":\"byte\",\"start\":{start},\"nullable\":{nullable}}}"
            );
        }
        LayoutNode::Short { start, nullable } => {
            let _ = write!(
                s,
                "{{\"kind\":\"short\",\"start\":{start},\"nullable\":{nullable}}}"
            );
        }
        LayoutNode::Int { start, nullable } => {
            let _ = write!(
                s,
                "{{\"kind\":\"int\",\"start\":{start},\"nullable\":{nullable}}}"
            );
        }
        LayoutNode::ShortRaw { start, nullable } => {
            let _ = write!(
                s,
                "{{\"kind\":\"shortraw\",\"start\":{start},\"nullable\":{nullable}}}"
            );
        }
        LayoutNode::IntRaw { start, nullable } => {
            let _ = write!(
                s,
                "{{\"kind\":\"int4raw\",\"start\":{start},\"nullable\":{nullable}}}"
            );
        }
        LayoutNode::Vector(e) => {
            let _ = write!(s, "{{\"kind\":\"vector\",\"elem\":{e}}}");
        }
        LayoutNode::Array(e) => {
            let _ = write!(s, "{{\"kind\":\"array\",\"elem\":{e}}}");
        }
        LayoutNode::FlatArray { elem, stride } => {
            let _ = write!(
                s,
                "{{\"kind\":\"flatarray\",\"elem\":{elem},\"stride\":{stride}}}"
            );
        }
        LayoutNode::Ref => s.push_str("{\"kind\":\"ref\"}"),
        LayoutNode::ChildRec(e) => {
            let _ = write!(s, "{{\"kind\":\"childrec\",\"elem\":{e}}}");
        }
        LayoutNode::Iterated(it) => {
            // Keyed collections are `iterated` — the reader walks them by cursor (Phase 3),
            // never as a byte layout; only the element type-id is needed to read a yielded rec.
            let (sub, elem) = match it {
                Iterated::Sorted { elem, .. } => ("sorted", elem),
                Iterated::Ordered { elem, .. } => ("ordered", elem),
                Iterated::Hash { elem, .. } => ("hash", elem),
                Iterated::Index { elem, .. } => ("index", elem),
                Iterated::Radix { elem, .. } => ("radix", elem),
                Iterated::Trie { elem, .. } => ("trie", elem),
            };
            let _ = write!(
                s,
                "{{\"kind\":\"iterated\",\"sub\":\"{sub}\",\"elem\":{elem}}}"
            );
        }
    }
}

/// Minimal JSON string escaper for descriptor names (type + field identifiers). They are almost
/// always bare identifiers, but escape the JSON-significant bytes so an unusual name can never
/// produce malformed JSON on the JS side.
fn json_escape(s: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

impl Stores {
    /// @PLN105 Phase 0 — emit the [`LayoutDesc`] for the type-closure of `roots`.
    /// Read-only: it transcribes the existing type table, changing nothing. Walks
    /// the exact same closure the @PLN97 layout hash commits to (`layout_closure`).
    ///
    /// The transcription is EXHAUSTIVE over `Parts` (a new storage kind fails to
    /// compile here), so the descriptor can never silently drop a layout kind.
    #[must_use]
    pub fn layout_descriptor(&self, roots: &[u16]) -> LayoutDesc {
        let mut nodes = BTreeMap::new();
        let mut names = BTreeMap::new();
        let mut sizes = BTreeMap::new();
        for kt in self.layout_closure(roots) {
            let t = &self.types[kt as usize];
            names.insert(kt, t.name.clone());
            sizes.insert(kt, self.size(kt));
            nodes.insert(kt, self.transcribe(kt));
        }
        LayoutDesc {
            nodes,
            names,
            sizes,
        }
    }

    /// Transcribe one type's `Parts` into a [`LayoutNode`]. Exhaustive by design.
    fn transcribe(&self, kt: u16) -> LayoutNode {
        let field = |f: &crate::database::Field| LayoutField {
            name: f.name.clone(),
            position: f.position,
            content: f.content,
            nullable: f.nullable,
        };
        match &self.types[kt as usize].parts {
            Parts::Base => {
                LayoutNode::Base(BaseKind::from_type_id(kt).unwrap_or(BaseKind::Integer))
            }
            Parts::Struct(fields) => LayoutNode::Record(fields.iter().map(field).collect()),
            Parts::EnumValue(tag, fields) => {
                LayoutNode::EnumValue(*tag, fields.iter().map(field).collect())
            }
            Parts::Enum(vs) => LayoutNode::Choices(vs.clone()),
            Parts::Byte(start, nullable) => LayoutNode::Byte {
                start: *start,
                nullable: *nullable,
            },
            Parts::Short(start, nullable) => LayoutNode::Short {
                start: *start,
                nullable: *nullable,
            },
            Parts::Int(start, nullable) => LayoutNode::Int {
                start: *start,
                nullable: *nullable,
            },
            Parts::ShortRaw(start, nullable) => LayoutNode::ShortRaw {
                start: *start,
                nullable: *nullable,
            },
            Parts::IntRaw(start, nullable) => LayoutNode::IntRaw {
                start: *start,
                nullable: *nullable,
            },
            Parts::Vector(e) => LayoutNode::Vector(*e),
            Parts::Array(e) => LayoutNode::Array(*e),
            Parts::Sorted(e, keys) => LayoutNode::Iterated(Iterated::Sorted {
                elem: *e,
                keys: keys.clone(),
            }),
            Parts::Ordered(e, keys) => LayoutNode::Iterated(Iterated::Ordered {
                elem: *e,
                keys: keys.clone(),
            }),
            Parts::Hash(e, keys) => LayoutNode::Iterated(Iterated::Hash {
                elem: *e,
                keys: keys.clone(),
            }),
            Parts::Index(e, keys, left) => LayoutNode::Iterated(Iterated::Index {
                elem: *e,
                keys: keys.clone(),
                left: *left,
            }),
            // The descriptor's `Iterated` needs its own trie variant, and that is a FORMAT
            // change (it crosses to the JS `deliver` reader).  Deferred to step 3 with the
            // rest of the trie's behaviour, so it lands with a test that can construct one —
            // a codec arm you cannot exercise is worse than a loud stub.
            Parts::Trie(e, key) => LayoutNode::Iterated(Iterated::Trie {
                elem: *e,
                key: *key,
            }),
            Parts::Radix(e, keys) => LayoutNode::Iterated(Iterated::Radix {
                elem: *e,
                keys: keys.clone(),
            }),
            Parts::DbRef => LayoutNode::Ref,
            Parts::ChildRec(c) => LayoutNode::ChildRec(*c),
        }
    }

    /// @PLN105 Phase 0 — the sufficiency proof: walk a live value using ONLY the
    /// descriptor for structure (the store is read for bytes exactly as JS reads
    /// wasm memory), appending the same bytes `read_data` would. Returns `Err` for
    /// the store-internal kinds `read_data` refuses (keyed collections, stored
    /// `DbRef`/`ChildRec`) — the walkable-subset boundary; those are cursor-walked
    /// in a later phase, not serialized.
    ///
    /// # Errors
    /// If a node type is not in the serializable subset, or the type-id is absent
    /// from the descriptor.
    pub fn read_via_descriptor(
        &self,
        desc: &LayoutDesc,
        r: &DbRef,
        tp: u16,
        little_endian: bool,
        out: &mut Vec<u8>,
    ) -> Result<(), String> {
        let store = &self.allocations[r.store_nr as usize];
        let push = |out: &mut Vec<u8>, bytes: &[u8]| out.extend_from_slice(bytes);
        let le = little_endian;
        let node = desc
            .nodes
            .get(&tp)
            .ok_or_else(|| format!("type {tp} not in descriptor"))?;
        match node {
            LayoutNode::Base(kind) => match kind {
                BaseKind::Integer => {
                    let v = store.get_int(r.rec, r.pos);
                    push(out, &if le { v.to_le_bytes() } else { v.to_be_bytes() });
                }
                BaseKind::Long => {
                    let v = store.get_long(r.rec, r.pos);
                    push(out, &if le { v.to_le_bytes() } else { v.to_be_bytes() });
                }
                BaseKind::Single => {
                    let v = store.get_single(r.rec, r.pos);
                    push(out, &if le { v.to_le_bytes() } else { v.to_be_bytes() });
                }
                BaseKind::Float => {
                    let v = store.get_float(r.rec, r.pos);
                    push(out, &if le { v.to_le_bytes() } else { v.to_be_bytes() });
                }
                BaseKind::Boolean => out.push(store.get_byte(r.rec, r.pos, 0) as u8),
                BaseKind::Character => {
                    let v = store.get_u32_raw(r.rec, r.pos);
                    push(out, &if le { v.to_le_bytes() } else { v.to_be_bytes() });
                }
                BaseKind::Text => {
                    let s = store.get_str(store.get_u32_raw(r.rec, r.pos));
                    push(out, s.as_bytes());
                }
            },
            LayoutNode::Byte { .. } => out.push(store.get_byte(r.rec, r.pos, 0) as u8),
            LayoutNode::Short { .. } => {
                let v = store.get_short(r.rec, r.pos, 0) as i16;
                push(out, &if le { v.to_le_bytes() } else { v.to_be_bytes() });
            }
            LayoutNode::ShortRaw { start, .. } => {
                let v = store.get_i16_raw(r.rec, r.pos, *start) as i16;
                push(out, &if le { v.to_le_bytes() } else { v.to_be_bytes() });
            }
            LayoutNode::Int { .. } => {
                let v = store.get_i32_raw(r.rec, r.pos);
                push(out, &if le { v.to_le_bytes() } else { v.to_be_bytes() });
            }
            LayoutNode::IntRaw { .. } => {
                let v = store.get_u32_raw(r.rec, r.pos);
                push(out, &if le { v.to_le_bytes() } else { v.to_be_bytes() });
            }
            LayoutNode::Choices(_) => out.push(store.get_byte(r.rec, r.pos, 0) as u8),
            LayoutNode::Record(fields) | LayoutNode::EnumValue(_, fields) => {
                for f in fields {
                    // The enum discriminant, absent fields, and an index node's
                    // `#left`/`#right`/`#color` tree bookkeeping are not data.
                    if !f.is_data() {
                        continue;
                    }
                    let field_r = DbRef {
                        store_nr: r.store_nr,
                        rec: r.rec,
                        pos: r.pos + u32::from(f.position),
                    };
                    self.read_via_descriptor(desc, &field_r, f.content, le, out)?;
                }
            }
            LayoutNode::Vector(elem_tp) => {
                let v_rec = store.get_u32_raw(r.rec, r.pos);
                let length = if v_rec == 0 {
                    0
                } else {
                    store.get_u32_raw(v_rec, 4)
                };
                let elem_size = u32::from(desc.size(*elem_tp));
                for i in 0..length {
                    let elem = DbRef {
                        store_nr: r.store_nr,
                        rec: v_rec,
                        pos: 8 + elem_size * i,
                    };
                    self.read_via_descriptor(desc, &elem, *elem_tp, le, out)?;
                }
            }
            LayoutNode::Array(elem_tp) => {
                let v_rec = store.get_u32_raw(r.rec, r.pos);
                let length = if v_rec == 0 {
                    0
                } else {
                    store.get_u32_raw(v_rec, 4)
                };
                let elm_recs: Vec<u32> = (0..length)
                    .map(|i| store.get_u32_raw(v_rec, 8 + 4 * i))
                    .collect();
                for elm_rec in elm_recs {
                    let elem = DbRef {
                        store_nr: r.store_nr,
                        rec: elm_rec,
                        pos: 8,
                    };
                    self.read_via_descriptor(desc, &elem, *elem_tp, le, out)?;
                }
            }
            LayoutNode::Iterated(_)
            | LayoutNode::Ref
            | LayoutNode::ChildRec(_)
            | LayoutNode::FlatArray { .. } => {
                return Err(format!(
                    "type {tp} ({}) is a store-internal kind — not in the serializable subset \
                     (cursor-walked in a later phase)",
                    desc.name(tp)
                ));
            }
        }
        Ok(())
    }
}
