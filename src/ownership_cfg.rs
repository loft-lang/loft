//! @PLN94 Phase 1 — a control-flow graph over loft's structured Value-IR, and (Phase 2+)
//! a monotone dataflow fixpoint over it, run BESIDE the shipped analysis as an independent
//! completeness oracle. This module is an OBSERVER: it never rewrites IR and nothing in the
//! normal compile path consumes it — it is reached only via `LOFT_OWN_ORACLE` (SI-1: shipped
//! codegen stays byte-identical). Plan: `doc/claude/plans/94-cfg-ownership-dataflow/`.
//!
//! @I60 — Scope & dependency/lifetime tracker (deps): the independent flow-sensitive cross-check tier.
//!
//! loft's control flow is STRUCTURED (`If` / `Loop` / `Break(n)` / `Continue(n)` / `Return`,
//! no gotos; `for` lowers to a `Loop` whose exit test is an `If(cond, Break(0), Null)` buried
//! in the range `Set`). So the CFG is built by a syntax-directed walk, not a MIR lowering:
//! statement-level `If` and control-carrying `If` split into then/else/join blocks; a `Loop`
//! gets a header (with the back-edge) and an exit block (reached via `Break`); `Break(n)` /
//! `Continue(n)` / `Return` add the corresponding edge and end the current straight line.

use crate::data::{Data, DefType, Value};
use crate::use_analysis::{Own, classifies_structurally, ownership_of, return_ownership};
use std::collections::{BTreeMap, BTreeSet};

type BlockId = usize;

/// One basic block: a maximal straight-line run plus its control successors.
#[derive(Default)]
struct Bb {
    label: &'static str,
    /// variables defined (assigned) in this block — the raw material for reaching-defs (1.2).
    defs: Vec<u16>,
    /// (var, RHS) per assignment in program order — the ownership pass (Phase 2) classifies each
    /// RHS structurally (`ownership_of`) or, for a `Var` RHS, resolves it flow-sensitively.
    owns: Vec<(u16, Value)>,
    /// (position in `owns`, var, op) per statement-level call whose first operand is a variable
    /// (`OpFreeRef(b)`), so a check can read the facts AT the call ([`facts_at_frees`]).
    calls_on: Vec<(usize, u16, u32)>,
    /// short op labels, for the `cfg` dump / hand-verification only.
    ops: Vec<String>,
    succ: Vec<BlockId>,
}

/// A function's control-flow graph. Blocks are index-addressed (no references), so the graph
/// builds without borrow-checker friction.
pub struct Cfg {
    blocks: Vec<Bb>,
    entry: BlockId,
    exit: BlockId,
}

struct LoopCtx {
    header: BlockId,
    exit: BlockId,
}

/// Whether an `If` is being walked as a statement (a direct element of a block's operator
/// sequence — always a branch, since arms may define vars differently) or as a value (only a
/// branch when it carries a control transfer; a pure scalar `if a 0 else 1` stays atomic).
#[derive(Clone, Copy, PartialEq)]
enum Ctx {
    Stmt,
    Val,
}

struct Builder {
    cfg: Cfg,
    loops: Vec<LoopCtx>,
}

impl Builder {
    fn new_block(&mut self, label: &'static str) -> BlockId {
        let id = self.cfg.blocks.len();
        self.cfg.blocks.push(Bb {
            label,
            ..Default::default()
        });
        id
    }

    fn edge(&mut self, from: BlockId, to: BlockId) {
        if !self.cfg.blocks[from].succ.contains(&to) {
            self.cfg.blocks[from].succ.push(to);
        }
    }

    fn op(&mut self, b: BlockId, label: String) {
        self.cfg.blocks[b].ops.push(label);
    }

    fn def(&mut self, b: BlockId, var: u16) {
        if !self.cfg.blocks[b].defs.contains(&var) {
            self.cfg.blocks[b].defs.push(var);
        }
    }

    /// Record an assignment's (var, RHS) in program order for the ownership pass.
    fn record_own(&mut self, b: BlockId, var: u16, rhs: &Value) {
        self.cfg.blocks[b].owns.push((var, rhs.clone()));
    }

    /// Does `v` contain a control transfer (`Break`/`Continue`/`Return`) targeting THIS or an
    /// enclosing loop/function — i.e. not one consumed by a nested `Loop`? Used to decide
    /// whether a `Set` RHS (or a value-context `If`) must split the CFG. A nested `Loop`'s own
    /// `Break(0)` targets itself, so we do not descend into nested loops (labeled `Break(n>0)`
    /// escaping an inner loop is a known-deferred edge case — see IMPL.md 1.3).
    fn has_transfer(v: &Value) -> bool {
        match v.unspan() {
            Value::Return(_) | Value::Break(_) | Value::Continue(_) => true,
            Value::Loop(_) => false,
            other => {
                let mut found = false;
                other.for_each_child(&mut |c| found |= Self::has_transfer(c));
                found
            }
        }
    }

    /// Record every `Set` target inside `v` as a def of block `b` (for an atomic value-`If`,
    /// whose arms may still assign vars even though it does not split control).
    fn collect_defs(&mut self, v: &Value, b: BlockId) {
        let mut vars: Vec<u16> = Vec::new();
        v.walk(&mut |x| {
            if let Value::Set(n, _) = x {
                vars.push(*n);
            }
        });
        for n in vars {
            self.def(b, n);
        }
    }

    /// Walk a statement sequence starting in `cur`; return the block control falls through to,
    /// or `None` if the sequence diverges (ends in return/break/continue).
    fn walk_seq(&mut self, ops: &[Value], cur: BlockId) -> Option<BlockId> {
        let mut c = Some(cur);
        for op in ops {
            let Some(cc) = c else { break };
            c = self.walk_stmt(op, cc, Ctx::Stmt);
        }
        c
    }

