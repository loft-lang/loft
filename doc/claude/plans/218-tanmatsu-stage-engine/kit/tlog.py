# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Follow every /dev/cu.usbmodem* port that appears, appending each line with the PC's time to
tanmatsu.log; reopen a port when it comes back (an app restart drops it).  Ends after an hour."""
import glob, time, serial, sys
log = open(sys.argv[1], "a", buffering=1)
open_ports, end = {}, time.time() + 3600
while time.time() < end:
    for p in glob.glob("/dev/cu.usbmodem*"):
        if p not in open_ports:
            try:
                open_ports[p] = serial.Serial(p, 115200, timeout=0)
                log.write(f"{time.strftime('%H:%M:%S')} [{p}] opened\n")
            except Exception:
                pass
    for p, s in list(open_ports.items()):
        try:
            data = s.read(4096)
            if data:
                for line in data.decode("utf-8", "replace").splitlines():
                    if line.strip():
                        log.write(f"{time.strftime('%H:%M:%S')} [{p[-5:]}] {line}\n")
        except Exception:
            log.write(f"{time.strftime('%H:%M:%S')} [{p}] closed\n")
            del open_ports[p]
    time.sleep(0.05)
