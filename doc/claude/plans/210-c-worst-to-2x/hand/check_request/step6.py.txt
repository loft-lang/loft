# FLOOR — (R-DecodeView) structure clause, as its emission would run: nothing of the tree is
# built.  `decode(frame).ok` is the shape twin (read_value with every build statement dropped);
# `pa_text(pa_decode(frame), "op")` is the walk twin over the top map, re-reading each entry at
# its position.  The text answered is still a fresh copy (the twin's `.to_string()`).
import sys
from lib import *
src = open(sys.argv[1]).read()
HS = r'''
/* hand: read_value's shape twin — answers next (or -1 when !ok), the major type and the
 * text/bytes span; builds nothing. */
static int64_t hs_read(const uint8_t *b, int64_t n, int64_t pos, int64_t depth, int *maj, int64_t *lo, int64_t *len) {
    if (pos >= n || depth > 256) return -1;
    int64_t bb = b[pos], major = bb / 32, info = bb % 32, arg = 0, ap = pos + 1;
    if (info < 24) arg = info;
    else if (info == 24) { if (ap >= n) return -1; arg = b[ap]; ap += 1; if (arg < 24) return -1; }
    else if (info == 25) { if (ap + 1 >= n) return -1; arg = b[ap] * 256 + b[ap + 1]; ap += 2; if (arg < 256) return -1; }
    else if (info == 26) { if (ap + 3 >= n) return -1; arg = ((b[ap] * 256 + b[ap + 1]) * 256 + b[ap + 2]) * 256 + (int64_t) b[ap + 3]; ap += 4; if (arg < 65536) return -1; }
    else if (info == 27) { if (ap + 7 >= n || b[ap] >= 128) return -1; arg = b[ap]; for (int k = 1; k < 8; k++) arg = arg * 256 + b[ap + k]; ap += 8; if (arg < 4294967296LL) return -1; }
    else return -1;
    *maj = (int) major;
    if (major == 0 || major == 1) return ap;
    if (major == 2 || major == 3) { if (ap + arg > n) return -1; *lo = ap; *len = arg; return ap + arg; }
    if (major == 4) { int m; int64_t l, ln, p = ap; for (int64_t i = 0; i < arg; i++) { p = hs_read(b, n, p, depth + 1, &m, &l, &ln); if (p < 0) return -1; } return p; }
    if (major == 5) { int m; int64_t l, ln, p = ap; for (int64_t i = 0; i < arg; i++) { p = hs_read(b, n, p, depth + 1, &m, &l, &ln); if (p < 0) return -1; p = hs_read(b, n, p, depth + 1, &m, &l, &ln); if (p < 0) return -1; } return p; }
    if (major == 7 && (info == 20 || info == 21 || info == 22)) return ap;
    return -1;
}
static loft_text hs_check_request(loft_ref frame) {
    loft_store_t *s = loft_store_of(frame);
    const uint8_t *b = loft_elem_base(s, loft_collection_rec(s, frame.rec, frame.pos));
    int64_t n = loft_vec_len(frame), lo = 0, len = 0;
    int maj = -1;
    int64_t next = hs_read(b, n, 0, 0, &maj, &lo, &len);
    if (next < 0 || next != n) return loft_text_own(loft_lit("malformed-frame", 15));
    loft_text op = { 0, 0, 0 };
    if (maj == 5) {
        /* the walk twin: the top map's entries, each re-read at its position */
        int64_t bb = b[0], info = bb % 32, cnt = info < 24 ? info : 0, p = 1;
        if (info == 24) { cnt = b[1]; p = 2; } else if (info == 25) { cnt = b[1] * 256 + b[2]; p = 3; }
        for (int64_t i = 0; i < cnt; i++) {
            int km, vm = -1; int64_t kl = 0, kn = 0, vl = 0, vn = 0;
            int64_t q = hs_read(b, n, p, 1, &km, &kl, &kn);
            int64_t r = hs_read(b, n, q, 1, &vm, &vl, &vn);
            p = r;
            if (km == 3 && kn == 2 && loft_utf8_valid(b + kl, 2) && memcmp(b + kl, "op", 2) == 0) {
                if (vm == 3 && loft_utf8_valid(b + vl, (size_t) vn)) op = loft_text_own((loft_str){ (const char *) b + vl, (size_t) vn });
                break;
            }
        }
    }
    uint8_t ok = n_valid_op__fr(loft_view(&op));
    loft_text_free(&op);
    if (!ok) return loft_text_own(loft_lit("unknown-op", 10));
    return loft_text_own(loft_lit("", 0));
}
'''
src = sub1(src, 'static loft_text n_check_request__fr(loft_ref v0_frame) {\n',
    HS + 'static loft_text n_check_request__fr(loft_ref v0_frame) {\n    return hs_check_request(v0_frame);\n')
open(sys.argv[2], 'w').write(src)
