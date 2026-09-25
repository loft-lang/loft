#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN11 arc A — extract the IR store-schema registration from the Rust that
# `loft --native --show-rust` generates for tools/ir_schema/ir.loft, and emit a
# self-contained `register_ir_schema(db: &mut Stores) -> IrSchemaIds` function.
#
# Pipeline (hybrid, per design 2026-06-01):
#   ir.loft  --(loft --native)-->  generated.rs  --(this script)-->  ir_schema_gen.rs
#   then a hand-written typed API (src/data_store.rs) is layered on top.
#
# WHY this is a verbatim replay, not a rebasing pass:
#   - The IR block (from `enumerate("TypeT")` to `db.finish()`) DEFINES every
#     non-base `tN` / `vec_*` id it uses, and only REFERENCES base types t0..t6
#     beyond that (verified: our field/value lines reference no stdlib type —
#     see README findings).  So we emit a base-type prelude then replay the
#     kept block lines verbatim; ids stay exactly as the compiler assigned them.
#   - The block is NAME-selected (stdlib `db.value(...)` lines interleave the
#     init line-range), so we keep only lines that build an IR type.
#
# Usage:
#   python3 tools/ir_schema/extract.py tools/ir_schema/generated.rs > src/ir_schema_gen.rs

import re
import sys

IR_ENUMS = {"TypeT", "Node", "DbParts", "DbContent"}
IR_STRUCTS = {
    "Position", "Key", "SortKey", "NameRef", "NameNr", "IntegerSpec",
    "Block", "Attribute", "Variable", "Function",
    "LinkedFieldGroup", "Definition", "Data",
    # loft#1359 — the import tables `rebuild_indices` replays on a warm load.
    "AppliedImport", "UseName", "TypeVarBound",
    # @PLN11 D2a — database type-schema types (Stores.types).
    "DbField", "EnumPair", "KeyField", "DbType", "Bundle",
}


def is_ir_name(name: str) -> bool:
    return name in IR_ENUMS or name in IR_STRUCTS or bool(re.match(r"(Ty|Nd|Pt|Dc)[A-Z]", name))


def to_snake(name: str) -> str:
    return re.sub(r"(?<!^)(?=[A-Z])", "_", name).lower()


