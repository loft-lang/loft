// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @I70 — Database subsystem: reading a layout dump back into a description.
//
//! @PLN196 step 0 — the layout dump a `.dschema` sidecar records, read back into the
//! [`LayoutDesc`] it was rendered from, and the NAME-KEYED form two descriptions are compared
//! in.
//!
//! A store saved by an older program carries only the text of its layout.  An upgrade has to
//! read that store through the old layout, so the text has to become a description again —
//! the same structure the live type table produces — or the old store has two readers that
//! can drift.  [`LayoutDesc::from_dump`] is that inverse, and it is exact: it refuses any line
//! it cannot re-render byte-for-byte, so a fact the parser would lose is a refusal here
//! rather than a silent default later.
//!
//! Type numbers differ between builds and type NAMES do not, so two producers are compared
//! after [`LayoutDesc::canonical`] renumbers each by name.

use crate::database::{BaseKind, Iterated, LayoutDesc, LayoutField, LayoutNode};
use std::collections::BTreeMap;

impl LayoutDesc {
    /// Read a layout dump (`Stores::layout_dump`, the part of a `.dschema` sidecar after its
    /// `--` fence) back into the description it was rendered from.
    ///
    /// Type ids are assigned in line order; compare with another producer through
    /// [`LayoutDesc::canonical`].  The `@endian` line must name this host's byte order: a
    /// store from the other order is a different layout, and no reader here walks it.
    ///
    /// # Errors
    /// A line that does not parse, a type named twice, a reference to a type the dump does
    /// not describe, an endianness other than this host's, or a dump the parsed description
    /// does not re-render exactly — each named with its line.
    pub fn from_dump(dump: &str) -> Result<LayoutDesc, String> {
        let mut lines = dump.lines();
        let own = if cfg!(target_endian = "big") {
            "big"
        } else {
            "little"
        };
        match lines.next() {
            Some(l) if l == format!("@endian\t{own}") => {}
            Some(l) => return Err(format!("the dump was written on another host: `{l}`")),
            None => return Err("the dump is empty".to_string()),
        }
        // Pass 1: every type's name, id and size, so a reference can resolve to a type
        // whose line comes later.
        let mut rows: Vec<(String, u16, &str)> = Vec::new();
        let mut ids: BTreeMap<String, u16> = BTreeMap::new();
        for line in lines {
            let mut cols = line.splitn(3, '\t');
            let (Some(name), Some(size), Some(parts)) = (cols.next(), cols.next(), cols.next())
            else {
                return Err(format!("not a layout line: `{line}`"));
            };
            let size = size
                .strip_prefix("size=")
                .and_then(|s| s.parse::<u16>().ok())
                .ok_or_else(|| format!("no size on `{line}`"))?;
            let id = u16::try_from(rows.len()).map_err(|_| "too many types".to_string())?;
            if ids.insert(name.to_string(), id).is_some() {
                return Err(format!("the type `{name}` is described twice"));
            }
            rows.push((name.to_string(), size, parts));
        }
        let mut desc = LayoutDesc {
            nodes: BTreeMap::new(),
            names: BTreeMap::new(),
            sizes: BTreeMap::new(),
            placements: BTreeMap::new(),
            linked: std::collections::BTreeSet::new(),
        };
        for (id, (name, size, _)) in rows.iter().enumerate() {
            desc.names.insert(id as u16, name.clone());
            desc.sizes.insert(id as u16, *size);
        }
        // Pass 2: the parts.  An enum's variants name their variant types, so those are
        // resolved after every node exists.
        for (id, (name, _, parts)) in rows.iter().enumerate() {
            let id = id as u16;
            let mut cx = Cx {
                ids: &ids,
                line: name,
            };
            let (node, placement) = cx.parts(name, parts)?;
            if let Some(token) = placement {
                desc.placements.insert(id, token);
            }
            desc.nodes.insert(id, node);
        }
        // An `enum{…}` line names each variant.  A struct-enum's variants are all types (an
        // `enumvalue` whose tag is its position plus one), a value enum's are none
        // (`u16::MAX`), as in `Parts`.  A variant whose type has another name says so
        // (`Item=src1::Item`, loft#2096); a bare name is that variant's type only when EVERY
        // variant of the line resolves that way — a value enum's `Integer` is not the
        // struct-enum variant type `Integer` some other enum declares.
        let mut resolved: Vec<(u16, Vec<(u16, String)>)> = Vec::new();
        for (id, n) in &desc.nodes {
            let LayoutNode::Choices(vs) = n else {
                continue;
            };
            // A variant type is an `enumvalue` carrying its position as its tag — or, for a
            // unit variant whose type had to be renamed, a plain record holding only the tag
            // field (`src1::Nil  struct{enum@0:byte}`), which is how such a row registers.
            let typed = |i: usize, t: u16| match desc.nodes.get(&t) {
                Some(LayoutNode::EnumValue(tag, _)) => usize::from(*tag) == i + 1,
                Some(LayoutNode::Record(fs)) => fs.first().is_some_and(|f| f.name == "enum"),
                _ => false,
            };
            let mut explicit = false;
            let mut all = true;
            let mut out = Vec::with_capacity(vs.len());
            for (i, (_, v)) in vs.iter().enumerate() {
                let (name, ty) = match v.split_once('=') {
                    Some((n, t)) => {
                        explicit = true;
                        (n, t)
                    }
                    None => (v.as_str(), v.as_str()),
                };
                let t = ids.get(ty).copied().filter(|t| typed(i, *t));
                all &= t.is_some();
                out.push((t.unwrap_or(u16::MAX), name.to_string()));
            }
            if !all {
                if explicit {
                    return Err(format!(
                        "type `{}`: names a variant type that is not one of its variants",
                        desc.names[id]
                    ));
                }
                for v in &mut out {
                    v.0 = u16::MAX;
                }
            }
            resolved.push((*id, out));
        }
        for (id, vs) in resolved {
            desc.nodes.insert(id, LayoutNode::Choices(vs));
        }
        let back = desc.render_dump();
        if back != dump {
            let at = back
                .lines()
                .zip(dump.lines())
                .find(|(a, b)| a != b)
                .map_or_else(
                    || "the line count".to_string(),
                    |(a, b)| format!("`{b}` (read back as `{a}`)"),
                );
            return Err(format!("the dump does not read back exactly: {at}"));
        }
        Ok(desc)
    }

