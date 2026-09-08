// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)
//! Type definitions and type metadata for the database.

use crate::calc;
use crate::data::IntegerSpec;
use crate::database::{Field, Parts, Stores, TYPEVAR_ROW_PREFIX};
use crate::keys::Content;
use std::collections::HashSet;
use std::fmt::Write as _;

/// Type-table name of the 12-byte `DbRef` storage shape a closure record
/// ADOPTS — `free_named`'s cascade reclaims what it points at.
pub(crate) const DBREF_OWNED: &str = "dbref";
/// Type-table name of the BORROWED 12-byte `DbRef` shape (#682) — same bytes,
/// but the target belongs to someone else, so the cascade leaves it alone.
pub(crate) const DBREF_BORROW: &str = "dbref_borrow";

/// Compute the `Key::type_nr` discriminator for a struct field's `content`
/// type.  Built-in base types (0..=5: integer/long/single/float/boolean/text)
/// map to `1 + content` (1..=6) so the legacy 8-byte-int hash paths
/// continue to work.  Narrow integer storage (`Parts::Int` / `Short` /
/// `ShortRaw` / `Byte`) gets its own discriminator (8..=11) so the hash
/// and compare callsites read the right byte width instead of the
/// catch-all 1-byte fallback that silently broke `hash<Row[i32_field]>`
/// and `hash<Row[u32_field]>` lookups.  Non-integer custom types
/// (Struct, Hash, etc.) fall through to 7 (the historical catch-all,
/// reads 1 byte — unchanged behaviour for nested-ref hash keys, which
/// are unusual and unlikely to round-trip correctly anyway).
/// `LOFT_TRACE_SCHEMA=1` — narrate every schema type registration and rollback
/// to stderr as `[schema] <event> <name> -> <nr>`.
///
/// The schema is a shared, long-lived medium: a REPL session, the debugger and
/// the package loader all parse into one `Stores`, and a parse that is later
/// rolled back must leave it exactly as it found it.  When that fails the only
/// symptom is a "Double structure type" abort at some *later*, unrelated parse,
/// which names the collision but not the generation that leaked it — this trace
/// is what attributes the leak to the parse that caused it (#618).
///
/// Pairs with [`Stores::schema_fingerprint`], which turns the same fact into an
/// assertion instead of a thing to read.
fn schema_trace(event: &str, name: &str, nr: u16) {
    if std::env::var_os("LOFT_TRACE_SCHEMA").is_some() {
        crate::loft_eprintln!("[schema] {event} {name:?} -> {nr}");
    }
}

/// `LOFT_TRACE_MINT=1` — narrate every collection-type lookup to stderr as
/// `[mint] <kind> <name> hit=<nr>|MINT=<nr> len=<n> <- <caller frames>`.
///
/// The collection constructors ([`Stores::vector`], `sorted`, `hash`, `index`)
/// dedupe on the RENDERED NAME, so a lookup that misses silently mints a second
/// type for a collection that already exists — a duplicate whose id gets baked
/// into emitted code while the runtime table never grows to hold it.  The
/// symptom is an out-of-bounds panic far from the cause, and nothing in it names
/// the miss.  This trace is what attributes the panic to the lookup that missed:
/// diff a working run against a broken one and the extra `MINT=` line is the
/// fault.  The caller frames separate the pipelines (parse / warm load / native
/// emitter) that each hold their own `Stores`.
///
/// Pairs with [`schema_trace`], which does the same for struct registration.
fn mint_trace(kind: &str, name: &str, found: Option<u16>, len: usize) {
    if std::env::var_os("LOFT_TRACE_MINT").is_none() {
        return;
    }
    let what = found.map_or_else(|| format!("MINT={len}"), |nr| format!("hit={nr}"));
    let bt = std::backtrace::Backtrace::force_capture().to_string();
    let frames: Vec<&str> = bt
        .lines()
        .filter(|l| l.contains("loft::") && !l.contains("types::"))
        .take(4)
        .map(str::trim)
        .collect();
    crate::loft_eprintln!(
        "[mint] {kind} {name:?} {what} len={len} <- {}",
        frames.join(" | ")
    );
}

fn key_type_nr_for_content(content: u16, types: &[Type]) -> i8 {
    key_descriptor_for_content(content, types).0
}

/// The key width AND the field's storage start, from the one place that already knows
/// both (loft#812).
///
/// `Parts::Byte`, `Parts::Short` and `Parts::ShortRaw` all encode a value as `val - min`,
/// so a reader that assumes `min == 0` is off by exactly `min`. Deriving the width and the
/// shift together here is what keeps them from drifting: a width that grows a shift later
/// cannot get one without this match naming it.
///
/// `ShortRaw`'s name is about the absence of the `+1` NULL SENTINEL, not the absence of a
/// shift — it stores `(val - min) as u16` (`set_i16_raw`) and reads `read + min`
/// (`get_short_full`). Reading it as a sign-extended `i16` instead is what made an `i16`
/// key unfindable, and a `u16` key at or above 32768 decode NEGATIVE.
///
/// `Parts::Int` really is raw (`set_i32_raw` stores the value itself), so it and every
/// base type answer `0`, which is inert at the read sites.  `Parts::IntRaw` likewise.
///
/// @FR-L-Narrow-Enc — the `type_nr` is what carries the ENCODING to the key readers, which is
/// why the unsigned 4-byte Part needs its own number rather than sharing `Int`'s: at one
/// number they order, hash and reconstruct a key through one decode, and a `u32` at or above
/// 2147483648 then hashes as a negative and the lookup misses a record that is present.
fn key_descriptor_for_content(content: u16, types: &[Type]) -> (i8, i32) {
    if content <= 5 {
        return (1 + content as i8, 0);
    }
    match &types[content as usize].parts {
        Parts::Int(_, _) => (8, 0),
        Parts::Short(min, _) => (9, *min),
        Parts::Byte(min, _) => (10, *min),
        Parts::ShortRaw(min, _) => (11, *min),
        // The unsigned 4-byte encoding is its own key kind: reading it under `Int`'s
        // signed `get_i32_raw` orders and hashes a `u32` at or above 2147483648 as a
        // negative number, so a keyed lookup answered `null` for a record its own
        // iteration yields (loft#1437).  Raw like `Int`, so the shift is 0.
        Parts::IntRaw(_, _) => (12, 0),
        _ => (7, 0),
    }
}

/// The schema key a narrow-integer Part is registered under.
///
/// One home with the native generator, which looks a registered element type up BY THIS
/// NAME: the two spelling it separately is how a `vector<u32>` element was registered as
/// `int<0,false>` by `init()` while the compiler had `int_raw<0,false>`, minting an extra
/// type and renaming every id past it.
///
/// The kind is the caller's own identity, not a re-derivation from `(width, nullable)` —
/// `Stores::short` is called with `nullable == false` for a slot that still uses the `+1`
/// sentinel encoding, so the width and the flag together do not name the kind.
fn narrow_part_name(kind: crate::data::NarrowIntKind, min: i32, nullable: bool) -> String {
    kind.part_name(min, nullable)
        .expect("every narrow kind has a Part name; only the wide `Int` has none")
}

impl Stores {
    /// @PLN11 D2a — install a store-loaded type schema (`Vec<Type>` from
    /// [`crate::ir_read::read_schema`]) into this `Stores`, replacing whatever
    /// was there.  Rebuilds the `name → known_type` lookup from each type's
    /// name (correct for a non-colliding schema such as the core stdlib; P379
    /// library-qualified names would need the names map stored separately).
    /// The derived `parents` back-references stay empty — only parse-time layout
    /// validation + debug display read them, neither of which runs on the load
    /// path.
    ///
    /// `names` is keyed by the name the *constructor* renders
    /// ([`Stores::vector`], [`Stores::sorted`], …), which is how those calls
    /// dedupe.  [`Stores::finish`] then PROMOTES a collection whose content is
    /// shared and renames it in place — `vector<X>` → `array<X>`,
    /// `sorted<X[k]>` → `ordered<X[k]>` — without touching `names`, so on a cold
    /// run the pre-promotion spelling stays the live lookup key.  Rebuilding the
    /// map from the stored names alone would therefore lose exactly those keys,
    /// and the next `vector`/`sorted` call would miss and mint a SECOND type for
    /// a collection that already exists.  That id gets baked into emitted native
    /// code while `init()` never registers it, so the runtime type table is
    /// shorter than the id it is indexed with → an out-of-bounds panic on every
    /// warm-cache `--native` run.  [`Stores::promoted_lookup_key`] restores the
    /// constructor spelling as an alias so the round-trip is faithful.
    pub(crate) fn install_schema(&mut self, types: Vec<Type>) {
        self.names = types
            .iter()
            .enumerate()
            .map(|(i, t)| (t.name.clone(), i as u16))
            .collect();
        // Aliases go on AFTER the primary map, and never overwrite: a schema
        // that also holds a genuinely unpromoted `vector<X>` keeps its own id.
        for (i, t) in types.iter().enumerate() {
            if let Some(key) = Self::promoted_lookup_key(t) {
                self.names.entry(key).or_insert(i as u16);
            }
        }
        self.types = types;
    }

    /// The constructor spelling of a type [`Stores::finish`] promoted, or `None`
    /// when the type was never promoted.  Promotion rewrites only the leading
    /// word of the rendered name and leaves the `<content[key]>` tail byte-identical
    /// (`Stores::key_name` and `Stores::create_key` render that tail the same way),
    /// so recovering the original key is an exact prefix swap rather than a re-render.
    fn promoted_lookup_key(t: &Type) -> Option<String> {
        match t.parts {
            Parts::Array(_) => t.name.strip_prefix("array<").map(|r| format!("vector<{r}")),
            Parts::Ordered(_, _) => t
                .name
                .strip_prefix("ordered<")
                .map(|r| format!("sorted<{r}")),
            _ => None,
        }
    }

    /**
    To define the 7 base types of the language.
    */
    pub(super) fn base_type(&mut self, name: &str, size: u8) {
        self.names.insert(name.to_string(), self.types.len() as u16);
        self.types
            .push(Type::new(name, Parts::Base, u16::from(size)));
    }

    /**
    Define a new database structure (record).
    # Panics
    when such a structure already exists.
    */
    pub fn structure(&mut self, name: &str, enum_value: i32) -> u16 {
        let num = self.types.len() as u16;
        schema_trace("register", name, num);
        assert!(
            !self.names.contains_key(name),
            "Double structure type {name}"
        );
        self.names.insert(name.to_string(), num);
        let mut tp = Type::new(
            name,
            if enum_value <= 0 {
                Parts::Struct(Vec::new())
            } else {
                Parts::EnumValue(enum_value as u8, Vec::new())
            },
            u16::MAX,
        );
        tp.align = u8::MAX;
        self.types.push(tp);
        num
    }

    #[must_use]
    pub fn has_type(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    /// Take on the type definitions `from` has and this schema does not.
    ///
    /// A debug session parks a `State` whose `Stores` was CLONED from the parser's when the
    /// program was compiled, and every later parse — an `__eval_N` the debugger compiles over
    /// the paused frame — registers its types in the parser's schema only.  A record of such a
    /// type then has an id the running `State` cannot resolve, which is the panic loft#1187
    /// records for its fourth route: *"a struct type defined after the `State` was built is not
    /// in its schema"*.
    ///
    /// Ids are POSITIONAL, and that is what makes the sync a plain append: both schemas grew
    /// from one clone, so this one is a prefix of `from` and every id keeps its meaning.  A
    /// schema that is NOT a prefix is left alone rather than merged — renumbering ids under a
    /// running program is how a keyed read starts naming a type the program never used
    /// (`LOFT_STRICT_SCHEMA_IDS`), and refusing is the recoverable direction.
    ///
    /// Returns whether the schemas agree afterwards.
    pub fn adopt_new_types(&mut self, from: &Self) -> bool {
        if from.types.len() < self.types.len() {
            return false;
        }
        if self
            .types
            .iter()
            .zip(from.types.iter())
            .any(|(a, b)| a.name != b.name)
        {
            return false;
        }
        for tp in &from.types[self.types.len()..] {
            self.types.push(tp.clone());
        }
        for (name, id) in &from.names {
            self.names.entry(name.clone()).or_insert(*id);
        }
        true
    }

    /// The registered db structure name of a built type id (the reverse of `name`).
    /// `""` for an out-of-range id.  Used by typedef to derive a synth `__nullable<S>`
    /// enum's db name from its payload struct's (already-disambiguated) db name.
    #[must_use]
    pub fn type_name(&self, id: u16) -> &str {
        self.types.get(id as usize).map_or("", |t| t.name.as_str())
    }

    #[allow(dead_code)]
    pub fn set_default(&mut self, tp: u16, f: u16, value: Content) {
        if let Parts::Struct(fld) | Parts::EnumValue(_, fld) = &mut self.types[tp as usize].parts {
            fld[f as usize].default = Some(value);
        }
    }

    /// loft#876 — deposit a field's DECLARED constant default (`height: float = 1.5`).
    ///
    /// The by-name twin of [`Self::set_field_nullable`], and deposited for the same
    /// reason: the default lives parser-side as an IR node, the store layer has no
    /// evaluator, and nothing here implies it.  Without it a `text as Struct` cast
    /// writes the TYPE's zero for a key the document omits, while a struct literal
    /// writes the declared default — the same field with two absent values depending
    /// on how the record was made.
    pub fn set_field_default(&mut self, structure: u16, name: &str, value: Content) {
        if structure == u16::MAX {
            return;
        }
        if let Parts::Struct(s) | Parts::EnumValue(_, s) = &mut self.types[structure as usize].parts
            && let Some(f) = s.iter_mut().find(|f| f.name == name)
        {
            f.default = Some(value);
        }
    }

    /**
    Add a new field to a structure
    # Panics
    When the field has a position outside the structure size or on a non-structure type.
    */
    pub fn field(&mut self, structure: u16, name: &str, content: u16) -> u16 {
        if content == u16::MAX {
            return 0;
        }
        let mut others = Vec::new();
        // Which numbers each EXISTING field gains, as a list: a keyed member arriving last has
        // to give its siblings each other as well as itself (see the `(Col-Group)` block below).
        let mut linked: std::collections::HashMap<u16, Vec<u16>> = std::collections::HashMap::new();
        if matches!(
            self.types[content as usize].parts,
            Parts::Struct(_) | Parts::EnumValue(_, _) | Parts::Enum(_)
        ) {
            self.types[content as usize].parents.insert(structure);
        }
        if let Parts::Array(c)
        | Parts::Vector(c)
        | Parts::Sorted(c, _)
        | Parts::Ordered(c, _)
        | Parts::Hash(c, _)
        | Parts::Index(c, _, _)
        | Parts::Trie(c, _)
        | Parts::Radix(c, _) = self.types[content as usize].parts
        {
            // A dead `main_vector<unknown>` wrapper — left over when a vector's
            // element type was an unresolved cross-package forward reference on
            // pass 1 (#375) — carries a `Parts::Vector(u16::MAX)` field.  The
            // real wrapper is built and used instead; this orphan is never
            // instantiated, but the native database-build still walks every
            // registered type to link parents.  Skip the MAX sentinel rather
            // than indexing `self.types` out of bounds.
            if c != u16::MAX {
                self.types[c as usize].parents.insert(structure);
            }
        }
        if let Parts::Struct(fld) | Parts::EnumValue(_, fld) = &self.types[structure as usize].parts
        {
            // @FR-Col-Group — the pairing test, and the one place a group is FORMED.
            //
            // A group needs a KEYED member: two plain vectors over one element type must NOT
            // be linked, because inserting into one must not propagate to the other.  A trie
            // and a spatial are keyed collections like the rest, so they join on the same
            // terms (loft#927).
            //
            // The test is on the PAIR, not on the field being ADDED.  Asking it only of the
            // new field made group formation depend on which member came first:
            // `{ data: vector<E>, look: sorted<E[k]> }` was one record set while
            // `{ look: sorted<E[k]>, data: vector<E> }` was two independent collections,
            // because the keyed field arrived first and found no sibling and the vector then
            // arrived and never ran the search.  Both spellings say the same thing, so both
            // must mean the same thing (loft#1158).  It is the one-way `others` link below
            // (loft#843) one level up and the missing `trie`/`spatial` kinds (loft#927) one
            // level over: every one of the three failed SILENTLY, by building a second
            // collection whose `len` is a legal `0`.
            // The keyed test is on the STRUCT, not on the pair.  `(Col-Group)` reads *"two or
            // more collections over ONE element type in ONE struct are several routes to a
            // single record set, provided at least one of THEM is keyed"* — `them` is every
            // collection over that element type in the struct, and the rule's own second
            // sentence settles the rest by being applied twice: if `a` and `h` are one record
            // set and `b` and `h` are one record set, then a record entering through `a` is in
            // `h`, and a record in `h` is in `b`.
            //
            // Asked of the PAIR, two plain vectors beside a keyed member skipped each other,
            // so `{ a: vector<E>, b: vector<E>, h: hash<E[k]> }` made the hash a HUB rather
            // than the group a set: a write through `h` reached both vectors, a write through
            // either vector reached only `h`, and each vector held its own entries plus what
            // came in through the hash (loft#1375, silent on both backends — `len` of the
            // short member is a legal value, the failure shape this rule's own paragraph
            // warns about).
            //
            // The rule's last sentence — two members neither of which is keyed are INDEPENDENT
            // — is unchanged in what it decides and only qualified in when it applies: it
            // holds where the struct has NO keyed collection over that element type, which is
            // the `group_has_key == false` case below.
            let elem = self.content(content);
            let new_is_keyed = Self::is_group_kind(&self.types[content as usize].parts);
            let mut matched: Vec<u16> = Vec::new();
            let group_has_key = new_is_keyed
                || (elem != u16::MAX
                    && fld.iter().any(|f| {
                        self.content(f.content) == elem
                            && Self::is_group_kind(&self.types[f.content as usize].parts)
                    }));
            for (f_nr, f) in fld.iter().enumerate() {
                let fld_content = self.content(f.content);
                if fld_content == u16::MAX || fld_content != elem {
                    continue;
                }
                if !group_has_key {
                    continue;
                }
                if others.is_empty() {
                    // Leading `u16::MAX` marks this field as a VIEW of records
                    // another field also holds — read by the JSON walker to skip
                    // default-initialising it. It is a marker, not a link, and
                    // everything that walks this list skips it.
                    others.push(u16::MAX);
                }
                // Link BOTH ways. Only the earlier-declared field used to point at
                // the later one, so which collection maintained the others depended
                // on DECLARATION ORDER: an insert spelled through the second field
                // reached only that field, and said nothing. Two keyed collections
                // over one element type are two VIEWS of one set — neither spelling
                // is the privileged one (loft#843).
                others.push(f_nr as u16);
                matched.push(f_nr as u16);
                linked
                    .entry(f_nr as u16)
                    .or_default()
                    .push(fld.len() as u16);
            }
            // A keyed member arriving LAST has to join the members that were skipped while it
            // was absent.  `Stores::field` runs once per field as the struct is built, so at
            // the moment `b` was added to `{ a: vector<E>, b: vector<E>, h: hash<E[k]> }` the
            // struct held no key and the two vectors were correctly INDEPENDENT; the key
            // arrives afterwards and makes them one set.  Without this, membership depended on
            // where the keyed member was WRITTEN — `{h, a, b}` and `{a, h, b}` formed the group
            // and `{a, b, h}` did not — which is the declaration-order dependence loft#843 and
            // loft#1158 already removed for the pairwise case.
            if new_is_keyed {
                for &x in &matched {
                    for &y in &matched {
                        if x != y {
                            linked.entry(x).or_default().push(y);
                        }
                    }
                }
            }
        }
        if let Parts::Struct(s) | Parts::EnumValue(_, s) = &mut self.types[structure as usize].parts
        {
            for (f_nr, f) in s.iter_mut().enumerate() {
                if let Some(add) = linked.get(&(f_nr as u16)) {
                    for n in add {
                        if !f.other_indexes.contains(n) {
                            f.other_indexes.push(*n);
                        }
                    }
                }
            }
            let num = s.len() as u16;
            s.push(Field {
                name: name.to_string(),
                content,
                position: u16::MAX,
                default: None,
                nullable: false,
                other_indexes: others,
            });
            if num > 8
                || self.types[content as usize].complex
                || matches!(
                    self.types[content as usize].parts,
                    Parts::Struct(_) | Parts::EnumValue(_, _)
                )
            {
                self.types[structure as usize].complex = true;
            }
            num
        } else {
            panic!(
                "Adding field {name} to a non structure type {}",
                self.types[structure as usize].name
            );
        }
    }

    /// @PLN127 arc D — mark a field as DECLARED nullable.
    ///
    /// Separate from [`Self::field`] because only one caller knows: the parser,
    /// which peels `Optional(τ)` before laying the field out (the storage is
    /// identical either way) and therefore has the fact exactly where the field
    /// is registered. Every other producer of a field — the generated IR schema,
    /// the index bookkeeping triple — is internal and non-null, so a default of
    /// `false` is the right answer for all of them and none needs touching.
    ///
    /// Keyed by NAME rather than index because `--native` REPLAYS the schema from
    /// generated `init()` code, and the generator emits this call beside the
    /// `db.field` it belongs to. One spelling for both backends is what keeps
    /// them from disagreeing — they did, the first time, and the parity probe is
    /// what caught it.
    ///
    /// Deposited, not derived: `text?` and `text` share a content type and spell
    /// absence with a SENTINEL, so nothing in the store implies this.
    pub fn set_field_nullable(&mut self, structure: u16, name: &str, nullable: bool) {
        if structure == u16::MAX {
            return;
        }
        if let Parts::Struct(s) | Parts::EnumValue(_, s) = &mut self.types[structure as usize].parts
            && let Some(f) = s.iter_mut().find(|f| f.name == name)
        {
            f.nullable = nullable;
        }
    }

    #[must_use]
    pub fn content(&self, tp: u16) -> u16 {
        match self.types[tp as usize].parts {
            Parts::Vector(c)
            | Parts::Array(c)
            | Parts::Ordered(c, _)
            | Parts::Sorted(c, _)
            | Parts::Index(c, _, _)
            | Parts::Hash(c, _)
            | Parts::Radix(c, _)
            | Parts::Trie(c, _) => c,
            _ => u16::MAX,
        }
    }

    #[must_use]
    /// loft#1152 — is this collection a KEYED kind, the sort that can index a shared
    /// record set?  The one home for the question the group test asks of BOTH sides.
    ///
    /// A `vector` is not one: two `vector<T>` fields must stay independent, because
    /// inserting into one must not propagate to the other.  A vector may still JOIN a group
    /// — it is the record holder in the shape DATABASE.md documents by name — but only
    /// beside a member that is a keyed kind.
    fn is_group_kind(parts: &Parts) -> bool {
        matches!(
            parts,
            Parts::Sorted(_, _)
                | Parts::Ordered(_, _)
                | Parts::Hash(_, _)
                | Parts::Index(_, _, _)
                | Parts::Trie(_, _)
                | Parts::Radix(_, _)
        )
    }

    pub fn is_linked(&self, tp: u16) -> bool {
        tp != u16::MAX && self.types[tp as usize].linked
    }

    #[must_use]
    pub fn is_base(&self, tp: u16) -> bool {
        tp != u16::MAX && matches!(self.types[tp as usize].parts, Parts::Base | Parts::Enum(_))
    }

    #[must_use]
    pub fn field_type(&self, rec: u16, fld: u16) -> u16 {
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) = &self.types[rec as usize].parts
        {
            fields[fld as usize].content
        } else {
            u16::MAX
        }
    }

