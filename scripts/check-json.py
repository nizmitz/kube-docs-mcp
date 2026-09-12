#!/usr/bin/env python3
"""Parse every given file as JSON. Used by .pre-commit-config.yaml."""
import json
import sys

for f in sys.argv[1:]:
    with open(f, "rb") as fh:
        json.load(fh)
