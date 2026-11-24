#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Unpack a release bundle where neither `unzip` nor 7-Zip is installed (a Linux image whose
# package mirror did not answer), keeping each file's mode as `unzip` does: the bundle's
# `bin/loft` stays executable.
#
#   python3 scripts/unzip_keep_modes.py <bundle.zip> <into-dir>
import os
import sys
import zipfile

with zipfile.ZipFile(sys.argv[1]) as z:
    for info in z.infolist():
        path = z.extract(info, sys.argv[2])
        mode = info.external_attr >> 16
        if mode:
            os.chmod(path, mode & 0o7777)
