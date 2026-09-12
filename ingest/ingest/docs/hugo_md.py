"""Hugo markdown: strip shortcodes, inline code samples, keep content."""

from __future__ import annotations

import re
from pathlib import Path

# {{< name args >}} ... {{< /name >}}  and  {{% name %}} ... {{% /name %}}
_OPEN = re.compile(r"\{\{[<%]\s*/?\s*([\w-]+)\b([^%>]*?)\s*/?[%>]\}\}")
_ATTR = re.compile(r'(\w+)\s*=\s*"([^"]*)"')
_INLINE_TEXT = {"glossary_tooltip", "glossary_definition", "param", "tooltip", "link"}
_DROP_WHOLE = {"toc", "feature-state", "figure", "youtube", "mermaid", "table", "tabs", "tab"}


def _find_examples_root(src: Path) -> Path | None:
    for parent in src.parents:
        cand = parent / "examples"
        if (parent.name in ("en", "content") or parent.parent.name == "content") and cand.is_dir():
            return cand
        if parent.name == "content":
            en = parent / "en" / "examples"
            return en if en.is_dir() else None
    return None


def preprocess(text: str, src: Path) -> str:
    examples = _find_examples_root(src)

    def repl(m: re.Match[str]) -> str:
        name = m.group(1)
        attrs = dict(_ATTR.findall(m.group(2) or ""))
        raw = m.group(0)
        if name in ("code_sample", "codenew", "code") and examples and attrs.get("file"):
            f = examples / attrs["file"]
            if f.is_file():
                lang = attrs.get("language") or f.suffix.lstrip(".")
                return f"\n```{lang}\n{f.read_text(errors='replace').rstrip()}\n```\n"
            return ""
        if name in _INLINE_TEXT:
            return str(attrs.get("text") or attrs.get("term_id", "").replace("-", " "))
        if name == "feature-state":
            state, since = attrs.get("state", ""), attrs.get("for_k8s_version", "?")
            return f"Feature state: {state} (since {since})"
        if name in _DROP_WHOLE or raw.startswith(("{{< /", "{{% /", "{{</", "{{%/")):
            return ""
        # note/warning/caution/tab wrappers → keep inner content, mark type
        return f"\n{name.capitalize()}: " if name in ("note", "warning", "caution") else ""

    return _OPEN.sub(repl, text)
