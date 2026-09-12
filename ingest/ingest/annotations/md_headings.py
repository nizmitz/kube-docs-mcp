"""Annotation entries from markdown headings that look like annotation keys."""

from __future__ import annotations

import re

from ingest.db import AnnotationRow

_LINK_ITEM = re.compile(r"^\s*[-*]\s+\[([^\]]+)\]\([^)]*\)\s*$")
_LINK = re.compile(r"\[([^\]]+)\]\([^)]*\)")
_HTML = re.compile(r"<[^>]+>")
MAX_DESC = 600


def _is_key(text: str) -> bool:
    t = text.strip().strip("`")
    return " " not in t and ("/" in t or "." in t)


def parse(
    text: str, doc_url: str, heading_level: int = 2, default_applies: list[str] | None = None
) -> list[AnnotationRow]:
    marker = "#" * heading_level
    h_re = re.compile(rf"^{marker}\s+(.+?)\s*(\{{#([^}}]+)\}})?\s*$")
    any_h = re.compile(r"^#{1,6}\s")
    rows: list[AnnotationRow] = []
    key: str | None = None
    anchor = ""
    applies: list[str] = []
    paras: list[str] = []
    in_fence = False

    def flush() -> None:
        if not key:
            return
        desc = " ".join(" ".join(paras).split())[:MAX_DESC]
        rows.append(
            AnnotationRow(
                key=key,
                applies_to=applies or list(default_applies or ["*"]),
                description=desc or f"annotation {key}",
                doc_url=f"{doc_url}#{anchor}" if anchor else doc_url,
                source="auto",
            )
        )

    for line in text.split("\n"):
        if line.startswith("```"):
            in_fence = not in_fence
            continue
        if in_fence:
            if key:
                paras.append(line)
            continue
        m = h_re.match(line)
        if m and _is_key(m.group(1)):
            flush()
            key = m.group(1).strip().strip("`")
            anchor = m.group(3) or re.sub(
                r"[^a-z0-9-]", "", key.lower().replace("/", "").replace(".", "")
            )
            applies, paras = [], []
            continue
        if any_h.match(line) and (m is None or not _is_key(m.group(1))):
            # a heading of same-or-higher level that is not a key ends the entry;
            # deeper headings are part of the description
            level = len(line) - len(line.lstrip("#"))
            if level <= heading_level:
                flush()
                key = None
            continue
        if not key:
            continue
        lm = _LINK_ITEM.match(line)
        if lm and not paras:
            applies.append(lm.group(1).strip("`"))
            continue
        clean = _HTML.sub("", _LINK.sub(r"\1", line)).strip()
        if clean:
            paras.append(clean)
    flush()
    return rows
