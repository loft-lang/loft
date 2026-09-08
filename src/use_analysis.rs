//! USE-analysis (first version) — derive a per-binding copy-vs-borrow VERDICT from
//\! @I60 — Scope & dependency/lifetime tracker (deps)
//! how a variable is *used*, not from the shape of its right-hand side.
//!
//! This is the dependable layer the copy-vs-borrow elision builds on; the original
//! design is `doc/claude/plans/25-nullable-sequences/use-analysis-prework-design.md`
//! and `materialization-algorithm-design.md`. Its outputs are consumed by
//! **default-on codegen**: the `Verdict` drives an `ElidePlan` (borrow-inline, wired
//! into `scopes::elide_borrows`) and a `MovePlan` (last-use move, wired into
//! `scopes::move_elide`); the dumps (`LOFT_MATERIALIZE_DUMP`, `--report-copies`,
//! `LOFT_WARN_COPIES`) are opt-in views on the same verdicts.
//!
//! By the time this runs (post-parse, in `scopes::check`), a `v = src.f` vector copy
//! has already been lowered to the **copy idiom**: a fresh `OpDatabase` buffer `vdb`,
//! `v = OpGetField(vdb, …)`, and one `OpAppendVector(v, src.f)` filling it. So the
//! analysis recognises that idiom (exactly what the elision rewrite consumes) and
//! decides whether the copy could instead be a borrow.
//!
//! Soundness is by **conservative default**: a binding is `Borrow` ONLY when proven
//! safe at the Tier-0 envelope — single def, the source `src` is a parameter, and
//! neither `v` nor `src` ever appears outside a known-reader argument position (so `v`
//! is read-only and non-escaping — ¬D1/¬D3 — and `src` is unmutated — ¬D2; the param
//! lifetime gives the rest of ¬D3). Anything not proven is `Copy`. An unrecognised use
//! can only *lose* an elision, never produce a wrong borrow.

use crate::data::{Data, DefType, Type, Value};
use crate::lexer::Position;
use crate::variables::Function;
use std::collections::{HashMap, HashSet};

/// The materialization decision for one binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Proven safe to alias the source store instead of deep-copying (Tier 0).
    Borrow,
    /// The conservative default — deep-copy into an independent store.
    Copy,
}

/// @PLN90 — the WARNING bucket for a copy (which `COPY_DIAGNOSTICS.md` drives). The verdict
/// says *whether* a copy happens; this says *what to do about it*.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyClass {
    /// A `Borrow` — already eliminated, not a copy. No warning.
    Eliminated,
    /// Bucket 2 — a borrow WOULD be sound; the copy is only analysis/codegen weakness. The
    /// north-star elimination worklist. WARN (the actionable "you didn't have to copy").
    Avoidable,
    /// Inherent to the ownership model — constructing an owning structure (`S { f: src }`)
    /// or assigning into an owning slot (`v[i] = e`) *owns* its data. This is exactly what
    /// the programmer asked for, not a surprise. SILENT (no warning).
    Implicit,
    /// Forced by circumstance (a short-lived source, a later mutation) — required as
    /// written, but the user could restructure. Indicated (informational), never silent.
    Forced,
    /// @PLN90 (item 1) — an UNBOUND copy whose SOURCE is a compiler-generated temporary
    /// (`_`-prefixed: `__ref_N`, `___par_mat_e_N`, `_comp_N`, …; `is_compiler_generated`).
    /// A real copy and a candidate for US to eliminate (the developer worklist), but NOT
    /// user-actionable — the user never wrote the source, so the user-facing report must
    /// exclude it. Distinct from `Implicit` (genuinely no unbound copy) and from
    /// `Avoidable`/`Forced` (which name a source the user can act on).
    Internal,
}

/// One row of the analysis result: the verdict for a single vector-copy binding.
#[derive(Clone, Debug)]
pub struct VerdictRow {
    pub var_nr: u16,
    pub var_name: String,
    /// The source variable of the copied `src` / `src.f` (`u16::MAX` if not a var).
    pub source: u16,
    pub verdict: Verdict,
    /// Human-readable justification (for the dump and test diagnostics).
    pub reason: &'static str,
    /// @PLN90 — the warning bucket: `Eliminated` (Borrow) · `Avoidable` (warn — the
    /// worklist) · `Implicit` (model-inherent ownership — silent) · `Forced` (informational).
    /// See [`CopyClass`] and COPY_DIAGNOSTICS.md.
    pub class: CopyClass,
    /// @PLN90 item 2 — the copy site's source location (`file:line:pos`), when the emitting
    /// op carries a span. Makes a `<record>` element-set copy (no named target var)
    /// actionable in the report. `None` when the op is unspanned.
    pub loc: Option<Position>,
    /// @PLN131 Q6.1 — where `source` is used AGAIN after the copy site, when that use carries
    /// a position. This is the fact a conditional suggestion has to state: the copy exists
    /// *because* of this use, so it is the one the author decides about. `None` when the
    /// source is not a variable or the surviving use is unspanned.
    pub source_last_use: Option<Position>,
    /// @PLN90 Step 5 — true for a SURVIVAL-SPLIT row (a construction / record copy classified by
    /// its source's fate — "you duplicated a live value"). The user-facing `report_copies` shows
    /// only these; the var-buffer / return-buffer copies (a separate elision/`__retbuf` class,
    /// which is where the stdlib's copies land) stay in the developer dump.
    pub survival: bool,
}

fn def_nrs(data: &Data, names: &[&str]) -> HashSet<u32> {
    let mut s = HashSet::new();
    for name in names {
        let nr = data.def_nr(name);
        if nr != u32::MAX {
            s.insert(nr);
        }
    }
    s
}

/// PROJECTION ops return a *reference into* their base container (arg 0) — so the
/// base is accessed in whatever context the projection's RESULT is: a projection
/// inside a write (`OpSetInt(OpGetVector(t,…), …)`) writes `t`; inside a read
/// (`OpGetInt(OpGetVectorNullable(t,…), …)`) reads `t`. They therefore PROPAGATE the
/// incoming context to arg 0 (the remaining args — an index — are pure reads).
///
/// **The membership test is the op's own declaration: `-> reference[arg0]`.** Every
/// element read carries it in both spellings (`OpGetVector` / `OpVectorRef` and their
/// nullable twins), as does the keyed lookup `OpGetRecord` and the field read
/// `OpGetField`. `OpGetDbRef` is the one member without it and is deliberate: it reads a
/// STORED `DbRef` out of a record, which may name any store, and it is here because a
/// closure's `__closure` read must still resolve to the record the caller holds.
///
/// [`Ownership::borrow_base_guarded`] roots a borrow through this set, so a
/// store-preserving read MISSING from it makes the oracle call a view of somebody
/// else's container `Owned` — the over-free direction (loft#1318, where `h[k].v` handed
/// to a fn-ref emptied the caller's hash, and the `sorted` and `index` kinds faulted).
///
/// [`is_projection_op`] asks the narrower ROOT-NAMING question over an overlapping list,
/// and the two are not the same set: this one carries `OpGetDbRef` and both nullable
/// element reads, that one carries neither. Where they differ, say which fact the site
/// wants — `@FR-O-Oracle` for own-vs-borrow, root-naming for which container a view came
/// out of.
fn projection_ops(data: &Data) -> HashSet<u32> {
    def_nrs(
        data,
        &[
            "OpGetVector",
            "OpGetVectorNullable",
            "OpGetField",
            "OpVectorRef",
            "OpVectorRefNullable",
            "OpGetRecord",
            "OpGetDbRef",
        ],
    )
}

/// @PLN90 item 3 — ops that WRITE THROUGH THEIR FIRST ARGUMENT (mirrors the
/// `first_arg_write` set in `parser::find_written_vars`). Used to build `mut_max_pos`,
/// the position-aware "the source is mutated after the copy" fact. `OpCopyRecord` writes
/// its SECOND arg (the dest), handled separately.
fn is_first_arg_write_name(n: &str) -> bool {
    n.starts_with("OpSet")
        || n.starts_with("OpAppendStack")
        || n.starts_with("OpClearStack")
        || matches!(
            n,
            "OpNewRecord"
                | "OpAppendCopy"
                | "OpAppendVector"
                | "OpClearVector"
                | "OpClearKeyed"
                | "OpSetKeyed"
                | "OpHashRemove"
                | "OpInsertVector"
                | "OpRemoveVector"
        )
}

/// LENGTH ops — the collection `len` methods (`t_6vector_len`, `t_6sorted_len`, …).
///
/// They observe how MANY elements a collection has, never what any of them is, so a
/// `len` read cannot witness an element write.  The dead-store lint therefore does not
/// let one discharge its signal: `d = self.data; if i < len(d) { d[i] = x }` is the
/// copy-mutate footgun in full, and counting `len(d)` as "the copy was read" made the
/// lint silent on exactly the shape it exists to catch.  Not hypothetical — the shipped
/// `graphics` canvas is written this way, so every `set_pixel` was a no-op and a
/// `--html` page rendered a blank texture with no diagnostic anywhere.
///
/// The bound guard is not the author's mistake, either: it is the idiom the `v[i]`
/// may-be-null warning ASKS for (skip-pattern 5), so the two lints were in tension —
/// satisfying one silenced the other.
fn is_length_op_name(n: &str) -> bool {
    // Method naming is `t_<LEN><Type>_<method>` (CODE.md), so match the suffix.
    n.starts_with("t_") && n.ends_with("_len")
}

/// VALUE-READER ops return a fresh value, not a reference into a container — their
/// arguments are pure reads regardless of the surrounding context.
fn value_reader_ops(data: &Data) -> HashSet<u32> {
    def_nrs(
        data,
        &[
            "OpGetInt",
            "OpGetByte",
            "OpGetEnum",
            "OpGetSingle",
            "OpGetCharacter",
            "OpLengthVector",
            "t_6vector_len",
        ],
    )
}

/// The op-number sets the use / dead-store / ownership walks consult.
///
/// Every one of them is a pure function of the definition **name** table, so for a
/// given [`Data`] they are constant — yet each walk rebuilt them, and the walks ask
/// once per FUNCTION. Two of the four are a full scan of every definition doing
/// string prefix matches, so the cost is O(functions × definitions): a
/// `println`-sized program rebuilt them **9 000 times over 708 definitions**, which
/// measured ~40 % of a warm-cache startup run (the rebuild itself plus the
/// `HashSet<u32>` inserts and rehashes it drives).
///
/// Cached on [`Data::op_sets`]. That is sound here for the reason it is NOT sound for
/// `function_defs` (loft#854): these derive from def names, which never change once a
/// definition exists, whereas `function_defs` derives from `Definition::code`, which
/// `scopes.rs` rewrites — so a `Data`-lived cache of that serves a stale body after
/// any rewrite. It is the same reason `caller_index` may live on `Data`.
///
/// The sets are `Arc`-shared so a consumer that needs to OWN them (`Uses`) pays a
/// refcount bump instead of a rebuild.
#[derive(Clone)]
pub(crate) struct OpSets {
    /// Projection ops — see [`projection_ops`].
    pub(crate) projections: std::sync::Arc<HashSet<u32>>,
    /// Value-reader ops — see [`value_reader_ops`].
    pub(crate) value_readers: std::sync::Arc<HashSet<u32>>,
    /// Ops writing through arg 0 — see [`is_first_arg_write_name`].
    pub(crate) write_first_arg: std::sync::Arc<HashSet<u32>>,
    /// Collection `len` methods — see [`is_length_op_name`].
    pub(crate) lengths: std::sync::Arc<HashSet<u32>>,
    /// The ops that RELEASE their first argument — one notion, five spellings: the plain
    /// reference free, its tag-checked twin, the text free, and the two witness-guarded
    /// conditional frees.  Every matcher that asks *"is this a free?"* reads THIS set, because
    /// a hand-spelled name list goes blind to a new spelling at once — `check_ref_leaks`
    /// reported a leak the compiler did not have when `OpFreeRefOrHandUp` arrived (loft#1186),
    /// and of the nine lists this replaced no two agreed.  @FR-O-Override's contract is stated
    /// over this notion, not over the one name the rule spells.
    pub(crate) frees: std::sync::Arc<HashSet<u32>>,
    /// The UNCONDITIONAL reference frees — `OpFreeRef` and `OpFreeRefTag`.  A check that asks
    /// *"is this store released here, whatever the run"* reads this one.
    pub(crate) unconditional_ref_frees: std::sync::Arc<HashSet<u32>>,
    /// The witness-GUARDED reference frees — `OpFreeRefIfDistinct` and `OpFreeRefOrHandUp`: a
    /// no-op where the placeholder aliases its witness, so a path walk reads one as CLEARING a
    /// pending free rather than as a free.
    pub(crate) conditional_ref_frees: std::sync::Arc<HashSet<u32>>,
    /// `OpFreeText` — the text release, or `u32::MAX` where the table has none.
    pub(crate) text_free: u32,
}

impl OpSets {
    /// Build every set. The two prefix-matched sets share ONE pass over the
    /// definition table — they scanned it separately before.
    pub(crate) fn build(data: &Data) -> Self {
        let mut write_first_arg = HashSet::new();
        let mut lengths = HashSet::new();
        for d in 0..data.definitions() {
            let n = data.def(d).name();
            if is_first_arg_write_name(n) {
                write_first_arg.insert(d);
            }
            if is_length_op_name(n) {
                lengths.insert(d);
            }
        }
        let nrs = |names: &[&str]| -> HashSet<u32> {
            names
                .iter()
                .map(|n| data.def_nr(n))
                .filter(|&d| d != u32::MAX)
                .collect()
        };
        let unconditional_ref_frees = nrs(&["OpFreeRef", "OpFreeRefTag"]);
        let conditional_ref_frees = nrs(&["OpFreeRefIfDistinct", "OpFreeRefOrHandUp"]);
        let text_free = data.def_nr("OpFreeText");
        let mut frees: HashSet<u32> = unconditional_ref_frees
            .iter()
            .chain(conditional_ref_frees.iter())
            .copied()
            .collect();
        if text_free != u32::MAX {
            frees.insert(text_free);
        }
        Self {
            projections: std::sync::Arc::new(projection_ops(data)),
            value_readers: std::sync::Arc::new(value_reader_ops(data)),
            write_first_arg: std::sync::Arc::new(write_first_arg),
            lengths: std::sync::Arc::new(lengths),
            frees: std::sync::Arc::new(frees),
            unconditional_ref_frees: std::sync::Arc::new(unconditional_ref_frees),
            conditional_ref_frees: std::sync::Arc::new(conditional_ref_frees),
            text_free,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Ctx {
    /// The node sits in a known-reader op's argument position (a pure read).
    ReaderArg,
    /// Any other position — conservatively a mutation / escape / unknown use.
    Other,
}

/// The base variable of a `src` / `src.f` expression, if any.
fn base_var(node: &Value, get_field: u32) -> Option<u16> {
    match node.unspan() {
        Value::Var(s) => Some(*s),
        Value::Call(d, args) if *d == get_field => match args.first().map(Value::unspan) {
            Some(Value::Var(s)) => Some(*s),
            _ => None,
        },
        _ => None,
    }
}

/// @PLN107 S1 — per-variable ACCESS classification for the dead-store lint. Returns, indexed
/// by `var_nr`, `(reads, write_targets)`: how many times each local is READ (its value
/// observed) versus used only as the WRITE-TARGET base of an element/field/keyed setter
/// (`d[i]=x`, `d.f=x`).
///
/// Deliberately DECOUPLED from `Function::uses` (which drives codegen last-use elision and
/// MUST NOT change): a `Var(d)` at arg 0 of an `OpSet*` bumps `uses` but is a write-target,
/// not a read. WRITES are restricted to the `OpSet*` family on purpose — the copy-fill
/// `OpAppendVector` that lowers `d = s.f` is a DEFINITION, not a user mutation, and counting
/// it would misread an unused copy as a dead store. Projection handling mirrors
/// [`projection_ops`]: `d.f[i]=x` → `OpSetInt(OpGetField(Var(d),f), i, v)` descends to the
/// root var `d`, and the projection's INDEX args are ordinary reads.
pub(crate) fn dead_store_accesses(body: &Value, n_vars: usize, data: &Data) -> Vec<(u16, u16)> {
    let ops = data.op_sets();
    let mut acc = vec![(0u16, 0u16); n_vars];
    let cx = AccessCx {
        data,
        projs: &ops.projections,
        writes: &ops.write_first_arg,
        lens: &ops.lengths,
        copy_record: data.def_nr("OpCopyRecord"),
    };
    classify_access(body, &cx, &mut acc);
    acc
}

/// Shared read-only context for the access walk (op-name sets computed once).
struct AccessCx<'a> {
    data: &'a Data,
    /// Projection ops (`OpGetField`/`OpGetVector`/…) — a write through one of these
    /// propagates the write context to arg 0.
    projs: &'a HashSet<u32>,
    /// Ops that write through arg 0 (`first_arg_write_ops`): their arg-0 base is a
    /// write-DESTINATION, never a read (this is what makes the `d = s.f` copy-fill append
    /// stop counting `d` as read).
    writes: &'a HashSet<u32>,
    /// Collection `len` methods ([`length_ops`]): the subject is observed for its COUNT,
    /// which no element write can change, so it is not a value read.
    lens: &'a HashSet<u32>,
    /// `OpCopyRecord` — the one write op whose destination is arg **1**, not arg 0
    /// (`OpCopyRecord(source, dest, type)`).  `w[i] = Row{…}` lowers to it, so without
    /// this the whole-element assign was invisible to the dead-store lint (loft#670).
    copy_record: u32,
}

fn is_setter(op: u32, data: &Data) -> bool {
    data.def(op).name().starts_with("OpSet")
}

fn bump_read(acc: &mut [(u16, u16)], v: u16) {
    if let Some(slot) = acc.get_mut(v as usize) {
        slot.0 = slot.0.saturating_add(1);
    }
}

fn bump_write(acc: &mut [(u16, u16)], v: u16) {
    if let Some(slot) = acc.get_mut(v as usize) {
        slot.1 = slot.1.saturating_add(1);
    }
}

/// Classify every `Var` occurrence reachable from `node`. Only the write-introducing /
/// var-carrying variants are special-cased; everything else delegates to `for_each_child`
/// (whose `Var` children are all value-observing reads). `Set`/`Iter` TARGET vars are NOT
/// visited by `for_each_child`, so a plain definition/reassignment correctly counts as
/// neither a read nor a copy-mutate write.
fn classify_access(node: &Value, cx: &AccessCx, acc: &mut [(u16, u16)]) {
    match node.unspan() {
        Value::Var(v) | Value::TupleGet(v, _) | Value::FnRefDnr(v) => bump_read(acc, *v),
        Value::FnRef(_, clos, _) => bump_read(acc, *clos),
        Value::TuplePut(v, _, inner) => {
            bump_write(acc, *v);
            classify_access(inner, cx, acc);
        }
        Value::CallRef(v, args) => {
            bump_read(acc, *v);
            for a in args {
                classify_access(a, cx, acc);
            }
        }
        // A collection `len`: its subject is observed for COUNT only, so it is not a value
        // read and cannot discharge the dead-store signal.  Index/projection args on the way
        // down are ordinary reads; a var reached any OTHER way still counts normally, so a
        // copy that is genuinely used stays silent.
        Value::Call(op, args) if cx.lens.contains(op) && !args.is_empty() => {
            classify_length_subject(&args[0], cx, acc);
            for a in &args[1..] {
                classify_access(a, cx, acc);
            }
        }
        // Any op that writes through arg 0: arg-0 base is a write-DESTINATION (not a read).
        // Count it as a copy-mutate WRITE-TARGET only for the `OpSet*` family — append/insert/
        // clear are definitional/bulk fills (the `d = s.f` copy-fill lands here) and are neither
        // a read nor the dead-store mutation signal. The remaining args are ordinary reads.
        Value::Call(op, args) if cx.writes.contains(op) && !args.is_empty() => {
            classify_write_base(&args[0], cx, acc, is_setter(*op, cx.data));
            for a in &args[1..] {
                classify_access(a, cx, acc);
            }
        }
        // `OpCopyRecord(source, dest, type)` — the destination is arg 1.  It counts as a
        // copy-mutate write only when it is a PROJECTION of a var (`w[i] = Row{…}`, which
        // overwrites an element of an existing container).  A BARE var destination is a
        // definitional fill (`d = s.f` materialising a value-struct copy), which is neither
        // a read nor the dead-store signal — the same split `classify_write_base` already
        // draws for the append family (loft#670).
        Value::Call(op, args) if *op == cx.copy_record && args.len() >= 2 => {
            classify_access(&args[0], cx, acc);
            let elem_write = !matches!(args[1].unspan(), Value::Var(_));
            classify_write_base(&args[1], cx, acc, elem_write);
            for a in &args[2..] {
                classify_access(a, cx, acc);
            }
        }
        other => other.for_each_child(&mut |c| classify_access(c, cx, acc)),
    }
}

/// Descend a `len` subject to its root var WITHOUT counting a read there, mirroring
/// [`classify_write_base`]'s walk.  `len(d)` and `len(d.f)` both observe only a count;
/// any index expression along the chain is a real read and is classified normally.
fn classify_length_subject(node: &Value, cx: &AccessCx, acc: &mut [(u16, u16)]) {
    match node.unspan() {
        Value::Var(_) => {}
        Value::Call(op, args) if cx.projs.contains(op) && !args.is_empty() => {
            classify_length_subject(&args[0], cx, acc);
            for a in &args[1..] {
                classify_access(a, cx, acc);
            }
        }
        other => classify_access(other, cx, acc),
    }
}

/// Descend the write-destination base (`args[0]` of a write-through op) through its
/// projection chain (`d.f[i]=x` → `OpSet…(OpGetField(Var(d),f), …)`) to the root var,
/// counting projection INDEX args as ordinary reads. The root var is recorded as a
/// write-target only when `count_target` (an `OpSet*` mutation); otherwise it is a
/// definitional/bulk destination — neither read nor target.
fn classify_write_base(node: &Value, cx: &AccessCx, acc: &mut [(u16, u16)], count_target: bool) {
    match node.unspan() {
        Value::Var(v) => {
            if count_target {
                bump_write(acc, *v);
            }
        }
        Value::Call(op, args) if cx.projs.contains(op) && !args.is_empty() => {
            classify_write_base(&args[0], cx, acc, count_target);
            for a in &args[1..] {
                classify_access(a, cx, acc);
            }
        }
        other => classify_access(other, cx, acc),
    }
}

struct Uses {
    get_field: u32,
    op_append: u32,
    op_database: u32,
    op_free: u32,
    /// @PLN90 — `OpCopyRecord` def_nr: a record deep-copy (`v[i] = e`, a `?? E{…}` default
    /// element, a struct copy). Not append-based, so the var-buffer / construction /
    /// return-buffer branches never see it; recorded here so the decision covers it.
    op_copy_record: u32,
    /// Shared with [`OpSets`] — an `Arc` so building `Uses` per function is a refcount
    /// bump, not a rebuild of a set that cannot have changed.
    projections: std::sync::Arc<HashSet<u32>>,
    value_readers: std::sync::Arc<HashSet<u32>>,
    /// @PLN90 item 3 — ops that write through their first argument (see
    /// [`is_first_arg_write_name`]).
    write_first_arg: std::sync::Arc<HashSet<u32>>,
    /// Pre-order position counter — a total order on nodes that, OUTSIDE loops,
    /// matches execution order (Tier 1 uses it to prove a source is unmutated
    /// after the copy-fill). Bumped once per visited node.
    pos: usize,
    /// Loop nesting at the current node (`Value::Loop`, which for-loops desugar
    /// to). Back-edges break the position↔execution correspondence, so Tier 1
    /// refuses any copy whose fill sits at depth > 0.
    loop_depth: u32,
    /// @PLN90 item 2 — track source locations for the report. Only ON when
    /// `LOFT_COPY_SURVIVAL` is set (positions are needed only for the survival report), so the
    /// default hot path pays no `Position` clone and stays byte-identical.
    track_pos: bool,
    /// @PLN90 item 2 — the nearest ENCLOSING span position (breadcrumb). The copy ops
    /// (`OpAppendVector` / `OpCopyRecord`) carry no span themselves, so a copy site borrows the
    /// most recent spanned node's position. Updated (when `track_pos`) at every spanned node.
    cur_pos: Option<Position>,
    /// @PLN90 item 3 — the max position at which a var is actually WRITTEN (a `Set` target, an
    /// append/insert target, a record-copy target). Unlike `other_max_pos` (any non-reader use,
    /// which counts a read-only pass-to-callee or being another copy's source), this counts only
    /// real mutations — so "the source is mutated AFTER the copy" (⇒ a borrow is unsound ⇒
    /// Forced) is precise, and a read-only survivor stays Avoidable. Only tracked when
    /// `track_pos` (read solely by `survival_class`); `other_max_pos` is left untouched so the
    /// shipped var-buffer elision stays byte-identical.
    mut_max_pos: HashMap<u16, usize>,
    /// @PLN90 item 4 — the position of each var's FIRST definition (`Set`). Compared against a
    /// copy's enclosing-loop entry to tell a source defined OUTSIDE the loop (duplicated every
    /// iteration → unbound) from a per-iteration LOCAL (consumed each pass → a move). Tracked
    /// only when `track_pos`.
    first_def_pos: HashMap<u16, usize>,
    /// @PLN90 item 4 — a stack of enclosing-loop entry positions (the `Value::Loop` node's pos).
    /// A source whose first def is `< loop_entry` existed before the loop (outside); `>=` means
    /// it is defined inside the loop body (per-iteration). Tracked only when `track_pos`.
    loop_entry: Vec<usize>,
    /// Max position at which a var appeared in a NON-reader (write / escape /
    /// pass-to-callee) position — EXCLUDING the benign scope ops (`OpFreeRef` /
    /// `Drop`) and the copy-fill itself. For a source `x`, "all such positions <
    /// copy-fill position" means `x` is only constructed (never mutated) before
    /// the snapshot — the Tier-1 ¬D2 fact the set-based `ineligible` can't give.
    other_max_pos: HashMap<u16, usize>,
    /// Position of each copy var's `OpAppendVector` fill (the snapshot moment).
    copyfill_pos: HashMap<u16, usize>,
    /// Copy vars whose fill sits inside a loop (Tier-1 ineligible).
    copyfill_in_loop: HashSet<u16>,
    /// @PLN90 (bound-vs-unbound survival split) — max position at which a var appeared in
    /// ANY position (reader OR non-reader), i.e. its last use. Unlike `other_max_pos` (which
    /// tracks only non-reader uses) this includes plain reads, so "does the source survive
    /// (a use strictly after the copy site)?" — the move-vs-copy discriminator — can be
    /// answered. See `survival_class`.
    last_use_pos: HashMap<u16, usize>,
    /// @PLN131 Q6.1 — WHERE that last use is, kept in step with `last_use_pos`.
    ///
    /// `last_use_pos` is a traversal index, which answers "does the source survive?" but
    /// cannot answer "survive *where*". A suggestion has to name the line: the difference
    /// between "`src` is unused after here" and "`src` is used again at line 12" is the
    /// difference between a veteran affirming a condition in one second and going hunting
    /// for it. The walker already tracks `cur_pos` for the copy site; this records the same
    /// thing at the USE.
    last_use_loc: HashMap<u16, Option<Position>>,
    /// @PLN90 — field-target appends `OpAppendVector(OpGetField(rec, fld), src)`:
    /// `(base record var, source base var, copy-site-end pos, loop-survives, source location)`.
    /// This is the struct/enum-construction copy (`S { f: src }`) and `x.field += src` — the
    /// source is deep-copied into the field, which the var-buffer copy idiom above never sees.
    /// The position (taken AFTER the args are walked) + `loop_survives` (item 4: the source
    /// outlives the enclosing loop) drive the survival split; the location (item 2) is for the
    /// report.
    construct_copy: Vec<(Option<u16>, Option<u16>, usize, bool, Option<Position>)>,
    /// @PLN90 — `OpCopyRecord` deep-copies: `(dest base var, SOURCE base var, copy-site-end pos,
    /// loop-survives, source location)`. A record copy (`v[i] = e`, a `?? E{…}` default element,
    /// a struct copy) — not append-based, so the branches above miss it. The same-var no-op
    /// alias is excluded when recorded.
    record_copy: Vec<(Option<u16>, Option<u16>, usize, bool, Option<Position>)>,
    /// Vars that appeared in a non-reader position (⇒ not borrow-eligible). The
    /// copy-fill `OpAppendVector(v, src.f)` is *excluded* — it is the copy machinery,
    /// not a user mutation of `v`.
    ineligible: HashSet<u16>,
    /// Number of `Set(v, _)` for each var (a reassignment is > 1).
    def_count: HashMap<u16, u32>,
    /// Vars allocated a fresh store by `OpDatabase(vdb, …)`.
    database_vars: HashSet<u16>,
    /// For `Set(v, OpGetField(Var(vdb), …))`: the backing buffer `vdb`.
    def_vdb: HashMap<u16, u16>,
    /// Source bases of `OpAppendVector(Var(v), <src>)` (one entry per append).
    append_src: HashMap<u16, Vec<Option<u16>>>,
    /// Source EXPRESSIONS of `OpAppendVector(Var(v), <src>)` — kept for the
    /// elision plan (the field-read to inline in place of `v`'s reads).
    append_expr: HashMap<u16, Vec<Value>>,
}

/// An elidable copy: replace `v`'s reads with `source` and drop the copy idiom
/// (the `vdb` buffer + its alloc/append/free).
#[derive(Clone, Debug)]
pub struct ElidePlan {
    pub var: u16,
    pub vdb: u16,
    pub source: Value,
    /// The base var of `source` (`s` in `v = s.f`) — the owner the borrowers below
    /// must re-point to once `v` is gone.
    pub source_base: u16,
    /// Vars that BORROW `v` (`e = v[i]` / `e = v.fld`, `deps` ∋ `v`). After inlining,
    /// each borrows the live source element, so its dep must be re-pointed
    /// `v → source_base`. Only present (the plan only emitted) when every borrower is
    /// read-only and non-escaping — else the copy is kept (value semantics).
    pub borrowers: Vec<u16>,
}

/// @PLN90 phase B — a construction / record copy that can be lowered as a MOVE (a store transfer)
/// instead of a deep copy: the source is a **dead-after owned local**, so its store transfers into
/// the field/element (C86's last-use elision — the same rule `ElidePlan` runs for the var-buffer
/// idiom). Produced only under `LOFT_MOVE_ELIDE`. B1.2 computes + dumps it; the lowering that
/// consumes it lands in B1.3+ (no codegen change yet — byte-identical off). Design + captured IR:
/// `doc/claude/plans/90-copy-diagnostics/phase-b-design.md`.
#[derive(Clone, Debug)]
pub struct MovePlan {
    /// The container being built / assigned into (struct rec or vector target); `u16::MAX` if it
    /// has no named var (an anonymous element slot).
    pub container: u16,
    /// The dead-after owned-local source var whose store transfers into the field/element.
    pub source: u16,
    /// Construction (`S { f: src }` / `x.field += src`) vs record (`v[i] = e` / `o.f = src`).
    pub kind: MoveKind,
    /// The copy-site-end position — the lowering (B1.3) keys the transfer + free-suppression here.
    pub copy_end: usize,
    /// The source location, for the B1.2 detection dump.
    pub loc: Option<Position>,
}

/// The copy shape a [`MovePlan`] covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveKind {
    /// `S { f: src }` / `x.field += src` — a field-target append (`construct_copy`).
    Construct,
    /// `v[i] = e` / `o.f = src` — an `OpCopyRecord` (`record_copy`).
    Record,
}

impl Uses {
    /// @PLN90 item 3 — record a real WRITE of `base` at `pos` (max). Only tracked when
    /// `track_pos` (read solely by `survival_class`), so the default path is unaffected.
    fn mark_write(&mut self, base: Option<u16>, pos: usize) {
        if self.track_pos
            && let Some(v) = base
        {
            let e = self.mut_max_pos.entry(v).or_insert(0);
            *e = (*e).max(pos);
        }
    }

    /// @PLN90 item 4 — is a copy of `src` at the current site a repeated duplicate of a value
    /// that lives OUTSIDE the enclosing loop (⇒ unbound, one copy per iteration), rather than a
    /// per-iteration local consumed each pass (⇒ a move)? False outside any loop. A source with
    /// no `Set` def (a parameter) is outside by definition.
    fn loop_survives(&self, src: Option<u16>) -> bool {
        if self.loop_depth == 0 {
            return false;
        }
        let Some(&entry) = self.loop_entry.last() else {
            return false;
        };
        match src {
            None => false,
            Some(s) => self.first_def_pos.get(&s).is_none_or(|&d| d < entry),
        }
    }

