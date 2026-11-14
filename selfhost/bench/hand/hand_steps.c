/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later */
/* The bytecode runner's step loop written by hand in C: the target the generated runner was
 * built to reach, kept as the reference for what its emitted C should come to.  State's code
 * position, stack top, stack capacity, stack base and code base are locals; the hot operators
 * are inline; any case that needs a check's slow path (stack growth, a slot outside the
 * stack, division by zero) or an operator not written here hands that one step to the
 * generated loop, with the locals written back first.  It ran collatz in 0.29 s against
 * `loft --interpret`'s 0.335 s.  The generated `run` (selfhost/src/fill.loft, written by
 * create.loft) is its port; measure that with selfhost/bench/fill_speed.sh.  This file is
 * read, not built: it was spliced into run_fill.loft's C before the step loop moved into
 * `run`. */
uint8_t *loft_crt_stack_base(int64_t store);
static inline int64_t hs_ld(const uint8_t *p) { int64_t v; memcpy(&v, p, 8); return v; }
static inline void hs_st(uint8_t *p, int64_t v) { memcpy(p, &v, 8); }
static void loft_hand_steps(loft_ref S, int64_t end) {
    uint8_t *pa = loft_rec_ptr(S);
    int64_t pos = loft_pa_i64(pa, 0);
    int64_t stk = loft_pa_i64(pa, 8);
    int64_t sp = loft_pa_i64(pa, 16);
    int64_t cap = loft_pa_i64(pa, 24);
    loft_vh code = loft_vh_of(loft_field(S, 32));
    const uint8_t *cb = code.base;
    int64_t clen = code.len;
    uint8_t *sb = loft_crt_stack_base(stk);
    for (;;) {
        int64_t at = pos;
        if (at == end || at < 0 || at >= clen) break;
        switch (cb[at]) {
        case 0: { /* GotoWord */
            if (at + 5 > clen) goto out;
            int32_t st; memcpy(&st, cb + at + 1, 4);
            pos = at + 5 + st; continue; }
        case 1: { /* GotoFalseWord */
            if (at + 5 > clen || sp < 8) goto out;
            int32_t st; memcpy(&st, cb + at + 1, 4);
            uint8_t f = sb[sp - 8]; sp -= 8;
            pos = at + 5 + (f != 1 ? st : 0); continue; }
        case 5: { /* ConstInt */
            if (at + 9 > clen || sp + 8 > cap) goto out;
            hs_st(sb + sp, hs_ld(cb + at + 1)); sp += 8; pos = at + 9; continue; }
        case 6: { /* VarInt */
            if (at + 3 > clen) goto out;
            uint16_t p; memcpy(&p, cb + at + 1, 2);
            int64_t a = sp - p;
            if (a < 0 || a + 8 > cap || sp + 8 > cap) goto out;
            hs_st(sb + sp, hs_ld(sb + a)); sp += 8; pos = at + 3; continue; }
        case 7: { /* PutInt */
            if (at + 3 > clen || sp < 8) goto out;
            uint16_t p; memcpy(&p, cb + at + 1, 2);
            int64_t v = hs_ld(sb + sp - 8), nsp = sp - 8, a = nsp + 8 - p;
            if (a < 0 || a + 8 > cap) goto out;
            hs_st(sb + a, v); sp = nsp; pos = at + 3; continue; }
        case 9: case 11: case 12: case 13: case 15: { /* AddInt MulInt DivInt RemInt EqInt */
            if (sp < 16) goto out;
            int64_t v2 = hs_ld(sb + sp - 8), v1 = hs_ld(sb + sp - 16);
            uint8_t o = cb[at];
            if ((o == 12 || o == 13) && v2 == 0) goto out;
            if (o == 15) { sb[sp - 16] = (uint8_t) (v1 == v2); }
            else {
                int64_t r = o == 9 ? loft_add(v1, v2) : o == 11 ? loft_mul(v1, v2)
                    : o == 12 ? loft_div_at(v1, v2, "fill.loft", 83) : loft_rem_at(v1, v2, "fill.loft", 90);
                hs_st(sb + sp - 16, r);
            }
            sp -= 8; pos = at + 1; continue; }
        case 33: { /* IntVCPut kind dst a c */
            if (at + 14 > clen) goto out;
            uint8_t k = cb[at + 1];
            uint16_t d, a; memcpy(&d, cb + at + 2, 2); memcpy(&a, cb + at + 4, 2);
            int64_t c = hs_ld(cb + at + 6), ra = sp - a, rd = sp + 8 - d;
            if (k > 2 || ra < 0 || ra + 8 > cap || rd < 0 || rd + 8 > cap) goto out;
            int64_t v = hs_ld(sb + ra);
            hs_st(sb + rd, k == 0 ? loft_add(v, c) : k == 1 ? loft_sub(v, c) : loft_mul(v, c));
            pos = at + 14; continue; }
        case 35: { /* CmpIntVCJump kind a c step */
            if (at + 16 > clen) goto out;
            uint8_t k = cb[at + 1];
            uint16_t a; memcpy(&a, cb + at + 2, 2);
            int64_t c = hs_ld(cb + at + 4), ra = sp - a;
            int32_t st; memcpy(&st, cb + at + 12, 4);
            if (ra < 0 || ra + 8 > cap) goto out;
            int64_t v = hs_ld(sb + ra);
            int t = k == 0 ? v == c : k == 1 ? v != c : k == 2 ? v < c : k == 4 ? c < v : k == 5 ? c <= v : v <= c;
            pos = at + 16 + (t ? 0 : st); continue; }
        default: goto out;
        }
    }
out:
    loft_set_int_at(S, 0, pos, "hand", 0);
    loft_set_int_at(S, 16, sp, "hand", 0);
}

