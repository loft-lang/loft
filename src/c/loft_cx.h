/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * The store context (@PLN226): everything a thread of a loft program keeps of its own — its
 * store table, its frames, its fault and test jumps, its scratch — in one struct, handed to every
 * function as its first argument `cx`, as the rustc writer hands its `Stores` cell.  The main
 * thread's is `loft_main_cx`; a `par` worker's is made by the thread that starts it
 * (`loft_par_job_init`), and a worker reaches the stores it shares with that thread only through
 * its own table (formal/concurrency.md `C-Par`).  Nothing here is a thread-local.
 *
 * It comes right after `loft_cc.h` in every unit, and holds by value only the types defined here;
 * a field that points to a runtime type names its struct tag, which the runtime defines later. */
#ifndef LOFT_CX_H
#define LOFT_CX_H
#include <stddef.h>
#include <stdint.h>
#include <string.h>

/* Every function that reaches a thread's state takes its context first, as `cx`, and passes it
 * on: nothing reaches that state any other way. */
typedef struct loft_cx loft_cx;

typedef struct { const char *p; size_t len; } loft_str;
typedef struct { char *p; size_t len; size_t cap; } loft_text;

/* A reference (`DbRef`).  The trailing pad makes it 16 bytes, so `rec` and `pos` travel in
 * separate eightbytes of the calling convention: at 12 bytes gcc passes `store_nr|rec` and
 * `pos` in two registers, spills them as an 8-byte and a 4-byte store, and reloads `rec|pos`
 * with one 8-byte load spanning both — a store-forwarding stall on every reference a call
 * takes.  The pad is never read; a positional initialiser leaves it zero.  Measured over the
 * library benches: gcc −3 % (cbor decode −35 %), clang neutral (@PLN210 P1). */
typedef struct { uint16_t store_nr; uint32_t rec; uint32_t pos; uint32_t pad_; } loft_ref;

#ifndef LOFT_NO_JUMPS
#include <setjmp.h>
/* MinGW-w64 on 64-bit Windows spells `setjmp(b)` as `_setjmp(b, <frame>)`, which makes the
 * `longjmp` out of a failed test run Windows' SEH unwind through every frame in between — and
 * that unwind faults on frames gcc optimised (2121's halted tests, strict build, -O1: a jump to
 * nowhere; -O0, `-fno-inline` or a null frame: right).  A null frame is MinGW's own way to ask
 * for the plain register restore every other platform's `longjmp` does; no loft frame has a
 * destructor for the unwind to run. */
#if defined(__MINGW32__) && defined(_WIN64)
#undef setjmp
#define setjmp(b) _setjmp((b), NULL)
#endif
#define LOFT_JMP_BUF jmp_buf
#define LOFT_LONGJMP(b) longjmp(b, 1)
#else
/* A unit that runs no test and binds no lazy collection never jumps (`LOFT_NO_JUMPS`, a browser
 * page's module): wasm has setjmp only through its exception-handling proposal, which asyncify
 * cannot carry, so the unit holds no jump buffer at all and these sites are unreachable. */
typedef struct { int unused; } loft_no_jmp_buf;
#define LOFT_JMP_BUF loft_no_jmp_buf
#define LOFT_LONGJMP(b) abort()
#endif

/* `stack_trace()` reads the frames, so a unit that reaches it keeps them in every build; any
 * other unit keeps them where the build asks for the call chain a fault prints
 * (`-DLOFT_CALL_CHAIN`, the semantics tier) and compiles the pushes away where it does not. */
#if (defined(LOFT_FRAMES) || defined(LOFT_LEAK_SITES) || defined(LOFT_STRICT_STORES) || defined(LOFT_DOUBLE_FREE) \
     || defined(LOFT_ALLOC_TRACE)) \
    && !defined(LOFT_CALL_CHAIN)
#define LOFT_CALL_CHAIN 1
#endif

#define LOFT_TP_CLASSES 9
#define LOFT_TP_DEPTH 32

#define LOFT_TA_BASE 65536

#define LOFT_SLICE_POOL 16

#ifdef LOFT_HOSTED
/* One local of the routine, offered to the interpreter by name: kind 0 integer, 1 float,
 * 2 text, 3 reference, 4 boolean or plain enum, 5 single. */
typedef struct {
    const char *name;
    uint32_t kind, rs;
    loft_word bits;
    const char *p;
    size_t len;
    uint32_t rr, rp;
} loft_ho_entry;
#endif

typedef void (*loft_show1_fn)(loft_cx *, loft_text *, loft_ref, int, int);

