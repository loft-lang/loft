// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

//! Visibility across files (`@FR-F-Visible`, @C140): the one check for a use outside the
//! declaring file, the name-only types that cannot be built elsewhere, and the program types
//! that shadow a standard-library name (@C101).

use super::{DefType, Level, Parser, Type, diagnostic_format};

impl Parser {
    /// One `[visibility]` census line for [`Self::check_visibility`]: the use, its kind and item,
    /// and the declaration a rewrite starts from — the struct's or the variant's own for a field,
    /// the owning type's for a literal or a variant.  Paths canonical, so a census run from any
    /// directory names the same file.
    pub(super) fn print_census_site(&self, kind: &str, d_nr: u32, f_nr: usize) {
        let def = self.data.def(d_nr);
        let owner = if matches!(def.def_type, DefType::EnumValue) && def.parent != u32::MAX {
            self.data.def(def.parent)
        } else {
            def
        };
        let item = def
            .attributes
            .get(f_nr)
            .map_or_else(|| def.name.clone(), |a| format!("{}.{}", def.name, a.name));
        let at = if f_nr == usize::MAX {
            &owner.position
        } else {
            &def.position
        };
        let canon = |f: &str| {
            let path = crate::file_access::PathText::host(f);
            crate::file_access::canonical(&path).map_or_else(|| f.to_string(), |p| p.native())
        };
        let here = self.lexer.pos();
        eprintln!(
            "[visibility] {}:{} {kind} {item} — declared in {}:{}",
            canon(&here.file),
            here.line,
            canon(&owner.position.file),
            at.line
        );
    }

    /// The name a diagnostic gives the file `file` declares in — its last path part without the
    /// `.loft`, which is what a program qualifies it by.
    pub(super) fn library_of(file: &str) -> String {
        let path = crate::file_access::PathText::host(file);
        let last = path.parts().last().map_or("", String::as_str);
        last.rsplit_once('.')
            .map_or(last, |(stem, _)| stem)
            .to_string()
    }

    /// @PLN187 (@C140, @FR-F-Visible) — the one visibility check for a use, OUTSIDE its declaring file, of a
    /// struct or variant field (`field`: a read or a write; `literal-field`, `pattern-field`,
    /// `key`), of a struct literal (`literal`) and of an enum variant (`variant`, built or
    /// matched).  `d_nr` is the struct or the variant; `f_nr` its field, or `usize::MAX` for the
    /// type itself.  A use that lacks its `pub` is:
    ///
    /// * printed under `LOFT_TRACE_VISIBILITY=1` — the census (`scripts/pub_census.sh`), so the
    ///   census lists exactly what is still missing and is empty once everything is migrated;
    /// * refused — the rule, with the item, its file and the cure.
    ///
    /// Building a type that is not `pub` at all is [`Self::refuse_building_a_name_only_type`]'s.
    pub(crate) fn check_visibility(&mut self, kind: &str, d_nr: u32, f_nr: usize) {
        let trace = crate::env_once!(std::env::var_os("LOFT_TRACE_VISIBILITY").is_some());
        if self.first_pass || d_nr >= self.data.definitions() {
            return;
        }
        let def = self.data.def(d_nr);
        // A variant's fields and name are its enum's: the enum's file is the one that decides.
        let owner_nr = if matches!(def.def_type, DefType::EnumValue) && def.parent != u32::MAX {
            def.parent
        } else {
            d_nr
        };
        // A generic instance's visibility is its template's.
        let owner = self.data.def(self.data.visibility_def(owner_nr));
        if owner.source == self.data.source
            || owner.position.file == self.lexer.pos().file
            || owner.name.starts_with("__")
        {
            return;
        }
        let attr = def.attributes.get(f_nr);
        // A METHOD is stored as a `Routine` member: its visibility is its function's `pub`.
        if attr.is_some_and(|a| {
            a.name.starts_with("__") || a.hidden || matches!(a.typedef.base(), Type::Routine(_))
        }) {
            return;
        }
        // An enum's members are its VALUES (`Format.NotExists`): using one is using a variant.
        let kind = if matches!(def.def_type, DefType::Enum) && attr.is_some() {
            "variant"
        } else {
            kind
        };
        let lib = Self::library_of(&owner.position.file);
        let (why, fix) = match (kind, attr) {
            ("field" | "literal-field" | "pattern-field" | "key", Some(a)) if !a.pub_field => (
                format!(
                    "field `{}` of `{}` is not `pub` in `{lib}`",
                    a.name, def.name
                ),
                format!("`{lib}` declares it `pub {}`", a.name),
            ),
            ("literal", _) if !owner.pub_visible => {
                if !trace {
                    return; // `refuse_building_a_name_only_type` refuses it
                }
                (
                    format!("`{}` is not `pub` in `{lib}`", owner.name),
                    format!("`{lib}` declares it `pub`"),
                )
            }
            ("literal", _) => {
                let Some(private) = def.attributes.iter().find(|a| {
                    !a.pub_field
                        && !a.hidden
                        && !a.name.starts_with("__")
                        && a.name != "enum"
                        && !matches!(a.typedef.base(), Type::Routine(_))
                }) else {
                    return;
                };
                (
                    format!(
                        "`{}` cannot be built outside `{lib}`: its field `{}` is not `pub`",
                        def.name, private.name
                    ),
                    format!(
                        "`{lib}` declares every field of `{}` `pub`, or the program builds it \
                         with `{lib}`'s functions",
                        def.name
                    ),
                )
            }
            ("variant", _) if !owner.pub_visible => (
                format!(
                    "`{}` is a variant of `{}`, which is not `pub` in `{lib}`",
                    def.name, owner.name
                ),
                format!("`{lib}` declares `pub enum {}`", owner.name),
            ),
            _ => return,
        };
        if trace {
            self.print_census_site(kind, d_nr, f_nr);
        }
        diagnostic!(self.lexer, Level::Error, "{why}.\n  fix: {fix}");
    }

