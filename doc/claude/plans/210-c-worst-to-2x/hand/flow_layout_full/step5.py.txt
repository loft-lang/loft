# step 5: a character-vector range turned into text in ONE pass: room reserved once for the
# range, elements read from the held header's base, ASCII bytes stored directly (the general
# per-char path kept for the rest and for an out-of-range index).
import sys
s=open(sys.argv[1]).read()
old='''    loft_text_set(out, loft_lit("", 0));
    for (int64_t j = start; j < start + count; j++) {
        uint32_t c = loft_get_char(loft_vh_get(hi0, 4, j), 0);
        loft_push_code(out, c != 0 ? c : loft_char_of(INT64_C(0)));
    }
'''
new='''    out->len = 0;
    if (start >= 0 && count > 0 && start + count <= hi0.len) {
        loft_reserve(out, (size_t) count);
        const uint8_t *src = hi0.base + (size_t) start * 4;
        for (int64_t j = 0; j < count; j++) {
            uint32_t c; memcpy(&c, src + j * 4, 4);
            if (c - 1u < 0x7Fu) out->p[out->len++] = (char) c;
            else loft_push_code(out, (c > 0x10FFFF || (c >= 0xD800 && c <= 0xDFFF)) ? loft_char_of(INT64_C(0)) : c);
        }
        return;
    }
    for (int64_t j = start; j < start + count; j++) {
        uint32_t c = loft_get_char(loft_vh_get(hi0, 4, j), 0);
        loft_push_code(out, c != 0 ? c : loft_char_of(INT64_C(0)));
    }
'''
assert s.count(old)==1
s=s.replace(old,new)
open(sys.argv[2],'w').write(s)
