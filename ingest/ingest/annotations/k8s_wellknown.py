"""Parse kubernetes.io 'Well-Known Labels, Annotations and Taints' page format."""

from __future__ import annotations

import re
from pathlib import Path

from ingest.db import AnnotationRow
from ingest.docs.hugo_md import preprocess as _hugo

_H3 = re.compile(r"^###\s+(.+?)\s*(\{#.*\})?\s*$")
_H2 = re.compile(r"^##\s")
_META = re.compile(r"^(Type|Example|Used on):\s*(.*)$")
_TYPE_MAP = {"label": "label", "annotation": "annotation", "taint": "taint"}
_SHORTCODE = re.compile(r"\{\{[<%].*?[%>]\}\}")
_LINK = re.compile(r"\[([^\]]+)\]\([^)]*\)")


def _applies(used_on: str) -> list[str]:
    s = used_on.strip().rstrip(".")
    if not s or s.lower().startswith("all objects"):
        return ["*"]
    # take leading comma/and separated Kind names before any parenthetical
    head = s.split("(")[0]
    parts = re.split(r",|\band\b|/", head)
    kinds = [p.strip().strip("`") for p in parts]
    kinds = [k for k in kinds if k and re.match(r"^[A-Z][A-Za-z]+$", k)]
    return kinds or [s]


def parse(text: str, doc_url: str) -> list[AnnotationRow]:
    rows: list[AnnotationRow] = []
    key: str | None = None
    meta: dict[str, str] = {}
    body: list[str] = []
    anchor = ""

    def flush() -> None:
        if not key:
            return
        t = _TYPE_MAP.get(meta.get("Type", "annotation").split()[0].lower(), "annotation")
        desc = _LINK.sub(r"\1", " ".join(" ".join(body).split()))
        example = meta.get("Example") or None
        if example:
            example = example.strip("`")
        rows.append(
            AnnotationRow(
                key=key,
                type=t,
                applies_to=_applies(meta.get("Used on", "")),
                description=desc or f"{t} {key}",
                doc_url=f"{doc_url}#{anchor}" if anchor else doc_url,
                example=example,
                source="auto",
            )
        )

    in_fence = False
    last_meta: str | None = None
    for line in _hugo(text, Path("_index.md")).split("\n"):
        if line.startswith("```"):
            in_fence = not in_fence
            body.append(line)
            continue
        if in_fence:
            body.append(line)
            continue
        m = _H3.match(line)
        if m:
            flush()
            raw_key = m.group(1).strip().strip("`")
            if " " in raw_key and "/" not in raw_key:
                key, meta, body = None, {}, []
                continue
            key = raw_key.split(" ")[0]
            last_meta = None
            anchor = (
                (m.group(2) or "").strip("{}#")
                if m.group(2)
                else key.replace("/", "-").replace(".", "").lower()
            )
            meta, body = {}, []
            continue
        if _H2.match(line):
            flush()
            key, meta, body = None, {}, []
            continue
        if key:
            mm = _META.match(line)
            if mm and mm.group(1) not in meta:
                meta[mm.group(1)] = _SHORTCODE.sub("", mm.group(2)).strip()
                last_meta = mm.group(1)
            elif last_meta and line.strip():
                # wrapped continuation of the previous metadata line
                meta[last_meta] += " " + _SHORTCODE.sub("", line).strip()
            else:
                last_meta = None
                body.append(_SHORTCODE.sub("", line))
    flush()
    return rows