    fn visit(&mut self, node: &Value, ctx: Ctx) {
        let pos = self.pos;
        self.pos += 1;
        // @PLN90 item 2 — breadcrumb the nearest enclosing span position (only when tracking,
        // so the default path pays nothing). A copy op borrows this for its report location.
        if self.track_pos {
            if let Some(p) = node.span_pos() {
                self.cur_pos = Some(p.clone());
            } else if let Value::Line(n) = node {
                // @PLN90 S5.2 — a bare line marker is a coarse fallback for copies that
                // sit under no span (an inline construct's `OpAppendVector`, an `[]` fold).
                // Empty `file` signals "borrow the caller's source file" (report/warn fill
                // it); a real span later in the same statement overrides this.
                self.cur_pos = Some(Position {
                    file: String::new(),
                    line: *n,
                    pos: 0,
                });
            }
        }
        // @PLN90 item 3 — record a REAL write of a var at this position, for `mut_max_pos` (the
        // Forced test). Covers every write op (first-arg family + `OpCopyRecord`'s second-arg
        // dest); the `Set` arm below adds reassign targets. Gated inside `mark_write`.
        if let Value::Call(d, args) = node.unspan() {
            if self.write_first_arg.contains(d) {
                self.mark_write(args.first().and_then(|a| base_var(a, self.get_field)), pos);
            } else if *d == self.op_copy_record {
                self.mark_write(args.get(1).and_then(|a| base_var(a, self.get_field)), pos);
            }
        }
        match node.unspan() {
            Value::Var(v) => {
                // @PLN90 — last use in ANY position (the survival discriminator).
                let lu = self.last_use_pos.entry(*v).or_insert(0);
                // @PLN131 Q6.1 — the location moves only when the position does, so the two
                // never disagree about which use they describe.
                if pos >= *lu {
                    self.last_use_loc.insert(*v, self.cur_pos.clone());
                }
                *lu = (*lu).max(pos);
                if ctx != Ctx::ReaderArg {
                    self.ineligible.insert(*v);
                    let e = self.other_max_pos.entry(*v).or_insert(0);
                    *e = (*e).max(pos);
                }
            }
            // A loop body's back-edge can re-execute a write after a read, so the
            // pre-order position no longer tracks execution order inside it — bump
            // the depth so Tier 1 can refuse copies filled here.
            Value::Loop(b) => {
                self.loop_depth += 1;
                // @PLN90 item 4 — the loop's entry position: a source whose first def is < this
                // lives outside the loop (copied every iteration); >= means a per-iteration local.
                if self.track_pos {
                    self.loop_entry.push(pos);
                }
                for op in &b.operators {
                    self.visit(op, Ctx::Other);
                }
                if self.track_pos {
                    self.loop_entry.pop();
                }
                self.loop_depth -= 1;
            }
            // Scope machinery, not a user mutation: `OpFreeRef(x)` / `Drop(x)` must
            // not count as a non-reader use of `x` (a free placed AFTER the last
            // read is exactly what we want; the scope pass repositions it post-
            // elision anyway). Visit nothing — recursing would mark the arg `Other`.
            Value::Call(d, _) if *d == self.op_free => {}
            Value::Drop(_) => {}
            Value::Set(v, rhs) => {
                *self.def_count.entry(*v).or_insert(0) += 1;
                // @PLN90 item 3 — a `Set` writes `v` (its def is before any copy of it; a
                // reassign after a copy is the mutation that forces the copy to be independent).
                self.mark_write(Some(*v), pos);
                // @PLN90 item 4 — remember where `v` was FIRST defined (loop-inside vs -outside).
                if self.track_pos {
                    self.first_def_pos.entry(*v).or_insert(pos);
                }
                // Fresh-buffer def: `v = OpGetField(vdb, 0, _)` where vdb is OpDatabase'd.
                if let Value::Call(d, args) = rhs.unspan()
                    && *d == self.get_field
                    && let Some(Value::Var(vdb)) = args.first().map(Value::unspan)
                {
                    self.def_vdb.insert(*v, *vdb);
                }
                // The def-read is benign for its source; visit it as a read.
                self.visit(rhs, Ctx::ReaderArg);
            }
            Value::Call(d, args) if *d == self.op_append => {
                // `OpAppendVector(target, src, rec_tp)`. (The append target's write is recorded
                // by the general write-mark at the top of `visit`.)
                if let Some(Value::Var(v)) = args.first().map(Value::unspan) {
                    // Copy-fill into a plain local — record the source base; do NOT
                    // mark `v` ineligible (this append IS the copy, not a user write).
                    self.copyfill_pos.insert(*v, pos);
                    if self.loop_depth > 0 {
                        self.copyfill_in_loop.insert(*v);
                    }
                    let src = args.get(1).and_then(|s| base_var(s, self.get_field));
                    self.append_src.entry(*v).or_default().push(src);
                    if let Some(s) = args.get(1) {
                        self.append_expr
                            .entry(*v)
                            .or_default()
                            .push(s.unspan().clone());
                    }
                    for a in &args[1..] {
                        self.visit(a, Ctx::ReaderArg);
                    }
                } else {
                    // Append into a field / non-var target — a real mutation.
                    // @PLN90 — when the target is a struct/enum FIELD
                    // (`OpAppendVector(OpGetField(rec, fld), src)`) the source is
                    // deep-copied into the field: a structure copy the var-buffer idiom
                    // above never records (struct/enum construction `S { f: src }`, or
                    // `x.field += src`). Capture (base record, source base) so the
                    // copy-vs-borrow decision + survival split cover it.
                    let cc = if let Some(Value::Call(d0, _)) = args.first().map(Value::unspan)
                        && *d0 == self.get_field
                    {
                        let rec = args.first().and_then(|t| base_var(t, self.get_field));
                        let src = args.get(1).and_then(|s| base_var(s, self.get_field));
                        Some((rec, src))
                    } else {
                        None
                    };
                    for a in args {
                        self.visit(a, Ctx::Other);
                    }
                    // Position taken AFTER the args are walked: a use strictly greater is a
                    // use of the source AFTER this copy (⇒ the source survives). The copy op
                    // carries no span, so borrow the nearest enclosing one (item 2). `loop_surv`
                    // (item 4) = the source outlives the enclosing loop (copied every iteration).
                    if let Some((rec, src)) = cc {
                        let loop_surv = self.loop_survives(src);
                        self.construct_copy.push((
                            rec,
                            src,
                            self.pos,
                            loop_surv,
                            self.cur_pos.clone(),
                        ));
                    }
                }
            }
            Value::Call(d, args) if self.projections.contains(d) => {
                // Projection: arg 0 (the base container) is accessed in the INCOMING
                // context — a projection inside a write writes its base; inside a read
                // reads it. The remaining args (an index) are pure reads.
                let mut it = args.iter();
                if let Some(base) = it.next() {
                    self.visit(base, ctx);
                }
                for a in it {
                    self.visit(a, Ctx::ReaderArg);
                }
            }
            Value::Call(d, args) if *d == self.op_copy_record => {
                // @PLN90 — `OpCopyRecord(source, dest, tp)` deep-copies one record: arg 0 is the
                // SOURCE (`data`), arg 1 is the DEST (`to`) — verified against `State::copy_record`
                // (io.rs) and `parser::find_written_vars` (dest = second arg). The survival split
                // classifies the SOURCE's fate, and the row's target/name is the DEST. Skip the
                // same-var no-op alias (`OpCopyRecord(x, x)` — the runtime short-circuits it).
                // The dest's write is recorded by the general write-mark at the top of `visit`.
                let src = args.first().and_then(|a| base_var(a, self.get_field));
                let dest = args.get(1).and_then(|a| base_var(a, self.get_field));
                let record = !(dest.is_some() && dest == src);
                let c = if self.value_readers.contains(d) {
                    Ctx::ReaderArg
                } else {
                    Ctx::Other
                };
                for a in args {
                    self.visit(a, c);
                }
                // Position AFTER the args: a later use of the source ⇒ it survives. The
                // `OpCopyRecord` carries no span, so borrow the nearest enclosing one. The row's
                // target/name is the DEST; the classified var is the SOURCE (`src`). `loop_surv`
                // (item 4) = the source outlives the enclosing loop (copied every iteration).
                if record {
                    let loop_surv = self.loop_survives(src);
                    self.record_copy
                        .push((dest, src, self.pos, loop_surv, self.cur_pos.clone()));
                }
            }
            Value::Call(d, args) => {
                if *d == self.op_database
                    && let Some(Value::Var(vdb)) = args.first().map(Value::unspan)
                {
                    self.database_vars.insert(*vdb);
                }
                let c = if self.value_readers.contains(d) {
                    Ctx::ReaderArg
                } else {
                    Ctx::Other
                };
                for a in args {
                    self.visit(a, c);
                }
            }
            other => {
                // Structural nodes (Block, If, Loop, For, Return, Drop, Insert, …):
                // recurse with the conservative default — a `Var` directly under any
                // of these (a return/block tail, a stored value) is a non-reader use,
                // which correctly marks it ineligible.
                other.for_each_child(&mut |c| self.visit(c, Ctx::Other));
            }
        }
    }
}

/// Compute the borrow-vs-copy verdict for every elidable vector-copy binding in
/// `code`, up to `max_tier` (0 = the shipped param-source rule; 1 = also
/// read-only-local sources, ordering-proven). Higher tiers are additive: a tier-1
/// run still emits every tier-0 Borrow.
/// Build + walk the copy/borrow use-facts for one function body. Extracted from `analyze_fn` so the
/// report-only link-safety oracle (`link_safety_of`) reads the SAME facts the shipped elision does,
/// with no second, drifting analysis.
fn collect_uses(code: &Value, data: &Data, survival_on: bool) -> Uses {
    let ops = data.op_sets();
    let mut u = Uses {
        get_field: data.def_nr("OpGetField"),
        op_append: data.def_nr("OpAppendVector"),
        op_database: data.def_nr("OpDatabase"),
        op_free: data.def_nr("OpFreeRef"),
        op_copy_record: data.def_nr("OpCopyRecord"),
        projections: std::sync::Arc::clone(&ops.projections),
        value_readers: std::sync::Arc::clone(&ops.value_readers),
        write_first_arg: std::sync::Arc::clone(&ops.write_first_arg),
        ineligible: HashSet::new(),
        def_count: HashMap::new(),
        database_vars: HashSet::new(),
        def_vdb: HashMap::new(),
        append_src: HashMap::new(),
        append_expr: HashMap::new(),
        pos: 0,
        loop_depth: 0,
        track_pos: survival_on,
        cur_pos: None,
        mut_max_pos: HashMap::new(),
        first_def_pos: HashMap::new(),
        loop_entry: Vec::new(),
        other_max_pos: HashMap::new(),
        copyfill_pos: HashMap::new(),
        copyfill_in_loop: HashSet::new(),
        last_use_pos: HashMap::new(),
        last_use_loc: HashMap::new(),
        construct_copy: Vec::new(),
        record_copy: Vec::new(),
    };
    u.visit(code, Ctx::Other);
    // `def_vdb` means "`v` is the handle of a FRESH buffer", and its own doc says *where vdb is
    // OpDatabase'd* — a condition the walk cannot check at insertion, because the `OpDatabase`
    // may not have been visited yet.  Enforce it here, once the whole body has been seen, so the
    // map matches what every consumer reads it as.
    //
    // Without it ANY `v = OpGetField(x, …)` counted as owning a transferable store — including a
    // read of an EXISTING element through a borrow, `hs = p.0` over a `vector<(…)>`.
    // `move_elidable_source`'s last gate is exactly "owns a transferable store", so it admitted
    // `hs`, and `move_rewrite` then dropped the `OpCopyRecord`: sound when the source is
    // CONSTRUCTED (its build ops get retargeted onto the destination), but `hs` has no build ops,
    // so the copy WAS the write and `p.1 = hs` vanished from the IR — silently, on both backends
    // (`formal/binding.md` D-bind-12).
    let allocated = std::mem::take(&mut u.database_vars);
    u.def_vdb.retain(|_, vdb| allocated.contains(vdb));
    u.database_vars = allocated;
    u
}

/// @PLN102 transparent-link widening — the per-bind SAFETY predicate (step 2). The single source of
/// truth read by BOTH the report-only oracle (`link_safety_of`) and the codegen widening
/// (`analyze_fn` under `LOFT_LINK_WIDEN`), so the gate-on suite validates exactly what codegen uses.
/// See `link_safety_of` for the invariant + the soundness argument.
fn bind_link_safe(u: &Uses, function: &Function, v: u16, src: Option<u16>) -> bool {
    // A SELF-source (`v += v`, so `append_src[v] = [Some(v)]`) is a self-append, not a copy of a
    // distinct store — a "link" would be `v` borrowing itself, which drops its buffer and leaves it
    // slotless. It is never a link candidate. (The shipped tiers dodge it via `src_is_param` /
    // `src_local_stable` never firing here by default; the widening must exclude it explicitly.)
    if src == Some(v) {
        return false;
    }
    let single_def = u.def_count.get(&v).copied().unwrap_or(0) == 1;
    let v_non_escaping = !u.ineligible.contains(&v);
    let source_outlives = src.is_some_and(|s| {
        function.is_argument(s)
            || (u.def_count.get(&s).copied().unwrap_or(0) <= 1
                && u.last_use_pos.get(&s).copied().unwrap_or(0)
                    >= u.last_use_pos.get(&v).copied().unwrap_or(0))
    });
    single_def && src.is_some() && v_non_escaping && source_outlives
}

/// @PLN102 transparent-link widening — the per-bind OBSERVABILITY predicate (step 3), ALIAS-AWARE.
/// The single source of truth for both `link_observability_of` and the codegen widening. See
/// `link_observability_of` for the invariant. `false` if the bind has no recorded copy-fill.
fn bind_link_unobservable(u: &Uses, function: &Function, v: u16, src: Option<u16>) -> bool {
    let Some(fill) = u.copyfill_pos.get(&v).copied() else {
        return false;
    };
    let local_readonly = !u.ineligible.contains(&v);
    let n = function.next_var();
    let source_stable = src.is_some_and(|base| {
        let base_stable = u.other_max_pos.get(&base).copied().unwrap_or(0) < fill;
        let aliases_stable = (0..n).filter(|&b| b != v && b != base).all(|b| {
            if function.tp(b).base().depend().contains(&base) {
                u.other_max_pos.get(&b).copied().unwrap_or(0) < fill
            } else {
                true
            }
        });
        base_stable && aliases_stable
    });
    local_readonly && source_stable
}

/// @PLN102 transparent-link widening — build step 2: the REPORT-ONLY safety oracle. For each
/// single-source copy-fill bind `v = <src>.f` it returns `(v, base, safe)`, where `safe` is the
/// conservative "a shared-store LINK would be UAF-safe here" verdict:
///
///   * the source `base`'s store OUTLIVES `v` — a parameter (caller-owned, alive across the frame),
///     or a non-reassigned local whose last use is at/after `v`'s (its store is not reclaimed while
///     `v` is live), AND
///   * `v` does NOT escape (`v ∉ ineligible` — no return / store / pass-to-callee that could outlive
///     `base`).
///
/// SOUND BY CONSERVATISM: last-use ordering under-approximates store lifetime (a store lives to scope
/// exit), and `ineligible` also excludes a mutated local, so this can only MISS a safe link, never
/// invent an unsafe one — it cannot green-light a #415 dangle. Drives no codegen. Pinned by
/// `tests/link_safe_oracle.rs` against the safety matrix.
fn link_safety_of(code: &Value, function: &Function, data: &Data) -> Vec<(u16, Option<u16>, bool)> {
    let u = collect_uses(code, data, false);
    let mut vars: Vec<u16> = u.append_src.keys().copied().collect();
    vars.sort_unstable();
    let mut out = Vec::new();
    for v in vars {
        // Only the single-source copy-fill idiom (a fresh `OpDatabase` buffer filled by one append).
        let fresh_buffer = u
            .def_vdb
            .get(&v)
            .is_some_and(|vdb| u.database_vars.contains(vdb));
        let appends = &u.append_src[&v];
        if !fresh_buffer || appends.len() != 1 {
            continue;
        }
        out.push((v, appends[0], bind_link_safe(&u, function, v, appends[0])));
    }
    out
}

/// `LOFT_DUMP_LINK_SAFE` — emit one `link-safe-dbg:` line per copy-fill bind of every USER function
/// (`STD_SOURCE` skipped — its facts are stable and would flood the dump). Report-only.
pub fn dump_link_safety(data: &Data) {
    if !crate::keys::dump_link_safe() {
        return;
    }
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) || def.source == crate::data::STD_SOURCE {
            continue;
        }
        for (v, base, safe) in link_safety_of(&def.code, &def.variables, data) {
            let base_name = base.map_or("-".to_string(), |b| def.variables.name(b).to_string());
            eprintln!(
                "link-safe-dbg: fn={} var={} base={} safe={}",
                def.name,
                def.variables.name(v),
                base_name,
                u8::from(safe)
            );
        }
    }
}

/// @PLN102 transparent-link widening — build step 3: the REPORT-ONLY observability oracle. For each
/// single-source copy-fill bind `a = s.v` it returns `(a, base, unobservable)`, where a shared-store
/// LINK is UNOBSERVABLE (copy ≡ link) iff **nothing mutates either side's store after the bind**:
///
///   * the local `a` is not mutated after the bind (`a ∉ ineligible` — the copy-idiom-aware
///     read-only fact), AND
///   * the SOURCE store is not mutated after the bind — `other_max_pos[base] < fill` (no non-reader
///     use of the base after the copy-fill), ALIAS-AWARE: every var whose store aliases the base (its
///     `deps` reference `base` — a sibling `&`-reference, an element view) is ALSO stable after the
///     fill. Without the alias clause, `a = s.v; b = &s.v; b[i]=x; read a` would falsely read
///     unobservable (the write via `b` reaches `s.v`'s store, so a link would reflect it).
///
/// SOUND BY CONSERVATISM: `other_max_pos` counts a pre-bind def too, but the bind's own fill is later,
/// so a stable source reads `< fill`; a set-based over-count only MISSES a link, never invents an
/// observable one. Drives no codegen. Pinned by `tests/link_obs_oracle.rs` against Matrix O.
fn link_observability_of(
    code: &Value,
    function: &Function,
    data: &Data,
) -> Vec<(u16, Option<u16>, bool)> {
    let u = collect_uses(code, data, false);
    let mut vars: Vec<u16> = u.append_src.keys().copied().collect();
    vars.sort_unstable();
    let mut out = Vec::new();
    for a in vars {
        let fresh_buffer = u
            .def_vdb
            .get(&a)
            .is_some_and(|vdb| u.database_vars.contains(vdb));
        let appends = &u.append_src[&a];
        if !fresh_buffer || appends.len() != 1 {
            continue;
        }
        out.push((
            a,
            appends[0],
            bind_link_unobservable(&u, function, a, appends[0]),
        ));
    }
    out
}

/// `LOFT_DUMP_LINK_OBS` — emit one `link-obs-dbg:` line per copy-fill bind of every USER function.
pub fn dump_link_observability(data: &Data) {
    if !crate::keys::dump_link_obs() {
        return;
    }
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) || def.source == crate::data::STD_SOURCE {
            continue;
        }
        for (v, base, unobs) in link_observability_of(&def.code, &def.variables, data) {
            let base_name = base.map_or("-".to_string(), |b| def.variables.name(b).to_string());
            eprintln!(
                "link-obs-dbg: fn={} var={} base={} unobs={}",
                def.name,
                def.variables.name(v),
                base_name,
                u8::from(unobs)
            );
        }
    }
}

fn analyze_fn(
    code: &Value,
    function: &Function,
    data: &Data,
    max_tier: u8,
) -> (Vec<VerdictRow>, Vec<ElidePlan>, Vec<MovePlan>) {
    analyze_fn_survival(code, function, data, max_tier, false)
}

/// [`analyze_fn`] with the survival split forced on.
///
/// `force_survival` exists for the default-on copy notice (@PLN130 F5): `warn_copies` reads
/// `r.survival` to pick the actionable rows, so it needs the split computed even with no env
/// flag set. It must NOT be forced globally — the flag-off path keeps the phase-1
/// classification verbatim, and turning it on for everyone reclassified rows the
/// `use_analysis` tests pin (4 failures, measured).
fn analyze_fn_survival(
    code: &Value,
    function: &Function,
    data: &Data,
    max_tier: u8,
    force_survival: bool,
) -> (Vec<VerdictRow>, Vec<ElidePlan>, Vec<MovePlan>) {
    // @PLN90 — the survival split + its report locations, produced under LOFT_COPY_SURVIVAL
    // (the raw dev dump), the user-facing `--report-copies`, or `force_survival` (the
    // default-on notice). Read once so the walk (`track_pos`) and the classification agree.
    let survival_on = force_survival
        || std::env::var_os("LOFT_COPY_SURVIVAL").is_some()
        || crate::keys::report_copies_enabled()
        || crate::keys::warn_copies_enabled();
    let u = collect_uses(code, data, survival_on);

    // The SOURCE-mutation fact (¬D2) is the parser's mature, interprocedural
    // mutation analysis — `find_written_vars` also catches a source handed to a
    // mutating callee, which our intraprocedural walk would miss. (We cannot use it
    // for the LOCAL `v`: `find_written_vars` marks a var written by its own
    // defining `Set`, so every bound local is "written"; `v`'s read-only-ness needs
    // the copy-idiom-aware walk above, which excludes `v`'s def and copy-fill.)
    let written = {
        let mut w = HashSet::new();
        crate::parser::find_written_vars(code, data, &mut w, &mut HashMap::new());
        w
    };

    let mut vars: Vec<u16> = u.append_src.keys().copied().collect();
    vars.sort_unstable();

    let mut rows = Vec::new();
    let mut plans = Vec::new();
    for v in vars {
        let appends = &u.append_src[&v];
        // The copy idiom: v is a fresh OpDatabase buffer, filled by exactly one append.
        let fresh_buffer = u
            .def_vdb
            .get(&v)
            .is_some_and(|vdb| u.database_vars.contains(vdb));
        if !fresh_buffer || appends.len() != 1 {
            // @PLN90 phase 1 — the return-buffer copy. When the single-append target is
            // not a fresh local buffer but IS an argument vector, it is the return buffer
            // (a passed-in buffer the function fills and returns): `fn f(b: Box) -> vector
            // { b.rows }` materialises `b.rows` into `__retbuf`. The var-buffer idiom skips
            // it (the buffer is a param, not an `OpDatabase` local), so emit a Copy row for
            // coverage. Diagnostic only — `continue` below means no `ElidePlan`, so no
            // codegen change (and eliding it would be the P4 borrowed-return).
            if appends.len() == 1 && function.is_argument(v) {
                rows.push(VerdictRow {
                    var_nr: v,
                    var_name: function.name(v).to_string(),
                    source: appends[0].unwrap_or(u16::MAX),
                    verdict: Verdict::Copy,
                    reason: "materialised into the return buffer (field / whole-vector return copy)",
                    // AVOIDABLE: returning a borrowed view of the field is sound (the caller
                    // keeps the subject alive); the copy is here only because the borrowed
                    // return path is not yet correct (@PLN85 P4). Eliminating it = that fix.
                    class: CopyClass::Avoidable,
                    loc: None,
                    source_last_use: None,
                    survival: false,
                });
            }
            continue; // not a single-source local copy — not ours to elide
        }
        let src = appends[0];
        let single_def = u.def_count.get(&v).copied().unwrap_or(0) == 1;
        let v_readonly = !u.ineligible.contains(&v);
        // A PARAMETER is defined at entry, a definition the def count does not see: its
        // reads ahead of the copy-fill see the caller's value, and rewriting them onto the
        // source would answer the source there (`for … { s += v[i]; v = w }` read `w` on
        // the first turn).  The verdicts below are for a LOCAL, as their own words say.
        let v_is_local = !function.is_argument(v);
        let src_is_param = src.is_some_and(|s| function.is_argument(s));
        let src_unmutated = src.is_some_and(|s| !written.contains(&s));

        // TIER 1 (max_tier >= 1): a read-only LOCAL source. The set-based facts
        // can't prove a local unmutated (its construction looks like a write), so
        // use the ordering fact: the copy-fill is on a straight-line path (not in a
        // loop) AND every non-reader appearance of the source precedes the fill —
        // i.e. the source is only constructed, never mutated/freed, after the
        // snapshot (¬D2), and the inlining itself extends the source's lifetime over
        // v's reads (¬D3). `v` read-only/non-escaping is the same ¬D1 as tier 0.
        let src_local_stable = max_tier >= 1
            && src.is_some_and(|s| !function.is_argument(s))
            && !u.copyfill_in_loop.contains(&v)
            && match (src, u.copyfill_pos.get(&v)) {
                (Some(s), Some(&fill)) => u.other_max_pos.get(&s).copied().unwrap_or(0) < fill,
                _ => false,
            };

        // @PLN90 — the warning bucket. Avoidable = a borrow would be sound, blocked only by
        // analysis conservatism (an unproven read-only local source) — the worklist.
        // Implicit = a literal source is construction, not a copy of existing data (owned by
        // the model — silent). Forced = the result escapes / is reassigned / mutated, or the
        // source is mutated (the value must own its store). `Borrow` is already eliminated.
        let (verdict, reason, class) =
            if single_def && v_is_local && v_readonly && src_is_param && src_unmutated {
                (
                    Verdict::Borrow,
                    "tier0: read-only local, unmutated param source",
                    CopyClass::Eliminated,
                )
            } else if single_def && v_is_local && v_readonly && src_local_stable {
                (
                    Verdict::Borrow,
                    "tier1: read-only local, ordering-proven read-only local source",
                    CopyClass::Eliminated,
                )
            } else if crate::keys::link_widen_enabled()
                && bind_link_safe(&u, function, v, src)
                && bind_link_unobservable(&u, function, v, src)
            {
                // @PLN102 build step 4 — the transparent-link WIDENING (gated `LOFT_LINK_WIDEN`). A bind
                // the tiers above leave as a copy is realized as a shared-store link when it is provably
                // SAFE (source outlives the local, no escape) AND UNOBSERVABLE (neither side's store is
                // mutated after the bind, ALIAS-AWARE) — the two oracles proven report-only in steps 2/3.
                // The observable result is unchanged (copy ≡ link here); it just realizes more links. The
                // `ElidePlan` production below runs unchanged, incl. its own borrower-safety gate. Dead
                // when the flag is off ⇒ byte-identical.
                (
                    Verdict::Borrow,
                    "widen: safe + unobservable link (LOFT_LINK_WIDEN)",
                    CopyClass::Eliminated,
                )
            } else if src.is_none() {
                (
                    Verdict::Copy,
                    "source is not a plain var/field (e.g. a literal)",
                    CopyClass::Implicit,
                )
            } else if !single_def {
                (
                    Verdict::Copy,
                    "reassigned (multiple defs)",
                    CopyClass::Forced,
                )
            } else if !v_readonly {
                (Verdict::Copy, "local mutated or escapes", CopyClass::Forced)
            } else if !src_is_param && !src_local_stable {
                (
                    Verdict::Copy,
                    "source not a parameter / not provably read-only local",
                    CopyClass::Avoidable,
                )
            } else {
                (Verdict::Copy, "source mutated", CopyClass::Forced)
            };

        if verdict == Verdict::Borrow
            && let Some(vdb) = u.def_vdb.get(&v)
            && let Some(exprs) = u.append_expr.get(&v)
            && exprs.len() == 1
            && let Some(source_base) = base_var(&exprs[0], u.get_field)
        {
            // Vars that borrow `v` (their `deps` reference it). After `v` is inlined
            // they borrow the live source element, so each must be re-pointed
            // `v → source_base`. We may only do that — and therefore only elide a
            // borrowed `v` — when every borrower is itself read-only and
            // non-escaping (∉ ineligible); otherwise eliding would route the
            // borrower's write/escape onto the live source, so keep the copy.
            // @PLN25 — deps are a lifetime property, agnostic to the `Optional`
            // nullability marker; peel it (`.base()`) so a nullable element
            // borrower (`e = v[i]` typing `Item?` under the DN1 index flip) is
            // still recognised as borrowing `v`. Without the peel its dep is
            // invisible here, so a mutated/escaping borrower is missed and the
            // copy is wrongly elided (the mutation leaks to the source).
            let borrowers: Vec<u16> = (0..function.next_var())
                .filter(|&e| e != v && function.tp(e).base().depend().contains(&v))
                .collect();
            if borrowers.iter().all(|e| !u.ineligible.contains(e)) {
                plans.push(ElidePlan {
                    var: v,
                    vdb: *vdb,
                    source: exprs[0].clone(),
                    source_base,
                    borrowers,
                });
            }
        }

        rows.push(VerdictRow {
            var_nr: v,
            var_name: function.name(v).to_string(),
            source: src.unwrap_or(u16::MAX),
            verdict,
            reason,
            class,
            loc: None,
            source_last_use: None,
            survival: false,
        });
    }

    // @PLN90 phase 1 — construction / field-append copies. A field-target append
    // (`S { f: src }` construction, or `x.field += src`) deep-copies the source into the
    // field, which the var-buffer idiom above does not classify. Emit a Copy row so the
    // copy-vs-borrow decision COVERS the copy. Diagnostic only: always `Copy`, so it never
    // produces an `ElidePlan` (no codegen change). Phase 2 (the bound-vs-unbound survival
    // split, `survival_class`) sorts Implicit/Avoidable/Forced — gated on `LOFT_COPY_SURVIVAL`
    // (`survival_on`, read once at the top of `analyze_fn`).
    for entry in &u.construct_copy {
        let (rec, src, copy_end, loop_surv) = (entry.0, entry.1, entry.2, entry.3);
        // Flag OFF → the original phase-1 classification verbatim (byte-identical). Flag ON →
        // the bound-vs-unbound survival split.
        let (class, reason) = if survival_on {
            survival_class(src, copy_end, loop_surv, &u, function, data)
        } else {
            (
                CopyClass::Implicit,
                "struct/enum field owns its data (construction/field-append copy)",
            )
        };
        rows.push(VerdictRow {
            var_nr: rec.unwrap_or(u16::MAX),
            var_name: rec.map_or_else(|| "<field>".to_string(), |r| function.name(r).to_string()),
            source: src.unwrap_or(u16::MAX),
            verdict: Verdict::Copy,
            reason,
            class,
            loc: entry.4.clone(),
            source_last_use: src.and_then(|s| u.last_use_loc.get(&s).cloned().flatten()),
            survival: true,
        });
    }

    // @PLN90 phase 1 — record deep-copies (`OpCopyRecord`): a `v[i] = e` element-slot set,
    // a `?? E{…}` default element, a struct copy. Not append-based, so the var-buffer /
    // construction / return-buffer paths above never see them. Emit a Copy row so the
    // decision covers the copy. Diagnostic only — never an `ElidePlan`, no codegen change.
    for entry in &u.record_copy {
        let (tgt, src, copy_end, loop_surv) = (entry.0, entry.1, entry.2, entry.3);
        // Flag OFF → the original phase-1 classification verbatim (byte-identical). Flag ON →
        // the bound-vs-unbound survival split.
        let (class, reason) = if survival_on {
            survival_class(src, copy_end, loop_surv, &u, function, data)
        } else {
            (CopyClass::Implicit, "record deep-copy (OpCopyRecord)")
        };
        rows.push(VerdictRow {
            var_nr: tgt.unwrap_or(u16::MAX),
            var_name: tgt.map_or_else(|| "<record>".to_string(), |t| function.name(t).to_string()),
            source: src.unwrap_or(u16::MAX),
            verdict: Verdict::Copy,
            reason,
            class,
            loc: entry.4.clone(),
            source_last_use: src.and_then(|s| u.last_use_loc.get(&s).cloned().flatten()),
            survival: true,
        });
    }

    // @PLN90 phase B (B1.2) — the MOVE-elision plans: a construction/record copy whose source is a
    // dead-after owned local can transfer its store into the field/element instead of copying.
    // Gated on `LOFT_MOVE_ELIDE`; empty otherwise (default zero-cost + byte-identical). No lowering
    // consumes these yet — B1.2 only computes + dumps them to prove detection.
    let mut move_plans: Vec<MovePlan> = Vec::new();
    if crate::keys::move_elide_enabled() {
        for entry in &u.construct_copy {
            if let Some(s) = move_elidable_source(entry.1, entry.2, entry.3, &u, function) {
                move_plans.push(MovePlan {
                    container: entry.0.unwrap_or(u16::MAX),
                    source: s,
                    kind: MoveKind::Construct,
                    copy_end: entry.2,
                    loc: entry.4.clone(),
                });
            }
        }
        for entry in &u.record_copy {
            if let Some(s) = move_elidable_source(entry.1, entry.2, entry.3, &u, function) {
                move_plans.push(MovePlan {
                    container: entry.0.unwrap_or(u16::MAX),
                    source: s,
                    kind: MoveKind::Record,
                    copy_end: entry.2,
                    loc: entry.4.clone(),
                });
            }
        }
    }
    (rows, plans, move_plans)
}

/// Does duplicating a value of this type allocate?
///
/// A `value struct` (@PLN101) is stored INLINE wherever it lives — a vector element, a record
/// field, a stack slot — so writing one into a container writes its bytes into storage the
/// container already has. Nothing is allocated, and there is no borrow to reach for instead:
/// the destination's storage IS those bytes.
///
/// **Two facts have to meet, and the marker is only the first.** `value struct` says how the
/// value is STORED; it does not say what the value OWNS. A `value struct` may declare a
/// `vector` or a `text` field, and copying that one really does duplicate a store — measured:
/// `a = VH { xs: [1, 2] }; v = [a]; a.xs += [3]` leaves `v[0].xs` at 2 while `a.xs` is 3, which
/// is a deep copy by any reading. So the fields are walked, and any heap-owning one anywhere
/// beneath answers `false`.
///
/// A plain (non-`value`) struct answers `false` even with all-scalar fields: it lives in a
/// record reached by a `DbRef`, so a copy allocates that record. That is the `RS` half of
/// loft#1190, and it keeps its notice.
fn copy_allocates_nothing(data: &Data, tp: &Type) -> bool {
    fn inline_and_free(data: &Data, tp: &Type, seen: &mut Vec<u32>) -> bool {
        match tp.base() {
            Type::Integer(_)
            | Type::Float
            | Type::Single
            | Type::Boolean
            | Type::Character
            | Type::Null => true,
            // A plain enum is a discriminant; a STRUCT-enum (`Type::Enum(_, true, _)`) carries a
            // record payload and is reached by a `DbRef`, so it allocates.
            Type::Enum(_, false, _) => true,
            Type::Reference(d, _) => {
                let d = *d;
                if !data.is_value_struct(d) {
                    return false;
                }
                if seen.contains(&d) {
                    // A value struct cannot contain itself by value, so a cycle here means the
                    // types are still being resolved. Answer the conservative way — a copy that
                    // might allocate keeps its notice.
                    return false;
                }
                seen.push(d);
                let ok = (0..data.attributes(d))
                    .all(|a| inline_and_free(data, &data.attr_type(d, a), seen));
                seen.pop();
                ok
            }
            _ => false,
        }
    }
    matches!(tp.base(), Type::Reference(d, _) if data.is_value_struct(*d))
        && inline_and_free(data, tp, &mut Vec::new())
}

/// @PLN90 — the bound-vs-unbound survival split for a construction / record copy. The
/// silent/indicate line is keyed on the copy's SOURCE FATE, never on the emitting op
/// (COPY_DIAGNOSTICS.md § bound vs unbound):
///
/// - **bound → `Implicit` (silent):** a literal / freshly-built source (nothing pre-existing
///   is duplicated), or a **move** (the source is consumed at the copy — no use strictly
///   after the copy site — so its single backing transfers).
/// - **unbound → indicated:** a still-live source is duplicated into an independent structure.
///   `Avoidable` when the survivor is read-only (a borrow/move would have avoided the copy —
///   the worklist); `Forced` when the source is mutated after the copy (an independent copy is
///   genuinely required). `loop_surv` (item 4): a copy inside a loop whose source is defined
///   OUTSIDE the loop is a duplicate made every iteration → also a survivor (indicated); a
///   per-iteration local source is a move (silent).
///
/// Called only when `LOFT_COPY_SURVIVAL` is set — the flag-OFF path keeps the original
/// phase-1 classification verbatim at the call site, so the default dump stays byte-identical.
fn survival_class(
    src: Option<u16>,
    copy_end: usize,
    loop_surv: bool,
    u: &Uses,
    function: &Function,
    data: &Data,
) -> (CopyClass, &'static str) {
    let Some(s) = src else {
        return (
            CopyClass::Implicit,
            "born-owned: literal / freshly-built source — no live structure duplicated",
        );
    };
    // The copy's OWN read of the source is at a position <= copy_end, so a use strictly after
    // loft#1190 — a source that allocates NOTHING when duplicated is `Implicit` whatever its
    // fate.  The `Avoidable` class is the borrow worklist, and there is no borrow to reach for
    // here: an inline value's storage IS the destination's bytes, so `[a, b]` writes them where
    // they belong and a move would save nothing.  Advice that names a rewrite buying zero is
    // what teaches a reader to stop reading the channel.
    if copy_allocates_nothing(data, function.tp(s)) {
        return (
            CopyClass::Implicit,
            "inline value with no heap part — the copy allocates nothing and no borrow could avoid it",
        );
    }
    // is a genuine later use — the source survives, an independent duplicate now coexists.
    let survives_straight = u.last_use_pos.get(&s).is_some_and(|&p| p > copy_end);
    if !survives_straight && !loop_surv {
        // A move — the source is consumed here (a per-iteration local, in the loop case). Bound,
        // silent for everyone; not a copy to eliminate.
        return (
            CopyClass::Implicit,
            "move: source consumed at the copy — its single backing transfers",
        );
    }
    let (class, reason) = if u.mut_max_pos.get(&s).is_some_and(|&p| p > copy_end) {
        // @PLN90 item 3 — the source is genuinely WRITTEN after the copy (reassigned, appended
        // to, record-copied into): a borrow would let that mutation leak into the copy's owner,
        // so the independent copy is forced. This is precise — unlike the old `other_max_pos`
        // test it does NOT count a read-only pass-to-callee or being another copy's source.
        // (Residual, conservative-toward-Avoidable: a mutating-callee on a single-def LOCAL is
        // not caught → shows Avoidable; the phase-B elision analysis is the real borrow checker
        // and rejects an unsound candidate, so a false-Avoidable is safe for the worklist.)
        (
            CopyClass::Forced,
            "unbound: source survives AND is written after — an independent copy is required",
        )
    } else if loop_surv && !survives_straight {
        // @PLN90 item 4 — a value defined outside the loop, copied every iteration.
        (
            CopyClass::Avoidable,
            "in-loop copy of a value defined outside the loop — duplicated every iteration; a borrow would avoid it",
        )
    } else {
        // A read-only survivor (read, or passed read-only) — the avoidable worklist.
        (
            CopyClass::Avoidable,
            "unbound: source survives read-only — a borrow/move would avoid this copy",
        )
    };
    // @PLN90 item 1 — an INDICATED copy whose SOURCE is a compiler-generated temporary
    // (`_`-prefixed) is not user-actionable: the user never wrote `__ref_N` / `_comp_N`. Route
    // it to `Internal` so the user-facing report excludes it while the developer worklist still
    // counts it (it may be a copy WE can eliminate).
    if function.is_compiler_generated(s) {
        return (
            CopyClass::Internal,
            "compiler-internal source (_-prefixed) — a copy we may eliminate; excluded from the user report",
        );
    }
    (class, reason)
}

