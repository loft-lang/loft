#!/usr/bin/env python3
"""X8 price on the crawler lane's `read_i16_vec` (bench/18_consumer_crawler, the `while`
that pushes `read_one_i16(bf)`): the File's handle, byte order and cursor held in locals
across the loop.  The first value goes through the per-value read, which opens the handle;
every later one takes its two bytes straight off the buffered file, and the cursor is
written back to the File record once, after the loop.  Every edit asserts the line it
replaces.

    loft --native-release --native-emit l18.rs bench/18_consumer_crawler/bench.loft
    price_binary_read.py l18.rs l18_hold.rs
    bench/portal/hand_price.sh l18.rs l18 && bench/portal/hand_price.sh l18_hold.rs l18_hold
"""
import sys

if len(sys.argv) != 3 or sys.argv[1] in ("-h", "--help"):
    print(__doc__)
    sys.exit(0 if "--help" in sys.argv or "-h" in sys.argv else 2)
src, dst = sys.argv[1:3]
L = open(src, encoding="utf-8").read().split("\n")
i = next(k for k, l in enumerate(L) if l.startswith("fn n_read_i16_vec("))
j = next(k for k in range(i + 1, len(L)) if L[k].startswith("fn "))
done = 0
for k in range(i, j):
    if "let mut __ph_1 = vector::push_header(&(var_out), &stores.allocations);" in L[k]:
        L[k] += "\n    let mut __fh: i32 = i32::MIN; let mut __le = false; let mut __cur: i64 = 0; let mut __first = true;"
        done += 1
    if L[k].strip() == "let _pre_39 = n_read_one_i16(cell, var_bf);":
        L[k] = """      let _pre_39 = if __first {
        let v = n_read_one_i16(cell, var_bf); __first = false;
        let st = stores.store(&var_bf);
        __le = st.get_byte(var_bf.rec, var_bf.pos + 32, 0) == 2; __cur = st.get_long(var_bf.rec, var_bf.pos + 16); __fh = st.get_i32_raw(var_bf.rec, var_bf.pos + 28);
        v
      } else {
        match stores.files.get_mut(__fh as usize).and_then(|x| x.as_mut()).and_then(|f| f.take_array::<2>()) {
          Some(b) => { __cur += 2; i64::from(if __le { i16::from_le_bytes(b) } else { i16::from_be_bytes(b) }) }
          None => i64::MIN,
        }
      };"""
        done += 10
    if L[k].strip() == "} /*while_2*/ };":
        L[k] = """    } /*while_2*/
  if !__first { let st = stores.store_mut(&var_bf); st.set_long(var_bf.rec, var_bf.pos + 8, __cur - 2); st.set_long(var_bf.rec, var_bf.pos + 16, __cur); } };"""
        done += 100
assert done == 111, done
open(dst, "w", encoding="utf-8").write("\n".join(L))
