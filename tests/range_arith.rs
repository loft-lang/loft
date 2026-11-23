// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Range` — the EMISSION pins.  An integer operator whose RESULT provably fits the
//! type emits the processor's operator (`wrapping_add`/`wrapping_sub`/`wrapping_mul`/
//! `wrapping_neg`, a bare `/` or `%`); one whose range the proof cannot bound keeps its
//! checked template (`ops::op_add_int(` and kin, the `*Nullable` twins, the `_long_nn`
//! forms).  `LOFT_NO_RANGE_ARITH=1` restores every template.  The cell corpus
//! (`tests/scripts/157-range-arith.loft`) says the VALUES hold on both backends, in every
//! switch state and under the falsifiers; this pins what is emitted.
