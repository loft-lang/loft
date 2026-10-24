// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a compiled target's decisions, as data

//! The switches of the compiled-target rewrites (@PLN200 B3.2): one `LOFT_NO_*` environment
//! switch per admitted rewrite, each the first bisect step for a native-only wrong answer
//! (NATIVE_SWITCHES.md).  Read once per program by [`Switches::from_env`]; every emitter
//! honours the same switches, so a rewrite switched off is off on every compiled target.

/// The rewrite switches; `true` turns that rewrite off.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy)]
pub struct Switches {
    /// `LOFT_NO_BOUNDED_NEST=1` — no nest is guarded and run plain.
    pub bounded_nest_disabled: bool,
    /// `LOFT_NO_BOUNDED_SUM=1` — no reduction runs its plain block sum first
    /// (`(R-BoundedNest)`'s reduction clause).
    pub bounded_sum_disabled: bool,
    /// `LOFT_NO_GUARDED_CHAIN=1` — no counted loop's affine chains are guarded and run plain.
    pub chain_guard_disabled: bool,
    /// `LOFT_NO_DISTINCT_GROWTH=1` — a loop that grows ANY store binds no element base and a
    /// remainder that grows one binds no record address, as before `@FR-R-Base`'s growth
    /// clause; the bisect step for a wrong element or field read in a loop that appends
    /// elsewhere.  `LOFT_HOIST_VERIFY=1` is the falsifier (each base and address re-derived).
    pub distinct_growth_disabled: bool,
    /// `LOFT_NO_NEST_RAW_READS=1` — the plain arm keeps its bounds-tested reads and null
    /// selects (step 1's form).
    pub nest_raw_disabled: bool,
    /// `LOFT_NO_TWIN_BASE=1` — a twin takes its header inputs alone again, and every element
    /// read or write inside it resolves the store (`@FR-R-Base`'s twin clause off).
    pub twin_base_disabled: bool,
    /// `LOFT_NO_PLAIN_CHAIN_PTR` — the plain-chains copy of a loop binds no record address.
    pub plain_chain_ptr_disabled: bool,
    /// `LOFT_NO_PARAM_TWIN_PTR` — a record parameter's address serves no twin call.
    pub param_twin_ptr_disabled: bool,
    /// `LOFT_NO_MINT_WINDOW=1` — a minted element's field writes resolve the store again.
    pub mint_window_disabled: bool,
    /// `LOFT_NO_RECORD_PTR=1` — no record view carries its address; every field read and
    /// write resolves the store again.
    pub record_ptr_disabled: bool,
    /// `LOFT_NO_PARAM_RECORD_PTR=1` — `@FR-R-RecPtr`'s parameter clause off: a record
    /// parameter's field accesses resolve the store each, as before the clause.
    pub param_record_ptr_disabled: bool,
    /// `LOFT_NO_ELEMENT_ADDRESS=1` — `@FR-R-RecPtr`'s element clause off: a value record
    /// written into `v[i]` takes its element through `vec_get_or_raise_runtime` and its
    /// address back out of the reference ([`Output::element_copy_dest`]).
    pub element_address_disabled: bool,
    /// `LOFT_NO_FIELD_HEADER=1` — `@FR-R-RecPtr`'s nested clause off: a vector field of a
    /// record view (`v.items`) resolves its store and record again at each length or
    /// element read ([`Output::field_header_expr`]).
    pub field_header_disabled: bool,
    /// `LOFT_NO_DISTINCT_VERSION=1` — no loop is emitted twice on a store-distinct test.
    pub distinct_version_disabled: bool,
    /// `LOFT_NO_HEADER_DBREF=1` — rebuild the vector's `DbRef` at every fused read.
    pub header_dbref_disabled: bool,
    /// `LOFT_NO_REFILL_KEEP=1` — a refilling buffer's entry clear releases its vector, as
    /// before the live clause.
    pub refill_keep_disabled: bool,
    /// `LOFT_NO_REFILL_TEXT=1` — every pooled call site keeps its release and the plain callee.
    pub refill_text_disabled: bool,
    /// `LOFT_NO_REFILL_ELEMENTS=1` — `@FR-R-RefillText`'s collection clause off: a
    /// heap-element refill buffer is released whole at entry and its elements claimed anew.
    pub refill_elements_disabled: bool,
    /// `LOFT_NO_BYTE_READ=1` — a `vector<u8>` element read keeps its template.
    pub byte_read_disabled: bool,
    /// `LOFT_NO_BYTE_RESOLVE=1` — `@FR-R-Base`'s byte clause needs a held BASE again: a
    /// header held without one keeps the template (the resolve clause off).
    pub byte_resolve_disabled: bool,
    /// `LOFT_NO_BASE_RECPTR=1` — a record view bound from an element of a vector whose BASE
    /// the loop holds resolves the store for its address again (`@FR-R-RecPtr`'s base
    /// clause off).
    pub base_rec_ptr_disabled: bool,
    /// `LOFT_NO_INDEX_RECPTR=1` — a record view bound from `v[i]?` over a vector whose header
    /// and BASE the frame holds reads its element through `get_vector_hoisted` and resolves
    /// its address from the store again (`@FR-R-RecPtr`'s index clause off).
    pub index_rec_ptr_disabled: bool,
    /// `LOFT_NO_GUARDED_FILL=1` — `t = base + i; if t < N { v[t] = c }` keeps its loop
    /// (`@FR-R-Fill`'s guarded clause off).
    pub guarded_fill_disabled: bool,
    /// `LOFT_NO_INVARIANT_HOIST=1` — every invariant integer chain is evaluated at every
    /// use again, as before @PLN157 § V-ao; the bisect step for a wrong index or a wrong
    /// arithmetic value inside a loop on native.  `LOFT_HOIST_VERIFY=1` is the falsifier
    /// (every use re-evaluates the chain and compares it with the memo).
    pub invariant_hoist_disabled: bool,
    /// `LOFT_NO_SCALAR_HOIST=1` — read every record scalar field per iteration, as before
    /// @PLN157 P4c.  One step finer than `LOFT_NO_VECTOR_HOIST` when bisecting a wrong
    /// answer in a loop that reads a record's fields; `LOFT_HOIST_VERIFY=1` is the
    /// falsifier (each hoisted read is re-derived and compared).
    pub scalar_hoist_disabled: bool,
    /// `LOFT_NO_VIEW_HOIST=1` — a `&`-bound vector view (`d = &cv.data`) derives no header
    /// at its binding, so every later `len(d)` / `d[i]` in the block resolves the store
    /// again, as before @PLN157 § V-n.  The bisect step for a wrong element read through
    /// such a view outside a loop; `LOFT_HOIST_VERIFY=1` is the falsifier.
    pub view_hoist_disabled: bool,
    /// `LOFT_NO_WRAPPER_INLINE=1` — a call to a stdlib one-op wrapper (`len(v)`, `sqrt(x)`)
    /// is emitted as the CALL again, as before @PLN157 § V-o, instead of as its op; the
    /// bisect step for a wrong length or libm value on native, and the before-half of the
    /// A/B.  See [`hoist::one_op_wrapper`].
    pub wrapper_inline_disabled: bool,
    /// `LOFT_NO_PUSH_HOIST=1` — a loop that pushes to a vector hoists nothing, as before
    /// @PLN157 § V-q (`@FR-R-Push`); the bisect step for a wrong element or length out of a
    /// loop that appends.  `LOFT_HOIST_VERIFY=1` is the falsifier.
    pub push_hoist_disabled: bool,
    /// `LOFT_NO_MINT_HOIST=1` — a loop that APPENDS a record element (the
    /// `OpNewRecord`/`OpFinishRecord` mint group) hoists nothing, as before @PLN157 § V-s
    /// (`@FR-R-Mint`): the bisect step for a wrong scalar read out of a loop that appends
    /// records.  `LOFT_HOIST_VERIFY=1` is the falsifier (each hoisted scalar is re-read
    /// and compared).
    pub mint_hoist_disabled: bool,
    /// `LOFT_NO_RECORD_PUSH=1` — a record append keeps its mint-group templates
    /// (`OpNewRecord`'s dispatch + prefill, `OpFinishRecord`'s dispatch) as before @PLN157
    /// § V-t (`@FR-R-PushRec`); the bisect step for a wrong element out of a record-appending
    /// loop.  `LOFT_HOIST_VERIFY=1` is the falsifier (the header is re-derived and compared
    /// at the slot and at the finish).
    pub record_push_disabled: bool,
    /// `LOFT_NO_HEAP_RECORD_PUSH=1` — a heap-owning element keeps its mint-group templates
    /// (`@FR-R-PushRec`'s heap clause off): the bisect step for a wrong or stale handle in an
    /// appended record whose fields own heap.  `LOFT_POISON_CLAIM=1` is the falsifier.
    pub heap_record_push_disabled: bool,
    /// `LOFT_NO_FIELD_APPEND=1` (or the runtime's own `LOFT_NO_PLAIN_FIELD_APPEND=1`) — a
    /// straight-line append to a struct's plain vector field keeps its `OpNewRecord` /
    /// `OpFinishRecord` templates (`@FR-R-FieldAppend` off): the bisect step for a wrong
    /// element out of `r.f += [x]` outside a loop.
    pub field_append_disabled: bool,
    /// `LOFT_NO_BARE_APPEND=1` — an append to a plain vector that no push header holds keeps
    /// its `OpNewRecord` / `OpFinishRecord` templates (`@FR-R-FieldAppend`'s bare clause
    /// off): the bisect step for a wrong element or length out of an unhoisted `v += [x]`.
    pub bare_append_disabled: bool,
    /// `LOFT_NO_REBOUND_MOVER=1` — a loop whose body rebinds a mover (a pushed or minted
    /// vector) declines every hoist again, as before `(R-Mint)`'s rebound clause; the bisect
    /// step for a wrong element or scalar read in a loop that declares a vector per pass.
    pub rebound_mover_disabled: bool,
    /// `LOFT_NO_GROUP_PUSH=1` — a mint group outside any held header keeps its templates
    /// (`@FR-R-GroupPush` off); the bisect step for a wrong element out of a literal record
    /// append.  `LOFT_HOIST_VERIFY=1` is the falsifier (header re-derived at slot and finish).
    pub group_push_disabled: bool,
    /// `LOFT_NO_MOVE_APPEND=1` — a paired append keeps the deep copy, as before @PLN157
    /// § V-j; the bisect step for a wrong element, a leak or a double free out of a
    /// `for f in call(…) {{ v += [f] }}` loop.
    pub move_append_disabled: bool,
    /// `LOFT_NO_LAZY_SPLIT=1` — every such loop builds and walks its `vector<text>` again;
    /// the bisect step for a wrong or missing piece out of a loop over a `split`.
    pub lazy_split_disabled: bool,
    /// `LOFT_NO_RANGE_END_HEADER=1` — a range end always reads its prelude local; the bisect
    /// step for a wrong trip count out of a `for i in 0..len(v)` loop.
    pub range_end_header_disabled: bool,
    /// `LOFT_NO_SPLIT_TABLE=1` — every such bind builds its `vector<text>` again; the
    /// bisect step for a wrong length, piece or walk out of a split bound to a name.
    pub split_table_disabled: bool,
    /// `LOFT_NO_TEXT_BASE=1` — a text element read in a loop that holds the vector's header
    /// and base resolves the store per element again (`@FR-R-Base`'s text clause off); the
    /// bisect step for a wrong or stale text read out of `v[i]` on native.
    pub text_base_disabled: bool,
    /// `LOFT_NO_TEXT_FIELD=1` — `@FR-R-RecPtr`'s text clause off: a text FIELD of a held
    /// iteration's record view resolves the store at each read, as before the clause.
    pub text_field_disabled: bool,
    /// `LOFT_NO_TEXT_BORROW=1` — every walk of texts copies its element again, as before
    /// `@FR-R-TextBorrow`; the bisect step for a wrong or stale text read through the loop
    /// variable of `for p in vector<text>` on native.
    pub text_borrow_disabled: bool,
    /// `LOFT_NO_CHAR_WALK=1` — every character walk takes its step as written and asks
    /// its null test per iteration again, as before `@FR-R-CharWalk`; the bisect step for
    /// a wrong character, a wrong byte offset or a missed fault out of `for c in text` on
    /// native.
    pub char_walk_disabled: bool,
    /// `LOFT_NO_LITERAL_HOIST=1` — every loop-body literal rebuilds per iteration, as
    /// before @PLN157 § V-x; the bisect step for a wrong constant vector inside a loop.
    pub literal_hoist_disabled: bool,
    /// `LOFT_NO_COMPLETE_WRITE=1` — every record keeps its default prefill, as before
    /// @PLN157 § V-y; the bisect step for a wrong default/sentinel in a literal-built
    /// record on native.
    pub complete_write_disabled: bool,
    /// `LOFT_NO_ELEMENT_FIRST=1` — every record-literal field keeps its temp-store
    /// build and deep copy, as before @PLN157 § V-z; the bisect step for a wrong
    /// vector field of an appended record on native.
    pub element_first_disabled: bool,
    /// `LOFT_NO_LOOP_RECORD` (generation time): every such local frees its store at the
    /// iteration's end and takes a fresh one next pass again.
    pub loop_record_disabled: bool,
    /// `LOFT_NO_LOOP_BUFFER_REUSE` (generation time): every such buffer re-mints per
    /// iteration again.
    pub loop_buffer_disabled: bool,
    /// `LOFT_NO_PUSH_FILL` (generation time): a counted push loop reserves nothing and
    /// fills nothing — the per-push ladder and the per-element loop again (§ V-am).
    pub push_fill_disabled: bool,
    /// `LOFT_NO_PUSH_WINDOW` (generation time): a reserved counted push loop pushes through
    /// its push header again, the length written back per push (`@FR-R-PushFill`'s window
    /// clause).
    pub push_window_disabled: bool,
    /// `LOFT_NO_JOIN_READ` (generation time): `v[i]?.f` runs its join on every pass and
    /// reads the result through the store again (`@FR-R-Base`'s join clause).
    pub join_read_disabled: bool,
    /// `LOFT_NO_RETBUF_ADOPT=1` — a vector-returning function keeps its delivery copies,
    /// as before @PLN157 § V-u; the bisect step for a wrong vector return or a leak at a
    /// vector-returning call.
    pub retbuf_adopt_disabled: bool,
    /// `LOFT_NO_CALLEE_INPUTS=1` — no callee twin is emitted and every call keeps its plain
    /// form, as before @PLN157 § V-p (`@FR-R-Inputs`): the bisect step for a wrong value read
    /// through a record parameter inside a callee that a hoisting loop calls.
    /// `LOFT_HOIST_VERIFY=1` is the falsifier — inside the twin every input is re-read.
    pub callee_inputs_disabled: bool,
    /// `LOFT_NO_FN_HEADER=1` — `(R-Header)`'s FUNCTION clause off: a body that writes no
    /// store and reads a vector path twice or more keeps deriving the path's header at every
    /// read instead of once at entry.  The first bisect step for a wrong element read
    /// outside any loop under `--native`.
    pub fn_header_disabled: bool,
    /// `LOFT_NO_FILL_HOIST=1` — a filling loop keeps its per-element form (@PLN157 § V-ae,
    /// `@FR-R-Fill`): the bisect step for a wrong element or a missed write out of
    /// `for i in lo..hi { v[base + i] = c }`.
    pub fill_hoist_disabled: bool,
    /// `LOFT_NO_ELEM_FUSE=1` — keep loft#885 stage 1 (the loop-invariant header) but emit
    /// stage 2's scalar element reads unfused, as an address and then a load.
    ///
    /// The middle rung of the A/B, and the reason it exists: with only `LOFT_NO_VECTOR_HOIST`
    /// the two stages can be measured together but never apart, so what stage 2 is worth on
    /// top of the header hoist could only be estimated. It also bisects a native-only wrong
    /// answer in a vector loop one stage further than the all-or-nothing switch does.
    pub elem_fuse_disabled: bool,
    /// `LOFT_NO_NN_FAST=1` — emit every float compare through its `#rust`
    /// template, as before @PLN157 P3.  The bisect switch for a native-only
    /// wrong boolean around floats, same contract as `LOFT_NO_VECTOR_HOIST`.
    pub nn_fast_disabled: bool,
    /// `LOFT_NO_TYPED_KEYED=1` — a lookup in a `hash` with one integer key is emitted as the
    /// general `OpGetRecord` again instead of `OpGetHashLong` (`@FR-R-TypedKeyed`); the
    /// bisect step for a wrong or missing record out of such a lookup on native.
    pub typed_keyed_disabled: bool,
    /// `LOFT_NO_RANGE_ARITH=1` — every integer operator keeps its checked template, as
    /// before `@FR-R-Range`; the bisect step for a wrong integer value on native where the
    /// range proof admitted a plain operator.  `LOFT_HOIST_VERIFY=1` is the falsifier.
    pub range_arith_disabled: bool,
    /// `LOFT_NO_FNREF_WORK_BUFFER=1` — a dispatch arm hands a candidate's work buffer the
    /// null sentinel again, so the candidate mints its own on every call.  The bisect step
    /// for a stale or shared scratch vector in a function reached through a function value.
    pub fnref_work_disabled: bool,
    /// `LOFT_NO_LEAF_PRELUDE=1` — emit the frame push on leaves too, as
    /// before N4.  The bisect switch for a diagnostic that lost its
    /// innermost frame, same contract as `LOFT_NO_VECTOR_HOIST`.
    pub leaf_elide_disabled: bool,
    /// `LOFT_NO_LEAF_CHAIN=1` — in the lean tier, elide the prelude on true
    /// leaves only, not on a function whose whole call tree is frameless
    /// (`@FR-R-LeafChain`).  The bisect step one finer than
    /// `LOFT_NO_LEAF_PRELUDE`.
    pub leaf_chain_disabled: bool,
    /// `LOFT_NO_GUARD_FREE=1` — every non-leaf frame constructs its fn-ref buffer guard
    /// again, as before `@FR-R-GuardFree`; the bisect step for a leaked or doubly-freed
    /// fn-ref return buffer in a program with recursion.
    pub guard_free_disabled: bool,
    /// `LOFT_NO_WRITE_HOIST=1` — classify in-place element writes as hoist
    /// blockers again (@PLN157 P4a), so a loop that writes keeps the pre-885
    /// per-element form for everything.  One step finer than
    /// `LOFT_NO_VECTOR_HOIST` when bisecting a wrong answer in a loop that
    /// both reads and writes vectors.
    pub write_hoist_disabled: bool,
}