    /// The same description with its types renumbered by NAME, in name order — the form in
    /// which two producers (the live table, a sidecar, an older build) are compared, since
    /// their type numbers differ and their names do not.
    ///
    /// # Errors
    /// When two types share a name: a name-keyed description cannot tell them apart, so the
    /// comparison would be meaningless.
    pub fn canonical(&self) -> Result<LayoutDesc, String> {
        let mut order: Vec<(&String, u16)> = self.names.iter().map(|(id, n)| (n, *id)).collect();
        order.sort();
        let mut map: BTreeMap<u16, u16> = BTreeMap::new();
        for (new, (name, old)) in order.iter().enumerate() {
            if new > 0 && order[new - 1].0 == *name {
                return Err(format!("two types are named `{name}`"));
            }
            map.insert(*old, new as u16);
        }
        let re = |k: u16| -> u16 { map.get(&k).copied().unwrap_or(k) };
        let fields = |fs: &[LayoutField]| -> Vec<LayoutField> {
            fs.iter()
                .map(|f| LayoutField {
                    content: re(f.content),
                    target: if f.target == u16::MAX {
                        f.target
                    } else {
                        re(f.target)
                    },
                    ..f.clone()
                })
                .collect()
        };
        let node = |n: &LayoutNode| -> LayoutNode {
            match n {
                LayoutNode::Record(fs) => LayoutNode::Record(fields(fs)),
                LayoutNode::EnumValue(t, fs) => LayoutNode::EnumValue(*t, fields(fs)),
                LayoutNode::Choices(vs) => LayoutNode::Choices(
                    vs.iter()
                        .map(|(t, v)| (if *t == u16::MAX { *t } else { re(*t) }, v.clone()))
                        .collect(),
                ),
                LayoutNode::Vector(e) => LayoutNode::Vector(re(*e)),
                LayoutNode::Array(e) => LayoutNode::Array(re(*e)),
                LayoutNode::ChildRec(e) => LayoutNode::ChildRec(re(*e)),
                LayoutNode::FlatArray { elem, stride } => LayoutNode::FlatArray {
                    elem: re(*elem),
                    stride: *stride,
                },
                LayoutNode::Iterated(it) => LayoutNode::Iterated(match it {
                    Iterated::Sorted { elem, keys } => Iterated::Sorted {
                        elem: re(*elem),
                        keys: keys.clone(),
                    },
                    Iterated::Ordered { elem, keys } => Iterated::Ordered {
                        elem: re(*elem),
                        keys: keys.clone(),
                    },
                    Iterated::Hash { elem, keys } => Iterated::Hash {
                        elem: re(*elem),
                        keys: keys.clone(),
                    },
                    Iterated::Index { elem, keys } => Iterated::Index {
                        elem: re(*elem),
                        keys: keys.clone(),
                    },
                    Iterated::Radix { elem, keys } => Iterated::Radix {
                        elem: re(*elem),
                        keys: keys.clone(),
                    },
                    Iterated::Trie { elem, key } => Iterated::Trie {
                        elem: re(*elem),
                        key: *key,
                    },
                }),
                other => other.clone(),
            }
        };
        Ok(LayoutDesc {
            nodes: self.nodes.iter().map(|(k, n)| (re(*k), node(n))).collect(),
            names: self
                .names
                .iter()
                .map(|(k, n)| (re(*k), n.clone()))
                .collect(),
            sizes: self.sizes.iter().map(|(k, s)| (re(*k), *s)).collect(),
            placements: self
                .placements
                .iter()
                .map(|(k, p)| (re(*k), p.clone()))
                .collect(),
            linked: self.linked.iter().map(|k| re(*k)).collect(),
        })
    }

