// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! How a running `State` — and the parallel context its workers inherit — reaches the
//! definition table it was run against.
//!
//! A run borrows its `Data` (`State::execute_argv(name, &data, …)`) and the `State`
//! keeps reading it afterwards: a fn-ref call sizes its callee's buffers from it, a
//! fault names its call chain from it, and a host that resumes the program across calls
//! (the browser kernel, the live dispatcher) re-points it with `State::rebind_data`
//! whenever the `Data` has moved.  No lifetime can express "valid until the host's next
//! rebind", so the handle is a pointer; this type is the ONE place it is dereferenced.

use crate::data::Data;
use std::ptr::NonNull;

/// A handle on the `Data` a `State` runs against, or none: a `State` before its first
/// run, and a parallel worker spawned without one, hold [`DataRef::NONE`].
///
/// **Soundness is the installer's:** whoever builds a handle with [`DataRef::new`] keeps
/// that `Data` alive, at that address, for as long as the handle is reachable — a run
/// installs it for its own duration, and a resuming host re-installs it before every
/// resumption.  A caller holding both as locals gets that from drop order, by declaring
/// the `Data` FIRST so the `State` drops before it.
#[derive(Clone, Copy, Default)]
pub struct DataRef(Option<NonNull<Data>>);

impl DataRef {
    /// No definition table: every reader answers its "outside a run" default.
    pub const NONE: Self = Self(None);

    /// A handle on `data`; see the type's soundness note.
    #[must_use]
    pub fn new(data: &Data) -> Self {
        Self(Some(NonNull::from(data)))
    }

    /// Whether no definition table is installed.
    #[must_use]
    pub fn is_none(self) -> bool {
        self.0.is_none()
    }

    /// The definition table, or `None` outside a run.  The borrow is tied to the handle,
    /// so a reader that must also mutate its `State` copies the handle into a local
    /// first: `let handle = self.data_ptr; let Some(data) = handle.get() else { … };`.
    #[must_use]
    pub fn get(&self) -> Option<&Data> {
        // SAFETY: a handle is only built from a live `&Data` by `DataRef::new`, and its
        // installer keeps that `Data` alive and in place while the handle is reachable
        // (the type's soundness note).
        self.0.map(|p| unsafe { p.as_ref() })
    }
}
