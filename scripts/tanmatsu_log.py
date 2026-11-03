#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Keep the console of every connected Tanmatsu in a log file (TANMATSU.md § Reading an app's log).

The Tanmatsu's two chips each show up as an Espressif USB-Serial/JTAG port, and a port goes
away and comes back on every restart into an app.  This follows each port by its serial
number (the chip's MAC), re-opens it as soon as it is back, and appends every line to
`<dir>/<serial>.log` with the PC's time in front, so what an app printed is there to read
afterwards, however often the device restarted in between.

    scripts/tanmatsu_log.py                 # run in the foreground (Ctrl-C ends it)
    scripts/tanmatsu_log.py --install       # as a systemd user service, started now and at login
    scripts/tanmatsu_log.py --uninstall     # stop and remove the service
    scripts/tanmatsu_log.py --tail [serial] # print the last lines of a log

Needs read access to the ports (the `dialout` group).  ModemManager also opens such ports
and reads from them: TANMATSU.md gives the udev line that keeps it away.
"""

import argparse
import datetime
import errno
import glob
import os
import select
import subprocess
import sys
import time

BY_ID = "/dev/serial/by-id"
PATTERN = "usb-Espressif_USB_JTAG_serial_debug_unit_*-if00"
DEFAULT_DIR = os.path.join(os.path.expanduser("~"), ".cache", "loft", "tanmatsu")
SERVICE = "loft-tanmatsu-log.service"


def ports():
    """Each connected Espressif USB-Serial/JTAG port, by its serial number (the chip's MAC)."""
    out = {}
    for link in glob.glob(os.path.join(BY_ID, PATTERN)):
        serial = os.path.basename(link)[len("usb-Espressif_USB_JTAG_serial_debug_unit_"):-len("-if00")]
        out[serial.replace(":", "").lower()] = link
    return out


def open_raw(path):
    """The port opened for reading, raw: no echo, no line discipline, no hang-up on close."""
    import termios  # POSIX only; `--help` and `--tail` run anywhere
    fd = os.open(path, os.O_RDONLY | os.O_NOCTTY | os.O_NONBLOCK)
    attrs = termios.tcgetattr(fd)
    attrs[0] = 0                                  # iflag
    attrs[1] = 0                                  # oflag
    attrs[2] = termios.CS8 | termios.CREAD | termios.CLOCAL
    attrs[3] = 0                                  # lflag
    attrs[4] = attrs[5] = termios.B115200
    termios.tcsetattr(fd, termios.TCSANOW, attrs)
    return fd


def stamp():
    return datetime.datetime.now().strftime("%Y-%m-%d %H:%M:%S.%f")[:-3]


class Port:
    """One chip's port: its log file, and the part of a line read but not yet ended."""

    def __init__(self, serial, log_dir):
        self.serial = serial
        self.fd = None
        self.partial = b""
        self.log = open(os.path.join(log_dir, f"{serial}.log"), "a", encoding="utf-8")

    def note(self, text):
        self.log.write(f"{stamp()} --- {text}\n")
        self.log.flush()

    def write_lines(self, data):
        self.partial += data.replace(b"\r", b"").replace(b"\0", b"")
        *lines, self.partial = self.partial.split(b"\n")
        for line in lines:
            self.log.write(f"{stamp()} {line.decode('utf-8', 'replace')}\n")
        if lines:
            self.log.flush()

    def close(self, why):
        if self.fd is not None:
            try:
                os.close(self.fd)
            except OSError:
                pass
            self.fd = None
            if self.partial:
                self.write_lines(b"\n")
            self.note(f"port gone ({why})")


def run(log_dir):
    os.makedirs(log_dir, exist_ok=True)
    known = {}
    while True:
        present = ports()
        for serial, link in present.items():
            port = known.setdefault(serial, Port(serial, log_dir))
            if port.fd is None:
                try:
                    port.fd = open_raw(link)
                    port.note(f"port open: {os.path.realpath(link)}")
                except OSError as e:
                    if e.errno in (errno.EACCES, errno.EPERM):
                        sys.exit(f"tanmatsu_log: no permission to read {link}: join the dialout group")
        for serial, port in known.items():
            if port.fd is not None and serial not in present:
                port.close("disconnected")
        fds = {p.fd: p for p in known.values() if p.fd is not None}
        if not fds:
            time.sleep(0.05)
            continue
        ready, _, _ = select.select(list(fds), [], [], 0.05)
        for fd in ready:
            port = fds[fd]
            try:
                data = os.read(fd, 4096)
            except OSError as e:
                port.close(os.strerror(e.errno))
                continue
            if not data:
                port.close("end of file")
                continue
            port.write_lines(data)


def install(log_dir):
    """A systemd user service running this script with the dialout group, now and at every login."""
    unit_dir = os.path.join(os.path.expanduser("~"), ".config", "systemd", "user")
    os.makedirs(unit_dir, exist_ok=True)
    script = os.path.realpath(__file__)
    # `sg` gives the service the group even when the user manager started before it was granted.
    unit = (
        "[Unit]\nDescription=Keep the Tanmatsu's console in a log (loft scripts/tanmatsu_log.py)\n\n"
        "[Service]\n"
        f"ExecStart=/usr/bin/sg dialout -c \"/usr/bin/python3 -u {script} --dir {log_dir}\"\n"
        "Restart=always\nRestartSec=2\n\n[Install]\nWantedBy=default.target\n"
    )
    with open(os.path.join(unit_dir, SERVICE), "w", encoding="utf-8") as f:
        f.write(unit)
    subprocess.run(["systemctl", "--user", "daemon-reload"], check=True)
    subprocess.run(["systemctl", "--user", "enable", "--now", SERVICE], check=True)
    print(f"{SERVICE} running; logs in {log_dir}")


def uninstall():
    subprocess.run(["systemctl", "--user", "disable", "--now", SERVICE], check=False)
    path = os.path.join(os.path.expanduser("~"), ".config", "systemd", "user", SERVICE)
    if os.path.exists(path):
        os.remove(path)
    subprocess.run(["systemctl", "--user", "daemon-reload"], check=False)
    print(f"{SERVICE} removed")


def tail(log_dir, serial, lines):
    logs = sorted(glob.glob(os.path.join(log_dir, f"{serial or '*'}.log")))
    for path in logs:
        with open(path, encoding="utf-8", errors="replace") as f:
            content = f.readlines()
        print(f"== {path}")
        sys.stdout.writelines(content[-lines:])


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--dir", default=DEFAULT_DIR, help="where the logs go (default %(default)s)")
    ap.add_argument("--install", action="store_true", help="run as a systemd user service")
    ap.add_argument("--uninstall", action="store_true", help="stop and remove the service")
    ap.add_argument("--tail", nargs="?", const="", metavar="SERIAL", help="print the last lines of a log")
    ap.add_argument("-n", type=int, default=60, help="how many lines --tail prints")
    args = ap.parse_args()
    if args.install:
        install(args.dir)
    elif args.uninstall:
        uninstall()
    elif args.tail is not None:
        tail(args.dir, args.tail.replace(":", "").lower(), args.n)
    else:
        try:
            run(args.dir)
        except KeyboardInterrupt:
            pass


if __name__ == "__main__":
    main()
