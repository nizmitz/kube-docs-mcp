"""Plain markdown: no preprocessing."""

from __future__ import annotations

from pathlib import Path


def preprocess(text: str, src: Path) -> str:
    return text
