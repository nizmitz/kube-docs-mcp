"""Commented YAML reference files (e.g. falco.yaml, rules files) → sections per top-level key."""

from __future__ import annotations

import re
from pathlib import Path

from ingest.db import Section
from ingest.docs.base import Block, blocks_to_sections, map_url
from ingest.models import UrlSpec

_TOP_KEY = re.compile(r"^([A-Za-z_][\w.-]*):")
_RULE_NAME = re.compile(r"^- (?:rule|macro|list):\s*(.+?)\s*$")


def render_document(
    text: str, rel_path: str, url_spec: UrlSpec, version_ctx: dict[str, str] | None = None
) -> list[Section]:
    title = Path(rel_path).name
    blocks: list[Block] = [Block(level=0, heading="")]
    cur_lines: list[str] = []

    def flush() -> None:
        if cur_lines:
            blocks[-1].paragraphs.append("```yaml\n" + "\n".join(cur_lines).rstrip() + "\n```")
            cur_lines.clear()

    for line in text.split("\n"):
        m = _TOP_KEY.match(line) or _RULE_NAME.match(line)
        if m:
            flush()
            blocks.append(Block(level=2, heading=m.group(1)))
        cur_lines.append(line)
    flush()
    url = map_url(rel_path, url_spec, version_ctx)
    return blocks_to_sections(blocks, title, url)
