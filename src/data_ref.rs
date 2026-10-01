// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! How a running `State` — and the parallel context its workers inherit — reaches the
//! definition table it was run against.
//!
//! The `State` keeps reading that table after the call that started the run returns: a
//! fn-ref call sizes its callee's buffers from it, a fault names its call chain from it,
//! and a host that resumes the program across calls (the browser kernel, the live
//! dispatcher, a paused debugger) reads it on every resumption.  So the `State`
//! CO-OWNS the table: it holds a share of an `Arc<Data>`, and the table lives as long as
//! the last `State` or worker that can read it, wherever its first owner moved.

use crate::data::Data;
use std::sync::Arc;

/// A `State`'s share of the `Data` it runs against, or none: a `State` before its first
/// run, and a parallel worker spawned without one, hold [`DataRef::NONE`].
#[derive(Clone, Default)]
pub struct DataRef(Option<Arc<Data>>);

impl DataRef {
    /// No definition table: every reader answers its "outside a run" default.
    pub const NONE: Self = Self(None);

    /// A share of `data`.
    #[must_use]
    pub fn new(data: Arc<Data>) -> Self {
        Self(Some(data))
    }

    /// Whether no definition table is installed.
    #[must_use]
    pub fn is_none(&self) -> bool {
        self.0.is_none()
    }

    /// The definition table, or `None` outside a run.
    #[must_use]
    pub fn get(&self) -> Option<&Data> {
        self.0.as_deref()
    }

    /// The shared table itself, for handing to a worker or another `State`.
    #[must_use]
    pub fn shared(&self) -> Option<&Arc<Data>> {
        self.0.as_ref()
    }
}

/// What a run entry point accepts as its definition table.  A caller that already shares
/// its table hands the `Arc` and pays nothing; a caller holding a plain `&Data` gets a
/// private copy for the `State` to keep, which is always sound and costs one clone of the
/// table per run.
pub trait IntoSharedData {
    /// The table as a share the `State` may keep.
    fn into_shared(self) -> Arc<Data>;
}

impl IntoSharedData for &Data {
    fn into_shared(self) -> Arc<Data> {
        Arc::new(self.clone())
    }
}

impl IntoSharedData for &Arc<Data> {
    fn into_shared(self) -> Arc<Data> {
        Arc::clone(self)
    }
}

impl IntoSharedData for Arc<Data> {
    fn into_shared(self) -> Arc<Data> {
        self
    }
}
