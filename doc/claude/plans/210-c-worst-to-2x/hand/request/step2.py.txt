# step 2: the decode loop's push keeps the length in the header (a push WINDOW for an 8-byte
# element): the slot is written, the length written back once where the loop ends or exits.
import sys
src, dst = sys.argv[1], sys.argv[2]
c = open(src).read()
helper = '''
LOFT_HOT_INLINE void loft_ph_wput8(loft_ph *h, uint64_t bits) {
    if (LOFT_LIKELY(h->len < h->cap)) {
        memcpy(h->base + 8 + (size_t) h->len * 8, &bits, 8);
        h->len++;
        return;
    }
    if (h->base && h->cap) memcpy(h->base + 4, &h->len, 4);
    loft_ph_put8(h, bits);
}
static inline void loft_ph_wsync(loft_ph *h) { if (h->base && h->cap) memcpy(h->base + 4, &h->len, 4); }
'''
anchor = "/* `LOFT_VERIFY`'s proof for a push header: a fresh derivation"
assert c.count(anchor) == 1
c = c.replace(anchor, helper + anchor)
s = c.index("static loft_ref n_b64_decode__fr(loft_str v0_b64, loft_ref v1___retbuf, loft_ref v2_vals) {")
e = c.index("\n}\n", s)
f = c[s:e]
f = f.replace("loft_ph_putb8(&ph1, (uint64_t) v8_v)", "loft_ph_wput8(&ph1, (uint64_t) v8_v)")
# every way out of the loop syncs the length first
f = f.replace("            v34___ret_tail_1 = loft_ref_null();", "            loft_ph_wsync(&ph1);\n            v34___ret_tail_1 = loft_ref_null();")
f = f.replace("            v35___ret_tail_2 = loft_ref_null();", "            loft_ph_wsync(&ph1);\n            v35___ret_tail_2 = loft_ref_null();")
f = f.replace("    brk_1:;\n", "    brk_1:;\n    loft_ph_wsync(&ph1);\n", 1)
c = c[:s] + f + c[e:]
open(dst, "w").write(c)
