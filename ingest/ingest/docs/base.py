"""Section model helpers: splitting, size capping, URL mapping."""

from __future__ import annotations

from dataclasses import dataclass, field

from ingest.db import Section
from ingest.models import UrlSpec

MAX_SECTION_CHARS = 4000


@dataclass(slots=True)
class Block:
    """A heading-scoped chunk of plain text produced by a driver."""

    level: int  # 1..6 ; 0 = preamble before first heading
    heading: str
    paragraphs: list[str] = field(default_factory=list)


def map_url(rel_path: str, spec: UrlSpec, version_ctx: dict[str, str] | None = None) -> str:
    """Map a file path relative to docs root to its public URL."""
    base = spec.base.format(**(version_ctx or {})) if version_ctx else spec.base
    if not base.endswith("/"):
        base += "/"
    p = rel_path.replace("\\", "/")
    if spec.strip_ext and "." in p.rsplit("/", 1)[-1]:
        p = p.rsplit(".", 1)[0]
    parts = p.split("/")
    if parts and parts[-1] in (spec.index_file, "index", "README"):
        parts = parts[:-1]
    path = "/".join(parts)
    url = base + path
    if spec.trailing_slash and not url.endswith("/"):
        url += "/"
    return url


def _split_long(text: str, limit: int) -> list[str]:
    if len(text) <= limit:
        return [text]
    out: list[str] = []
    cur: list[str] = []
    size = 0
    for para in text.split("\n\n"):
        if size + len(para) + 2 > limit and cur:
            out.append("\n\n".join(cur))
            cur, size = [], 0
        while len(para) > limit:  # single huge paragraph/code block: hard split
            out.append(para[:limit])
            para = para[limit:]
        cur.append(para)
        size += len(para) + 2
    if cur:
        out.append("\n\n".join(cur))
    return out


def blocks_to_sections(
    blocks: list[Block], title: str, url: str, limit: int = MAX_SECTION_CHARS
) -> list[Section]:
    """Turn heading-scoped blocks into Sections with `Title > H2 > H3` section paths."""
    sections: list[Section] = []
    stack: list[tuple[int, str]] = []
    for b in blocks:
        if b.level > 0:
            while stack and stack[-1][0] >= b.level:
                stack.pop()
            stack.append((b.level, b.heading))
        text = "\n\n".join(p for p in b.paragraphs if p.strip()).strip()
        if not text:
            continue
        path = " > ".join([title, *[h for _, h in stack]]) if stack else title
        chunks = _split_long(text, limit)
        for i, chunk in enumerate(chunks):
            sp = path if i == 0 else f"{path} (cont. {i})"
            sections.append(Section(url=url, title=title, section_path=sp, text=chunk))
    return sections
