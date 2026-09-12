"""MkDocs / Material markdown: flatten admonitions and content tabs."""

from __future__ import annotations

import re
from pathlib import Path

_ADMON = re.compile(
    r'^(?P<indent>[ \t]*)(?:!!!|\?\?\?\+?|===)\s+(?P<kind>[\w-]+)?\s*(?:"(?P<title>[^"]*)")?\s*$'
)


def preprocess(text: str, src: Path) -> str:
    out: list[str] = []
    dedent_stack: list[int] = []
    for line in text.split("\n"):
        m = _ADMON.match(line)
        if m:
            indent = len(m.group("indent").expandtabs(4))
            title = m.group("title") or (m.group("kind") or "").capitalize()
            out.append(f"{' ' * indent}**{title}**" if title else "")
            dedent_stack.append(indent + 4)
            continue
        while dedent_stack:
            body_indent = dedent_stack[-1]
            if line.strip() == "" or len(line) - len(line.lstrip(" ")) >= body_indent:
                if line.strip():
                    line = (
                        line[body_indent:] if line.startswith(" " * body_indent) else line.lstrip()
                    )
                break
            dedent_stack.pop()
        out.append(line)
    return "\n".join(out)
