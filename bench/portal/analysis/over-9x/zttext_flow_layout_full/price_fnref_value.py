#!/usr/bin/env python3
"""Lever B — a fn-ref whose every dispatch target answers a value record rides the tuple:
`resolve(id)` answers Style as (f64,i64,bool,i64) straight from n_default_style (the match
over the known targets is kept), no store minted/adopted/freed per call; mono_measure takes
the size through its existing __inv variant, run_height already takes the tuple."""
import sys; from price_lib import edit
ST = "let __st_t: (f64, i64, bool, i64) = match var_resolve.0 { 861_u32 => n_default_style(cell), _ => (f64::NAN, i64::MIN, false, i64::MIN) };"
MM = lambda farg: "match var_measure.0 { 860_u32 => n_mono_measure__inv(cell, %s, DbRef::NULL, __st_t.0), _ => f64::NAN }" % farg
edit(sys.argv[1], sys.argv[2], [
 (1459, "_old___lift_1", "        " + ST + " let _ = _pre_8;"),
 (1461, "n_mono_measure(cell, _farg_0, _farg_1)", "        var_w = {(var_w) + ({ let _farg_0_h = _pre_9; let _farg_0: &str = &*_farg_0_h; " + MM("_farg_0") + " })};"),
 (1462, "OpFreeRef(cell,var___lift_1", ""),
 (1679, "_old___lift_1", "                " + ST + " let _ = _pre_10;"),
 (1680, "n_run_height(cell", "                let mut var_hh: f64 = n_run_height(cell, __st_t);"),
 (1685, "OpFreeRef(cell,var___lift_1", ""),
 (1776, "let _dst = var_st;", "                  " + ST + " let _ = _pre_10;"),
 (1779, "n_mono_measure(cell, _farg_0, _farg_1)", "                let mut var_rw: f64 = { let _farg_0_h = _pre_11; let _farg_0: &str = &*_farg_0_h; " + MM("_farg_0") + " };"),
 (1781, "n_run_height(cell", "                let mut var_rh: f64 = n_run_height(cell, __st_t);"),
 (1813, "OpFreeRef(cell,var_st,", ""),
 (1860, "let _dst = var_hst;", "      " + ST + " let _ = var_hstyle;"),
 (1862, "n_mono_measure(cell, _farg_0, _farg_1)", "    let mut var_hw: f64 = { let _farg_0: &str = \"-\"; " + MM("_farg_0") + " };"),
 (1864, "n_run_height(cell", "    let mut var_hh2: f64 = n_run_height(cell, __st_t);"),
 (1891, "OpFreeRef(cell,var_hst,", ""),
])
