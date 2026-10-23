// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I72 — Parallel execution runtime (the browser half: where `par`'s threads come from)
// @PLN117 — loft's own browser thread pool (no wasm-bindgen)

//! loft's browser thread pool: `par` on real Web Worker threads.
//!
//! rayon does the scheduling, exactly as on the native backend, so `par` and
//! `par_fold` behave the same everywhere.  Only the part rayon cannot do itself
//! is loft's: a browser has no `thread::spawn`, so the pool's threads arrive
//! from the outside.  The host (`doc/loft-thread.js`) spawns N Web Workers, each
//! instantiating the SAME module against the SAME shared memory, and each calls
//! [`loft_rayon_start_worker`] to collect one of the `ThreadBuilder`s that
//! [`loft_pool_build`] parked here.
//!
//! ## Why start-up is two calls and not one
//!
//! Spawning a Worker is asynchronous, and installing the pool waits for every
//! worker to prime.  Doing both inside one wasm call therefore deadlocks: the
//! main thread waits for workers that cannot finish booting until it returns to
//! the event loop (measured — @PLN117 amendment A2).  So the host calls
//! [`loft_pool_new`], awaits each worker's *ready* message, and only then calls
//! [`loft_pool_build`].
//!
//! Because the host has to sequence start-up anyway, it also does the spawning:
//! [`loft_pool_new`] hands back the handoff pointer and the host takes it from
//! there.  Nothing here calls out to JS, so this runtime needs no host import of
//! its own — which is what lets the SAME pool serve both the raw `--html`
//! bundle and the wasm-bindgen gallery one, whose glue builds its imports itself
//! and would reject an extra module.
//!
//! ## What the host must do per worker
//!
//! Every `WebAssembly.Instance` gets its own copy of the mutable globals, but
//! they all start at the SAME values — so a worker that runs any wasm before
//! being moved off the main thread's shadow stack corrupts it.  The host
//! allocates each worker a stack and a TLS block with [`loft_thread_alloc`],
//! sets that instance's `__stack_pointer`, and calls `__wasm_init_tls` before
//! anything else.  wasm-bindgen's thread transform does this internally; the raw
//! path does it from JS, which is why the build exports `__stack_pointer`.
//!
//! Not initialising the pool at all is a supported state: rayon falls back to a
//! single-threaded pool, so `par` runs sequentially and still returns the same
//! values (arc D — proven never to break).

use rayon::{ThreadBuilder, ThreadPoolBuilder};
use std::sync::atomic::{AtomicPtr, Ordering};

/// Upper bound on pool size.  `navigator.hardwareConcurrency` is host-controlled
/// and each worker costs a shadow stack plus a TLS block out of the wasm heap, so
/// the runtime — not the page — decides how far that can go.
const MAX_THREADS: u32 = 32;

/// Where [`loft_pool_build`]'s spawn handler parks each rayon `ThreadBuilder`
/// until a Web Worker calls [`loft_rayon_start_worker`] to claim it.
///
/// The locks are loft's own (`wasm_sync`, wired in through `[patch.crates-io]`):
/// the main thread pushes here and may not block, so it spins; the workers wait
/// and may block, so they park.
struct Handoff {
    threads: usize,
    queue: wasm_sync::Mutex<Vec<ThreadBuilder>>,
    arrived: wasm_sync::Condvar,
}

/// The single pool per page, or null before [`loft_pool_new`].
static HANDOFF: AtomicPtr<Handoff> = AtomicPtr::new(std::ptr::null_mut());

