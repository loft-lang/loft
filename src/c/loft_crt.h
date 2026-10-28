/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * The C runtime's text and store routines, includable by a separate C unit — a package's
 * `[c] shim` (@PLN91, the loft interpreter's `crt` library).  It is the first headers of the
 * runtime every C-backend unit starts with (`src/c/mod.rs`, `RUNTIME`), in that order; their
 * globals are `static`, so a shim linked into a C-backend program keeps its own copy and the
 * two never clash.  It carries the formatting routines (`loft_format_long`,
 * `loft_format_float`, `loft_format_text`) and the text buffer they write.
 *
 * Built at -O1 or above, as `c_shim` builds every shim (-O2): a few runtime functions in these
 * headers name routines a later header or the program defines, and only the optimiser's
 * dropping of the unused ones leaves no undefined symbol behind. */
#ifndef LOFT_CRT_H
#define LOFT_CRT_H
#include "loft_cc.h"
#include "loft_store.h"
#include "loft_rt.h"
/* The store's keyed collections (@PLN91 S3): `loft_copy_record` and the claim walks reach a
 * hash's, an index's and a radix tree's copy and removal, which these define. */
#include "loft_keys.h"
#include "loft_hash.h"
#include "loft_btree.h"
#include "loft_radix.h"
#include "loft_radix_walk.h"
#include "loft_radix_db.h"
#endif
