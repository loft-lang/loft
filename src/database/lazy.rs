// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)
//
// @PLN129 arc B — the SOURCE seam: what a lazy collection consults on a miss.
//
// Arc A had one source (a `.store` image) and reached it inline from
// `fetch_missing`. Arc B adds a second (a relational database), and the two
// differ in every detail except the question they answer — so the question is
// what lives here, and the details live in one implementation each.

use crate::database::Stores;
use crate::keys::{Content, DbRef};

// Only `row_key` reads the layout's parts, and that is the SQL half — so on a build
// without it (wasm) the import is dead and warns.
#[cfg(feature = "native-extensions")]
use crate::database::Parts;

/// What a lazy collection is bound to, derived from the binding's source string.
///
/// Derived rather than declared: `store_bind_lazy` takes one string, and which
/// kind of source it names is a fact about the string. That keeps the loft
/// surface at one function and lets a new source arrive without a new builtin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LazySource {
    /// A `.store` image — a local path or an `http(s)` URL read by page
    /// (`REMOTE_STORES.md`). loft's own bytes, so a fetch is a range read.
    File(String),
    /// A relational database, named by a driver prefix (`sqlite:people.db`). A
    /// foreign schema, so a fetch is a derived query and a row to materialise.
    #[cfg(feature = "native-extensions")]
    Sql(crate::database::sql_source::Driver, String),
    /// @PLN133 S8 — a database core has **no Rust driver for**, and loft does.
    ///
    /// Core binds one backend (sqlite); the loft library binds four behind one
    /// interface. Rather than restate the other three in Rust — N drivers now
    /// and +1 forever, with the loft versions left to drift — a source named by
    /// one of their schemes is fetched by calling loft
    /// ([`crate::state::State::run_until_return`]).
    ///
    /// **Its own variant rather than a fall-through**, because the fall-through
    /// is what failure path 1 is about: before this, `postgres://…` classified
    /// as a `.store` image and a miss reported something about a paged reader.
    /// An unreachable backend must read as unreachable.
    Loft(String),
}

/// The database schemes core routes to a loft driver.
///
/// The list is the loft library's four backends minus sqlite, which core has in
/// Rust and which stays there — it is the CONTROL for the swap (@PLN133 S8/S9).
/// Aliases are admitted because they are what a DSN already spells.
const LOFT_SCHEMES: [&str; 8] = [
    "postgres",
    "postgresql",
    "pg",
    "mysql",
    "mariadb",
    "maria",
    "duckdb",
    "duck",
];

impl LazySource {
    /// Classify a binding's source string.
    #[must_use]
    pub fn of(source: &str) -> LazySource {
        #[cfg(feature = "native-extensions")]
        if let Some((driver, target)) = crate::database::sql_source::driver_of(source) {
            return LazySource::Sql(driver, target.to_string());
        }
        // The scheme is what precedes the first `:`, and only a scheme this
        // build recognises as a DATABASE routes to loft. A Windows path
        // (`C:\data\people.store`) has a colon and is not a scheme, which is why
        // the list is closed rather than "anything before a colon".
        if let Some((scheme, _)) = source.split_once(':')
            && LOFT_SCHEMES.contains(&scheme.to_lowercase().as_str())
        {
            return LazySource::Loft(source.to_string());
        }
        LazySource::File(source.to_string())
    }
}

/// The answer a source gives to *"fetch the entry for this key"* — three
/// outcomes, because two of them are the reason arc C exists.
///
/// `Absent` and `Unreachable` are the same `null` to a loft program and must
/// never be the same fact here: "no such person" is stable and true, "the source
/// is down" is neither (README failure path 1).
#[derive(Debug)]
pub enum Fetched {
    /// The entry is now IN the collection. The caller re-runs the lookup rather
    /// than trusting a ref from here — the collection stays the only authority
    /// on what is resident.
    Inserted,
    /// The source was reached and does not hold this key.
    Absent,
    /// The source could not be consulted, or cannot serve this collection at
    /// all; the string says why, and it is what a program reads back through
    /// `store_lazy_error`.
    Unreachable(String),
}

/// What a collection with no driver reports, in ONE place.
///
/// Both backends reach a miss by different routes — the interpreter looks the
/// driver up in `Data`, `--native` reads a pointer generated `init()` installed —
/// and the gate for @PLN133 S8 compares their whole output rather than a line at
/// a time. A message written twice is the first thing that would diverge, and it
/// would diverge in the one place a user is already confused.
///
/// It names the ELEMENT TYPE, because that is what has no driver. Naming only the
/// source would send someone to look at a connection string that is perfectly
/// fine.
#[must_use]
pub fn no_lazy_driver(source: &str, element: &str) -> String {
    format!(
        "`{source}` needs a loft driver for {element} — define `fn lazy_fetch(coll: <the \
         {element} collection>, source: text, key_int: integer, key_text: text) -> integer` \
         and bind the collection again"
    )
}

