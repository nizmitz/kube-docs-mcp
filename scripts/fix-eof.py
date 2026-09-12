#!/usr/bin/env python3
"""Ensure every given file ends with exactly one newline. Used by .pre-commit-config.yaml."""
import sys

for f in sys.argv[1:]:
    with open(f, "rb") as fh:
        data = fh.read()
    if data and not data.endswith(b"\n"):
        with open(f, "ab") as fh:
            fh.write(b"\n")
