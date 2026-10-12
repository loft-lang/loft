// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)
//
// @PLN129 arc B — deriving the query from the store's own schema.
//
// The plan is named for this: a lookup that misses fetches exactly that record,
// from a query nobody wrote. The descriptor carries the type name, the field
// names and offsets, and — because loft's collection KINDS are query shapes —
// which fields are the key and in which direction they are ordered. That is a
// SELECT, and nothing here is enumerated ahead of time.
//
// Pure and unwired by design: it reads a `LayoutDesc` and returns a string, so
// it is testable with no database, no connection and no store.

use crate::database::{Iterated, LayoutDesc, LayoutNode};

/// What the caller is ASKING for, which is not always what the collection kind
/// can serve.
///
/// A keyed lookup on a `sorted` collection is an equality query; a slice of the
/// same collection is a range. The kind decides what is POSSIBLE — a `hash` has
/// no order, so it can never serve a range — and this decides which of the
/// possible shapes is wanted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryShape {
    /// Every key field pinned: at most one row (`WHERE id = ?`).
    Equality,
    /// The leading key fields pinned and the last one bounded, in key order
    /// (`WHERE person_id = ? AND started BETWEEN ? AND ?  ORDER BY started ASC`).
    ///
    /// This is also the answer to the N+1 pattern: one query returning many rows
    /// instead of a lookup per row (README failure path 8).
    Range,
    /// B2 — a predicate the collection's keys cannot express (`name LIKE 'Ada%'`,
    /// an aggregate, a column that is not a key), supplied verbatim by the
    /// caller.
    ///
    /// A shape rather than a second builder: the table and the columns are still
    /// derived, and only the `WHERE` comes from outside. That is what makes a row
    /// arriving this way indistinguishable from one arriving by key — which is
    /// the requirement the whole model rests on (QUERIES.md § one record, however
    /// it arrived).
    Filter(String),
}

/// How one placeholder is bound — which key field, and which end of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    Eq,
    Low,
    High,
}

/// One `?` in the derived SQL, in the order the placeholders appear.
///
/// `field` indexes the element's record fields, which is the same numbering the
/// collection's own key list uses — so a caller holding a key in collection
/// order binds positionally and cannot misalign the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Param {
    pub field: usize,
    pub bound: Bound,
}

/// One selected column and the record field it fills.
///
/// Carried rather than re-derived: the SELECT's column order and the field order
/// a row is written back into are the same fact, and deriving it twice is how
/// they drift.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedColumn {
    pub column: String,
    pub field: usize,
}

/// A derived query: the text, what its placeholders want, and where its columns
/// land.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sql {
    pub text: String,
    pub columns: Vec<SelectedColumn>,
    pub params: Vec<Param>,
}

/// How this engine spells an identifier.
///
/// It has to be declared because it cannot be derived: `"person"` is an
/// identifier in standard SQL and a string literal in MySQL, and no property of
/// the NAME says which engine is listening.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quoting {
    /// `"person"` — standard SQL: PostgreSQL, SQLite, DuckDB.
    ///
    /// The default, and it is what makes the plan's own motivating table work
    /// unquoted-would-not: `from` and `to` are ordinary loft field names for a
    /// history row and reserved words in every engine, so a bare identifier
    /// produces a query that parses nowhere. Quoting everything costs nothing
    /// and removes the whole class — no reserved-word list, and none to keep up
    /// to date as engines add words.
    #[default]
    Double,
    /// `` `person` `` — MySQL and MariaDB, unless `ANSI_QUOTES` is set.
    Backtick,
    /// The name verbatim. Available for a caller who knows the identifiers are
    /// safe and wants the query to read the way they wrote it; a name that is
    /// not a plain identifier is then REFUSED rather than emitted.
    Bare,
}

/// How this engine spells a bound parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Placeholder {
    /// `?` — SQLite, MySQL, MariaDB.
    #[default]
    Question,
    /// `$1`, `$2`, … — PostgreSQL, numbered from one in the order they appear.
    Numbered,
}

