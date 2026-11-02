# step2: the outer encode's array arm opens the append window once before the item loop and
# syncs it after, instead of per item (R-AppendTwin window clause, loop form).
import sys
s = open(sys.argv[1]).read()
b = "(loft_aw3 = loft_win_of(v1___retbuf, 1), t24 = n_encode__ap(t23, v1___retbuf, &loft_aw3), loft_win_sync(loft_aw3), t24);"
assert s.count(b) == 1
i = s.index(b)
f = s.rindex("for (;;) {", 0, i)
ind = s.rindex("\n", 0, f) + 1
pad = s[ind:f]
s = s[:ind] + pad + "loft_aw3 = loft_win_of(v1___retbuf, 1);\n" + s[ind:]
s = s.replace(b, "t24 = n_encode__ap(t23, v1___retbuf, &loft_aw3);")
i = s.index("t24 = n_encode__ap(t23, v1___retbuf, &loft_aw3);")
k = s.index("brk_2:;", i) + len("brk_2:;")
s = s[:k] + "\n" + pad + "loft_win_sync(loft_aw3);" + s[k:]
open(sys.argv[2], 'w').write(s)