    /// @PLN16.J — is `tp` an (inline) struct?  An intermediate field in an edit
    /// path (`pt.inner.x`) must be one: a nested struct is flattened into the parent
    /// record, so the path resolver descends it by summing offsets in the same
    /// record (the read path does the same — `ShowDb::write_fields`).
    #[must_use]
    pub fn is_struct(&self, tp: u16) -> bool {
        tp != u16::MAX && matches!(self.types[tp as usize].parts, Parts::Struct(_))
    }

    /// True if `tp` is a data-carrying enum variant (`Parts::EnumValue`) — a
    /// struct-like record (a tag byte + the variant's packed fields).  A bare
    /// variant value (e.g. `Circle { r: 2.0 }`) has this type; `size` treats it
    /// like a struct, reporting its own packed record size.
    #[must_use]
    pub fn is_enum_value(&self, tp: u16) -> bool {
        tp != u16::MAX && matches!(self.types[tp as usize].parts, Parts::EnumValue(_, _))
    }

    /// @PLN16.J — resolve a struct field by **name** to `(position, content)`:
    /// `position` is the field's byte offset within the record (added to the
    /// struct's `DbRef.pos`, matching the `ShowDb` read path), `content` its
    /// value-type number.  `None` if `tp` is not a struct / has no such field.
    /// The debugger's field edit (`pt.x = 9`) uses it to address the field in place.
    #[must_use]
    pub fn struct_field(&self, tp: u16, name: &str) -> Option<(u16, u16)> {
        if tp == u16::MAX {
            return None;
        }
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) = &self.types[tp as usize].parts
        {
            fields
                .iter()
                .find(|f| f.name == name)
                .map(|f| (f.position, f.content))
        } else {
            None
        }
    }

    /**
    Determine how structures are actually used.
    */
    pub fn finish(&mut self) {
        let mut vectors = HashSet::new();
        let mut linked = HashSet::new();
        // The collection types of every field that belongs to a linked GROUP —
        // two or more collections over one element type in one struct, auto-linked
        // by `add_field` into several routes to a SINGLE record set (loft#843).
        let mut grouped = Vec::new();
        for t_nr in 0..self.types.len() {
            if let Parts::Struct(fields) | Parts::EnumValue(_, fields) = &self.types[t_nr].parts {
                for f in fields {
                    match self.types[f.content as usize].parts {
                        Parts::Vector(v) | Parts::Sorted(v, _) => vectors.insert(v),
                        Parts::Hash(r, _)
                        | Parts::Radix(r, _)
                        | Parts::Trie(r, _)
                        | Parts::Index(r, _, _) => linked.insert(r),
                        _ => false,
                    };
                    if !f.other_indexes.is_empty() {
                        grouped.push(f.content);
                    }
                }
            }
            if let Parts::Sorted(v, _) = &self.types[t_nr].parts {
                vectors.insert(*v);
            }
        }
        // loft#901 — every member of a group names its elements by a 4-byte record
        // id: a hash slot encodes `rec.rec` (`hash::SLOT_RECORD`), an `array` /
        // `ordered` slot stores it raw and reads it back at a hard-coded payload
        // start, and an `index` keeps its red-black links in FIELDS of the record.
        // None of them can express a position INSIDE a record, so an element that
        // does not own one is unaddressable through its siblings.  Two shapes made
        // elements that do not:
        //
        //   * a hash packs its entries into a shared chunk (@PLN135 arc H), so the
        //     siblings of `hash<E[k]>` + `index<E[k]>` saw two elements at one
        //     record id — the index kept the first and dropped the rest, a sibling
        //     hash held the right NUMBER of slots all naming the first;
        //   * a `sorted` stores its elements inline, so as a view it had no record
        //     to name at all and `sorted<E[k]>` + `sorted<E[k]>` stayed empty.
        //
        // Both disappear once the group's element type is record-backed, which is
        // what `linked` means: it makes `record_new` claim one record per entry
        // instead of an arena slot, and `finish_type` below promote `vector` →
        // `array` and `sorted` → `ordered`.  It was only ever set as a SIDE EFFECT
        // of that promotion, so a group whose members are all keyed never set it.
        for c in grouped {
            let elem = self.content(c);
            if elem != u16::MAX {
                linked.insert(elem);
                self.types[elem as usize].linked = true;
            }
        }
        let mut in_progress = HashSet::new();
        for t_nr in 0..self.types.len() {
            self.finish_type(&linked, t_nr, &mut in_progress);
        }
        self.determine_keys();
        // self.dump_types();
    }

    /// @PLN25 — lay out a SINGLE synth `__nullable<S>` enum + its `Some` variant on demand
    /// (positions/sizes), WITHOUT the full `finish()` (which re-runs every type's `finish_type` and
    /// re-appends keyed-index bookkeeping → corruption at `enum_parent_size`).  Used on-demand for a
    /// FORWARD-referenced `S` whose synth enum is created during pass-2 body parse, after `fill_all`
    /// ran (371).  Empty `linked` set is correct here: a `Some` payload is a dense struct, not a
    /// vector, so no `Vector → Array` promotion applies.
    pub(crate) fn lay_out_synth(&mut self, enum_kt: u16, some_kt: u16) {
        let linked: HashSet<u16> = HashSet::new();
        let mut in_progress: HashSet<usize> = HashSet::new();
        if some_kt != u16::MAX && (some_kt as usize) < self.types.len() {
            self.finish_type(&linked, some_kt as usize, &mut in_progress);
        }
        if enum_kt != u16::MAX && (enum_kt as usize) < self.types.len() {
            // `enumerate` seeds the enum at `u16::MAX`; if anything left it sized, reset so
            // `finish_type` (re)computes size = max(variant sizes) once `Some` is linked.
            self.types[enum_kt as usize].size = u16::MAX;
            self.finish_type(&linked, enum_kt as usize, &mut in_progress);
        }
    }

    /// #686 — lay out a SINGLE closure record on demand (field positions + size), the
    /// sibling of [`Stores::lay_out_synth`] and deferred for the same reason: a capture
    /// whose type was a FORWARD reference in pass 1 can only be typed during pass-2 body
    /// parse, and the lambda's body bakes the field offsets into its IR right then — so
    /// the record must be positioned before it, not by the `finish()` at the end of the
    /// pass.  The full `finish()` is not an option mid-parse: it re-runs every type and
    /// re-appends keyed-index bookkeeping.
    ///
    /// Empty `linked` is correct here: a closure record holds its captures as scalars or
    /// 12-byte DbRefs, never as an inline keyed collection, so no `Vector → Array`
    /// promotion applies.
    pub(crate) fn lay_out_record(&mut self, kt: u16) {
        let linked: HashSet<u16> = HashSet::new();
        let mut in_progress: HashSet<usize> = HashSet::new();
        if kt != u16::MAX && (kt as usize) < self.types.len() {
            self.finish_type(&linked, kt as usize, &mut in_progress);
        }
    }

    /// Assign `(size, align, field positions)` to one registered type, recursing into the
    /// types it contains.
    ///
    /// Enforces @FR-L-Total: layout is a TOTAL function of the finished type — every
    /// registered type ends with exactly one `(size, align, offset-vector)`, and BOTH
    /// backends read those same offsets.  A backend computing a different offset is a bug,
    /// not a second layout.
    ///
    /// Returns early for a type that is not a record shape, or whose size is already
    /// assigned, or that is mid-recursion — so the result does not depend on visit order.
    pub(super) fn finish_type(
        &mut self,
        linked: &HashSet<u16>,
        t_nr: usize,
        in_progress: &mut HashSet<usize>,
    ) {
        if std::env::var("LOFT_TRACE_FINISH").is_ok()
            && self.types[t_nr].name.starts_with("__tuple<")
        {
            crate::loft_eprintln!(
                "[finish_type] ENTER t_nr={t_nr} name={} size_before={} groups_before={}",
                self.types[t_nr].name,
                self.types[t_nr].size,
                self.types[t_nr].field_groups.len(),
            );
        }
        if !matches!(
            self.types[t_nr].parts,
            Parts::Struct(_) | Parts::Enum(_) | Parts::EnumValue(_, _)
        ) || self.types[t_nr].size != u16::MAX
            || in_progress.contains(&t_nr)
        {
            return;
        }
        in_progress.insert(t_nr);
        let mut sizes = Vec::new();
        if let Parts::Enum(values) = self.types[t_nr].parts.clone() {
            let mut size = 1;
            let mut align = 1;
            for value in values {
                if value.0 != u16::MAX {
                    self.finish_type(linked, value.0 as usize, in_progress);
                    if size < self.types[value.0 as usize].size {
                        size = self.types[value.0 as usize].size;
                    }
                    if align < self.types[value.0 as usize].align {
                        align = self.types[value.0 as usize].align;
                    }
                }
            }
            self.types[t_nr].size = size;
            self.types[t_nr].align = align;
        }
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) = self.types[t_nr].parts.clone()
        {
            for f in fields {
                let c_nr = f.content as usize;
                if self.types[c_nr].size == u16::MAX && c_nr != t_nr {
                    self.finish_type(linked, c_nr, in_progress);
                }
                sizes.push((self.types[c_nr].size, self.types[c_nr].align));
                if let Parts::Vector(c) = self.types[c_nr].parts
                    && linked.contains(&c)
                {
                    self.types[c as usize].linked = true;
                    self.types[c_nr].parts = Parts::Array(c);
                    let renamed = format!("array<{}>", self.types[c as usize].name);
                    self.rename_type(c_nr as u16, renamed);
                }
                if let Parts::Sorted(c, key) = self.types[c_nr].parts.clone()
                    && linked.contains(&c)
                {
                    let mut name = format!("ordered<{}[", self.types[c as usize].name);
                    self.key_name(c, &key, &mut name);
                    self.types[c as usize].linked = true;
                    self.types[c_nr].parts = Parts::Ordered(c, key.clone());
                    self.rename_type(c_nr as u16, name);
                }
            }
        }
        // Build per-group descriptors (member field indices, atomic
        // size, alignment, member-internal offsets) so the layout
        // routine can place each LinkedFieldGroup as one block.
        // Non-group fields go through the standard packer.
        //
        // The group's pre-registered `size` / `alignment` come from
        // tuple_def or `index` and are computed at parse time using
        // STACK widths (Type::Integer is 8B regardless of `forced_size`).
        // For STORAGE layout we re-compute from `sizes[]` — the actual
        // database widths (`byte = 1B`, `int4 = 4B`, etc.) — so the
        // atomic block reflects the bytes the Store will hold.  Stack
        // widths stay on the LinkedFieldGroup for codegen / stack-side
        // tuple-element access.
        let groups_descriptor: Vec<(Vec<u16>, u16, u8, Vec<u16>)> = self.types[t_nr]
            .field_groups
            .iter()
            .map(|g| {
                // @PLN114 — a TUPLE group is a record of its elements, and records
                // pack TIGHT: `struct { a: u8, b: u32, c: u16 }` is 1+4+2 = 7 bytes,
                // no padding, because store access is unaligned-tolerant.  Honouring
                // each DB type's natural alignment here padded `(u8, u16)` to 4 where
                // the identical record is 3.  Index groups keep their alignment —
                // only the tuple kind mirrors the record.
                let tight = matches!(g.kind, crate::data::LinkedFieldKind::Tuple);
                let member_sa: Vec<(u16, u8)> = g
                    .field_indices
                    .iter()
                    .map(|&i| {
                        let (sz, al) = sizes[i as usize];
                        // Narrow members pack tight (the record does); an 8-aligned
                        // member keeps its boundary — a fn-ref's 8-byte `d_nr` is
                        // read as an i64 and truncates to 4 without it (#493).
                        //
                        // Keying on the member's `Parts` kind was tried and is WORSE
                        // (29 divergent shapes vs 19): `Parts::Base` covers plain
                        // `integer` AND `character` / `single` / `float`, so it pads
                        // members the record packs tight.  The remaining 19 shapes
                        // need the fn-ref's READER converted off the stack-view
                        // offsets, not a better discriminator here.
                        (sz, if tight { 1 } else { al })
                    })
                    .collect();
                let offsets = crate::data::LinkedFieldGroup::group_member_offsets(&member_sa);
                let storage_alignment = crate::data::LinkedFieldGroup::group_alignment(
                    &member_sa.iter().map(|&(_, a)| a).collect::<Vec<_>>(),
                );
                let storage_size = crate::data::LinkedFieldGroup::group_size(&member_sa);
                (
                    g.field_indices.clone(),
                    storage_size,
                    storage_alignment,
                    offsets,
                )
            })
            .collect();

        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) = &mut self.types[t_nr].parts {
            let mut size = 0;
            let mut alignment = 0;
            if !fields.is_empty() {
                let pos = if groups_descriptor.is_empty() {
                    calc::calculate_positions(
                        &sizes,
                        fields[0].name == "enum",
                        &mut size,
                        &mut alignment,
                    )
                } else {
                    calc::calculate_positions_with_groups(
                        &sizes,
                        &groups_descriptor,
                        fields[0].name == "enum",
                        &mut size,
                        &mut alignment,
                    )
                };
                for (field_nr, pos) in pos.iter().enumerate() {
                    fields[field_nr].position = *pos;
                }
            }
            self.types[t_nr].size = size;
            self.types[t_nr].align = alignment;
        }
        if std::env::var("LOFT_TRACE_FINISH").is_ok() {
            crate::loft_eprintln!(
                "[finish_type] t_nr={t_nr} name={} size={} align={} groups={}",
                self.types[t_nr].name,
                self.types[t_nr].size,
                self.types[t_nr].align,
                self.types[t_nr].field_groups.len(),
            );
        }
    }

    /// The type whose fields carry a keyed collection's key fields.
    ///
    /// Normally this is the element type itself. For a synthetic
    /// `__nullable<S>` element (@PLN25 E2 — a `vector<S>`/`hash<S[k]>` element
    /// rewritten so it can be null), the key fields live in the `Some`
    /// variant's payload, NOT at the enum's top level (offset 0 is the
    /// discriminant). Returning the `Some` variant here lets every
    /// key-resolution site — `hash`/`sorted`/`index` name→number resolution,
    /// `determine_keys`, `field_content` — read the key through the same
    /// payload, so the resolved field numbers and byte offsets agree across
    /// build time and run time. For any other element type, return it
    /// unchanged.
    pub(crate) fn key_owner(&self, content: u16) -> u16 {
        // Single-payload form: a synth `__nullable<S>` keeps S's keys inside the `Some`
        // variant's inline `payload` field (a dense `S`).  The key owner is therefore the
        // payload's struct itself, so every key resolution indexes S's own field list
        // (`"a"→0`), exactly like a non-nullable `hash<S[k]>`.  Resolve the `Some` variant by
        // db name (its variant-list slot can still be a `u16::MAX` placeholder at build time)
        // and return its `payload` content type.  The byte base of that payload within the
        // `Some` record is `key_base`.  Non-nullable elements return `content` unchanged.
        if let Some(some_nr) = self.nullable_some_variant(content)
            && let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
                &self.types[some_nr as usize].parts
            && let Some(f) = fields.iter().find(|f| f.name == "payload")
        {
            return f.content;
        }
        content
    }

    /// Where an `index`'s red-black bookkeeping (`#left_N` / `#right_N` / `#color_N`) lives for
    /// an element type — the RECORD that a tree node actually is.
    ///
    /// For a dense element that is the element struct itself.  For a synth `__nullable<S>` the
    /// stored record is the `Some` variant (discriminant plus the inline payload), and the enum
    /// has no field list to append to at all — so the bookkeeping belongs on `Some`.  Every
    /// reader of `Parts::Index`'s `left_field_nr` resolves through here, so the append and the
    /// byte offset `tree` descends from cannot disagree.
    pub(crate) fn index_owner(&self, content: u16) -> u16 {
        self.nullable_some_variant(content).unwrap_or(content)
    }

    /// Resolve the `Some` variant type-nr of a synth `__nullable<S>` element by db name,
    /// or `None` for any other element.  Shared by `key_owner` / `key_base`.
    pub(crate) fn nullable_some_variant(&self, content: u16) -> Option<u16> {
        // Guard `content == u16::MAX` / out-of-range (an unbuilt or first-pass type id reaches
        // `key_owner` from `new_record_field_op`); identity-resolve such ids rather than OOB-panic.
        let name = &self.types.get(content as usize)?.name;
        if name.starts_with("__nullable<") {
            return self.names.get(&format!("{name}::Some")).copied();
        }
        None
    }

    /// Is `rec`, an element of a collection whose element type is `content`, the ABSENT half
    /// of a `__nullable<S>` — the zeroed slot (discriminant 0) or the explicit `Null` variant
    /// (1), anything but a `Some` (2)?  `false` for every dense element.
    ///
    /// The one home for the test both halves of `@FR-Col-Group` ask about a null element: it
    /// is in no keyed view, so `link_siblings` indexes only a `Some` and [`Stores::remove`]
    /// unlinks only a `Some`.  Read apart, the ENTER half skipped it and the LEAVE half did
    /// not — a `vector<E?>` slot holding null reached the unlink loop as a record whose key
    /// reads as zero, and the hash zeroed a live sibling's bucket while the index asserted
    /// `Item not found`.
    pub(crate) fn absent_nullable_record(&self, content: u16, rec: &crate::keys::DbRef) -> bool {
        self.nullable_some_variant(content).is_some()
            && self.store(rec).get_byte(rec.rec, rec.pos, 0) != 2
    }

    /// The variant record that actually declares a field of a STRUCT-ENUM value.
    ///
    /// `c.limbs` on `enum Shape { Circle { limbs: vector<float> }, … }` is written through
    /// the ENUM type, but the field lives in the variant's own `EnumValue` record — the
    /// enum itself is `Parts::Enum`, which carries a variant list and no fields at all.
    /// Resolving a field against it therefore misses, and the two resolvers answer their
    /// not-found sentinels: `field_nr` says `0`, which is a real field number, and
    /// `field_type` says `u16::MAX`, which is then used to index the type table
    /// (loft#977 — a panic for a collection field, and the wrong field for the rest).
    ///
    /// Keyed on the field's byte `position` AND its `content` type, because the offset alone
    /// names several fields: every collection is one 4-byte handle laid down straight after
    /// the discriminant, so two variants each holding one put it at the same place.  A
    /// `vector` variant and a `hash` variant then look identical by offset, and resolving to
    /// the wrong one appends the record to a vector instead of keying it — silent at the
    /// write, visible only later as a lookup that finds nothing.
    ///
    /// Answers `enum_tp` unchanged for anything that is not a struct-enum and for a field no
    /// variant declares — identity, exactly like `key_owner`, so a caller can always ask.
    pub(crate) fn variant_owning_field(&self, enum_tp: u16, position: u16, content: u16) -> u16 {
        let Some(Parts::Enum(variants)) = self.types.get(enum_tp as usize).map(|t| &t.parts) else {
            return enum_tp;
        };
        for (variant_tp, _) in variants {
            // A payload-less variant registers as `u16::MAX` until (and unless) it is given
            // one, so it names no record to resolve against.
            if *variant_tp == u16::MAX {
                continue;
            }
            if let Some(Parts::EnumValue(_, fields)) =
                self.types.get(*variant_tp as usize).map(|t| &t.parts)
                && fields
                    .iter()
                    .any(|f| f.position == position && f.content == content)
            {
                return *variant_tp;
            }
        }
        enum_tp
    }

    /// Byte offset of S's fields within a stored `__nullable<S>` (`Some`) record — the
    /// position of the inline `payload` field, after the discriminant.  `0` for any
    /// non-nullable element (S's fields then sit at the record root).  Alignment-dependent,
    /// so it is read from the built `Some` structure, never hardcoded.
    pub(crate) fn key_base(&self, content: u16) -> u16 {
        if let Some(some_nr) = self.nullable_some_variant(content)
            && let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
                &self.types[some_nr as usize].parts
            && let Some(f) = fields.iter().find(|f| f.name == "payload")
        {
            return f.position;
        }
        0
    }

    /// THE key-position chokepoint.  Resolve key field `k` of a keyed collection's element
    /// `content` to its `(content_type, absolute_byte_position)` within a stored record.
    /// Folds `key_owner` (which struct holds the keys) and `key_base` (where that struct
    /// sits inside the record) so the build path (`determine_keys`) and the runtime read
    /// path (`field_content`) compute the SAME absolute offset — single-site, so a synth
    /// `__nullable<S>`'s payload base cannot be added at one and forgotten at the other
    /// (which would silently corrupt the keyed collection).
    pub(crate) fn key_field(&self, content: u16, k: u16) -> Option<(u16, u16)> {
        let owner = self.key_owner(content);
        let base = self.key_base(content);
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &self.types[owner as usize].parts
        {
            let f = &fields[k as usize];
            return Some((f.content, base + f.position));
        }
        None
    }

    /// THE key-arity chokepoint: every `(content type, absolute position)` ONE key field
    /// contributes, in comparison order.
    ///
    /// Normally exactly one.  A TUPLE field contributes one per element, at the element's
    /// own offset inside the field: a tuple key is a compound key that happens to be
    /// spelled as a single field, so `sorted<Cell[pos]>` with `pos: (integer, integer)` has
    /// to behave as `sorted<Cell[x, y]>` does — element 0 first, element 1 breaking its
    /// ties.  Without the expansion the whole tuple took the catch-all descriptor and every
    /// element after the first was invisible to the comparison: three cells inserted at
    /// `(1,9)`, `(2,0)` and `(1,2)` left TWO in the collection, the two sharing an element 0
    /// having collapsed into one.  Nested tuples expand the same way, so the flat element
    /// order is the comparison order all the way down.
    ///
    /// Both readers of key arity go through here — [`Stores::determine_keys_for`], which
    /// bakes the comparison descriptors, and [`Stores::get_keys`], which tells `read_key`
    /// how many stack values a lookup pushed.  They MUST agree: `read_key` pops one value
    /// per entry, so a list one short leaves a key value on the stack and the very next
    /// `get_stack::<DbRef>()` reads it as the collection (loft#720 is the same failure for
    /// `spatial<T[x,y]>`, and a tuple key reproduced it exactly — `h[(3, 4)]` looked up in
    /// store #4).  Deriving the arity twice is what let them disagree, so there is one
    /// derivation and two consumers.
    pub(crate) fn key_contents_for_field(&self, content: u16, position: u16) -> Vec<(u16, u16)> {
        if (content as usize) < self.types.len()
            && self.types[content as usize].name.starts_with("__tuple<")
            && let Parts::Struct(fields) = &self.types[content as usize].parts
        {
            let out: Vec<(u16, u16)> = fields
                .iter()
                .flat_map(|f| self.key_contents_for_field(f.content, position + f.position))
                .collect();
            if !out.is_empty() {
                return out;
            }
        }
        vec![(content, position)]
    }

    /// The comparison descriptors one key field contributes — [`Stores::key_contents_for_field`]
    /// resolved to key type codes.  `asc` applies to every descriptor the field produces: a
    /// descending tuple key reverses the whole tuple, which is what `sorted<Cell[-pos]>`
    /// reads as.
    fn key_descriptors_for_field(
        &self,
        content: u16,
        position: u16,
        asc: bool,
    ) -> Vec<crate::keys::Key> {
        self.key_contents_for_field(content, position)
            .into_iter()
            .map(|(c, pos)| {
                let (mut type_nr, start) = key_descriptor_for_content(c, &self.types);
                if !asc {
                    type_nr = -type_nr;
                }
                crate::keys::Key {
                    type_nr,
                    position: pos,
                    start,
                }
            })
            .collect()
    }

    pub(super) fn determine_keys(&mut self) {
        for t_nr in 0..self.types.len() {
            self.determine_keys_for(t_nr);
        }
    }

    /// Compute the runtime key descriptors of ONE collection type.
    ///
    /// `determine_keys` runs this over every type at the end of a parse, which is late
    /// enough for anything the parse only READS.  It is not late enough for a fact the
    /// parse BAKES: `fill_iter` writes the descriptor list straight into the `OpIterate`
    /// operand, so a collection type first created in pass 2 — after pass 1's `finish()`
    /// and before pass 2's — baked an EMPTY list and the range iterator then indexed
    /// `keys[0]` on it (loft#689: SIGSEGV on the interpreter, and on `--native` an
    /// out-of-bounds in `key_compare`).  Calling this on demand at the bake site closes
    /// the window, the same shape as `lay_out_record` for a pass-2-created struct's
    /// layout (loft#686).
    ///
    /// Idempotent: it clears and recomputes, so the end-of-parse sweep still produces the
    /// identical table whether or not a bake site ran it earlier.
    pub(crate) fn determine_keys_for(&mut self, t_nr: usize) {
        match self.types[t_nr].parts.clone() {
            // Hash and Radix both key on a bare `Vec<u16>` of ascending field
            // numbers.  Radix's key positions are what the Morton oracle reads
            // (@PLN48 S2): each coordinate axis is one entry, interleaved in list
            // order.
            // A trie keys on ONE field; same registration, a one-element list.
            Parts::Trie(c, k) => {
                self.types[t_nr].keys.clear();
                if let Some((content, position)) = self.key_field(c, k) {
                    let (tp, start) = key_descriptor_for_content(content, &self.types);
                    self.types[t_nr].keys.push(crate::keys::Key {
                        type_nr: tp,
                        position,
                        start,
                    });
                }
            }
            Parts::Hash(c, key_fields) | Parts::Radix(c, key_fields) => {
                self.types[t_nr].keys.clear();
                for key_field in key_fields {
                    if let Some((content, position)) = self.key_field(c, key_field) {
                        let ks = self.key_descriptors_for_field(content, position, true);
                        self.types[t_nr].keys.extend(ks);
                    }
                }
            }
            Parts::Ordered(c, key_fields)
            | Parts::Sorted(c, key_fields)
            | Parts::Index(c, key_fields, _) => {
                self.types[t_nr].keys.clear();
                for (key_field, asc) in &key_fields {
                    if let Some((content, position)) = self.key_field(c, *key_field) {
                        let ks = self.key_descriptors_for_field(content, position, *asc);
                        self.types[t_nr].keys.extend(ks);
                    }
                }
            }
            _ => (),
        }
        // `LOFT_TRACE_KEYS=1` prints the baked comparison descriptors per collection.  Worth
        // its ten lines: key ARITY has desynchronised from the lookup twice now (loft#720's
        // `spatial<T[x,y]>`, and a tuple key field), and both times the symptom was a
        // collection read from the wrong store rather than anything naming the keys.
        if std::env::var_os("LOFT_TRACE_KEYS").is_some() && !self.types[t_nr].keys.is_empty() {
            crate::loft_eprintln!(
                "[keys] t_nr={t_nr} {} -> {:?}",
                self.types[t_nr].name,
                self.types[t_nr]
                    .keys
                    .iter()
                    .map(|k| (k.type_nr, k.position))
                    .collect::<Vec<_>>()
            );
        }
    }

    #[allow(dead_code)]
    pub fn dump_types(&self) {
        for t_nr in 0..self.types.len() {
            print!("{t_nr}:{}", self.show_type(t_nr as u16, true));
        }
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn dump_type(&self, name: &str) -> String {
        for t in 0..self.types.len() {
            if self.types[t].name == name {
                return self.show_type(t as u16, false);
            }
        }
        String::new()
    }

    /// Debug helper: pretty-print a type's storage layout — overall
    /// size + alignment, plus every field's name, byte position, byte
    /// size, and content type.  Fields are sorted by position so gaps
    /// and overlaps are visually obvious.
    ///
    /// Useful for diagnosing layout bugs (e.g. tree::add writing
    /// at wrong offsets when bookkeeping fields land at non-contiguous
    /// positions per `calc::calculate_positions`'s alignment-aware
    /// reordering).
    ///
    /// For Sorted/Hash/Index/Radix: also shows the content struct's
    /// layout indented underneath, since those types' bookkeeping
    /// lives inside the content struct.
    #[must_use]
    #[allow(dead_code)]
    pub fn debug_layout(&self, name: &str) -> String {
        for t in 0..self.types.len() {
            if self.types[t].name == name {
                return self.debug_layout_by_nr(t as u16, 0);
            }
        }
        format!("(unknown type: {name})\n")
    }

    /// Same as `debug_layout` but takes a type id directly.
    #[must_use]
    #[allow(dead_code)]
    pub fn debug_layout_by_nr(&self, tp: u16, indent: usize) -> String {
        use std::fmt::Write;
        if tp == u16::MAX || (tp as usize) >= self.types.len() {
            return format!("(unknown type id: {tp})\n");
        }
        let pad = " ".repeat(indent);
        let mut out = String::new();
        let t = &self.types[tp as usize];
        let kind = match &t.parts {
            Parts::Struct(_) => "struct",
            Parts::EnumValue(disc, _) => return self.debug_layout_enumvalue(tp, indent, *disc),
            Parts::Enum(_) => "enum",
            Parts::Vector(_) => "vector",
            Parts::Array(_) => "array",
            Parts::Sorted(_, _) => "sorted",
            Parts::Ordered(_, _) => "ordered",
            Parts::Hash(_, _) => "hash",
            Parts::Index(_, _, _) => "index",
            Parts::Radix(_, _) => "spatial",
            Parts::Trie(_, _) => "trie",
            _ => "<other>",
        };
        let _ = writeln!(
            out,
            "{pad}[{tp}] {} {} (size={}, align={})",
            t.name, kind, t.size, t.align
        );
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) = &t.parts {
            self.debug_layout_fields(&mut out, indent + 2, fields);
        } else if let Parts::Vector(c)
        | Parts::Array(c)
        | Parts::Sorted(c, _)
        | Parts::Ordered(c, _)
        | Parts::Hash(c, _)
        | Parts::Trie(c, _)
        | Parts::Radix(c, _) = t.parts
        {
            let _ = writeln!(
                out,
                "{pad}  content → [{c}] {} (size={}, align={})",
                self.types[c as usize].name,
                self.types[c as usize].size,
                self.types[c as usize].align,
            );
            if matches!(
                self.types[c as usize].parts,
                Parts::Struct(_) | Parts::EnumValue(_, _)
            ) {
                out += &self.debug_layout_by_nr(c, indent + 4);
            }
        } else if let Parts::Index(c, _, left_field_nr) = t.parts {
            let _ = writeln!(
                out,
                "{pad}  content → [{c}] {} (size={}, align={}); bookkeeping starts at field index {left_field_nr}",
                self.types[c as usize].name,
                self.types[c as usize].size,
                self.types[c as usize].align,
            );
            // Compute the byte offset where tree::add expects to find
            // RB_LEFT — this is `database.fields(tp)`'s return value.
            let c = self.index_owner(c);
            if let Parts::Struct(fs) | Parts::EnumValue(_, fs) = &self.types[c as usize].parts {
                if (left_field_nr as usize) < fs.len() {
                    let left = &fs[left_field_nr as usize];
                    let _ = writeln!(
                        out,
                        "{pad}  tree::add starts at byte offset = 8 + {} (= field[{}] '{}' position) = {}",
                        left.position,
                        left_field_nr,
                        left.name,
                        8 + left.position,
                    );
                }
                out += &self.debug_layout_by_nr(c, indent + 4);
            }
        }
        if !t.keys.is_empty() {
            let _ = writeln!(out, "{pad}  keys:");
            for k in &t.keys {
                let _ = writeln!(
                    out,
                    "{pad}    field {} type {} {}",
                    k.position,
                    k.type_nr.abs(),
                    if k.type_nr < 0 { "desc" } else { "asc" },
                );
            }
        }
        out
    }

    /// Helper for `debug_layout_by_nr` — handles the EnumValue arm
    /// separately so the disc byte is documented.
    fn debug_layout_enumvalue(&self, tp: u16, indent: usize, disc: u8) -> String {
        use std::fmt::Write;
        let pad = " ".repeat(indent);
        let mut out = String::new();
        let t = &self.types[tp as usize];
        let _ = writeln!(
            out,
            "{pad}[{tp}] {} enum-value (disc={disc}, size={}, align={})",
            t.name, t.size, t.align
        );
        if let Parts::EnumValue(_, fields) = &t.parts {
            self.debug_layout_fields(&mut out, indent + 2, fields);
        }
        out
    }

    /// Validate a type's storage layout — detects overlapping fields.
    /// Only `Parts::Enum` (the tagged-union container) legitimately
    /// overlaps its variants; everything else (struct, enum-value,
    /// collection content) must have non-overlapping fields.
    ///
    /// Returns a list of human-readable issues; empty Vec means the
    /// layout is internally consistent.  Recursively checks the
    /// content type for collection kinds.
    ///
    /// Use this after `database.finish()` to catch layout bugs (e.g.
    /// late-mutation that leaves bookkeeping fields at position 0
    /// overlapping user data).
    #[must_use]
    #[allow(dead_code)]
    pub fn validate_layout(&self, name: &str) -> Vec<String> {
        for t in 0..self.types.len() {
            if self.types[t].name == name {
                let mut visited = std::collections::HashSet::new();
                let mut issues = Vec::new();
                self.validate_layout_by_nr(t as u16, &mut visited, &mut issues);
                return issues;
            }
        }
        vec![format!("(unknown type: {name})")]
    }

    /// Walk every registered type and validate its layout.  Returns
    /// a flat list of issues across all types (each line prefixed by
    /// the type name).  Use this after `database.finish()` to catch
    /// layout bugs in any user-defined struct / enum-value /
    /// collection-content type before they corrupt runtime data.
    ///
    /// Skips synthetic database types whose names start with `__`
    /// (currently none, but reserved for the parser-side `__tuple<…>`
    /// shape and similar) and the built-in primitives.
    #[must_use]
    pub fn validate_all_layouts(&self) -> Vec<String> {
        let mut visited = std::collections::HashSet::new();
        let mut issues = Vec::new();
        for tp in 0..self.types.len() {
            let t = &self.types[tp];
            // Skip primitive built-ins (no struct layout to validate)
            // and types whose size is u16::MAX (unlaid-out — likely
            // a parser placeholder that finish_type didn't reach).
            if t.size == u16::MAX {
                continue;
            }
            match t.parts {
                Parts::Struct(_)
                | Parts::EnumValue(_, _)
                | Parts::Enum(_)
                | Parts::Vector(_)
                | Parts::Array(_)
                | Parts::Sorted(_, _)
                | Parts::Ordered(_, _)
                | Parts::Hash(_, _)
                | Parts::Index(_, _, _)
                | Parts::Radix(_, _)
                | Parts::Trie(_, _) => {
                    self.validate_layout_by_nr(tp as u16, &mut visited, &mut issues);
                }
                _ => {}
            }
        }
        // Dedup — recursion can produce the same issue multiple times
        // when a struct is referenced from many places.
        issues.sort();
        issues.dedup();
        issues
    }

    /// Same as `validate_layout` but takes a type id directly.
    /// `visited` prevents infinite recursion through cyclic
    /// references (e.g. struct containing a vector of itself).
    #[allow(dead_code, clippy::many_single_char_names)]
    pub fn validate_layout_by_nr(
        &self,
        tp: u16,
        visited: &mut std::collections::HashSet<u16>,
        issues: &mut Vec<String>,
    ) {
        // u16::MAX is the canonical "unresolved type" sentinel — it
        // appears on field.content during parser-recovery paths
        // (e.g. an unknown type referenced from a struct field, or
        // a `Type::Unknown(0)` that propagated into a database
        // field).  Silently skip; the underlying parse error has
        // already been reported.  Only flag IDs that are positive
        // but past the end of the registry — that's a real bug.
        if tp == u16::MAX {
            return;
        }
        if (tp as usize) >= self.types.len() {
            issues.push(format!("type id {tp} out of range"));
            return;
        }
        if !visited.insert(tp) {
            return;
        }
        let t = &self.types[tp as usize];
        match &t.parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                self.check_fields_overlap(tp, fields, issues);
                self.check_fields_within_size(tp, fields, issues);
                // Recurse into each field's content type.
                for f in fields {
                    self.validate_layout_by_nr(f.content, visited, issues);
                }
            }
            Parts::Enum(_) => {
                // Tagged-union variants legitimately overlap.  Each
                // variant is a separate EnumValue type with its own
                // layout — recurse into the children.  We trust the
                // parent's `t.size` to be the max of variant sizes
                // (set in finish_type for Parts::Enum at line 238).
                let children: Vec<u16> = self
                    .types
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, child)| {
                        if let Parts::EnumValue(_, _) = &child.parts {
                            // Heuristic: child belongs to this enum if
                            // its name has the parent's name as prefix
                            // OR if the child appears in `parents`.
                            if child.parents.contains(&tp) {
                                return Some(idx as u16);
                            }
                        }
                        None
                    })
                    .collect();
                for c in children {
                    self.validate_layout_by_nr(c, visited, issues);
                }
            }
            Parts::Vector(c)
            | Parts::Array(c)
            | Parts::Sorted(c, _)
            | Parts::Ordered(c, _)
            | Parts::Hash(c, _)
            | Parts::Radix(c, _)
            | Parts::Trie(c, _) => {
                self.validate_layout_by_nr(*c, visited, issues);
            }
            Parts::Index(c, _, left_field_nr) => {
                // Validate the record that carries the bookkeeping (`index_owner` —
                // the `Some` variant for a synth `__nullable<S>` element).  Also verify
                // the bookkeeping field index is in range and points at a `#left_*`
                // field, since `database.fields(tp)` reads
                // `fields[left_field_nr].position` and a wrong index
                // would silently corrupt tree::add's offsets.
                let owner = self.index_owner(*c);
                if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
                    &self.types[owner as usize].parts
                {
                    if (*left_field_nr as usize) >= fields.len() {
                        issues.push(format!(
                            "{}: Parts::Index left_field_nr={} out of range (content has {} fields)",
                            t.name,
                            left_field_nr,
                            fields.len()
                        ));
                    } else {
                        let f = &fields[*left_field_nr as usize];
                        if !f.name.starts_with("#left_") {
                            issues.push(format!(
                                "{}: Parts::Index left_field_nr={} points at field '{}' (expected '#left_*')",
                                t.name, left_field_nr, f.name
                            ));
                        }
                        // Bookkeeping must be 3 contiguous fields:
                        // #left (4B), #right (4B), #color (1B).  Their
                        // positions can be anywhere (alignment may
                        // reorder them) but tree::add reads them at
                        // [pos, pos+4, pos+8] — so the LAYOUT must
                        // place them contiguously.
                        if (*left_field_nr as usize + 2) < fields.len() {
                            let l = &fields[*left_field_nr as usize];
                            let r = &fields[*left_field_nr as usize + 1];
                            let cf = &fields[*left_field_nr as usize + 2];
                            if l.position != u16::MAX
                                && r.position != u16::MAX
                                && cf.position != u16::MAX
                            {
                                let mut had = false;
                                if r.position != l.position + 4 {
                                    had = true;
                                    issues.push(format!(
                                        "{}: tree bookkeeping not at expected offsets — '{}'@{} '{}'@{} (tree::add expects right at left+4=int4 but layout has left as {}-byte content '{}')",
                                        t.name,
                                        l.name,
                                        l.position,
                                        r.name,
                                        r.position,
                                        self.types[l.content as usize].size,
                                        self.types[l.content as usize].name,
                                    ));
                                }
                                if cf.position != l.position + 8 {
                                    had = true;
                                    issues.push(format!(
                                        "{}: tree bookkeeping not at expected offsets — '{}'@{} '{}'@{} (tree::add expects color at left+8 but layout disagrees)",
                                        t.name, l.name, l.position, cf.name, cf.position
                                    ));
                                }
                                if had {
                                    issues.push(format!(
                                        "  content layout: {}",
                                        self.layout_summary(*c)
                                    ));
                                }
                            }
                        }
                    }
                }
                self.validate_layout_by_nr(*c, visited, issues);
            }
            _ => {}
        }
    }

    /// Helper — render a type's full layout as a single line, used
    /// to make validate_layout errors self-contained.  Format:
    /// `Score(size=29,align=8){value:integer@0..8, ...}`.
    fn layout_summary(&self, tp: u16) -> String {
        let t = &self.types[tp as usize];
        let mut out = format!("{}(size={},align={})", t.name, t.size, t.align);
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) = &t.parts {
            out += "{";
            let mut by_pos: Vec<(u16, usize, &Field)> = fields
                .iter()
                .enumerate()
                .map(|(i, f)| (f.position, i, f))
                .collect();
            by_pos.sort_by_key(|(p, i, _)| (*p, *i));
            for (n, (pos, idx, f)) in by_pos.iter().enumerate() {
                if n > 0 {
                    out += ", ";
                }
                let csz = self.types[f.content as usize].size;
                let cname = &self.types[f.content as usize].name;
                let pos_str = if *pos == u16::MAX {
                    "?".to_string()
                } else {
                    pos.to_string()
                };
                let end = if *pos == u16::MAX || csz == u16::MAX {
                    "?".to_string()
                } else {
                    (*pos + csz).to_string()
                };
                use std::fmt::Write;
                let _ = write!(out, "[{}]{}:{}@{}..{}", idx, f.name, cname, pos_str, end);
            }
            out += "}";
        }
        out
    }

    /// Helper — detect overlapping field byte ranges.  Each issue
    /// includes the full layout summary so the user can see the
    /// surrounding context without a separate debug_layout dump.
    #[allow(clippy::similar_names)]
    fn check_fields_overlap(&self, tp: u16, fields: &[Field], issues: &mut Vec<String>) {
        let t_name = self.types[tp as usize].name.clone();
        let mut by_pos: Vec<(u16, &Field)> = fields
            .iter()
            .filter(|f| f.position != u16::MAX)
            .map(|f| (f.position, f))
            .collect();
        by_pos.sort_by_key(|(p, _)| *p);
        let mut had_issue = false;
        for window in by_pos.windows(2) {
            let (a_pos, a) = window[0];
            let (b_pos, b) = window[1];
            let a_end = u32::from(a_pos) + u32::from(self.types[a.content as usize].size);
            if u32::from(b_pos) < a_end {
                had_issue = true;
                issues.push(format!(
                    "{}: fields '{}' [@{}..{}) and '{}' [@{}..) overlap",
                    t_name, a.name, a_pos, a_end, b.name, b_pos,
                ));
            }
        }
        if had_issue {
            issues.push(format!("  layout: {}", self.layout_summary(tp)));
        }
    }

    /// Helper — verify every field's [pos, pos+size) fits within the
    /// type's reported size.  Issues include the full layout summary.
    fn check_fields_within_size(&self, tp: u16, fields: &[Field], issues: &mut Vec<String>) {
        let t = &self.types[tp as usize];
        if t.size == u16::MAX {
            return;
        }
        let t_name = t.name.clone();
        let t_size = t.size;
        let mut had_issue = false;
        for f in fields {
            if f.position == u16::MAX {
                had_issue = true;
                issues.push(format!(
                    "{}: field '{}' has no position (u16::MAX)",
                    t_name, f.name
                ));
                continue;
            }
            let csz = self.types[f.content as usize].size;
            if csz == u16::MAX {
                continue;
            }
            let end = u32::from(f.position) + u32::from(csz);
            if end > u32::from(t_size) {
                had_issue = true;
                issues.push(format!(
                    "{}: field '{}' [@{}..{}) extends beyond type size {}",
                    t_name, f.name, f.position, end, t_size
                ));
            }
        }
        if had_issue {
            issues.push(format!("  layout: {}", self.layout_summary(tp)));
        }
    }

    /// Helper for `debug_layout_by_nr` — prints fields sorted by
    /// position with content-type size info.  Reveals gaps + overlaps.
    fn debug_layout_fields(&self, out: &mut String, indent: usize, fields: &[Field]) {
        use std::fmt::Write;
        let pad = " ".repeat(indent);
        // Sort by position so layout is visually contiguous.
        let mut by_pos: Vec<(u16, usize, &Field)> = fields
            .iter()
            .enumerate()
            .map(|(i, f)| (f.position, i, f))
            .collect();
        by_pos.sort_by_key(|(p, i, _)| (*p, *i));
        let mut prev_end: i32 = -1;
        for (pos, idx, f) in by_pos {
            let csz = self.types[f.content as usize].size;
            let cname = &self.types[f.content as usize].name;
            // Show gap if this field's start > prev_end (when both
            // positions are real, not the u16::MAX "not laid out" sentinel).
            if prev_end >= 0 && pos != u16::MAX && i32::from(pos) > prev_end {
                let _ = writeln!(
                    out,
                    "{pad}     ── gap [{}..{}) ({} bytes) ──",
                    prev_end,
                    pos,
                    i32::from(pos) - prev_end
                );
            }
            let _ = writeln!(
                out,
                "{pad}field[{idx}] '{}' @{} size={} ({})",
                f.name, pos, csz, cname
            );
            if pos != u16::MAX && csz != u16::MAX {
                prev_end = i32::from(pos) + i32::from(csz);
            }
        }
    }

    pub fn vector(&mut self, content: u16) -> u16 {
        let name = if content == u16::MAX {
            "vector".to_string()
        } else {
            format!("vector<{}>", self.types[content as usize].name)
        };
        mint_trace(
            "vector",
            &name,
            self.names.get(&name).copied(),
            self.types.len(),
        );
        if let Some(nr) = self.names.get(&name) {
            *nr
        } else {
            let num = self.types.len() as u16;
            self.types.push(Type::data(&name, Parts::Vector(content)));
            self.names.insert(name, num);
            num
        }
    }

    /// P213: register a `Parts::ChildRec(content)` type — a 4-byte u32
    /// rec-id pointing at a child record co-located in the same Store
    /// as the host.  Used by capturing-closure-in-struct-field codegen
    /// to embed a closure record's rec-id directly in the host without
    /// vector-header overhead.  Cascade is automatic via the
    /// `copy_claims` / `remove_claims` `Parts::ChildRec` arms.
    pub fn child_rec(&mut self, content: u16) -> u16 {
        let name = if content == u16::MAX {
            "child_rec".to_string()
        } else {
            format!("child_rec<{}>", self.types[content as usize].name)
        };
        if let Some(nr) = self.names.get(&name) {
            *nr
        } else {
            let num = self.types.len() as u16;
            self.types.push(Type::data(&name, Parts::ChildRec(content)));
            self.names.insert(name, num);
            num
        }
    }

    pub fn hash(&mut self, content: u16, key: &[String]) -> u16 {
        // Display name uses `content` (e.g. `hash<__nullable<Count>[t]>`), but
        // key fields resolve against `key_owner` — the `Some` payload for a
        // synth nullable element (@PLN25 E2).
        let owner = self.key_owner(content);
        let mut name = "hash<".to_string() + &self.types[content as usize].name + "[";
        let mut key_nrs = Vec::new();
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &self.types[owner as usize].parts
        {
            for (k_nr, k) in key.iter().enumerate() {
                if k_nr > 0 {
                    name += ",";
                }
                name += k;
                for (f_nr, f) in fields.iter().enumerate() {
                    if f.name == *k {
                        key_nrs.push(f_nr as u16);
                    }
                }
            }
        }
        name += "]>";
        mint_trace(
            "hash",
            &name,
            self.names.get(&name).copied(),
            self.types.len(),
        );
        if let Some(nr) = self.names.get(&name) {
            *nr
        } else {
            let num = self.types.len() as u16;
            self.types
                .push(Type::data(&name, Parts::Hash(content, key_nrs)));
            self.names.insert(name, num);
            num
        }
    }

    pub fn spatial(&mut self, content: u16, key: &[String]) -> u16 {
        let mut name = "spatial<".to_string() + &self.types[content as usize].name + "[";
        let key_nrs = self.field_name(content, key, &mut name);
        if let Some(nr) = self.names.get(&name) {
            *nr
        } else {
            let num = self.types.len() as u16;
            self.types
                .push(Type::data(&name, Parts::Radix(content, key_nrs)));
            self.names.insert(name, num);
            num
        }
    }

    /// Register a `trie<T[k]>` type — the `spatial` sibling, over ONE text key.
    ///
    /// Takes a single key name rather than a slice: `Parts::Trie` holds one `u16`,
    /// so a two-key trie is unrepresentable rather than rejected.
    pub fn trie(&mut self, content: u16, key: &str) -> u16 {
        let mut name = "trie<".to_string() + &self.types[content as usize].name + "[";
        let key_nrs = self.field_name(content, std::slice::from_ref(&key.to_string()), &mut name);
        let Some(&k) = key_nrs.first() else {
            return u16::MAX;
        };
        if let Some(nr) = self.names.get(&name) {
            *nr
        } else {
            let num = self.types.len() as u16;
            self.types.push(Type::data(&name, Parts::Trie(content, k)));
            self.names.insert(name, num);
            num
        }
    }

    /// Resolve a keyed collection's key NAMES to field numbers on its element, appending the
    /// rendered key list to `name`.
    ///
    /// Keys resolve against [`Self::key_owner`], not against `content` itself: a synth
    /// `__nullable<S>` element keeps S's keys inside the `Some` variant's inline payload, so
    /// indexing the enum's own field list finds none of them.  Same rule as [`Self::hash`] and
    /// [`Self::create_key`] — one question, one answer, whichever kind asks it.
    pub fn field_name(&self, content: u16, key: &[String], name: &mut String) -> Vec<u16> {
        let owner = self.key_owner(content);
        let mut key_nrs = Vec::new();
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &self.types[owner as usize].parts
        {
            for (k_nr, k) in key.iter().enumerate() {
                if k_nr > 0 {
                    *name += ",";
                }
                *name += k;
                for (f_nr, f) in fields.iter().enumerate() {
                    if f.name == *k {
                        key_nrs.push(f_nr as u16);
                    }
                }
            }
        }
        *name += "]>";
        key_nrs
    }

    #[must_use]
    pub fn field_nr(&self, record: u16, position: i32) -> u16 {
        if record == u16::MAX {
            // Should normally only occur in the first_phase of the parser.
            return 0;
        }
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &self.types[record as usize].parts
        {
            for (f_nr, f) in fields.iter().enumerate() {
                if f.position == position as u16 {
                    return f_nr as u16;
                }
            }
        }
        0
    }

    /**
    Keys with field number and ascending flag.
    */
    pub fn sorted(&mut self, content: u16, key: &[(String, bool)]) -> u16 {
        let mut name = "sorted<".to_string() + &self.types[content as usize].name + "[";
        let key_nrs = self.create_key(content, key, &mut name);
        mint_trace(
            "sorted",
            &name,
            self.names.get(&name).copied(),
            self.types.len(),
        );
        if let Some(nr) = self.names.get(&name) {
            *nr
        } else {
            let num = self.types.len() as u16;
            self.types
                .push(Type::new(&name, Parts::Sorted(content, key_nrs), 4));
            self.names.insert(name, num);
            num
        }
    }

    pub fn index(&mut self, content: u16, key: &[(String, bool)]) -> u16 {
        let mut name = "index<".to_string() + &self.types[content as usize].name + "[";
        let key_nrs = self.create_key(content, key, &mut name);
        // Dedup early.  Post-Category-D native codegen can call `db.index`
        // for the same content/keys combination twice (once in Phase 1a's
        // bare-io registration, once more during struct-field emission).
        // The field appending below must run exactly ONCE per unique
        // index type — otherwise the content struct accumulates stale
        // `#left_N / #right_N / #color_N` triples that push real user
        // fields to unexpected positions and break tree traversal.
        mint_trace(
            "index",
            &name,
            self.names.get(&name).copied(),
            self.types.len(),
        );
        if let Some(nr) = self.names.get(&name) {
            return *nr;
        }
        // P191: bookkeeping fields must be 4-byte ints to match
        // tree::add's hardcoded RB_LEFT=0 / RB_RIGHT=4 offsets, which
        // use set_i32_raw / get_i32_raw exclusively.  Using 8-byte
        // `integer` here makes alignment-aware packing place them 8
        // bytes apart, corrupting tree::add's writes.
        let int4 = self.int(0, false);
        let bool_c = self.name("boolean");
        // The tree node is the stored RECORD, which for a synth `__nullable<S>` element is the
        // `Some` variant rather than the enum — see `index_owner`.
        let host = self.index_owner(content);
        let mut nr = 1;
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &self.types[host as usize].parts
        {
            for f in fields {
                if f.name.starts_with("#left_") {
                    nr += 1;
                }
            }
        }
        let left = if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &mut self.types[host as usize].parts
        {
            let left = fields.len();
            fields.push(Field {
                name: format!("#left_{nr}"),
                content: int4,
                position: 0,
                default: None,
                nullable: false,
                other_indexes: Vec::new(),
            });
            fields.push(Field {
                name: format!("#right_{nr}"),
                content: int4,
                position: 0,
                default: None,
                nullable: false,
                other_indexes: Vec::new(),
            });
            fields.push(Field {
                name: format!("#color_{nr}"),
                content: bool_c,
                position: 0,
                default: None,
                nullable: false,
                other_indexes: Vec::new(),
            });
            left as u16
        } else {
            u16::MAX
        };
        // Register the bookkeeping triple as a linked field group on
        // the content type — `[left, left+1, left+2] = [#left_N,
        // #right_N, #color_N]` for index instance N.  Used by codegen
        // / runtime to walk index bookkeeping without string-prefix
        // matching on field names.
        //
        // Alignment is 4 (max of int4 align 4, int4 align 4, bool align 1).
        // Size is 9 bytes (4 + 4 + 1; no internal padding — bool is last
        // and 1-byte aligned).
        if left != u16::MAX {
            let int4_size = self.types[int4 as usize].size;
            let int4_align = self.types[int4 as usize].align;
            let bool_size = self.types[bool_c as usize].size;
            let bool_align = self.types[bool_c as usize].align;
            let members = [
                (int4_size, int4_align),
                (int4_size, int4_align),
                (bool_size, bool_align),
            ];
            let alignment = crate::data::LinkedFieldGroup::group_alignment(
                &members.iter().map(|&(_, a)| a).collect::<Vec<_>>(),
            );
            let size = crate::data::LinkedFieldGroup::group_size(&members);
            self.types[host as usize]
                .field_groups
                .push(crate::data::LinkedFieldGroup {
                    kind: crate::data::LinkedFieldKind::Index,
                    instance: nr,
                    field_indices: vec![left, left + 1, left + 2],
                    alignment,
                    size,
                });
        }
        let num = self.types.len() as u16;
        self.types
            .push(Type::new(&name, Parts::Index(content, key_nrs, left), 4));
        self.names.insert(name, num);
        num
    }

    /// Register a Tuple LinkedFieldGroup on `tp` whose members are the
    /// attribute indices `members` (in element order).  Used by the native
    /// codegen's `init()` emission to mirror the parse-time propagation in
    /// `typedef.rs::fill_database` (line 567-570) — the generated runtime
    /// would otherwise rebuild the tuple type WITHOUT its group metadata
    /// and `finish_type` would fall back to the simple alignment-descending
    /// packer, producing positions/size that diverge from the compile-side
    /// layout the IR was emitted against (PLAN51 Cluster V-a).
    ///
    /// `alignment` / `size` placeholders are recomputed by `finish_type`
    /// from member storage widths (see `groups_descriptor` construction
    /// at lines 304-322); the stored values on `LinkedFieldGroup` are
    /// never read by the storage-layout routine.
    pub fn add_tuple_group(&mut self, tp: u16, members: &[u16]) {
        self.types[tp as usize]
            .field_groups
            .push(crate::data::LinkedFieldGroup {
                kind: crate::data::LinkedFieldKind::Tuple,
                instance: 0,
                field_indices: members.to_vec(),
                alignment: 0,
                size: 0,
            });
    }

    /// Render a keyed collection's key field NUMBERS back to the bracketed name list — the
    /// inverse of [`Self::create_key`], and it indexes the same field list ([`Self::key_owner`]).
    ///
    /// The rendered list is part of the type NAME, and a type name is the dedup key, so a
    /// collection whose keys render differently in two places is two runtime types rather than
    /// one.
    pub(super) fn key_name(&mut self, content: u16, key: &[(u16, bool)], name: &mut String) {
        let owner = self.key_owner(content);
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &self.types[owner as usize].parts
        {
            for (k_nr, (k, asc)) in key.iter().enumerate() {
                if k_nr > 0 {
                    *name += ",";
                }
                if !*asc {
                    *name += "-";
                }
                *name += &fields[*k as usize].name;
            }
        }
        *name += "]>";
    }

    pub(super) fn create_key(
        &mut self,
        content: u16,
        key: &[(String, bool)],
        name: &mut String,
    ) -> Vec<(u16, bool)> {
        let owner = self.key_owner(content);
        let mut key_nrs = Vec::new();
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &self.types[owner as usize].parts
        {
            for (k_nr, (k, asc)) in key.iter().enumerate() {
                if k_nr > 0 {
                    *name += ",";
                }
                if !*asc {
                    *name += "-";
                }
                *name += k;
                for (f_nr, f) in fields.iter().enumerate() {
                    if f.name == *k {
                        key_nrs.push((f_nr as u16, *asc));
                    }
                }
            }
        }
        *name += "]>";
        key_nrs
    }

    pub fn byte(&mut self, min: i32, nullable: bool) -> u16 {
        let name = narrow_part_name(crate::data::NarrowIntKind::Byte, min, nullable);
        if let Some(nr) = self.names.get(&name) {
            *nr
        } else {
            let num = self.types.len() as u16;
            self.types
                .push(Type::new(&name, Parts::Byte(min, nullable), 1));
            self.names.insert(name, num);
            num
        }
    }

    /**
    Retrieve a defined type number by name.
    # Panics
    When a type name doesn't exist.
    */
    #[must_use]
    pub fn name(&self, name: &str) -> u16 {
        *self.names.get(name).unwrap_or(&u16::MAX)
    }

    /// Rename a type and keep the name INDEX in step.
    ///
    /// `finish_type` renames a `vector<T>` to `array<T>` and a `sorted<T[k]>` to
    /// `ordered<T[k]>` when the element is held by a keyed collection elsewhere.
    /// Writing `types[n].name` alone left `names` pointing at the OLD spelling
    /// only, so reflection reported `array<Tag>` as a field's type name and
    /// `type_named("array<Tag>")` answered null — a name the API hands out and
    /// then cannot resolve. The declared spelling keeps working, because a
    /// program that wrote `vector<Tag>` should still find it by that.
    fn rename_type(&mut self, t_nr: u16, name: String) {
        if self.types[t_nr as usize].name == name {
            return;
        }
        self.types[t_nr as usize].name.clone_from(&name);
        self.names.insert(name, t_nr);
    }

    /// The number of registered types — pair with [`rollback_types_to`] to undo
    /// the schema a throwaway parse registered.
    ///
    /// [`rollback_types_to`]: Self::rollback_types_to
    #[must_use]
    pub fn types_len(&self) -> u16 {
        self.types.len() as u16
    }

    /// Report a type this generated program placed at a different id than the
    /// compiler did — the drift that silently renames every id after it.
    ///
    /// A `--native` program builds its schema by REPLAYING registration calls
    /// from `init()`, while the type ids it operates on (`OpReadFile`'s
    /// `db_tp`, every keyed-collection id) were baked in as plain integers at
    /// compile time. That only works while both orders agree, and nothing used
    /// to check that they did: a single type created one position early or late
    /// shifted every id after it, so a `db_tp` silently resolved to a
    /// neighbouring type. The failures that produced were unattributable —
    /// `f#read as u16` returning null with no error, and a keyed lookup
    /// aborting with `find called on non-collection type` naming whichever type
    /// happened to sit at the shifted id, in code the program never called
    /// (loft#739).
    ///
    /// `expected` is the compile-time table's names in index order.
    ///
    /// What counts as a divergence is deliberately narrow: a type the compiler
    /// placed at `i` that this table holds at some **other** index `j`. That is
    /// a genuine shift — the type exists in both, at two different ids, so
    /// every id baked from that point on is off.
    ///
    /// A name the runtime table does not hold ANYWHERE is not reported. Several
    /// registration calls render a slot under a different name than the
    /// compiler's table did — `db.sorted` registers `sorted<Rec[id]>` where the
    /// compiler recorded `ordered<Rec[id]>`, `db.vector` registers
    /// `vector<Definition>` for `array<Definition>` — without moving anything.
    /// Comparing names position-by-position flags all of those, and they are not
    /// what corrupts an id. (It does mean this check cannot see a type the
    /// generated program fails to create at all; that would need a shape
    /// comparison rather than a name one.) A name the COMPILER's table holds
    /// twice — `Format` under prelude shadowing — is skipped as well: the
    /// name→id map keeps only the last, so such a name cannot say where it sits.
    ///
    /// Reports and CONTINUES. A shift means some ids are wrong, but not
    /// necessarily any the program actually uses — several drifts predate this
    /// check and produce correct output today — so aborting would fail programs
    /// that work. Set `LOFT_STRICT_SCHEMA_IDS` to make it fatal instead, which
    /// is what you want while hunting one of these.
    ///
    /// # Panics
    /// Only under `LOFT_STRICT_SCHEMA_IDS`, on the first shifted type.
    pub fn verify_schema_ids(&self, expected: &[&str]) {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut twice: HashSet<&str> = HashSet::new();
        for want in expected {
            if !seen.insert(want) {
                twice.insert(want);
            }
        }
        for (i, want) in expected.iter().enumerate() {
            let Some(&at) = self.names.get(*want) else {
                continue;
            };
            if usize::from(at) == i || twice.contains(want) {
                continue;
            }
            let held = self.types.get(i).map_or("<missing>", |t| t.name.as_str());
            let msg = format!(
                "generated schema diverges from the compiler's: {want:?} is type \
                 id {at} here but {i} in the compiler, so ids baked at or past {} \
                 can name the wrong type — that one currently reads as {held:?} \
                 (loft#739). This is a codegen bug in the `init()` emission order.",
                i.min(usize::from(at)),
            );
            assert!(
                std::env::var_os("LOFT_STRICT_SCHEMA_IDS").is_none(),
                "{msg}"
            );
            crate::loft_eprintln!("loft: {msg}");
            // One report is the diagnosis; the rest of the table is downstream
            // of the same shift and would only bury it.
            return;
        }
    }

    /// A cheap total summary of the registered schema — `(type count, name
    /// count, order-independent hash of every `name → nr` pair)`.
    ///
    /// The oracle for schema neutrality: an operation that is supposed to leave
    /// the schema untouched (a rolled-back REPL capture generation, a
    /// speculative parse) must return the SAME fingerprint it started with.
    /// Comparing counts alone would miss a rollback that removed one name and
    /// added another, so the hash covers the mapping itself; XOR keeps it
    /// independent of `HashMap` iteration order.
    #[must_use]
    pub fn schema_fingerprint(&self) -> (u16, u32, u64) {
        use std::hash::{Hash, Hasher};
        let mut acc: u64 = 0;
        for (name, &nr) in &self.names {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            name.hash(&mut h);
            nr.hash(&mut h);
            acc ^= h.finish();
        }
        (self.types.len() as u16, self.names.len() as u32, acc)
    }

    /// Drop every type registered at or after `keep`, the schema-side twin of
    /// `Data::rollback_to` (#618).
    ///
    /// A REPL value-capture parses a synthetic program into the session's
    /// `Data` **and** its `Stores`, then rolls the `Data` back.  Rolling back
    /// only half left the schema holding names whose definitions no longer
    /// exist, so the next capture that needed the same synthetic wrapper
    /// re-registered the name and `structure` aborted with "Double structure
    /// type" — reachable whenever a capture's return type has no pre-existing
    /// `main_vector<…>` wrapper (e.g. an element type wide enough to carry a
    /// range: `vector<integer(-2147483647, 4294967295)>`).
    ///
    /// Sound only because the two rollbacks are paired: a type registered by
    /// that parse is referenced only by definitions the same parse added.
    pub fn rollback_types_to(&mut self, keep: u16) {
        if usize::from(keep) >= self.types.len() {
            return;
        }
        for (name, &nr) in &self.names {
            if nr >= keep {
                schema_trace("rollback", name, nr);
            }
        }
        self.types.truncate(keep as usize);
        self.names.retain(|_, &mut nr| nr < keep);
    }

    pub fn short(&mut self, min: i32, nullable: bool) -> u16 {
        let name = narrow_part_name(crate::data::NarrowIntKind::Short, min, nullable);
        if let Some(nr) = self.names.get(&name) {
            *nr
        } else {
            let num = self.types.len() as u16;
            self.types
                .push(Type::new(&name, Parts::Short(min, nullable), 2));
            self.names.insert(name, num);
            num
        }
    }

    /// 2-byte narrow vector-element field type, used for
    /// `vector<u16>` / `vector<i16>` / `vector<integer limit(...) size(2)>`.
    /// Stored raw (no `+1` shift) so `vector_add`'s raw-byte copy works
    /// unchanged.  `i16::MIN` reserved as the null sentinel.  Struct
    /// fields with `u16` / `i16` continue to use `Parts::Short` (the
    /// legacy `+1` encoding with raw=0 null sentinel).
    pub fn short_raw(&mut self, min: i32, nullable: bool) -> u16 {
        let name = narrow_part_name(crate::data::NarrowIntKind::ShortRaw, min, nullable);
        if let Some(nr) = self.names.get(&name) {
            *nr
        } else {
            let num = self.types.len() as u16;
            self.types
                .push(Type::new(&name, Parts::ShortRaw(min, nullable), 2));
            self.names.insert(name, num);
            num
        }
    }

    /// 4-byte integer field type, used for `pub type T = integer size(4);`
    /// subtypes (e.g. `i32`).  Stored raw (no +1 shift); `i32::MIN` reserved
    /// as the null sentinel.  Stack values stay 8-byte i64 — narrowing
    /// happens at the field boundary via OpSetInt4/OpGetInt4.
    pub fn int(&mut self, min: i32, nullable: bool) -> u16 {
        let name = narrow_part_name(crate::data::NarrowIntKind::Int4, min, nullable);
        if let Some(nr) = self.names.get(&name) {
            *nr
        } else {
            let num = self.types.len() as u16;
            self.types
                .push(Type::new(&name, Parts::Int(min, nullable), 4));
            self.names.insert(name, num);
            num
        }
    }

    /// 4-byte UNSIGNED integer field type — a range that is non-negative and runs past
    /// `i32::MAX` (`u32`).  Stored raw, no shift; `u32::MAX` reserved as the null
    /// sentinel, which is why `u32` is declared `limit(0, 4294967294)`.
    ///
    /// The 4-byte twin of [`Stores::short_raw`].  A slot picks between this and
    /// [`Stores::int`] through [`crate::data::NarrowIntKind::part`], never by re-asking
    /// the range at the mint site.
    pub fn int_raw(&mut self, min: i32, nullable: bool) -> u16 {
        let name = narrow_part_name(crate::data::NarrowIntKind::Int4Raw, min, nullable);
        if let Some(nr) = self.names.get(&name) {
            *nr
        } else {
            let num = self.types.len() as u16;
            self.types
                .push(Type::new(&name, Parts::IntRaw(min, nullable), 4));
            self.names.insert(name, num);
            num
        }
    }

    /// Plan-06 phase 4d.C step 2 — register the 12-byte raw `DbRef` storage
    /// shape (store_nr u32 + rec u32 + pos u32). Used for the closure half of
    /// `Type::Function = (u32, DbRef)` slots stored in vectors / fields.
    ///
    /// Plan-22 phase 02c (2026-05-12): override the alignment from the
    /// default (= size = 12) to 4.  The DbRef is 3 contiguous u32
    /// words; natural alignment is 4 bytes.  `calc::calculate_positions`
    /// only knows about alignments {8, 4, 2, 1} — a field with align=12
    /// gets silently skipped, leaving its position at u16::MAX and
    /// the containing struct's size/align both 0.  Symptom: phase 02c's
    /// auto-Reference closure record fails with `field 's' has no
    /// position (u16::MAX)` until align=4.
    pub fn dbref(&mut self) -> u16 {
        self.dbref_shapes().0
    }

    /// #682 — the BORROWED sibling of [`Stores::dbref`]: byte-for-byte the same
    /// 12-byte `DbRef`, registered under its own name so `free_named`'s
    /// closure-record cascade can tell "a store this record adopted" from "a
    /// store it only points at".  Same shape means every read/write path and the
    /// layout descriptor are unchanged; only the free decision differs (see
    /// [`crate::data::Deps::borrowed_share_sentinel`]).
    pub fn dbref_borrow(&mut self) -> u16 {
        self.dbref_shapes().1
    }

    /// Register BOTH 12-byte `DbRef` storage shapes, owned first, and return
    /// their numbers.
    ///
    /// The pair is registered together, from either entry point, because type
    /// numbers are POSITIONAL and `--native` replays the registration sequence to
    /// rebuild the schema: its generated `init()` refers to every other type by
    /// the compile-time `tN` it had here, so a shape that appears only in some
    /// programs would shift every id after it and the replay would register
    /// fields against the wrong types.  Registering both unconditionally keeps
    /// the numbering independent of which captures turn out to be borrowed — a
    /// verdict that is not even known until scope analysis has run.
    fn dbref_shapes(&mut self) -> (u16, u16) {
        if let (Some(&owned), Some(&borrow)) =
            (self.names.get(DBREF_OWNED), self.names.get(DBREF_BORROW))
        {
            return (owned, borrow);
        }
        let register = |s: &mut Self, name: &str| -> u16 {
            if let Some(&nr) = s.names.get(name) {
                return nr;
            }
            let num = s.types.len() as u16;
            let mut tp = Type::new(name, Parts::DbRef, 12);
            tp.align = 4;
            s.types.push(tp);
            s.names.insert(name.to_string(), num);
            num
        };
        let owned = register(self, DBREF_OWNED);
        let borrow = register(self, DBREF_BORROW);
        (owned, borrow)
    }

    /// Is `content` the 12-byte `DbRef` shape a closure record ADOPTS — i.e. may
    /// `free_named`'s cascade reclaim the store this field points at?  False for
    /// the borrowed shape and for every non-`DbRef` field.
    #[must_use]
    pub(crate) fn dbref_is_adopted(&self, content: u16) -> bool {
        let tp = &self.types[content as usize];
        matches!(tp.parts, Parts::DbRef) && tp.name != DBREF_BORROW
    }

    /// #682 — re-point field `field_name` of struct type `tp` at the BORROWED
    /// 12-byte `DbRef` shape.  Layout-preserving by construction (both shapes are
    /// 12 bytes at align 4), which is why this may run after `finish()` has
    /// positioned the record; it changes only the cascade's free decision.
    /// Returns whether the field was found.
    pub(crate) fn borrow_dbref_field(&mut self, tp: u16, field_name: &str) -> bool {
        let borrow = self.dbref_borrow();
        let Parts::Struct(fields) = &mut self.types[tp as usize].parts else {
            return false;
        };
        if let Some(f) = fields.iter_mut().find(|f| f.name == field_name) {
            f.content = borrow;
            true
        } else {
            false
        }
    }

    pub fn enumerate(&mut self, name: &str) -> u16 {
        let num = self.types.len() as u16;
        self.types
            .push(Type::new(name, Parts::Enum(Vec::new()), u16::MAX));
        self.names.insert(name.to_string(), num);
        num
    }

    pub fn enum_value(&mut self, enum_tp: u16, value_name: &str, value_tp: u16) {
        // B2 guard: a caller that hasn't yet run type-resolution on the parent
        // enum may pass `u16::MAX` here (known_type unset).  Returning without
        // panicking lets the later passes recover; the missing variant-type
        // link surfaces as a normal type-check error downstream rather than
        // an `index out of bounds` crash in the allocator.
        if enum_tp == u16::MAX || (enum_tp as usize) >= self.types.len() {
            return;
        }
        if let Parts::Enum(variants) = &mut self.types[enum_tp as usize].parts {
            for variant in variants.iter_mut() {
                if variant.1 == value_name {
                    variant.0 = value_tp;
                }
            }
        }
    }

    pub fn db_type(&mut self, tp: &crate::data::Type, data: &crate::data::Data) -> u16 {
        match tp {
            crate::data::Type::Integer(IntegerSpec {
                min: minimum,
                not_null,
                ..
            }) => {
                let nullable = !not_null;
                let s = tp.size(nullable);
                if s == 1 {
                    self.byte(*minimum, nullable)
                } else if s == 2 {
                    self.short(*minimum, nullable)
                } else {
                    self.name("integer")
                }
            }
            crate::data::Type::Enum(_, false, _) => self.name("byte"),
            // #250: a nested-vector element type must resolve recursively — the
            // `_` fallback below (`def(type_def_nr(tp)).known_type`) mis-resolves
            // `vector<T>` to an unrelated default-library id (e.g. FieldValue),
            // so a 3+-deep `vector<vector<vector<X>>>` copy got a bogus type-id
            // and `copy_claims` dispatched as the wrong type → OOB panic.
            //
            // The element id comes from `Data::vector_element_type` — the one
            // derivation of that fact, shared with the parser's `vector_of` and
            // `typedef.rs::fill_database`.  Recursing through `db_type` instead
            // re-entered the Integer arm below, which knows nothing of the
            // element-side narrow forms (no `4 → int`, no `ShortRaw`) and so
            // widened every narrow element to 8-byte `integer` (loft#624 nested).
            crate::data::Type::Vector(elem, _) => {
                let e = match data.vector_element_type(elem, self) {
                    Some(e) => e,
                    None => self.db_type(elem, data),
                };
                self.vector(e)
            }
            _ => data.def(data.type_def_nr(tp)).known_type,
        }
    }

    /**
    Add a value to an enumerated type.
    # Panics
    When adding a value to a non-enumerated variable.
    */
    pub fn value(&mut self, known_type: u16, name: &str, value_type: u16) -> u16 {
        if let Parts::Enum(values) = &mut self.types[known_type as usize].parts {
            let num = values.len() as u16;
            values.push((value_type, name.to_string()));
            num
        } else {
            panic!(
                "Adding a value to a non enum type {}",
                self.types[known_type as usize].name
            );
        }
    }

    /// Does this enum discriminant name a VARIANT, or is it an ABSENCE?
    ///
    /// One home for a predicate that has **two** null bytes, which is why restating it
    /// keeps going wrong.  Both reach a program: an explicit null writes `255`
    /// (`OpConvEnumFromNull`), while zero-init storage and `OpGetEnum` on an absent
    /// record produce `0` — `default/01_code.loft` spells the pair as
    /// `OpConvBoolFromEnum`'s `@v1 != 255 && @v1 != 0`, and [`enum_val`](Self::enum_val)
    /// already answers `"null"` for both.  A reader that restates it as `disc == 0`
    /// therefore renders 255 through `enum_val`'s own fallback and prints `Col.null`
    /// — a variant path taken for a value that has no variant (loft#1459).  `255` is
    /// safe to reject because it is not a variant of any enum, and neither is `0`
    /// (variants are numbered from 1).
    #[must_use]
    pub const fn enum_is_null(disc: u8) -> bool {
        disc == 0 || disc == u8::MAX
    }

    #[must_use]
    pub fn enum_val(&self, known_type: u16, value: u8) -> &str {
        if known_type == u16::MAX {
            return "unknown";
        }
        if let Parts::Enum(values) = &self.types[known_type as usize].parts
            && value > 0
            && (value as usize) <= values.len()
        {
            return &values[value as usize - 1].1;
        }
        "null"
    }

    /// The JSON rendering of a payload-less variant: the name as a quoted
    /// string, or bare `null` for the absent discriminant (already valid JSON).
    ///
    /// One home for that rule. A value enum is reached two ways — as a scalar
    /// (`OpCastTextFromEnum`, when it is a plain local) and through a record
    /// (`ShowDb`'s enum arm, when it is a field or an element) — and the two
    /// rendered the same value differently until both asked here (loft#768).
    #[must_use]
    pub fn enum_val_json(&self, known_type: u16, value: u8) -> String {
        let name = self.enum_val(known_type, value);
        if name == "null" {
            name.to_string()
        } else {
            format!("\"{name}\"")
        }
    }

    #[must_use]
    pub fn to_enum(&self, known_type: u16, value: &str) -> u8 {
        if let Parts::Enum(values) = &self.types[known_type as usize].parts {
            for (idx, val) in values.iter().enumerate() {
                if val.1 == value {
                    return 1 + idx as u8;
                }
            }
        }
        0u8
    }

    #[must_use]
    pub fn is_null(
        &self,
        store: &crate::store::Store,
        rec: u32,
        pos: u32,
        known_type: u16,
    ) -> bool {
        if rec == 0 {
            return true;
        }
        if known_type <= 6 {
            match known_type {
                0 => store.get_int(rec, pos) == i64::MIN,
                // `character` — 4 bytes, and its in-band sentinel is CODEPOINT 0, which
                // `formal/types.md` reserves even for a non-null slot (`0 as character`
                // reads null) and loft#1014 made every other site agree on.
                //
                // This arm sat under `known_type < 6` and so could never run, and the
                // `u32::MAX` it tested was not the sentinel either — two ways of being
                // wrong that cancelled into "a character slot is never null".  The write
                // side puts 0 there, so an ABSENT character then rendered as a value: a
                // struct printed `a:' '` and `to_json()` put a SPACE on the wire where
                // the field should have been omitted, while `x.a == null` answered true.
                // One absent value, two answers.
                6 => store.get_u32_raw(rec, pos) == 0,
                1 => store.get_long(rec, pos) == i64::MIN,
                2 => store.get_single(rec, pos).is_nan(),
                3 => store.get_float(rec, pos).is_nan(),
                4 => store.get_byte(rec, pos, 0) > 1,
                5 => {
                    // @FR-L-Null-Text — text nullity is CONTENT-based, and `Store::text_is_null`
                    // is its one home: an unset handle and an allocated `STRING_NULL` record are
                    // one absence, so a field written `t: null` is omitted exactly where a parsed
                    // absent one is.  @P375 still holds inside it — an allocated `""` is a present
                    // value and must serialise as `""`.
                    store.text_is_null(rec, pos)
                }
                _ => false,
            }
        } else if let Parts::Enum(_) = &self.types[known_type as usize].parts {
            store.get_byte(rec, pos, 0) == 0
        } else if let Parts::Struct(_) | Parts::EnumValue(_, _) =
            &self.types[known_type as usize].parts
        {
            rec == 0
        } else if matches!(
            &self.types[known_type as usize].parts,
            Parts::Vector(_)
                | Parts::Sorted(_, _)
                | Parts::Array(_)
                | Parts::Ordered(_, _)
                | Parts::Hash(_, _)
                | Parts::Index(_, _, _)
                | Parts::Radix(_, _)
        ) {
            // @P375's rule still holds and is the first half: zero is the EMPTY
            // collection, so an unset handle must serialise as `[]` rather than be
            // dropped — that is what keeps a save→load round-trip whole.
            //
            // Its premise — *"a collection field carries no nullable flag, so it is
            // always at least the empty list, never null"* — expired.  @PLN25 gave a
            // collection field a declared `?`, and `mark_collection_absent` writes
            // `DbRef::ABSENT_REC` into the handle for it (loft#917), which is a THIRD
            // state beside "empty" and "populated".  Answering `false` for it put the
            // renderer one step behind every other reader: `xs == null` answered true
            // while `{x:j}` wrote `[]` for a vector (so the null did not survive its own
            // round trip) and PANICKED for a keyed kind, dereferencing `4294967295` as a
            // record number.
            //
            // `vector::is_absent_collection` is the one home for that test; the raw read
            // here is its slot-addressed half, and it cannot confuse the two zeros
            // because ABSENT_REC is neither.
            store.get_u32_raw(rec, pos) == crate::keys::DbRef::ABSENT_REC
        } else {
            // A narrow integer answers from its own home; anything else is not nullable
            // in a way this walk can see.
            self.narrow_is_null(store, rec, pos, known_type)
                .unwrap_or_default()
        }
    }

    /// Is the NARROW-integer slot at `(rec, pos)` holding its null code?  `None` when
    /// `known_type` is not a narrow integer, so a caller can fall through to its own
    /// dispatch.
    ///
    /// One home for the four widths, because the read side and the write side have to
    /// agree about a code that is invisible in the bytes: `Byte` reserves the raw code
    /// `255`, `Short`'s `+1` encoding reserves the raw code `0` (which `get_short` maps
    /// to `i32::MIN`), `ShortRaw` reserves the top code, and `Int` is a raw `i32` whose
    /// null is `i32::MIN`.
    ///
    /// ⚠ A width with no arm here does not fail loudly — it reads as NOT NULL.  An absent
    /// field then survives into output as its sentinel NUMBER (an absent `i32?` renders as
    /// `-2147483648`) while the same slot read through this function answers null, so the
    /// two disagree with nothing to report.  That asymmetry is why all four widths live in
    /// one function rather than as inline arms per caller: [STABILITY_REDFLAGS] Cluster D
    /// names the class — *"one width-fact, N drifting copies"*.
    ///
    /// Enforces @FR-L-Null for the narrow widths — the read twin of `write_narrow_value`.
    ///
    /// Write twins: [`Self::write_narrow_value`] for a value, and
    /// `set_default_value_nullable`'s narrow arms for the absent code.
    ///
    /// [STABILITY_REDFLAGS]: ../../../doc/claude/STABILITY_REDFLAGS.md
    #[must_use]
    pub fn narrow_is_null(
        &self,
        store: &crate::store::Store,
        rec: u32,
        pos: u32,
        known_type: u16,
    ) -> Option<bool> {
        if known_type == u16::MAX || known_type <= 6 {
            return None;
        }
        match self.types[known_type as usize].parts {
            // The sentinel is the RAW stored code, so read it with NO offset: `get_byte`
            // answers `stored + min`, and testing THAT against the sentinel only holds
            // when `min` is 0.  A `limit(10, 255)?` slot answered `265 == 255` — never
            // true — so its null rendered as the number 265.
            Parts::Byte(_, nullable) => Some(nullable && store.get_byte(rec, pos, 0) == 255),
            // The `+1` encoding reserves the stored code 0 for null, and `get_short`
            // maps that to `i32::MIN`.  An older test (`== 65535`) could not fire at all,
            // so every nullable 2-byte slot rendered its null as `-2147483648`.
            Parts::Short(_, nullable) => Some(nullable && store.get_short(rec, pos, 0) == i32::MIN),
            // The direct encoding reserves the top code.  `get_i16_raw` answers
            // `stored + min`, never `i32::MIN`, so that test could not fire either.
            Parts::ShortRaw(_, nullable) => {
                Some(nullable && store.get_short_full(rec, pos, 0) == i32::from(u16::MAX))
            }
            // The width that had no arm: a raw `i32` whose reserved code is `i32::MIN`,
            // which is also exactly what the write side puts there.
            Parts::Int(_, nullable) => Some(nullable && store.get_i32_raw(rec, pos) == i32::MIN),
            // The unsigned 4-byte encoding reserves the TOP code, as `ShortRaw` does one
            // width down — `i32::MIN` is the legal value 2147483648 here.
            Parts::IntRaw(_, nullable) => Some(nullable && store.get_u32_raw(rec, pos) == u32::MAX),
            _ => None,
        }
    }

    #[must_use]
    pub fn size(&self, tp: u16) -> u16 {
        if tp == u16::MAX {
            0
        } else {
            self.types[tp as usize].size
        }
    }

    /// The layout identity a store records, so a later reader can tell whether its own
    /// layout still matches the bytes on disk.
    ///
    /// Enforces @FR-L-Sound: a reader whose identity differs from a store's RECORDED
    /// identity must reject-or-migrate and NEVER read the bytes raw; a raw handoff is
    /// admitted only when the two are equal.  `schema_sidecar` is what compares them.
    ///
    /// @PLN97 — a stable FNV-1a hash of the STORAGE layout of `roots` and every
    /// type they reference (record sizes + `Parts` field byte positions,
    /// narrow-int encodings, collection element strides) AND the host endianness
    /// (@PLN97 F9 — the store is a host-endian raw image, so the identity must
    /// reject a same-layout store from the other endianness). Changes iff the byte
    /// layout or host endianness changes — the compact "layout identity" phases D
    /// (the schema sidecar) and F (the compiler migration aid) embed and compare to
    /// decide a raw-vs-serialize handoff. Sensitive to the #477 class (nested-vector
    /// strides); pair it with the schema (`ir_schema::data_to_json`) for the full
    /// identity. NOT `DefaultHasher` (that is not stable across Rust versions).
    #[must_use]
    pub fn layout_algo_hash(&self, roots: &[u16]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in self.layout_dump(roots).bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    /// @PLN97 — a human-readable, stable dump of the STORAGE layout of `roots`
    /// and every type they reference: a leading `@endian\t<little|big>` line
    /// (@PLN97 F9 — host endianness), then one line per type,
    /// `name\tsize=N\t<parts>`, sorted by name. `<parts>` renders the load-bearing
    /// layout facts only (struct field byte positions, narrow-int encodings,
    /// collection element strides — the #477 surface), keyed by type NAME so the
    /// dump is stable across unrelated known-type renumbering. Backs
    /// [`layout_algo_hash`] and the @PLN97 golden layout-conformance test.
    ///
    /// [`layout_algo_hash`]: Self::layout_algo_hash
    #[must_use]
    pub fn layout_dump(&self, roots: &[u16]) -> String {
        let mut items: Vec<u16> = self.layout_closure(roots).into_iter().collect();
        items.sort_by(|a, b| {
            self.layout_type_name(*a)
                .cmp(&self.layout_type_name(*b))
                .then(a.cmp(b))
        });
        use std::fmt::Write as _;
        let mut out = String::new();
        // @PLN97 F9 — pin the HOST endianness in the layout identity. The store is a
        // host-endian RAW image, so a byte-identical layout written on the other
        // endianness is NOT a valid raw handoff; without this line the identity hash
        // would match and misread it. Keep byte-identical with the twin in
        // `descriptor.rs::render_dump` (guarded by `descriptor_render_reproduces_layout_dump`).
        let _ = writeln!(
            out,
            "@endian\t{}",
            if cfg!(target_endian = "big") {
                "big"
            } else {
                "little"
            }
        );
        for kt in items {
            let _ = writeln!(
                out,
                "{}\tsize={}\t{}",
                self.layout_type_name(kt),
                self.size(kt),
                self.render_layout_parts(kt)
            );
        }
        out
    }

    fn layout_type_name(&self, kt: u16) -> String {
        self.types
            .get(kt as usize)
            .map_or_else(|| format!("#{kt}"), |t| t.name.clone())
    }

    /// Transitive closure of the types reachable from `roots` via their layout
    /// references (fields, collection elements, childrecs, data-enum variants).
    /// `pub(crate)` so the @PLN105 layout-descriptor emitter walks the exact same
    /// closure the layout hash commits to (no duplicate, drift-prone walk).
    pub(crate) fn layout_closure(&self, roots: &[u16]) -> std::collections::BTreeSet<u16> {
        let mut seen: std::collections::BTreeSet<u16> = std::collections::BTreeSet::new();
        let mut stack: Vec<u16> = roots
            .iter()
            .copied()
            .filter(|k| (*k as usize) < self.types.len())
            .collect();
        while let Some(kt) = stack.pop() {
            if !seen.insert(kt) {
                continue;
            }
            let mut refs: Vec<u16> = Vec::new();
            match &self.types[kt as usize].parts {
                Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                    refs.extend(fields.iter().map(|f| f.content));
                }
                Parts::Vector(e) | Parts::Array(e) => refs.push(*e),
                Parts::Sorted(e, _)
                | Parts::Ordered(e, _)
                | Parts::Hash(e, _)
                | Parts::Index(e, _, _)
                | Parts::Radix(e, _)
                | Parts::Trie(e, _) => refs.push(*e),
                Parts::ChildRec(c) => refs.push(*c),
                // A plain variant (no data) keeps `known_type == u16::MAX`;
                // only data-carrying variants have an `EnumValue` type to reach.
                Parts::Enum(vs) => {
                    refs.extend(vs.iter().map(|(t, _)| *t).filter(|t| *t != u16::MAX))
                }
                _ => {}
            }
            for r in refs {
                if (r as usize) < self.types.len() && !seen.contains(&r) {
                    stack.push(r);
                }
            }
        }
        seen
    }

    fn render_layout_parts(&self, kt: u16) -> String {
        let name = |k: u16| self.layout_type_name(k);
        match &self.types[kt as usize].parts {
            Parts::Trie(e, k) => format!(
                "trie<{}[{k}]>{}",
                name(*e),
                crate::placement::tag(crate::placement::TRIE)
            ),
            Parts::Base => "base".to_string(),
            Parts::Struct(fields) => {
                let inner: Vec<String> = fields
                    .iter()
                    .map(|f| format!("{}@{}:{}", f.name, f.position, name(f.content)))
                    .collect();
                format!("struct{{{}}}", inner.join(", "))
            }
            Parts::Enum(vs) => {
                let inner: Vec<String> = vs.iter().map(|(_, n)| n.clone()).collect();
                format!("enum{{{}}}", inner.join(", "))
            }
            Parts::EnumValue(tag, fields) => {
                let inner: Vec<String> = fields
                    .iter()
                    .map(|f| format!("{}@{}:{}", f.name, f.position, name(f.content)))
                    .collect();
                format!("enumvalue[{tag}]{{{}}}", inner.join(", "))
            }
            Parts::Byte(start, nul) => format!("byte(start={start},null={nul})"),
            Parts::Short(start, nul) => format!("short(start={start},null={nul})"),
            Parts::Int(start, nul) => format!("int4(start={start},null={nul})"),
            Parts::IntRaw(start, nul) => format!("int4raw(start={start},null={nul})"),
            Parts::ShortRaw(start, nul) => format!("shortraw(start={start},null={nul})"),
            Parts::Vector(e) => format!("vector<{}>(elem_size={})", name(*e), self.size(*e)),
            Parts::Array(e) => format!("array<{}>(elem_size={})", name(*e), self.size(*e)),
            Parts::Sorted(e, keys) => {
                format!(
                    "sorted<{}>(keys={keys:?},elem_size={})",
                    name(*e),
                    self.size(*e)
                )
            }
            Parts::Ordered(e, keys) => {
                format!(
                    "ordered<{}>(keys={keys:?},elem_size={})",
                    name(*e),
                    self.size(*e)
                )
            }
            Parts::Hash(e, keys) => {
                format!(
                    "hash<{}>(keys={keys:?},elem_size={}{})",
                    name(*e),
                    self.size(*e),
                    crate::placement::tag(crate::placement::HASH)
                )
            }
            Parts::Index(e, keys, left) => {
                format!(
                    "index<{}>(keys={keys:?},left={left},elem_size={}{})",
                    name(*e),
                    self.size(*e),
                    crate::placement::tag(crate::placement::INDEX)
                )
            }
            Parts::Radix(e, keys) => {
                format!(
                    "spatial<{}>(keys={keys:?},elem_size={}{})",
                    name(*e),
                    self.size(*e),
                    crate::placement::tag(crate::placement::RADIX)
                )
            }
            Parts::DbRef => "dbref12".to_string(),
            Parts::ChildRec(c) => format!("childrec<{}>", name(*c)),
        }
    }

    /// Plan-06 phase 2 — true iff `tp` (a struct or struct-enum-variant)
    /// contains any field that owns out-of-line data: text, references,
    /// vectors, hashes, sorted/index/spatial, or nested struct-enums.
    ///
    /// When this returns false, a struct of type `tp` can be safely
    /// `copy_block`-copied across stores without further deep-copy
    /// work — its bytes are self-contained.  Rebase-eligible.
    ///
    /// When true, the struct contains store-local positions or DbRefs
    /// that need translation/deep-copy across the worker→parent
    /// boundary; the caller falls back to `copy_from_worker`.
    ///
    /// For `Parts::Enum` (untyped enum, just a 1-byte discriminant):
    /// returns false — no owned data.
    /// For `Parts::Struct` / `Parts::EnumValue`: true iff any field
    /// is owned (text or DbRef-shaped).
    /// For other Parts (Base, Byte, Int, Vector, etc.): conservative
    /// true since the type itself is owned.
    #[must_use]
    pub fn has_owned_sub_fields(&self, tp: u16) -> bool {
        if tp == u16::MAX {
            return false;
        }
        match &self.types[tp as usize].parts {
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                fields.iter().any(|f| self.is_field_owned(f.content))
            }
            // Untyped enum (Parts::Enum) is a 1-byte discriminant; no
            // owned data.  But a struct-enum's owning type is the
            // parent Enum, whose variants are EnumValue records — so
            // for ANY enum tp passed here, we have to inspect variants.
            Parts::Enum(variants) => variants
                .iter()
                .any(|(v_tp, _)| self.has_owned_sub_fields(*v_tp)),
            // Bare types: conservative.
            _ => true,
        }
    }

    /// Plan-06 phase 2b refinement — per-variant ownership for
    /// struct-enums (DESIGN.md D11c).
    ///
    /// Given a struct-enum's parent type `enum_tp` and a discriminant
    /// byte read from the value, returns whether THAT specific variant
    /// has owned sub-fields.  Used by the per-element copy dispatch
    /// in parallel_execute_and_collect to take the cheap
    /// `copy_from_worker_unowned` path for variants that don't need
    /// the full deep-copy.
    ///
    /// Example: `Verdict { Pass{score:integer}, Fail{reason:text} }`
    /// - has_owned_sub_fields(Verdict) → true (because Fail has text)
    /// - variant_has_owned_sub_fields(Verdict, 0) → false (Pass: integer only)
    /// - variant_has_owned_sub_fields(Verdict, 1) → true (Fail: text)
    ///
    /// Returns `true` (conservative) if `enum_tp` is not a Parts::Enum
    /// or if `disc` is out of range.  This keeps callers safe when the
    /// type is something else.
    #[must_use]
    pub fn variant_has_owned_sub_fields(&self, enum_tp: u16, disc: u8) -> bool {
        if enum_tp == u16::MAX || (enum_tp as usize) >= self.types.len() {
            return true; // conservative
        }
        if let Parts::Enum(variants) = &self.types[enum_tp as usize].parts {
            // Plan-06 D11c per-variant check: each (v_tp, name) pair
            // is one variant; v_tp is the EnumValue type with that
            // variant's fields.  The discriminant indexes into the
            // variants list (0-based).
            if let Some((v_tp, _)) = variants.get(disc as usize) {
                return self.has_owned_sub_fields(*v_tp);
            }
        }
        // Not a Parts::Enum or disc out-of-range → fall back to
        // whole-type ownership.
        self.has_owned_sub_fields(enum_tp)
    }

    /// Internal helper for `has_owned_sub_fields`: classify a single
    /// field's content type.  Primitive integer/float/bool variants
    /// are unowned; everything that owns out-of-line data is owned.
    fn is_field_owned(&self, content: u16) -> bool {
        if content == u16::MAX {
            return false;
        }
        match &self.types[content as usize].parts {
            // text is tp==5 in the seed types; also catch Parts::Base
            // narrow-text shapes.
            Parts::Base if content == 5 => true,
            Parts::Base => false,
            Parts::Byte(_, _)
            | Parts::Short(_, _)
            | Parts::Int(_, _)
            | Parts::IntRaw(_, _)
            | Parts::ShortRaw(_, _) => false,
            Parts::Enum(variants) => {
                // Untyped enum (1-byte disc) is owned-free; struct-enum
                // (variants with payload) is owned because it carries
                // sub-data.
                variants
                    .iter()
                    .any(|(v_tp, _)| self.has_owned_sub_fields(*v_tp))
            }
            Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                fields.iter().any(|f| self.is_field_owned(f.content))
            }
            // Owning types: vector, hash, sorted, index, spatial, array.
            _ => true,
        }
    }
    /// For EnumValue types, return the parent enum's size (which covers
    /// the largest variant).  For all other types, return their own size.
    /// B2-runtime: unit enum variants may have a smaller type size than
    /// the parent enum needs.  The Database opcode must claim enough
    /// space for `set_default_value` to initialize all fields.
    ///
    /// # Panics
    ///
    /// When `tp` is a generic's TYPE VARIABLE row.  Every record allocation reaches
    /// here with its type row in hand, and a template's row surviving substitution is
    /// a compiler bug whose only other symptom is a field read at the wrong offset —
    /// see the comment in the body (loft#1070).
    pub fn enum_parent_size(&self, tp: u16) -> u16 {
        if tp == u16::MAX {
            return 0;
        }
        // loft#1070 — a generic's TYPE VARIABLE has a runtime row (`__typevar_T`,
        // registered by `typedef::fill_database` so a template body can be parsed at
        // all), and a record must never be allocated with it.  `Parser::
        // retarget_parametric_type_rows` re-points every row a monomorph inherited
        // from its template, on the invariant that after substitution the type
        // variable does not exist, so every surviving reference to its row is stale
        // BY CONSTRUCTION and there is no case where keeping one is right.  This is
        // that invariant, checked where the record is actually born.
        //
        // It is here because this is the one call every record allocation makes with
        // the type row still in hand — both `OpDatabase` twins (`state/io.rs`,
        // `codegen_runtime.rs`) and the placement arena — so a single site covers
        // both backends without the interp/native mirror being written twice.
        //
        // ⚠ It is checked at RUNTIME, and unconditionally, because the failure it
        // replaces has no other signal.  A record built to the placeholder's layout
        // reads a field out of the wrong word: `f<T>(x: T, c: boolean) -> T { if c {
        // y: T = x; y } else … }` answered `4294967198` for the `-7` it was handed,
        // on both backends, with no diagnostic and no crash.  What made loft#1070
        // diagnosable at all was the leak warning naming `__typevar_T` — so a version
        // of the same defect that FREES correctly would have been completely silent,
        // and the leak gate is not the guard it looked like.
        //
        // The message says "compiler bug" because it is one: no loft program can
        // provoke this by being wrong, only by finding a lowering site that
        // `retarget_parametric_type_rows` does not reach — it finds rows by the op's
        // own declaration naming the argument `tp` / `…_tp`, which is a convention,
        // not a checked fact.  Measured silent across the 4310-test corpus, so it
        // costs a comparison that fails on the first byte for every real type name.
        assert!(
            !self.types[tp as usize].name.starts_with(TYPEVAR_ROW_PREFIX),
            "internal compiler error: a record is being allocated with a generic \
             TYPE VARIABLE's row ({}, kt={tp}) — a template's layout escaped \
             substitution, so its fields would be read at the wrong offsets. \
             Please report this program at https://github.com/loft-lang/loft/issues",
            self.types[tp as usize].name
        );
        let own_size = self.types[tp as usize].size;
        // Check if any type in the system is an Enum whose variants include tp.
        // If so, use the Enum's size (which is the max of all variants).
        for t in &self.types {
            if let Parts::Enum(variants) = &t.parts {
                for (v_tp, _) in variants {
                    if *v_tp == tp && t.size > own_size {
                        return t.size;
                    }
                }
            }
        }
        own_size
    }

    #[must_use]
    pub fn position(&self, tp: u16, field: &str) -> u16 {
        if tp == u16::MAX {
            u16::MAX
        } else if let Parts::Struct(f) | Parts::EnumValue(_, f) = &self.types[tp as usize].parts {
            for fld in f {
                if fld.name == field {
                    return fld.position;
                }
            }
            u16::MAX
        } else {
            u16::MAX
        }
    }

    /// The byte offset of field number `idx` in struct type `tp` — [`Self::position`]'s twin,
    /// asked by INDEX rather than by name.
    ///
    /// The two spaces exist because the IR names a field both ways: a projection carries the
    /// OFFSET (`OpGetField(w, 16, …)`) and a container-qualified write carries the field
    /// NUMBER (`OpNewRecord(w, tp, 1)`).  Anything comparing the two — asking whether a view
    /// and a disturbance name the same place — has to convert, and this is where.
    /// `u16::MAX` when `tp` is not a struct or `idx` is past its fields, which every caller
    /// reads as "cannot say" rather than as an offset.
    #[must_use]
    pub fn field_position(&self, tp: u16, idx: u16) -> u16 {
        if tp == u16::MAX {
            return u16::MAX;
        }
        if let Parts::Struct(f) | Parts::EnumValue(_, f) = &self.types[tp as usize].parts {
            f.get(idx as usize).map_or(u16::MAX, |fld| fld.position)
        } else {
            u16::MAX
        }
    }

    #[must_use]
    pub fn is_text_type(&self, tp: u16) -> bool {
        self.names.get("text").copied() == Some(tp)
    }

    pub(super) fn show_fields(&self, pretty: bool, res: &mut String, v: &[Field]) {
        if pretty {
            *res += "\n";
        } else {
            *res += "{";
        }
        for (f_nr, p) in v.iter().enumerate() {
            let name = &self.types[p.content as usize].name;
            if pretty {
                *res += "    ";
            } else if f_nr > 0 {
                *res += ", ";
            }
            write!(res, "{}:{name}[{}]", p.name, p.position).unwrap();
            if !p.other_indexes.is_empty() {
                write!(res, " other {:?}", p.other_indexes).unwrap();
            }
            // loft#876 — a field's DECLARED default is deliberately NOT rendered.  This
            // dump is the @PLN97 layout identity, and a default changes no width and no
            // offset: rendering it would make `height: float = 1.5` a different layout
            // from `height: float`, so adding a default would refuse an existing store.
            // Same call as `nullable`, which is carried and never rendered for the same
            // reason.  (Until defaults were carried every field held the `Str("")`
            // placeholder, so the branch this replaces never fired.)
            if pretty {
                *res += "\n";
            }
        }
        if !pretty {
            *res += "}";
        }
    }

    #[must_use]
    pub fn show_type(&self, tp: u16, pretty: bool) -> String {
        if tp > self.types.len() as u16 {
            return format!("Unknown type({tp})");
        }
        let typedef = &self.types[tp as usize];
        let mut res = format!("{}[{}/{}]:", typedef.name, typedef.size, typedef.align);
        if let Parts::EnumValue(nr, _) = typedef.parts {
            write!(res, " EnumValue({nr})").unwrap();
        }
        if !typedef.parents.is_empty() {
            write!(res, " parents [").unwrap();
            for (n, p) in typedef.parents.iter().enumerate() {
                if n > 0 {
                    write!(res, ", ").unwrap();
                }
                write!(res, "{} {p}", self.types[*p as usize].name).unwrap();
            }
            write!(res, "]").unwrap();
        }
        if let Parts::Struct(v) | Parts::EnumValue(_, v) = &typedef.parts {
            self.show_fields(pretty, &mut res, v);
        } else if let Parts::Enum(v) = &typedef.parts {
            if pretty {
                res += "\n";
            } else {
                res += "[";
            }
            for (e_nr, (nr, e)) in v.iter().enumerate() {
                if pretty {
                    res += "    ";
                } else if e_nr > 0 {
                    res += ", ";
                }
                write!(res, "{e}").unwrap();
                if *nr != u16::MAX {
                    write!(res, ":{nr}").unwrap();
                }
                if pretty {
                    res += "\n";
                }
            }
            if !pretty {
                res += "]";
            }
        } else {
            write!(res, "{:?}", typedef.parts).unwrap();
            if !typedef.keys.is_empty() {
                res += " keys [";
                for k in &typedef.keys {
                    write!(
                        res,
                        "tp:{} desc:{} field:{}, ",
                        k.type_nr.abs(),
                        k.type_nr < 0,
                        k.position
                    )
                    .unwrap();
                }
                res += "]";
            }
            if pretty {
                res += "\n";
            }
        }
        res
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Type {
    pub name: String,
    pub parts: Parts,
    pub keys: Vec<crate::keys::Key>,
    pub(super) parents: std::collections::BTreeSet<u16>,
    pub(super) complex: bool,
    pub(super) linked: bool,
    pub(super) size: u16,
    pub(super) align: u8,
    /// Linked-field groups appended to this type's `Parts::Struct` /
    /// `Parts::EnumValue` field list.  Used **exclusively** for index
    /// bookkeeping triples (`#left_N` / `#right_N` / `#color_N`) per
    /// `index<T[key]>` instance.  Empty for plain user structs.
    /// Tuple element groups live on the parser-side
    /// `Definition::field_groups` instead.
    pub field_groups: Vec<crate::data::LinkedFieldGroup>,
}

impl Type {
    // @PLN11 D2a read seam — `complex`/`linked`/`size`/`align` are `pub(super)`
    // (computed by `finish`); expose them `pub(crate)` so the schema
    // materializer (`crate::ir_store`) can cache them.  `parents` stays private
    // (a derived back-reference index, rebuilt on load, never serialized).
    #[must_use]
    pub(crate) fn is_complex(&self) -> bool {
        self.complex
    }
    #[must_use]
    pub(crate) fn is_linked_flag(&self) -> bool {
        self.linked
    }
    #[must_use]
    pub(crate) fn size_bytes(&self) -> u16 {
        self.size
    }
    #[must_use]
    pub(crate) fn align_bytes(&self) -> u8 {
        self.align
    }

    /// @PLN11 D2a — reconstruct a `Type` from cached store fields.  `parents`
    /// (a derived back-reference index, read only by parse-time layout
    /// validation + debug display, never by codegen/execution) is restored
    /// empty; the load path skips the layout validation that consumes it.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_stored(
        name: String,
        parts: Parts,
        keys: Vec<crate::keys::Key>,
        complex: bool,
        linked: bool,
        size: u16,
        align: u8,
        field_groups: Vec<crate::data::LinkedFieldGroup>,
    ) -> Type {
        Type {
            name,
            parts,
            keys,
            parents: std::collections::BTreeSet::new(),
            complex,
            linked,
            size,
            align,
            field_groups,
        }
    }

    /// @PLN11 D2a — drop the derived `parents` index (for comparing a
    /// store-reloaded schema against a fresh parse, which carries `parents`).
    pub(crate) fn clear_parents(&mut self) {
        self.parents.clear();
    }

    pub(super) fn new(name: &str, parts: Parts, size: u16) -> Type {
        Type {
            name: name.to_string(),
            parts,
            keys: Vec::new(),
            parents: std::collections::BTreeSet::new(),
            complex: false,
            linked: false,
            size,
            align: size as u8,
            field_groups: Vec::new(),
        }
    }

    pub(super) fn data(name: &str, parts: Parts) -> Type {
        Type {
            name: name.to_string(),
            parts,
            keys: Vec::new(),
            parents: std::collections::BTreeSet::new(),
            complex: true,
            linked: false,
            size: 4,
            align: 4,
            field_groups: Vec::new(),
        }
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn contains(&self, tp: u16) -> bool {
        match self.parts {
            Parts::Vector(c)
            | Parts::Array(c)
            | Parts::Sorted(c, _)
            | Parts::Ordered(c, _)
            | Parts::Hash(c, _)
            | Parts::Index(c, _, _)
            | Parts::ChildRec(c)
            | Parts::Radix(c, _)
            | Parts::Trie(c, _) => c == tp,
            _ => false,
        }
    }

    /// Iterate every index-bookkeeping group on this type.  Each yields
    /// `(instance_nr, [left_idx, right_idx, color_idx])` — the field
    /// indices of the bookkeeping triple appended for one
    /// `index<T[key]>` registration.  Replaces ad-hoc
    /// `f.name.starts_with("#left_")` scans on `Parts::Struct` /
    /// `Parts::EnumValue` field lists.
    pub fn index_groups(&self) -> impl Iterator<Item = &crate::data::LinkedFieldGroup> {
        self.field_groups
            .iter()
            .filter(|g| matches!(g.kind, crate::data::LinkedFieldKind::Index))
    }
}

#[cfg(test)]
mod typevar_row_tests {
    use super::{Stores, TYPEVAR_ROW_PREFIX};

    /// loft#1070 — allocating a record with a generic's TYPE VARIABLE row is refused.
    ///
    /// The guard cannot be provoked from loft source: `retarget_parametric_type_rows`
    /// re-points every inherited row, and the whole 4310-test corpus runs without it
    /// firing. That is exactly why it is tested here instead — a tripwire nobody has
    /// shown can trip is indistinguishable from one that matches nothing, and this
    /// class's defect (a prefix spelled at the minting site and again at the checking
    /// site) is the way it would silently stop matching.
    ///
    /// It builds the row through the same public API `typedef::fill_database` uses, and
    /// names it with the shared constant rather than a literal, so a renamed prefix
    /// moves both ends together.
    #[test]
    #[should_panic(expected = "TYPE VARIABLE's row")]
    fn a_record_may_not_be_allocated_with_a_type_variable_row() {
        let mut s = Stores::new();
        let tp = s.structure(&format!("{TYPEVAR_ROW_PREFIX}T"), 0);
        let _ = s.enum_parent_size(tp);
    }

    /// The control: an ordinary type of a similar shape must still answer its size.
    ///
    /// Without it a guard that refused every allocation would pass the cell above.
    #[test]
    fn an_ordinary_row_still_answers_its_size() {
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let tp = s.structure("T", 0);
        s.field(tp, "value", int_c);
        s.finish();
        assert!(
            s.enum_parent_size(tp) > 0,
            "a real struct named `T` is not the type-variable row and must size normally"
        );
    }
}

#[cfg(test)]
mod layout_tests {
    use super::{Field, Parts, Stores, Type};

    /// Build a clean two-field struct via the public API.
    fn score_struct(s: &mut Stores) -> u16 {
        let int_c = s.name("integer");
        let txt_c = s.name("text");
        let tp = s.structure("Score", 0);
        s.field(tp, "name", txt_c);
        s.field(tp, "value", int_c);
        tp
    }

    #[test]
    fn validate_all_layouts_clean_after_init_returns_no_issues() {
        let s = Stores::new();
        assert!(s.validate_all_layouts().is_empty());
    }

    /// Build the shape that makes `finish()` promote: `Node` is the content of an
    /// `index` AND of a `vector`/`sorted`, so it is LINKED and both collections are
    /// renamed in place (`vector<Node>` → `array<Node>`, `sorted<Node[id]>` →
    /// `ordered<Node[id]>`).  Returns `(node, vector<Node>, sorted<Node[id]>)`.
    fn promoted_node_schema(s: &mut Stores) -> (u16, u16, u16) {
        let int_c = s.name("integer");
        let node = s.structure("Node", 0);
        s.field(node, "id", int_c);
        let key = [("id".to_string(), true)];
        let vec_c = s.vector(node);
        let idx_c = s.index(node, &key);
        let sorted_c = s.sorted(node, &key);
        let hash_c = s.hash(node, &["id".to_string()]);
        let graph = s.structure("Graph", 0);
        s.field(graph, "nodes", vec_c);
        s.field(graph, "idx", idx_c);
        let wide = s.structure("Wide", 0);
        s.field(wide, "a", sorted_c);
        s.field(wide, "b", hash_c);
        s.finish();
        (node, vec_c, sorted_c)
    }

    /// A schema round-trip (the whole-program warm cache) must not lose the lookup
    /// key of a type `finish()` promoted.  `install_schema` rebuilds `names` from the
    /// stored names, which for a promoted collection is the POST-promotion spelling —
    /// so without the constructor-key alias the next `vector`/`sorted` call misses and
    /// mints a SECOND type for a collection that already exists.  That duplicate id is
    /// baked into emitted native code while `init()` never registers it, panicking with
    /// "index out of bounds" on every warm-cache `--native` run.
    #[test]
    fn install_schema_keeps_the_lookup_key_of_a_promoted_collection() {
        let mut s = Stores::new();
        let (node, vec_c, sorted_c) = promoted_node_schema(&mut s);
        let key = [("id".to_string(), true)];
        // Promotion happened: the stored names are the post-promotion spellings.
        assert_eq!(s.types[vec_c as usize].name, "array<Node>");
        assert_eq!(s.types[sorted_c as usize].name, "ordered<Node[id]>");
        // Pre-round-trip, a repeat construction resolves to the existing type.
        assert_eq!(s.vector(node), vec_c);
        assert_eq!(s.sorted(node, &key), sorted_c);

        let before = s.types.len();
        s.install_schema(s.types.clone());
        // Post-round-trip it must still resolve to the SAME id, minting nothing.
        assert_eq!(s.vector(node), vec_c, "vector<Node> minted a duplicate");
        assert_eq!(
            s.sorted(node, &key),
            sorted_c,
            "sorted<Node[id]> minted a duplicate"
        );
        assert_eq!(s.types.len(), before, "the round-trip grew the type table");
    }

    /// The alias must never displace a type the schema really holds: a schema
    /// carrying BOTH a promoted `array<Node>` and a distinct unpromoted
    /// `vector<Node>` keeps each name on its own id.
    #[test]
    fn install_schema_alias_never_overwrites_a_real_name() {
        let mut s = Stores::new();
        let (_node, vec_c, _sorted_c) = promoted_node_schema(&mut s);
        let mut types = s.types.clone();
        let unpromoted = types.len() as u16;
        types.push(Type::data("vector<Node>", Parts::Vector(0)));
        s.install_schema(types);
        assert_eq!(s.names.get("vector<Node>").copied(), Some(unpromoted));
        assert_eq!(s.names.get("array<Node>").copied(), Some(vec_c));
    }

    #[test]
    fn validate_layout_unknown_type_reports_unknown() {
        let s = Stores::new();
        let issues = s.validate_layout("DoesNotExist");
        assert_eq!(issues, vec!["(unknown type: DoesNotExist)".to_string()]);
    }

    #[test]
    fn validate_layout_by_nr_silently_skips_u16_max() {
        // `field.content == u16::MAX` happens during parser-recovery
        // (unresolved field type) — the underlying parse error is
        // already reported, so validation must NOT add noise on top.
        let s = Stores::new();
        let mut visited = std::collections::HashSet::new();
        let mut issues = Vec::new();
        s.validate_layout_by_nr(u16::MAX, &mut visited, &mut issues);
        assert!(issues.is_empty(), "expected no issues, got: {issues:?}");
    }

    #[test]
    fn validate_layout_by_nr_flags_real_out_of_range() {
        // A type id past the end of the registry IS a real bug —
        // surface it.
        let s = Stores::new();
        let max = s.types.len() as u16;
        let mut visited = std::collections::HashSet::new();
        let mut issues = Vec::new();
        s.validate_layout_by_nr(max, &mut visited, &mut issues);
        assert!(
            issues.iter().any(|i| i.contains("out of range")),
            "expected out-of-range issue, got: {issues:?}"
        );
    }

    #[test]
    fn validate_layout_clean_struct_after_finish_no_issues() {
        let mut s = Stores::new();
        score_struct(&mut s);
        s.finish();
        let issues = s.validate_layout("Score");
        assert!(
            issues.is_empty(),
            "expected no issues for clean Score, got: {issues:?}"
        );
    }

    #[test]
    fn debug_layout_clean_struct_includes_fields_and_size() {
        let mut s = Stores::new();
        score_struct(&mut s);
        s.finish();
        let dump = s.debug_layout("Score");
        assert!(dump.contains("Score"), "dump missing type name: {dump}");
        assert!(dump.contains("name"), "dump missing 'name' field: {dump}");
        assert!(dump.contains("value"), "dump missing 'value' field: {dump}");
        // Both fields must appear with positions (not u16::MAX).
        assert!(
            !dump.contains("@65535"),
            "dump still has unlaid positions: {dump}"
        );
    }

    #[test]
    fn debug_layout_unknown_type_reports_unknown() {
        let s = Stores::new();
        let dump = s.debug_layout("Nope");
        assert!(
            dump.contains("unknown type: Nope"),
            "expected 'unknown type' in: {dump}"
        );
    }

    #[test]
    fn validate_layout_detects_overlapping_fields() {
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let tp = s.structure("Bad", 0);
        s.field(tp, "a", int_c);
        s.field(tp, "b", int_c);
        s.finish();
        // Force overlap: rewrite both fields to position 0.
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) = &mut s.types[tp as usize].parts
        {
            fields[0].position = 0;
            fields[1].position = 0;
        }
        let issues = s.validate_layout("Bad");
        assert!(
            issues.iter().any(|i| i.contains("overlap")),
            "expected overlap issue, got: {issues:?}"
        );
        assert!(
            issues.iter().any(|i| i.contains("layout:")),
            "expected layout summary in issues, got: {issues:?}"
        );
    }

    #[test]
    fn validate_layout_detects_field_beyond_size() {
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let tp = s.structure("Tiny", 0);
        s.field(tp, "v", int_c);
        s.finish();
        // Shrink the type's reported size below the field's end.
        s.types[tp as usize].size = 4; // integer is 8 bytes, field at @0
        let issues = s.validate_layout("Tiny");
        assert!(
            issues
                .iter()
                .any(|i| i.contains("extends beyond type size")),
            "expected beyond-size issue, got: {issues:?}"
        );
    }

    #[test]
    fn validate_layout_detects_field_without_position() {
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let tp = s.structure("Unlaid", 0);
        s.field(tp, "v", int_c);
        s.finish();
        // Force the field's position back to u16::MAX (simulates a
        // late-mutation that skipped finish_type).
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) = &mut s.types[tp as usize].parts
        {
            fields[0].position = u16::MAX;
        }
        let issues = s.validate_layout("Unlaid");
        assert!(
            issues.iter().any(|i| i.contains("no position")),
            "expected no-position issue, got: {issues:?}"
        );
    }

    #[test]
    fn validate_all_layouts_index_bookkeeping_after_p191_fix_no_issues() {
        // P191 fix: `database.index` now appends 4-byte int<0,false>
        // bookkeeping fields (`#left_N`, `#right_N`) so they land
        // contiguously at [pos, pos+4, pos+8] and match tree::add's
        // hardcoded RB_LEFT=0, RB_RIGHT=4, RB_FLAG=8 offsets.  This
        // test guards against regressing back to 8-byte `integer`
        // bookkeeping (which would resurface the corruption).
        let mut s = Stores::new();
        let score_tp = score_struct(&mut s);
        let _index_tp = s.index(score_tp, &[("name".to_string(), true)]);
        s.finish();
        let issues = s.validate_all_layouts();
        assert!(
            issues.is_empty(),
            "expected no layout issues after P191 fix, got: {issues:?}"
        );
    }

    #[test]
    fn validate_layout_index_left_field_nr_out_of_range() {
        let mut s = Stores::new();
        let score_tp = score_struct(&mut s);
        let index_tp = s.index(score_tp, &[("name".to_string(), true)]);
        s.finish();
        // Force `Parts::Index`'s left_field_nr to point past the end.
        if let Parts::Index(_, _, left) = &mut s.types[index_tp as usize].parts {
            *left = 9999;
        }
        let issues = s.validate_layout("index<Score[name]>");
        assert!(
            issues
                .iter()
                .any(|i| i.contains("Parts::Index left_field_nr=9999 out of range")),
            "expected out-of-range issue, got: {issues:?}"
        );
    }

    #[test]
    fn validate_layout_index_left_field_nr_points_at_wrong_field() {
        let mut s = Stores::new();
        let score_tp = score_struct(&mut s);
        let index_tp = s.index(score_tp, &[("name".to_string(), true)]);
        s.finish();
        // Force `Parts::Index` to point at field 0 (the user `name`
        // field, not `#left_*`).
        if let Parts::Index(_, _, left) = &mut s.types[index_tp as usize].parts {
            *left = 0;
        }
        let issues = s.validate_layout("index<Score[name]>");
        assert!(
            issues.iter().any(|i| i.contains("expected '#left_*'")),
            "expected wrong-field-name issue, got: {issues:?}"
        );
    }

    #[test]
    fn layout_summary_contains_size_align_and_field_byte_ranges() {
        let mut s = Stores::new();
        let tp = score_struct(&mut s);
        s.finish();
        let line = s.layout_summary(tp);
        assert!(line.starts_with("Score("), "expected name prefix: {line}");
        assert!(line.contains("size="), "expected size= in: {line}");
        assert!(line.contains("align="), "expected align= in: {line}");
        assert!(line.contains("name:text@"), "expected name field: {line}");
        assert!(
            line.contains("value:integer@"),
            "expected value field: {line}"
        );
        // The byte-range form is `@start..end`.
        assert!(line.contains(".."), "expected ..end in field range: {line}");
    }

    #[test]
    fn validate_all_layouts_skips_unlaid_types() {
        let mut s = Stores::new();
        // Push a struct with size = u16::MAX (unlaid) — validate_all
        // must skip it (no panic, no false positive).
        let tp = s.structure("Unlaid", 0);
        let int_c = s.name("integer");
        s.field(tp, "v", int_c);
        // Don't call finish() — type stays at size = u16::MAX.
        assert_eq!(s.types[tp as usize].size, u16::MAX);
        let issues = s.validate_all_layouts();
        assert!(
            issues.is_empty(),
            "expected no issues for unlaid type, got: {issues:?}"
        );
    }

    #[test]
    fn validate_layout_enum_variant_overlap_within_variant_detected() {
        // Within a single enum variant, fields must NOT overlap.  The
        // legitimate "overlap" between *different* variants of the
        // same enum is handled by the parent Parts::Enum arm walking
        // each variant separately, so it never compares across them.
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let enum_tp = s.enumerate("E");
        let var_tp = s.structure("E_Variant", 1);
        s.field(var_tp, "a", int_c);
        s.field(var_tp, "b", int_c);
        s.types[var_tp as usize].parents.insert(enum_tp);
        s.enum_value(enum_tp, "Variant", var_tp);
        s.finish();
        // Force the two fields inside the variant to overlap.
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
            &mut s.types[var_tp as usize].parts
        {
            fields[0].position = 0;
            fields[1].position = 0;
        }
        let issues = s.validate_layout("E_Variant");
        assert!(
            issues.iter().any(|i| i.contains("overlap")),
            "expected within-variant overlap issue, got: {issues:?}"
        );
    }

    #[test]
    fn debug_layout_index_includes_bookkeeping_fields() {
        let mut s = Stores::new();
        score_struct(&mut s);
        let score_tp = s.name("Score");
        s.index(score_tp, &[("name".to_string(), true)]);
        s.finish();
        let dump = s.debug_layout("Score");
        assert!(dump.contains("#left_1"), "missing #left_1 in: {dump}");
        assert!(dump.contains("#right_1"), "missing #right_1 in: {dump}");
        assert!(dump.contains("#color_1"), "missing #color_1 in: {dump}");
    }

    #[test]
    fn validate_layout_default_constructed_field_compiles() {
        // Construct a Field directly via struct literal — guards
        // against future reorderings that would break test setup.
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let tp = s.structure("Manual", 0);
        if let Parts::Struct(f) = &mut s.types[tp as usize].parts {
            f.push(Field {
                name: "x".to_string(),
                content: int_c,
                position: 0,
                default: None,
                nullable: false,
                other_indexes: Vec::new(),
            });
        }
        s.finish();
        let issues = s.validate_layout("Manual");
        assert!(
            issues.is_empty(),
            "expected no issues for Manual struct, got: {issues:?}"
        );
    }

    // ───────────────────────────────────────────────────────────────
    // Linked-field-group layout tests
    //
    // Verify that the `LinkedFieldGroup` infrastructure registered by
    // `Stores::index` keeps the bookkeeping triple
    // (`#left_N` / `#right_N` / `#color_N`) coherent through
    // `finish_type`'s alignment-aware packing — the 3 fields stay
    // associated as a group, and each field lands at an offset that
    // respects its declared alignment (4-byte int4 fields on a 4-byte
    // boundary, 1-byte bool on any boundary).
    //
    // These cover the index pattern.  Tuple linked-field-group tests
    // live in `src/data.rs::linked_field_tests` since tuples are a
    // parser-side concept (`Definition::field_groups`).
    // ───────────────────────────────────────────────────────────────

    use crate::data::LinkedFieldKind;

    /// Build a struct with one user field of a given content type and
    /// finish it.  Returns the type id and the field's content id.
    fn struct_with_single_field(s: &mut Stores, name: &str, field_content: u16) -> u16 {
        let tp = s.structure(name, 0);
        s.field(tp, "value", field_content);
        tp
    }

    /// Read a field by name; panics if absent or the type isn't a Struct.
    fn field_by_name<'a>(s: &'a Stores, tp: u16, name: &str) -> &'a Field {
        if let Parts::Struct(fields) | Parts::EnumValue(_, fields) = &s.types[tp as usize].parts {
            fields.iter().find(|f| f.name == name).unwrap_or_else(|| {
                panic!(
                    "field '{name}' not found on type '{}'",
                    s.types[tp as usize].name
                )
            })
        } else {
            panic!(
                "type '{}' is not a Struct/EnumValue",
                s.types[tp as usize].name
            )
        }
    }

    #[test]
    fn spatial_registers_its_coordinate_keys() {
        // @PLN48 S2: a `spatial<Mob[x, y]>` must resolve its two coordinate fields to
        // `keys`, in list order, exactly as a `hash` does — that list is what the
        // Morton oracle interleaves.
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let tp = s.structure("Mob", 0);
        s.field(tp, "x", int_c);
        s.field(tp, "y", int_c);
        let sp = s.spatial(tp, &["x".to_string(), "y".to_string()]);
        s.finish();

        let keys = &s.types[sp as usize].keys;
        assert_eq!(keys.len(), 2, "two axes → two keys");
        // Ascending (positive type_nr), and x precedes y in interleave order.
        assert!(
            keys.iter().all(|k| k.type_nr > 0),
            "spatial keys are ascending"
        );
        assert!(
            keys[0].position < keys[1].position,
            "x must resolve to an earlier field than y"
        );
    }

    #[test]
    fn index_group_registered_with_three_field_indices() {
        // One index → exactly one Index-kind LinkedFieldGroup with
        // three field indices [left, right, color].
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let tp = struct_with_single_field(&mut s, "ItemA", int_c);
        let _idx = s.index(tp, &[("value".to_string(), true)]);
        s.finish();

        let groups: Vec<_> = s.types[tp as usize].index_groups().collect();
        assert_eq!(
            groups.len(),
            1,
            "expected exactly one Index group, got {groups:?}"
        );
        assert_eq!(groups[0].kind, LinkedFieldKind::Index);
        assert_eq!(
            groups[0].field_indices.len(),
            3,
            "Index triple must have 3 fields"
        );
        assert_eq!(groups[0].instance, 1, "first index gets instance=1");
    }

    #[test]
    fn index_group_field_names_are_left_right_color_in_order() {
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let tp = struct_with_single_field(&mut s, "ItemB", int_c);
        let _idx = s.index(tp, &[("value".to_string(), true)]);
        s.finish();

        let group = s.types[tp as usize].index_groups().next().expect("group");
        let names: Vec<&str> = if let Parts::Struct(fields) = &s.types[tp as usize].parts {
            group
                .field_indices
                .iter()
                .map(|&i| fields[i as usize].name.as_str())
                .collect()
        } else {
            panic!("not a struct")
        };
        assert_eq!(names, vec!["#left_1", "#right_1", "#color_1"]);
    }

    #[test]
    fn index_group_int_fields_are_4byte_aligned() {
        // Bookkeeping #left and #right are 4-byte ints (P191).  After
        // finish, their positions must be 4-byte aligned and they must
        // be exactly 4 bytes apart so `tree::add`'s hardcoded
        // `RB_LEFT=0` / `RB_RIGHT=4` arithmetic works.
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let tp = struct_with_single_field(&mut s, "ItemC", int_c);
        let _idx = s.index(tp, &[("value".to_string(), true)]);
        s.finish();

        let left = field_by_name(&s, tp, "#left_1");
        let right = field_by_name(&s, tp, "#right_1");
        let color = field_by_name(&s, tp, "#color_1");

        assert_eq!(
            left.position % 4,
            0,
            "#left_1 not 4-byte aligned (pos={})",
            left.position
        );
        assert_eq!(
            right.position,
            left.position + 4,
            "#right_1 must be at left+4"
        );
        // #color is 1 byte — alignment 1, so any byte boundary is fine.
        // It must be at left+8 per tree::add's RB_COLOR=8 expectation.
        assert_eq!(
            color.position,
            left.position + 8,
            "#color_1 must be at left+8"
        );
    }

    #[test]
    fn index_group_offsets_with_8byte_user_field() {
        // `value: integer` is 8 bytes / 8-byte aligned.  After packing,
        // bookkeeping (4+4+1) must still be contiguous and aligned.
        // Calculate_positions packs largest-first, so integer at 0,
        // then the two 4-byte ints, then the byte.
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let tp = struct_with_single_field(&mut s, "ItemD", int_c);
        let _idx = s.index(tp, &[("value".to_string(), true)]);
        s.finish();

        let left = field_by_name(&s, tp, "#left_1");
        let right = field_by_name(&s, tp, "#right_1");
        let color = field_by_name(&s, tp, "#color_1");

        // 4-byte alignment for the two int fields, contiguous +4 +8 spacing.
        assert_eq!(left.position % 4, 0);
        assert_eq!(right.position, left.position + 4);
        assert_eq!(color.position, left.position + 8);
    }

    #[test]
    fn multiple_indexes_register_distinct_groups() {
        // Two indexes on the same struct → two Index groups with
        // instance=1 and instance=2.  Each group's three fields must
        // be its own #left_N / #right_N / #color_N triple.
        let mut s = Stores::new();
        let txt_c = s.name("text");
        let int_c = s.name("integer");
        let tp = s.structure("ItemE", 0);
        s.field(tp, "name", txt_c);
        s.field(tp, "value", int_c);
        let _idx_name = s.index(tp, &[("name".to_string(), true)]);
        let _idx_value = s.index(tp, &[("value".to_string(), true)]);
        s.finish();

        let groups: Vec<_> = s.types[tp as usize].index_groups().collect();
        assert_eq!(groups.len(), 2, "expected two Index groups");
        assert_eq!(groups[0].instance, 1);
        assert_eq!(groups[1].instance, 2);

        // Each group's 3 field indices point to that instance's triple.
        if let Parts::Struct(fields) = &s.types[tp as usize].parts {
            for group in &groups {
                let triple_names: Vec<&str> = group
                    .field_indices
                    .iter()
                    .map(|&i| fields[i as usize].name.as_str())
                    .collect();
                let suffix = format!("_{}", group.instance);
                assert!(triple_names[0].ends_with(&suffix));
                assert!(triple_names[1].ends_with(&suffix));
                assert!(triple_names[2].ends_with(&suffix));
                assert!(triple_names[0].starts_with("#left_"));
                assert!(triple_names[1].starts_with("#right_"));
                assert!(triple_names[2].starts_with("#color_"));
            }
        }
    }

    #[test]
    fn multiple_index_groups_atomic_placement_keeps_each_triple_contiguous() {
        // With group-aware packing
        // (`calculate_positions_with_groups`), each index triple is
        // placed as ONE atomic block — `#left_N`, `#right_N`,
        // `#color_N` stay contiguous regardless of other fields'
        // sizes, and tree::add's `right = left+4`, `color = left+8`
        // invariants both hold for every group instance.
        let mut s = Stores::new();
        let txt_c = s.name("text");
        let int_c = s.name("integer");
        let tp = s.structure("ItemF", 0);
        s.field(tp, "name", txt_c);
        s.field(tp, "value", int_c);
        let _ = s.index(tp, &[("name".to_string(), true)]);
        let _ = s.index(tp, &[("value".to_string(), true)]);
        s.finish();

        let l1 = field_by_name(&s, tp, "#left_1").position;
        let r1 = field_by_name(&s, tp, "#right_1").position;
        let c1 = field_by_name(&s, tp, "#color_1").position;
        let l2 = field_by_name(&s, tp, "#left_2").position;
        let r2 = field_by_name(&s, tp, "#right_2").position;
        let c2 = field_by_name(&s, tp, "#color_2").position;

        // Each triple's left+4=right and left+8=color invariants hold.
        assert_eq!(l1 % 4, 0, "#left_1 4-byte aligned");
        assert_eq!(r1, l1 + 4, "#right_1 = left_1 + 4");
        assert_eq!(c1, l1 + 8, "#color_1 = left_1 + 8 (atomic group)");
        assert_eq!(l2 % 4, 0, "#left_2 4-byte aligned");
        assert_eq!(r2, l2 + 4, "#right_2 = left_2 + 4");
        assert_eq!(c2, l2 + 8, "#color_2 = left_2 + 8 (atomic group)");

        // Triples don't overlap as ranges.
        let triple1 = l1..(c1 + 1);
        let triple2 = l2..(c2 + 1);
        assert!(
            triple1.end <= triple2.start || triple2.end <= triple1.start,
            "triples must not overlap: {triple1:?} vs {triple2:?}",
        );
    }

    #[test]
    fn index_group_with_byte_user_field_keeps_color_at_left_plus_8() {
        // **Atomicity verified**: even with a 1-byte user field
        // (which used to disrupt largest-first packing and pull the
        // bool #color_N to the trailing 1-byte fill region), the
        // group-aware packer reserves the entire 9-byte triple as
        // ONE atomic block at a 4-byte-aligned position.
        // tree::add's `color = left + 8` invariant holds.
        let mut s = Stores::new();
        let byte_c = s.byte(0, false);
        let tp = s.structure("ItemG", 0);
        s.field(tp, "flag", byte_c);
        let _idx = s.index(tp, &[("flag".to_string(), true)]);
        s.finish();

        let left = field_by_name(&s, tp, "#left_1");
        let right = field_by_name(&s, tp, "#right_1");
        let color = field_by_name(&s, tp, "#color_1");

        assert_eq!(left.position % 4, 0, "#left_1 4-byte aligned");
        assert_eq!(
            right.position,
            left.position + 4,
            "#right_1 follows #left_1 by 4"
        );
        assert_eq!(
            color.position,
            left.position + 8,
            "#color_1 must be at left+8 — atomicity ensures the bool stays \
             with its triple even when user struct has 1-byte fields",
        );
    }

    #[test]
    fn index_group_field_indices_match_actual_field_positions() {
        // The LinkedFieldGroup's `field_indices` are indices into the
        // type's `Parts::Struct(fields)`.  This test verifies that
        // those indices, when used to look up fields, return EXACTLY
        // the bookkeeping triple.  Catches any off-by-one in the
        // group registration.
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let tp = s.structure("ItemH", 0);
        s.field(tp, "value", int_c);
        let _ = s.index(tp, &[("value".to_string(), true)]);
        s.finish();

        // Resolve the int4 / bool DB type ids before borrowing `s.types`
        // immutably for the group lookup (`s.int` / `s.name` need &mut).
        let int4 = s.int(0, false);
        let bool_c = s.name("boolean");

        let indices = s.types[tp as usize]
            .index_groups()
            .next()
            .expect("group")
            .field_indices
            .clone();
        if let Parts::Struct(fields) = &s.types[tp as usize].parts {
            // The three indexed fields must be #left, #right, #color in order.
            assert_eq!(fields[indices[0] as usize].name, "#left_1");
            assert_eq!(fields[indices[1] as usize].name, "#right_1");
            assert_eq!(fields[indices[2] as usize].name, "#color_1");
            // And those same indices must point to fields with the right widths.
            // #left/#right are 4-byte int4, #color is 1-byte bool.
            assert_eq!(fields[indices[0] as usize].content, int4);
            assert_eq!(fields[indices[1] as usize].content, int4);
            assert_eq!(fields[indices[2] as usize].content, bool_c);
        }
    }

    // ───────────────────────────────────────────────────────────────
    // Group-alignment verification
    //
    // The `LinkedFieldGroup` carries `alignment` (the MAX alignment
    // of its members) and `size` (atomic placement size).  These are
    // what the layout routine SHOULD honour when placing the group
    // as a single unit — the index triple must land on a 4-byte
    // boundary so the int4 members are correctly aligned, and a
    // hypothetical tuple `(byte, integer)` would need an 8-byte
    // boundary so the integer member lands on its natural alignment.
    //
    // These tests pin the alignment metadata on registered groups
    // and verify against the actual member sizes.
    // ───────────────────────────────────────────────────────────────

    #[test]
    fn group_alignment_helper_returns_max_member_alignment() {
        use crate::data::LinkedFieldGroup;
        // Index triple: int4 (align 4), int4 (align 4), bool (align 1)
        assert_eq!(LinkedFieldGroup::group_alignment(&[4, 4, 1]), 4);
        // Tuple (byte, integer): byte (align 1), integer (align 8)
        assert_eq!(LinkedFieldGroup::group_alignment(&[1, 8]), 8);
        // All bytes: alignment 1
        assert_eq!(LinkedFieldGroup::group_alignment(&[1, 1, 1]), 1);
        // Single member: that's the alignment
        assert_eq!(LinkedFieldGroup::group_alignment(&[8]), 8);
        // Empty: defaults to 1 (no members → no constraint)
        assert_eq!(LinkedFieldGroup::group_alignment(&[]), 1);
    }

    #[test]
    fn group_size_helper_packs_members_at_natural_alignment() {
        use crate::data::LinkedFieldGroup;
        // Index triple [(4,4), (4,4), (1,1)]: 4 + 4 + 1 = 9, no padding
        assert_eq!(LinkedFieldGroup::group_size(&[(4, 4), (4, 4), (1, 1)]), 9);
        // Tuple (byte, integer): byte at 0, padding 1..7, integer at 8 → total 16
        assert_eq!(LinkedFieldGroup::group_size(&[(1, 1), (8, 8)]), 16);
        // Tuple (integer, byte): integer at 0, byte at 8 → total 9 (no padding,
        // byte after integer needs no align bump).
        assert_eq!(LinkedFieldGroup::group_size(&[(8, 8), (1, 1)]), 9);
        // Tuple (byte, single): byte at 0, padding 1..3, single at 4 → total 8
        assert_eq!(LinkedFieldGroup::group_size(&[(1, 1), (4, 4)]), 8);
    }

    #[test]
    fn index_group_carries_correct_alignment_metadata() {
        // After registering an index, the group's `alignment` field
        // must be 4 (max of int4=4, int4=4, bool=1).
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let tp = struct_with_single_field(&mut s, "ItemJ", int_c);
        let _idx = s.index(tp, &[("value".to_string(), true)]);
        s.finish();

        let group = s.types[tp as usize].index_groups().next().expect("group");
        assert_eq!(
            group.alignment, 4,
            "index group alignment must be 4 (max of int4, int4, bool)"
        );
        assert_eq!(
            group.size, 9,
            "index group atomic size must be 9 bytes (4 + 4 + 1, no internal padding)"
        );
    }

    #[test]
    fn index_group_alignment_drives_first_field_position() {
        // Verify that the first field of the group (after `finish`)
        // lands on a position divisible by `group.alignment`.
        // This is the LAYOUT INVARIANT the routine must honour:
        // **the group's alignment is the max member alignment**.
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let tp = struct_with_single_field(&mut s, "ItemK", int_c);
        let _idx = s.index(tp, &[("value".to_string(), true)]);
        s.finish();

        let indices = s.types[tp as usize]
            .index_groups()
            .next()
            .expect("group")
            .field_indices
            .clone();
        let group = s.types[tp as usize]
            .index_groups()
            .next()
            .expect("group")
            .clone();
        if let Parts::Struct(fields) = &s.types[tp as usize].parts {
            let first_pos = fields[indices[0] as usize].position;
            assert_eq!(
                first_pos as u8 % group.alignment,
                0,
                "group's first field at position {} must be aligned to {}",
                first_pos,
                group.alignment,
            );
        }
    }

    #[test]
    fn index_group_with_byte_user_field_alignment_metadata_unchanged() {
        // Mixed-alignment user fields don't affect the GROUP's
        // declared alignment — it's still 4 because the int4 members
        // need 4-byte boundaries.  The layout routine MUST honour
        // this even when other small-aligned fields are present.
        let mut s = Stores::new();
        let byte_c = s.byte(0, false);
        let tp = s.structure("ItemL", 0);
        s.field(tp, "flag", byte_c);
        let _idx = s.index(tp, &[("flag".to_string(), true)]);
        s.finish();

        let group = s.types[tp as usize].index_groups().next().expect("group");
        // Group's declared alignment is 4 regardless of user field size.
        assert_eq!(group.alignment, 4);
        // First member (#left_1) lands on a 4-byte boundary.
        let l_idx = group.field_indices[0];
        if let Parts::Struct(fields) = &s.types[tp as usize].parts {
            assert_eq!(
                fields[l_idx as usize].position as u8 % 4,
                0,
                "#left_1 must be 4-byte aligned even when user struct has 1-byte fields",
            );
        }
    }

    #[test]
    fn multiple_index_groups_each_aligned_independently() {
        // Each registered group keeps its own alignment metadata,
        // and each group's first member must land on its declared
        // alignment boundary.
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let txt_c = s.name("text");
        let tp = s.structure("ItemM", 0);
        s.field(tp, "name", txt_c);
        s.field(tp, "value", int_c);
        let _ = s.index(tp, &[("name".to_string(), true)]);
        let _ = s.index(tp, &[("value".to_string(), true)]);
        s.finish();

        let groups: Vec<_> = s.types[tp as usize].index_groups().cloned().collect();
        assert_eq!(groups.len(), 2);
        for group in &groups {
            assert_eq!(group.alignment, 4, "every index group has alignment 4");
            assert_eq!(group.size, 9, "every index group has size 9");
            if let Parts::Struct(fields) = &s.types[tp as usize].parts {
                let l_idx = group.field_indices[0];
                let pos = fields[l_idx as usize].position;
                assert_eq!(
                    pos as u8 % group.alignment,
                    0,
                    "group instance {} #left at position {} not aligned to {}",
                    group.instance,
                    pos,
                    group.alignment,
                );
            }
        }
    }

    // ───────────────────────────────────────────────────────────────
    // Safe member access — the new infrastructure must let consumers
    // walk every member of a registered group via
    // `member_field_index(i)` → host fields → `fields[idx].position`,
    // returning a valid offset for every member after `finish`.  No
    // string-prefix matching, no ad-hoc index arithmetic.
    // ───────────────────────────────────────────────────────────────

    #[test]
    fn safe_member_access_via_member_field_index_returns_all_fields() {
        // Walk every member of an index group via the public API,
        // verify each yields a real field with a valid position.
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let tp = struct_with_single_field(&mut s, "ItemSafe1", int_c);
        let _ = s.index(tp, &[("value".to_string(), true)]);
        s.finish();

        let group = s.types[tp as usize]
            .index_groups()
            .next()
            .expect("group")
            .clone();

        if let Parts::Struct(fields) = &s.types[tp as usize].parts {
            // Every group member must yield a valid field.
            for member_idx in 0..group.arity() {
                let field_idx = group
                    .member_field_index(member_idx)
                    .expect("member index in range");
                let field = &fields[field_idx as usize];
                assert_ne!(
                    field.position,
                    u16::MAX,
                    "member {member_idx} (field '{}') has no position assigned",
                    field.name,
                );
                assert!(
                    field.name.starts_with('#'),
                    "member {member_idx} should be bookkeeping, got '{}'",
                    field.name,
                );
            }
            // Out-of-range member returns None — no panic.
            assert!(group.member_field_index(group.arity()).is_none());
            assert!(group.member_field_index(usize::MAX).is_none());
        }
    }

    #[test]
    fn safe_access_member_positions_respect_first_field_alignment() {
        // The first member's position must be aligned to the group's
        // alignment.  Subsequent members are at consecutive ascending
        // positions per `calculate_positions`'s assignment.  Test
        // verifies the GROUP's claimed alignment is honoured for the
        // anchor point.
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let tp = struct_with_single_field(&mut s, "ItemSafe2", int_c);
        let _ = s.index(tp, &[("value".to_string(), true)]);
        s.finish();

        let group = s.types[tp as usize]
            .index_groups()
            .next()
            .expect("group")
            .clone();

        if let Parts::Struct(fields) = &s.types[tp as usize].parts {
            let first_idx = group.member_field_index(0).unwrap();
            let first_pos = fields[first_idx as usize].position;
            assert_eq!(
                first_pos as u8 % group.alignment,
                0,
                "first member at position {first_pos} not aligned to group.alignment={}",
                group.alignment,
            );
        }
    }

    #[test]
    #[allow(clippy::many_single_char_names)]
    fn safe_access_yields_distinct_non_overlapping_positions() {
        // Every member's position must be DISTINCT from every other
        // member's, AND no two members can share bytes in the
        // host struct.  This guards against off-by-one in
        // `field_indices` registration.
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let tp = struct_with_single_field(&mut s, "ItemSafe3", int_c);
        let _ = s.index(tp, &[("value".to_string(), true)]);
        s.finish();

        let group = s.types[tp as usize]
            .index_groups()
            .next()
            .expect("group")
            .clone();

        // Resolve each member's (position, content_size) for byte-range comparison.
        let int4 = s.int(0, false);
        let int4_size = s.types[int4 as usize].size;
        let bool_c = s.name("boolean");
        let bool_size = s.types[bool_c as usize].size;

        let ranges: Vec<(u16, u16)> = if let Parts::Struct(fields) = &s.types[tp as usize].parts {
            (0..group.arity())
                .map(|i| {
                    let idx = group.member_field_index(i).unwrap();
                    let pos = fields[idx as usize].position;
                    let sz = if fields[idx as usize].content == bool_c {
                        bool_size
                    } else {
                        int4_size
                    };
                    (pos, pos + sz)
                })
                .collect()
        } else {
            panic!("not a struct")
        };

        // No range overlaps any other.
        for i in 0..ranges.len() {
            for j in (i + 1)..ranges.len() {
                let (a, b) = ranges[i];
                let (c, d) = ranges[j];
                assert!(
                    b <= c || d <= a,
                    "members {i}={:?} and {j}={:?} overlap",
                    ranges[i],
                    ranges[j],
                );
            }
        }
    }

    #[test]
    fn safe_access_through_multiple_index_groups() {
        // With two indexes, every member of every group must be
        // independently accessible and yield a valid distinct
        // position.  6 distinct positions total (3 per group × 2
        // groups), all accessible without name-string parsing.
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let txt_c = s.name("text");
        let tp = s.structure("ItemSafe4", 0);
        s.field(tp, "name", txt_c);
        s.field(tp, "value", int_c);
        let _ = s.index(tp, &[("name".to_string(), true)]);
        let _ = s.index(tp, &[("value".to_string(), true)]);
        s.finish();

        let groups: Vec<_> = s.types[tp as usize].index_groups().cloned().collect();
        assert_eq!(groups.len(), 2);

        let mut all_positions: Vec<u16> = Vec::new();
        if let Parts::Struct(fields) = &s.types[tp as usize].parts {
            for group in &groups {
                for member_idx in 0..group.arity() {
                    let idx = group
                        .member_field_index(member_idx)
                        .expect("member in range");
                    let field = &fields[idx as usize];
                    assert_ne!(field.position, u16::MAX);
                    // Verify the field's name carries the group's instance
                    // suffix — sanity check that field_indices points where
                    // we think.
                    assert!(
                        field.name.ends_with(&format!("_{}", group.instance)),
                        "field '{}' should end with '_{}'",
                        field.name,
                        group.instance,
                    );
                    all_positions.push(field.position);
                }
            }
        }
        // 2 groups × 3 members = 6 positions, all distinct.
        all_positions.sort_unstable();
        let original_len = all_positions.len();
        all_positions.dedup();
        assert_eq!(
            all_positions.len(),
            original_len,
            "all 6 member positions must be distinct, got duplicates",
        );
    }

    #[test]
    fn index_groups_iterator_excludes_non_index_groups() {
        // `index_groups()` filter must ignore any future Tuple-kind
        // entries on Type::field_groups.  Today no Tuple groups land
        // there (tuples live on Definition::field_groups), but this
        // test guards the filter against accidental contamination.
        let mut s = Stores::new();
        let int_c = s.name("integer");
        let tp = s.structure("ItemI", 0);
        s.field(tp, "value", int_c);
        let _ = s.index(tp, &[("value".to_string(), true)]);
        s.finish();

        // Inject a fake Tuple group to confirm the filter excludes it.
        s.types[tp as usize]
            .field_groups
            .push(crate::data::LinkedFieldGroup {
                kind: crate::data::LinkedFieldKind::Tuple,
                instance: 0,
                field_indices: vec![0],
                alignment: 1,
                size: 0,
            });
        let total = s.types[tp as usize].field_groups.len();
        let index_only: Vec<_> = s.types[tp as usize].index_groups().collect();
        assert_eq!(total, 2, "should have one Index + one Tuple group");
        assert_eq!(index_only.len(), 1, "index_groups() must filter out Tuple");
        assert_eq!(index_only[0].kind, LinkedFieldKind::Index);
    }
}