/// Is this a legal unquoted SQL identifier?
fn plain_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The names loft cannot derive, plus how this engine spells things.
///
/// The descriptor gives a type name and field names; the DATABASE owns the
/// schema and owes loft neither ([BINDING.md](../../doc/claude/plans/129-lazy-database-stores/BINDING.md)).
/// So `Person.name` may be `persoon.naam`, and that fact has to be stated
/// somewhere. An empty mapping is exactly the derivation's default, which is why
/// there is one builder rather than a default path and an override path.
#[derive(Debug, Clone, Default)]
pub struct Mapping {
    tables: std::collections::BTreeMap<String, String>,
    columns: std::collections::BTreeMap<(String, String), String>,
    pub quoting: Quoting,
    pub placeholder: Placeholder,
}

impl Mapping {
    /// A mapping with no overrides, for `quoting` and `placeholder`.
    #[must_use]
    pub fn new(quoting: Quoting, placeholder: Placeholder) -> Mapping {
        Mapping {
            tables: std::collections::BTreeMap::new(),
            columns: std::collections::BTreeMap::new(),
            quoting,
            placeholder,
        }
    }

    /// Declare that loft type `type_name` lives in table `table`.
    ///
    /// # Errors
    /// When the descriptor has no such type — a typo in a mapping is otherwise
    /// invisible, because the derivation would simply use the default and query
    /// a table nobody meant.
    pub fn map_table(
        &mut self,
        desc: &LayoutDesc,
        type_name: &str,
        table: &str,
    ) -> Result<(), String> {
        if !desc.names.values().any(|n| n == type_name) {
            return Err(format!("no type `{type_name}` in this schema"));
        }
        self.tables.insert(type_name.to_string(), table.to_string());
        Ok(())
    }

    /// Declare that `type_name.field` is column `column`.
    ///
    /// # Errors
    /// When the type is not in the descriptor, or has no such field. Checked
    /// HERE rather than at query time, because a mapping is written once and a
    /// query runs on a miss — the error belongs where the mistake is.
    pub fn map_column(
        &mut self,
        desc: &LayoutDesc,
        type_name: &str,
        field: &str,
        column: &str,
    ) -> Result<(), String> {
        let Some((id, _)) = desc.names.iter().find(|(_, n)| *n == type_name) else {
            return Err(format!("no type `{type_name}` in this schema"));
        };
        let Some(LayoutNode::Record(fields)) = desc.nodes.get(id) else {
            return Err(format!("type `{type_name}` is not a record"));
        };
        if !fields.iter().any(|f| f.is_data() && f.name == field) {
            return Err(format!("type `{type_name}` has no field `{field}`"));
        }
        self.columns.insert(
            (type_name.to_string(), field.to_string()),
            column.to_string(),
        );
        Ok(())
    }

    /// The table for `type_name` — declared, or the type's own name lowercased.
    ///
    /// Lowercase because that is the spelling that means the same thing on every
    /// engine: PostgreSQL folds an unquoted name down, and a table created the
    /// ordinary way is therefore already lowercase. A table that really is
    /// mixed-case is what the override is for.
    fn table(&self, type_name: &str) -> String {
        self.tables
            .get(type_name)
            .cloned()
            .unwrap_or_else(|| type_name.to_lowercase())
    }

    /// The column for `type_name.field` — declared, or the field's own name.
    fn column(&self, type_name: &str, field: &str) -> String {
        self.columns
            .get(&(type_name.to_string(), field.to_string()))
            .cloned()
            .unwrap_or_else(|| field.to_string())
    }