struct loft_cx {
    /* the text pool (`loft_text.h`) */
    char *tp_blocks[LOFT_TP_CLASSES][LOFT_TP_DEPTH];
    unsigned char tp_n[LOFT_TP_CLASSES];
    int no_park;
    /* the store table (`loft_store.h`) */
    struct loft_store_t *stores;
    uint32_t nstores;
    uint64_t stores_allocated;
    const struct loft_store_t *hint_table;
    uint32_t hint;
    uint64_t hgen_next;
    uint64_t heap_total;
#if defined(LOFT_ALLOC_TRACE)
    uint32_t trace_worker;
    char trace_name_buf[48];
#endif
    /* the runtime (`loft_rt.h`) */
    LOFT_JMP_BUF test_jmp;
    int in_test;
    loft_text fail_msg;
#if defined(LOFT_HALT_RELEASE)
    struct loft_held_frame *held;
    size_t nheld;
    size_t capheld;
    size_t lazy_held_floor;
#endif
    LOFT_JMP_BUF *lazy_jmp;
    loft_text lazy_fault;
#if defined(LOFT_CALL_CHAIN)
    struct loft_frame *frames;
    size_t nframes;
    size_t capframes;
    size_t frames_floor;
#endif
#if defined(LOFT_CALL_CHAIN) && defined(LOFT_ALLOC_TRACE)
    uint32_t trace_last_site;
#endif
#if !defined(LOFT_CALL_CHAIN) && defined(LOFT_HOSTED)
    uint32_t hdepth;
#elif !defined(LOFT_CALL_CHAIN) && defined(LOFT_MAX_CALL_DEPTH)
    uint32_t cdepth;  /* the lean tier's call depth (`cr_call_push_lean`) */
#endif
#if (defined(LOFT_STRICT_STORES) || defined(LOFT_DOUBLE_FREE))
    struct loft_free_site *freed_at;
    uint32_t nfreed_at;
#endif
#if defined(LOFT_STRICT_STORES)
    uint32_t strict_floor;
#endif
#if defined(LOFT_LEAK_SITES)
    struct loft_leak_site *sites;
    uint32_t nsites;
#endif
#if defined(LOFT_HOSTED)
    void *host_stores;
    uint32_t br_hdepth;
    int ho_off;
    loft_ho_entry hov[256];
    uint32_t nhov;
    loft_text br_answer;  /* a bridge call's text answer, held until the next (`hosted.rs`) */
    loft_text gen_text;   /* a generator step's text value (`loft_shared_gen_next`) */
#endif
    struct loft_evac *evacs;
    uint32_t nevacs;
    uint32_t evac_cap;
    uint64_t ta_base[LOFT_TA_BASE / 8];
    size_t ta_top;
    struct loft_ta_blk *ta_spill;
    loft_ref sp_ref[LOFT_SLICE_POOL];
    uint8_t sp_used[LOFT_SLICE_POOL];
    int sp_n;
    struct loft_gen_slot *gens;
    uint32_t ngens;
    loft_ref **park_slots;
    uint32_t npark_slots;
    uint32_t park_slots_cap;
    struct loft_fnref_held *fnrefs;
    size_t nfnrefs;
    size_t capfnrefs;
    struct loft_child_job *child_jobs;
    size_t child_njobs;
    size_t child_cap;
    int child_depth;
    struct loft_show_closer *closers;
    size_t nclosers;
    size_t closers_cap;
    loft_show1_fn tail_fn;
    loft_ref tail_ref;
    int tail_ind;
    int rr_on;
    loft_text rr_path;  /* "/leaves/1": the pointer without its '#' */
    size_t *rr_marks;  /* the path's length before each push */
    size_t rr_nmarks;
    size_t rr_cmarks;
    struct loft_rr_place *rr_at;  /* a record's pointer, in cx->rr_pool */
    size_t rr_nat;
    size_t rr_cat;
    size_t *rr_tab;  /* open-addressed index into cx->rr_at */
    size_t rr_ctab;
    loft_text rr_pool;
    struct loft_rr_target *rr_t;
    size_t rr_nt;
    size_t rr_ct;
    loft_text rr_tail_path;
    size_t *rr_tail_marks;
    size_t rr_tail_nmarks;
    int rr_tail_set;
    uint8_t *pk_p;
    size_t pk_n;
    size_t pk_cap;
    struct loft_eq_job *eq_jobs;
    size_t eq_njobs;
    size_t eq_jobs_cap;
    loft_ref *eq_seen;
    size_t eq_seen_n;
    size_t eq_seen_cap;
#if defined(LOFT_FFI)
    /* a library native's claim log (`loft_ffi_host.h`) */
    int ffi_log_on;
    uint16_t ffi_log_store;
    uint32_t *ffi_log_recs;
    uint32_t ffi_log_n, ffi_log_cap;
#endif
    /* `par` (`loft_keyed.h`) */
    struct loft_par_buf *par_bufs;
    int npar;
    int par_cap;
#if (defined(LOFT_THREADS) || defined(LOFT_FFI))
    int par_sharing;
#endif
    struct loft_store_t *par_caller;
    uint32_t par_caller_n;
};

/* The main thread's context: the one `main` hands to everything it calls.  Its fields start at
 * zero but for those `loft_cx_fresh` sets, which it sets the same way. */
static loft_cx loft_main_cx = {
#ifdef LOFT_HOSTED
    .br_hdepth = 0xFFFFFFFFu,
#endif
    .in_test = 0
};

/* A context as a new thread starts it: what `loft_main_cx` holds before `main` runs. */
static void loft_cx_fresh(loft_cx *cx) {
    memset(cx, 0, sizeof *cx);
#ifdef LOFT_HOSTED
    cx->br_hdepth = 0xFFFFFFFFu;
#endif
}
#endif