    /// @FR-F-Surface (loft#2115) — a METHOD its library declares without `pub` is not on the
    /// library's surface, so a call from outside its file reaches it by neither spelling:
    /// `x.m(…)`, which finds it in the receiver type's attribute table (every method of the
    /// type is there once its package is loaded, by any `use` or by a trigger), nor
    /// `lib::m(x)`, which [`crate::data::Data::qualified_refusal`] asks by NAME while a method
    /// is keyed by its receiver.  `d_nr` is the definition the call SELECTED — a generic
    /// method's template, before it is instantiated, because an instance is minted where it is
    /// first called and carries no `pub` of its own.  The stdlib, the caller's own file and
    /// source, and a synthesised dispatcher are not judged.
    pub(super) fn refuse_unpublished_method(&mut self, d_nr: u32, qualified: bool) {
        if self.first_pass || self.default || d_nr >= self.data.definitions() {
            return;
        }
        let def = self.data.def(d_nr);
        if def.pub_visible
            || def.synthetic.is_some()
            || !matches!(def.def_type, DefType::Function | DefType::Generic)
            || def.source == crate::data::STD_SOURCE
            || def.source == self.data.source
            || def.position.file == self.lexer.pos().file
        {
            return;
        }
        let Some(key) = crate::data::Data::split_key(&def.name)
            .filter(|k| k.kind == crate::data::KeyKind::Method)
        else {
            return;
        };
        let name = key.rest.to_string();
        let lib = Self::library_of(&def.position.file);
        let spelling = if qualified {
            format!("`{lib}::{name}`")
        } else {
            format!("`.{name}(…)`")
        };
        diagnostic!(
            self.lexer,
            Level::Error,
            "`{name}` is not `pub` in `{lib}`, so {spelling} cannot reach it: a library's \
             surface is its `pub` items.\n  fix: `{lib}` declares it `pub`, or the program \
             uses something `{lib}` does publish"
        );
    }

    /// @PLN187 (@C140, @FR-F-Visible) — a type that is NAME ONLY outside its file (not `pub`, but named by a
    /// `pub` signature there) can be named, passed and stored there, never BUILT: a literal of it
    /// outside its file is refused.  Building a non-`pub` type was refused before name-only
    /// types existed, through the name itself; this keeps that refusal now the name is reachable.
    /// The standard library's types are left to its migration (@PLN187 step 4).
    pub(crate) fn refuse_building_a_name_only_type(&mut self, td_nr: u32) {
        if self.first_pass || td_nr >= self.data.definitions() {
            return;
        }
        let def = self.data.def(td_nr);
        let owner_nr = self.data.visibility_def(
            if matches!(def.def_type, DefType::EnumValue) && def.parent != u32::MAX {
                def.parent
            } else {
                td_nr
            },
        );
        let owner = self.data.def(owner_nr);
        if owner.source == self.data.source
            || owner.source == crate::data::STD_SOURCE
            || owner.position.file == self.lexer.pos().file
            || owner.pub_visible
            || !self.data.is_name_only(owner_nr)
        {
            return;
        }
        let lib = Self::library_of(&owner.position.file);
        let name = owner.name.clone();
        diagnostic!(
            self.lexer,
            Level::Error,
            "`{name}` is not `pub` in `{lib}`, so it cannot be built outside that file: `{lib}` \
             only lets its `pub` functions hand it out.\n  fix: build it with `{lib}`'s \
             functions, or `{lib}` declares it `pub`"
        );
    }

    /// @C101 — a program type that shares a standard-library type's name shadows it wherever
    /// it is written, above its declaration too.  Pass 1 resolves a name it has not seen
    /// declared yet: an unknown one gets a forward-reference stub its declaration adopts, but a
    /// stdlib name is found, so everything written above the program's own `struct File` bound
    /// to the stdlib's (loft#1882).  Before pass 1 reads a program file, each type it declares
    /// that would shadow a prelude type gets that stub in the file's own namespace — the lookup
    /// asks the file before the stdlib — so the declaration adopts it like any forward reference.
    pub(super) fn claim_declared_type_names(&mut self, file: &str) {
        if !self.first_pass || self.default || self.data.source == crate::data::STD_SOURCE {
            return;
        }
        let src = self
            .lexer
            .source_text(file)
            .map_or_else(|| Self::read_source(file), str::to_string);
        for name in crate::libscan::scan_type_declarations(&src) {
            if self.prelude_shadowed(&name) {
                self.data.add_def(&name, self.lexer.pos(), DefType::Unknown);
            }
        }
    }
}
