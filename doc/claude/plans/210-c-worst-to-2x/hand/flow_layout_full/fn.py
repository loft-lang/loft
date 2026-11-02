# helper: return (start,end) of a function DEFINITION body (after the signature's ") {")
def body(s, name):
    import re
    m = re.search(r'\n(static[^\n;]*\b' + re.escape(name) + r'\([^;{]*\)) \{\n', s)
    assert m, name
    a = m.end() - 2
    b = s.index('\n}\n', a)
    return a, b
def edit(s, name, pairs):
    a, b = body(s, name)
    t = s[a:b]
    for old, new in pairs:
        assert t.count(old) == 1, (name, old, t.count(old))
        t = t.replace(old, new)
    return s[:a] + t + s[b:]