impl Switches {
    /// The switches as this process's environment sets them.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn from_env() -> Self {
        Switches {
            bounded_nest_disabled: std::env::var("LOFT_NO_BOUNDED_NEST").is_ok_and(|v| v != "0"),
            bounded_sum_disabled: !crate::keys::bounded_sum_enabled(),
            chain_guard_disabled: std::env::var("LOFT_NO_GUARDED_CHAIN").is_ok_and(|v| v != "0"),
            distinct_growth_disabled: std::env::var("LOFT_NO_DISTINCT_GROWTH")
                .is_ok_and(|v| v != "0"),
            nest_raw_disabled: std::env::var("LOFT_NO_NEST_RAW_READS").is_ok_and(|v| v != "0"),
            param_twin_ptr_disabled: std::env::var("LOFT_NO_PARAM_TWIN_PTR")
                .is_ok_and(|v| v != "0"),
            plain_chain_ptr_disabled: std::env::var("LOFT_NO_PLAIN_CHAIN_PTR")
                .is_ok_and(|v| v != "0"),
            // One rule: `LOFT_NO_VECTOR_BASE` switches its twin clause off with it.
            twin_base_disabled: std::env::var("LOFT_NO_TWIN_BASE").is_ok_and(|v| v != "0")
                || !crate::keys::vector_base_enabled(),
            mint_window_disabled: std::env::var("LOFT_NO_MINT_WINDOW").is_ok_and(|v| v != "0"),
            // One rule: `LOFT_NO_VECTOR_BASE` switches its record clause off with it.
            record_ptr_disabled: std::env::var("LOFT_NO_RECORD_PTR").is_ok_and(|v| v != "0")
                || !crate::keys::vector_base_enabled(),
            base_rec_ptr_disabled: std::env::var("LOFT_NO_BASE_RECPTR").is_ok_and(|v| v != "0"),
            index_rec_ptr_disabled: std::env::var("LOFT_NO_INDEX_RECPTR").is_ok_and(|v| v != "0"),
            element_address_disabled: std::env::var("LOFT_NO_ELEMENT_ADDRESS")
                .is_ok_and(|v| v != "0"),
            field_header_disabled: std::env::var("LOFT_NO_FIELD_HEADER").is_ok_and(|v| v != "0"),
            guarded_fill_disabled: std::env::var("LOFT_NO_GUARDED_FILL").is_ok_and(|v| v != "0"),
            param_record_ptr_disabled: std::env::var("LOFT_NO_PARAM_RECORD_PTR")
                .is_ok_and(|v| v != "0"),
            distinct_version_disabled: std::env::var("LOFT_NO_DISTINCT_VERSION")
                .is_ok_and(|v| v != "0"),
            header_dbref_disabled: std::env::var("LOFT_NO_HEADER_DBREF").is_ok_and(|v| v != "0"),
            byte_read_disabled: std::env::var("LOFT_NO_BYTE_READ").is_ok_and(|v| v != "0"),
            byte_resolve_disabled: std::env::var("LOFT_NO_BYTE_RESOLVE").is_ok_and(|v| v != "0"),
            refill_keep_disabled: std::env::var("LOFT_NO_REFILL_KEEP").is_ok_and(|v| v != "0"),
            refill_text_disabled: std::env::var("LOFT_NO_REFILL_TEXT").is_ok_and(|v| v != "0"),
            refill_elements_disabled: std::env::var("LOFT_NO_REFILL_ELEMENTS")
                .is_ok_and(|v| v != "0"),
            scalar_hoist_disabled: std::env::var("LOFT_NO_SCALAR_HOIST").is_ok_and(|v| v != "0"),
            invariant_hoist_disabled: std::env::var("LOFT_NO_INVARIANT_HOIST")
                .is_ok_and(|v| v != "0"),
            fill_hoist_disabled: !crate::keys::fill_hoist_enabled(),
            view_hoist_disabled: std::env::var("LOFT_NO_VIEW_HOIST").is_ok_and(|v| v != "0"),
            wrapper_inline_disabled: std::env::var("LOFT_NO_WRAPPER_INLINE")
                .is_ok_and(|v| v != "0"),
            push_hoist_disabled: std::env::var("LOFT_NO_PUSH_HOIST").is_ok_and(|v| v != "0"),
            mint_hoist_disabled: std::env::var("LOFT_NO_MINT_HOIST").is_ok_and(|v| v != "0"),
            record_push_disabled: std::env::var("LOFT_NO_RECORD_PUSH").is_ok_and(|v| v != "0"),
            heap_record_push_disabled: std::env::var("LOFT_NO_HEAP_RECORD_PUSH")
                .is_ok_and(|v| v != "0"),
            field_append_disabled: std::env::var("LOFT_NO_FIELD_APPEND").is_ok_and(|v| v != "0")
                || std::env::var_os("LOFT_NO_PLAIN_FIELD_APPEND").is_some(),
            bare_append_disabled: std::env::var("LOFT_NO_BARE_APPEND").is_ok_and(|v| v != "0"),
            rebound_mover_disabled: std::env::var("LOFT_NO_REBOUND_MOVER").is_ok_and(|v| v != "0"),
            group_push_disabled: std::env::var("LOFT_NO_GROUP_PUSH").is_ok_and(|v| v != "0"),
            move_append_disabled: std::env::var("LOFT_NO_MOVE_APPEND").is_ok_and(|v| v != "0"),
            lazy_split_disabled: std::env::var("LOFT_NO_LAZY_SPLIT").is_ok_and(|v| v != "0"),
            range_end_header_disabled: std::env::var("LOFT_NO_RANGE_END_HEADER")
                .is_ok_and(|v| v != "0"),
            split_table_disabled: std::env::var("LOFT_NO_SPLIT_TABLE").is_ok_and(|v| v != "0"),
            text_base_disabled: std::env::var("LOFT_NO_TEXT_BASE").is_ok_and(|v| v != "0"),
            text_field_disabled: std::env::var("LOFT_NO_TEXT_FIELD").is_ok_and(|v| v != "0"),
            text_borrow_disabled: std::env::var("LOFT_NO_TEXT_BORROW").is_ok_and(|v| v != "0"),
            char_walk_disabled: std::env::var("LOFT_NO_CHAR_WALK").is_ok_and(|v| v != "0"),
            literal_hoist_disabled: std::env::var("LOFT_NO_LITERAL_HOIST").is_ok_and(|v| v != "0"),
            complete_write_disabled: std::env::var("LOFT_NO_COMPLETE_WRITE")
                .is_ok_and(|v| v != "0"),
            element_first_disabled: std::env::var("LOFT_NO_ELEMENT_FIRST").is_ok_and(|v| v != "0"),
            loop_buffer_disabled: !crate::keys::loop_buffer_reuse_enabled(),
            loop_record_disabled: std::env::var("LOFT_NO_LOOP_RECORD").is_ok_and(|v| v != "0")
                || !crate::keys::loop_buffer_reuse_enabled(),
            push_fill_disabled: !crate::keys::push_fill_enabled(),
            push_window_disabled: !crate::keys::push_window_enabled(),
            join_read_disabled: !crate::keys::join_read_enabled(),
            retbuf_adopt_disabled: std::env::var("LOFT_NO_RETBUF_ADOPT").is_ok_and(|v| v != "0"),
            callee_inputs_disabled: std::env::var("LOFT_NO_CALLEE_INPUTS").is_ok_and(|v| v != "0"),
            fn_header_disabled: std::env::var("LOFT_NO_FN_HEADER").is_ok_and(|v| v != "0"),
            elem_fuse_disabled: std::env::var("LOFT_NO_ELEM_FUSE").is_ok_and(|v| v != "0"),
            nn_fast_disabled: std::env::var("LOFT_NO_NN_FAST").is_ok_and(|v| v != "0"),
            typed_keyed_disabled: std::env::var("LOFT_NO_TYPED_KEYED").is_ok_and(|v| v != "0"),
            range_arith_disabled: std::env::var("LOFT_NO_RANGE_ARITH").is_ok_and(|v| v != "0"),
            fnref_work_disabled: std::env::var("LOFT_NO_FNREF_WORK_BUFFER").is_ok_and(|v| v != "0"),
            leaf_elide_disabled: std::env::var("LOFT_NO_LEAF_PRELUDE").is_ok_and(|v| v != "0"),
            leaf_chain_disabled: std::env::var("LOFT_NO_LEAF_CHAIN").is_ok_and(|v| v != "0"),
            guard_free_disabled: std::env::var("LOFT_NO_GUARD_FREE").is_ok_and(|v| v != "0"),
            write_hoist_disabled: std::env::var("LOFT_NO_WRITE_HOIST").is_ok_and(|v| v != "0"),
        }
    }
}