/// @PLN90 phase B (B1.2) — is this construction/record copy site a MOVE-elidable one, and if so,
/// which source var transfers? A site is move-elidable iff the source is the phase-A survival
/// **move** (consumed at the copy: not straight-line-surviving, not loop-repeated) AND it meets the
/// elision preconditions: it is a **local** (not a parameter — the caller owns a param's store) that
/// **owns a transferable store** (a vdb-buffered vector `def_vdb`, or a fresh `OpDatabase`'d record
/// `database_vars` — never a view/projection, which owns no store to move). Conservative by design:
/// a false negative just keeps the copy; a false positive would be an unsound move, so the checks
/// only widen with proof. Returns `Some(source)` when elidable.
fn move_elidable_source(
    src: Option<u16>,
    copy_end: usize,
    loop_surv: bool,
    u: &Uses,
    function: &Function,
) -> Option<u16> {
    let s = src?; // a literal source has nothing pre-existing to move
    if function.is_argument(s) {
        return None; // a parameter's store belongs to the caller — not ours to transfer
    }
    // Survival MOVE only: a source that survives (read after) or is copied every loop iteration
    // must keep its COPY (C86) — never move.
    let survives = u.last_use_pos.get(&s).is_some_and(|&p| p > copy_end);
    if survives || loop_surv {
        return None;
    }
    // Owns a transferable store.
    if u.def_vdb.contains_key(&s) || u.database_vars.contains(&s) {
        Some(s)
    } else {
        None
    }
}

/// @PLN90 phase B (B1.2) — the move-elidable construction/record copies of function `d_nr` (a
/// dead-after owned-local source whose store can transfer into the field/element). Empty unless
/// `LOFT_MOVE_ELIDE` is set. No lowering consumes these yet; this is the detection the B1.3+
/// lowering will read. See [`MovePlan`] and `phase-b-design.md`.
#[must_use]
pub fn move_plans(data: &Data, d_nr: u32) -> Vec<MovePlan> {
    let def = data.def(d_nr);
    analyze_fn(&def.code, &def.variables, data, env_tier()).2
}

/// @PLN90 phase B — dump every move-elidable site when `LOFT_MOVE_ELIDE` is set (the opt-in
/// diagnostic; the rewrite itself is default-on). A no-op otherwise, so the default-on elision does
/// not spew plan lines. The dump is the positive control that detection fires on the dead-source
/// shapes and stays silent on survivors.
pub fn dump_move_plans(data: &Data) {
    if !crate::keys::move_elide_dump_enabled() {
        return;
    }
    let mut total = 0u32;
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            continue;
        }
        for p in analyze_fn(&def.code, &def.variables, data, env_tier()).2 {
            total += 1;
            let at = p
                .loc
                .as_ref()
                .map_or_else(String::new, |l| format!(" at {l}"));
            let container = if p.container == u16::MAX {
                "<slot>".to_string()
            } else {
                def.variables.name(p.container).to_string()
            };
            eprintln!(
                "MOVE-PLAN fn={} kind={:?} container={} source={}{}",
                def.name,
                p.kind,
                container,
                def.variables.name(p.source),
                at
            );
        }
    }
    eprintln!("MOVE-PLAN total={total}");
}

/// The elision tier selected by the environment: 0 (shipped param-source rule,
/// the default) unless `LOFT_ELIDE_T1` is set, which adds the Tier-1 read-only-
/// local-source verdicts. Higher tiers can attach more flags here as they land.
/// Kept here so every consumer (the rewrite, the dump, the tests' default) reads
/// one source of truth.
#[must_use]
pub fn env_tier() -> u8 {
    // Additive flags — each enabled tier raises the ceiling. Later tiers attach
    // their own flag here (e.g. `LOFT_ELIDE_T2` -> `tier = tier.max(2)`).
    let mut tier = 0;
    if std::env::var_os("LOFT_ELIDE_T1").is_some() {
        tier = tier.max(1);
    }
    tier
}

/// Public, test-facing entry: the verdicts for one function by its def number, at
/// the env-selected tier (Tier 0 unless `LOFT_ELIDE_T1` is set).
#[must_use]
pub fn verdicts_for(data: &Data, d_nr: u32) -> Vec<VerdictRow> {
    verdicts_for_tier(data, d_nr, env_tier())
}

/// Public, test-facing entry: the verdicts for one function at an EXPLICIT tier —
/// lets a test exercise a tier's logic regardless of the environment, so the
/// not-yet-wired tiers stay evaluable in isolation.
#[must_use]
pub fn verdicts_for_tier(data: &Data, d_nr: u32, max_tier: u8) -> Vec<VerdictRow> {
    let def = data.def(d_nr);
    analyze_fn(&def.code, &def.variables, data, max_tier).0
}

/// The elision plans (Borrow verdicts) for one function — what the borrow rewrite
/// consumes — at the env-selected tier.
#[must_use]
pub fn elision_plans(code: &Value, function: &Function, data: &Data) -> Vec<ElidePlan> {
    analyze_fn(code, function, data, env_tier()).1
}

// ============================================================================
// Ownership classification (Owned | Borrowed | Join) — the @PLN85 over-free fact.
//
// The over-free class (a borrowed view escapes into an owned position — return,
// reassign, append — and a free site frees its store while the view is still
// live) is NOT a per-site bug; it is one missing fact RE-DERIVED at ~16 sites.
// The fact: for any value that escapes into an owned position, is its store
//   * Owned     — a freshly minted store the producer owns (free is correct), or
//   * Borrowed  — a view into a store owned elsewhere (must NOT free here), or
//   * Join      — owned on one runtime branch, borrowed on the other (the
//                 `v[i] ?? default` shape) — the decision is runtime-dependent,
//                 so the escape must MATERIALISE the borrow branch to owned.
//
// Crucially the JOIN is the part the flattened return-dep facts
// (`return_adopts_fresh_store` / `returns_borrowed_view`) LOSE: a `fn pick(t,i)
// -> M { t[i] ?? m_none() }` flattens to "borrowed view of t", hiding that the
// `m_none()` arm is owned. So this classifier walks the return EXPRESSION (which
// recovers the `??`/`if-else` join) rather than reading the collapsed dep.
//
// This classification is consumed by DEFAULT-ON codegen: `ownership_of` (below) is
// the one fact every own-vs-borrow site reads instead of re-deriving — interp
// `state/codegen.rs`, native `generation/dispatch.rs`, gated `join_own_enabled`. The
// `ownership_*` tests + the @PLN89 differential oracle validate it. Design study:
// `doc/claude/plans/85-store-lifetime-retirement/over-free-class-study.md`
// (§ Three chokepoints).
//
// A var comes to hold a value TWO ways, and both count (loft#704): a `Set` re-binds
// it, and an append/clear FILLS it in place. The fill delivers the var's own buffer
// whatever the source was — the append deep-copies — so it is an Owned alternative,
// and a scan that saw only `Set` read `match e { Filled { items } => { items },
// _ => { [] } }` as a plain Borrowed-of-`e`: the split it exists to name was invisible
// because the `[]` arm defines nothing. See `Defs::filled`.
//
// APPROXIMATIONS (sound by conservatism — a value can only OVER-report Join/Borrowed):
//   * Var resolution is flow-INSENSITIVE — a var classifies as the join of ALL
//     its real (non-`= null`-init) defs across the body, not the def that reaches
//     the point of use. For the over-free shapes (single-def views, owned-then-
//     reassigned slots) this gives the right answer; a genuinely path-split var
//     can only over-report Join, never wrongly report Owned.
//   * A bare PARAMETER used as a value classifies as `Borrowed` (the caller owns
//     it) — including a retbuf param a callee fills in place, which is really
//     owned. Conservative: it can lose an Owned, never invent one. This is why the
//     fill above is an ALTERNATIVE joined with a var's other defs and never the whole
//     answer: a buffer that is only ever filled keeps this reading, because claiming
//     `Owned` for it would license a callee to free the CALLER's buffer.
// ============================================================================

/// The store-ownership of a value at an own-vs-borrow decision site — THE one fact
/// every such site READS instead of re-deriving (the OWNERSHIP_MODEL north star).
/// `Borrowed`/`Join` carry the `base`: the caller-visible var whose store the value
/// aliases — the witness the `Join` runtime guard needs and what distinguishes a
/// borrowed-of-arg value from an owned one. `base` is `u16::MAX` when unresolvable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Own {
    /// A freshly minted store the producer owns (an `OpDatabase`/`OpNewRecord`
    /// buffer, a struct literal, a call whose return adopts a fresh store). Free /
    /// adopt / set the source-free bit is correct.
    Owned,
    /// A view into `base`'s store (a vector-element / field projection, a returned
    /// parameter, or a value that resolves to one). Never free it; to land in an
    /// owned slot, deep-copy.
    Borrowed { base: u16 },
    /// Runtime-dependent: owned on one branch, borrowed-of-`base` on the other (a
    /// `??` / `if-else` whose arms split). The decision is per-execution — adopt iff
    /// the value's store ≠ `base`'s store (the owned branch ran), else materialise.
    Join { base: u16 },
}

impl Own {
    /// The base var a `Borrowed`/`Join` value aliases, or `None` for `Owned`.
    #[must_use]
    fn base(self) -> Option<u16> {
        match self {
            Own::Owned => None,
            Own::Borrowed { base } | Own::Join { base } => Some(base),
        }
    }

    /// The lattice join of two `??`/`if` arms. Two equal borrows of the SAME base
    /// stay `Borrowed`; any owned-vs-borrowed split (or differing bases) becomes a
    /// `Join` witnessed by whichever arm carries a base.
    #[must_use]
    fn join(self, other: Own) -> Own {
        match (self, other) {
            (Own::Owned, Own::Owned) => Own::Owned,
            (Own::Borrowed { base: a }, Own::Borrowed { base: b }) if a == b => {
                Own::Borrowed { base: a }
            }
            _ => Own::Join {
                base: self.base().or_else(|| other.base()).unwrap_or(u16::MAX),
            },
        }
    }
}

/// One owned-slot reassignment (`v = X` where `v` already held a value): the class
/// of the value `v` held BEFORE this assignment and of the new RHS. A `prior =
/// Owned`, `rhs = Join`/`Borrowed` row is the over-free leak shape — the displaced
/// owned store must be freed before `v` takes the borrow (the `local_source` root).
#[derive(Clone, Debug)]
pub struct ReassignSite {
    pub var: u16,
    pub var_name: String,
    pub prior: Own,
    pub rhs: Own,
}

/// The kind of free site the over-free fix acts at — the Gap-A sites the value
/// classification alone does not surface (see `ownership-analysis-gaps.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreeKind {
    /// `OpCopyRecord(src, _, tp)` with the `0x8000` source-free bit — frees `src`
    /// after copying it into a vector element (the `out += [src]` append idiom,
    /// the `elem_accumulate` chokepoint). A `Borrowed`/`Join` `src` is over-freed.
    AppendSource,
    /// A return-delivery buffer (a heap PARAMETER the fn delivers its result
    /// through) reassigned to a `Borrowed`/`Join` value — the buffer then aliases a
    /// store it does not own, so freeing it over-frees that store (the
    /// `match_return` chokepoint: `_mv_items_1 = OpGetField(e, …)`).
    ParamDeliver,
}

/// A site where the over-free fix must read the carried ownership instead of
/// blindly freeing. Carries the value's class AND (Gap B) the borrow base to
/// materialise from when the borrow is a direct projection in this function.
#[derive(Clone, Debug)]
pub struct FreeSite {
    pub kind: FreeKind,
    /// The freed source var (`AppendSource`) or the buffer param (`ParamDeliver`);
    /// `u16::MAX` when the source is not a plain var.
    pub slot: u16,
    pub slot_name: String,
    /// Ownership of the value the site frees/delivers.
    pub class: Own,
    /// The base var to materialise from when the borrow arm is a DIRECT projection
    /// in this fn (`None` when the borrow comes from a call — materialise = deep
    /// copy the whole returned value — or the value is `Owned`).
    pub base: Option<u16>,
    pub base_name: Option<String>,
}

/// The recursive ownership classifier over the post-lowering `Value` IR. Holds the
/// op-def numbers it keys on and a memoised, recursion-guarded per-function return
/// classification (so an interprocedural `pick(t,i)` call resolves to `pick`'s
/// return class, recovering the `??` join the flattened return dep loses).
struct Ownership<'a> {
    data: &'a Data,
    op_database: u32,
    op_new_record: u32,
    op_copy_record: u32,
    projections: std::sync::Arc<HashSet<u32>>,
    ret_memo: HashMap<u32, Own>,
    visiting: HashSet<u32>,
    /// Vars currently being classified — the var-level twin of [`Self::visiting`].
    /// A self-referential default (`c = t[k] ?? c`) makes a var's RHS mention the var
    /// itself, so `classify` recursed forever and overflowed the stack: a SIGSEGV
    /// during COMPILATION, reproducible with `--check` on two lines of valid-looking
    /// source (crawler LOFT-HANDOFF H1).
    visiting_vars: HashSet<u16>,
}

/// The tail (value) expression of a function body, or `None` for a native/`#rust`
/// definition (no loft `Block` body — its flattened return dep is then exact).
fn fn_body_tail(code: &Value) -> Option<&Value> {
    match code.unspan() {
        Value::Block(b) => b.operators.last(),
        _ => None,
    }
}

/// Which definition does a fn-ref variable hold, read off ONE of its defining
/// right-hand sides?
///
/// A fn-ref reaches its variable three ways and only one of them is a bare marker: an
/// explicit `FnRef` / `FnRefDnr` names the target — a CAPTURING lambda assigns a BLOCK
/// (build the closure record, then the ref), so the marker is the block's TAIL — while a
/// NON-capturing lambda is stored as the bare definition number.
///
/// The marker is read from what the right-hand side YIELDS ([`collect_yielded`]), never
/// from the whole tree: the capturing block WRITES each capture into the record before it
/// yields the ref, so a capture that is itself a fn-ref appears there as a second
/// definition number and made every such variable read as naming TWO targets (loft#1329).
/// The same reading covers the bare-integer case, where a capturing block is full of
/// unrelated ints (a type id, a field offset) that are not candidates either.
///
/// `None` = this right-hand side names no target at all.  `Some(u32::MAX)` = it names
/// TWO, which is a different answer and must stay one: a var whose FIRST definition
/// resolves cleanly and whose second is ambiguous has no single callee, and collapsing
/// "ambiguous" into "absent" would let the first definition's answer stand for both.
///
/// One home, because two readers resolve the same question —
/// `scopes::collect_fnref_targets` (whole function, walking every `Set`) and
/// [`Ownership::classify`]'s `CallRef` arm (one var, off the `Defs` table it already
/// built) — and a fn-ref they disagreed about would be lifted by one and adopted by the
/// other.
pub(crate) fn fnref_target_in(rhs: &Value) -> Option<u32> {
    let mut yielded: Vec<&Value> = Vec::new();
    collect_yielded(rhs, &mut yielded);
    let mut found: Option<u32> = None;
    let mut ambiguous = false;
    for inner in &yielded {
        // `collect_yielded` already hands back unspanned nodes, and this peels anyway:
        // `Value::unspan`'s contract is that a site discriminating on specific variants
        // calls it, and a site that is correct only because of what its one caller does
        // is one refactor away from being wrong.
        let d = match inner.unspan() {
            Value::FnRef(d, _, _) => u32::try_from(*d).ok(),
            Value::FnRefDnr(d) => Some(u32::from(*d)),
            _ => None,
        };
        if let Some(d) = d {
            match found {
                Some(prev) if prev != d => ambiguous = true,
                _ => found = Some(d),
            }
        }
    }
    if found.is_none() {
        for inner in &yielded {
            if let Value::Int(d) = inner.unspan() {
                found = u32::try_from(*d).ok();
                break;
            }
        }
    }
    if ambiguous { Some(u32::MAX) } else { found }
}

/// The sub-values a right-hand side can EVALUATE TO — one per path it may take.
///
/// [`fnref_target_in`] reads its markers from here rather than from the whole tree, and the
/// difference is the whole of what a capturing lambda's assignment looks like: it is a BLOCK
/// that mints the closure record, WRITES each capture into it, and then yields the `FnRef`.
/// A capture that is itself a fn-ref is written as an `FnRefDnr` argument of that write, so a
/// tree walk sees a second definition number and reports the variable as naming TWO targets —
/// the answer reserved for a slot two different lambdas were assigned to.  A capture is a
/// payload, not a candidate: only what the right-hand side yields names the target.
///
/// Every branch is yielded, so a fn-ref genuinely chosen between two lambdas
/// (`f = if c { a } else { b }`) still reports the ambiguity that reading is for.
fn collect_yielded<'a>(rhs: &'a Value, out: &mut Vec<&'a Value>) {
    let tail = |ops: &'a [Value]| {
        ops.iter()
            .rev()
            .find(|o| !matches!(o.unspan(), Value::Line(_)))
    };
    match rhs.unspan() {
        Value::Block(bl) => {
            if let Some(last) = tail(&bl.operators) {
                collect_yielded(last, out);
            }
        }
        Value::Insert(ops) => {
            if let Some(last) = tail(ops) {
                collect_yielded(last, out);
            }
        }
        Value::If(_, then, alt) => {
            collect_yielded(then, out);
            collect_yielded(alt, out);
        }
        Value::Return(inner) | Value::Drop(inner) => collect_yielded(inner, out),
        other => out.push(other),
    }
}

/// The target every one of a fn-ref variable's definitions agrees on, or `None`.
///
/// Disagreement is `None` on purpose: a slot two lambdas were assigned to has no single
/// callee, and the ownership of what it returns is then not a static fact.  Callers read
/// `None` as "unresolved" and keep their pre-existing conservative behaviour.
pub(crate) fn fnref_target_of(rhss: &[Value]) -> Option<u32> {
    let mut agreed: Option<u32> = None;
    for r in rhss {
        match (agreed, fnref_target_in(r)) {
            (_, None) => {}
            (None, Some(d)) => agreed = Some(d),
            (Some(prev), Some(d)) if prev == d => {}
            (Some(_), Some(_)) => return None,
        }
    }
    agreed.filter(|d| *d != u32::MAX)
}

/// A function's def facts: every real definition `v = rhs` (in source order,
/// skipping `v = null` declaration sentinels), the vars `OpDatabase` mints a
/// fresh store into (which are Owned even with no `Set`-def — e.g. a retbuf param
/// a `materialized_view_return` fills in place), and the vars some branch FILLS
/// IN PLACE (loft#704).
#[derive(Default)]
pub(crate) struct Defs {
    rhs: HashMap<u16, Vec<Value>>,
    db_vars: HashSet<u16>,
    /// loft#704 — vars a branch fills IN PLACE (`OpClearVector(v)` /
    /// `OpAppendVector(v, …)`) rather than re-binding with a `Set`.
    ///
    /// A var comes to hold a value two ways, and only one of them is a `Set`.  The
    /// append DEEP-COPIES into `v`'s existing store, so a filled branch delivers `v`'s
    /// OWN buffer whatever the source was — an Owned alternative that a `Set`-only scan
    /// cannot see.  `match e { Filled { items } => { items }, _ => { [] } }` has exactly
    /// one of each, and read as a plain Borrowed-of-`e`: the empty arm lowers to
    /// `OpClearVector(retbuf); OpAppendVector(retbuf, …)`, which defines nothing.
    filled: HashSet<u16>,
    /// The DEFINITION each fn-ref variable in this body was assigned — what lets
    /// [`Ownership::classify`] resolve a `CallRef` through its callee's return summary,
    /// exactly as it resolves a `Call` through the definition the node names.
    ///
    /// A `CallRef` names a runtime VALUE, not a definition, so the target has to be
    /// recovered from the assignment that put the closure in the variable.  Shared with
    /// the scope pass rather than re-derived (`scopes::collect_fnref_targets`).
    fnref_targets: HashMap<u16, u32>,
    /// The CALLER variables each fn-ref's closure record holds, in capture-slot order — what
    /// lets a return that borrows the hidden `__closure` attribute name a witness at all.
    /// Per fn-ref variable, the closure record's captures as `(field offset, caller var)`,
    /// in offset order — what the closure BUILD's `OpSetDbRef(___clos_N, off, var)` wrote.
    fnref_captures: HashMap<u16, Vec<(i32, u16)>>,
    /// Caller variables assigned at more than one site.  A closure captures the store its
    /// variable held at BUILD time (`L-CapHeap`), so a variable reassigned afterwards no longer
    /// names what the closure hands back and cannot witness for it.
    multi_assigned: HashSet<u16>,
}

/// The ops that establish a var's CONTENTS without re-binding it — see [`Defs::filled`].
struct FillOps {
    database: u32,
    clear_vector: u32,
    append_vector: u32,
}

impl FillOps {
    fn of(data: &Data) -> Self {
        Self {
            database: data.def_nr("OpDatabase"),
            clear_vector: data.def_nr("OpClearVector"),
            append_vector: data.def_nr("OpAppendVector"),
        }
    }
}

/// Every variable `node` MINTS a fresh store into (an `OpDatabase` destination).
///
/// The ownership marker in dep form: `[]` lowers to `OpDatabase(__vdb_N, …)` and the
/// value then types as a dep on `__vdb_N`, which says *I own this store* — the opposite
/// of the borrow a dep normally records. A branch join has to tell the two apart before
/// it can union its arms' deps (loft#978, `Parser::arm_join_type`), and this answers it
/// from the SAME walk the ownership classifier reads, so the two cannot drift.
#[must_use]
pub(crate) fn minted_vars(data: &Data, node: &Value) -> HashSet<u16> {
    let mut out = Defs::default();
    collect_defs(node, &FillOps::of(data), &mut out);
    out.db_vars
}

fn collect_defs(node: &Value, ops: &FillOps, out: &mut Defs) {
    match node.unspan() {
        Value::Set(v, rhs) => {
            if !matches!(rhs.unspan(), Value::Null) {
                out.rhs.entry(*v).or_default().push(rhs.unspan().clone());
            }
            collect_defs(rhs, ops, out);
        }
        Value::Call(d, args) if *d == ops.database => {
            if let Some(Value::Var(v)) = args.first().map(Value::unspan) {
                out.db_vars.insert(*v);
            }
            for a in args {
                collect_defs(a, ops, out);
            }
        }
        Value::Call(d, args) if *d == ops.clear_vector || *d == ops.append_vector => {
            if let Some(Value::Var(v)) = args.first().map(Value::unspan) {
                out.filled.insert(*v);
            }
            for a in args {
                collect_defs(a, ops, out);
            }
        }
        other => other.for_each_child(&mut |c| collect_defs(c, ops, out)),
    }
}

/// Collect the over-free candidate sites in `node` (recursively):
/// - `appends`: the `src` of each `OpCopyRecord(src, _, tp)` whose `tp` carries the
///   `0x8000` source-free bit (the `out += [src]` element-append free).
/// - `delivers`: each `(param, rhs)` of a `Set` to a HEAP parameter (a
///   return-delivery buffer reassigned — the `match_return` aliasing site).
fn collect_free_candidates(
    node: &Value,
    op_copy_record: u32,
    func: &Function,
    appends: &mut Vec<Value>,
    delivers: &mut Vec<(u16, Value)>,
) {
    match node.unspan() {
        Value::Call(d, args) if *d == op_copy_record => {
            if args.len() >= 3
                && let Value::Int(tp) = args[2].unspan()
                && tp & 0x8000 != 0
            {
                appends.push(args[0].unspan().clone());
            }
            for a in args {
                collect_free_candidates(a, op_copy_record, func, appends, delivers);
            }
        }
        Value::Set(v, rhs) => {
            if func.is_argument(*v)
                && func.tp(*v).heap_dep().is_some()
                && !matches!(rhs.unspan(), Value::Null)
            {
                delivers.push((*v, rhs.unspan().clone()));
            }
            collect_free_candidates(rhs, op_copy_record, func, appends, delivers);
        }
        other => other.for_each_child(&mut |c| {
            collect_free_candidates(c, op_copy_record, func, appends, delivers)
        }),
    }
}

impl<'a> Ownership<'a> {
    fn new(data: &'a Data) -> Self {
        Ownership {
            data,
            op_database: data.def_nr("OpDatabase"),
            op_new_record: data.def_nr("OpNewRecord"),
            op_copy_record: data.def_nr("OpCopyRecord"),
            projections: std::sync::Arc::clone(&data.op_sets().projections),
            ret_memo: HashMap::new(),
            visiting: HashSet::new(),
            visiting_vars: HashSet::new(),
        }
    }

    /// The ownership class of the value `d_nr` returns. For a loft-body function it
    /// classifies the return EXPRESSION (recovering a `??` join); for a native
    /// definition it falls back to the flattened canonical fact.
    fn return_ownership(&mut self, d_nr: u32) -> Own {
        if let Some(&c) = self.ret_memo.get(&d_nr) {
            return c;
        }
        let def = self.data.def(d_nr);
        let tail = fn_body_tail(&def.code);
        // No loft body (native op / `#rust` stdlib): no intraprocedural join to
        // recover — the flattened canonical fact is exact. Its base is the first
        // VISIBLE param the return dep names (in this callee's own var space).
        if tail.is_none() || !matches!(def.def_type, DefType::Function) {
            let attrs = def.attributes();
            let c = if def.returns_borrowed_view() {
                let base = def
                    .returned()
                    .depend()
                    .iter()
                    .find(|&&a| (a as usize) < attrs.len() && !attrs[a as usize].hidden)
                    .map_or(u16::MAX, |&a| a);
                Own::Borrowed { base }
            } else {
                Own::Owned
            };
            self.ret_memo.insert(d_nr, c);
            return c;
        }
        if !self.visiting.insert(d_nr) {
            // Recursion back-edge: conservatively Borrowed (never assume a self-
            // referential return is freshly owned), base unresolved. Not memoised —
            // the enclosing frame computes and caches the real class.
            return Own::Borrowed { base: u16::MAX };
        }
        let mut defs = Defs::default();
        collect_defs(&def.code, &FillOps::of(self.data), &mut defs);
        // @FR-O-Oracle — the answer must be a function of the VALUE, never of who asked.
        // The in-flight var set belongs to the function whose body is being walked: a slot
        // number names a variable within ONE function's variable space, so the caller's set
        // says nothing here and must not cross the boundary.  A caller's `__ncc_3` and a
        // callee's `__ret_1` are both var 3, and a set that travels reads the callee's own
        // temp as self-referential — `Borrowed { base: MAX }` for an arm that borrows
        // nothing, which every witness-gated free downstream then declines on (loft#1119).
        // The FUNCTION-level guard above is the one that stops genuine recursion; this
        // scoping does not weaken it.
        let outer_vars = std::mem::take(&mut self.visiting_vars);
        let class = self.classify(tail.unwrap(), &def.variables, &defs);
        self.visiting_vars = outer_vars;
        self.visiting.remove(&d_nr);
        self.ret_memo.insert(d_nr, class);
        class
    }