    /// @PLN196 — the facts a layout dump does not render, one line each, sorted: which types
    /// are stored by reference (`T\tlinked`), and per field its nullability
    /// (`T.f\tnullable`), a reference's target (`T.f\ttarget=U`) and an index group's other
    /// members (`T.f\tgroup=2,3`).  A version-2 sidecar carries this beside the dump, so an
    /// upgrade reads the old layout as completely as the program that wrote it had it.
    #[must_use]
    pub fn render_facts(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        for (id, name) in &self.names {
            if self.linked.contains(id) {
                lines.push(format!("{name}\tlinked"));
            }
            if let Some(LayoutNode::Record(fs) | LayoutNode::EnumValue(_, fs)) = self.nodes.get(id)
            {
                for f in fs {
                    if f.nullable {
                        lines.push(format!("{name}.{}\tnullable", f.name));
                    }
                    if f.target != u16::MAX {
                        let t = self.names.get(&f.target).cloned().unwrap_or_default();
                        lines.push(format!("{name}.{}\ttarget={t}", f.name));
                    }
                    if !f.group.is_empty() {
                        let g: Vec<String> = f.group.iter().map(u16::to_string).collect();
                        lines.push(format!("{name}.{}\tgroup={}", f.name, g.join(",")));
                    }
                }
            }
        }
        lines.sort();
        let mut out = lines.join("\n");
        if !out.is_empty() {
            out.push('\n');
        }
        out
    }

