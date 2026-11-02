# loft_ref padded to 16 bytes: gcc no longer reloads rec+pos across the two argument-register
# halves (a store-forwarding stall per loft_ref argument per call).
import sys
from lib import *
src = open(sys.argv[1]).read()
src = sub1(src, 'typedef struct { uint16_t store_nr; uint32_t rec; uint32_t pos; } loft_ref;',
                'typedef struct { uint16_t store_nr; uint32_t rec; uint32_t pos; uint32_t pad_; } loft_ref;')
open(sys.argv[2], 'w').write(src)
