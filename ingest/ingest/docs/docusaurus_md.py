"""Docusaurus / MDX markdown: strip imports, JSX tags, admonition fences."""

from __future__ import annotations

import re
from pathlib import Path

_IMPORT = re.compile(r"^(import|export)\s.*?(;|\n)$", re.MULTILINE)
_JSX_TAG = re.compile(r"^\s*</?[A-Z][\w.]*(\s[^>]*)?/?>\s*$", re.MULTILINE)
_INLINE_JSX = re.compile(r"</?[A-Z][\w.]*(\s[^>]*)?/?>")
_ADMON_OPEN = re.compile(r"^:::(\w+)\s*(.*)$", re.MULTILINE)
_ADMON_CLOSE = re.compile(r"^:::\s*$", re.MULTILINE)
_HTML_COMMENT = re.compile(r"<!--.*?-->", re.DOTALL)
_JSX_COMMENT = re.compile(r"\{/\*.*?\*/\}", re.DOTALL)


def preprocess(text: str, src: Path) -> str:
    text = _HTML_COMMENT.sub("", text)
    text = _JSX_COMMENT.sub("", text)
    text = _IMPORT.sub("", text)
    text = _JSX_TAG.sub("", text)
    text = _INLINE_JSX.sub("", text)
    text = _ADMON_OPEN.sub(lambda m: f"**{(m.group(2) or m.group(1)).strip().capitalize()}**", text)
    text = _ADMON_CLOSE.sub("", text)
    return text