    /// Every EARLY `return <e>` in the body of `d_nr`, classified the way
    /// [`Self::return_ownership`] classifies the tail.
    ///
    /// The tail is one delivery site among several: a function returns from wherever a
    /// `return` stands, and each site hands the caller a value of its own ownership.  A
    /// predicate that reads only the tail (`fn f(c) -> text { if c { return mk() } "x" }`)
    /// answers for the literal and never sees the owned call — which is how such a
    /// function stayed unbuffered and orphaned one String per early return (loft#1338).
    /// Not memoised: the tail's class is what callers consult and cache; this is asked
    /// once, by the orphan predicate, for the function's own delivery.
    fn early_return_ownerships(&mut self, d_nr: u32) -> Vec<Own> {
        let def = self.data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) || fn_body_tail(&def.code).is_none() {
            return Vec::new();
        }
        // A null arm returns a SENTINEL, not a buffer: `return null` in a `-> text?`
        // function lowers to `OpConvTextFromNull()`, a constant Str with nothing behind
        // it to orphan, and reading it as owned text would hand a buffer to a function
        // whose real tail forwards a borrow (`text_src(i, tag) { if i == 0 { return
        // null } return tag }`), which is the promotion the framework's own verdict
        // declines.
        let null_text = self.data.def_nr("OpConvTextFromNull");
        let mut returned: Vec<Value> = Vec::new();
        def.code.walk(&mut |v| {
            if let Value::Return(inner) = v
                && !matches!(inner.unspan(), Value::Null)
                && !matches!(inner.unspan(), Value::Call(d, args) if *d == null_text && args.is_empty())
            {
                returned.push((**inner).clone());
            }
        });
        if returned.is_empty() || !self.visiting.insert(d_nr) {
            return Vec::new();
        }
        let mut defs = Defs::default();
        collect_defs(&def.code, &FillOps::of(self.data), &mut defs);
        let outer_vars = std::mem::take(&mut self.visiting_vars);
        let classes = returned
            .iter()
            .map(|e| {
                self.visiting_vars.clear();
                self.classify(e, &def.variables, &defs)
            })
            .collect();
        self.visiting_vars = outer_vars;
        self.visiting.remove(&d_nr);
        classes
    }

    /// Classify a value expression within `func` (using `defs` to resolve local
    /// vars to their defining RHS). The recursive core of the analysis.
    fn classify(&mut self, node: &Value, func: &Function, defs: &Defs) -> Own {
        match node.unspan() {
            Value::Var(v) if !self.visiting_vars.insert(*v) => {
                // Recursion back-edge: this var appears in its OWN definition, as in
                // `c = t[k] ?? c`.  Mirrors the function-level guard below — return
                // conservatively rather than recursing.  `Borrowed { base: MAX }`
                // (never "freshly owned") joins with the real arm to `Join`, the
                // owned-vs-borrowed split the reassign check already treats as the
                // risky shape; the alternative was an unbounded recursion that took
                // the compiler down with it.
                Own::Borrowed { base: u16::MAX }
            }
            Value::Var(v) => {
                // A var `OpDatabase` MINTED a store into owns that store — and it owns
                // exactly that.  Where a `Set` also defines it, the var's class is the JOIN
                // of the mint with those definitions, in which a bare-`Var` right-hand side
                // is a COPY into the minted store (@FR-B-Copy: a whole-value bind is
                // independent of its source, and the mint IS the copy's store) and so
                // Owned, while a call or a projection is whatever the oracle says of it.
                // Reading the mint ALONE — "Owned regardless of any other def" — called a
                // local minted once and then rebound to a call that may hand back its own
                // argument (`c = M {…}; c = cond(c, 3)`) or to a capture read `Owned`, the
                // one verdict that licenses a free, where the flow-sensitive shadow read
                // `Join`/`Borrowed`; Check A's fact-disagree on 1017b/1326/1331 was that
                // shortcut, not the shadow (@FR-O-Oracle, QUALITY.md B7r).
                let minted = defs.db_vars.contains(v);
                // A definition that binds NO STORE — `x = null` — says nothing about what
                // `x` may own on its other definitions and is left out of the join, exactly
                // as a null arm is in the `If` arm below; a var with only such definitions
                // owns nothing anyone could mis-free.
                let class = match defs.rhs.get(v) {
                    Some(rhss) if rhss.iter().any(|r| !holds_no_store(self.data, r)) => {
                        let bound = rhss
                            .iter()
                            .filter(|r| !holds_no_store(self.data, r))
                            .map(|r| {
                                if minted && matches!(r.unspan(), Value::Var(_)) {
                                    Own::Owned
                                } else {
                                    self.classify(r, func, defs)
                                }
                            })
                            .reduce(Own::join)
                            .unwrap_or(Own::Owned);
                        // loft#704 — a branch that FILLS `v` in place rather than
                        // re-binding it delivers `v`'s own buffer (the append deep-
                        // copies), so it is an Owned alternative to the bound ones.
                        // Without it a `match` whose borrowed arm re-binds and whose
                        // `[]` arm fills read as a plain Borrowed, losing the very
                        // split `Join` exists to name.
                        //
                        // Only where there IS something to join with.  A var with no
                        // `Set` at all is the retbuf-param shape the approximation
                        // above deliberately calls `Borrowed`: it is really owned, but
                        // saying so would tell a callee it may free the CALLER's
                        // buffer.  This adds an alternative; it never replaces that.
                        if defs.filled.contains(v) || minted {
                            bound.join(Own::Owned)
                        } else {
                            bound
                        }
                    }
                    // No local def: a store the function minted in place with no `Set` at
                    // all (the retbuf a `materialized_view_return` fills — a parameter, but
                    // this function's own store), else a parameter (the caller owns it ⇒
                    // Borrowed of itself) or an uninitialised local (Owned — nothing to
                    // mis-free).
                    _ => {
                        if minted {
                            Own::Owned
                        } else if func.is_argument(*v) {
                            Own::Borrowed { base: *v }
                        } else {
                            Own::Owned
                        }
                    }
                };
                self.visiting_vars.remove(v);
                class
            }
            Value::Call(d, args) => {
                if *d == self.op_database || *d == self.op_new_record {
                    Own::Owned
                } else if self.projections.contains(d) {
                    // A projection (`OpGetField`/`OpGetVector*`/`OpGetDbRef`) is a
                    // view into its base container (arg 0), rooted at a var.
                    match self.borrow_base(node, func, defs) {
                        Some(base) => Own::Borrowed { base },
                        None => Own::Owned,
                    }
                } else {
                    self.call_ownership(*d, args, func, defs)
                }
            }
            // `??` / `if-else` lowers to `If`: the join of its two arms.  An arm that holds
            // NO STORE — a `null`, the reference null sentinel — is the join's identity, not
            // an owned value: `if <present> { <projection> } else { null }` (the tag read of
            // a nullable slot, `Parser::emit_nullable_slot_read`) views exactly what its
            // present arm views, and calling the pair a `Join` made a rebind release the
            // container's store as if the local had owned it.
            Value::If(_, then, els) => {
                let seen = through_null_arm(self.data, node);
                if matches!(seen.unspan(), Value::If(_, _, _)) {
                    self.classify(then, func, defs)
                        .join(self.classify(els, func, defs))
                } else {
                    self.classify(seen, func, defs)
                }
            }
            // A block's value is its tail; passthrough wrappers forward.
            Value::Block(b) => b
                .operators
                .last()
                .map_or(Own::Owned, |t| self.classify(t, func, defs)),
            Value::Insert(ops) => ops
                .last()
                .map_or(Own::Owned, |t| self.classify(t, func, defs)),
            Value::Return(v) => self.classify(v, func, defs),
            // `@FR-B-View` — a heap member read out of a tuple (`a = t.0`) is a VIEW of the
            // tuple's member, exactly as a struct field read is; a scalar member is a value.
            // A read off a work temp holding a bare copy of another tuple local (the
            // destructure's `T1.4` temp) views THAT local's member: the temp copies the
            // tuple's words, and a heap member's word is the handle.  Without this arm the
            // read fell to the fallback and was called `Owned`, so the dead-store lint
            // warned that a write through the view was lost — while both backends landed it.
            Value::TupleGet(base, i) => {
                let heap = matches!(func.tp(*base), crate::data::Type::Tuple(elems)
                    if elems.get(*i as usize).is_some_and(crate::data::is_dbref));
                if !heap {
                    return Own::Owned;
                }
                let mut b = *base;
                if let Some(rhss) = defs.rhs.get(&b)
                    && let [only] = rhss.as_slice()
                    && let Value::Var(x) = only.unspan()
                    && matches!(func.tp(*x), crate::data::Type::Tuple(_))
                {
                    b = *x;
                }
                Own::Borrowed { base: b }
            }
            // @FR-O-Oracle — a call resolves through the callee's return summary, and a
            // call has TWO spellings.  `Value::Call` names its definition; a `CallRef`
            // names a runtime value, so the target is recovered from the assignment that
            // put the closure in the variable and the SAME `call_ownership` answers.
            //
            // Without this arm a `CallRef` fell to the fallback below and was called
            // `Owned` — the one answer that licenses a free — so the oracle was silently
            // wrong about every closure call and was saved only by its readers gating on
            // the `Call` spelling first.  What that cost is the mint arm of a closure whose
            // return may also be a borrow: the deps PROXY calls the whole thing a borrow,
            // the oracle was never asked, and the minted store got no owner (loft#1248).
            //
            // ⚠ ONLY the WITNESSED `Join` is delivered; every other verdict keeps the
            // `Own::Owned` a `CallRef` answered before this arm existed, and that narrowness
            // is measured rather than cautious.  `scan_set` reads this verdict to decide
            // whether a reassigned local is tracked as OWNED, so departing on a `Borrowed`
            // too moved the ownership-transition free for closure calls this fix has nothing
            // to say about — `--native` then freed a capture the caller still held and
            // `1114`'s named twin read `7`, an unrelated record in the recycled slot.
            //
            // A witnessed `Join` is the one verdict the three `callref_join_first_bind`
            // readers act on, so it is the whole of what this arm needs to carry.  Widening
            // it to the honest full answer is a separate change with its own measurement to
            // make; `Own::Unknown` — forcing each caller to decide rather than defaulting to
            // the permissive value — is what would make that attempt safe.
            Value::CallRef(fn_var, args) => {
                let Some(d) = defs
                    .fnref_targets
                    .get(fn_var)
                    .copied()
                    .filter(|d| *d != u32::MAX)
                else {
                    return Own::Owned;
                };
                let callee_own = self.return_ownership(d);
                let callee_base = match callee_own {
                    Own::Owned => return Own::Owned,
                    Own::Borrowed { base } | Own::Join { base } => base,
                };
                let mut base = self.caller_arg_base(d, callee_base, args, func, defs);
                if base == u16::MAX {
                    base = self.closure_capture_base(d, callee_base, *fn_var, defs);
                }
                // ⚠ An UNNAMEABLE base answers `Owned` here, and that is a fallback readers
                // must not take at face value: every site that would free on it gates on
                // `callref_capture_blocks`, which asks the CALLEE's own verdict
                // (`return_ownership`) rather than this one.  Answering `Borrowed { u16::MAX }`
                // instead was measured to break the direct nullable-capture return
                // (`fn(n) -> P? { return c; }`, guard 1114), whose delivery reads this arm.
                match callee_own {
                    Own::Join { .. } if base != u16::MAX => Own::Join { base },
                    _ => Own::Owned,
                }
            }
            // Everything else is a literal, a scalar/void op, or control carrying no value
            // payload — nothing that can name a store some other binding owns, so calling it
            // `Owned` cannot license a free of someone else's record.  The two shapes that
            // CAN are both named above: a projection, and a call in either spelling.
            _ => Own::Owned,
        }
    }

    /// The ownership of a `call(args)` result, with the borrow base translated from
    /// the callee's parameter space into the CALLER's argument (the interprocedural
    /// piece): the callee's return borrows one of its visible params; map that param
    /// position to the caller's argument so the `base` is a var the caller can witness.
    fn call_ownership(
        &mut self,
        callee_d: u32,
        caller_args: &[Value],
        func: &Function,
        defs: &Defs,
    ) -> Own {
        let callee_own = self.return_ownership(callee_d);
        let callee_base = match callee_own {
            Own::Owned => return Own::Owned,
            Own::Borrowed { base } | Own::Join { base } => base,
        };
        let base = self.caller_arg_base(callee_d, callee_base, caller_args, func, defs);
        match callee_own {
            Own::Join { .. } => Own::Join { base },
            _ => Own::Borrowed { base },
        }
    }

    /// Map the callee's borrowed parameter `callee_base` (a var in the callee's
    /// space) to the CALLER's argument var at the same VISIBLE-parameter position.
    /// `u16::MAX` when it is not a visible param or the matching arg is not a var.
    ///
    /// A PROJECTION argument is mapped to its ROOT container through [`view_root_slots`] —
    /// the same walk the @P290 bracket protects through, so the store the guard witnesses
    /// against is the store the bracket marks.  `u16::MAX` when the argument is not a view
    /// of one nameable container: a mint, or a join reaching two different roots.
    ///
    /// **An argument that is itself a CALL is resolved by the oracle rather than by that
    /// walk.**  `view_root_slots` is structural and stops at a loft-defined call, whose
    /// returned store may be its own argument's or one it minted — the split only
    /// [`Ownership::classify`] decides.  So ask it: `g(pick(vs, 0))` roots at `vs` exactly
    /// as `g(vs[0])` does, one frame further out, and `g(mk())` stays unnameable because
    /// the store `mk` minted belongs to no caller variable.  Without this the witness is
    /// missing for every call-shaped argument, and a missing witness is the OVER-FREE
    /// direction at a `CallRef` (loft#1318).
    fn caller_arg_base(
        &mut self,
        callee_d: u32,
        callee_base: u16,
        caller_args: &[Value],
        func: &Function,
        defs: &Defs,
    ) -> u16 {
        match structural_arg_base(self.data, callee_d, callee_base, caller_args) {
            Ok(base) => base,
            // A structural walk names a variable and what projects out of one; an argument
            // that is itself a CALL is not one of those shapes, and the oracle is what
            // answers for a call (@FR-O-Oracle).  Ask it: a callee handing back a view of
            // its own argument roots this argument in the caller's variable just as a
            // projection does, one frame further out.  `Owned` — the callee minted the
            // store — leaves the base unnameable, which is the right answer there, because
            // then no caller variable holds it.
            Err(arg) => match self.classify(arg, func, defs) {
                Own::Borrowed { base } | Own::Join { base } => base,
                Own::Owned => u16::MAX,
            },
        }
    }
    /// loft#1248 — the caller variable a return that borrows the closure may be handing back.
    ///
    /// `caller_arg_base` maps a callee's borrowed VISIBLE parameter to the argument at the
    /// same position and answers `u16::MAX` for a hidden one.  `__closure` is hidden, so a
    /// closure returning something it CAPTURED had no witness and the conservative no-lift
    /// stood — correct, and it cost the mint arm of every such `??` its owner.
    ///
    /// The mapping is in the IR: the closure build writes each captured value into the record
    /// with `OpSetDbRef(___clos_N, <slot>, <caller var>)`, which `fnref_captures` collects.
    /// What the IR does NOT carry is which SLOT the return borrows — the dep names `__closure`
    /// and stops there.
    ///
    /// So this answers only where that ambiguity cannot arise: EXACTLY ONE store-bearing
    /// capture.  With two, the return may be either, and comparing against the wrong one would
    /// adopt a store the caller still holds — the over-free this gate exists to refuse.  A
    /// closure with two heap captures keeps the leak it has; closing that needs the dep to
    /// name the capture rather than the record.
    fn closure_capture_base(
        &self,
        callee_d: u32,
        callee_base: u16,
        fn_var: u16,
        defs: &Defs,
    ) -> u16 {
        // Asked of the callee's VARIABLE space, which is what `callee_base` names.  It is not
        // the attribute space `caller_arg_base` indexes into: measured on the closure this
        // fix is about, `__closure` is variable 3 and attribute 2, so an attr-indexed test
        // reads out of range and answers "not the closure" for the one case it exists for.
        if self.data.def(callee_d).variables().name(callee_base) != "__closure" {
            return u16::MAX;
        }
        let Some(captures) = defs.fnref_captures.get(&fn_var) else {
            return u16::MAX;
        };
        // Which capture the return can hand back is written in the callee's body: the offset
        // its `??` subject (or tail) reads through `OpGetDbRef(__closure, off)`.  One offset
        // names one caller variable; two is `c ?? d` and no single witness answers for it.
        let offsets = capture_return_offsets(self.data, callee_d);
        let var = match (captures.as_slice(), offsets.as_slice()) {
            ([(_, only)], _) => *only,
            (many, [off]) => match many.iter().find(|(o, _)| o == off) {
                Some((_, v)) => *v,
                None => return u16::MAX,
            },
            _ => return u16::MAX,
        };
        // The closure holds the store the variable had at BUILD time; a variable assigned again
        // since may name a different store, and comparing against that would adopt or free the
        // closure's own capture.
        if defs.multi_assigned.contains(&var) {
            return u16::MAX;
        }
        var
    }

    /// The owned-slot reassignments in function `d_nr`: for each var with more than
    /// one real def, the class it held before its LAST def and the class of that
    /// def. The `prior = Owned` rows are the displaced-owned-store leak candidates.
    fn reassign_sites(&mut self, d_nr: u32) -> Vec<ReassignSite> {
        let data = self.data;
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            return Vec::new();
        }
        self.reassign_sites_of(&def.code, &def.variables)
    }

    /// As [`Self::reassign_sites`] but on a `(code, function)` pair directly — for
    /// the scope pass, which holds the function being analysed by reference (it is
    /// not yet written back into `data`).
    fn reassign_sites_of(&mut self, code: &Value, func: &Function) -> Vec<ReassignSite> {
        let mut defs = Defs::default();
        collect_defs(code, &FillOps::of(self.data), &mut defs);
        // Only HEAP-typed vars can carry the over-free leak: a reassigned scalar
        // loop counter has no store to displace (the class is record-specific —
        // "scalar never fires" per the boundary map). Filter them out.
        //
        // Through `base`, because `S?` is `S` behind a nullability marker and holds the
        // same store (@FR-L-Null: layout(τ) = layout(τ?)). Asked bare, every nullable heap
        // local fell out of this filter, so the oracle had no reassignment row for the
        // shape loft#1106 turned out to be — an ownership defect on a `τ?` local. An
        // instrument blind to a class reports it green.
        let mut vars: Vec<u16> = defs
            .rhs
            .keys()
            .copied()
            .filter(|v| defs.rhs[v].len() > 1 && func.tp(*v).base().heap_dep().is_some())
            .collect();
        vars.sort_unstable();
        vars.into_iter()
            .map(|v| {
                let rhss = defs.rhs[&v].clone();
                let n = rhss.len();
                let prior = rhss[..n - 1]
                    .iter()
                    .map(|r| self.classify(r, func, &defs))
                    .reduce(Own::join)
                    .unwrap_or(Own::Owned);
                let rhs = self.classify(&rhss[n - 1], func, &defs);
                ReassignSite {
                    var: v,
                    var_name: func.name(v).to_string(),
                    prior,
                    rhs,
                }
            })
            .collect()
    }

    /// The base var a `Borrowed`/`Join` value's borrow arm views, when that arm is
    /// a DIRECT projection in this function — the store the Stage-3 materialise
    /// copies FROM (Gap B). `None` when the borrow is produced by a call (the
    /// materialise deep-copies the whole returned value) or the value is `Owned`.
    fn borrow_base(&self, node: &Value, func: &Function, defs: &Defs) -> Option<u16> {
        let mut visited = Vec::new();
        self.borrow_base_guarded(node, func, defs, &mut visited)
    }

    /// The recursive worker with CYCLE protection: a var whose def-chain reaches
    /// itself (`bs += […]` reads `bs`; two-var swaps) otherwise recursed forever
    /// — a stack-overflow SIGSEGV once the oracle ran by default (the D-own-1
    /// flip; loft_suite's 85-store-lifetime-return-field-of-local under the wrap
    /// harness).  A revisited var yields `None`: a cyclic chain has no single
    /// borrow base, and every caller handles `None` conservatively.
    fn borrow_base_guarded(
        &self,
        node: &Value,
        func: &Function,
        defs: &Defs,
        visited: &mut Vec<u16>,
    ) -> Option<u16> {
        match node.unspan() {
            Value::Call(d, args) if self.projections.contains(d) => {
                match args.first().map(Value::unspan) {
                    Some(Value::Var(b)) => Some(*b),
                    // A struct-ENUM payload projection names its subject through a
                    // variant check — `sh.inner` is `OpGetField(if <tag> { sh } else
                    // { sentinel }, …)` — so the subject is one wrapper further out
                    // than a struct field's.  Stop at it exactly as the `Var` arm above
                    // does ([`projection_container_var`] peels the same shape for the
                    // materialise walk and the emitters).  Without the peel the `If` arm
                    // below resolved the subject THROUGH its own definitions, found a
                    // mint, and answered `None` — so a payload view classified `Owned`,
                    // the over-free direction this doc's own caveat names, and the
                    // `lost-write` lint told the author a write that lands was lost.
                    Some(Value::If(_, t, e))
                        if variant_check_subject(self.data, t, e).is_some() =>
                    {
                        variant_check_subject(self.data, t, e)
                    }
                    // nested `o.inner.rows`
                    Some(inner) => self.borrow_base_guarded(inner, func, defs, visited),
                    None => None,
                }
            }
            Value::Var(v) => {
                if visited.contains(v) {
                    return None;
                }
                visited.push(*v);
                if let Some(rhss) = defs.rhs.get(v) {
                    rhss.iter()
                        .rev()
                        .find_map(|r| self.borrow_base_guarded(r, func, defs, visited))
                } else if func.is_argument(*v) && func.tp(*v).heap_dep().is_some() {
                    Some(*v) // a borrowed heap param IS its own base (the caller's arg)
                } else {
                    None
                }
            }
            Value::If(_, then, els) => self
                .borrow_base_guarded(then, func, defs, visited)
                .or_else(|| self.borrow_base_guarded(els, func, defs, visited)),
            Value::Block(b) => b
                .operators
                .last()
                .and_then(|t| self.borrow_base_guarded(t, func, defs, visited)),
            Value::Return(v) => self.borrow_base_guarded(v, func, defs, visited),
            _ => None,
        }
    }

    /// The over-free free SITES in function `d_nr` (Gap A): each append element
    /// source-free (`AppendSource`) with its source's class + base, and each
    /// return-buffer reassign to a `Borrowed`/`Join` value (`ParamDeliver`). These
    /// are the sites the value classification alone does not surface — the Stage-3
    /// fix reads the class here and frees / materialises accordingly.
    fn free_sites(&mut self, d_nr: u32) -> Vec<FreeSite> {
        let def = self.data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            return Vec::new();
        }
        let func = &def.variables;
        let mut defs = Defs::default();
        collect_defs(&def.code, &FillOps::of(self.data), &mut defs);
        let mut appends = Vec::new();
        let mut delivers = Vec::new();
        collect_free_candidates(
            &def.code,
            self.op_copy_record,
            func,
            &mut appends,
            &mut delivers,
        );

        let name = |v: u16| (v != u16::MAX).then(|| func.name(v).to_string());
        let mut sites = Vec::new();
        for src in appends {
            let class = self.classify(&src, func, &defs);
            // Only a Borrowed/Join source is over-freed; an Owned source-free is
            // correct (and load-bearing for the owned branch of a Join elsewhere).
            if !matches!(class, Own::Borrowed { .. } | Own::Join { .. }) {
                continue;
            }
            let base = class.base().filter(|&b| b != u16::MAX);
            let slot = match src.unspan() {
                Value::Var(v) => *v,
                _ => u16::MAX,
            };
            sites.push(FreeSite {
                kind: FreeKind::AppendSource,
                slot,
                slot_name: name(slot).unwrap_or_default(),
                class,
                base,
                base_name: base.and_then(name),
            });
        }
        for (p, rhs) in delivers {
            let class = self.classify(&rhs, func, &defs);
            let base = class.base().filter(|&b| b != u16::MAX);
            // A retbuf aliases a store it does not own ONLY when set to a DIRECT
            // borrow — a raw projection with a known base (`_mv_items_1 =
            // OpGetField(e,…)`). A call delivering into the retbuf MATERIALISES a
            // copy (the retbuf is then Owned — the clean `best = rows(b)` shape),
            // so `base.is_none()` cases are excluded; this also guarantees a usable
            // materialise base for every reported ParamDeliver.
            if matches!(class, Own::Borrowed { .. } | Own::Join { .. }) && base.is_some() {
                sites.push(FreeSite {
                    kind: FreeKind::ParamDeliver,
                    slot: p,
                    slot_name: func.name(p).to_string(),
                    class,
                    base,
                    base_name: base.and_then(name),
                });
            }
        }
        sites
    }
}

/// The STRUCTURAL half of the callee→caller base translation, read by the oracle
/// ([`Ownership::caller_arg_base`]) and by its flow-sensitive shadow (`ownership_cfg`) alike, so
/// the two cannot drift on it: the shadow's private copy carried none of loft#1318's three fixes
/// and reported the oracle's CORRECT answers as disagreements — `Borrowed(MAX)` for a call
/// delivering through a hidden buffer, and no root for a projection argument (QUALITY.md B7r).
///
/// `Ok(base)` names the caller variable the callee's borrowed parameter `callee_base` arrives
/// in, `u16::MAX` where nothing does: a hidden parameter that is not a delivery buffer (a
/// return MECHANISM, naming nothing on the caller's side — `__closure` is resolved from the
/// closure build instead), a missing argument, or a projection reaching two roots (a join whose
/// arms name different containers has no single store to compare against).  The delivery
/// BUFFER is the hidden parameter that IS nameable: the caller allocates its own `__ref_N` and
/// passes it at that position, so a callee handing back its `__retbuf` hands back a store the
/// caller already holds — the answer that stops the caller adopting its own buffer.  A
/// projection argument is witnessed by its ROOT (`view_root_slots`): `pick(v[0], …)`,
/// `pick(w.s, …)` and `pick(h[k], …)` all answer a `DbRef` living in the root container's
/// store.  `Err(arg)` hands back an argument that is itself a CALL, which a structural walk
/// cannot root and only the oracle can (@FR-O-Oracle).
pub(crate) fn structural_arg_base<'a>(
    data: &Data,
    callee_d: u32,
    callee_base: u16,
    caller_args: &'a [Value],
) -> Result<u16, &'a Value> {
    let attrs = data.def(callee_d).attributes();
    if callee_base == u16::MAX || (callee_base as usize) >= attrs.len() {
        return Ok(u16::MAX);
    }
    if attrs[callee_base as usize].hidden && !is_synth_buffer(&attrs[callee_base as usize].name) {
        return Ok(u16::MAX);
    }
    // The caller's args align with the callee's VISIBLE params, in order.
    let arg_index = attrs[..callee_base as usize]
        .iter()
        .filter(|a| !a.hidden)
        .count();
    let Some(arg) = caller_args.get(arg_index) else {
        return Ok(u16::MAX);
    };
    match view_root_slots(data, arg).as_deref() {
        Some([root]) => Ok(*root),
        Some(_) => Ok(u16::MAX),
        None => Err(arg),
    }
}

/// Public, test-facing entry: the ownership class of function `d_nr`'s return.
#[must_use]
pub fn return_ownership(data: &Data, d_nr: u32) -> Own {
    Ownership::new(data).return_ownership(d_nr)
}

/// @PLN103 P1.5 — how a function DELIVERS a heap (vector / reference) return, and
/// therefore **who frees the storage it names**.
///
/// The three answers are genuinely different obligations, and reading them as two
/// is how storage goes missing or gets freed twice:
///
/// * [`Owned`](HeapDelivery::Owned) — a fresh store the callee minted and hands
///   over. The CALLER frees it.
/// * [`RetBuf`](HeapDelivery::RetBuf) — written into the hidden buffer the caller
///   supplied; the result borrows that buffer, which the caller already owns.
/// * [`View`](HeapDelivery::View) — a view of something the callee did not create:
///   an argument, or its own long-lived state. Nobody frees it on the caller's
///   behalf, because there is nothing new to free.
///
/// Read from the return type's deps on the COMMITTED IR, which is robust: a
/// per-arm walk of the delivered IR was tried and dropped, because post-synthesis
/// the arm structure is rewritten and per-arm `ownership_of` then misleads.
///
/// One home because the answers are consumed far apart — the `--show-ownership`
/// overlay renders it, and @PLN119 decides from it whether a library function can
/// be placed at all. A placed `View` return would hand the caller a copy it never
/// frees (a leak) where in-process it gets a borrow it correctly ignores.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HeapDelivery {
    /// Not a heap return (a scalar, text, or void).
    NotHeap,
    /// A fresh store the callee owns and hands over; the caller frees it.
    Owned,
    /// Materialised into the caller-supplied return buffer.
    RetBuf,
    /// A borrowed view of an argument or of the callee's own state.
    View,
}

/// loft#981 / loft#982 — the @P290 call-bracket's COVERAGE question, in one place so
/// the emit that PROTECTS a call's arguments and the gate that decides whether the
/// returned store may be source-freed cannot drift apart (they are two reads of one
/// fact, at opposite ends of the same emitted sequence).
///
/// Answers, for one call site: which ref-typed arguments can be bracketed with
/// `protect_store_frees`, and do they cover EVERY ref-typed argument?
///
/// Coverage is what makes a BORROWED-VIEW return's source-free safe. A callee whose
/// return dep names a visible parameter may hand back either that parameter's store
/// (the borrow arm — the caller must not free it) or a store it minted itself (the
/// owned arm — the caller must free it, or it leaks one store per call). No static
/// bit can carry that split, so the decision is made at RUNTIME by the bracket: a
/// returned store belonging to a protected argument is refused the free by
/// `do_copy_record` / `OpCopyRecord`, and a callee-minted one is not protected and
/// so is freed.
///
/// The bracket needs a SLOT to name. A bare `Var` is one; so is any argument that
/// only DERIVES a view of one — `b.s`, `w[0]`, `vb.v`, `o ?? q` — because the store
/// it names is the root variable's store and that variable already holds its `DbRef`
/// before the call ([`view_root_slots`]). When an argument is neither, the witness
/// set is incomplete and the caller keeps the old, conservative "never free" answer
/// — the leak stays for that shape rather than risking a free of a store the caller
/// still reaches.
#[must_use]
/// The witness @FR-O-Move needs at a call: when a return BORROWS a parameter the caller must
/// copy, so the bracket has to name the argument's store.  D-own-6 is the register entry for
/// what this missed when the witness was not total.
pub fn protectable_ref_args(data: &Data, d_nr: u32, call: &Value) -> (Vec<u16>, bool) {
    // loft#1245 — a `CallRef` is a call whose callee lives in a variable.  Reading only
    // the `Call` spelling gave every fn-ref site an EMPTY, incomplete witness set, so the
    // caller kept the conservative never-free answer and the store the callee minted was
    // orphaned once per call.  `callee_of` resolves both spellings; an unresolved fn-ref
    // still answers "incomplete", which is that same conservative answer.
    let (Value::Call(_, args) | Value::CallRef(_, args)) = call.unspan() else {
        return (Vec::new(), false);
    };
    let Some(fn_nr) = callee_of(data, d_nr, call) else {
        return (Vec::new(), false);
    };
    let attrs = data.def(fn_nr).attributes();
    let mut protectable = Vec::new();
    let mut covers_all = true;
    for (i, arg) in args.iter().enumerate() {
        let Some(tp) = attrs.get(i).map(|a| &a.typedef) else {
            continue;
        };
        // Can the callee's return borrow THIS argument's store?  `heap_dep` is the
        // canonical "carries a store" question — and asking it through `base` as well,
        // because an `Optional` wrapper hides the storage under it.  A scalar argument
        // is no one's borrow source and neither protects nor blocks.
        if tp.heap_dep().is_none() && tp.base().heap_dep().is_none() {
            continue;
        }
        // Every argument whose type CARRIES A STORE is protectable, because the bracket
        // marks that store through the argument's own `DbRef` and every one of these
        // holds one.  loft#981 is why this must stay in step with `heap_dep` above: a
        // KEYED COLLECTION was outside the older filter, so it was neither protected nor
        // counted as uncovered — the set read complete while protecting nothing, and the
        // free it licensed took a hash parameter's element out from under the caller
        // (`fn take(h) -> R { h[k] ?? R{…} }`, tests/scripts/882-…).  The cure then was to
        // make it INCOMPLETE, which was right but left the leak; the cure now is to
        // protect it, which is what the emit could do all along.
        match arg {
            Value::Var(av) if is_protectable_store_type(tp) => {
                protectable.push(*av);
            }
            // A `null` argument holds NO STORE, so nothing the callee returns can be a
            // borrow of it — it neither needs protecting nor leaves the witness set
            // incomplete.  Reading it as uncovered is what made the caller keep the
            // conservative never-free answer for an OMITTED `τ? = null` parameter,
            // which the parser fills with exactly this `Value::Null`: `fn f(a: P? = null)
            // -> P { a? }` called as `f()` then leaked the record the null path built,
            // once per call.  The same call with a bare VAR holding null was always
            // clean, which is what localised it (loft#1021).
            Value::Null => {}
            // loft#1029 — the same argument one lowering later. `Parser::convert` turns a
            // `null` LITERAL in a reference-typed argument position into a call to
            // `OpNullRefSentinel`, so by the time this runs the `Value::Null` arm above no
            // longer matches and the site read as uncovered — the conservative never-free
            // answer, and one leaked record per call for `fn pick(f: S?) -> S { f? }`
            // called as `pick(null)`. The sentinel holds NO STORE (`store_nr == u16::MAX`),
            // exactly like the bare `Null` it was lowered from, so nothing the callee
            // returns can be a borrow of it: it neither needs protecting nor blocks.
            Value::Call(d, cargs)
                if cargs.is_empty() && data.def(*d).name() == "OpNullRefSentinel" => {}
            // loft#1029 — an argument that merely DERIVES a view of a variable. The
            // bracket marks a STORE, not a slot, so the slot it names does not have to
            // be the argument itself: `pick(b.s, …)` lowers to `OpGetField(Var(b), …)`,
            // whose `DbRef` lies in `b`'s store, so protecting `b` protects exactly the
            // store the callee's return might borrow. Without this the site read as
            // uncovered and every one of these ordinary spellings leaked one record per
            // call on BOTH backends — a field, a nested field, a vector ELEMENT, a
            // vector-typed field, `??`, and an `if` in argument position.
            //
            // Asked through the same emit filter as the `Var` arm above: a witness the
            // bracket cannot emit must leave the set INCOMPLETE, never vanish from it
            // (loft#981 — the narrower list read as complete freed a hash parameter's
            // element out from under the caller).
            _ if is_protectable_store_type(tp) => match view_root_slots(data, arg) {
                Some(roots) => protectable.extend(roots),
                None => covers_all = false,
            },
            _ => covers_all = false,
        }
    }
    protectable.sort_unstable();
    protectable.dedup();
    (protectable, covers_all)
}

/// Can the @P290 bracket mark the store behind an argument of this type?
///
/// True for every type whose value IS a `DbRef` into a store the caller can still reach —
/// a reference, a vector, a struct-enum, and each keyed collection.  `protect_store_frees`
/// marks `allocations[r.store_nr]`, so all it needs is that `DbRef`.
///
/// Keep this in step with the `heap_dep` question the caller asks first: a type that
/// carries a store and is NOT here leaves the witness set incomplete and the caller keeps
/// the conservative never-free answer — correct, but it leaks one record per call.  A type
/// that is here without carrying a store would be worse: the set would read complete while
/// protecting nothing, which is the loft#981 use-after-free.
fn is_protectable_store_type(tp: &Type) -> bool {
    // In step with the `heap_dep` question its caller asks — both peel — because
    // `@FR-L-Null` says a `τ?` value IS the same `DbRef`.  Asked bare, a `τ?` parameter
    // passed the caller's filter and then failed here, so the witness set read INCOMPLETE
    // and the site kept the conservative never-free: correct, and leaking one store per
    // call on the arm where the callee MINTS.
    //
    // ⚠ This peel was written down and deliberately deferred, on the ground that *"the
    // change has no measurement asking for it"* — it was not inert (it moves emitted code
    // in six corpus programs, every one a guard for this machinery: 1021, 1029, 1105, 1106,
    // 1107, 882) and it moves in the direction where a mistake is a use-after-free rather
    // than a leak.  loft#1150 is the measurement: `fn f(x: hash<T[k]>?, c) -> hash<T[k]>?
    // { if c { x } else { [lit] } }` leaked its mint once per call, while the identical
    // DENSE signature was clean — one program, two spellings, and only the wrapper between
    // them.  All six named guards are green under `LOFT_STRICT_STORES=1` and `LOFT_POISON=1`
    // on both backends with the peel in place, which is the check the deferral asked for.
    crate::data::is_dbref(tp.base())
}

/// loft#1029 — the variable slots whose STORES an argument's value can lie in, or
/// `None` when the argument is not a view of any nameable variable.
///
/// The @P290 bracket protects a store, and it names that store through a variable
/// holding a `DbRef` into it. So an argument does not have to BE a variable — it only
/// has to be derived from one by operations that stay inside the same store:
///
/// * `Var(v)` — the slot itself.
/// * a field or element PROJECTION of one ([`is_projection_op`]) — `b.s`, `d.b.s`,
///   `w[0]`, `vb.v` — walked to the root the chain starts at. A LOFT-DEFINED call is
///   not one: its returned store may be the argument's OR one it minted, which is the
///   very split the bracket exists to decide, and its result reaches this list as a
///   plain `Var` because the caller lifts it into a temp first.
/// * a JOIN (`o ?? q`, `if c { q } else { r }`) — every arm, since either store can
///   be the one that comes back. Over-protecting is safe in the direction that
///   matters: an extra marked store can only REFUSE a free, never license one.
/// * `null` in either spelling, which holds no store and so needs no witness.
///
/// A value that mints its own store (a struct or collection LITERAL, still wrapped in
/// its construction block) is deliberately NOT here: the block has not run when the
/// bracket is emitted, so its work-ref still holds null and marking it would protect
/// nothing while reading as covered — trading this leak for a use-after-free. That
/// shape is cured at the call site instead, by hoisting the construction
/// (`Scopes::inline_built_borrow_source`).
/// `pub` for loft#1154's join gate, which needs the SLOTS and not just the boolean
/// [`bracket_can_name`] answers — a join's arms must be protected individually, and an arm the
/// bracket cannot name is the one that vetoes the whole decision.
pub fn view_root_slots(data: &Data, arg: &Value) -> Option<Vec<u16>> {
    // A SPAN is source position, not structure: the parser wraps a field access in one
    // and leaves a bare local unwrapped, which is why `pick(q, …)` was clean while
    // `pick(b.s, …)` leaked. Reading through it is safe because the bracket is emitted
    // from the SLOT list this returns — neither backend matches the argument value
    // again (`state/codegen.rs` builds its own `Var`, `generation/dispatch.rs` renders
    // `var_<name>`), so a span can never reach the emit.
    match arg.unspan() {
        Value::Var(v) => Some(vec![*v]),
        Value::Null => Some(Vec::new()),
        Value::If(_, then_v, else_v) => {
            let mut roots = view_root_slots(data, then_v)?;
            roots.extend(view_root_slots(data, else_v)?);
            Some(roots)
        }
        // A bare `{ v }` arm carries no ops of its own — the parser wraps an `if`'s
        // arms this way. A block with construction ops in it is a MINT, not a view,
        // and falls through to `None` above.
        Value::Block(bl) if bl.operators.len() == 1 => view_root_slots(data, &bl.operators[0]),
        Value::Call(d, cargs) => {
            if cargs.is_empty() && data.def(*d).name() == "OpNullRefSentinel" {
                return Some(Vec::new());
            }
            if !is_projection_op(data, *d) {
                return None;
            }
            view_root_slots(data, cargs.first()?)
        }
        _ => None,
    }
}

/// Can the @P290 bracket NAME the store this argument's value will lie in?
///
/// `false` means the witness set would read incomplete at this argument, and the caller then
/// keeps the conservative never-free answer — correct, but it copies the returned store and
/// orphans the one the callee minted, one record per call. The cure is to give the value a
/// NAME (bind it to a temp before the call), which is what `Scopes::scan_args` does; this is
/// the question it asks first.
///
/// One home for the question, because the answer must be the same one
/// [`protectable_ref_args`] will reach later — a hoist decided on a different reading would
/// either bind arguments nothing needed, or leave the leak it was meant to close.
///
/// Every op [`is_projection_op`] lists widens what this can name, so a precise witness and the
/// bind below are not rivals: the bind is what catches whatever the witness still cannot reach.
#[must_use]
pub fn bracket_can_name(data: &Data, arg: &Value) -> bool {
    view_root_slots(data, arg).is_some()
}