    /// Set the facts [`LayoutDesc::render_facts`] wrote onto a description read from its dump.
    ///
    /// # Errors
    /// A line naming a type or field the description does not have, or a fact it does not
    /// know.
    pub fn apply_facts(&mut self, facts: &str) -> Result<(), String> {
        let mut by_name: BTreeMap<String, u16> =
            self.names.iter().map(|(k, n)| (n.clone(), *k)).collect();
        for line in facts.lines().filter(|l| !l.is_empty()) {
            let (what, fact) = line
                .split_once('\t')
                .ok_or_else(|| format!("not a layout fact: `{line}`"))?;
            if fact == "linked" {
                let id = by_name
                    .get(what)
                    .ok_or_else(|| format!("no type `{what}`"))?;
                self.linked.insert(*id);
                continue;
            }
            // A type name may hold dots only inside brackets; the field is after the last
            // top-level one.
            let (tname, fname) = what
                .rsplit_once('.')
                .ok_or_else(|| format!("not a field: `{what}`"))?;
            let id = *by_name
                .get(tname)
                .ok_or_else(|| format!("no type `{tname}`"))?;
            // A target outside the closure has a name and no node (`layout_descriptor`).
            let target = match fact.strip_prefix("target=") {
                Some(t) => Some(match by_name.get(t) {
                    Some(id) => *id,
                    None => {
                        let id = self.names.keys().max().map_or(0, |m| m + 1);
                        self.names.insert(id, t.to_string());
                        by_name.insert(t.to_string(), id);
                        id
                    }
                }),
                None => None,
            };
            let Some(LayoutNode::Record(fs) | LayoutNode::EnumValue(_, fs)) =
                self.nodes.get_mut(&id)
            else {
                return Err(format!("`{tname}` has no fields"));
            };
            let f = fs
                .iter_mut()
                .find(|f| f.name == fname)
                .ok_or_else(|| format!("`{tname}` has no field `{fname}`"))?;
            if fact == "nullable" {
                f.nullable = true;
            } else if let Some(t) = target {
                f.target = t;
            } else if let Some(g) = fact.strip_prefix("group=") {
                f.group = g
                    .split(',')
                    .map(|n| n.parse::<u16>().map_err(|_| format!("bad group `{g}`")))
                    .collect::<Result<_, _>>()?;
            } else {
                return Err(format!("unknown layout fact `{fact}`"));
            }
        }
        Ok(())
    }

    /// The types no other type in the description refers to — for a store's sidecar, which
    /// records the closure of ONE root, the root itself.  A version-1 sidecar does not name
    /// its root, so an upgrade reads it from here; more than one answer means the sidecar
    /// cannot say.
    #[must_use]
    pub fn unreferenced(&self) -> Vec<u16> {
        let mut referenced = std::collections::BTreeSet::new();
        for n in self.nodes.values() {
            match n {
                LayoutNode::Record(fs) | LayoutNode::EnumValue(_, fs) => {
                    referenced.extend(fs.iter().map(|f| f.content));
                }
                LayoutNode::Choices(vs) => referenced.extend(vs.iter().map(|(v, _)| *v)),
                LayoutNode::Vector(e) | LayoutNode::Array(e) | LayoutNode::ChildRec(e) => {
                    referenced.insert(*e);
                }
                LayoutNode::FlatArray { elem, .. } => {
                    referenced.insert(*elem);
                }
                LayoutNode::Iterated(it) => {
                    referenced.insert(it.elem());
                }
                _ => {}
            }
        }
        self.nodes
            .keys()
            .copied()
            .filter(|k| !referenced.contains(k))
            .collect()
    }

