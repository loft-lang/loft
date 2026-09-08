<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# V-g — the working form beside the copy (the loft-codegen gate, step 1)

Probe: [`V-g-read-only-view.loft`](V-g-read-only-view.loft) — `view_a(pts, i) -> Pt { pts[i]? }`
bound into a read-only local, written inline (`use_inline`) and through the call
(`use_call`).  Captured with `loft introspect` on 2026-09-08, before and after the elision.

## The working reference — the projection written inline (unchanged by the elision)

```
fn n_use_inline(pts:vector<ref(Pt)>) -> float {#block(1):float
  __ref_p2_1(1):ref(Pt) = null;
  __ncc_1(1):ref(Pt)["pts"] = null;
  [5] a(1):ref(Pt)["pts"] = {#ncc(2):ref(Pt)["pts"]
    __ncc_1(1):ref(Pt)["pts"] = OpGetVectorNullable(pts(0), 16i32, 1i32);
    if OpConvBoolFromRef(__ncc_1(1)) __ncc_1(1) else {#Object(3):ref(Pt)["__ref_p2_1"]
      OpDatabase(__ref_p2_1(1), 81i32);
      OpSetFloat(__ref_p2_1(1), 0i32, 0f64);
      OpSetFloat(__ref_p2_1(1), 8i32, 0f64);
      __ref_p2_1(1);
    }#Object(3):ref(Pt)["__ref_p2_1"];
  }#ncc(2):ref(Pt)["pts"];
  __ret_1(1):float = OpAddFloat(OpGetFloat(a(1), 0i32), OpGetFloat(a(1), 8i32));
  OpFreeRef(__ref_p2_1(1));
  return __ret_1(1);
}#block(1):float
```

`a` is a VIEW (`ref(Pt)["pts"]`); the `?` fallback's minted record lives in a hidden owner
(`__ref_p2_1`) that the scope exit frees; `a` itself is never freed.  No store in range.

## Through the call — BEFORE (the borrow-copy)

```
fn n_use_call(pts:vector<ref(Pt)>) -> float {#block(1):float
  __ref_1(1):ref(Pt) = null;
  [7] a(1):ref(Pt) = n_view_a(pts(0), 1i32, __ref_1(1));
  __ret_1(1):float = OpAddFloat(OpGetFloat(a(1), 0i32), OpGetFloat(a(1), 8i32));
  OpFreeRef(a(1));
  OpFreeRef(__ref_1(1));
  return __ret_1(1);
}#block(1):float
```

`a` has no dep (stripped by `scan_set`), the interpreter binds it through `OpBindOrCopy`
(materialise the view arm, adopt the minted arm) and frees it unconditionally; native
copies into a `null_named` slot store.  One store per call.

## Through the call — AFTER (the elision)

```
fn n_use_call(pts:vector<ref(Pt)>) -> float {#block(1):float
  __ref_1(1):ref(Pt) = null;
  [7] a(1):ref(Pt)["pts"] = n_view_a(pts(0), 1i32, __ref_1(1));
  __ret_1(1):float = OpAddFloat(OpGetFloat(a(1), 0i32), OpGetFloat(a(1), 8i32));
  OpFreeRefIfDistinct(a(1), pts(0));
  OpFreeRef(__ref_1(1));
  return __ret_1(1);
}#block(1):float
```

`a` keeps its dep — the inline form's view — and the scope exit releases only a minted
arm, by store identity against the argument (`OpFreeRefIfDistinct(a, pts)`): the collection
join's route (loft#1257) applied to a record whose copy no program can observe.  Both
backends bind it with the plain delivery.  Zero stores in range, one (freed) out of range.