/// Is `d_nr` a field/element PROJECTION — an op that READS a `DbRef` out of its first
/// argument and answers one living in that argument's store?
///
/// Each of the four offsets or indexes within a store somebody else owns and allocates
/// nothing: `OpGetField` moves within the record (`DbRef { store_nr, rec, pos: pos + off }`),
/// `OpGetVector` indexes within the vector's own store (its out-of-range sentinel preserves
/// `store_nr` too), `OpVectorRef` dereferences a linked element's record pointer, and
/// `OpGetRecord` looks a key up in a keyed collection.  For all four the root variable's
/// store IS the result's store, which is what makes a projection chain nameable by its root
/// and lets the @P290 bracket protect `pick(b.s, …)`, `pick(w[0], …)` and `pick(h[k], …)`.
///
/// ⚠ **The two NULLABLE element reads meet the criterion above and are deliberately NOT on
/// the list, because adding them strands a store.** `OpGetVectorNullable` and
/// `OpVectorRefNullable` are `v[i]` where an out-of-range index answers the null element
/// instead of raising, and both are declared `-> reference[r]`, so the store they answer in
/// is the receiver's exactly as their dense twins' is.
///
/// What blocks them is a disagreement one layer down, and it is `@FR-O-Proxy`'s named
/// hazard in the ALLOCATE direction: the interpreter's materialise arm
/// (`state/codegen.rs`, @PLN130 F1) fires on the deps PROXY — empty deps plus
/// [`crate::generation::container_element_base`] — while the free sweep reads the ORACLE.
/// A `par` body's element bind is typed without a dep and classifies `Borrowed`, so it sits
/// in the gap: adding the spellings makes the arm allocate a store the sweep then declines
/// to free. Measured on `tests/scripts/1040-generic-par-worker-in-generic-fn.loft` — three
/// leaked `Cell` records, both backends. The arm's own premise says why: it assumes empty
/// deps mean `scopes.rs` STRIPPED them and a free therefore exists, which holds for the
/// reassigned-container case it was written for and not for a dep that was never set.
///
/// Nothing today reads the wrong answer through the gap — `caller_arg_base` resolves a
/// nullable element read by asking the oracle instead, which is why loft#1318 closed
/// without this. Correcting the list needs the two sites to ask ONE question first.
///
/// **The criterion is not "the return deps on parameter 0"**, which several more ops also
/// satisfy: `OpNewRecord` and `OpInsertVector` both answer a `DbRef` in argument 0's store
/// and are excluded, because they GROW that store rather than read it — a chain rooted at one
/// is a construction, and the readers below ask which container an existing view came out of.
///
/// One list, four readers, because the alternative is measured rather than hypothetical: a
/// list SHORT by `OpGetRecord` left `pick(h[k], …)` without a witness, so the caller kept the
/// conservative never-free answer and leaked one record per call at every keyed kind (hash /
/// sorted / index, both backends).  The four readers are this bracket's
/// [`view_root_slots`], the parser's `Parser::projection_root_mut` (which inline container
/// needs a name), `scopes::base_container_var` and `generation::container_element_base` (@PLN130
/// F2 — which container a view reads out of).  They ask different questions of the same shape;
/// keeping the shape in one place is what stops the answers drifting apart.
#[must_use]
pub fn is_projection_op(data: &Data, d_nr: u32) -> bool {
    matches!(
        data.def(d_nr).name(),
        "OpGetField" | "OpGetVector" | "OpVectorRef" | "OpGetRecord"
    )
}

/// The container VARIABLE a projection chain ultimately reads out of — the ONE derivation of
/// *which container did this view come out of?*
///
/// `OpGetVector` / `OpVectorRef` are `v[i]`, `OpGetField` is `s.f`; all of them yield a `DbRef`
/// sharing `store_nr` with the container, which is what makes binding one a borrow rather than
/// an owner.  The whole chain is peeled, because a view can sit several levels down
/// (`outs[2].inner`): looking only at the outermost call's first argument finds another call.
///
/// A struct-ENUM base is peeled too.  A payload projection does not name its subject directly
/// — `sh.inner`, and a `match`/`is` payload binding, lower to `OpGetField(if <tag == Holder>
/// { sh } else { OpNullRefSentinel() }, …)` — so the chain bottomed out at `None` and no
/// reader could name the subject: the view-materialise walk saw no container to be disturbed,
/// and a payload view live across a reassignment of its subject was neither copied nor
/// reported, where the plain-struct twin is both.  Only the shape the lowering emits is
/// peeled — exactly one arm names a variable and the other is the null sentinel — so a user
/// `if` choosing between two containers still answers `None`.
///
/// `None` when the chain does not bottom out in a plain variable, notably a field of a CALL
/// result (`return mk().a.b`), whose base is a temporary rather than a container someone else
/// owns: materialising there is a different question, and answering it here broke
/// `tests/scripts/85-store-lifetime-return-field-of-local`.  Every caller then leaves the
/// binding exactly as it is rather than guessing.
///
/// Two readers had this loop byte-for-byte — `scopes::base_container_var` and
/// `generation::container_element_base`, each documented as "mirroring" the other — and the
/// mirror is what the peel above had to be written into twice to work at all: the walk that
/// NAMES a view to materialise and the emitter that COPIES it must agree, or the compiler
/// reports a copy it did not make.
#[must_use]
pub fn projection_container_var(data: &Data, value: &Value) -> Option<u16> {
    projection_container_place(data, value).map(|(v, _)| v)
}

/// The offset [`projection_container_place`] reports for a chain that projects through no
/// field, so the place is the variable itself.
pub const ANY_FIELD: u32 = u32::MAX;

/// The PLACE a projection ultimately reads out of: the base variable, and the byte OFFSET of
/// the field it projects through — [`ANY_FIELD`] when it projects through none.
///
/// [`projection_container_var`] is this answer with the offset dropped, and the offset is what
/// tells `w.a` from `w.b`.  Both are containers of their own that happen to share a parent
/// variable, and a disturbance of one does not end the places inside the other: reading the
/// parent alone shook every view rooted at it whichever field it named, which silently emptied
/// `moros_editor`'s undo stack (loft#1373's own regression, and loft#1384 is the case that
/// costs).
///
/// The offset comes from the LAST `OpGetField` before the base variable — `w.a[0]` is
/// `OpGetVector(OpGetField(w, 16, …), 88, idx)`, so the place is `(w, 16)` — and a chain with
/// no field read, `v[0]` on a local, is `(v, ANY_FIELD)`.  A deeper chain (`w.inner.a[0]`)
/// answers its OUTERMOST field, which is the one a disturbance of `w` can name; the inner
/// steps are a lower bound, as everything in this walk is.
#[must_use]
pub fn projection_container_place(data: &Data, value: &Value) -> Option<(u16, u32)> {
    projection_place_of(data, value, false)
}

/// [`projection_container_place`], counting the two NULLABLE element reads as the projections
/// they are — the answer for a reader that asks *"which place does this value READ OUT OF?"*
/// rather than *"may I trigger the deps proxy on it?"*.
///
/// `OpGetVectorNullable` / `OpVectorRefNullable` are `v[i]` where an out-of-range index answers
/// the null element instead of raising; they are declared `-> reference[r]` and meet
/// [`is_projection_op`]'s criterion exactly, and that function's doc block says so at length.
/// They are kept off ITS list for a reason one layer down — the interpreter's materialise arm
/// fires on the deps PROXY while the free sweep reads the oracle, so widening the list there
/// makes the arm allocate a store the sweep declines to free.  That hazard belongs to the
/// PROXY, not to the notion, so a reader that only needs to NAME a place takes them.
///
/// The view-materialise walk is that reader: `(N-Index)` types `v[i]` as `τ?`, so a discharged
/// element read is the ordinary spelling of a projection and its container has to be nameable,
/// or the binding stays a live alias across a `remove` that renumbers it (loft#1401).  Nothing
/// here reaches the proxy: the walk only records which place a view names, and the copy that
/// pairs with it is emitted per ARM.
///
/// Fold the two back together once the arm and the sweep ask one question — the split is a
/// statement about today's readers, not about the language.
#[must_use]
pub fn view_source_place(data: &Data, value: &Value) -> Option<(u16, u32)> {
    projection_place_of(data, value, true)
}

/// The shared peel behind [`projection_container_place`] and [`view_source_place`]:
/// `nullable_reads` says whether the two null-answering element reads count as projections.
fn projection_place_of(data: &Data, value: &Value, nullable_reads: bool) -> Option<(u16, u32)> {
    let mut cur = value;
    let mut field = ANY_FIELD;
    loop {
        if let Value::If(_, t, e) = cur.unspan()
            && let Some(v) = variant_check_subject(data, t, e)
        {
            return Some((v, field));
        }
        let Value::Call(d, args) = cur.unspan() else {
            return None;
        };
        let projects = is_projection_op(data, *d)
            || (nullable_reads
                && matches!(
                    data.def(*d).name(),
                    "OpGetVectorNullable" | "OpVectorRefNullable"
                ));
        if !projects {
            return None;
        }
        if data.def(*d).name() == "OpGetField"
            && let Some(Value::Int(off)) = args.get(1).map(Value::unspan)
        {
            field = *off as u32;
        }
        match args.first().map(Value::unspan) {
            Some(Value::Var(c)) => return Some((*c, field)),
            Some(inner) => cur = inner,
            None => return None,
        }
    }
}

/// The subject of a VARIANT-CHECK `if` — the shape a struct-enum payload projection wraps its
/// base in: one arm is the subject variable, the other `OpNullRefSentinel()`.
///
/// BOTH arms are tested, and the sentinel one by the OP IT CALLS rather than by "not a
/// variable".  A loose reading claims user nodes: `a?` discharging a nullable parameter lowers
/// to `if a.rec != 0 { a } else { <mint> }`, which also has one `Var` arm — reading that as a
/// projection base changed a generic's return delivery from an adopt to a copy nobody freed,
/// one leaked record per call (caught by `tests/scripts/1026-generic-discharged-null-return`
/// under the wrap harness's leak gate).  Same class as loft#1379: recognise a lowering by what
/// it BUILDS, never by node shape alone.
fn variant_check_subject(data: &Data, then_arm: &Value, else_arm: &Value) -> Option<u16> {
    let named = |v: &Value| match v.unspan() {
        Value::Var(c) => Some(*c),
        _ => None,
    };
    let sentinel = |v: &Value| {
        matches!(v.unspan(), Value::Call(d, args)
            if args.is_empty() && data.def(*d).name() == "OpNullRefSentinel")
    };
    match (named(then_arm), named(else_arm)) {
        (Some(v), None) if sentinel(else_arm) => Some(v),
        (None, Some(v)) if sentinel(then_arm) => Some(v),
        _ => None,
    }
}

/// loft#981 / loft#982 — may this call site set `OpCopyRecord`'s `0x8000` source-free
/// bit on the store a call returned?
///
/// Yes for a return that borrows nothing (unchanged: the callee minted it and nobody
/// else owns it), and yes for a BORROWED-VIEW return whose every ref argument this
/// site protects — see [`protectable_ref_args`] for why the bracket is what decides
/// the borrow/owned split at runtime. No otherwise.
#[must_use]
pub fn call_return_frees_source(data: &Data, d_nr: u32, call: &Value) -> bool {
    // loft#1245 — both spellings, for the reason on [`protectable_ref_args`]: this gate
    // and that witness set are two reads of ONE fact at opposite ends of the same emitted
    // sequence, so a spelling one of them cannot see is a spelling neither can.
    if !matches!(call.unspan(), Value::Call(_, _) | Value::CallRef(_, _)) {
        return false;
    }
    let Some(fn_nr) = callee_of(data, d_nr, call) else {
        return false;
    };
    // loft#1114 / loft#1245 — a CAPTURING fn-ref never source-frees.  `returns_borrowed_view`
    // reads a HIDDEN-only return dep as "the callee minted this, the caller adopts", which is
    // right for `ref_return`'s `__ref_N` and a text work buffer and WRONG for `__closure`:
    // that record is the caller's, so a lambda handing back what it CAPTURED hands back a
    // store the outer scope still owns.  Freeing it releases a live variable — the capture
    // reads poison on the next access, which is loft#1114's exact fault.
    //
    // The @P290 bracket cannot rescue this one: it witnesses ARGUMENTS, and a capture is not
    // an argument, so there is no witness to name.  Declining the free is therefore the
    // conservative answer and deliberately keeps the pre-existing leak on the minting arm of
    // a capturing lambda — a leak is recoverable where a premature free is not, and the bind
    // still COPIES, so `(B-Copy)` holds either way.
    if callref_captures(data, d_nr, call) {
        return false;
    }
    !data.def(fn_nr).returns_borrowed_view() || protectable_ref_args(data, d_nr, call).1
}

/// loft#1106 — does a FIRST bind of a NULLABLE heap local from this call have to go
/// through the runtime join guard, the way its non-null twin already does?
///
/// `Some((record_def, base))` names the record type to allocate and the argument the
/// callee's return may borrow.  `None` leaves the bind exactly as it was.
///
/// `S?` is `Optional(Reference(S))` — the same storage as `S` behind a nullability
/// marker — and the heap first-bind dispatch on both backends asks its shape question
/// against the BARE type, so a nullable local never reached it.  It therefore got
/// neither of the two things that dispatch does: the (B-Copy) copy that keeps a bound
/// call result INDEPENDENT of the argument it may alias, and the @P290 bracket that
/// frees the callee's minted store on the arm where the return is not a borrow.  A
/// write through the bound result reached the caller's own variable, and the minting
/// arm leaked one record per call — the `-> S` twin of the same call did neither.
///
/// One home for the question, because three sites act on the answer and they must
/// agree: `scopes::scan_set` strips the local's deps so a free is emitted at all, and
/// the two backends emit the guard that makes that free correct.  A site that decided
/// this differently would either free a store the caller still names, or strip the
/// deps off a bind that stays a plain alias.
///
/// A JOIN or a pure BORROW with a nameable witness — `(F-Ret)` says the value a call
/// answers is FRESH whichever arm produced it, so a callee that always hands its argument
/// back (`fn keep(s: Node) -> Node { s }`, an element view `v[0]` through `-> T?`, a
/// generic instance of either) is bound through the same guard: the store aliases the
/// witness on every run and is copied on every run, and a `null` answer has no store to
/// alias and stays null.  Asked for a JOIN alone, a nullable local bound from a
/// borrow-returning callee stayed a plain alias — a write through it reached the caller's
/// argument on both backends, and a witnessed local's witness then named the CALLER's
/// store and released it (`@FR-O-Witness`, QUALITY.md B7v).  A call that borrows
/// nothing, or one whose witness the bracket cannot name, keeps today's plain adopt.
#[must_use]
pub fn nullable_join_first_bind(
    data: &Data,
    d_nr: u32,
    tp: &Type,
    value: &Value,
) -> Option<(u32, u16)> {
    if !crate::keys::join_own_enabled() {
        return None;
    }
    if !matches!(tp, Type::Optional(_)) {
        return None;
    }
    let (Type::Reference(rec, _) | Type::Enum(rec, true, _)) = tp.base() else {
        return None;
    };
    let Value::Call(fn_nr, _) = value.unspan() else {
        return None;
    };
    let callee = data.def(*fn_nr);
    if !callee.is_loft_defined() || !callee.returns_borrowed_view() {
        return None;
    }
    let base = match ownership_of(data, d_nr, value) {
        Own::Join { base } => base,
        // A pure BORROW is guarded only where the witness is the one argument the return
        // deps name: a return that may borrow EITHER of two arguments has two bases and one
        // witness would adopt the other, and a base the oracle resolved to an argument the
        // deps do not name is a translation the deps contradict (`(O-Oracle)`: never upgrade
        // on a base that cannot be trusted) — both keep today's plain adopt.
        Own::Borrowed { base } => {
            let attrs = callee.attributes();
            let visible: Vec<u16> = callee
                .returned()
                .depend()
                .iter()
                .copied()
                .filter(|&d| attrs.get(d as usize).is_some_and(|a| !a.hidden))
                .collect();
            let [dep] = visible[..] else {
                return None;
            };
            let Value::Call(_, args) = value.unspan() else {
                return None;
            };
            match args.get(dep as usize).map(Value::unspan) {
                Some(Value::Var(arg)) if *arg == base => base,
                _ => return None,
            }
        }
        Own::Owned => return None,
    };
    if base == u16::MAX {
        return None;
    }
    Some((*rec, base))
}

/// loft#1248 — the `CallRef` sibling of [`nullable_join_first_bind`]: does a FIRST bind
/// from a CLOSURE call have to go through the runtime join guard?
///
/// `Some((record_def, base))` names the record type to allocate and the value the callee's
/// return may borrow.  `None` leaves the bind exactly as it was.
///
/// Apart from its sibling rather than folded into it, because the two answer for spellings
/// whose OTHER paths differ: a `Value::Call` that is not nullable is already served by the
/// heap first-bind dispatch's own call arm, so that sibling only has to cover the nullable
/// hole.  A `CallRef` reaches NEITHER — the dispatch arm and the `scan_set` deps strip are
/// both keyed on `Value::Call` — so this one covers both nullabilities.  Folding them would
/// mean one predicate whose answer means "the hole" for one spelling and "everything" for
/// the other.
///
/// Same three readers and the same obligation: `scopes::scan_set` strips the local's deps so
/// a free is emitted at all, and the two backends emit the guard that makes that free
/// correct.  A site deciding this differently would either free a store the caller still
/// names, or strip the deps off a bind that stays a plain alias.
///
/// Narrow for the same reason: only a JOIN with a nameable witness.  A closure that borrows
/// nothing is already owned and adopts; one whose witness the @P290 bracket cannot name keeps
/// today's conservative no-free, which costs the leak it already had.
#[must_use]
pub fn callref_join_first_bind(
    data: &Data,
    d_nr: u32,
    tp: &Type,
    value: &Value,
) -> Option<(u32, u16)> {
    if !crate::keys::join_own_enabled() {
        return None;
    }
    let (Type::Reference(rec, _) | Type::Enum(rec, true, _)) = tp.base() else {
        return None;
    };
    if !matches!(value.unspan(), Value::CallRef(_, _)) {
        return None;
    }
    let Own::Join { base } = ownership_of(data, d_nr, value) else {
        return None;
    };
    if base == u16::MAX {
        return None;
    }
    Some((*rec, base))
}

/// The caller variable a fn-ref's COLLECTION `??` return may still be aliasing — the base a
/// bind of that call frees by store IDENTITY against (loft#1257, loft#1320).
///
/// The collection sibling of [`callref_join_first_bind`], and it answers the OPPOSITE way:
/// that one strips the local's deps so a plain free is emitted and `OpBindOrCopy` makes the
/// local own a store either way; a collection has no per-execution copy, so here the dep is
/// KEPT — it names the witness — and the free is `OpFreeRefIfDistinct(local, base)`.  Same
/// store as the base ⇒ still borrowing ⇒ decline; distinct ⇒ the closure minted ⇒ free.
///
/// Narrow for the same reasons: a `Join` with a NAMEABLE base only, and never through a
/// fn-ref that captures a store — a capture reaches the return through `__closure`, which
/// no caller variable names, so the identity test would have nothing true to compare with.
/// The caller variable a fn-ref call's DECLARED return borrows, or `None` where the type
/// names none.
///
/// The oracle's `CallRef` arm answers `Own::Owned` for a base it cannot NAME, and that
/// fallback is not a verdict: a forwarding lambda (`fwd = fn(q) { inner(q) }`) reaches its
/// own return through `__closure`, so its summary reads `Owned` while the type it declares
/// says `vector<T>["q"]` — the return borrows a visible parameter.  A site that frees on the
/// oracle alone therefore releases the caller's store.
///
/// The DECLARED dep is the fact the summary lost, and this maps it the same way
/// [`Ownership::caller_arg_base`] maps a resolved one, so the base a guarded free witnesses
/// against and the base the bracket protects cannot disagree.  Answers `None` for a hidden
/// attribute (a delivery buffer is not a borrow) and for an argument no caller variable
/// names, which is the conservative direction: no witness, no free.
#[must_use]
pub fn callref_declared_borrow_base(data: &Data, d_nr: u32, call: &Value) -> Option<u16> {
    let Value::CallRef(v_nr, args) = call.unspan() else {
        return None;
    };
    let defs = function_defs(data, d_nr);
    let callee = defs
        .fnref_targets
        .get(v_nr)
        .copied()
        .filter(|d| *d != u32::MAX)?;
    let def = data.def(callee);
    if !def.returns_borrowed_view() {
        return None;
    }
    let attrs = def.attributes();
    let dep = *def
        .returned()
        .depend()
        .iter()
        .find(|&&d| attrs.get(d as usize).is_some_and(|a| !a.hidden))?;
    let func = &data.def(d_nr).variables;
    let base = Ownership::new(data).caller_arg_base(callee, dep, args, func, &defs);
    (base != u16::MAX).then_some(base)
}

#[must_use]
pub fn callref_collection_join_base(
    data: &Data,
    d_nr: u32,
    tp: &Type,
    value: &Value,
) -> Option<u16> {
    if !crate::keys::join_own_enabled() {
        return None;
    }
    let base_tp = tp.base();
    if !(matches!(base_tp, Type::Vector(_, _)) || crate::parser::vectors::is_keyed(base_tp)) {
        return None;
    }
    if !matches!(value.unspan(), Value::CallRef(_, _)) {
        return None;
    }
    if callref_capture_blocks(data, d_nr, value) {
        return None;
    }
    match ownership_of(data, d_nr, value) {
        Own::Join { base } if base != u16::MAX => Some(base),
        // `Own::Owned` is also this arm's FALLBACK for a base the summary could not name —
        // a forwarding lambda reaches its own return through `__closure` — so the DECLARED
        // dep is asked next.  It names the same kind of witness (a visible parameter mapped
        // to the caller's argument), which is why both answers feed one identity free rather
        // than two mechanisms.
        Own::Owned => callref_declared_borrow_base(data, d_nr, value),
        _ => None,
    }
}

/// The closure-record offsets a callee's return may hand back — every `OpGetDbRef(__closure,
/// off)` in its body whose result is not consumed on the spot by an op that answers no store
/// (`OpGetInt(OpGetDbRef(__closure, 12), 0)` reads a field of a capture; it cannot hand the
/// capture back).  Empty means the return borrows no capture at all.
#[must_use]
pub fn capture_return_offsets(data: &Data, callee_d: u32) -> Vec<i32> {
    let get_dbref = data.def_nr("OpGetDbRef");
    let def = data.def(callee_d);
    let vars = def.variables();
    let closure_read = |v: &Value| -> Option<i32> {
        let Value::Call(d, args) = v.unspan() else {
            return None;
        };
        if *d != get_dbref || args.len() < 2 {
            return None;
        }
        let (Value::Var(base), Value::Int(off)) = (args[0].unspan(), args[1].unspan()) else {
            return None;
        };
        (vars.name(*base) == "__closure").then_some(*off)
    };
    fn walk(
        node: &Value,
        data: &Data,
        closure_read: &dyn Fn(&Value) -> Option<i32>,
        out: &mut Vec<i32>,
    ) {
        if let Some(off) = closure_read(node) {
            if !out.contains(&off) {
                out.push(off);
            }
            return;
        }
        if let Value::Call(d, args) = node.unspan()
            && !crate::data::is_dbref(data.def(*d).returned().base())
            && let Some(first) = args.first()
            && closure_read(first).is_some()
        {
            // A capture read consumed by a non-store op: skip it, walk the rest.
            for a in &args[1..] {
                walk(a, data, closure_read, out);
            }
            return;
        }
        node.for_each_child(&mut |c| walk(c, data, closure_read, out));
    }
    let mut out = Vec::new();
    walk(&def.code, data, &closure_read, &mut out);
    out
}

/// Must a site that would FREE this fn-ref call's result decline?  True where the callee may
/// hand back a capture and no caller variable can witness which store that is — the
/// unresolved half of `formal/closures.md` D-clo-7.  False where the return borrows no
/// capture, or where the oracle names the capture's variable (one slot, assigned once), which
/// the identity and `OpBindOrCopy` routes then treat exactly like an argument witness.
#[must_use]
pub fn callref_capture_blocks(data: &Data, d_nr: u32, call: &Value) -> bool {
    let Value::CallRef(v_nr, _) = call.unspan() else {
        return false;
    };
    if !callref_captures(data, d_nr, call) {
        return false;
    }
    let targets =
        crate::scopes::collect_fnref_targets(&data.def(d_nr).code, data.def(d_nr).variables());
    let Some(callee) = targets.get(v_nr).copied().filter(|d| *d != u32::MAX) else {
        return true;
    };
    // The callee's return summary says whether what comes back can BE a capture: its deps
    // name `__closure` when the return hands a capture's store back (a record's `??` subject,
    // a capture returned directly, a captured struct's collection field), and only hidden
    // buffers when the chosen arm was COPIED into the caller's `__retbuf` — which is how a
    // `??` over a captured collection is delivered, so nothing there is borrowed.
    let def = data.def(callee);
    let closure_attr = def
        .attributes()
        .iter()
        .position(|a| a.name == "__closure")
        .map_or(u16::MAX, |i| i as u16);
    if !def.returned().depend().contains(&closure_attr) {
        return false;
    }
    if capture_return_offsets(data, callee).is_empty() {
        return true;
    }
    !matches!(ownership_of(data, d_nr, call), Own::Join { base } if base != u16::MAX)
}

/// Does this call go through a fn-ref that captures something a RETURN COULD BORROW FROM?
///
/// The question matters wherever a site is about to decide that a returned store is the
/// caller's to free.  A capture is reached through `__closure`, a HIDDEN attribute, and two
/// otherwise-reliable readings both get it wrong: `Def::returns_borrowed_view` treats a
/// hidden-only return dep as *"the callee minted this"*, and [`protectable_ref_args`] reports
/// its witness set COMPLETE for a call whose arguments are all scalars — vacuously, because
/// there was nothing to witness.  Together those say *"owned, and fully bracketed"* about a
/// value that is neither: the store belongs to the enclosing scope, and no argument names it.
///
/// So such a fn-ref keeps the conservative answer at every such site.  It costs the leak that
/// was already there; the alternative is releasing a live variable, which is what loft#1114
/// was.
///
/// **What makes a capture dangerous is that it HOLDS A STORE**, and that is narrower than
/// having a capture at all.  The hazard above is a returned value that borrows from the
/// enclosing scope; a captured SCALAR holds no store, so nothing can be borrowed from it, and
/// declining on its account buys nothing.  Read as mere presence this leaked one store per
/// call for `m = 7; g = fn(k) -> P { P { n: m } }` — a closure over an integer returning a
/// freshly minted struct, with no discharge and nothing borrowable anywhere in it
/// (loft#1248).
///
/// The fn-ref type's deps name the CLOSURE RECORD rather than the captured variables, so the
/// question is asked one level in: the record's FIELDS are the captures, and `is_dbref` is
/// the one home for whether a field holds a store.  A dep that is not a resolvable record
/// keeps the conservative answer, because a capture that cannot be read is exactly the one
/// that must not be assumed harmless.
#[must_use]
pub fn callref_captures(data: &Data, d_nr: u32, call: &Value) -> bool {
    let Value::CallRef(v_nr, _) = call.unspan() else {
        return false;
    };
    let vars = data.def(d_nr).variables();
    let Type::Function(_, _, deps) = vars.tp(*v_nr).base() else {
        return false;
    };
    deps.iter().any(|&v| {
        let Type::Reference(clos, _) = vars.tp(v).base() else {
            return true;
        };
        data.def(*clos)
            .attributes()
            .iter()
            .any(|a| crate::data::is_dbref(a.typedef.base()))
    })
}

/// Which definition does this call reach, in EITHER spelling?
///
/// `Call(d, …)` names its callee in the node; `CallRef(v, …)` holds it in a variable, and
/// a reader that matches only the first is blind to the second with nothing to grep for.
/// That blindness is loft#1245: the heap first-bind dispatch on both backends opened with
/// `let Value::Call(..) = … else`, so a fn-ref bind never reached the copy-or-adopt split
/// and fell through to a plain adopt — it ALIASED a borrowed return (against B-Copy) and
/// left a minted one with no owner.
///
/// `None` for anything that is not a call, and for a fn-ref whose target is unresolved or
/// ambiguous — callers read that as "keep the pre-existing conservative emit".
#[must_use]
pub fn callee_of(data: &Data, d_nr: u32, value: &Value) -> Option<u32> {
    match value.unspan() {
        Value::Call(fn_nr, _) => Some(*fn_nr),
        // ⚠ A `-> τ?` fn-ref answers `None`, so every reader above keeps its pre-existing
        // emit for the nullable spelling.  `Optional(Reference)` is the same storage behind
        // a marker, but none of the machinery that handles it for a DIRECT call is wired
        // for a fn-ref: the heap first-bind dispatch matches `Reference` / `Enum(_, true)`
        // and reaches `τ?` only through `nullable_join_first_bind`, which is itself
        // `Call`-only and wants a JOIN with a nameable witness.  Admitting the nullable
        // spelling without that twin frees a store the caller still holds — measured on
        // `1114-a-nullable-heap-capture-…`'s `fn(q: P2s?) -> P2s? { q }`, which became a
        // use-after-free on the interpreter.  loft#1106's `CallRef` twin is what would
        // close it; until then the nullable spelling keeps the leak it already had.
        //
        // Asked through `Data::nullable_struct_payload`, which is the one home for the
        // question in BOTH spellings — the `Optional(Reference(S))` the author writes and
        // the `Enum(__nullable<S>, true)` the field rewrite produces (loft#1114).  Reading
        // `Optional` alone matched nothing here: by the time a return type reaches this,
        // the rewrite has already run.
        //
        // loft#1353 — the nullable spelling IS admitted where the return borrows a VISIBLE
        // argument: the reassign copy the readers emit brackets every ref argument
        // (`protectable_ref_args`, both spellings since loft#1245), so the source-free that
        // follows the copy cannot reach the caller's store, and the copy is what `(B-Copy)`
        // asks of `j = if c { hr(b) } else { d }` — a nullable record from a fn-ref chosen
        // by an `if` aliased the argument's field on the interpreter while `--native`
        // copied.  A return that borrows the CLOSURE (a captured store: the `1114` shape
        // above) is still declined — no caller variable names that store, so the bracket
        // cannot protect it and the freed-source bit would reach it.
        Value::CallRef(v, _) => fnref_target_of(
            function_defs(data, d_nr)
                .rhs
                .get(v)
                .map_or(&[], Vec::as_slice),
        )
        .filter(|d| {
            let def = data.def(*d);
            data.nullable_struct_payload(def.returned()).is_none()
                || !fnref_return_borrows_closure(def)
        }),
        _ => None,
    }
}

/// Does this lambda's return borrow its CLOSURE — a captured store no caller variable
/// names?
///
/// A return dep that names no visible parameter names the closure record
/// (`fnref_result_type` reads the same fact at the call: an index past the visible
/// arguments is the fn-ref slot's own record).  The visible parameters are the leading
/// non-hidden attributes; a hidden one (a text work buffer, the `__closure` record) is
/// not a caller-supplied store.
fn fnref_return_borrows_closure(def: &crate::data::Definition) -> bool {
    let visible = def.attributes().iter().filter(|a| !a.hidden).count();
    def.returned()
        .depend()
        .iter()
        .any(|&a| a == u16::MAX || a as usize >= visible)
}

/// See [`HeapDelivery`].
#[must_use]
pub fn heap_return_delivery(data: &Data, d_nr: u32) -> HeapDelivery {
    let def = data.def(d_nr);
    let Some(deps) = def.returned.heap_dep() else {
        return HeapDelivery::NotHeap;
    };
    if deps.is_empty() {
        return HeapDelivery::Owned;
    }
    let vars = &def.variables;
    if deps
        .iter()
        .any(|&d| d != u16::MAX && is_synth_buffer(vars.name(d)))
    {
        HeapDelivery::RetBuf
    } else {
        HeapDelivery::View
    }
}

/// @PLN104 — the loft#568 interpreter-orphan predicate, in ONE place so the promotion
/// oracle (`report_tret_promotions`) and the `--show-ownership` overlay name the same
/// class.  Returns the risk kind when a text-returning fn hands owned text back BY VALUE:
/// its return is backed FRAME-LOCALLY (`Owned`, or a `Borrowed`/`Join` of a LOCAL — not an
/// argument the caller owns and outlives the frame) AND is not already delivered through a
/// hidden `&text` retbuf.  The interpreter orphans such a return (the `String` dies with
/// the frame); native RAII drops it.  `None` = safe: already buffered, borrows an argument,
/// or not a text return.
#[must_use]
pub fn text_return_orphan_risk(data: &Data, d_nr: u32) -> Option<&'static str> {
    use crate::data::Type;
    let def = data.def(d_nr);
    if !matches!(def.returned().base(), Type::Text(_)) {
        return None;
    }
    // Only a HIDDEN `&text` buffer delivers a return.  A user-written `&text` parameter
    // is the caller's variable — counting it here left `fn f(s: &text, c) -> text { if c
    // { return mk() } … }` unbuffered, and `--native` then wrote the returned text INTO
    // `s` (loft#1338).  `text_work_buffers` is the one home for the count.
    if def.text_work_buffers() > 0 {
        return None;
    }
    let borrows_arg = |base: u16| base != u16::MAX && def.variables.is_argument(base);
    // A returned TEXT LOCAL is a view of this frame whatever bound it: a text `Set` copies
    // bytes into the local's own `String` (`OpAppendText`), so `t = text_src(i, s); return t`
    // hands up `t`'s buffer, not the argument `text_src` borrowed.  The ownership oracle
    // follows the binding to `s` — the right answer for the STORE question it exists for,
    // and one that left this local orphaned on every call (loft#1357).  A `&text` place
    // (`RefVar`) and an argument are the caller's.
    let mut returns_text_local = false;
    let vars = &def.variables;
    let is_text_local = |e: &Value| {
        matches!(e.unspan(), Value::Var(v)
            if !vars.is_argument(*v)
                && !matches!(vars.tp(*v), Type::RefVar(_))
                && matches!(vars.tp(*v).base(), Type::Text(_)))
    };
    if let Some(tail) = fn_body_tail(&def.code) {
        returns_text_local |= is_text_local(tail);
    }
    def.code.walk(&mut |v| {
        if let Value::Return(inner) = v {
            returns_text_local |= is_text_local(inner);
        }
    });
    if returns_text_local {
        return Some("view-of-local");
    }
    // The tail first, then every early `return`: each is a delivery site, and one that
    // hands back frame-local text is enough to orphan (loft#1338).  The kind named is the
    // first risky site's, which is what the promotion needs to know — it re-routes ALL of
    // them through the one buffer.
    let mut own = Ownership::new(data);
    let tail = own.return_ownership(d_nr);
    std::iter::once(tail)
        .chain(own.early_return_ownerships(d_nr))
        .find_map(|o| match o {
            Own::Owned => Some("owned-by-value"),
            Own::Borrowed { base } if !borrows_arg(base) => Some("view-of-local"),
            Own::Join { base } if !borrows_arg(base) => Some("join-of-local"),
            _ => None,
        })
}

/// Public, test-facing entry: the owned-slot reassignment sites of function `d_nr`.
#[must_use]
pub fn reassign_sites(data: &Data, d_nr: u32) -> Vec<ReassignSite> {
    Ownership::new(data).reassign_sites(d_nr)
}

/// The `local_source` over-free fix's input: the heap slots that hold an OWNED
/// store displaced by a later `Borrowed`/`Join` reassignment (`prior=Owned`). The
/// scope pass strips these slots' deps so the owned path deep-copies + frees them
/// (the displaced store would otherwise be orphaned — the `chosen = dflt(); … chosen
/// = pool[wj]` leak). Operates on the pre-scope `(code, function)` directly.
#[must_use]
pub fn displaced_owned_slots(code: &Value, function: &Function, data: &Data) -> HashSet<u16> {
    Ownership::new(data)
        .reassign_sites_of(code, function)
        .into_iter()
        .filter(|s| {
            matches!(s.prior, Own::Owned)
                && matches!(s.rhs, Own::Borrowed { .. } | Own::Join { .. })
                // A retbuf-promoted PARAM slot is NOT this fix's territory: the
                // dep-strip would disable the dep-carrying explicit reassign-free
                // (the p462 `_rb_w_` witness partner) while codegen's dep-empty
                // pre-Set free excludes arguments — the displaced first store
                // then leaks one per call (p462_cond_reassign_retbuf under
                // gate-ON, M×N).  Param-slot displaced frees stay with the
                // witness mechanism.
                && !function.is_argument(s.var)
        })
        .map(|s| s.var)
        .collect()
}