    /// The part of the description reachable from `root`: the closure a store rooted there
    /// can hold, as `Stores::layout_closure` walks it.
    #[must_use]
    pub fn closure_of(&self, root: u16) -> LayoutDesc {
        let mut seen = std::collections::BTreeSet::new();
        let mut stack = vec![root];
        while let Some(t) = stack.pop() {
            if !self.nodes.contains_key(&t) || !seen.insert(t) {
                continue;
            }
            match &self.nodes[&t] {
                LayoutNode::Record(fs) | LayoutNode::EnumValue(_, fs) => {
                    stack.extend(fs.iter().map(|f| f.content));
                }
                LayoutNode::Choices(vs) => stack.extend(vs.iter().map(|(v, _)| *v)),
                LayoutNode::Vector(e) | LayoutNode::Array(e) | LayoutNode::ChildRec(e) => {
                    stack.push(*e);
                }
                LayoutNode::FlatArray { elem, .. } => stack.push(*elem),
                LayoutNode::Iterated(it) => stack.push(it.elem()),
                _ => {}
            }
        }
        let keep = |k: &u16| seen.contains(k);
        LayoutDesc {
            nodes: self
                .nodes
                .iter()
                .filter(|(k, _)| keep(k))
                .map(|(k, n)| (*k, n.clone()))
                .collect(),
            names: self
                .names
                .iter()
                .filter(|(k, _)| keep(k))
                .map(|(k, n)| (*k, n.clone()))
                .collect(),
            sizes: self
                .sizes
                .iter()
                .filter(|(k, _)| keep(k))
                .map(|(k, s)| (*k, *s))
                .collect(),
            placements: self
                .placements
                .iter()
                .filter(|(k, _)| keep(k))
                .map(|(k, p)| (*k, p.clone()))
                .collect(),
            linked: self.linked.iter().copied().filter(|k| keep(k)).collect(),
        }
    }

    /// The description with every fact the DUMP does not carry set to what
    /// [`LayoutDesc::from_dump`] answers for it: a field's declared nullability, a
    /// reference's target, a collection's group, and which types are linked.
    /// Comparing a live description in this form against a parsed one asks exactly "did the
    /// text lose a layout fact", without failing on the facts it is known not to record.
    #[must_use]
    pub fn as_dumped(&self) -> LayoutDesc {
        let strip = |fs: &[LayoutField]| -> Vec<LayoutField> {
            fs.iter()
                .map(|f| LayoutField {
                    nullable: false,
                    target: u16::MAX,
                    group: Vec::new(),
                    ..f.clone()
                })
                .collect()
        };
        let mut d = self.clone();
        d.linked.clear();
        // A reference target outside the closure is a name with no node; the dump has none.
        let nodes: std::collections::BTreeSet<u16> = d.nodes.keys().copied().collect();
        d.names.retain(|k, _| nodes.contains(k));
        for n in d.nodes.values_mut() {
            match n {
                LayoutNode::Record(fs) => *fs = strip(fs),
                LayoutNode::EnumValue(_, fs) => *fs = strip(fs),
                _ => {}
            }
        }
        d
    }
}

/// The resolution context for one dump line: the name→id table, and the line's type name
/// for the error text.
struct Cx<'a> {
    ids: &'a BTreeMap<String, u16>,
    line: &'a str,
}