impl Stores {
    /// Ask one source for one key. The dispatch that makes the seam a seam.
    ///
    /// `db` is read by the SQL arm alone, so a build without it (wasm) sees an unused
    /// parameter. Kept in the signature rather than renamed: it is part of the seam's
    /// shape, and which sources a build happens to carry should not change it.
    #[cfg_attr(
        not(feature = "native-extensions"),
        allow(
            unused_variables,
            clippy::needless_pass_by_value,
            // Same reason as `db` above: without the SQL arm nothing in the body
            // reaches `self`.  The seam's shape is not a per-build fact.
            clippy::unused_self
        )
    )]
    pub(crate) fn fetch_from_source(
        &mut self,
        data: &DbRef,
        db: u16,
        src: &LazySource,
        key: &[Content],
    ) -> Fetched {
        match src {
            #[cfg(paged_store)]
            LazySource::File(path) => self.fetch_from_file(path, data, key),
            #[cfg(not(paged_store))]
            LazySource::File(path) => Fetched::Unreachable(format!(
                "`{path}` needs the paged reader, which this build does not have"
            )),
            #[cfg(feature = "native-extensions")]
            LazySource::Sql(driver, target) => self.fetch_from_sql(*driver, target, data, db, key),
            // Never reached through here: `State` peels a loft source off
            // BEFORE calling the fetch, because running a loft function needs a
            // `State` and this has only a `Stores`. Answering rather than
            // panicking keeps the seam total — a caller that has not been
            // taught the split gets a refusal, not a crash.
            LazySource::Loft(source) => Fetched::Unreachable(format!(
                "`{source}` is served by a loft driver, and this lookup did not route to one"
            )),
        }
    }

    /// @PLN133 S8/S9 — the binding's source, when a LOFT driver is what serves it.
    ///
    /// Asked before the fetch rather than inside it, because the answer decides
    /// WHO fetches: `Stores` cannot run a loft function and `State` can, so the
    /// two callers of the miss path make this call while they still hold both.
    ///
    /// **S9 — a declared driver WINS, including over a source core drives in
    /// Rust.** Core binds sqlite and the loft library binds four behind one
    /// interface; this is how a program moves its sqlite reads onto the loft
    /// driver, one element type at a time, with the Rust source still serving
    /// every type that has none. That is what makes the swap measurable rather
    /// than a flag day: the same program, the same database, one collection on
    /// each path, and @PLN129's count assertions comparing them.
    ///
    /// `have_driver` is the caller's answer to *"is there a driver for this
    /// collection's element type"*, because the two backends learn it
    /// differently — the interpreter asks `Data`, `--native` asks the table
    /// generated `init()` filled — and neither of those is reachable from here.
    #[must_use]
    // `LazySource`'s variant set is cfg-dependent (`Sql` needs the loader), so
    // the catch-all covers two variants in a full build and one without it.
    // Naming `File(_)` instead would make the arm wrong the moment a source is
    // added — the arm means "anything core can drive", not one variant.
    #[cfg_attr(
        not(feature = "native-extensions"),
        allow(clippy::match_wildcard_for_single_variants)
    )]
    pub fn lazy_loft_source(&self, coll: &DbRef, have_driver: bool) -> Option<String> {
        let source = self.lazy_source(coll)?;
        match LazySource::of(&source) {
            // No Rust driver exists for it, so loft is the only answer — and when
            // the program has none either, the refusal is what the caller reports.
            LazySource::Loft(s) => Some(s),
            // A source core CAN drive. The program's own driver takes it only if
            // it declared one for this type; otherwise nothing changes.
            _ if have_driver => Some(source),
            _ => None,
        }
    }

    /// @PLN133 S9 — the name of the ELEMENT type a collection holds.
    ///
    /// The key a lazy driver is looked up by, and the reason it is a name: the
    /// driver is declared in parse-time `Data` and reached from runtime
    /// `Stores`, which count types in different spaces. A name is the one key
    /// both hold without a mapping that has to be kept in step — and @PLN133 S8's
    /// own `LOFT_STRICT_SCHEMA_IDS` exists because that kind of mapping drifts.
    ///
    /// `""` for a type that holds no element, which no keyed lookup reaches.
    #[must_use]
    pub fn element_type_name(&self, db_tp: u16) -> &str {
        let c = self.content(db_tp);
        if c == u16::MAX {
            return "";
        }
        &self.types[c as usize].name
    }

    /// Record a refusal against a collection, through arc C's channel.
    ///
    /// Public because @PLN133 S8's fetch runs in `State`, which is where the
    /// refusal is now decided; the channel and its sticky first-reason rule are
    /// unchanged.
    pub fn lazy_fail(&mut self, coll: &DbRef, why: &str) {
        self.lazy_refuse((coll.store_nr, coll.rec, coll.pos), why);
    }

    /// @PLN133 S8 — start remembering the stores a lazy DRIVER creates.
    ///
    /// A raise in loft short-circuits the dispatch loop, so the scope-exit frees
    /// the compiler emitted never run. For a program that is about to exit that
    /// is harmless and the suite grandfathers it; for a CONTAINED fault it is
    /// not, because the program continues and a traversal over an unreachable
    /// source would leak once per failed fetch.
    ///
    /// Nested fetches nest: an inner window's stores are merged into the outer
    /// one when the inner call returns normally, so an outer fault still
    /// accounts for everything created underneath it.
    ///
    /// Answers the previous window, which the caller hands back to
    /// [`Stores::lazy_watch_end`].
    pub fn lazy_watch_begin(&mut self) -> Option<Vec<u16>> {
        self.lazy_driver_allocs.replace(Vec::new())
    }

    /// Close the window opened by [`Stores::lazy_watch_begin`].
    ///
    /// `faulted` decides what the window MEANS. On a normal return the stores
    /// are the program's — the driver's own scope-exit code freed what it
    /// owned, and what it inserted belongs to the collection — so they are
    /// merged into any enclosing window and nothing is freed. On a fault they
    /// are what the abandoned frames held, and freeing them is the teardown the
    /// fault skipped.
    ///
    /// Answers how many stores it freed, which is `0` on the normal path.
    pub fn lazy_watch_end(&mut self, outer: Option<Vec<u16>>, faulted: bool) -> usize {
        let mine = self.lazy_driver_allocs.take().unwrap_or_default();
        self.lazy_driver_allocs = outer;
        if !faulted {
            if let Some(up) = &mut self.lazy_driver_allocs {
                up.extend_from_slice(&mine);
            }
            return 0;
        }
        let mut freed = 0;
        for slot in mine {
            // A store the driver already freed, or one that was reused and is
            // live again, is not this window's to free: `free_named` no-ops on
            // an already-free slot, and the `free` check keeps the count honest.
            if (slot as usize) < self.allocations.len() && !self.allocations[slot as usize].free {
                self.free_named(
                    &DbRef {
                        store_nr: slot,
                        rec: 0,
                        pos: 0,
                    },
                    "lazy-driver-unwind",
                );
                freed += 1;
            }
        }
        freed
    }

    /// The `.store` image source — arc A's fetch, unchanged.
    ///
    /// The order is load-bearing and predates the seam: TRY the fetch, and only
    /// on failure ask whether the source was reachable at all. Asking first
    /// would put a stat on the path of every successful fault.
    #[cfg(paged_store)]
    fn fetch_from_file(&mut self, path: &str, data: &DbRef, key: &[Content]) -> Fetched {
        // Clear first, so what is read back after the call is THIS fetch's
        // refusal and not one an earlier `store_load_key` left behind.
        self.paged_refusal = None;
        // Routed by the COLLECTION's kind, not by the key's shape. A `spatial`
        // lookup arrives as one `Content::Long` per axis, and a one-axis one is
        // indistinguishable from a hash's integer key — so reading the shape would
        // send `spatial<T[x]>` to the hash loader and get a refusal for a binding
        // that works (@PLN136).
        let radix = {
            let root = self.allocations[data.store_nr as usize].known_type;
            let tp = self.collection_type_of_store(root);
            matches!(
                self.types.get(tp as usize).map(|t| &t.parts),
                Some(crate::database::Parts::Radix(..))
            )
        };
        // One key, whichever spelling it arrived in. A composite key that is NOT a
        // coordinate is not fetchable yet — @PLN125's associated types are what
        // would carry it — so it is left to answer absent rather than fetching the
        // wrong row.
        let hit = if radix {
            // A point is the degenerate box, and it loads every record AT that
            // point: a spatial collection keeps duplicates at one coordinate, so
            // taking the first would leave the rest permanently unreachable.
            let at: Option<Vec<i64>> = key
                .iter()
                .map(|c| match c {
                    Content::Long(v) => Some(*v),
                    _ => None,
                })
                .collect();
            match at {
                Some(at) if !at.is_empty() => self.load_box(data, path, &at, &at, -1) > 0,
                _ => false,
            }
        } else {
            match key {
                [Content::Long(k)] => self.load_key(data, path, *k),
                [Content::Str(s)] => self.load_key_text(data, path, s.str()),
                _ => false,
            }
        };
        if hit {
            return Fetched::Inserted;
        }
        let refusal = self.paged_refusal.take();
        match crate::paged_reader::PageSource::open(path) {
            // Unreachable is decided HERE and not by the recorded refusal, which
            // for this case says only "it cannot be opened as a paged source"
            // while the open itself names the connection error.
            Err(reason) => Fetched::Unreachable(reason),
            // The source opened, so a `false` from the loader is one of two
            // things and they are not the same fact (loft#802): it REFUSED the
            // shape — permanent, and no lookup will ever succeed — or it reached
            // the data and the key was not there. Answering `Absent` for both is
            // what let a refused binding report itself as healthy.
            Ok(_) => refusal.map_or(Fetched::Absent, Fetched::Unreachable),
        }
    }

    /// The database source — steps 5 and 6: derive the query, run it, and
    /// materialise the row into the collection.
    ///
    /// The collection is asked FIRST (by `find_or_fetch`) and re-asked after,
    /// which is arc A's rule unchanged: this never hands a record back
    /// directly, so the collection stays the only authority on what is resident
    /// and identity keeps falling out of it.
    #[cfg(feature = "native-extensions")]
    fn fetch_from_sql(
        &mut self,
        driver: crate::database::sql_source::Driver,
        target: &str,
        data: &DbRef,
        db: u16,
        key: &[Content],
    ) -> Fetched {
        use crate::database::sql_query::{Mapping, QueryShape, derive_select};
        use crate::database::sql_source::SqlConn;

        let desc = self.layout_descriptor(&[db]);
        let Some(sql) = derive_select(&desc, db, &QueryShape::Equality, &Mapping::default()) else {
            return Fetched::Unreachable(format!(
                "no query can be derived for `{}` — its element has a field that is \
                 not a column",
                self.types[db as usize].name
            ));
        };
        if sql.params.len() != key.len() {
            return Fetched::Unreachable(format!(
                "`{}` is keyed on {} column(s) and the lookup gave {}",
                self.types[db as usize].name,
                sql.params.len(),
                key.len()
            ));
        }
        let binds: Vec<crate::database::sql_source::Cell> = key.iter().map(cell_of).collect();
        let conn = match SqlConn::open(driver, target) {
            Ok(c) => c,
            Err(why) => return Fetched::Unreachable(why),
        };
        // Step 7 — the schema is interrogated ONCE, and a bind it cannot serve
        // never answers a lookup.
        if let Some(why) = self.schema_refusal(data, db, &desc, &sql, &conn) {
            return Fetched::Unreachable(why);
        }
        let rows = match conn.query(&sql.text, &binds) {
            Ok(r) => r,
            // A query that does not RUN is unreachable, never an absence: a
            // renamed column and a missing person must not read the same.
            Err(why) => return Fetched::Unreachable(why),
        };
        let Some(row) = rows.into_iter().next() else {
            return Fetched::Absent;
        };
        if row.len() != sql.columns.len() {
            return Fetched::Unreachable(format!(
                "`{target}` answered {} column(s) where the query asked {}",
                row.len(),
                sql.columns.len()
            ));
        }
        self.materialise(data, db, &sql, &row);
        Fetched::Inserted
    }

    /// @PLN129 arc B2 — run an explicit predicate against this collection's
    /// bound source and materialise every row it matches INTO the collection.
    /// Returns how many records the collection gained.
    ///
    /// The escape hatch for what the keys cannot express — `name LIKE 'Ada%'`, a
    /// predicate on a non-key column. Those cannot be derived from a layout, and
    /// a scan-then-filter would silently read the table, so this is EXPLICIT and
    /// visible in the source (QUERIES.md § what it cannot express).
    ///
    /// It is not a side channel returning a detached result set: rows land in the
    /// collection, and a row whose key is already resident is SKIPPED rather than
    /// materialised again. That is the rule the whole model rests on — the
    /// collection is asked first, always — and it is what makes a person reached
    /// by `LIKE` and a person reached by navigation the same record.
    #[cfg(feature = "native-extensions")]
    pub fn lazy_query(&mut self, coll: &DbRef, condition: &str) -> i64 {
        use crate::database::sql_query::{Mapping, QueryShape, derive_select};
        use crate::database::sql_source::SqlConn;

        let slot = (coll.store_nr, coll.rec, coll.pos);
        let Some(source) = self.lazy_source(coll) else {
            return self.lazy_refuse(slot, "this collection is not bound to a source");
        };
        let LazySource::Sql(driver, target) = LazySource::of(&source) else {
            return self.lazy_refuse(slot, &format!("`{source}` is not a database source"));
        };
        // The collection's TYPE, from the store it was allocated into — a
        // `reference` argument carries no type, and every derived name comes
        // from this one. Through the same resolver the paged loader uses, so a
        // collection declared as a struct FIELD (`struct Firm { people:
        // hash<Worker[id]> }`) resolves to the field's type rather than to the
        // wrapper's — which is what makes `company.people` an
        // owner-parameterised query rather than an unreachable shape (#632).
        let db = self.collection_type_of_store(self.allocations[coll.store_nr as usize].known_type);
        if db == u16::MAX {
            return self.lazy_refuse(
                slot,
                "this collection has no type to derive a query from — a struct \
                 wrapping several keyed collections cannot say which one this is",
            );
        }
        let desc = self.layout_descriptor(&[db]);
        let shape = QueryShape::Filter(condition.to_string());
        let Some(sql) = derive_select(&desc, db, &shape, &Mapping::default()) else {
            return self.lazy_refuse(
                slot,
                &format!(
                    "no query can be derived for `{}`",
                    self.types[db as usize].name
                ),
            );
        };
        let conn = match SqlConn::open(driver, &target) {
            Ok(c) => c,
            Err(why) => return self.lazy_refuse(slot, &why),
        };
        // An explicit predicate is NOT index-checked: the caller asked for
        // exactly this, a `LIKE` has no index to use, and refusing it would
        // remove the escape hatch this exists to be. The columns still have to
        // be there, and a query that does not run reports through arc C.
        let rows = match conn.query(&sql.text, &[]) {
            Ok(r) => r,
            Err(why) => return self.lazy_refuse(slot, &why),
        };
        let mut added = 0i64;
        for row in rows {
            if row.len() != sql.columns.len() {
                return self.lazy_refuse(slot, "the source answered a different column count");
            }
            let Some(key) = self.row_key(db, &desc, &sql, &row) else {
                return self.lazy_refuse(slot, "a row arrived without the collection's key");
            };
            // Ask the collection FIRST. Without this a person already fetched by
            // key would be materialised a second time, and `is_same` would answer
            // false for one obvious person (BINDING.md § the real cost).
            if self.find(coll, db, &key).rec != 0 {
                continue;
            }
            self.materialise(coll, db, &sql, &row);
            added += 1;
        }
        added
    }

    /// @PLN129 arc B step 8 — fetch a KEY RANGE from this collection's bound
    /// source in ONE query. Returns how many records the collection gained.
    ///
    /// This is the cure for N+1, and it is a cure rather than an optimisation:
    /// walking 500 employers one lookup at a time is 500 round trips, and that
    /// makes the natural way to write a traversal the pathological one (README
    /// failure path 8). One range slice is one query returning many rows.
    ///
    /// The collection has to be ORDERED (`sorted` / `index`) — a hash has no
    /// order to range over — and keyed on ONE column, because two numbers cannot
    /// say which value pins a composite key's leading column. A composite range
    /// is the explicit query's job until there is a shape that can carry it.
    #[cfg(feature = "native-extensions")]
    pub fn lazy_range(&mut self, coll: &DbRef, lo: i64, hi: i64) -> i64 {
        use crate::database::sql_query::{Mapping, QueryShape, derive_select};
        use crate::database::sql_source::{Cell, SqlConn};

        let slot = (coll.store_nr, coll.rec, coll.pos);
        let Some(source) = self.lazy_source(coll) else {
            return self.lazy_refuse(slot, "this collection is not bound to a source");
        };
        let LazySource::Sql(driver, target) = LazySource::of(&source) else {
            return self.lazy_refuse(slot, &format!("`{source}` is not a database source"));
        };
        let db = self.collection_type_of_store(self.allocations[coll.store_nr as usize].known_type);
        if db == u16::MAX {
            return self.lazy_refuse(slot, "this collection has no type to derive a query from");
        }
        let desc = self.layout_descriptor(&[db]);
        let Some(sql) = derive_select(&desc, db, &QueryShape::Range, &Mapping::default()) else {
            return self.lazy_refuse(
                slot,
                &format!(
                    "`{}` has no range to fetch — an ordered collection is what \
                     derives one",
                    self.types[db as usize].name
                ),
            );
        };
        if sql.params.len() != 2 {
            return self.lazy_refuse(
                slot,
                "a range fetch takes ONE key column; a composite key needs an \
                 explicit query",
            );
        }
        let conn = match SqlConn::open(driver, &target) {
            Ok(c) => c,
            Err(why) => return self.lazy_refuse(slot, &why),
        };
        if let Some(why) = self.schema_refusal(coll, db, &desc, &sql, &conn) {
            return self.lazy_refuse(slot, &why);
        }
        let rows = match conn.query(&sql.text, &[Cell::Int(lo), Cell::Int(hi)]) {
            Ok(r) => r,
            Err(why) => return self.lazy_refuse(slot, &why),
        };
        let mut added = 0i64;
        for row in rows {
            if row.len() != sql.columns.len() {
                return self.lazy_refuse(slot, "the source answered a different column count");
            }
            let Some(key) = self.row_key(db, &desc, &sql, &row) else {
                return self.lazy_refuse(slot, "a row arrived without the collection's key");
            };
            // The collection first, on this path too: a range overlapping what
            // is already resident must not duplicate it.
            if self.find(coll, db, &key).rec != 0 {
                continue;
            }
            self.materialise(coll, db, &sql, &row);
            added += 1;
        }
        added
    }

    /// Record why an explicit query could not run, and answer "nothing arrived".
    ///
    /// Through arc C's channel rather than a return code: a count cannot carry a
    /// reason, and `0` is also what a predicate matching nothing answers.
    ///
    /// NOT gated on `native-extensions`, though every other SQL-side helper here is:
    /// it only records into `lazy_errors`, which is an ungated field, and `lazy_fail`
    /// — public since @PLN133 S8 moved the refusal decision into `State` — calls it
    /// from code that compiles on every target. Gated, the wasm builds failed to
    /// compile it (`no method named lazy_refuse`), which is what took the browser,
    /// Web-Worker `par` and node-bridge jobs down together.
    pub(crate) fn lazy_refuse(&mut self, slot: (u16, u32, u32), why: &str) -> i64 {
        let entry = self.lazy_errors.entry(slot).or_insert((0, why.to_string()));
        entry.0 += 1;
        0
    }

    /// The collection key carried by one fetched row.
    ///
    /// Read through the FIELD's content type rather than the cell's, because
    /// `find` compares against `field_content`, which reads the stored field the
    /// same way — a key built from what SQLite happened to return would not
    /// compare equal to the identical record already resident.
    #[cfg(feature = "native-extensions")]
    fn row_key(
        &self,
        db: u16,
        desc: &crate::database::LayoutDesc,
        sql: &crate::database::sql_query::Sql,
        row: &crate::database::sql_source::Row,
    ) -> Option<Vec<Content>> {
        use crate::database::LayoutNode;
        use crate::database::sql_source::Cell;
        // Every keyed kind, spelled out: an omission here does not read as a
        // missing feature, it silently builds the wrong key and duplicates a
        // record the collection already holds.
        let (elem, key_fields): (u16, Vec<u16>) = match &self.types[db as usize].parts {
            Parts::Hash(c, keys) | Parts::Radix(c, keys) => (*c, keys.clone()),
            Parts::Trie(c, k) => (*c, vec![*k]),
            Parts::Sorted(c, keys) | Parts::Ordered(c, keys) | Parts::Index(c, keys, _) => {
                (*c, keys.iter().map(|(k, _)| *k).collect())
            }
            Parts::Base
            | Parts::Struct(_)
            | Parts::Enum(_)
            | Parts::EnumValue(_, _)
            | Parts::Byte(_, _)
            | Parts::Short(_, _)
            | Parts::Int(_, _)
            | Parts::IntRaw(_, _)
            | Parts::ShortRaw(_, _)
            | Parts::Vector(_)
            | Parts::Array(_)
            | Parts::DbRef
            | Parts::ChildRec(_) => return None,
        };
        let LayoutNode::Record(fields) = desc.nodes.get(&elem)? else {
            return None;
        };
        let mut key = Vec::with_capacity(key_fields.len());
        for k in &key_fields {
            let at = sql.columns.iter().position(|c| c.field == *k as usize)?;
            let cell = row.get(at)?.as_ref();
            key.push(match fields.get(*k as usize)?.content {
                5 => Content::Str(crate::keys::Str::new(match cell {
                    Some(Cell::Text(s)) => s.as_str(),
                    _ => "",
                })),
                3 | 2 => Content::Float(match cell {
                    Some(Cell::Real(v)) => *v,
                    #[allow(clippy::cast_precision_loss)]
                    Some(Cell::Int(v)) => *v as f64,
                    _ => 0.0,
                }),
                _ => Content::Long(match cell {
                    Some(Cell::Int(v)) => *v,
                    #[allow(clippy::cast_possible_truncation)]
                    Some(Cell::Real(v)) => *v as i64,
                    Some(Cell::Text(s)) => s.parse().unwrap_or(0),
                    None => 0,
                }),
            });
        }
        Some(key)
    }

    /// @PLN129 arc B step 7 — why this schema cannot serve this collection, or
    /// `None`.
    ///
    /// Decided once per binding and remembered: nothing about a foreign schema
    /// changes between two lookups, so re-asking would spend a round trip per
    /// fault to learn the same thing.
    #[cfg(feature = "native-extensions")]
    fn schema_refusal(
        &mut self,
        data: &DbRef,
        db: u16,
        desc: &crate::database::LayoutDesc,
        sql: &crate::database::sql_query::Sql,
        conn: &crate::database::sql_source::SqlConn,
    ) -> Option<String> {
        use crate::database::SchemaCheck;
        let slot = (data.store_nr, data.rec, data.pos);
        match self.lazy_sources.get(&slot).map(|b| &b.check) {
            Some(SchemaCheck::Ok) => return None,
            Some(SchemaCheck::Refused(why)) => return Some(why.clone()),
            _ => {}
        }
        let verdict = self.interrogate_schema(db, desc, sql, conn);
        if let Some(b) = self.lazy_sources.get_mut(&slot) {
            b.check = match &verdict {
                None => SchemaCheck::Ok,
                Some(why) => SchemaCheck::Refused(why.clone()),
            };
        }
        verdict
    }

    /// Ask the source whether it can serve this query: the columns must exist
    /// with a compatible affinity, and an INDEX must serve the lookup.
    #[cfg(feature = "native-extensions")]
    fn interrogate_schema(
        &self,
        db: u16,
        desc: &crate::database::LayoutDesc,
        sql: &crate::database::sql_query::Sql,
        conn: &crate::database::sql_source::SqlConn,
    ) -> Option<String> {
        use crate::database::LayoutNode;
        use crate::database::sql_source::affinity;
        let content_tp = self.content(db);
        let Some(LayoutNode::Record(fields)) = desc.nodes.get(&content_tp) else {
            return Some("the element type is not a record".to_string());
        };
        let table = sql.text.split(" FROM ").nth(1)?.split(' ').next()?;
        let declared = match conn.columns_of(table) {
            Ok(c) if !c.is_empty() => c,
            Ok(_) => return Some(format!("the source has no table {table}")),
            Err(why) => return Some(why),
        };
        for sel in &sql.columns {
            // The column name as the query spells it, minus the quoting the
            // mapping added — the catalogue answers unquoted names.
            let want = sel.column.trim_matches(['"', '`']);
            let Some((_, tp)) = declared.iter().find(|(n, _)| n == want) else {
                return Some(format!("{table} has no column `{want}`"));
            };
            // Affinity, not the declared string: SQLite treats a declared type
            // as advice and uses the affinity it implies, so a check on the
            // string would be checking spelling rather than behaviour.
            let a = affinity(tp);
            let wrong = match fields[sel.field].content {
                // A number in a TEXT/BLOB column comes back as text and is
                // converted on the way in, which is a silent reinterpretation
                // of someone's data rather than a fetch.
                0..=4 => a == "TEXT" || a == "BLOB",
                5 => a == "INTEGER" || a == "REAL",
                _ => false,
            };
            if wrong {
                return Some(format!(
                    "{table}.{want} is {tp} ({a} affinity), which cannot hold `{}`",
                    desc.names
                        .get(&fields[sel.field].content)
                        .cloned()
                        .unwrap_or_default()
                ));
            }
        }
        conn.plan_is_indexed(&sql.text).err()
    }

    /// Write one fetched row into a fresh element record and link it into the
    /// collection.
    ///
    /// Through `record_new` + `record_finish` — the pair `coll += [x]` and
    /// `coll[k] = v` both use — so a SQL arrival and an ordinary loft insert end
    /// in the same place, which is what makes "one record however it arrived"
    /// true rather than intended.
    ///
    /// `record_finish` is also what makes the KIND someone else's problem: it
    /// dispatches to `hash::add` / the sorted placement / `tree::add`, so a
    /// `sorted` or `index` collection materialises without a second insert path
    /// here — and there is no per-kind list for a new collection kind to go
    /// missing from ([DATABASE.md § Adding or changing a collection
    /// kind](../../doc/claude/DATABASE.md)).
    #[cfg(feature = "native-extensions")]
    fn materialise(
        &mut self,
        data: &DbRef,
        db: u16,
        sql: &crate::database::sql_query::Sql,
        row: &crate::database::sql_source::Row,
    ) {
        let content_tp = self.content(db);
        let Parts::Struct(fields) = self.types[content_tp as usize].parts.clone() else {
            return;
        };
        let entry = self.record_new(data, db, u16::MAX);
        for (col, sel) in sql.columns.iter().enumerate() {
            let f = &fields[sel.field];
            let at = entry.pos + u32::from(f.position);
            write_cell(
                &mut self.allocations[entry.store_nr as usize],
                entry.rec,
                at,
                f.content,
                row[col].as_ref(),
            );
        }
        self.record_finish(data, &entry, db, u16::MAX);
    }
}