/// Public, test-facing entry: the over-free free SITES of function `d_nr` (the
/// append element source-frees + return-buffer-aliasing deliveries, with class +
/// borrow base) — the Gap-A/B context the Stage-3 fix reads.
#[must_use]
pub fn free_sites(data: &Data, d_nr: u32) -> Vec<FreeSite> {
    Ownership::new(data).free_sites(d_nr)
}

/// THE own-vs-borrow oracle: the ownership of `value` as produced in function
/// `d_nr`, with the borrow `base` resolved (interprocedurally for a call — the
/// callee's borrowed param mapped to the caller's argument). This is the ONE fact
/// every own-vs-borrow chokepoint READS instead of re-deriving — the unification
/// entry point (the OWNERSHIP_MODEL north star).
#[must_use]
/// Answers @FR-O-Owner for one value: which single thing owns this store.  Every heap store
/// has exactly one owner at any moment, and this is where that is decided.
///
/// This IS @FR-O-Oracle — the one own-vs-borrow derivation, taken from the IR (a store mint
/// is `Owned`, a projection is `Borrowed(base)`, a call resolves through the callee's return
/// summary).  ⚠ It does NOT read `deps`: the dep list is a separate, cheaper stand-in for
/// the same question (@FR-O-Proxy) that is unsound alone.  A chokepoint should read here.
pub fn ownership_of(data: &Data, d_nr: u32, value: &Value) -> Own {
    ownership_of_with(data, d_nr, value, &function_defs(data, d_nr))
}

/// The whole-function half of [`ownership_of`]: every var's defining right-hand
/// sides, the `OpDatabase` vars, and the vars a branch fills in place.
///
/// Split out because it depends on the FUNCTION, not on the value being asked
/// about, while `ownership_of` recomputes it per question. That is quadratic
/// wherever one function is asked many times — loft#854: a vector literal is one
/// `Set` per element, `scopes::scan_set` asks about each, and each answer walked
/// (and CLONED the right-hand side of) the whole function. 86 400 elements took
/// over 13 minutes at 99 % CPU, reading as a hang.
///
/// A caller that asks repeatedly about ONE function computes this once and passes
/// it to [`ownership_of_with`]. It is deliberately not cached on `Data`: the
/// result is a function of `Definition::code`, which the scope pass REWRITES
/// (`scopes.rs` assigns `definitions[d_nr].code` at four points), so a cache
/// living as long as `Data` would answer from a body that no longer exists —
/// silently, and in the direction that mis-classifies ownership. The memo belongs
/// where a `&Data` borrow already proves the body cannot change underneath it.
#[must_use]
pub(crate) fn function_defs(data: &Data, d_nr: u32) -> Defs {
    let mut defs = Defs::default();
    let def = data.def(d_nr);
    collect_defs(&def.code, &FillOps::of(data), &mut defs);
    defs.fnref_targets = crate::scopes::collect_fnref_targets(&def.code, &def.variables);
    defs.fnref_captures = crate::scopes::collect_fnref_captures(&def.code, &def.variables, data);
    defs.multi_assigned = crate::scopes::multi_assigned_in(&def.code);
    defs
}

/// [`ownership_of`] against an already-computed [`function_defs`] for `d_nr`.
///
/// The caller owns the obligation the borrow cannot express: `defs` must be the
/// defs of THIS `d_nr`, collected from the body `data` holds now.
#[must_use]
pub(crate) fn ownership_of_with(data: &Data, d_nr: u32, value: &Value, defs: &Defs) -> Own {
    let def = data.def(d_nr);
    Ownership::new(data).classify(value, &def.variables, defs)
}

/// True when `classify` resolves a `Call(d, …)` STRUCTURALLY — a store mint
/// (`OpDatabase`/`OpNewRecord` → `Owned`) or a projection (`OpGetField` &c → the
/// `Borrowed(base)` view into arg 0) — rather than through the callee's
/// interprocedural return summary (`call_ownership`). The @PLN94 oracle's transfer
/// routes only NON-structural (genuine user/native) calls through its independent
/// `call_own`; these primitives carry fixed ownership semantics both analyses share,
/// so it must delegate them to `ownership_of` instead of the summary path (else a
/// projection local is mis-classed `Owned` — the over-free/unsound direction).
#[must_use]
pub fn classifies_structurally(data: &Data, d: u32) -> bool {
    d == data.def_nr("OpDatabase")
        || d == data.def_nr("OpNewRecord")
        || data.op_sets().projections.contains(&d)
}

/// Does this value hold NO STORE at all — a `null` literal, or the reference null sentinel
/// `Parser::convert` lowers it to in a reference-typed position?  Such a value neither needs
/// protecting nor can be borrowed from, and in a join it is the identity: the other arm's
/// class is the pair's.  The one spelling of that fact for the classifier, the argument
/// witness and the view-root walk.
#[must_use]
pub(crate) fn holds_no_store(data: &Data, value: &Value) -> bool {
    match value.unspan() {
        Value::Null => true,
        Value::Call(d, args) => args.is_empty() && data.def(*d).name() == "OpNullRefSentinel",
        _ => false,
    }
}

/// The value a two-arm `if` DELIVERS when one arm holds no store: the other arm.  A tagged
/// nullable slot is read as `if <present> { <payload projection> } else { nullref }`
/// (`Parser::emit_nullable_slot_read`), and for every question about ownership — does this
/// bind a view, what does it own, may it be freed at a rebind — the pair answers as its
/// present arm does.  Any other value answers as itself, so a caller can read through
/// unconditionally.
#[must_use]
pub(crate) fn through_null_arm<'v>(data: &Data, value: &'v Value) -> &'v Value {
    if let Value::If(_, then, els) = value.unspan() {
        if holds_no_store(data, els) {
            return through_null_arm(data, then);
        }
        if holds_no_store(data, then) {
            return through_null_arm(data, els);
        }
    }
    value
}

/// The bare verdict name (no base) — for the free-site dump's `class=` field.
fn own_kind(own: Own) -> &'static str {
    match own {
        Own::Owned => "Owned",
        Own::Borrowed { .. } => "Borrowed",
        Own::Join { .. } => "Join",
    }
}

/// A readable `Owned` / `Borrowed(base=<name>)` / `Join(base=<name>)`, resolving the
/// base var to its name in `func`'s space (`?` when unresolved).
pub fn fmt_own(own: Own, func: &Function) -> String {
    let base = |b: u16| {
        if b == u16::MAX {
            "?".to_string()
        } else {
            func.name(b).to_string()
        }
    };
    match own {
        Own::Owned => "Owned".to_string(),
        Own::Borrowed { base: b } => format!("Borrowed(base={})", base(b)),
        Own::Join { base: b } => format!("Join(base={})", base(b)),
    }
}

/// @PLN103 P1.4 — is `name` a SYNTHESIZED owned buffer (a var whose store the var
/// itself owns: the vector-delivery / NRVO / materialise-copy / return buffers)?
/// A `Borrowed` verdict whose base is one of these (or the var itself) is really an
/// OWNED store held via that buffer, not an alias of a live sibling (P0.2 finding).
#[must_use]
pub fn is_synth_buffer(name: &str) -> bool {
    name.starts_with("__vdb")
        || name.starts_with("__ref")
        || name.starts_with("_mvcopy")
        || name == "__retbuf"
}

/// @PLN107 value-struct completeness — is `ty` a `value struct` local (a `Type::Reference`
/// whose record is marked in `Data.value_structs`)? Such a local always OWNS its store: a
/// value struct read out of a field/element is COPIED (§ `scopes::value_struct_copy`), never
/// aliased, so a never-read mutation of it is a lost write. A plain reference struct is also a
/// `Type::Reference` but NOT value-marked → aliases → not owned here. `&value struct` is a
/// `RefVar` (excluded upstream), so only the bare value-struct binding reaches this.
fn is_value_struct_local(ty: &crate::data::Type, data: &Data) -> bool {
    matches!(ty, crate::data::Type::Reference(d, _) if data.is_value_struct(*d))
}

/// Does another variable resolve to the same synthesized backing buffer as `v`?
///
/// A synth base normally reads as "owns its store, held via that delivery buffer": the
/// buffer was minted to back `v` alone, so `v` is a private copy and a write into it is
/// lost.  `w = &v` on a local vector mints nothing — `w` takes `v`'s existing buffer — so
/// BOTH resolve to it and the write through `w` reaches `v`.  Sharing is what separates
/// the two, and it is the same fact `--show-ownership` renders when two rows come back
/// `Owned (backing=__vdb_1)`.
///
/// Without this the dead-store lint fired on `w = &v` — the write-through cure its own
/// message recommends — and a warning gates a library's CI (loft#670).
fn is_shared_backing(data: &Data, d_nr: u32, func: &Function, v: u16, base: u16) -> bool {
    (0..func.var_count())
        .map(|u| u as u16)
        .filter(|&u| u != v && u != base)
        .any(|u| {
            matches!(
                ownership_of(data, d_nr, &Value::Var(u)),
                Own::Borrowed { base: b } | Own::Join { base: b } if b == base
            )
        })
}

/// @PLN103 P1.4 — render `own` for the ownership overlay (the P0.2 rendering rule,
/// corrected in P1): `ownership_of` reports a bare argument as `Borrowed{base=self}`
/// (self-base) and an owned delivery buffer as `Borrowed{base=<buffer>}`, neither of
/// which is a dangerous alias — so translate them:
/// - **self-base** (`base == v`): the var is an argument with no local def → it borrows
///   the CALLER's value → `Borrowed(caller-arg)`.
/// - **synthesized-buffer base** (`base != v`, [`is_synth_buffer`]): the var OWNS its
///   store, held via that delivery buffer → `Owned (backing=<name>)`.
/// - any other `Borrowed{base=X}` → a genuine ALIAS of a live sibling `X` (the dangerous
///   case) → `Borrowed(base=<name>)`.
///
/// `Join` is NEVER translated (a real runtime split). RENDERER-ONLY — `ownership_of`
/// stays faithful (Out-of-scope §1); `v` is the var being classified (for self-base).
#[must_use]
pub fn render_own(own: Own, func: &Function, v: u16) -> String {
    let name = |b: u16| {
        if b == u16::MAX {
            "?".to_string()
        } else {
            func.name(b).to_string()
        }
    };
    match own {
        Own::Owned => "Owned".to_string(),
        Own::Borrowed { base: b } if b == v => "Borrowed(caller-arg)".to_string(),
        Own::Borrowed { base: b } if b != u16::MAX && is_synth_buffer(func.name(b)) => {
            format!("Owned (backing={})", name(b))
        }
        Own::Borrowed { base: b } => format!("Borrowed(base={})", name(b)),
        Own::Join { base: b } => format!("Join(base={})", name(b)),
    }
}

/// Print every function's verdicts when `LOFT_MATERIALIZE_DUMP` is set. Called from
/// `scopes::check`; a no-op otherwise. Behaviour-neutral — diagnostics only.
pub fn dump_all(data: &Data) {
    if std::env::var_os("LOFT_MATERIALIZE_DUMP").is_none() {
        return;
    }
    let mut own = Ownership::new(data);
    // @PLN90 — the tally: `avoidable` is the north-star elimination worklist; `implicit`
    // is model-inherent ownership (silent, not a copy-to-fix); `forced` is informational;
    // `internal` is a copy of a compiler-generated source — a developer-worklist copy we may
    // eliminate, but excluded from the user-facing report (item 1).
    let (mut avoidable_copies, mut implicit_copies, mut forced_copies, mut internal_copies) =
        (0u32, 0u32, 0u32, 0u32);
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            continue;
        }
        for r in analyze_fn(&def.code, &def.variables, data, env_tier()).0 {
            let bucket = match r.class {
                CopyClass::Eliminated => "eliminated",
                CopyClass::Avoidable => {
                    avoidable_copies += 1;
                    "AVOIDABLE"
                }
                CopyClass::Implicit => {
                    implicit_copies += 1;
                    "implicit"
                }
                CopyClass::Forced => {
                    forced_copies += 1;
                    "forced"
                }
                CopyClass::Internal => {
                    internal_copies += 1;
                    "internal"
                }
            };
            // @PLN90 item 2 — the copy-site location, when the emitting op carried a span.
            // Makes a `<record>` element-set copy (no named target) actionable.
            let at = r
                .loc
                .as_ref()
                .map_or_else(String::new, |p| format!(" at {p}"));
            eprintln!(
                "MAT fn={} v={}({}) src={} verdict={:?} bucket={} [{}]{}",
                def.name, r.var_nr, r.var_name, r.source, r.verdict, bucket, r.reason, at
            );
        }
        // The ownership fact (inert): the return class + the owned-slot
        // reassignments, each with its borrow base. The over-free leak shape is a
        // `prior=Owned rhs=Join(...)` row.
        let fvars = &def.variables;
        eprintln!(
            "OWN fn={} return={}",
            def.name,
            fmt_own(own.return_ownership(d_nr), fvars)
        );
        for s in own.reassign_sites(d_nr) {
            eprintln!(
                "OWN fn={} reassign v={}({}) prior={} rhs={}",
                def.name,
                s.var,
                s.var_name,
                fmt_own(s.prior, fvars),
                fmt_own(s.rhs, fvars)
            );
        }
        // The free SITES (Gap A) + borrow base (Gap B), now interprocedurally
        // resolved (a call source's base is the CALLER's argument), still inert.
        for s in own.free_sites(d_nr) {
            eprintln!(
                "OWN fn={} free kind={:?} slot={}({}) class={} base={}",
                def.name,
                s.kind,
                s.slot,
                s.slot_name,
                own_kind(s.class),
                s.base_name.as_deref().unwrap_or("-")
            );
        }
    }
    // @PLN90 — the worklist headline: avoidable = the north-star target (teach the analysis
    // to borrow these so they auto-eliminate); implicit = model-inherent ownership (silent);
    // forced = must own by circumstance (informational); internal = compiler-generated source
    // (developer worklist, excluded from the user-facing report).
    eprintln!(
        "MAT-WORKLIST avoidable_copies={avoidable_copies} implicit_copies={implicit_copies} forced_copies={forced_copies} internal_copies={internal_copies}"
    );
}

/// @PLN107 S4a — the ENFORCED dead-store lint (`LOFT_DEAD_STORES`). Warns when a non-escaping
/// local is mutated via an `OpSet*` (`d[i]=x`, `d.f=x`) yet its value is never read AND it OWNS
/// its store — the copy-mutate footgun (`d = self.data; d[i]=x` COPIES, so the write is lost).
///
/// Runs POST-`scopes::check` (called from `main` after the program loads) so the copy-idiom /
/// value-struct-copy rewrites are in place and [`ownership_of`] is reliable: a `Borrowed`/`Join`
/// var aliases a source it borrows (a `&`-reference, a reference-struct field alias, a
/// vector-element view), so its write PROPAGATES and is NOT a dead store — only `Owned` copies
/// warn. The read/write split is [`dead_store_accesses`]; exclusions mirror `test_used`
/// (`_`/`#` temporaries, arguments, closure-captured, global shadows). The `reads==0 &&
/// write_targets>0` signal implies `uses>0`, so this lint and `test_used` (`uses==0`) are
/// disjoint. Populates `diags`; the caller renders them. Sibling of [`warn_copies`].
/// loft#985 — the post-scope-check lint family, in ONE place so every path that loads a
/// program runs the same set.
///
/// These five share a precondition — they read the ownership verdicts and the materialised
/// copies that only exist once `scopes::check` has run — so they cannot live with the
/// parse-time diagnostics, and they had come to live in `main.rs`'s program path alone.
/// `loft test` / `--tests`, which is the path a LIBRARY's CI takes, ran none of them: a
/// library could ship a `#superseded` steer pointing at nothing (a hard ERROR on the
/// program path) and writes that land in a copy, with a green suite. That is exactly the
/// hole @PLN107's lint was written for — its motivating case is a published `graphics`
/// canvas whose every drawing primitive was a no-op through the copy-mutate shape.
///
/// Call it ONCE per loaded program, after `scopes::check`. Not once per test: each test
/// compiles its own bytecode from the same `Data`, so a per-test call would report every
/// finding N times.
///
/// The error gate is part of the fact, not the caller's business (loft#883): every lint
/// below reads RESOLVED types, and an aborting error means resolution did not finish — an
/// unresolved type carries empty deps, `ownership_of` reads empty deps as OWNED, and a
/// borrowing `for` variable in an unrelated library then reads as a lost write. The gate is
/// the whole set rather than the lint that was reported, because they share the
/// precondition.
pub fn post_scope_lints(
    data: &Data,
    diags: &mut crate::diagnostics::Diagnostics,
    fallback_file: &str,
) {
    if diags.level() >= crate::diagnostics::Level::Error {
        return;
    }
    // @PLN90 W5 — the enforced copy lint (gated `LOFT_WARN_COPIES`).
    warn_copies(data, diags, fallback_file);
    // @PLN107 S4a — the dead-store / lost-write lint (gated `LOFT_DEAD_STORES`).
    warn_dead_stores(data, diags, fallback_file);
    // @PLN139 stage G — the double-move lint.
    warn_double_move(data, diags, fallback_file);
    // loft#894 — the lost-temporary-write lint.
    warn_lost_temp_writes(data, diags, fallback_file);
    // @PLN102 arc C step 4 — the `#superseded` fold lint, including its hard ERROR for a
    // steer whose successor does not resolve.
    superseded_fold_diagnostics(data, diags, fallback_file);
    warn_linked_group_append(data, diags, fallback_file);
    // loft#1397 — a payload binding whose subject's place is overwritten with another variant.
    warn_variant_overwritten(data, diags, fallback_file);
}

/// loft#1397 — a `match` / `is` PAYLOAD binding whose subject's PLACE is overwritten with a
/// DIFFERENT variant while the binding is still read.
///
/// `formal/binding.md` `(B-Disturb)` is explicit that overwriting a place is NOT disturbing
/// it — *"`o.inner = Box{…}` writes INTO the place `o.inner` already occupies, so a view of
/// it survives"* — so the binding still names the payload slot and reads what is there now.
/// Both backends agree, and the VALUE is what the rules give.  Nothing here is a lifetime
/// defect, and the rules do not change to match the code.
///
/// What is missing is the DIAGNOSTIC.  loft#980's `variant-field-unchecked` exists for exactly
/// this hazard — a field only some variants declare, read at a tag the program did not check —
/// and it is deliberately quiet for `match` / `is` bindings, because those are per-arm and are
/// the cure it names.  That exemption assumes the variant cannot change under the binding.  It
/// can: the arm is entered as `Holder`, the subject's place is overwritten with `Empty`, and
/// the per-arm binding keeps reading `Empty`'s bytes at `Holder`'s offsets.
///
/// Keyed on the ARM's own tag test rather than on a variant's discriminant derived here, so
/// the lint cannot drift from the numbering the parser emits: the condition names the place
/// and the tag in one node, and the overwrite is an `OpSetEnum` on that same place with a
/// different one.
///
/// A LOCAL subject never reaches this.  `sh = Empty{…}` builds into a work-ref and repoints
/// the local, so its `OpSetEnum` names a different place — and that shape is a REASSIGNMENT,
/// which `(B-View)` already materialises and warns about.  Only a place the overwrite clause
/// covers gets here, which is exactly the case that is correct-but-unwarned.
///
/// `warning`, not advice, by the tier rule: the value read is another variant's, typed as this
/// one's, which is the same wrong result loft#980 reports.
pub fn warn_variant_overwritten(
    data: &Data,
    diags: &mut crate::diagnostics::Diagnostics,
    fallback_file: &str,
) {
    if !crate::keys::variant_overwritten_enabled() {
        return;
    }
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            continue;
        }
        // The DEFINITION's file, for the reason `warn_lost_temp_writes` gives: a warning gates
        // a library's CI, so a dependency's line must not be paired with the consumer's path.
        let file = if def.position.file.is_empty() {
            fallback_file
        } else {
            def.position.file.as_str()
        };
        let mut hits: Vec<(u16, u16, u32, u32)> = Vec::new();
        find_variant_arms(data, &def.code, &mut |place, tag, arm| {
            let mut w = VariantWatch {
                data,
                function: &def.variables,
                place,
                tag,
                bound: HashSet::new(),
                stale: false,
                at: def.position.clone(),
                hits: Vec::new(),
            };
            w.walk(arm);
            for (b, at) in w.hits {
                hits.push((b, place.0, at.line, at.pos));
            }
        });
        hits.sort_unstable();
        hits.dedup_by_key(|(b, _, _, _)| *b);
        for (binding, base, line, col) in hits {
            let function = &def.variables;
            let name = function.name(binding).to_string();
            let field = name
                .trim_start_matches("_mv_")
                .rsplit_once('_')
                .map_or_else(|| name.clone(), |(n, _)| n.to_string());
            let subject = function.name(base).to_string();
            let msg = format!(
                "`{field}` names the payload of the variant this arm matched, and `{subject}` \
                 is given a DIFFERENT variant while `{field}` is still read \u{2014} the binding \
                 keeps naming the same slot, so this reads the new variant's bytes at \
                 `{field}`'s offset. Copy the payload out before the write (`x = {field};`) \
                 and read `x`"
            );
            diags.add_at_coded(
                crate::diagnostics::Level::Warning,
                Some("variant-overwritten-binding"),
                &msg,
                file,
                line,
                col,
            );
            // A diagnostic says what is wrong; the fix says what to write instead, and that is
            // the half a reader acts on.  CONDITIONAL, and the condition is real rather than a
            // formality: the cure depends on which value the author wanted.  Copying the
            // payload out keeps the one the arm MATCHED; an author who meant to read what the
            // subject holds now wants the opposite edit — re-enter the match after the write —
            // and no tool can tell those apart from the source.
            diags.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: format!("copy the payload out first: `x = {field};` then read `x`"),
                condition: Some(format!(
                    "if `{field}` is meant to be the payload this arm matched, rather than \
                     whatever `{subject}` holds after the write"
                )),
                edit: None,
                concept: "pattern matching",
                concept_ref: "@F29",
            });
        }
    }
}

/// Call `f` for every variant-check `if` — the lowering a `match` arm and an `is` both take —
/// with the PLACE its tag is read from, the tag it tests, and the arm it guards.
fn find_variant_arms(data: &Data, node: &Value, f: &mut impl FnMut((u16, u32), i32, &Value)) {
    if let Value::If(cond, then_arm, _) = node.unspan()
        && let Some((place, tag)) = variant_check(data, cond)
    {
        f(place, tag, then_arm);
    }
    node.unspan()
        .for_each_child(&mut |c| find_variant_arms(data, c, f));
}

/// The place and tag a variant-check condition reads — `OpEqInt(OpConvIntFromEnum(OpGetEnum(P,
/// 0)), N)`, and the non-struct spelling without the `OpGetEnum`.
///
/// An `is` wraps its condition in an `Insert` that first stabilises the subject into a local,
/// so the tail is taken rather than the whole node.
fn variant_check(data: &Data, cond: &Value) -> Option<((u16, u32), i32)> {
    let tail = match cond.unspan() {
        Value::Block(bl) | Value::Loop(bl) => bl.operators.last()?.unspan(),
        // The `is` spelling stabilises its subject into a local first and yields the test as
        // the LAST item of an `Insert`, where `match` hands the test bare.
        Value::Insert(items) => items.last()?.unspan(),
        other => other,
    };
    let Value::Call(d, args) = tail else {
        return None;
    };
    if data.def(*d).name() != "OpEqInt" || args.len() != 2 {
        return None;
    }
    let Value::Int(tag) = args[1].unspan() else {
        return None;
    };
    let Value::Call(c, cargs) = args[0].unspan() else {
        return None;
    };
    if data.def(*c).name() != "OpConvIntFromEnum" {
        return None;
    }
    let subject = match cargs.first()?.unspan() {
        Value::Call(g, gargs) if data.def(*g).name() == "OpGetEnum" => gargs.first()?,
        other => other,
    };
    Some((projection_container_place(data, subject)?, *tag))
}

/// The ordered walk of one variant arm: what is BOUND, when the subject's place stops holding
/// the variant the arm matched, and which bindings are read after that.
struct VariantWatch<'a> {
    data: &'a Data,
    function: &'a crate::variables::Function,
    place: (u16, u32),
    tag: i32,
    bound: HashSet<u16>,
    stale: bool,
    /// The nearest source position seen, so the report lands on the READ rather than on the
    /// function — a warning a reader cannot locate is a warning they cannot act on.
    at: crate::lexer::Position,
    hits: Vec<(u16, crate::lexer::Position)>,
}

impl VariantWatch<'_> {
    fn walk(&mut self, node: &Value) {
        let outer = self.at.clone();
        if let Some(p) = node.span_pos() {
            self.at = p.clone();
        }
        self.walk_inner(node);
        self.at = outer;
    }

    fn walk_inner(&mut self, node: &Value) {
        match node.unspan() {
            // The value runs before the target is written, so a read inside it is a read
            // BEFORE this binding exists.
            Value::Set(v, rhs) => {
                self.walk(rhs);
                // A PAYLOAD binding is named by the parser (`_mv_<field>_N`).  The side
                // table that records which field each one projects (`mv_field_origin`) is
                // cleared when the two passes are joined, so the name is the fact that
                // survives to here — the same one `Variables::is_overwritten_view` reads.
                if self.function.name(*v).starts_with("_mv_") {
                    // Re-bound after the overwrite: it names the new variant's payload, which
                    // is what the program asked for.
                    self.bound.insert(*v);
                    self.hits.retain(|(h, _)| h != v);
                }
                return;
            }
            Value::Var(v) if self.stale && self.bound.contains(v) => {
                self.hits.push((*v, self.at.clone()));
            }
            Value::Call(d, args) if self.data.def(*d).name() == "OpSetEnum" => {
                if let (Some(p), Some(Value::Enum(m, _))) = (
                    args.first()
                        .and_then(|a| projection_container_place(self.data, a)),
                    args.get(2).map(Value::unspan),
                ) && p == self.place
                    && i32::from(*m) != self.tag
                {
                    self.stale = true;
                }
            }
            _ => {}
        }
        node.unspan().for_each_child(&mut |c| self.walk(c));
    }
}

/// Advise when two members of one linked collection GROUP are appended to in the same block
/// (loft#1227).
///
/// The literal spelling is advised in the parser (`Parser::advise_linked_group_fill`); this is
/// the SAME advice for the same modelling mistake reached by a different spelling, so it reuses
/// that diagnostic code — one mistake, two spellings, and a code is a frozen public surface.
///
/// Two collections over one element type are two routes to a SINGLE record set. Before
/// loft#1221 the bare-variable form at a keyed field was silently DROPPED, so the most natural
/// spelling was inert; now it appends, and the idiom went from a silent no-op to a silent
/// doubling.
///
/// **Scoped per BLOCK, and deliberately an UNDER-approximation.** One rule answers the three
/// questions loft#1227 records as open design work — and they are the three
/// `LOFT_NO_DOUBLE_MOVE` already answers, so this follows that precedent rather than deciding
/// again: filling member A in one arm and B in the other is not a double fill (separate arms
/// are separate blocks); a loop body counts once however often it runs (it is one block); and
/// two fills separated by a branch are not paired. Under-approximating costs a missed report
/// and nothing else — this is `advice`, which never gates a build — while over-approximating
/// puts noise on correct code, the failure the ADVICE tier exists to avoid.
pub fn warn_linked_group_append(
    data: &Data,
    diags: &mut crate::diagnostics::Diagnostics,
    fallback_file: &str,
) {
    if !crate::keys::linked_group_lint_enabled() {
        return;
    }
    let new_rec = data.def_nr("OpNewRecord");
    if new_rec == u32::MAX {
        return;
    }
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            continue;
        }
        let def_file = if def.position.file.is_empty() {
            fallback_file
        } else {
            def.position.file.as_str()
        };
        let mut cx = GroupAppends {
            data,
            func: &def.variables,
            new_rec,
            cur: None,
            file: def_file,
        };
        cx.scan_block(&def.code, diags);
    }
}

/// One block's group appends — see [`warn_linked_group_append`].
struct GroupAppends<'a> {
    data: &'a Data,
    func: &'a Function,
    new_rec: u32,
    /// The nearest enclosing line marker, which is where a report lands.
    cur: Option<Position>,
    file: &'a str,
}

impl GroupAppends<'_> {
    fn scan_block(&mut self, node: &Value, diags: &mut crate::diagnostics::Diagnostics) {
        let mut here: Vec<(u16, usize, Position)> = Vec::new();
        let saved = self.cur.clone();
        self.collect(node, true, &mut here);
        self.report(&here, diags);
        self.cur = saved;
        let mut nested: Vec<Value> = Vec::new();
        Self::nested_blocks(node, true, &mut nested);
        for b in &nested {
            self.scan_block(b, diags);
        }
    }
    fn collect(&mut self, node: &Value, top: bool, out: &mut Vec<(u16, usize, Position)>) {
        if let Some(p) = node.span_pos() {
            self.cur = Some(p.clone());
        } else if let Value::Line(n) = node {
            // BOTH carriers. `Span` alone silently degrades every report to the enclosing
            // function's line, because a statement's position rides a `Line` marker —
            // `DoubleMove::scan` reads both for the same reason.
            self.cur = Some(Position {
                file: String::new(),
                line: *n,
                pos: 0,
            });
        }
        if !top && matches!(node.unspan(), Value::Block(_)) {
            return;
        }
        if let Value::Call(d, args) = node.unspan()
            && *d == self.new_rec
            && let Some(Value::Var(v)) = args.first().map(Value::unspan)
            && let Some(Value::Int(fld)) = args.get(2).map(Value::unspan)
            && *fld >= 0
            && *fld != i32::from(u16::MAX)
            && let Some(at) = self.cur.clone()
        {
            out.push((*v, *fld as usize, at));
        }
        node.unspan()
            .for_each_child(&mut |c| self.collect(c, false, out));
    }
    fn nested_blocks(node: &Value, top: bool, out: &mut Vec<Value>) {
        if !top && matches!(node.unspan(), Value::Block(_)) {
            out.push(node.clone());
            return;
        }
        node.unspan()
            .for_each_child(&mut |c| Self::nested_blocks(c, false, out));
    }
    fn report(&self, here: &[(u16, usize, Position)], diags: &mut crate::diagnostics::Diagnostics) {
        let mut vars: Vec<u16> = here.iter().map(|(v, _, _)| *v).collect();
        vars.sort_unstable();
        vars.dedup();
        for v in vars {
            if v as usize >= self.func.var_count() {
                continue;
            }
            // `.base()` — a holder spelled `S?` has the same fields and the same groups, so
            // the advice is the same. Matched bare, a nullable holder is the one spelling this
            // cannot see; the `optional` screen named it on this code's first run.
            let Type::Reference(td, _) = self.func.tp(v).base() else {
                continue;
            };
            for (_, members) in &crate::parser::objects::collection_groups_of(self.data, *td) {
                let filled: Vec<&String> = members
                    .iter()
                    .filter(|m| here.iter().any(|(hv, f, _)| *hv == v && *f == m.a_nr))
                    .map(|m| &m.name)
                    .collect();
                if filled.len() < 2 {
                    continue;
                }
                let Some(at) = here
                    .iter()
                    .filter(|(hv, f, _)| *hv == v && members.iter().any(|m| m.a_nr == *f))
                    .map(|(_, _, p)| p)
                    .max_by_key(|p| (p.line, p.pos))
                else {
                    continue;
                };
                let file = if at.file.is_empty() {
                    self.file
                } else {
                    at.file.as_str()
                };
                let holder = filled[0];
                let others = filled[1..]
                    .iter()
                    .map(|nm| format!("`{nm}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let route = if filled.len() == 2 {
                    "is a second route"
                } else {
                    "are second routes"
                };
                let msg = format!(
                    "{others} {route} to `{holder}`'s records, not collections of their own — \
                     this block appends to both, so one record set ends up holding everything \
                     they were given"
                );
                diags.add_at_coded(
                    crate::diagnostics::Level::Advice,
                    Some("linked-group-double-fill"),
                    &msg,
                    file,
                    at.line,
                    at.pos,
                );
            }
        }
    }
}

pub fn warn_dead_stores(
    data: &Data,
    diags: &mut crate::diagnostics::Diagnostics,
    fallback_file: &str,
) {
    if !crate::keys::dead_stores_enabled() {
        return;
    }
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            continue;
        }
        // `var_source` gives a line in the file THIS definition was parsed from, so the
        // file must come from the definition too — the same pairing that put the copy
        // notice on the consumer's entry file (loft#781). It bites harder here: this is a
        // `warning`, and a warning gates a library's CI, so a dependency's dead store
        // would fail a consumer that cannot see the line it names.
        let def_file = if def.position.file.is_empty() {
            fallback_file
        } else {
            def.position.file.as_str()
        };
        let func = &def.variables;
        let n = func.var_count();
        let acc = dead_store_accesses(&def.code, n, data);
        for i in 0..n {
            let v = i as u16;
            let name = func.name(v);
            if name.starts_with('_')
                || name.contains('#')
                || func.is_argument(v)
                || func.is_captured(v)
                || data.def_nr(name) != u32::MAX
            {
                continue;
            }
            // A `&`-reference (`RefVar`) explicitly aliases its target; a write through it always
            // propagates, so never a dead store — exclude regardless of the ownership verdict.
            if matches!(func.tp(v), crate::data::Type::RefVar(_)) {
                continue;
            }
            let (reads, write_targets) = acc.get(i).copied().unwrap_or((0, 0));
            // The dead-store signal: mutated via an `OpSet*` (write_targets>0) but never read
            // (reads==0). A var mutated this way was read as the write BASE at parse, so `uses`
            // was >0 there and `test_used` (uses==0) stayed silent — the two lints are disjoint
            // structurally. (`uses` is not maintained in the post-scopes `def.variables`.)
            if !(reads == 0 && write_targets > 0) {
                continue;
            }
            // A dead store requires the var to OWN its store (a copy). A Borrowed/Join var
            // aliases a source it borrows, so the write PROPAGATES — not dead. Exception: the
            // copy idiom builds a vector copy as `OpGetField(__vdb, …)`, so `ownership_of` reports
            // `Borrowed{base=__vdb}` for a genuinely-owned copy — a base that is a SYNTH buffer
            // (or the var itself) means owned-via-buffer, not an alias of a live sibling
            // (`is_synth_buffer`, the P0.2 finding shared with `report_copies`).
            //
            // @PLN107 value-struct completeness: a `value struct` local ALWAYS owns its store —
            // reading a value struct out of a field/element COPIES it (the `value_struct_copy`
            // pass materialises a fresh `OpCopyRecord`; DESIGN @PLN101), so `m = w.field; m.x = 9`
            // (m unread) is a lost write, the same footgun as the vector copy. `ownership_of`
            // conservatively reports `Borrowed{source-view}` there, so without this explicit
            // value-struct test it is a false NEGATIVE. A plain reference `struct` ALIASES (the
            // write propagates) and correctly stays `Borrowed` → silent; `&value struct` is a
            // `RefVar`, already excluded above.
            let owns = is_value_struct_local(func.tp(v), data)
                || match ownership_of(data, d_nr, &Value::Var(v)) {
                    Own::Owned => true,
                    Own::Borrowed { base } | Own::Join { base } => {
                        base == v
                            || (base != u16::MAX
                                && is_synth_buffer(func.name(base))
                                && !is_shared_backing(data, d_nr, func, v, base))
                    }
                };
            if !owns {
                continue;
            }
            let (line, col) = func.var_source(v);
            // @PLN102 arc-C steer (alias-where-correct.md step 6): the write-through-intent case.
            // A copy mutated then discarded has no correct copy meaning, so point straight at the
            // explicit fix — `&` for write-through — and offer the read-back alternative if a copy
            // really was intended. (The write-through is ALWAYS the programmer's explicit `&`, never
            // inferred — see alias-where-correct.md: copy is the semantics, links are explicit.)
            let msg = format!(
                "'{name}' is mutated but its value is never read — the write is LOST. A whole-value \
                 bind (`{name} = …`) COPIES the heap value (C86), so the mutation lands in the copy, \
                 not the source."
            );
            diags.add_at_coded(
                crate::diagnostics::Level::Warning,
                Some("lost-write"),
                &msg,
                def_file,
                line,
                col,
            );
            // @PLN131 — both ways out are Conditional, and that is the honest tier: the
            // diagnostic proves the write is lost, not which of the two the author meant.
            // `&` write-through leads, because it is the idiom that generalises; reading
            // the local back is a local repair of one binding.
            diags.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: format!("bind a live reference: `{name} = &…`"),
                condition: Some(format!(
                    "the write is meant to reach the source `{name}` was bound from"
                )),
                edit: None,
                concept: "reference",
                concept_ref: "@F21",
            });
            diags.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: format!("read `{name}` after the mutation"),
                condition: Some(format!(
                    "a copy WAS intended — then the mutated `{name}` is what should be read"
                )),
                edit: None,
                concept: "copy",
                concept_ref: "@F106",
            });
        }
    }
}