impl Cx<'_> {
    fn err(&self, what: &str) -> String {
        format!("type `{}`: {what}", self.line)
    }

    fn id(&self, name: &str) -> Result<u16, String> {
        self.ids
            .get(name)
            .copied()
            .ok_or_else(|| self.err(&format!("names `{name}`, which the dump does not describe")))
    }

    /// Parse one line's `<parts>` column; the second answer is a placement token that is not
    /// this build's.
    fn parts(&mut self, name: &str, s: &str) -> Result<(LayoutNode, Option<String>), String> {
        if s == "base" {
            let kind = match name {
                "integer" => BaseKind::Integer,
                "long" => BaseKind::Long,
                "single" => BaseKind::Single,
                "float" => BaseKind::Float,
                "boolean" => BaseKind::Boolean,
                "text" => BaseKind::Text,
                "character" => BaseKind::Character,
                _ => return Err(self.err("is `base` but not one of the seven base types")),
            };
            return Ok((LayoutNode::Base(kind), None));
        }
        if s == "dbref12" {
            return Ok((LayoutNode::Ref, None));
        }
        if let Some(inner) = s.strip_prefix("struct{").and_then(|r| r.strip_suffix('}')) {
            return Ok((LayoutNode::Record(self.fields(inner)?), None));
        }
        if let Some(rest) = s.strip_prefix("enumvalue[") {
            let (tag, rest) = rest
                .split_once("]{")
                .ok_or_else(|| self.err("malformed enumvalue"))?;
            let tag = tag
                .parse::<u8>()
                .map_err(|_| self.err("enumvalue tag is not a byte"))?;
            let inner = rest
                .strip_suffix('}')
                .ok_or_else(|| self.err("malformed enumvalue"))?;
            return Ok((LayoutNode::EnumValue(tag, self.fields(inner)?), None));
        }
        if let Some(inner) = s.strip_prefix("enum{").and_then(|r| r.strip_suffix('}')) {
            // Kept as written (`Item=src1::Item` included) until every node exists; the
            // variants are resolved after the last line.
            let vs = if inner.is_empty() {
                Vec::new()
            } else {
                split_top(inner)
                    .into_iter()
                    .map(|v| (u16::MAX, v.to_string()))
                    .collect()
            };
            return Ok((LayoutNode::Choices(vs), None));
        }
        for (prefix, make) in NARROW {
            if let Some(rest) = s.strip_prefix(prefix).and_then(|r| r.strip_suffix(')')) {
                let (start, null) = rest
                    .strip_prefix("start=")
                    .and_then(|r| r.split_once(",null="))
                    .ok_or_else(|| self.err("malformed narrow integer"))?;
                let start = start
                    .parse::<i32>()
                    .map_err(|_| self.err("narrow start is not a number"))?;
                let nullable = match null {
                    "true" => true,
                    "false" => false,
                    _ => return Err(self.err("narrow null flag is not a boolean")),
                };
                return Ok((make(start, nullable), None));
            }
        }
        if let Some(rest) = s
            .strip_prefix("childrec<")
            .and_then(|r| r.strip_suffix('>'))
        {
            return Ok((LayoutNode::ChildRec(self.id(rest)?), None));
        }
        if let Some(rest) = s.strip_prefix("trie<") {
            // `trie<E[k]>` plus a placement suffix.
            let close = matching(rest, 0).ok_or_else(|| self.err("malformed trie"))?;
            let inner = &rest[..close];
            let (elem, key) = inner
                .rsplit_once('[')
                .and_then(|(e, k)| k.strip_suffix(']').map(|k| (e, k)))
                .ok_or_else(|| self.err("malformed trie key"))?;
            let key = key
                .parse::<u16>()
                .map_err(|_| self.err("trie key is not a field number"))?;
            let placement = placement_of(&rest[close + 1..], crate::placement::TRIE)
                .ok_or_else(|| self.err("malformed trie placement"))?;
            let node = LayoutNode::Iterated(Iterated::Trie {
                elem: self.id(elem)?,
                key,
            });
            return Ok((node, placement));
        }
        // `kind<E>(args)` — the vectors and the keyed collections.
        let (kind, rest) = s
            .split_once('<')
            .ok_or_else(|| self.err(&format!("unknown parts `{s}`")))?;
        let close = matching(rest, 0).ok_or_else(|| self.err("unbalanced `<`"))?;
        let elem = self.id(&rest[..close])?;
        let args = rest[close + 1..]
            .strip_prefix('(')
            .and_then(|r| r.strip_suffix(')'))
            .ok_or_else(|| self.err("missing `(…)`"))?;
        if matches!(kind, "vector" | "array") {
            // The element size is re-rendered from the element's own line; reading it back
            // through the round trip is what checks it.
            if !args.starts_with("elem_size=") {
                return Err(self.err("missing elem_size"));
            }
            let node = if kind == "vector" {
                LayoutNode::Vector(elem)
            } else {
                LayoutNode::Array(elem)
            };
            return Ok((node, None));
        }
        let keys = args
            .strip_prefix("keys=")
            .and_then(|r| r.split_once(",elem_size="))
            .ok_or_else(|| self.err("missing keys"))?;
        let (keys, size_and_tag) = keys;
        let tag = size_and_tag.split_once(',').map_or("", |(_, t)| t);
        let tag = if tag.is_empty() {
            String::new()
        } else {
            format!(",{tag}")
        };
        let (own, node) = match kind {
            "sorted" => (
                "",
                Iterated::Sorted {
                    elem,
                    keys: self.flagged_keys(keys)?,
                },
            ),
            "ordered" => (
                "",
                Iterated::Ordered {
                    elem,
                    keys: self.flagged_keys(keys)?,
                },
            ),
            "hash" => (
                crate::placement::HASH,
                Iterated::Hash {
                    elem,
                    keys: self.plain_keys(keys)?,
                },
            ),
            "index" => (
                crate::placement::INDEX,
                Iterated::Index {
                    elem,
                    keys: self.flagged_keys(keys)?,
                },
            ),
            "spatial" => (
                crate::placement::RADIX,
                Iterated::Radix {
                    elem,
                    keys: self.plain_keys(keys)?,
                },
            ),
            other => return Err(self.err(&format!("unknown collection kind `{other}`"))),
        };
        let placement = if own.is_empty() {
            if !tag.is_empty() {
                return Err(self.err("a sorted collection carries no placement"));
            }
            None
        } else {
            placement_of(&tag, own).ok_or_else(|| self.err("malformed placement"))?
        };
        Ok((LayoutNode::Iterated(node), placement))
    }

    /// `name@pos:type, …` — split at the top level, since a type name holds commas.
    fn fields(&self, inner: &str) -> Result<Vec<LayoutField>, String> {
        if inner.is_empty() {
            return Ok(Vec::new());
        }
        split_top(inner)
            .into_iter()
            .map(|f| {
                let (name, rest) = f
                    .split_once('@')
                    .ok_or_else(|| self.err(&format!("field `{f}` has no position")))?;
                let (pos, tp) = rest
                    .split_once(':')
                    .ok_or_else(|| self.err(&format!("field `{f}` has no type")))?;
                let position = pos
                    .parse::<u16>()
                    .map_err(|_| self.err(&format!("field `{f}` position is not a number")))?;
                // An absent field's content renders as `#65535` (`LayoutDesc::name`).
                let content = if tp == format!("#{}", u16::MAX) {
                    u16::MAX
                } else {
                    self.id(tp)?
                };
                Ok(LayoutField {
                    name: name.to_string(),
                    position,
                    content,
                    nullable: false,
                    target: u16::MAX,
                    group: Vec::new(),
                })
            })
            .collect()
    }

    /// `[0, 2]` — the `{:?}` of a `Vec<u16>`.
    fn plain_keys(&self, s: &str) -> Result<Vec<u16>, String> {
        let inner = s
            .strip_prefix('[')
            .and_then(|r| r.strip_suffix(']'))
            .ok_or_else(|| self.err("keys are not a list"))?;
        if inner.is_empty() {
            return Ok(Vec::new());
        }
        inner
            .split(", ")
            .map(|k| {
                k.parse::<u16>()
                    .map_err(|_| self.err("a key is not a field"))
            })
            .collect()
    }

    /// `[(0, true), (1, false)]` — the `{:?}` of a `Vec<(u16, bool)>`.
    fn flagged_keys(&self, s: &str) -> Result<Vec<(u16, bool)>, String> {
        let inner = s
            .strip_prefix('[')
            .and_then(|r| r.strip_suffix(']'))
            .ok_or_else(|| self.err("keys are not a list"))?;
        if inner.is_empty() {
            return Ok(Vec::new());
        }
        inner
            .split("), (")
            .map(|k| {
                let k = k.trim_start_matches('(').trim_end_matches(')');
                let (f, asc) = k
                    .split_once(", ")
                    .ok_or_else(|| self.err("a key is not a pair"))?;
                let f = f
                    .parse::<u16>()
                    .map_err(|_| self.err("a key is not a field"))?;
                let asc = match asc {
                    "true" => true,
                    "false" => false,
                    _ => return Err(self.err("a key order is not a boolean")),
                };
                Ok((f, asc))
            })
            .collect()
    }
}

