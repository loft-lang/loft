# fn.py: helpers to slice one emitted function's DEFINITION out of a unit.
def span(s, name):
    a = s.index("\nstatic " , 0)
    key = " " + name + "("
    i = 0
    while True:
        i = s.index(key, i + 1)
        ls = s.rindex("\n", 0, i) + 1
        le = s.index("\n", i)
        if s[ls:le].rstrip().endswith("{") and s[ls:ls+7] == "static ":
            break
    j = s.index("\n}\n", i) + 3
    return ls, j