/// Reserve a zeroed, leaked block of wasm memory — the host carves each worker's
/// shadow stack and TLS block out of calls to this.
///
/// Returns 0 when the request cannot be satisfied, which the host reports rather
/// than spawning a worker onto a null stack.
#[unsafe(no_mangle)]
pub extern "C" fn loft_thread_alloc(size: u32, align: u32) -> u32 {
    let align = (align.max(8) as usize).next_power_of_two();
    let Ok(layout) = std::alloc::Layout::from_size_align(size.max(1) as usize, align) else {
        return 0;
    };
    // SAFETY: a non-zero-size layout; the block is deliberately never freed — it
    // backs a thread that lives as long as the page.
    (unsafe { std::alloc::alloc_zeroed(layout) }) as u32
}

/// Start building the pool, and return the handoff pointer the host must pass to
/// every worker it spawns (0 if a pool already exists).
///
/// Call this from the page's main thread — it is the thread that may not block,
/// and this is where loft says so.
#[unsafe(no_mangle)]
pub extern "C" fn loft_pool_new(num_threads: u32) -> u32 {
    if !HANDOFF.load(Ordering::Acquire).is_null() {
        return 0;
    }
    let threads = num_threads.clamp(1, MAX_THREADS) as usize;
    let handoff: &'static Handoff = Box::leak(Box::new(Handoff {
        threads,
        queue: wasm_sync::Mutex::new(Vec::with_capacity(threads)),
        arrived: wasm_sync::Condvar::new(),
    }));
    let ptr = std::ptr::from_ref(handoff).cast_mut();
    HANDOFF.store(ptr, Ordering::Release);
    // This thread runs the page: `memory.atomic.wait32` throws here, so every
    // lock it takes from now on — including rayon's own — has to spin instead.
    wasm_sync::set_can_block(false);
    ptr as u32
}

/// Install the global rayon pool over the workers the host has spawned.  Call
/// only after every worker has reported ready.  Returns 0 on success.
#[unsafe(no_mangle)]
pub extern "C" fn loft_pool_build() -> i32 {
    let ptr = HANDOFF.load(Ordering::Acquire);
    if ptr.is_null() {
        return -1;
    }
    // SAFETY: only ever set from `loft_pool_new`, to a leaked allocation.
    let handoff: &'static Handoff = unsafe { &*ptr };
    let built = ThreadPoolBuilder::new()
        .num_threads(handoff.threads)
        .spawn_handler(|thread| {
            let mut queue = handoff.queue.lock().expect("loft: thread handoff poisoned");
            queue.push(thread);
            // Notify while still holding the lock: that is what lets a spinning
            // waiter read the state and the wake-up as one step.
            handoff.arrived.notify_one();
            Ok(())
        })
        .build_global();
    if built.is_ok() { 0 } else { -2 }
}

/// Report, into the page's own output, how many worker threads each `par`
/// dispatch actually used.
///
/// A page that expected parallelism has no other way to tell whether it got it —
/// a host without cross-origin isolation silently runs everything on one thread,
/// with correct results either way.  The page shell arms this from
/// `?loftTrace=1`; it is also what the in-browser threading gates read.
#[unsafe(no_mangle)]
pub extern "C" fn loft_set_par_trace(on: u32) {
    crate::par_pool::set_par_trace(on != 0);
}

/// A Web Worker's entry point: claim one rayon thread and run it.  Never
/// returns while the pool lives.
///
/// The host calls this after moving the worker onto its own stack and TLS block.
#[unsafe(no_mangle)]
pub extern "C" fn loft_rayon_start_worker(handoff: *const u8) {
    // A Worker is not the page's main thread, so it may park rather than spin.
    wasm_sync::set_can_block(true);
    // SAFETY: the host passes back the pointer `loft_pool_new` handed it, which
    // refers to a leaked `Handoff`.
    let handoff: &Handoff = unsafe { &*handoff.cast::<Handoff>() };
    let thread = {
        let mut queue = handoff.queue.lock().expect("loft: thread handoff poisoned");
        loop {
            if let Some(thread) = queue.pop() {
                break thread;
            }
            queue = handoff
                .arrived
                .wait(queue)
                .expect("loft: thread handoff poisoned");
        }
    };
    thread.run();
}