def main(path: str) -> int:
    lines = open(path).read().splitlines()

    init_start = next(i for i, l in enumerate(lines) if l.startswith("fn init("))
    block_start = next(
        i for i in range(init_start, len(lines)) if 'enumerate("TypeT")' in lines[i]
    )
    block_end = next(
        i for i in range(block_start, len(lines)) if lines[i].strip() == "db.finish();"
    )
    block = [l.rstrip() for l in lines[block_start:block_end]]

    # Pass 1: which tN ids are OUR IR types?  Record enum/struct id maps.
    def_re = re.compile(r"let (t\d+) = db\.(structure|enumerate)\(\"([^\"]+)\"")
    ir_ids = set()
    enum_ids = {}
    struct_ids = {}
    for l in block:
        m = def_re.search(l)
        if m and is_ir_name(m.group(3)):
            ir_ids.add(m.group(1))
            (enum_ids if m.group(2) == "enumerate" else struct_ids)[m.group(3)] = m.group(1)

    # Pass 2: TWO-PHASE emit (mirrors the IR doc's "(1) shells, (2) fields").
    # Phase 1 = every `let tN = db.structure/enumerate("IrName"…)` declaration,
    # so all type ids are bound before any field references them — fixes the
    # forward refs `NdBlock.block -> Block`
    # (finding 3: generated def order is NOT dependency-respecting).
    # Phase 2 = the field/value/vector/byte_enum lines, in original order.
    field_re = re.compile(r"db\.(field|value)\((t\d+),")
    decls = []
    bodies = []
    for l in block:
        m = def_re.search(l)
        if m:
            if is_ir_name(m.group(3)):
                decls.append(l)
            continue  # drop stdlib structure/enumerate
        m = field_re.search(l)
        if m:
            if m.group(2) in ir_ids:
                bodies.append(l)
            continue  # drop stdlib field/value
        # locals (byte_enum / vec_* / tN-vectors / `let _ = tN;`) — keep all in
        # the body phase; the database dedupes repeated byte(0)/vector(t).
        # Every NAMED local the generator binds for a field's storage —
        # `byte_enum`, `vec_*`, and (since `ir.loft` grew by-reference fields)
        # `dbref_*` / `crec_*`. Keeping only a hand-listed few is how the
        # committed file went stale in CONTENT as well as in labels: a field
        # whose local was dropped referenced a name nothing bound, so the file
        # could not be regenerated at all and was edited by hand instead.
        # Anything kept but unused is harmless — the file head allows it, and
        # the database dedupes repeated constructors.
        if re.search(r"let [A-Za-z_]\w* = db\.", l) or re.search(r"let _ = t\d+;", l):
            bodies.append(l)
    keep = decls + bodies

    # RENUMBER our type labels to a deterministic, stdlib-independent sequence.
    #
    # `tN` is only a Rust local name — it is bound to whatever `db.structure`
    # returns — but the extractor used to copy it verbatim out of
    # `generated.rs`, where the number is an ABSOLUTE type id counted after the
    # whole stdlib. So adding one stdlib type shifted every label and a fresh
    # regen differed from the committed file in ~1300 lines, which is what made
    # regeneration unusable and pushed schema edits into hand-adds instead.
    #
    # Relabelling in declaration order, starting after the `t0..t6` base
    # prelude, makes the output depend on `ir.loft` alone.
    remap = {}
    for l in decls:
        m = def_re.search(l)
        remap[m.group(1)] = f"t{len(remap) + 7}"
    # The body declares `tN` locals of its own — `let t97 = db.vector(t73);` and
    # friends. They are labels exactly like the declarations above, so they join
    # the same sequence, in body order.
    local_re = re.compile(r"let (t\d+) = db\.")
    for l in bodies:
        m = local_re.search(l)
        if m and m.group(1) not in remap:
            remap[m.group(1)] = f"t{len(remap) + 7}"
    tok_re = re.compile(r"\bt(\d+)\b")

    def relabel(line: str) -> str:
        def one(m):
            tok = m.group(0)
            if int(m.group(1)) < 7:
                return tok  # a base type — the prelude binds these
            if tok not in remap:
                # The README's "our IR types depend only on BASE types" is what
                # makes this extraction self-contained. If that ever stops being
                # true, fail here rather than emit a label nothing binds.
                raise SystemExit(
                    f"extract.py: {tok} is neither a base type nor one of ours — "
                    "the IR schema has grown a stdlib dependency, which this "
                    "extraction cannot represent"
                )
            return remap[tok]

        return tok_re.sub(one, line)

    keep = [relabel(l) for l in keep]
    enum_ids = {k: remap[v] for k, v in enum_ids.items()}
    struct_ids = {k: remap[v] for k, v in struct_ids.items()}

    out = []
    out.append("// @generated by tools/ir_schema/extract.py from tools/ir_schema/ir.loft")
    out.append("// via `loft --native --show-rust`.  DO NOT EDIT — regenerate.")
    out.append("// Copyright (c) 2026 Jurjen Stellingwerff")
    out.append("// SPDX-License-Identifier: LGPL-3.0-or-later")
    out.append("//")
    out.append("//! The store schema for the compiler IR (@PLN11 arc A): registers every")
    out.append("//! IR struct/enum as `Stores` type records so the IR can live in a store and")
    out.append("//! the schema-driven inspection layer (`Stores::show_json`) can walk it.")
    # Generated code trips several pedantic lints by nature (long fn, reused
    # `vec_*` local names, redundant lets); blanket-allow them at the file head.
    # Emit in the fmt-canonical multi-line shape so `cargo fmt` is a no-op and a
    # fresh regen stays byte-identical to the committed file.
    out.append("#![allow(")
    out.append("    clippy::too_many_lines,")
    out.append("    unused_variables,")
    out.append("    clippy::let_and_return,")
    out.append("    clippy::similar_names")
    out.append(")]")
    out.append("")
    out.append("use crate::database::Stores;")
    out.append("")
    out.append("/// Registered type numbers for the IR types, returned by")
    out.append("/// [`register_ir_schema`] so the typed accessor layer can bind to them.")
    out.append("#[derive(Clone, Copy, Debug)]")
    out.append("pub struct IrSchemaIds {")
    for nm in sorted(enum_ids):
        out.append(f"    /// `known_type` of the `{nm}` enum.")
        out.append(f"    pub {to_snake(nm)}: u16,")
    for nm in sorted(struct_ids):
        out.append(f"    /// `known_type` of `{nm}`.")
        out.append(f"    pub {to_snake(nm)}: u16,")
    out.append("}")
    out.append("")
    out.append("/// Register the full compiler-IR store schema into `db`.")
    out.append("/// Generated verbatim from the `--native` layout; register once per `Stores`.")
    out.append("#[must_use]")
    out.append("pub fn register_ir_schema(db: &mut Stores) -> IrSchemaIds {")
    for t in range(7):
        out.append(f"    let t{t}: u16 = {t};")
    out.append("    let _ = (t0, t1, t2, t3, t4, t5, t6);")
    for l in keep:
        out.append("    " + l.strip())
    out.append("    db.finish();")
    out.append("    IrSchemaIds {")
    for nm in sorted(enum_ids):
        out.append(f"        {to_snake(nm)}: {enum_ids[nm]},")
    for nm in sorted(struct_ids):
        out.append(f"        {to_snake(nm)}: {struct_ids[nm]},")
    out.append("    }")
    out.append("}")

    # Single trailing newline (fmt-conformant — no trailing blank line), so a
    # fresh regen is byte-identical to the committed src/ir_schema_gen.rs.
    sys.stdout.write("\n".join(out) + "\n")
    sys.stderr.write(
        f"emitted register_ir_schema: {len(keep)} schema lines, "
        f"{len(struct_ids)} structs + {len(enum_ids)} enums\n"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else "tools/ir_schema/generated.rs"))
