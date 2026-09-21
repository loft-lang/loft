"""Benchmark 6: Newton's method square root (100,000 calls of 50 steps).

The timed loop runs `--n` ops, each on an input that differs by the repetition number in
VALUE, never in the amount of work, and folds every result into `sink`.  The `hash`
column is one canonical op's result; every lane must print the same one.
"""
import sys
import time

import math


def newton_sqrt(x):
    guess = x / 2.0
    for _ in range(50):
        guess = (guess + x / guess) / 2.0
    return guess


def roots(calls, salt):
    acc = 0.0
    for j in range(calls):
        acc += newton_sqrt(float(j + 1 + salt))
    return int(math.floor(acc + 0.5))

def arg_n(dflt):
    """`--n N`: how many ops the timed loop runs (the harness calibrates it per lane)."""
    n = dflt
    for i, a in enumerate(sys.argv):
        if a == "--n" and i + 1 < len(sys.argv):
            n = int(sys.argv[i + 1])
    return max(n, 2)


def row(name, iters, us, items, result, sink):
    """One measured routine, in the row format every lane prints (bench/README.md)."""
    print("routine\titers\tus\tns_op\tpx\tns_px\thash")
    print(f"{name}\t{iters}\t{us}\t{us * 1000 // iters}\t{items}\t{us * 1000 / (iters * items):.3f}\t{result:x}")
    print(f"time: {us // 1000}ms sink={sink}")


def timed(n, op):
    t0 = time.perf_counter_ns()
    sink = 0
    for r in range(n):
        sink += op(r)
    return (time.perf_counter_ns() - t0) // 1000, sink

if __name__ == "__main__":
    n = arg_n(2)
    calls = 100_000
    us, sink = timed(n, lambda r: roots(calls, r & 1))
    row("newton_sqrt", n, us, calls, roots(calls, 0), sink)