    /// Spell one identifier for this engine. `None` when `Bare` was asked for a
    /// name that cannot be written bare.
    fn quote(&self, name: &str) -> Option<String> {
        // Doubling is how both dialects escape their own quote character, so a
        // name containing one still round-trips rather than closing the
        // identifier early.
        Some(match self.quoting {
            Quoting::Double => format!("\"{}\"", name.replace('"', "\"\"")),
            Quoting::Backtick => format!("`{}`", name.replace('`', "``")),
            Quoting::Bare if plain_identifier(name) => name.to_string(),
            Quoting::Bare => return None,
        })
    }

    /// The nth placeholder, numbered from zero.
    fn placeholder(&self, n: usize) -> String {
        match self.placeholder {
            Placeholder::Question => "?".to_string(),
            Placeholder::Numbered => format!("${}", n + 1),
        }
    }
}

/// Can this field be one column?
///
/// The seed scalars and `text` are columns. Everything else is not: a nested
/// struct is another table's row, a vector is many rows, and a stored pointer is
/// meaningless outside this process. A reference at this boundary is a foreign
/// KEY — an ordinary integer or text field — so it is covered here, and following
/// it is another lookup rather than a join (README § the traversal IS the join).
///
/// **A narrow integer (`i32`, `u8`, `size(2)`) is refused, and that is a named
/// gap rather than an oversight.** Writing one back means choosing between four
/// encodings and their null sentinels, and a nullable narrow field is written
/// through a different setter again — so it waits for a cell that can be checked
/// rather than being guessed at now. Refusing is loud; guessing would put wrong
/// numbers in the record.
fn scalar_column(desc: &LayoutDesc, content: u16) -> bool {
    matches!(desc.nodes.get(&content), Some(LayoutNode::Base(_)))
}

/// The key fields of a collection, as `(field index, ascending)` pairs, plus
/// whether the kind has an ORDER at all.
///
/// Spelled out per kind rather than closed with `_`: a kind whose query shape
/// nobody decided must not silently inherit another kind's
/// ([DATABASE.md § Adding or changing a collection kind](../../doc/claude/DATABASE.md)).
fn key_fields(it: &Iterated) -> Option<(Vec<(usize, bool)>, bool)> {
    Some(match it {
        Iterated::Hash { keys, .. } => (keys.iter().map(|k| (*k as usize, true)).collect(), false),
        Iterated::Sorted { keys, .. } | Iterated::Ordered { keys, .. } => (
            keys.iter().map(|(k, asc)| (*k as usize, *asc)).collect(),
            true,
        ),
        Iterated::Index { keys, .. } => (
            keys.iter().map(|(k, asc)| (*k as usize, *asc)).collect(),
            true,
        ),
        // A spatial index is a Morton-order structure over coordinates, and SQL
        // has no shape that means the same thing. Refusing is the honest answer:
        // the alternative is a bounding-box scan that looks like a lazy fetch and
        // reads the table (README matrix — "a kind with no mapping must refuse
        // rather than scan").
        Iterated::Radix { .. } => return None,
        // A trie DOES have a SQL shape, and it is `sorted`'s: one key, ascending,
        // answering equality and a range on it. Its PREFIX is a different shape
        // (`LIKE 'x%'`) that `key_fields` cannot express, so it is simply not
        // offered here rather than approximated by a range.
        //
        // The caveat it inherits is `sorted<T[text]>`'s, not a new one: a trie
        // orders BYTES and SQL orders by collation, so a range's boundary rows can
        // differ between the two. Same fact, same treatment — refusing a trie while
        // answering for `sorted` would be inconsistent, not safer.
        Iterated::Trie { key, .. } => (vec![(*key as usize, true)], true),
    })
}

