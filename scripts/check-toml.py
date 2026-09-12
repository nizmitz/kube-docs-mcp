#!/usr/bin/env python3
"""Parse every given file as TOML. Used by .pre-commit-config.yaml."""
import sys
import tomllib

for f in sys.argv[1:]:
    with open(f, "rb") as fh:
        tomllib.load(fh)
