#!/usr/bin/env python3
"""Fail if any given file contains a PEM private key marker. Used by .pre-commit-config.yaml."""
import sys

pattern = b"PRIVATE KEY-----"
bad = []
for f in sys.argv[1:]:
    with open(f, "rb") as fh:
        if pattern in fh.read():
            bad.append(f)
if bad:
    print("private key material in:", ", ".join(bad))
    sys.exit(1)
