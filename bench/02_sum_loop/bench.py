"""Benchmark 2: Integer loop — 5 million steps of a loop-carried mix per op.

The timed loop runs `--n` ops, each on an input that differs by the repetition number in
VALUE, never in the amount of work, and folds every result into `sink`.  The `hash`
column is one canonical op's result; every lane must print the same one.
"""
import sys
import time

def mix(count, salt):
    acc = salt
    for i in range(count):
        acc = ((acc ^ i) + (i & 7)) & 16777215
    return acc

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
    count = 5_000_000
    us, sink = timed(n, lambda r: mix(count, r))
    row("sum_loop", n, us, count, mix(count, 0), sink)
