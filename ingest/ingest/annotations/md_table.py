"""Extract annotation rows from the first GFM table whose header has the `key` column."""

from __future__ import annotations

import re

from ingest.db import AnnotationRow
from ingest.models import Columns

_ROW = re.compile(r"^\s*\|(.+)\|\s*$")
_SEP = re.compile(r"^\s*\|?\s*:?-{2,}")
_LINK = re.compile(r"\[([^\]]+)\]\(#?([^)]*)\)")
_ANCHOR = re.compile(r'<a\s+(?:name|id)="([^"]+)"')
_HEADING = re.compile(r"^#{1,6}\s+(.+?)\s*(\{#([^}]+)\})?\s*$")
_HTML = re.compile(r"<[^>]+>")


def _cells(line: str) -> list[str]:
    m = _ROW.match(line)
    inner = m.group(1) if m else line.strip().strip("|")
    return [c.strip().strip("`") for c in re.split(r"(?<!\\)\|", inner)]


def _col(header: list[str], name: str | None) -> int | None:
    if name is None:
        return None
    low = [h.lower() for h in header]
    return low.index(name.lower()) if name.lower() in low else None


def _cell(c: list[str], i: int | None) -> str | None:
    if i is None or i >= len(c):
        return None
    v = _HTML.sub("", _LINK.sub(r"\1", c[i])).strip()
    return " ".join(v.split()) or None


def _slug(text: str) -> str:
    return re.sub(r"[^a-z0-9-]", "", text.lower().strip().replace(" ", "-").replace("/", ""))


def _anchor_paragraphs(lines: list[str]) -> dict[str, str]:
    """Map anchor/heading-slug → first paragraph after it."""
    out: dict[str, str] = {}
    pending: list[str] = []
    buf: list[str] = []
    in_fence = False

    def close() -> None:
        if pending and buf:
            text = " ".join(" ".join(buf).split())
            for a in pending:
                out.setdefault(a, text)
            pending.clear()
        buf.clear()

    for line in lines:
        if line.startswith("```"):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        anchors = _ANCHOR.findall(line)
        hm = _HEADING.match(line)
        if anchors or hm:
            close()
            if hm:
                pending.append(hm.group(3) or _slug(_LINK.sub(r"\1", hm.group(1))))
            pending.extend(anchors)
            rest = _HTML.sub("", line).strip() if anchors and not hm else ""
            if rest and not rest.startswith(("|", "#", "<")):
                buf.append(rest)
            continue
        if line.strip() == "" or line.lstrip().startswith("|"):
            if buf:
                close()
            continue
        if pending:
            buf.append(_HTML.sub("", _LINK.sub(r"\1", line)).strip())
    close()
    return out


def parse(
    text: str, columns: Columns, doc_url: str, default_applies: list[str]
) -> list[AnnotationRow]:
    rows: list[AnnotationRow] = []
    lines = text.split("\n")
    anchors: dict[str, str] | None = None
    i = 0
    while i < len(lines) - 1:
        if "|" not in lines[i] or not _SEP.match(lines[i + 1]):
            i += 1
            continue
        header = _cells(lines[i])
        ki = _col(header, columns.key)
        if ki is None:
            i += 2
            continue
        di, ai = _col(header, columns.description), _col(header, columns.applies_to)
        ti, dfi = _col(header, columns.type), _col(header, columns.default)
        vi, si = _col(header, columns.values), _col(header, columns.since)
        j = i + 2
        while j < len(lines) and "|" in lines[j]:
            c = _cells(lines[j])
            j += 1
            if len(c) <= ki or not c[ki]:
                continue
            raw_key = c[ki]
            lm = _LINK.match(raw_key)
            anchor = lm.group(2) if lm else None
            key = _LINK.sub(r"\1", raw_key).split()[0].strip("`*")
            if "/" not in key and "." not in key:
                continue
            desc = _cell(c, di)
            if not desc:
                if anchors is None:
                    anchors = _anchor_paragraphs(lines)
                desc = anchors.get(anchor or _slug(key)) or anchors.get(_slug(key))
            applies_raw = _cell(c, ai)
            applies = (
                [a.strip() for a in applies_raw.split(",") if a.strip()]
                if applies_raw
                else (default_applies or ["*"])
            )
            vt = _cell(c, ti)
            dflt = _cell(c, dfi)
            if dflt and dflt.upper() != "N/A":
                vt = f"{vt} (default: {dflt})" if vt else f"default: {dflt}"
            vals = _cell(c, vi)
            rows.append(
                AnnotationRow(
                    key=key,
                    applies_to=applies,
                    description=desc or f"annotation {key}",
                    doc_url=f"{doc_url}#{anchor}" if anchor else doc_url,
                    value_type=vt,
                    allowed_values=[v.strip() for v in vals.split(",")] if vals else None,
                    since=_cell(c, si),
                    source="auto",
                )
            )
        return rows  # only the first matching table
    return rows