    /// Walk one value as a statement; returns the fall-through block (`None` on divergence).
    fn walk_stmt(&mut self, v: &Value, cur: BlockId, ctx: Ctx) -> Option<BlockId> {
        match v.unspan() {
            Value::Block(b) => self.walk_seq(&b.operators, cur),
            Value::Insert(ops) => self.walk_seq(ops, cur),
            Value::Return(_) => {
                self.op(cur, "Return".into());
                let e = self.cfg.exit;
                self.edge(cur, e);
                None
            }
            Value::Break(n) => {
                self.op(cur, "Break".into());
                let t = self.loop_exit(*n);
                self.edge(cur, t);
                None
            }
            Value::Continue(n) => {
                self.op(cur, "Continue".into());
                let t = self.loop_header(*n);
                self.edge(cur, t);
                None
            }
            Value::Loop(body) => {
                let header = self.new_block("loop_hdr");
                self.edge(cur, header);
                let exit = self.new_block("loop_exit");
                self.loops.push(LoopCtx { header, exit });
                let body_end = self.walk_seq(&body.operators, header);
                if let Some(be) = body_end {
                    self.edge(be, header); // back-edge
                }
                self.loops.pop();
                Some(exit)
            }
            Value::If(_, t, e) => {
                let branch = ctx == Ctx::Stmt || Self::has_transfer(t) || Self::has_transfer(e);
                if !branch {
                    // pure scalar expression-`if`: no control split, but its arms may assign.
                    self.op(cur, "if(expr)".into());
                    self.collect_defs(t, cur);
                    self.collect_defs(e, cur);
                    return Some(cur);
                }
                self.op(cur, "if-cond".into());
                let tb = self.new_block("then");
                let eb = self.new_block("else");
                self.edge(cur, tb);
                self.edge(cur, eb);
                let te = self.walk_stmt(t, tb, Ctx::Stmt);
                let ee = self.walk_stmt(e, eb, Ctx::Stmt);
                if te.is_none() && ee.is_none() {
                    None // both arms diverge — nothing falls through
                } else {
                    let join = self.new_block("join");
                    if let Some(x) = te {
                        self.edge(x, join);
                    }
                    if let Some(y) = ee {
                        self.edge(y, join);
                    }
                    Some(join)
                }
            }
            Value::Set(var, val) => {
                if Self::has_transfer(val) {
                    // RHS carries control flow (the `for` range `Set` holds the exit `Break`).
                    let after = self.walk_stmt(val, cur, Ctx::Val)?;
                    self.def(after, *var);
                    self.record_own(after, *var, val);
                    self.op(after, format!("Set v{var}"));
                    Some(after)
                } else {
                    self.def(cur, *var);
                    self.record_own(cur, *var, val);
                    self.op(cur, format!("Set v{var}"));
                    Some(cur)
                }
            }
            other => {
                if Self::has_transfer(other) {
                    // A straight-line op whose child carries a transfer (degenerate in structured
                    // loft IR — breaks/returns live in statement/`If`/`Set`-RHS positions). Walk
                    // the children in order, in place, so no child ref escapes the closure.
                    let mut c = Some(cur);
                    other.for_each_child(&mut |k| {
                        if let Some(cc) = c {
                            c = self.walk_stmt(k, cc, Ctx::Val);
                        }
                    });
                    c
                } else {
                    if let Value::Call(d, args) = other
                        && let Some(Value::Var(b)) = args.first().map(Value::unspan)
                    {
                        let at = self.cfg.blocks[cur].owns.len();
                        self.cfg.blocks[cur].calls_on.push((at, *b, *d));
                    }
                    self.op(cur, Self::op_label(other));
                    Some(cur)
                }
            }
        }
    }

    /// `Break(n)` / `Continue(n)`: `n == 0` is the innermost loop (top of the stack).
    fn loop_exit(&self, n: u16) -> BlockId {
        self.loops[self.loops.len() - 1 - n as usize].exit
    }
    fn loop_header(&self, n: u16) -> BlockId {
        self.loops[self.loops.len() - 1 - n as usize].header
    }

    fn op_label(v: &Value) -> String {
        match v {
            Value::Var(n) => format!("Var v{n}"),
            Value::Call(d, _) => format!("Call#{d}"),
            Value::CallRef(n, _) => format!("CallRef v{n}"),
            Value::Drop(_) => "Drop".into(),
            Value::Line(_) => "Line".into(),
            Value::Null => "Null".into(),
            _ => "op".into(),
        }
    }
}

/// Build the CFG for a function body (an owned clone, so it is not borrowed from `Data`).
fn build(body: &Value) -> Cfg {
    let cfg = Cfg {
        blocks: Vec::new(),
        entry: 0,
        exit: 0,
    };
    let mut b = Builder {
        cfg,
        loops: Vec::new(),
    };
    let entry = b.new_block("entry");
    let exit = b.new_block("exit");
    b.cfg.entry = entry;
    b.cfg.exit = exit;
    // The function body is a `Block` (possibly `Span`-wrapped).
    let end = match body.unspan() {
        Value::Block(bl) => b.walk_seq(&bl.operators, entry),
        other => b.walk_stmt(other, entry, Ctx::Stmt),
    };
    if let Some(e) = end {
        b.edge(e, exit); // implicit fall-through return
    }
    b.cfg
}