type NarrowMake = fn(i32, bool) -> LayoutNode;

/// The narrow-integer spellings, longest prefix first so `int4raw(` is not read as `int4(`.
const NARROW: [(&str, NarrowMake); 5] = [
    ("int4raw(", |start, nullable| LayoutNode::IntRaw {
        start,
        nullable,
    }),
    ("shortraw(", |start, nullable| LayoutNode::ShortRaw {
        start,
        nullable,
    }),
    ("int4(", |start, nullable| LayoutNode::Int {
        start,
        nullable,
    }),
    ("short(", |start, nullable| LayoutNode::Short {
        start,
        nullable,
    }),
    ("byte(", |start, nullable| LayoutNode::Byte {
        start,
        nullable,
    }),
];

/// The placement a suffix records: `Some(None)` when it is this build's (`own`), `Some(Some
/// (token))` when it is another's, `None` when the suffix is malformed.
#[allow(clippy::option_option)]
fn placement_of(suffix: &str, own: &str) -> Option<Option<String>> {
    if suffix.is_empty() {
        // The baseline renders as nothing (`crate::placement::tag`).
        return Some((crate::placement::tag(own) != String::new()).then(|| "1".to_string()));
    }
    let token = suffix.strip_prefix(",placement=")?;
    Some((crate::placement::tag(own) != suffix).then(|| token.to_string()))
}