/// Where one straight-line sequence has already handed a variable's value to an owner:
/// source var → the position of that hand-off. A second entry for the same var is the defect.
type Handoffs = HashMap<u16, Position>;

/// The walk behind [`warn_double_move`]. Carries the position breadcrumb and the findings;
/// the per-sequence pending set travels as an argument, because a conditional subtree gets
/// its own and must not write back into its parent's.
struct DoubleMove<'a> {
    data: &'a Data,
    func: &'a Function,
    copy_d: u32,
    /// The nearest enclosing span/line, the location a hand-off is reported at.
    cur: Option<Position>,
    /// `(source var, first hand-off, second hand-off)`, one per var per sequence.
    found: Vec<(u16, Position, Position)>,
}

impl DoubleMove<'_> {
    /// Walk a subtree that is CERTAIN to run, given what `st` has already been handed off.
    fn scan(&mut self, node: &Value, st: &mut Handoffs) {
        if let Some(p) = node.span_pos() {
            self.cur = Some(p.clone());
        } else if let Value::Line(n) = node {
            // A bare line marker is the coarse fallback, exactly as the copy notice uses it:
            // an empty `file` means "borrow the definition's own file" (filled in by the
            // caller, which is the pairing loft#781 got wrong the other way round).
            self.cur = Some(Position {
                file: String::new(),
                line: *n,
                pos: 0,
            });
        }
        match node.unspan() {
            // A branch is the whole reason this is a walk and not a `walk`. Two hand-offs in
            // opposite arms release the value ONCE however the branch goes, so they must not
            // pair — each arm therefore scans with a FRESH set, and hand-offs inside an arm
            // never reach the parent's. What the parent must still learn from the subtree is
            // every variable it ASSIGNS: a conditional reassignment gives the variable a new
            // value on one path, so a pending hand-off from before it is no longer certainly
            // the same resource, and pairing it with a later one would be a false positive.
            Value::If(cond, then, els) => {
                self.scan(cond, st);
                for arm in [then.as_ref(), els.as_ref()] {
                    kill_assigned(arm, st);
                    self.scan(arm, &mut Handoffs::new());
                }
            }
            // A loop body is certain relative to ITSELF (two hand-offs inside one iteration
            // are two releases) but not relative to the code around it, so it is scanned the
            // same way an arm is. The iteration count is what stays invisible: ONE hand-off
            // in a body that runs twice is a real double release this lint cannot see, and
            // that false negative is the documented boundary.
            Value::Loop(_) | Value::Iter(..) | Value::Parallel(_) => {
                kill_assigned(node, st);
                self.scan_children_isolated(node);
            }
            // Nothing after a terminator runs, so the pending set cannot pair across it.
            Value::Return(v) => {
                self.scan(v, st);
                st.clear();
            }
            Value::Break(_) | Value::Continue(_) => st.clear(),
            // A reassignment replaces the value, so what was handed off is no longer what
            // this variable holds — `s1 = S{h:c}; c = mk(); s2 = S{h:c}` moves two distinct
            // resources into two containers and is correct.
            //
            // A plain whole-value copy `t = s` is itself a hand-off (the copy owns the
            // resource, `scopes::copy_moves_drop_from`), so `t = s; u = s` is the same
            // double release as two containers built from one droppable.
            Value::Set(v, rhs) => {
                self.scan(rhs, st);
                if let Value::Var(src) = rhs.unspan()
                    && crate::scopes::copy_moves_drop_from(self.func, self.data, *v, *src, false)
                        == Some(*src)
                {
                    self.record_source(*src, st);
                }
                st.remove(v);
            }
            Value::Call(d, args) if *d == self.copy_d && args.len() >= 3 => {
                for a in args {
                    self.scan(a, st);
                }
                self.record(args, st);
            }
            // Everything else — a `Block`/`Insert` statement sequence, an ordinary call, an
            // operand — runs straight through, so its children share the caller's set and
            // are visited in evaluation order.
            _ => self.scan_children(node, st),
        }
    }

    /// Recurse into every child in evaluation order, sharing the caller's pending set.
    fn scan_children(&mut self, node: &Value, st: &mut Handoffs) {
        node.for_each_child(&mut |c| self.scan(c, st));
    }

    /// Recurse into every child, each with its OWN pending set — the shape a branch, a loop
    /// body and a parallel arm share: certain within itself, not with what surrounds it.
    fn scan_children_isolated(&mut self, node: &Value) {
        node.for_each_child(&mut |c| self.scan(c, &mut Handoffs::new()));
    }

    /// Record one `OpCopyRecord` that hands its source's ownership away, and report the
    /// SECOND such hand-off of the same variable.
    fn record(&mut self, args: &[Value], st: &mut Handoffs) {
        // The exact predicate the drop suppression uses (`scopes::collect_drop_transferred`),
        // so the lint and the mechanism cannot drift: a hand-off is what makes the source
        // stop dropping, and this asks the same question of the same node.
        // A whole-value copy into a compiler buffer (a materialised branch arm, a return
        // buffer) moves the drop the way `t = s` does — the same arm the collector reads.
        if let (Value::Var(src), Value::Var(dst)) = (args[0].unspan(), args[1].unspan())
            && self.func.name(*dst).starts_with("__ref")
            && crate::scopes::copy_moves_drop_from(self.func, self.data, *dst, *src, true)
                == Some(*src)
        {
            self.record_source(*src, st);
            return;
        }
        let moved = matches!(args[2].unspan(), Value::Int(tp) if tp & 0x8000 != 0);
        if !moved
            && !crate::scopes::copy_hands_off(&args[1], self.func, self.data)
            && !crate::scopes::appends_to_element(&args[1], self.func, self.data)
        {
            return;
        }
        let Some(src) = crate::scopes::drop_bearing_source(&args[0], self.func) else {
            return;
        };
        self.record_source(src, st);
    }

    /// Record a hand-off of `src`, and report the SECOND one of the same variable.
    fn record_source(&mut self, src: u16, st: &mut Handoffs) {
        // Only a variable the AUTHOR wrote can be acted on. A compiler temp handed off twice
        // (`__lift_N`, `__ref_N`, `_elm_N`) is either impossible or our own bug, and either
        // way names nothing the reader can edit.
        let name = self.func.name(src);
        if name.starts_with('_') || name.contains('#') {
            return;
        }
        let Some(at) = self.cur.clone() else { return };
        if let Some(first) = st.get(&src) {
            self.found.push((src, first.clone(), at));
            // Drop the pending entry so a third hand-off reports once more against the
            // second, rather than N-1 times against the first.
            st.insert(src, self.found.last().expect("just pushed").2.clone());
        } else {
            st.insert(src, at);
        }
    }
}

/// Every variable a subtree ASSIGNS. A conditional assignment must retire the parent's
/// pending hand-off for that variable — see the `If` arm of [`DoubleMove::scan`].
fn kill_assigned(node: &Value, st: &mut Handoffs) {
    if st.is_empty() {
        return;
    }
    node.walk(&mut |n| {
        if let Value::Set(v, _) = n {
            st.remove(v);
        }
    });
}

/// @PLN139 stage G — the DOUBLE-MOVE lint (`LOFT_NO_DOUBLE_MOVE` opts out).
///
/// @PLN139 made a copy into a container a MOVE: the container owns the value now and its
/// death releases it. That closed loft#849 — and made a shape that used to LEAK into a
/// double close. `c = mk(); s1 = S { h: c }; s2 = S { h: c }` hands one resource to two
/// owners, and both release it. Rust prevents this with move checking, which loft does not
/// have, so the hazard is caught by a diagnostic instead of by the type system.
///
/// `warning`, not `advice`, per the two-tier rule: ignoring it produces a wrong result.
/// A warning gates a library's CI, so the lint is deliberately an UNDER-approximation — it
/// fires only where both hand-offs are certain to run:
///
/// - opposite arms of an `if` release the value once however the branch goes → silent;
/// - a reassignment between the two hand-offs makes them two distinct resources → silent,
///   and a reassignment on only ONE path retires the pending hand-off for the same reason;
/// - a terminator between them means the second never runs → silent.
///
/// The boundary it cannot see is the iteration count: one hand-off inside a loop body that
/// runs twice is a real double release with one static node behind it. So is a hand-off on
/// only one branch, which LEAKS rather than double-releasing. Both need the control-flow
/// graph loft does not build, and both are false NEGATIVES — the safe direction for a tier
/// that gates. Runs POST-`scopes::check`, from `main`, beside [`warn_dead_stores`].
pub fn warn_double_move(
    data: &Data,
    diags: &mut crate::diagnostics::Diagnostics,
    fallback_file: &str,
) {
    if !crate::keys::double_move_enabled() {
        return;
    }
    let copy_d = data.def_nr("OpCopyRecord");
    if copy_d == u32::MAX {
        return;
    }
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            continue;
        }
        // The file must come from the DEFINITION, not the entry file: this is a warning, and
        // a warning gates a library's CI, so a dependency's line number paired with the
        // consumer's path would fail a consumer on a line it cannot see (loft#781).
        let def_file = if def.position.file.is_empty() {
            fallback_file
        } else {
            def.position.file.as_str()
        };
        let mut cx = DoubleMove {
            data,
            func: &def.variables,
            copy_d,
            cur: None,
            found: Vec::new(),
        };
        cx.scan(&def.code, &mut Handoffs::new());
        for (src, first, at) in std::mem::take(&mut cx.found) {
            let name = def.variables.name(src);
            let ty = data.type_name_str(def.variables.tp(src));
            let file = if at.file.is_empty() {
                def_file
            } else {
                at.file.as_str()
            };
            // Name the FACT, not the cure — the cure is `--explain`'s job. Both positions
            // matter to the reader: the second is where the defect is written, the first is
            // the owner they have forgotten about. Naming a line the caret already points at
            // reads as a second site that is not there, so the same-line case drops it.
            let earlier = if first.line == at.line {
                "earlier on this line".to_string()
            } else {
                format!("at line {}", first.line)
            };
            let msg = format!(
                "`{name}` is handed to a second owner here — a container or a copy already \
                 took ownership of it {earlier}, and each owner releases what it owns, so \
                 this one {ty} value is released TWICE"
            );
            diags.add_at_coded(
                crate::diagnostics::Level::Warning,
                Some("double-move"),
                &msg,
                file,
                at.line,
                at.pos,
            );
            // Both ways out are Conditional, and that is the honest tier: the diagnostic
            // proves one value reaches two owners, not which of the two the author meant.
            diags.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: format!("build a second {ty} for the second container"),
                condition: Some(format!(
                    "the two containers are meant to hold separate values — `{name}` holds one"
                )),
                edit: None,
                concept: "move",
                concept_ref: "@F106",
            });
            diags.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: "give the value to one container only".to_string(),
                condition: Some(format!(
                    "the containers are meant to SHARE — then only one may own `{name}`, and \
                     the other must read it from that one"
                )),
                edit: None,
                concept: "move",
                concept_ref: "@F106",
            });
        }
    }
}

/// @PLN102 arc C step 4 — the FOLD lint (C5.2).  For every `#superseded "Y"` symbol X in loft's
/// OWN code — the stdlib (`STD_SOURCE`, so loft's `make ci` enforces its stdlib folds) or the entry
/// project (`MAIN_SOURCE`, a library author's own lib / a user's program); a third-party dependency
/// (source `2..`) is excluded, since a consumer cannot fix its fold: (a) the successor `Y` must
/// RESOLVE — an unresolvable successor is a hard
/// `Level::Error`, so a *dangling* steer never ships; (b) X's body must CALL `Y` — X is a shim over
/// the successor, not independent code — else an advisory `Level::Warning` (promote to a hard
/// `make ci` check once the surface is clean).  Every steer thus ships with its fold, or the lint
/// fires.  INERT until a symbol is actually marked `#superseded`, so the suite is byte-identical.
/// Populates `diags`; the caller renders them with the other diagnostics.
/// @PLN24 arc B / @PLN128 arc C — refuse a `#c` binding whose shape exceeds the
/// arity the CONTRACT covers, on both backends.
///
/// Arc B calls C for real, so this is no longer "the interpreter cannot do it";
/// it is the narrow residue. A shape that works on one backend and silently
/// misbehaves on the other is the divergence class the ship gate exists to
/// catch — before arc B, `strlen("hello")` compiled under `--interpret` and
/// answered **7562** — so anything not covered is refused loudly instead.
///
/// **Arc C made this ONE contract.** It used to run on the interpreter only, so
/// an over-ceiling binding compiled and ran under `--native` and failed only
/// when something interpreted it. That made `#c` two languages, and put the
/// failure on a downstream consumer rather than on the author who wrote the
/// declaration. Both backends are now held to [`MAX_C_ARITY`], which was raised
/// to 32 so that unifying did not have to narrow what already compiles.
///
/// Two passes, because the two audiences need different things:
///
/// - **Declarations**, scoped to OWN code (the stdlib, or the entry project — a
///   library author's own lib, a user's program). This is what puts the error
///   in front of the person who can fix it, at the moment they write it, whether
///   or not anything calls it yet. A third-party dependency is excluded for the
///   same reason `superseded_fold_diagnostics` excludes it: a consumer cannot
///   edit a dependency's declaration, so merely LOADING one must not fail.
/// - **Call sites**, everywhere. Calling an over-ceiling binding genuinely
///   cannot work, so it is refused wherever it appears — including through a
///   third-party dependency, where it is the one case a consumer must be told
///   about because the call is theirs.
pub fn c_binding_call_unsupported(
    data: &Data,
    diags: &mut crate::diagnostics::Diagnostics,
    fallback_file: &str,
) {
    fn walk(code: &Value, data: &Data, found: &mut Vec<u32>) {
        if let Value::Call(d, _) = code.unspan()
            && uncovered(data, *d).is_some()
            && !found.contains(d)
        {
            found.push(*d);
        }
        code.for_each_child(&mut |c| walk(c, data, found));
    }
    /// Why this binding is outside the contract, or `None` when it is inside.
    fn uncovered(data: &Data, d_nr: u32) -> Option<String> {
        let def = data.def(d_nr);
        if def.c_sig.is_empty() {
            return None;
        }
        let sig = match crate::c_signature::of(data, d_nr, crate::c_signature::CTarget::host()) {
            Some(Ok(s)) => s,
            // A signature that did not parse was already reported where it was
            // written; do not report it twice at every call site.
            _ => return None,
        };
        if sig.params.len() > crate::c_signature::MAX_C_ARITY {
            return Some(format!(
                "it takes {} C arguments and a `#c` binding covers 0..={}",
                sig.params.len(),
                crate::c_signature::MAX_C_ARITY
            ));
        }
        None
    }
    // Declarations in own code first, so the author's own build names it even
    // when nothing calls it; then call sites anywhere. `reported` keeps a
    // binding that is both declared here AND called here to one diagnostic.
    let mut reported: Vec<u32> = Vec::new();
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        let own = def.source == crate::data::STD_SOURCE || data.source_is_owned(def.source);
        if own && !def.c_sig.is_empty() && uncovered(data, d_nr).is_some() {
            reported.push(d_nr);
        }
    }
    let mut called: Vec<u32> = Vec::new();
    for d_nr in 0..data.definitions() {
        walk(data.def(d_nr).code(), data, &mut called);
    }
    for d_nr in called {
        if !reported.contains(&d_nr) {
            reported.push(d_nr);
        }
    }
    for d_nr in reported {
        let def = data.def(d_nr);
        let pos = def.position();
        let file = if pos.file.is_empty() {
            fallback_file
        } else {
            pos.file.as_str()
        };
        let why = uncovered(data, d_nr).unwrap_or_default();
        diags.add_at_coded(
            crate::diagnostics::Level::Error,
            Some("c-binding-not-interpretable"),
            &format!(
                "`{}` is bound to the C symbol `{}` with `#c`, which no backend can \
                 call: {why}",
                def.display_name(),
                def.c_symbol
            ),
            file,
            pos.line,
            pos.pos,
        );
        // @PLN131 — the fix does not spell an `edit`: it is C the compiler cannot
        // write. @PLN128 arc C removed the second fix that used to sit here —
        // "run it on `--native`, which can make the call as written" — because
        // that is no longer true, and a fix line naming a backend that does not
        // help is worse than no second option.
        diags.fix_last(crate::diagnostics::Fix {
            kind: crate::diagnostics::FixKind::Mechanical,
            title: format!(
                "wrap it in an ANSI-C shim taking at most {} parameters",
                crate::c_signature::MAX_C_ARITY
            ),
            condition: None,
            edit: None,
            concept: "direct C binding",
            concept_ref: "@F92",
        });
    }
}

/// Which of `callee`'s parameters does its body WRITE THROUGH — `fn hurt(e: E) { e.taken = … }`?
///
/// Answered from the callee's own emitted body, so it covers any depth of field or element
/// write, and it stays right when the body changes. `find_field_written_vars` is the same
/// walk `check_ref_mutations` uses to decide whether a `&` parameter was really mutated, so
/// the two cannot disagree about what a write through a parameter IS.
///
/// Two kinds of parameter are excluded, both because a write through them is not lost:
/// a `&` reference (`RefVar`) is an explicit alias, and a compiler-introduced parameter
/// (`__retbuf` and friends) is how a return VALUE is delivered, not a caller's variable.
fn write_through_params(data: &Data, callee: u32) -> HashSet<u16> {
    let def = data.def(callee);
    let mut written = HashSet::new();
    crate::parser::find_field_written_vars(&def.code, data, &mut written);
    let func = &def.variables;
    written.retain(|&v| {
        func.is_argument(v)
            && !matches!(func.tp(v), Type::RefVar(_))
            && !func.name(v).starts_with("__")
    });
    written
}

/// Is `arg` a call that hands back a COPY of a place the caller can still reach?
///
/// This is the difference between a write that LOSES data and one that merely writes into
/// something nobody wanted. `first(s)` returns `E["s"]` — the dep says the value came out
/// of the caller's `s` — so a write into the returned copy is a write the caller meant for
/// `s` and will not find there. `mk()` returns a dep-free `E` it built itself, so a write
/// into it loses nothing that existed before the call: pointless, but not a lost write, and
/// warning about it would be noise.
///
/// A dep is only believed when it names a parameter the CALL SITE actually filled with a
/// real variable. A function that builds its result into a caller-provided return buffer
/// also carries a dep — `alloc_canvas(w, h, fill)` returns `Canvas["cv"]` — but `cv` is the
/// compiler's own work-ref, not a place the program can reach, so the copy is nobody's
/// data. Skipping `_`-prefixed names is what tells the two apart, and it is the same
/// convention [`warn_dead_stores`] uses to recognise a synthetic.
///
/// A result BOUND to a local first (`e = first(s)`) arrives as a `Var` and is deliberately
/// not matched — see [`warn_lost_temp_writes`].
fn copies_a_reachable_place(data: &Data, func: &Function, arg: &Value) -> bool {
    let Value::Call(fn_nr, inner_args) = arg.unspan() else {
        return false;
    };
    let def = data.def(*fn_nr);
    if !def.name().starts_with("n_") || *def.code() == Value::Null {
        return false;
    }
    // Deps are variable numbers in the CALLEE's frame, and its parameters come first, so a
    // dep below the argument count names the argument the caller supplied for it.
    def.returned().depend().iter().any(|&j| {
        inner_args
            .get(j as usize)
            .is_some_and(|a| matches!(a.unspan(), Value::Var(v) if !func.name(*v).starts_with('_')))
    })
}

/// loft#894 — a write through a struct RETURNED from a function, which reaches nothing.
///
/// `hurt(first(s), 10.0)` and `hurt(s.es[0] ?? E {}, 10.0)` are the same types, the same
/// call, and the same write — but the first one does nothing. Returning a struct hands back
/// a COPY (C86), which lives in a temporary that is freed at the end of the statement, so
/// `hurt` writes the copy and the element keeps the value it had. Passing the element
/// directly hands over a view of it, so the same write lands.
///
/// Nothing at the call site distinguishes the two, which is what made this expensive
/// downstream: dryopea lost six tests to it at once, and every one read as a bug in the
/// thing being mutated rather than in the one-line accessor, because the read-back is
/// simply the value from before the call.
///
/// Two facts have to meet for data to be lost, and the lint requires both:
/// the callee must WRITE THROUGH the parameter ([`write_through_params`], read off its own
/// body), and the argument must be a copy of a place the caller can still REACH
/// ([`copies_a_reachable_place`], read off the return type's deps). Requiring the second is
/// what keeps `hurt(mk(), …)` quiet — a write into a freshly built value loses nothing —
/// and what keeps the builder idiom quiet, where the callee returns the value it wrote.
///
/// Binding the call result to a real local first (`e = first(s); hurt(e, …)`) is
/// deliberately SILENT too: `e` is a copy the program can still read, which makes it a
/// question about copies rather than a lost write, and [`warn_copies`] owns that.
///
/// `warning` tier, per the two-tier rule: ignoring it produces a wrong result. It is
/// therefore an UNDER-approximation — it reports the shape it can prove and stays silent on
/// a write lost through any other kind of temporary.
pub fn warn_lost_temp_writes(
    data: &Data,
    diags: &mut crate::diagnostics::Diagnostics,
    fallback_file: &str,
) {
    if !crate::keys::lost_temp_writes_enabled() {
        return;
    }
    let mut params: HashMap<u32, HashSet<u16>> = HashMap::new();
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            continue;
        }
        // The file comes from the DEFINITION, not the entry file: this is a warning, and a
        // warning gates a library's CI, so a dependency's line paired with the consumer's
        // path would fail a consumer on a line it cannot see (loft#781).
        let def_file = if def.position.file.is_empty() {
            fallback_file
        } else {
            def.position.file.as_str()
        };
        let mut found = Vec::new();
        scan_lost_temp_writes(
            data,
            &def.variables,
            &def.code,
            &mut params,
            None,
            &mut found,
        );
        for (callee, param, at) in found {
            let file = match &at {
                Some(p) if !p.file.is_empty() => p.file.as_str(),
                _ => def_file,
            };
            let (line, col) = at
                .as_ref()
                .map_or((def.position.line, 0), |p| (p.line, p.pos));
            let callee_def = data.def(callee);
            let fn_name = callee_def.original_name();
            let param_name = callee_def.variables.name(param);
            let msg = format!(
                "`{fn_name}` writes to `{param_name}`, but the argument here is a value RETURNED \
                 by a call — a temporary that is freed at the end of this statement, so the write \
                 is LOST. Returning a struct hands back a COPY (C86); pass the element itself, or \
                 bind the result and read it back."
            );
            diags.add_at_coded(
                crate::diagnostics::Level::Warning,
                Some("lost-write"),
                &msg,
                file,
                line,
                col,
            );
            diags.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: "pass the element itself instead of a call that returns it".to_string(),
                condition: Some(
                    "the write is meant to reach what the call read it from".to_string(),
                ),
                edit: None,
                concept: "reference",
                concept_ref: "@F21",
            });
            diags.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: format!("declare the parameter `&{param_name}` so the write is delivered"),
                condition: Some("the accessor is meant to be written through".to_string()),
                edit: None,
                concept: "reference",
                concept_ref: "@F21",
            });
        }
    }
}

/// Walk `node` for calls whose argument at a write-through parameter is a `__lift_N`
/// temporary. Collects `(callee, parameter var, position)`.
fn scan_lost_temp_writes(
    data: &Data,
    func: &Function,
    node: &Value,
    params: &mut HashMap<u32, HashSet<u16>>,
    at: Option<&Position>,
    out: &mut Vec<(u32, u16, Option<Position>)>,
) {
    let here = node.span_pos().or(at);
    // Matched on the BARE node, not `unspan()`: `for_each_child` hands a `Span`'s inner
    // value straight back, so unspanning here would match the same call twice — once
    // through its wrapper and once as its own child — and report it twice.
    if let Value::Call(callee, args) = node {
        let def = data.def(*callee);
        // A user function only: an `Op*` builtin has no loft body to have written through,
        // and its first-arg writes are exactly what `find_field_written_vars` reads.
        if matches!(def.def_type, DefType::Function) && *def.code() != Value::Null {
            let written = params
                .entry(*callee)
                .or_insert_with(|| write_through_params(data, *callee));
            if !written.is_empty() {
                for (i, arg) in args.iter().enumerate() {
                    if written.contains(&(i as u16)) && copies_a_reachable_place(data, func, arg) {
                        out.push((*callee, i as u16, here.cloned()));
                    }
                }
            }
        }
    }
    node.for_each_child(&mut |child| {
        scan_lost_temp_writes(data, func, child, params, here, out);
    });
}

pub fn superseded_fold_diagnostics(
    data: &Data,
    diags: &mut crate::diagnostics::Diagnostics,
    fallback_file: &str,
) {
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        let succ = def.superseded();
        // Scope to loft's OWN code — the stdlib (`STD_SOURCE`, checked by loft's `make ci`) or the
        // entry project being built (`MAIN_SOURCE`, a library author's own lib / a user's program).
        // A THIRD-PARTY dependency (source `2..`) is excluded: a consumer cannot fix a dependency's
        // fold, so it must not error their compile (the dependency's author catches it in their own
        // build, where their lib is the entry).
        let own = def.source == crate::data::STD_SOURCE || data.source_is_owned(def.source);
        if succ.is_empty() || !own {
            continue;
        }
        let pos = def.position();
        let file = if pos.file.is_empty() {
            fallback_file
        } else {
            pos.file.as_str()
        };
        let shown = def.display_name();
        // (a) the successor must resolve — as a free fn `n_<succ>`, or (if X is a
        // method) the same-receiver method `t_<LEN><Type>_<succ>`.  Each tried in
        // X's own source, then the stdlib (`STD_SOURCE`, visible everywhere).
        let mut candidates = vec![format!("n_{succ}")];
        if let Some(prefix) = def.method_type_prefix() {
            candidates.push(format!("{prefix}{succ}"));
        }
        let y_nr = candidates
            .iter()
            .map(|name| {
                let n = data.source_nr(def.source, name);
                if n == u32::MAX {
                    data.source_nr(crate::data::STD_SOURCE, name)
                } else {
                    n
                }
            })
            .find(|&n| n != u32::MAX)
            .unwrap_or(u32::MAX);
        if y_nr == u32::MAX {
            diags.add_at_coded(
                crate::diagnostics::Level::Error,
                Some("superseded-unknown-successor"),
                &format!(
                    "`#superseded \"{succ}\"` on `{shown}`: no such successor `{succ}` — the \
                     steer would ship dangling"
                ),
                file,
                pos.line,
                pos.pos,
            );
            // @PLN131 — neither spells an `edit`, for two different reasons. The successor
            // name is the thing the compiler just failed to find, so any spelling it
            // offered would be a guess. And dropping the attribute is a deletion it cannot
            // PLACE: the diagnostic sits at the definition, not at the attribute's span, so
            // an edit here would tell an applier to delete the function.
            diags.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: format!("name a successor that exists, in place of `{succ}`"),
                condition: Some(format!(
                    "`{shown}` really is superseded — the successor is a bare symbol name, so \
                     a renamed or not-yet-written `{succ}` reads the same as a wrong one"
                )),
                edit: None,
                concept: "superseded",
                concept_ref: "@F109",
            });
            diags.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: format!("drop the `#superseded` attribute from `{shown}`"),
                condition: Some(format!("`{shown}` is not actually superseded by anything")),
                edit: None,
                concept: "superseded",
                concept_ref: "@F109",
            });
            continue;
        }
        // (b) the shim check — X's body must CALL Y (fold the old form onto the new).
        // For a GENERIC successor the body's call targets the monomorphised
        // instantiation (`t_<Type>_sum`), not the `n_sum` template `y_nr` resolved to,
        // so also match by the user-facing name (both render as `sum`). Gate that loose
        // match on the successor ACTUALLY being generic, so an unrelated same-named
        // symbol can't masquerade as the fold — a non-generic successor must be the
        // direct call to `y_nr`.
        let succ_generic = matches!(data.def(y_nr).def_type, crate::data::DefType::Generic);
        let folds = def.code.any_node(&mut |n| {
            matches!(n, Value::Call(d, _)
                if *d == y_nr || (succ_generic && data.def(*d).display_name() == succ))
        });
        if !folds {
            diags.add_at_coded(
                crate::diagnostics::Level::Warning,
                Some("superseded-not-folded"),
                &format!(
                    "`{shown}` is `#superseded` by `{succ}` but its body never calls `{succ}` — \
                     two implementations of one idea, and they will drift"
                ),
                file,
                pos.line,
                pos.pos,
            );
            // @PLN131 — the fold LEADS, because it is what makes the steer true: an
            // un-folded pair is two implementations of one idea, and they drift. Dropping
            // the attribute is sound but gives up the steer, so it ranks second.
            diags.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: format!("reimplement `{shown}` as a shim that calls `{succ}`"),
                condition: Some(format!(
                    "`{succ}` can express what `{shown}` does — a successor that CANNOT is not \
                     a supersession, and belongs behind a contract bump instead"
                )),
                edit: None,
                concept: "superseded",
                concept_ref: "@F109",
            });
            diags.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: format!("drop the `#superseded` attribute from `{shown}`"),
                condition: Some(format!(
                    "`{shown}` is meant to stay an independent implementation, not a shim"
                )),
                edit: None,
                concept: "superseded",
                concept_ref: "@F109",
            });
        }
    }
}

/// @PLN90 W5 — the ENFORCED copy lint (`LOFT_WARN_COPIES`). Routes every **Avoidable** unbound
/// structure copy (a still-live value duplicated where a borrow/move would remove it — the
/// worklist) through the normal `Level::Warning` diagnostics channel, so it surfaces during a
/// normal compile with a source location, the copied type, and the `&`/restructure hint. `Forced`
/// (required as written) and `Implicit`/`Eliminated`/`Internal` stay silent here — only the
/// actionable set warns. Shares the survival-split verdict with `report_copies`; a no-op unless
/// `warn_copies_enabled()`. Populates `diags`; the caller renders them with the other diagnostics.
pub fn warn_copies(data: &Data, diags: &mut crate::diagnostics::Diagnostics, fallback_file: &str) {
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            continue;
        }
        for r in analyze_fn_survival(&def.code, &def.variables, data, env_tier(), true).0 {
            // Only survival-split (source-duplicating) copies are user-facing, and only the
            // Avoidable class is the actionable worklist — mirror `report_copies`'s filter.
            if !r.survival || !matches!(r.class, CopyClass::Avoidable) {
                continue;
            }
            let ty = if r.source == u16::MAX {
                "a structure".to_string()
            } else {
                data.type_name_str(def.variables.tp(r.source))
            };
            // The copy op carries no span; it borrows the nearest span or (S5.2) the enclosing
            // line marker. A line-only fallback has an empty `file`, and the file that line
            // belongs to is the one THIS DEFINITION was parsed from — not the entry file.
            // Pairing a dependency's line number with the consumer's path is a real line in
            // the wrong file, so it lands on whatever happens to sit there: a comment or a
            // `const` in 29 of 67 notices from one consumer (loft#781). `fallback_file` is
            // the last resort, for a definition carrying no position either.
            let def_file = if def.position.file.is_empty() {
                fallback_file
            } else {
                def.position.file.as_str()
            };
            // When even the line is unknown, fall back to line 0 + the fn name.
            let (file, line, col) = r.loc.as_ref().map_or((def_file, 0, 0), |p| {
                let f = if p.file.is_empty() {
                    def_file
                } else {
                    p.file.as_str()
                };
                (f, p.line, p.pos)
            });
            let where_ = if r.loc.is_some() {
                String::new()
            } else {
                format!(" in `{}`", def.name)
            };
            // Name the LEVER, not the analysis. "a borrow/move would avoid this copy" is true
            // and useless to an author: they wrote no copy and cannot write a borrow. What
            // they CAN do is measured (@PLN130 F5) — both of these take the row to zero:
            //
            //   src = [1,2,3]; h = Holder { v: src }; use(src)   <- copies
            //   src = [1,2,3]; h = Holder { v: src };            <- move, no copy
            //   h = Holder { v: [1,2,3] };                       <- built in place, no copy
            //
            // So the message names the surviving variable and the two ways out. Advice, not
            // Warning: the program is CORRECT, it just pays a copy — and the repo rule is
            // that a diagnostic gates only when ignoring it can produce a wrong result.
            let src_name = if r.source == u16::MAX {
                String::new()
            } else {
                format!(" `{}`", def.variables.name(r.source))
            };
            let msg = if src_name.is_empty() {
                // No named source, so no fix attaches below — this branch KEEPS its
                // resolution in the prose. Handing the fix to `--explain` only works where
                // there is a fix to hand it to; stripping it here would leave the reader
                // with nothing at all (@PLN131).
                format!(
                    "copy of {ty}{where_} — the value is still in use after this point, so it \
                     could not be moved. Build it in place, or stop using the source \
                     afterwards, and the copy becomes a move"
                )
            } else {
                format!(
                    "copy of {ty}{where_} —{src_name} is still used after this point, so it \
                     could not be moved"
                )
            };
            // @PLN131 prerequisite arc — the copy notice gets the FIRST code, because it is
            // the diagnostic the suggestions work builds on. The code is the frozen
            // identity a fix attaches to; the prose above stays free to improve.
            diags.add_at_coded(
                crate::diagnostics::Level::Advice,
                Some("avoidable-copy"),
                &msg,
                file,
                line,
                col,
            );

            // @PLN131 ship steps 1–2 — what to write instead. Ranked most-teaching first.
            //
            // Only a NAMED source gets fixes: both rewrites are about a specific variable's
            // later use, and neither can be stated without one.
            if r.source != u16::MAX {
                let name = def.variables.name(r.source);
                // Mechanical: building the value where it belongs means there was never a
                // first copy to elide. No condition — its meaning is fixed by the code.
                //
                // No `edit` is spelled. The IR desugars a construction and a `+=` append to
                // the same node, so the compiler cannot yet tell which shape it is looking
                // at — and the prototype measured that the append shape has NO
                // build-in-place rewrite. Offering a synthesised edit for both would emit a
                // fix that does not exist for the second-commonest avoidable shape in the
                // corpus, which is worse than offering the title alone.
                diags.fix_last(crate::diagnostics::Fix {
                    kind: crate::diagnostics::FixKind::Mechanical,
                    title: "build the value in place".to_string(),
                    condition: None,
                    edit: None,
                    concept: "move",
                    concept_ref: "@F106",
                });
                // Conditional: the condition NAMES the surviving use, which is the whole
                // point of carrying its location (Q6.1). "after here" sends a veteran
                // hunting; "at line 12" is affirmable in a second.
                let where_used = r.source_last_use.as_ref().map_or_else(
                    || format!("`{name}` is not needed after this point"),
                    |p| {
                        format!(
                            "`{name}` is used again at line {} — you do not need that",
                            p.line
                        )
                    },
                );
                diags.fix_last(crate::diagnostics::Fix {
                    kind: crate::diagnostics::FixKind::Conditional,
                    title: format!("drop the later use of `{name}`"),
                    condition: Some(where_used),
                    edit: None,
                    concept: "move",
                    concept_ref: "@F106",
                });
            }
        }
    }
}

