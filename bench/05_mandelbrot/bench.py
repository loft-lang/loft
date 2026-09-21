"""Benchmark 5: Mandelbrot set (200x200, at most 256 iterations).

The timed loop runs `--n` ops, each on an input that differs by the repetition number in
VALUE, never in the amount of work, and folds every result into `sink`.  The `hash`
column is one canonical op's result; every lane must print the same one.
"""
import sys
import time

def mandelbrot(cx, cy):
    zx = zy = 0.0
    for i in range(256):
        if zx * zx + zy * zy > 4.0:
            return i
        zx, zy = zx * zx - zy * zy + cx, 2.0 * zx * zy + cy
    return 256


def grid(size, nudge):
    total = 0
    for y in range(size):
        for x in range(size):
            cx = (x / size) * 3.5 - 2.5 + nudge
            cy = (y / size) * 2.0 - 1.0
            total += mandelbrot(cx, cy)
    return total

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
    size = 200
    us, sink = timed(n, lambda r: grid(size, (r & 1) * 0.0000001))
    row("mandelbrot", n, us, size * size, grid(size, 0.0), sink)