fn dump(name: &str, cfg: &Cfg) {
    eprintln!(
        "CFG {name}  blocks={} entry=b{} exit=b{}",
        cfg.blocks.len(),
        cfg.entry,
        cfg.exit
    );
    for (i, bb) in cfg.blocks.iter().enumerate() {
        let succ = if bb.succ.is_empty() {
            "(none)".to_string()
        } else {
            bb.succ
                .iter()
                .map(|s| format!("b{s}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let defs = if bb.defs.is_empty() {
            String::new()
        } else {
            format!(
                "  defs=[{}]",
                bb.defs
                    .iter()
                    .map(|v| format!("v{v}"))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        eprintln!(
            "  b{i} [{}] -> {succ}{defs}  {}",
            bb.label,
            bb.ops.join(" · ")
        );
    }
}

// ============================================================================
// @PLN94 Phase 1.2 — the worklist dataflow engine, exercised on the classic trivial lattice
// (REACHING DEFINITIONS): a forward may-analysis over sets of definition sites, meet = union.
// `OUT[b] = gen[b] ∪ (IN[b] \ kill[b])`, `IN[b] = ⋃ OUT[preds]`. Monotone over a finite set of
// sites ⇒ the round-robin fixpoint converges (SI-3 asserts a bound so non-termination is loud,
// not a hang). This is the engine the ownership fact (Phase 2) rides on; reaching-defs is here
// only to validate the fixpoint mechanics against hand-computable expectations.
// ============================================================================

/// A definition site: variable `var` assigned in block `block` (block-granular).
struct Site {
    var: u16,
    block: BlockId,
}

struct ReachInfo {
    sites: Vec<Site>,
    inb: Vec<BTreeSet<usize>>,
    outb: Vec<BTreeSet<usize>>,
    passes: usize,
}

fn reaching_defs(cfg: &Cfg) -> ReachInfo {
    let n = cfg.blocks.len();
    // Enumerate def sites (var, block) — one per (var defined, block).
    let mut sites: Vec<Site> = Vec::new();
    for (b, bb) in cfg.blocks.iter().enumerate() {
        for &v in &bb.defs {
            sites.push(Site { var: v, block: b });
        }
    }
    // Predecessors, inverted from successors.
    let mut preds: Vec<Vec<BlockId>> = vec![Vec::new(); n];
    for (b, bb) in cfg.blocks.iter().enumerate() {
        for &s in &bb.succ {
            preds[s].push(b);
        }
    }
    // gen[b] = sites in b; kill[b] = every OTHER site of a var b (re)defines.
    let mut gens: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    let mut kill: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    for (si, s) in sites.iter().enumerate() {
        gens[s.block].insert(si);
    }
    for (b, bb) in cfg.blocks.iter().enumerate() {
        for (si, s) in sites.iter().enumerate() {
            if s.block != b && bb.defs.contains(&s.var) {
                kill[b].insert(si);
            }
        }
    }
    // Round-robin fixpoint in program order (≈ RPO for a mostly-structured CFG → fast).
    let mut inb: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    let mut outb: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    let mut passes = 0usize;
    loop {
        passes += 1;
        // SI-3: a monotone analysis over `sites` converges; the bound turns a bug that would
        // hang into a loud panic instead. `n + 2` is generous (convergence is ~loop-depth+2).
        assert!(
            passes <= n + 2,
            "reaching-defs did not converge in {passes} passes (n={n}) — monotonicity bug"
        );
        let mut changed = false;
        for b in 0..n {
            let mut nin = BTreeSet::new();
            for &p in &preds[b] {
                nin.extend(outb[p].iter().copied());
            }
            inb[b] = nin;
            let mut nout = gens[b].clone();
            for &d in &inb[b] {
                if !kill[b].contains(&d) {
                    nout.insert(d);
                }
            }
            if nout != outb[b] {
                outb[b] = nout;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    ReachInfo {
        sites,
        inb,
        outb,
        passes,
    }
}

fn dump_reach(name: &str, cfg: &Cfg, rd: &ReachInfo) {
    let fmt = |set: &BTreeSet<usize>| {
        if set.is_empty() {
            "{}".to_string()
        } else {
            let items: Vec<String> = set
                .iter()
                .map(|&d| format!("v{}@b{}", rd.sites[d].var, rd.sites[d].block))
                .collect();
            format!("{{{}}}", items.join(","))
        }
    };
    eprintln!(
        "RD {name}  blocks={} sites={} passes={}",
        cfg.blocks.len(),
        rd.sites.len(),
        rd.passes
    );
    for b in 0..cfg.blocks.len() {
        eprintln!(
            "  b{b} [{}]  in={}  out={}",
            cfg.blocks[b].label,
            fmt(&rd.inb[b]),
            fmt(&rd.outb[b])
        );
    }
}

// ============================================================================
// @PLN94 Phase 2 — the forward OWNERSHIP fact, flow-sensitive, on the CFG fixpoint. The lattice
// mirrors the shipped `Own` (Owned | Borrowed(base) | Join(base)) plus a `Bottom` (unreached).
// Where the shipped classifier is flow-INSENSITIVE (a var = the join of ALL its defs), this is
// per-program-point: the meet happens only at the arm-joins that actually merge, so a var whose
// reaching def is single here classifies precisely instead of collapsing to `Join`. The
// per-def transfer REUSES the shipped `ownership_of` for a structural RHS (OpDatabase → Owned,
// projection → Borrowed(base), …); a bare `Var` RHS resolves flow-sensitively to the source's
// current state. Shadow-diffed against `ownership_of` — must AGREE where there is no flow.
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum OFact {
    Bottom,
    Owned,
    Borrowed(u16),
    Join(u16),
}

impl OFact {
    fn from_own(o: Own) -> OFact {
        match o {
            // @PLN155 phase 2 — the shadow lattice has no "no answer" element, and giving it
            // one is a change to the fixpoint rather than a transcription.  `Owned` keeps the
            // answer this shadow read before `Unknown` existed, so the two derivations still
            // disagree in exactly the places the `own` dump's shadow-diff already reports.
            Own::Owned | Own::Unknown => OFact::Owned,
            Own::Borrowed { base } => OFact::Borrowed(base),
            Own::Join { base } => OFact::Join(base),
        }
    }

    /// Lattice meet (⊔) at a control-flow merge: `Bottom` is the identity; `Owned` merged with a
    /// `Borrowed`/different-base becomes runtime-dependent `Join`; `Join` dominates.
    fn meet(a: OFact, b: OFact) -> OFact {
        use OFact::{Borrowed, Bottom, Join, Owned};
        match (a, b) {
            (Bottom, x) | (x, Bottom) => x,
            (Owned, Owned) => Owned,
            (Borrowed(b1), Borrowed(b2)) if b1 == b2 => Borrowed(b1),
            // Borrowed on every path, of different stores: no path owns, so no free is ever
            // licensed — still `Borrowed`, with no one base to name (`ownership_of`'s answer).
            (Borrowed(_), Borrowed(_)) => Borrowed(u16::MAX),
            (Join(bb), _) | (_, Join(bb)) => Join(bb),
            // remaining: Owned×Borrowed — runtime-dependent.
            (Borrowed(bb), _) | (_, Borrowed(bb)) => Join(bb),
        }
    }

    fn show(self) -> String {
        match self {
            OFact::Bottom => "⊥".into(),
            OFact::Owned => "Owned".into(),
            OFact::Borrowed(b) => format!("Borrowed(v{b})"),
            OFact::Join(b) => format!("Join(v{b})"),
        }
    }

    /// Does `self` REFINE `other` (`self ⊑ other`, i.e. `self` is at least as precise)? A definite
    /// `Owned`/`Borrowed` refines the runtime-dependent `Join`; `Bottom` refines everything. Used
    /// to split a shadow-diff into a PRECISION win (this fact ⊏ the shipped `Join`) vs an unsound
    /// or coarser DISAGREE-ment (which must be zero — the soundness direction, tightened in 3.5).
    fn refines(self, other: OFact) -> bool {
        self == other
            || matches!(
                (self, other),
                (OFact::Bottom, _) | (OFact::Owned | OFact::Borrowed(_), OFact::Join(_))
            )
    }
}

type OState = BTreeMap<u16, OFact>;

/// @PLN94 (3.3) — consume a callee's return-ownership SUMMARY at a call site, INDEPENDENTLY of
/// the shipped classifier (so the oracle can eventually disagree with it on calls): `Owned` →
/// the result is owned; a borrowed/join-of-param return maps back to the caller's argument var.
/// Mirrors `use_analysis::call_ownership` so it agrees where the shipped fact is right.
fn call_own(data: &Data, d_nr: u32, callee_d: u32, args: &[Value]) -> OFact {
    match return_ownership(data, callee_d) {
        // @PLN155 phase 2 — as in `from_own`: the shadow keeps its pre-`Unknown` answer, so a
        // disagreement it reports stays a disagreement about the FACT and not about this
        // refactor.
        Own::Owned | Own::Unknown => OFact::Owned,
        Own::Borrowed { base } => {
            OFact::Borrowed(caller_arg_base(data, d_nr, callee_d, base, args))
        }
        Own::Join { base } => OFact::Join(caller_arg_base(data, d_nr, callee_d, base, args)),
    }
}

/// Map the callee's borrowed parameter (`callee_base`, an attribute index) to the caller's
/// variable it arrives in — through the ONE translation the oracle reads
/// (`use_analysis::structural_arg_base`), so the shadow cannot drift from it on the shapes
/// loft#1318 fixed (a hidden delivery buffer is nameable; a projection roots at its container).
/// A call-shaped argument is what only the oracle can root, and the shadow asks it for that
/// one value exactly as the oracle asks itself — the independence this file keeps is in the
/// FLOW, not in the translation.
fn caller_arg_base(data: &Data, d_nr: u32, callee_d: u32, callee_base: u16, args: &[Value]) -> u16 {
    match crate::use_analysis::structural_arg_base(data, callee_d, callee_base, args) {
        Ok(base) => base,
        Err(arg) => match ownership_of(data, d_nr, arg) {
            Own::Borrowed { base } | Own::Join { base } => base,
            // @PLN155 phase 2 — `u16::MAX` is already this function's "no nameable base", and
            // that is precisely what `Unknown` means, so the two share the arm honestly rather
            // than by omission.
            Own::Owned | Own::Unknown => u16::MAX,
        },
    }
}

/// Forward ownership fixpoint: returns each block's OUT-state and the pass count (SI-3).
/// Materialisation: a var RE-MINTED via `OpDatabase` on ANY path is a materialised
/// OWNED local (the function owns its store), so a whole-value `var = src` copy INTO it is also
/// owned — not a borrow of `src`. `OpDatabase(v)` is a `Call` statement, invisible to the `Set`-only
/// `record_own`, so the transfer would otherwise carry the false-arm copy's `Borrowed(src)` through
/// the join (the `n_choose` residual: `r = x` on the false arm, `OpDatabase(r)` on the true). This
/// is NARROW — it only reclasses a bare `Var` RHS, never a projection (`OpGetField` → a genuine
/// borrowing view stays `Borrowed`, so the A1b catch on the returned work-ref is preserved).
fn reminted_vars(data: &Data, d_nr: u32) -> BTreeSet<u16> {
    let op_database = data.def_nr("OpDatabase");
    let mut reminted: BTreeSet<u16> = BTreeSet::new();
    data.def(d_nr).code.walk(&mut |x| {
        if let Value::Call(d, args) = x
            && *d == op_database
            && let Some(Value::Var(v)) = args.first().map(Value::unspan)
        {
            reminted.insert(*v);
        }
    });
    reminted
}

fn predecessors(cfg: &Cfg) -> Vec<Vec<BlockId>> {
    let mut preds: Vec<Vec<BlockId>> = vec![Vec::new(); cfg.blocks.len()];
    for (b, bb) in cfg.blocks.iter().enumerate() {
        for &s in &bb.succ {
            preds[s].push(b);
        }
    }
    preds
}

/// A block's IN state: the per-var meet of its predecessors' OUT states.
fn block_in(preds: &[BlockId], outb: &[OState]) -> OState {
    let mut st: OState = OState::new();
    for &p in preds {
        for (&v, &f) in &outb[p] {
            let cur = st.get(&v).copied().unwrap_or(OFact::Bottom);
            st.insert(v, OFact::meet(cur, f));
        }
    }
    st
}

/// One assignment's transfer: the fact `var` takes from `rhs`, given the facts `st` holds so
/// far.  The one transfer both the fixpoint and [`facts_at_frees`] replay.
fn assign(
    data: &Data,
    d_nr: u32,
    reminted: &BTreeSet<u16>,
    st: &mut OState,
    var: u16,
    rhs: &Value,
) {
    // A `= null` DECLARATION sentinel is not a real def — skip it (B's `collect_defs`
    // does the same). Recording it would default a var whose real def is nested/
    // conditional (a `??` `__ncc_N` temp) to `Owned`, the unsound over-free direction;
    // skipping lets a read fall through to `ownership_of` (B's fact) instead.  A null
    // in either spelling holds no store, and is the identity in B's join too
    // (`holds_no_store`): a null on one path does not make a view runtime-owned.
    if crate::use_analysis::holds_no_store(data, rhs) {
        return;
    }
    let f = match rhs.unspan() {
        // A whole-value `var = src` copy INTO a re-minted (materialised-owned) var owns —
        // see `reminted` above. Otherwise a bare source var resolves flow-sensitively.
        Value::Var(_) if reminted.contains(&var) => OFact::Owned,
        // …and so does one the emitters copy (`bind_copies`, the oracle's own rule).
        Value::Var(u)
            if crate::use_analysis::bind_copies(data, &data.def(d_nr).variables, var, *u) =>
        {
            OFact::Owned
        }
        Value::Var(u) => st
            .get(u)
            .copied()
            .unwrap_or_else(|| OFact::from_own(ownership_of(data, d_nr, rhs))),
        // A NON-NATIVE user-function call consumes the callee summary directly (3.3).
        // Native-bodied functions and ops fall to the structural classifier: their
        // return ownership is carried by codegen metadata (`returns_borrowed_view`),
        // not the loft body `return_ownership` reads — replicating that is 3.4's op-tail.
        // EXCLUDE the primitive STRUCTURAL ops (store mint / projection): they are
        // `DefType::Function` with an empty native body too, but `ownership_of`'s
        // classify handles them exactly (mint → Owned, projection → Borrowed(base));
        // routing them through `call_own` mis-classes a projection local `Owned` — the
        // unsound over-free direction (3.4a: `xs = OpGetField(vdb, …)` regression).
        Value::Call(callee_d, args)
            if matches!(data.def(*callee_d).def_type, DefType::Function)
                && data.def(*callee_d).native().is_empty()
                && !classifies_structurally(data, *callee_d) =>
        {
            call_own(data, d_nr, *callee_d, args)
        }
        _ => OFact::from_own(ownership_of(data, d_nr, rhs)),
    };
    // A self-borrow `Borrowed(v)` for var `v` is NOT a real borrow — it is the @P302
    // self-dep `[v]` a keyed-collection local carries so a later `s += …` re-inits in
    // place: an OWNERSHIP marker, freed at scope exit. Normalise to `Owned` (matching
    // the shipped `get_free_vars` @P302 carve-out; you cannot borrow from yourself).
    // (A flow-INSENSITIVE db_var→Owned rule was tried and REVERTED: it forces the A1b
    // return work-ref — `OpDatabase`'d for its Cell then returned as a borrowing view —
    // to `Owned`, matching B's wrong-under-`LOFT_NO_A1B` fact and LOSING the catch.)
    // Only a KEYED local carries that marker: a link re-pointed at its own child
    // (`l = l.next`) is a real projection of itself, and stays the borrow it is.
    let f = if f == OFact::Borrowed(var)
        && crate::parser::vectors::is_keyed(data.def(d_nr).variables.tp(var))
    {
        OFact::Owned
    } else {
        f
    };
    st.insert(var, f);
}

fn ownership_dataflow(data: &Data, d_nr: u32, cfg: &Cfg) -> (Vec<OState>, usize) {
    let reminted = reminted_vars(data, d_nr);
    let n = cfg.blocks.len();
    let preds = predecessors(cfg);
    let mut outb: Vec<OState> = vec![OState::new(); n];
    let mut passes = 0usize;
    loop {
        passes += 1;
        assert!(
            passes <= n + 2,
            "ownership dataflow did not converge in {passes} passes (n={n}) — monotonicity bug"
        );
        let mut changed = false;
        for b in 0..n {
            let mut st = block_in(&preds[b], &outb);
            // Apply the block's assignments in program order (a later def sees earlier ones).
            for (var, rhs) in &cfg.blocks[b].owns {
                assign(data, d_nr, &reminted, &mut st, *var, rhs);
            }
            if st != outb[b] {
                outb[b] = st;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    (outb, passes)
}

/// The shadow-diff (`own`): vars whose flow-sensitive fact does NOT refine B's `ownership_of`
/// (neither identical nor a `⊏ Join` precision win). Each is a real defect in one implementation —
/// the coexistence finding. Returns `(var, mine, theirs)`; `precision_out` accumulates the wins.
fn disagreements(
    exit_state: &OState,
    data: &Data,
    d_nr: u32,
    precision_out: &mut Vec<(u16, OFact, OFact)>,
) -> Vec<(u16, OFact, OFact)> {
    let mut disagree = Vec::new();
    for (&v, &mine) in exit_state {
        let theirs = OFact::from_own(ownership_of(data, d_nr, &Value::Var(v)));
        // AGREE at the ownership-DECISION level: the same KIND (both `Owned` / both `Borrowed` /
        // both `Join`) is the same free decision (free / don't-free / conditional), even when the
        // tracked base var differs — the base is informational, and a conditional free
        // (`OpFreeRefIfDistinct`) keys off runtime distinctness, not the static base. Only a KIND
        // mismatch that is not a precision win (`⊏ Join`) is a real ownership disagreement.
        if std::mem::discriminant(&mine) == std::mem::discriminant(&theirs) {
            // agree (same free decision)
        } else if mine.refines(theirs) {
            precision_out.push((v, mine, theirs));
        } else {
            disagree.push((v, mine, theirs));
        }
    }
    disagree
}

/// Run the ownership fixpoint, dump per-block OUT-states, and SHADOW-DIFF each var's fact at the
/// function exit against the shipped flow-insensitive `ownership_of` — the 3.1 gate (agree on
/// straight-line) and the surface where the flow-sensitive `Join` precision shows up later.
fn dump_own(name: &str, cfg: &Cfg, data: &Data, d_nr: u32) {
    let (outb, passes) = ownership_dataflow(data, d_nr, cfg);
    let exit_state = &outb[cfg.exit];
    // Shadow-diff at the exit, split three ways: AGREE (identical), PRECISION (my flow-sensitive
    // fact ⊏ the shipped `Join` — a win), DISAGREE (my fact does NOT refine theirs — coarser or
    // unsound; must be zero). The shipped classifier is flow-insensitive (join of ALL defs), so a
    // var whose reaching def here is single classifies definite where B collapses to `Join`.
    let mut precision_pairs: Vec<(u16, OFact, OFact)> = Vec::new();
    let disagree_pairs = disagreements(exit_state, data, d_nr, &mut precision_pairs);
    let agree = exit_state.len() - precision_pairs.len() - disagree_pairs.len();
    let fmt = |(v, mine, theirs): &(u16, OFact, OFact)| {
        format!("v{v}: mine={} B={}", mine.show(), theirs.show())
    };
    let precision: Vec<String> = precision_pairs.iter().map(fmt).collect();
    let disagree: Vec<String> = disagree_pairs.iter().map(fmt).collect();
    eprintln!(
        "OWN {name}  blocks={} passes={passes}  agree={agree} precision={} disagree={}",
        cfg.blocks.len(),
        precision.len(),
        disagree.len()
    );
    for p in &precision {
        eprintln!("  PRECISION {p}");
    }
    for d in &disagree {
        eprintln!("  DISAGREE {d}");
    }
    for (b, st) in outb.iter().enumerate() {
        if st.is_empty() {
            continue;
        }
        let items: Vec<String> = st
            .iter()
            .map(|(&v, &f)| format!("v{v}={}", f.show()))
            .collect();
        eprintln!("  b{b} [{}]  {}", cfg.blocks[b].label, items.join(", "));
    }
}

/// The oracle entry point, reached only under `LOFT_OWN_ORACLE` (SI-1). Modes:
/// `cfg` dumps each user function's CFG (Phase 1.1); `rd` runs the reaching-defs fixpoint (1.2);
/// `own` runs the forward ownership fixpoint + shadow-diff vs `ownership_of` (Phase 2).  The
/// checks of a program's store lifetimes are the store records' (`LOFT_LIFETIME_CHECK`,
/// STORE_RECORDS.md § The checks).
pub fn oracle(data: &Data) {
    let Ok(mode) = std::env::var("LOFT_OWN_ORACLE") else {
        return;
    };
    if !matches!(mode.as_str(), "cfg" | "rd" | "own") {
        return;
    }
    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) {
            continue;
        }
        let body = data.def(d_nr).code.clone();
        // skip empty / native-only bodies
        if matches!(body.unspan(), Value::Null) {
            continue;
        }
        let cfg = build(&body);
        let name = data.def(d_nr).name();
        match mode.as_str() {
            "cfg" => dump(name, &cfg),
            "rd" => {
                let rd = reaching_defs(&cfg);
                dump_reach(name, &cfg, &rd);
            }
            _ => {
                dump_own(name, &cfg, data, d_nr);
                // The two summaries of a callee's RETURN — the deps proxy the emitters read
                // (`return_adopts_fresh_store`) and the oracle's IR-derived class — side by
                // side, one line per heap-returning function, so a disagreement between them
                // is a grep rather than a theory (@FR-O-Oracle walk, QUALITY-history.md B7r).
                let def = data.def(d_nr);
                if def.returned().base().heap_dep().is_some() {
                    eprintln!(
                        "RETSUM {name} adopts={} oracle={:?}",
                        def.return_adopts_fresh_store(),
                        return_ownership(data, d_nr)
                    );
                }
            }
        }
    }
}

/// The POST-codegen report — run at the END of `scopes::check`, after `get_free_vars` has
/// inserted the frees into `def.code`.  One mode: `census`, the licence census below
/// (`scripts/licence_census.py`), a REPORT that emits no RED and answers a question about the
/// compiler rather than about the program.
pub fn oracle_free_checks(data: &Data) {
    let Ok(mode) = std::env::var("LOFT_OWN_ORACLE") else {
        return;
    };
    if mode != "census" {
        return;
    }
    let mut tally: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut spellings: BTreeMap<String, usize> = BTreeMap::new();
    let mut examples: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    let mut walked = 0usize;
    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) {
            continue;
        }
        let body = data.def(d_nr).code.clone();
        if matches!(body.unspan(), Value::Null) {
            continue;
        }
        let name = data.def(d_nr).name();
        run_licence_census(
            name,
            &body,
            data,
            d_nr,
            &mut tally,
            &mut spellings,
            &mut examples,
        );
        walked += 1;
    }
    let total: usize = tally.values().sum();
    // `scopes::check` runs once per SOURCE, and each run walks every definition known so
    // far — so a program that loads the three stdlib files prints a report per source and
    // only the LAST one covers the whole program.  The function count is what identifies
    // it; `scripts/licence_census.py` reads the last block for that reason.
    eprintln!(
        "OWN-CENSUS over {walked} function(s): {total} emitted free(s), \
         by the fact that licensed them"
    );
    for (bucket, n) in &tally {
        // A share for a human to read, over counts in the tens of thousands — the
        // precision clippy warns about is orders of magnitude below what this prints at.
        #[allow(clippy::cast_precision_loss)]
        let pct = if total > 0 {
            100.0 * *n as f64 / total as f64
        } else {
            0.0
        };
        eprintln!("  {bucket:<18} {n:6}  {pct:5.1}%");
        for ex in examples.get(bucket).into_iter().flatten() {
            eprintln!("      {ex}");
        }
    }
    eprintln!("  -- by free spelling --");
    for (op, n) in &spellings {
        eprintln!("  {op:<22} {n:6}");
    }
}

/// @PLN155 phase 0 — the LICENCE census: which FACT licensed each emitted free?
///
/// The plan's kill probe.  `Scopes::owns_freeable_store` licenses a free from the deps PROXY
/// (`depend().is_empty()`) plus the `is_skip_free` veto and the parameter carve-out; the
/// ORACLE (@FR-O-Oracle, `use_analysis::ownership_of`) derives the same fact independently and
/// never consults `deps`, "which is why they can disagree".  This counts, over every free the
/// compiler actually emits, how many rest on a positively-derived owner fact and how many on
/// the proxy alone.
///
/// **If proxy-alone is a handful, @PLN155's phases 2-4 are not worth their cost** and the plan
/// closes with this number as its product — the census staying as the guard that says when
/// that changes.  So the categories are chosen to be a partition of the frees, not a list of
/// suspicions:
///
///  * `oracle-derived` — the oracle read this binding's own definitions and answered `Owned`.
///    A positive fact; the proxy agreeing with it is the healthy case.
///  * `minted` — `OpDatabase` minted a store into this var.  Also a positive fact, and the
///    strongest one: no `Set` is needed for it.
///  * `proxy-alone` — the proxy said "empty deps, therefore owned" and the oracle had NOTHING
///    to read: no definition, no mint, not a parameter, so its `Owned` is the permissive
///    default rather than a derivation.  **The category the plan is about.**
///  * `oracle-disagrees` — the oracle answers `Borrowed`/`Join` and a free was emitted anyway.
///    Not necessarily wrong (a transition free reads the `owned_refs` memo, which this cannot
///    see), but it is the population where the two derivations differ.
///  * `no-answer` — the oracle answers `Own::Unknown`: a `CallRef` whose target it cannot
///    resolve, or a callee returning a borrow whose base the caller cannot name (@PLN155
///    phase 2).  Sharper than `proxy-alone`, which it is carved out of: there the oracle had
///    nothing to READ about the binding, here it read and could not conclude.
///  * `veto` — the binding is `skip_free`, whose contract is *no ownership-derived free in any
///    spelling*; it should be empty of live spellings (`@FR-O-Override`).
///
/// ⚠ **A category reading 0 is a claim to check, not a result.**  The control is
/// `LOFT_OWN_INJECT_FREE_BORROWED=<var>`, which forces an unconditional free of a borrowed
/// binding: it must move `oracle-disagrees`.  A census whose buckets cannot be moved is
/// measuring its own walk.
///
/// Reports per FREE SITE, not per binding: one binding freed on three paths is three licences
/// to account for, and collapsing them would under-count exactly where the paths differ.
fn run_licence_census(
    name: &str,
    body: &Value,
    data: &Data,
    d_nr: u32,
    tally: &mut BTreeMap<&'static str, usize>,
    spellings: &mut BTreeMap<String, usize>,
    examples: &mut BTreeMap<&'static str, Vec<String>>,
) {
    let func = &data.def(d_nr).variables;
    let sets = data.op_sets();
    // One `function_defs` for the whole function: `ownership_evidence` per site would walk and
    // clone the whole body each time, which is loft#854's thirteen-minute shape.
    let defs = crate::use_analysis::function_defs(data, d_nr);
    let mut sites: Vec<(u16, String)> = Vec::new();
    body.walk(&mut |x| {
        if let Value::Call(d, args) = x
            && sets.frees.contains(d)
            && let Some(a0) = args.first()
        {
            let v = match a0.unspan() {
                Value::Var(v) => *v,
                Value::TupleGet(v, _) => *v,
                _ => return,
            };
            sites.push((v, data.def(*d).name().to_string()));
        }
    });
    for (v, op) in sites {
        *spellings.entry(op).or_default() += 1;
        let (own, evidence) = crate::use_analysis::ownership_evidence_with(data, d_nr, v, &defs);
        let bucket = if func.is_skip_free(v) {
            "veto"
        } else if matches!(own, Own::Unknown) {
            // @PLN155 phase 2 — kept OUT of `oracle-disagrees`, whose documented meaning is
            // "the oracle answers Borrowed/Join".  `Unknown` is not a disagreement, it is the
            // absence of an answer, and folding it in would make this census say something
            // false about its own largest categories.
            "no-answer"
        } else if !matches!(own, Own::Owned) {
            "oracle-disagrees"
        } else {
            match evidence {
                crate::use_analysis::OwnEvidence::Derived => "oracle-derived",
                crate::use_analysis::OwnEvidence::Minted => "minted",
                // A delivery buffer the callee mints into — a positive owner fact the caller's
                // frame has no `Set` for, and two-thirds of what `proxy-alone` used to hold.
                crate::use_analysis::OwnEvidence::SynthBuffer => "delivery-buffer",
                // A parameter answering `Owned` cannot happen (the arm returns `Borrowed` of
                // itself), so this is the promoted-retbuf carve-out reached another way; kept
                // separate rather than folded, because merging it would hide it.
                crate::use_analysis::OwnEvidence::Parameter => "parameter",
                crate::use_analysis::OwnEvidence::Fallback => "proxy-alone",
            }
        };
        *tally.entry(bucket).or_default() += 1;
        let ex = examples.entry(bucket).or_default();
        // Every `proxy-alone` site is listed, never a sample: it is the category the plan's
        // kill criterion reads, and "a handful" is a claim about the SET, which a sample
        // cannot support.  The other buckets get six examples — enough to see what is in
        // them, and they are not what the decision turns on.
        if bucket == "proxy-alone" || ex.len() < 6 {
            ex.push(format!(
                "{name}:{} (v{v}) dep={:?}",
                func.name(v),
                func.tp(v).depend()
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    //! @PLN94 Phase 1.3 — the CFG + fixpoint on the control-flow shapes the position-proxy cannot
    //! express, on hand-built IR (parser-free). Each asserts a reaching-defs / edge fact AND the
    //! SI-3 bound. A back-edge is detected as an edge to a lower-numbered block (loop header).
    use super::{BlockId, build, reaching_defs};
    use crate::data::{Type, Value, v_block, v_if, v_set};

    fn loop_block(ops: Vec<Value>) -> Value {
        match v_block(ops, Type::Void, "loop") {
            Value::Block(b) => Value::Loop(b),
            _ => unreachable!(),
        }
    }

    #[test]
    fn branch_join_unions_arms_and_kills_initial() {
        // r(v1) = 0; if c { r = 1 } else { r = 2 }; r
        let body = v_block(
            vec![
                v_set(1, Value::Int(0)),
                v_if(
                    Value::Var(0),
                    v_block(vec![v_set(1, Value::Int(1))], Type::Void, "then"),
                    v_block(vec![v_set(1, Value::Int(2))], Type::Void, "else"),
                ),
                Value::Var(1),
            ],
            Type::Void,
            "body",
        );
        let cfg = build(&body);
        let rd = reaching_defs(&cfg);
        let v1_at_exit: Vec<BlockId> = rd.inb[cfg.exit]
            .iter()
            .map(|&d| &rd.sites[d])
            .filter(|s| s.var == 1)
            .map(|s| s.block)
            .collect();
        assert_eq!(v1_at_exit.len(), 2, "join must union both arm defs of r");
        assert!(
            !v1_at_exit.contains(&cfg.entry),
            "the initial r=0 must not reach past the branch (killed on both arms)"
        );
    }

    #[test]
    fn loop_carried_def_reaches_header_and_converges() {
        // s(v1) = 0; loop { if c break; s = s + 1 }
        let body = v_block(
            vec![
                v_set(1, Value::Int(0)),
                loop_block(vec![
                    v_if(Value::Var(0), Value::Break(0), Value::Null),
                    v_set(1, Value::Int(9)),
                ]),
            ],
            Type::Void,
            "body",
        );
        let cfg = build(&body);
        let rd = reaching_defs(&cfg);
        assert!(
            rd.passes <= cfg.blocks.len() + 2,
            "SI-3: bounded convergence"
        );
        let header = cfg
            .blocks
            .iter()
            .enumerate()
            .flat_map(|(b, bb)| bb.succ.iter().copied().filter(move |&s| s < b))
            .next()
            .expect("a loop must have a back-edge");
        let v1_at_header = rd.inb[header]
            .iter()
            .filter(|&&d| rd.sites[d].var == 1)
            .count();
        assert!(
            v1_at_header >= 2,
            "loop header must see s from BOTH the initial store and the body (loop-carried), got {v1_at_header}"
        );
    }

    #[test]
    fn early_return_edges_to_function_exit() {
        // loop { if c { return } ; s = 1 }
        let body = v_block(
            vec![loop_block(vec![
                v_if(
                    Value::Var(0),
                    v_block(
                        vec![Value::Return(Box::new(Value::Var(0)))],
                        Type::Void,
                        "ret",
                    ),
                    Value::Null,
                ),
                v_set(1, Value::Int(1)),
            ])],
            Type::Void,
            "body",
        );
        let cfg = build(&body);
        let returns_to_exit = cfg
            .blocks
            .iter()
            .any(|bb| bb.ops.iter().any(|o| o == "Return") && bb.succ.contains(&cfg.exit));
        assert!(
            returns_to_exit,
            "an early return must edge to the FUNCTION exit, not the loop exit"
        );
    }

    #[test]
    fn nested_loops_converge_with_two_headers() {
        // loop { loop { s = 1; if c break } if c break } — each break(0) targets its own loop.
        let body = v_block(
            vec![loop_block(vec![
                loop_block(vec![
                    v_set(1, Value::Int(1)),
                    v_if(Value::Var(0), Value::Break(0), Value::Null),
                ]),
                v_if(Value::Var(0), Value::Break(0), Value::Null),
            ])],
            Type::Void,
            "body",
        );
        let cfg = build(&body);
        let rd = reaching_defs(&cfg);
        assert!(
            rd.passes <= cfg.blocks.len() + 2,
            "SI-3: bounded convergence on nested loops"
        );
        let back_targets: std::collections::BTreeSet<BlockId> = cfg
            .blocks
            .iter()
            .enumerate()
            .flat_map(|(b, bb)| bb.succ.iter().copied().filter(move |&s| s < b))
            .collect();
        assert!(
            back_targets.len() >= 2,
            "nested loops → at least two distinct loop headers, got {}",
            back_targets.len()
        );
    }

    #[test]
    fn ofact_meet_is_a_join_semilattice() {
        use super::OFact;
        use super::OFact::{Borrowed, Bottom, Join, Owned};
        // Bottom (unreached) is the identity.
        assert_eq!(OFact::meet(Bottom, Owned), Owned);
        assert_eq!(OFact::meet(Borrowed(3), Bottom), Borrowed(3));
        // Equal facts are idempotent.
        assert_eq!(OFact::meet(Owned, Owned), Owned);
        assert_eq!(OFact::meet(Borrowed(2), Borrowed(2)), Borrowed(2));
        // Owned ⊔ Borrowed → runtime-dependent Join (the `v[i] ?? d` / reassign-per-arm shape).
        assert_eq!(OFact::meet(Owned, Borrowed(5)), Join(5));
        assert_eq!(OFact::meet(Borrowed(5), Owned), Join(5)); // commutative
        // Borrowing from different bases borrows on every path: no free is ever licensed, so it
        // stays `Borrowed`, with no one base to name — what `ownership_of` answers.
        assert_eq!(OFact::meet(Borrowed(1), Borrowed(2)), Borrowed(u16::MAX));
        assert_eq!(
            OFact::meet(Borrowed(u16::MAX), Borrowed(3)),
            Borrowed(u16::MAX)
        );
        // Join dominates.
        assert_eq!(OFact::meet(Join(7), Owned), Join(7));
        assert_eq!(OFact::meet(Owned, Join(7)), Join(7));
    }

    #[test]
    fn ofact_refines_marks_precision_and_flags_the_unsound_direction() {
        use super::OFact::{Borrowed, Bottom, Join, Owned};
        // A definite fact refines the shipped runtime-dependent `Join` — a PRECISION win (sound:
        // narrowing "maybe owned/borrowed" to a definite borrow/own never frees more than B would).
        assert!(Owned.refines(Join(3)));
        assert!(Borrowed(1).refines(Join(2)));
        assert!(Bottom.refines(Owned));
        assert!(Owned.refines(Owned)); // identity
        // The DANGEROUS direction must NOT refine (→ flagged DISAGREE): claiming `Owned` where B
        // says `Borrowed` would free a borrowed store (UAF/double-free). This is the soundness gate.
        assert!(!Owned.refines(Borrowed(1)));
        assert!(!Borrowed(1).refines(Owned));
        // Coarser-than-B (my `Join` where B is definite) is also not a refinement.
        assert!(!Join(1).refines(Owned));
    }
}