/// @PLN90 Step 5 — the USER-FACING copy report (`LOFT_REPORT_COPIES` / `--report-copies`).
///
/// Unlike `dump_all` (the raw developer trace), this surfaces ONLY the *unbound* structure
/// copies the user can act on — `Avoidable` (a borrow/move would remove it — the worklist) and
/// `Forced` (required as written, informational) — each with a source location, the copied
/// type, and a fix hint, followed by a rollup and the ranked Avoidable worklist. `Implicit`
/// (moves / literals) and `Internal` (compiler-generated sources) are excluded — they are not
/// the user's to fix. Diagnostic only; called from `scopes::check`, a no-op unless enabled.
pub fn report_copies(data: &Data) {
    if !crate::keys::report_copies_enabled() {
        return;
    }
    struct Row {
        fname: String,
        loc: String,
        ty: String,
        avoidable: bool,
        reason: &'static str,
    }
    let mut rows: Vec<Row> = Vec::new();
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            continue;
        }
        for r in analyze_fn(&def.code, &def.variables, data, env_tier()).0 {
            // Only survival-split copies (source duplications) are user-facing; the var-buffer /
            // return-buffer copies are a separate elision class (and where the stdlib's copies
            // land — the survival baseline is 0), kept to the developer dump.
            if !r.survival {
                continue;
            }
            let avoidable = match r.class {
                CopyClass::Avoidable => true,
                CopyClass::Forced => false,
                // Eliminated / Implicit (move, literal) / Internal (compiler-gen) — not the
                // user's to act on.
                _ => continue,
            };
            // S5.2 — a line-only fallback carries an empty `file` (the copy sat under no span);
            // render it as `line N` rather than the raw `:N:0`.
            let loc = r.loc.as_ref().map_or_else(
                || "<location unknown>".to_string(),
                |p| {
                    if p.file.is_empty() {
                        format!("line {}", p.line)
                    } else {
                        p.to_string()
                    }
                },
            );
            let ty = if r.source == u16::MAX {
                "a structure".to_string()
            } else {
                data.type_name_str(def.variables.tp(r.source))
            };
            rows.push(Row {
                fname: def.name.clone(),
                loc,
                ty,
                avoidable,
                reason: r.reason,
            });
        }
    }

    eprintln!(
        "loft copy report — unbound structure copies (a copy the alias-default did not make silently)"
    );
    if rows.is_empty() {
        eprintln!("  none — every structure copy is a move, a literal, or already borrowed.");
        return;
    }
    for row in &rows {
        let tag = if row.avoidable {
            "avoidable"
        } else {
            "forced "
        };
        eprintln!(
            "  {}  fn {}  copies {}  [{tag}]  {}",
            row.loc, row.fname, row.ty, row.reason
        );
    }
    let avoidable = rows.iter().filter(|r| r.avoidable).count();
    let forced = rows.len() - avoidable;
    eprintln!(
        "  ── {} unbound {}: {avoidable} avoidable, {forced} forced (moves & literals are silent)",
        rows.len(),
        if rows.len() == 1 { "copy" } else { "copies" }
    );
    if avoidable > 0 {
        eprintln!("  Avoidable — a `&` borrow or a small restructure would remove these:");
        for row in rows.iter().filter(|r| r.avoidable) {
            eprintln!("    {}  {}", row.loc, row.ty);
        }
    }
}

/// @PLN103 (temporal extension) — static free-before-dependent-read detector.
///
/// The `--show-ownership` overlay classifies WHO owns each store, but the verdict is
/// temporal-agnostic: it renders identically whether a store's `OpFreeRef` lands
/// before or after the last read of a view into it. The captured-group element-access
/// UAF (`plans/captured-group-elem-uaf.md`) was invisible to it for exactly that
/// reason — the correct and the use-after-free lowering share every ownership verdict.
/// This walk adds the missing temporal check.
///
/// Invariant enforced: along each straight-line path of the committed IR, an
/// `OpFreeRef(S)` must not be followed by a genuine READ of any binding `B` whose
/// static deps include `S` (`B` is a live view into the freed store), unless `S` is
/// re-allocated (`OpDatabase(S)`) between the free and the read.
///
/// Returns `(store, via)` pairs — the freed store var and the view read after it —
/// sorted + deduped. Empty = clean. Conservative by construction (branch state is
/// intersected at joins; only an UNCONDITIONAL `OpFreeRef` frees), so it under-reports
/// rather than false-positives.
pub fn free_before_dependent_read(data: &Data, d_nr: u32) -> Vec<(u16, u16)> {
    let def = data.def(d_nr);
    let vars = &def.variables;
    let free_ref = data.def_nr("OpFreeRef");
    let op_db = data.def_nr("OpDatabase");
    // Reverse map: store var `S` -> bindings `B` that VIEW `S`, TRANSITIVELY. A view
    // of a view keeps `S` live through an intermediate local — a nested `match arg {…}`
    // binds `_match_subj_2 = arg`, and `arg` deps `__vdb_1`, so a deref of
    // `_match_subj_2` reads `__vdb_1`. A single-hop map misses it (`arg` is only
    // bare-MOVED into `_match_subj_2`, never deref'd), so walk the dep graph to a
    // fixpoint — the same closure `scopes::transitive_depers` uses for reclaim.
    // Skip marker deps (the u16::MAX one-buffer sentinel, the 0x8000 callee-frame tag)
    // and self-deps (work-refs carry their own var in the dep list).
    let n = vars.count();
    let direct = |b: u16| -> Vec<u16> {
        vars.tp(b)
            .depend()
            .into_iter()
            .filter(|&d| d != u16::MAX && d & 0x8000 == 0 && d != b)
            .collect::<Vec<u16>>()
    };
    let mut reaches: Vec<HashSet<u16>> = (0..n).map(|b| direct(b).into_iter().collect()).collect();
    loop {
        let mut changed = false;
        for b in 0..n as usize {
            for m in reaches[b].iter().copied().collect::<Vec<u16>>() {
                if (m as usize) < n as usize {
                    for a in reaches[m as usize].iter().copied().collect::<Vec<u16>>() {
                        if reaches[b].insert(a) {
                            changed = true;
                        }
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut dependents: HashMap<u16, Vec<u16>> = HashMap::new();
    for b in 0..n {
        for &s in &reaches[b as usize] {
            dependents.entry(s).or_default().push(b);
        }
    }
    if dependents.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<(u16, u16)> = Vec::new();
    let mut freed: HashSet<u16> = HashSet::new();
    scan_uaf(
        &def.code,
        &mut freed,
        &dependents,
        free_ref,
        op_db,
        &mut out,
    );
    out.sort_unstable();
    out.dedup();
    out
}

fn scan_uaf(
    node: &Value,
    freed: &mut HashSet<u16>,
    dependents: &HashMap<u16, Vec<u16>>,
    free_ref: u32,
    op_db: u32,
    out: &mut Vec<(u16, u16)>,
) {
    match node.unspan() {
        Value::Block(bl) | Value::Loop(bl) => {
            for op in &bl.operators {
                scan_uaf(op, freed, dependents, free_ref, op_db, out);
            }
        }
        Value::Insert(ops) => {
            for op in ops {
                scan_uaf(op, freed, dependents, free_ref, op_db, out);
            }
        }
        Value::If(c, t, e) => {
            scan_uaf(c, freed, dependents, free_ref, op_db, out);
            let mut ft = freed.clone();
            scan_uaf(t, &mut ft, dependents, free_ref, op_db, out);
            let mut fe = freed.clone();
            scan_uaf(e, &mut fe, dependents, free_ref, op_db, out);
            // Definitely-freed after the join = freed on BOTH branches. A store freed
            // on only one path is live on the other, so it is NOT carried as freed.
            *freed = ft.intersection(&fe).copied().collect();
        }
        Value::Return(inner) | Value::Drop(inner) | Value::Yield(inner) => {
            scan_uaf(inner, freed, dependents, free_ref, op_db, out);
        }
        Value::Call(op, args) if *op == free_ref => {
            if let Some(Value::Var(s)) = args.first().map(Value::unspan) {
                freed.insert(*s);
            }
        }
        Value::Call(op, args) if *op == op_db => {
            if let Some(Value::Var(s)) = args.first().map(Value::unspan) {
                freed.remove(s); // re-allocated — the store is live again
            }
        }
        // Any other op is a read-leaf for this walk: it neither frees/allocs a store
        // nor branches, so check whether it DEREFERENCES a view of a freed store.
        other => check_reads(other, freed, dependents, out),
    }
}

fn check_reads(
    node: &Value,
    freed: &HashSet<u16>,
    dependents: &HashMap<u16, Vec<u16>>,
    out: &mut Vec<(u16, u16)>,
) {
    if freed.is_empty() {
        return;
    }
    for &s in freed {
        if let Some(views) = dependents.get(&s) {
            for &b in views {
                if deref_read(node, b, false) {
                    out.push((s, b));
                }
            }
        }
    }
}

/// True when `node` DEREFERENCES var `target` — reads the CONTENTS of its store,
/// as opposed to merely delivering the reference. The distinction is what separates
/// a real use-after-free from a benign stale dep:
///
///   * A bare `Var(target)` in a DELIVERY position — `return v`, `x = v` (a move),
///     a block tail — hands the *reference* on. The ownership / retbuf machinery
///     moves it safely, so a stale dep on an already-freed store there is NOT a UAF
///     (the return-hoist `__ret_N` false positives, `85-...` / `562-...`).
///   * `target` consumed inside a `Call` (`OpGetVector(v, …)`, `OpGetText(…v…)`, a
///     fn arg) reads the store's contents — a genuine deref that IS a UAF once the
///     store is freed (the captured-group `arg[0]` access).
///
/// So a bare `Var` only counts when it is already inside a call (`in_call`); the
/// inherently-dereferencing slots (`TupleGet` / `CallRef` / `Iter` / `FnRef` /
/// always count. Write targets (`Set`/`TuplePut` LHS) never do —
/// `for_each_child` restricts those to their RHS.
fn deref_read(node: &Value, target: u16, in_call: bool) -> bool {
    let n = node.unspan();
    let hit = match n {
        Value::Var(x) => in_call && *x == target,
        Value::TupleGet(x, _) | Value::FnRefDnr(x) => *x == target,
        Value::CallRef(x, _) | Value::Iter(x, _, _, _) => *x == target,
        Value::FnRef(_, w, _) => *w == target,
        _ => false,
    };
    if hit {
        return true;
    }
    // A Call / CallRef consumes its arguments, so a bare `Var` beneath one is a read.
    let child_in_call = in_call || matches!(n, Value::Call(_, _) | Value::CallRef(_, _));
    let mut found = false;
    n.for_each_child(&mut |c| {
        if !found && deref_read(c, target, child_in_call) {
            found = true;
        }
    });
    found
}

#[cfg(test)]
mod uaf_overlay_tests {
    //! Injected-fault controls for the free-before-dependent-read walk (`scan_uaf`),
    //! parser-free so the positive control does not depend on the compiler still
    //! EMITTING the bug (the materialisation fix removed the only real-code trigger).
    use super::{check_reads, scan_uaf};
    use crate::data::{Type, Value, v_block};
    use std::collections::{HashMap, HashSet};

    // Arbitrary op-numbers for the walk; the walk only compares against these.
    const FREE_REF: u32 = 100;
    const OP_DB: u32 = 101;
    const GETTER: u32 = 200; // stands in for OpGetVector / OpGetField / OpGetText

    fn deps(store: u16, view: u16) -> HashMap<u16, Vec<u16>> {
        let mut d = HashMap::new();
        d.insert(store, vec![view]);
        d
    }
    fn run(code: &Value, dependents: &HashMap<u16, Vec<u16>>) -> Vec<(u16, u16)> {
        let mut freed = HashSet::new();
        let mut out = Vec::new();
        scan_uaf(code, &mut freed, dependents, FREE_REF, OP_DB, &mut out);
        out.sort_unstable();
        out.dedup();
        out
    }
    fn free(s: u16) -> Value {
        Value::Call(FREE_REF, vec![Value::Var(s)])
    }
    fn deref(v: u16) -> Value {
        Value::Call(GETTER, vec![Value::Var(v)])
    }

    /// POSITIVE control — a deref of view `B` AFTER `OpFreeRef(S)` (S ∈ deps(B)) is flagged.
    #[test]
    fn flags_deref_after_free() {
        let code = v_block(vec![free(10), deref(3)], Type::Void, "body");
        assert_eq!(run(&code, &deps(10, 3)), vec![(10, 3)]);
    }

    /// NEGATIVE — a bare `Var` DELIVERY (`return b`) after the free is a safe move, not a deref.
    #[test]
    fn silent_on_bare_delivery_after_free() {
        let code = v_block(
            vec![free(10), Value::Return(Box::new(Value::Var(3)))],
            Type::Void,
            "body",
        );
        assert!(run(&code, &deps(10, 3)).is_empty());
    }

    /// NEGATIVE — a deref BEFORE the free is correctly ordered.
    #[test]
    fn silent_when_deref_precedes_free() {
        let code = v_block(vec![deref(3), free(10)], Type::Void, "body");
        assert!(run(&code, &deps(10, 3)).is_empty());
    }

    /// NEGATIVE — re-allocating the store (`OpDatabase(S)`) between free and deref clears it.
    #[test]
    fn silent_after_realloc() {
        let code = v_block(
            vec![free(10), Value::Call(OP_DB, vec![Value::Var(10)]), deref(3)],
            Type::Void,
            "body",
        );
        assert!(run(&code, &deps(10, 3)).is_empty());
    }

    /// NEGATIVE — a store freed on only ONE branch is live on the other; a deref after the
    /// join is not a definite UAF (branch state is intersected).
    #[test]
    fn silent_when_free_on_one_branch_only() {
        let code = v_block(
            vec![
                Value::If(
                    Box::new(Value::Var(0)),
                    Box::new(v_block(vec![free(10)], Type::Void, "then")),
                    Box::new(v_block(vec![], Type::Void, "else")),
                ),
                deref(3),
            ],
            Type::Void,
            "body",
        );
        assert!(run(&code, &deps(10, 3)).is_empty());
    }

    /// The `check_reads` leaf helper agrees with the walk on a lone deref.
    #[test]
    fn check_reads_matches_walk() {
        let mut freed = HashSet::new();
        freed.insert(10);
        let mut out = Vec::new();
        check_reads(&deref(3), &freed, &deps(10, 3), &mut out);
        assert_eq!(out, vec![(10, 3)]);
    }
}

/// @PLN35 — static return-source-free detector (sibling of `free_before_dependent_read`).
///
/// A record work-ref that the return value ALIASES (is transferred as) must not be freed
/// with a PLAIN `OpFreeRef` before the return — that hands the caller a freed store (35c
/// sub-class A: `[Kw { word }, ..] => LetS{…}` froze the returned `LetS` record). The
/// P4-records safe form is `OpFreeRefIfDistinct(S, ret)`, which is never a plain free and
/// so is not flagged. This class is invisible to `free_before_dependent_read`: the return
/// does not DEREFERENCE the store in-frame, it delivers a reference the caller reads.
///
/// Returns the freed return-source vars, sorted + deduped. Empty = clean.
pub fn return_source_freed(data: &Data, d_nr: u32) -> Vec<u16> {
    let def = data.def(d_nr);
    let sets = data.op_sets();
    let ops = FreeOps {
        fr: std::sync::Arc::clone(&sets.unconditional_ref_frees),
        ft: sets.text_free,
        fif: std::sync::Arc::clone(&sets.conditional_ref_frees),
        db: data.def_nr("OpDatabase"),
    };
    let mut out: Vec<u16> = Vec::new();
    let mut freed: HashSet<u16> = HashSet::new();
    scan_rsf(&def.code, &def.code, &ops, &mut freed, &mut out);
    out.sort_unstable();
    out.dedup();
    out
}

/// loft#759 — the `&`-parameter twin of [`return_source_freed`].
///
/// A callee has exactly two ways to hand a heap value to its caller: a `return`, and a
/// write through a `&` parameter. `return_source_freed` covers the first; without this
/// the second was asserted nowhere, which is how a `&File` rebind shipped freeing the
/// record its caller went on writing through.
///
/// The shape: a work-ref buffer `__ref_N` is passed to a call whose result is published
/// through a `&` parameter. No deep copy stands between the two — codegen writes the
/// returned `DbRef` straight into the caller's slot — so whenever the callee returns the
/// buffer it was handed, `__ref_N` IS what the caller now holds, and a plain
/// `OpFreeRef(__ref_N)` after that publish frees the caller's record.
///
/// `OpFreeRefIfDistinct(__ref_N, f)` is the safe form (`scan_set` pairs it) and is not
/// flagged, nor is a store re-allocated by `OpDatabase` after the publish.
///
/// The publish set is a MAY-alias union across branches: a buffer published on only one
/// arm of an `if` is still the caller's record on that arm, so a plain free after the
/// join is a real fault there. That is the mirror of `scan_rsf`'s intersection, which
/// tracks "definitely freed" rather than "may have escaped".
///
/// Returns the offending work-ref vars, sorted + deduped. Empty = clean.
pub fn ref_param_publish_freed(data: &Data, d_nr: u32) -> Vec<u16> {
    let def = data.def(d_nr);
    let sets = data.op_sets();
    let ops = FreeOps {
        fr: std::sync::Arc::clone(&sets.unconditional_ref_frees),
        ft: sets.text_free,
        fif: std::sync::Arc::clone(&sets.conditional_ref_frees),
        db: data.def_nr("OpDatabase"),
    };
    let mut out: Vec<u16> = Vec::new();
    let mut published: HashSet<u16> = HashSet::new();
    scan_rpf(&def.code, def.variables(), &ops, &mut published, &mut out);
    out.sort_unstable();
    out.dedup();
    out
}

/// Is `v` a `&` parameter — a place a set writes THROUGH, into storage the caller owns?
fn is_ref_param(vars: &Function, v: u16) -> bool {
    v < vars.count() && matches!(vars.tp(v), Type::RefVar(_))
}

/// The work-ref buffers a call's argument list hands to the callee. Named by convention
/// (`__ref_N` / `__rref_N`), the same key `scan_set` pairs its witness off.
fn work_ref_args(rhs: &Value, vars: &Function, out: &mut Vec<u16>) {
    let Value::Call(_, args) = rhs.unspan() else {
        return;
    };
    for arg in args {
        let av = match arg.unspan() {
            Value::Var(av) | Value::Set(av, _) => *av,
            _ => continue,
        };
        if av < vars.count() {
            let n = vars.name(av);
            if n.starts_with("__ref_") || n.starts_with("__rref_") {
                out.push(av);
            }
        }
    }
}

/// Path-sensitive walk: track the work-ref buffers PUBLISHED through a `&` parameter on
/// the current path, and flag each one a plain `OpFreeRef` then frees.
fn scan_rpf(
    node: &Value,
    vars: &Function,
    ops: &FreeOps,
    published: &mut HashSet<u16>,
    out: &mut Vec<u16>,
) {
    match node.unspan() {
        Value::Block(bl) | Value::Loop(bl) => {
            for op in &bl.operators {
                scan_rpf(op, vars, ops, published, out);
            }
        }
        Value::Insert(items) => {
            for op in items {
                scan_rpf(op, vars, ops, published, out);
            }
        }
        Value::Set(v, rhs) => {
            scan_rpf(rhs, vars, ops, published, out);
            if is_ref_param(vars, *v) {
                let mut buffers = Vec::new();
                work_ref_args(rhs, vars, &mut buffers);
                published.extend(buffers);
            }
        }
        Value::If(c, t, e) => {
            scan_rpf(c, vars, ops, published, out);
            let mut pt = published.clone();
            scan_rpf(t, vars, ops, &mut pt, out);
            let mut pe = published.clone();
            scan_rpf(e, vars, ops, &mut pe, out);
            // MAY-alias: published on either arm is published for the free that follows.
            *published = pt.union(&pe).copied().collect();
        }
        Value::Call(d, args) if ops.fr.contains(d) => {
            if let Some(Value::Var(s)) = args.first().map(Value::unspan)
                && published.contains(s)
            {
                out.push(*s);
            }
        }
        Value::Call(d, args) if ops.fif.contains(d) || *d == ops.db => {
            // IfDistinct is the SAFE free; OpDatabase re-allocates — both end the alias.
            if let Some(Value::Var(s)) = args.first().map(Value::unspan) {
                published.remove(s);
            }
        }
        Value::Return(r) | Value::Drop(r) => scan_rpf(r, vars, ops, published, out),
        Value::Span(b) => scan_rpf(&b.1, vars, ops, published, out),
        _ => {}
    }
}

/// The scope-free / alloc op numbers the return-source walk keys off — read off
/// [`OpSets`], the one home of the free-op notion, so a new spelling reaches this walk too.
struct FreeOps {
    fr: std::sync::Arc<HashSet<u32>>, // the UNCONDITIONAL reference frees
    ft: u32,                          // OpFreeText
    fif: std::sync::Arc<HashSet<u32>>, // the witness-GUARDED frees (the SAFE conditional ones)
    db: u32,                          // OpDatabase (re-alloc)
}

/// Path-sensitive walk: track the stores plain-`OpFreeRef`-freed on the current path
/// (an `OpFreeRefIfDistinct` or a re-`OpDatabase` clears one), and at each `Return R`
/// flag any record `R` aliases that is currently in the freed set. Path-sensitive so a
/// plain free on a `return null` path (where the freed record is NOT the return) is not
/// a false positive — only a free of the store that IS the returned value is a bug.
fn scan_rsf(
    node: &Value,
    code: &Value,
    ops: &FreeOps,
    freed: &mut HashSet<u16>,
    out: &mut Vec<u16>,
) {
    match node.unspan() {
        Value::Block(bl) | Value::Loop(bl) => {
            for op in &bl.operators {
                scan_rsf(op, code, ops, freed, out);
            }
        }
        Value::Insert(items) => {
            for op in items {
                scan_rsf(op, code, ops, freed, out);
            }
        }
        Value::Set(_, rhs) => scan_rsf(rhs, code, ops, freed, out),
        Value::If(c, t, e) => {
            scan_rsf(c, code, ops, freed, out);
            let mut ft = freed.clone();
            scan_rsf(t, code, ops, &mut ft, out);
            let mut fe = freed.clone();
            scan_rsf(e, code, ops, &mut fe, out);
            *freed = ft.intersection(&fe).copied().collect();
        }
        Value::Call(d, args) if ops.fr.contains(d) => {
            if let Some(Value::Var(s)) = args.first().map(Value::unspan) {
                freed.insert(*s);
            }
        }
        Value::Call(d, args) if ops.fif.contains(d) || *d == ops.db => {
            // IfDistinct is the SAFE free; OpDatabase re-allocates — both clear the store.
            if let Some(Value::Var(s)) = args.first().map(Value::unspan) {
                freed.remove(s);
            }
        }
        Value::Return(r) | Value::Drop(r) => {
            let mut sources: HashSet<u16> = HashSet::new();
            let mut seen: HashSet<u16> = HashSet::new();
            ret_alias_sources(r, code, ops, &mut seen, &mut sources);
            for &s in &sources {
                if freed.contains(&s) {
                    out.push(s);
                }
            }
        }
        Value::Span(b) => scan_rsf(&b.1, code, ops, freed, out),
        _ => {}
    }
}

fn is_free_op(node: &Value, fops: &FreeOps) -> bool {
    matches!(node.unspan(), Value::Call(d, _) if fops.fr.contains(d) || *d == fops.ft || fops.fif.contains(d))
}

/// The last op of a sequence that carries the block's VALUE — skipping trailing
/// scope-exit frees and `Line` markers (the same rule as `scopes::last_non_free_result`).
fn last_value_op<'a>(ops: &'a [Value], fops: &FreeOps) -> Option<&'a Value> {
    ops.iter()
        .rev()
        .find(|op| !is_free_op(op, fops) && !matches!(op.unspan(), Value::Line(_)))
}

/// Collect the record work-refs the return value aliases: a returned `Var` is traced
/// through its last `Set` to the RHS (the arm Objects), a returned `If` unions both arms,
/// a block yields its last value op. A var with no `Set` assignment is a leaf source
/// (a directly-built work-ref or a param — harmless if not actually freed).
fn ret_alias_sources(
    node: &Value,
    code: &Value,
    fops: &FreeOps,
    seen: &mut HashSet<u16>,
    out: &mut HashSet<u16>,
) {
    match node.unspan() {
        Value::Var(v) => {
            if seen.insert(*v) {
                if let Some(rhs) = last_set_rhs(*v, code) {
                    ret_alias_sources(rhs, code, fops, seen, out);
                } else {
                    out.insert(*v);
                }
            }
        }
        Value::If(_, t, e) => {
            ret_alias_sources(t, code, fops, seen, out);
            ret_alias_sources(e, code, fops, seen, out);
        }
        Value::Block(bl) | Value::Loop(bl) => {
            if let Some(l) = last_value_op(&bl.operators, fops) {
                ret_alias_sources(l, code, fops, seen, out);
            }
        }
        Value::Insert(ops) => {
            if let Some(l) = last_value_op(ops, fops) {
                ret_alias_sources(l, code, fops, seen, out);
            }
        }
        Value::Span(b) => ret_alias_sources(&b.1, code, fops, seen, out),
        _ => {}
    }
}

/// The RHS of the LAST `Set(v, rhs)` for var `v` anywhere in `code` (pre-order; later
/// wins). Explicit recursion rather than `for_each_child` — the callback form cannot
/// escape a `&'a` reference to the RHS.
fn last_set_rhs<'a>(v: u16, code: &'a Value) -> Option<&'a Value> {
    let mut found: Option<&'a Value> = None;
    collect_last_set(v, code, &mut found);
    found
}

fn collect_last_set<'a>(v: u16, node: &'a Value, found: &mut Option<&'a Value>) {
    match node {
        Value::Set(t, rhs) => {
            // Skip the hoisted null-init `Set(v, Null)`: a record work-ref is BUILT by
            // `OpDatabase` (a Call, not a Set), and its only `Set` is the declaration
            // `= null`. Tracing through that loses the build and drops the source (35c).
            // The last NON-null Set is the real value (`__ret_1 = if{…}`).
            if *t == v && !matches!(rhs.unspan(), Value::Null) {
                *found = Some(rhs);
            }
            collect_last_set(v, rhs, found);
        }
        Value::Block(bl) | Value::Loop(bl) => {
            for o in &bl.operators {
                collect_last_set(v, o, found);
            }
        }
        Value::Insert(ops)
        | Value::Call(_, ops)
        | Value::CallRef(_, ops)
        | Value::Tuple(ops)
        | Value::Parallel(ops) => {
            for o in ops {
                collect_last_set(v, o, found);
            }
        }
        Value::If(c, t, e) => {
            collect_last_set(v, c, found);
            collect_last_set(v, t, found);
            collect_last_set(v, e, found);
        }
        Value::Return(inner)
        | Value::Drop(inner)
        | Value::Yield(inner)
        | Value::TuplePut(_, _, inner) => collect_last_set(v, inner, found),
        Value::Iter(_, a, b, c) => {
            collect_last_set(v, a, found);
            collect_last_set(v, b, found);
            collect_last_set(v, c, found);
        }
        Value::Span(b) => collect_last_set(v, &b.1, found),
        _ => {}
    }
}

#[cfg(test)]
mod return_source_tests {
    //! Injected-fault controls for the path-sensitive return-source-free walk (`scan_rsf`),
    //! parser-free — the compiler no longer emits the bug, so the positive control must be
    //! synthetic. Op-numbers are arbitrary; the walk only compares against them.
    use super::{FreeOps, scan_rsf};
    use crate::data::{Type, Value, v_block, v_if, v_set};
    use std::collections::HashSet;

    const FR: u32 = 100;
    const FT: u32 = 101;
    const FIF: u32 = 102;
    const DB: u32 = 103;
    fn ops() -> FreeOps {
        FreeOps {
            fr: std::sync::Arc::new(HashSet::from([FR])),
            ft: FT,
            fif: std::sync::Arc::new(HashSet::from([FIF])),
            db: DB,
        }
    }
    fn run(code: &Value) -> Vec<u16> {
        let mut out = Vec::new();
        let mut freed = HashSet::new();
        scan_rsf(code, code, &ops(), &mut freed, &mut out);
        out.sort_unstable();
        out.dedup();
        out
    }
    fn free(s: u16) -> Value {
        Value::Call(FR, vec![Value::Var(s)])
    }
    fn free_if(s: u16, r: u16) -> Value {
        Value::Call(FIF, vec![Value::Var(s), Value::Var(r)])
    }
    fn ret(v: u16) -> Value {
        Value::Return(Box::new(Value::Var(v)))
    }
    // A record work-ref built into a block that yields it (the arm `Object`).
    fn obj(v: u16) -> Value {
        v_block(vec![Value::Var(v)], Type::Void, "Object")
    }

    /// POSITIVE — `ret` aliases {1,2}; both plain-freed before `return ret` (the 35c bug).
    #[test]
    fn flags_plain_free_of_return_source() {
        let code = v_block(
            vec![
                v_set(10, v_if(Value::Var(0), obj(1), obj(2))),
                free(1),
                free(2),
                ret(10),
            ],
            Type::Void,
            "body",
        );
        assert_eq!(run(&code), vec![1, 2]);
    }

    /// NEGATIVE — `OpFreeRefIfDistinct` is the safe conditional free (post-fix shape).
    #[test]
    fn silent_on_free_if_distinct() {
        let code = v_block(
            vec![
                v_set(10, v_if(Value::Var(0), obj(1), obj(2))),
                free_if(1, 10),
                free_if(2, 10),
                ret(10),
            ],
            Type::Void,
            "body",
        );
        assert!(run(&code).is_empty());
    }

    /// NEGATIVE — path-sensitive: a plain free on a `return null` path (the freed record is
    /// NOT the return there) is safe — the 497/98 false-positive shape.
    #[test]
    fn silent_when_other_path_returns_it() {
        let code = v_block(
            vec![v_if(
                Value::Var(0),
                v_block(vec![free_if(2, 1), ret(1)], Type::Void, "then"),
                v_block(
                    vec![free(1), free(2), Value::Return(Box::new(Value::Null))],
                    Type::Void,
                    "else",
                ),
            )],
            Type::Void,
            "body",
        );
        assert!(run(&code).is_empty());
    }

    /// NEGATIVE — re-`OpDatabase` between the free and the return clears it.
    #[test]
    fn silent_after_realloc_before_return() {
        let code = v_block(
            vec![free(1), Value::Call(DB, vec![Value::Var(1)]), ret(1)],
            Type::Void,
            "body",
        );
        assert!(run(&code).is_empty());
    }
}

#[cfg(test)]
mod ref_param_publish_tests {
    //! Injected-fault controls for the `&`-publish walk (`scan_rpf`, loft#759). The
    //! compiler no longer emits the bug, so the positive control is synthetic — without
    //! one, "no findings" would read the same whether the walk is clean or blind.
    use super::{FreeOps, scan_rpf};
    use crate::data::{Deps, Type, Value, v_block, v_if, v_set};
    use crate::variables::Function;
    use std::collections::HashSet;

    const FR: u32 = 100;
    const FT: u32 = 101;
    const FIF: u32 = 102;
    const DB: u32 = 103;
    const CALLEE: u32 = 200;

    fn ops() -> FreeOps {
        FreeOps {
            fr: std::sync::Arc::new(HashSet::from([FR])),
            ft: FT,
            fif: std::sync::Arc::new(HashSet::from([FIF])),
            db: DB,
        }
    }

    /// `f` (var 0) is a `&Rec` parameter, `__ref_1` (var 1) the work-ref buffer, and
    /// `loc` (var 2) a plain local of the same record type — the a1-vs-a2 contrast.
    fn vars() -> Function {
        let rec = Type::Reference(1, Deps::none());
        let mut f = Function::new("reb", "t.loft");
        f.add_temp_var("f", &Type::RefVar(Box::new(rec.clone())));
        f.add_temp_var("__ref_1", &rec);
        f.add_temp_var("loc", &rec);
        f
    }

    fn run(code: &Value) -> Vec<u16> {
        let mut out = Vec::new();
        let mut published = HashSet::new();
        scan_rpf(code, &vars(), &ops(), &mut published, &mut out);
        out.sort_unstable();
        out.dedup();
        out
    }

    /// `<target> = callee(__ref_1)` — the publish.
    fn publish(target: u16) -> Value {
        v_set(target, Value::Call(CALLEE, vec![Value::Var(1)]))
    }
    fn free(s: u16) -> Value {
        Value::Call(FR, vec![Value::Var(s)])
    }
    fn free_if(s: u16, w: u16) -> Value {
        Value::Call(FIF, vec![Value::Var(s), Value::Var(w)])
    }

    /// POSITIVE — published through the `&`, then plain-freed: the loft#759 shape.
    #[test]
    fn flags_plain_free_after_ref_param_publish() {
        let code = v_block(vec![publish(0), free(1)], Type::Void, "body");
        assert_eq!(run(&code), vec![1]);
    }

    /// NEGATIVE — `OpFreeRefIfDistinct` is the safe form the fix emits.
    #[test]
    fn silent_on_free_if_distinct() {
        let code = v_block(vec![publish(0), free_if(1, 0)], Type::Void, "body");
        assert!(run(&code).is_empty());
    }

    /// NEGATIVE — the same call assigned to a LOCAL deep-copies into a store the local
    /// owns, so the buffer is genuinely this frame's to free (matrix cell a1).
    #[test]
    fn silent_when_target_is_a_local() {
        let code = v_block(vec![publish(2), free(1)], Type::Void, "body");
        assert!(run(&code).is_empty());
    }

    /// POSITIVE — published on ONE arm only. MAY-alias: the free after the join still
    /// hits the caller's record on the arm that published (matrix cell c1).
    #[test]
    fn flags_free_after_conditional_publish() {
        let code = v_block(
            vec![v_if(Value::Var(9), publish(0), Value::Null), free(1)],
            Type::Void,
            "body",
        );
        assert_eq!(run(&code), vec![1]);
    }

    /// NEGATIVE — nothing published at all; a bare buffer free is the normal shape.
    #[test]
    fn silent_without_a_publish() {
        let code = v_block(vec![free(1)], Type::Void, "body");
        assert!(run(&code).is_empty());
    }
}
