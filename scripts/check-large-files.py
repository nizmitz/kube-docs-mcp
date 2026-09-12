#!/usr/bin/env python3
"""Fail if any given file exceeds 1MiB. Used by .pre-commit-config.yaml."""
import os
import sys

LIMIT = 1024 * 1024
big = [f for f in sys.argv[1:] if os.path.isfile(f) and os.path.getsize(f) > LIMIT]
if big:
    print("large files (>1MiB):", ", ".join(big))
    sys.exit(1)
