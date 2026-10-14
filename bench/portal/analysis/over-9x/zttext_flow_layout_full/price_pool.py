#!/usr/bin/env python3
"""Lever A/E — token_width's `runs` result buffer (vector<Run>, text elements) is pooled
across calls instead of minted and freed per token: one static slot, never freed; slice_runs
still clears and refills it (clear_vector_release walks the previous texts, set_str claims
the new ones) — the in-place text refill of F6 is NOT priced here."""
import sys; from price_lib import edit
edit(sys.argv[1], sys.argv[2], [
 (1411, "fn n_token_width(", "static mut TW_POOL: DbRef = DbRef::NULL;\n" + open(sys.argv[1]).read().split('\n')[1410]),
 (1424, 'OpFreeRef(cell,var___ref_1, "var___ref_1")', ""),
 (1431, "var___ref_1 = OpDatabase(cell,var___ref_1, 106_i32)", "  var___ref_1 = unsafe { if TW_POOL.store_nr == u16::MAX { TW_POOL = OpDatabase(cell, DbRef::NULL, 106_i32); } TW_POOL }; if false {"),
 (1467, 'OpFreeRef(cell,var___ref_1, "var___ref_1")', ""),
])
