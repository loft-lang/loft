/* lcshim2's shim: one function that needs the C library (`strlen`), so a wasm build of a
 * program using this package and `lcshim` links two packages' copies of it (loft#2280). */
#include <stdint.h>
#include <string.h>

int64_t lc2_len(const char *s);

int64_t lc2_len(const char *s) {
    return (int64_t) strlen(s) * 10;
}
