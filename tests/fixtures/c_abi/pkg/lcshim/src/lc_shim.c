/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * @PLN24 arc D — the ANSI-C shim, at its real size.
 *
 * One wrapper per shape the fixed integer-class trampolines cannot express.
 * The plan's whole trade is "complexity in the shim, not in loft-core", and
 * these are the lines that trade costs: if they were not small, the trade
 * would not be one anyone should take.
 */
#include <stdint.h>
#include <string.h>
#include <stdlib.h>

/* A double travels in an SSE register, not the integer registers the caller
 * uses — a different register file, so no cast reaches it.  It crosses as its
 * bit pattern instead.
 *
 * `int64_t`, never `long`: a bit pattern is 64 bits and C `long` is 32 of them
 * on Windows (LLP64).  With `long` the `memcpy(&v, &bits, 8)` below read FOUR
 * BYTES PAST the argument, and the Windows leg reported `scale 0` — the low half
 * of 10.0's pattern, which is all zeroes. */
static double lcs_scale(double v, double by) { return v * by; }

int64_t lcs_shim_scale(int64_t bits, int64_t by_bits);
int64_t lcs_shim_scale(int64_t bits, int64_t by_bits) {
  double v, by, r;
  memcpy(&v, &bits, 8);
  memcpy(&by, &by_bits, 8);
  r = lcs_scale(v, by);
  memcpy(&bits, &r, 8);
  return bits;
}

/* An out-parameter: two answers, one return slot. */
static int64_t lcs_divmod(int64_t a, int64_t b, int64_t *rem) {
  if (b == 0) { *rem = 0; return 0; }
  *rem = a % b;
  return a / b;
}

int64_t lcs_shim_mod(int64_t a, int64_t b);
int64_t lcs_shim_mod(int64_t a, int64_t b) { int64_t r; lcs_divmod(a, b, &r); return r; }

/* A caller-frees return.  loft never frees a `char *` it is handed, so this is
 * the shape that has to go through a shim: the owned buffer is copied into
 * storage the caller must NOT free, and released here. */
const char *lcs_shim_owned(int64_t n);
const char *lcs_shim_owned(int64_t n) {
  static char buf[32];
  char *owned = (char *)malloc(16);
  if (owned == 0) { return "?"; }
  memcpy(owned, "shim-", 6);
  buf[0] = '\0';
  strcat(buf, owned);
  free(owned);
  buf[5] = (char)('0' + (n % 10));
  buf[6] = '\0';
  return buf;
}

/* A vector read through its element pointer and count (@C107), and one WRITTEN through it:
 * the two crossings the C backend must carry as the interpreter does (`loft_cvec_ptr`). */
int64_t lcs_shim_sum(const int64_t *p, int64_t n);
int64_t lcs_shim_sum(const int64_t *p, int64_t n) {
  int64_t i, s = 0;
  for (i = 0; i < n; i++) { s += p[i] * (i + 1); }
  return s;
}
void lcs_shim_fill(double *out, int64_t n, int64_t start_milli);
void lcs_shim_fill(double *out, int64_t n, int64_t start_milli) {
  int64_t i;
  for (i = 0; i < n; i++) { out[i] = (double) (start_milli + i) / 1000.0; }
}