/// A loft key value as a bound parameter.
#[cfg(feature = "native-extensions")]
fn cell_of(c: &Content) -> crate::database::sql_source::Cell {
    use crate::database::sql_source::Cell;
    match c {
        Content::Long(v) => Cell::Int(*v),
        Content::Float(v) => Cell::Real(*v),
        Content::Single(v) => Cell::Real(f64::from(*v)),
        Content::Str(s) => Cell::Text(s.str().to_string()),
    }
}

/// Write one column value into a record field.
///
/// The mirror of `Stores::field_content`, which READS a field by the same
/// content-type ids — 0 integer, 1 long, 2 single, 3 float, 4 boolean, 5 text,
/// 6 character. Keeping the two spelled the same way is what makes a fetched
/// record read back as the value the database holds.
///
/// A SQL `NULL` writes loft's null for that type rather than a zero: C80 says a
/// read never raises, so absence has to be a VALUE, and `0` is a number someone
/// may have meant.
#[cfg(feature = "native-extensions")]
fn write_cell(
    store: &mut crate::store::Store,
    rec: u32,
    at: u32,
    content: u16,
    cell: Option<&crate::database::sql_source::Cell>,
) {
    use crate::database::sql_source::Cell;
    let as_int = |c: Option<&Cell>| match c {
        Some(Cell::Int(v)) => Some(*v),
        // A column typed REAL or TEXT reaching an integer field is a schema
        // mismatch; take what converts and leave the rest null rather than
        // writing a number nobody stored.
        Some(Cell::Real(v)) => Some(*v as i64),
        Some(Cell::Text(s)) => s.parse::<i64>().ok(),
        None => None,
    };
    let as_float = |c: Option<&Cell>| match c {
        Some(Cell::Real(v)) => Some(*v),
        #[allow(clippy::cast_precision_loss)]
        Some(Cell::Int(v)) => Some(*v as f64),
        Some(Cell::Text(s)) => s.parse::<f64>().ok(),
        None => None,
    };
    match content {
        // integer / long — `i64::MIN` is loft's null for both.
        0 => {
            store.set_int(rec, at, as_int(cell).unwrap_or(i64::MIN));
        }
        1 => {
            store.set_long(rec, at, as_int(cell).unwrap_or(i64::MIN));
        }
        2 => {
            #[allow(clippy::cast_possible_truncation)]
            store.set_single(rec, at, as_float(cell).unwrap_or(f64::NAN) as f32);
        }
        3 => {
            store.set_float(rec, at, as_float(cell).unwrap_or(f64::NAN));
        }
        // boolean and character are byte- and word-sized, and both read back
        // through the same accessors `field_content` uses.
        4 => {
            store.set_byte(rec, at, 0, i32::from(as_int(cell).unwrap_or(0) != 0));
        }
        6 => {
            let ch = match cell {
                Some(Cell::Text(s)) => s.chars().next().map_or(0, u32::from),
                other => u32::try_from(as_int(other).unwrap_or(0)).unwrap_or(0),
            };
            store.set_u32_raw(rec, at, ch);
        }
        // text — a NULL leaves the pointer at 0, which IS loft's null text, so
        // `''` and NULL stay two different answers all the way in.
        5 => {
            let ptr = match cell {
                Some(Cell::Text(s)) => store.set_str(s),
                Some(Cell::Int(v)) => store.set_str(&v.to_string()),
                Some(Cell::Real(v)) => store.set_str(&v.to_string()),
                None => 0,
            };
            store.set_u32_raw(rec, at, ptr);
        }
        // Nothing else reaches here: `derive_select` refuses any field that is
        // not one of the above, so a type with one never produced a query.
        _ => {}
    }
}