/// Derive the `SELECT` that serves `shape` on the collection type `collection`,
/// spelling every name the way `map` says.
///
/// `None` whenever the schema cannot say what the query is — a kind with no SQL
/// shape, a range asked of an unordered kind, an element that is not a record, a
/// field that is not a column, or (under [`Quoting::Bare`]) a name that cannot be
/// written unquoted. A refusal is the point: a malformed query that runs is worse
/// than one that was never built.
///
/// An empty `Mapping` is the pure derivation — the default and the override feed
/// this one builder, so the two cannot drift apart.
#[must_use]
pub fn derive_select(
    desc: &LayoutDesc,
    collection: u16,
    shape: &QueryShape,
    map: &Mapping,
) -> Option<Sql> {
    let LayoutNode::Iterated(it) = desc.nodes.get(&collection)? else {
        return None;
    };
    let elem = it.elem();
    // A `__nullable<S>` element keys through its `Some` payload, so its node is
    // not a record and it lands here. Refusing is right until the mapping can
    // say what the column of a nullable element is.
    let LayoutNode::Record(fields) = desc.nodes.get(&elem)? else {
        return None;
    };
    let (keys, ordered) = key_fields(it)?;
    if keys.is_empty() || keys.iter().any(|(k, _)| *k >= fields.len()) {
        return None;
    }
    if *shape == QueryShape::Range && !ordered {
        return None; // a hash has no order to range over
    }

    let type_name = desc.names.get(&elem)?;
    let table = map.quote(&map.table(type_name))?;

    // Only the fields the PROGRAM wrote become columns: a `#`-prefixed synthetic
    // field can have an ordinary type, so filtering by field TYPE alone would
    // select a column no table has (`LayoutField::is_data`).
    let mut columns = Vec::with_capacity(fields.len());
    for (i, f) in fields.iter().enumerate() {
        if !f.is_data() {
            continue;
        }
        if !scalar_column(desc, f.content) {
            return None;
        }
        columns.push(SelectedColumn {
            column: map.quote(&map.column(type_name, &f.name))?,
            field: i,
        });
    }
    if columns.is_empty() {
        return None;
    }

    let mut params = Vec::with_capacity(keys.len() + 1);
    let mut wheres = Vec::with_capacity(keys.len());
    // The trailing key is the one a range bounds; every key before it is pinned,
    // which is what makes `index<Position[person_id, started]>` a composite range
    // rather than N lookups.
    let last = keys.len() - 1;
    if let QueryShape::Filter(condition) = shape {
        // The caller's predicate, verbatim and unbound. It cannot be
        // parameterised — loft does not parse it and so cannot know where its
        // values are — which is why this shape is EXPLICIT in the loft source
        // and a keyed lookup is not.
        wheres.push(condition.clone());
    }
    for (n, (field, _)) in keys.iter().enumerate() {
        if let QueryShape::Filter(_) = shape {
            break; // the predicate replaces the key, it does not extend it
        }
        let col = map.quote(&map.column(type_name, &fields[*field].name))?;
        if *shape == QueryShape::Range && n == last {
            wheres.push(format!(
                "{col} BETWEEN {} AND {}",
                map.placeholder(params.len()),
                map.placeholder(params.len() + 1)
            ));
            params.push(Param {
                field: *field,
                bound: Bound::Low,
            });
            params.push(Param {
                field: *field,
                bound: Bound::High,
            });
        } else {
            wheres.push(format!("{col} = {}", map.placeholder(params.len())));
            params.push(Param {
                field: *field,
                bound: Bound::Eq,
            });
        }
    }

    let cols = columns
        .iter()
        .map(|c| c.column.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let mut text = format!("SELECT {cols} FROM {table} WHERE {}", wheres.join(" AND "));
    if *shape == QueryShape::Range {
        // Only the ranged key is ordered. The keys before it are pinned to one
        // value each, so ordering by them says nothing — and the direction comes
        // from the collection's own declaration, never from a convention.
        use std::fmt::Write as _;
        let (field, asc) = keys[last];
        let dir = if asc { "ASC" } else { "DESC" };
        let col = map.quote(&map.column(type_name, &fields[field].name))?;
        let _ = write!(text, " ORDER BY {col} {dir}");
    }

    Some(Sql {
        text,
        columns,
        params,
    })
}