/// The byte index of the `>` closing an opening `<` that precedes `s[from..]`.
///
/// The depth counts `<` `[` `(` `{` against their closers, and a `->` (a function type's
/// arrow) closes nothing.
fn matching(s: &str, from: usize) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0i32;
    for i in from..b.len() {
        match b[i] {
            b'<' | b'[' | b'(' | b'{' => depth += 1,
            b'>' if i > 0 && b[i - 1] == b'-' => {}
            b'>' | b']' | b')' | b'}' => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// Split at every `, ` that is not inside brackets.
fn split_top(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'<' | b'[' | b'(' | b'{' => depth += 1,
            b'>' if i > 0 && b[i - 1] == b'-' => {}
            b'>' | b']' | b')' | b'}' => depth -= 1,
            b',' if depth == 0 && b.get(i + 1) == Some(&b' ') => {
                out.push(&s[start..i]);
                start = i + 2;
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    out.push(&s[start..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_top_keeps_bracketed_commas() {
        assert_eq!(
            split_top("a@0:byte<-128,false>, b@1:hash<X[k, j]>, c@2:fn(a, b) -> c"),
            vec![
                "a@0:byte<-128,false>",
                "b@1:hash<X[k, j]>",
                "c@2:fn(a, b) -> c"
            ]
        );
    }

    #[test]
    fn a_foreign_placement_is_carried_not_lost() {
        let own = crate::placement::tag(crate::placement::HASH);
        let foreign = if own.is_empty() { ",placement=9" } else { "" };
        let dump = format!(
            "@endian\t{}\nT\tsize=8\tstruct{{k@0:integer}}\nh\tsize=4\thash<T>(keys=[0],elem_size=8{foreign})\ninteger\tsize=8\tbase\n",
            if cfg!(target_endian = "big") {
                "big"
            } else {
                "little"
            }
        );
        let d = LayoutDesc::from_dump(&dump).expect("parses");
        assert_eq!(d.render_dump(), dump);
        assert_eq!(d.placements.len(), 1, "the foreign token is recorded");
    }

    #[test]
    fn a_line_that_does_not_read_back_is_refused() {
        let dump = format!(
            "@endian\t{}\nT\tsize=8\tstruct{{k@0:integer}}\nv\tsize=4\tvector<T>(elem_size=99)\ninteger\tsize=8\tbase\n",
            if cfg!(target_endian = "big") {
                "big"
            } else {
                "little"
            }
        );
        let e = LayoutDesc::from_dump(&dump).expect_err("elem_size 99 is not T's size");
        assert!(e.contains("does not read back"), "{e}");
    }
}
