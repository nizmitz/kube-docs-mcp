"""Markdown → Blocks via markdown-it-py. Base for hugo/mkdocs/docusaurus/plain drivers."""

from __future__ import annotations

import re
from collections.abc import Callable
from pathlib import Path
from typing import Any

import yaml
from markdown_it import MarkdownIt
from markdown_it.token import Token
from mdit_py_plugins.front_matter import front_matter_plugin

from ingest.db import Section
from ingest.docs.base import Block, blocks_to_sections, map_url
from ingest.models import UrlSpec

Preprocessor = Callable[[str, Path], str]

_md = MarkdownIt("commonmark").enable("table").use(front_matter_plugin)
_TAG = re.compile(r"<[^>]+>")
_BLOCK_TAG = re.compile(r"</?(p|div|tr|br|li|h[1-6]|table|ul|ol|pre)\b[^>]*>", re.IGNORECASE)
_CELL_OPEN = re.compile(r"<(td|th)\b[^>]*>", re.IGNORECASE)
_CELL_CLOSE = re.compile(r"</(td|th)>", re.IGNORECASE)
_HTML_COMMENT = re.compile(r"<!--.*?-->", re.DOTALL)


def html_to_text(html: str) -> str:
    """Lossy HTML → text keeping row/cell structure (used for generated reference tables)."""
    html = _HTML_COMMENT.sub("", html)
    html = _CELL_CLOSE.sub("", html)
    html = _CELL_OPEN.sub(" | ", html)
    html = _BLOCK_TAG.sub("\n", html)
    text = _TAG.sub("", html)
    text = (
        text.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&").replace("&quot;", '"')
    )
    lines = [" ".join(ln.split()).strip(" |") for ln in text.split("\n")]
    return "\n".join(ln for ln in lines if ln)


def parse_frontmatter(text: str) -> dict[str, Any]:
    if not text.startswith("---"):
        return {}
    end = text.find("\n---", 3)
    if end == -1:
        return {}
    try:
        data = yaml.safe_load(text[3:end])
    except yaml.YAMLError:
        return {}
    return data if isinstance(data, dict) else {}


def inline_text(tok: Token) -> str:
    out: list[str] = []
    for c in tok.children or []:
        if c.type in ("text", "code_inline"):
            out.append(c.content)
        elif c.type in ("softbreak", "hardbreak"):
            out.append(" ")
        elif c.type in ("html_inline", "image"):
            continue
        elif c.children:
            out.append(inline_text(c))
    return "".join(out)


def tokens_to_blocks(tokens: list[Token]) -> list[Block]:
    blocks: list[Block] = [Block(level=0, heading="")]
    list_depth = 0
    row: list[str] = []
    i = 0
    while i < len(tokens):
        t = tokens[i]
        cur = blocks[-1]
        if t.type == "heading_open":
            level = int(t.tag[1])
            text = _ANCHOR_RE.sub("", inline_text(tokens[i + 1])).strip()
            blocks.append(Block(level=level, heading=text))
            i += 3
            continue
        if t.type in ("fence", "code_block"):
            fence = f"```{t.info.strip()}\n{t.content.rstrip()}\n```"
            cur.paragraphs.append(fence)
        elif t.type == "paragraph_open":
            text = inline_text(tokens[i + 1]).strip()
            if text:
                prefix = ("  " * (list_depth - 1) + "- ") if list_depth else ""
                cur.paragraphs.append(prefix + text)
            i += 3
            continue
        elif t.type in ("bullet_list_open", "ordered_list_open"):
            list_depth += 1
        elif t.type in ("bullet_list_close", "ordered_list_close"):
            list_depth = max(0, list_depth - 1)
        elif t.type == "tr_open":
            row = []
        elif t.type in ("th_open", "td_open"):
            row.append(inline_text(tokens[i + 1]).strip())
            i += 3
            continue
        elif t.type == "tr_close":
            cur.paragraphs.append(" | ".join(row))
        elif t.type == "html_block":
            text = html_to_text(t.content)
            if text:
                cur.paragraphs.append(text)
        i += 1
    # merge consecutive table rows into one paragraph
    for b in blocks:
        merged: list[str] = []
        for p in b.paragraphs:
            if " | " in p and merged and " | " in merged[-1] and "\n" not in merged[-1][-1:]:
                merged[-1] = merged[-1] + "\n" + p
            else:
                merged.append(p)
        b.paragraphs = merged
    return blocks


_H1_RE = re.compile(r"^#\s+(.+?)\s*$", re.MULTILINE)
_ANCHOR_RE = re.compile(r"\s*\{#[^}]*\}\s*$")


def render_document(
    text: str,
    rel_path: str,
    url_spec: UrlSpec,
    version_ctx: dict[str, str] | None = None,
    preprocess: Preprocessor | None = None,
    src_path: Path | None = None,
) -> list[Section]:
    fm = parse_frontmatter(text)
    if fm.get("draft") is True:
        return []
    body = text
    if preprocess:
        body = preprocess(body, src_path or Path(rel_path))
    tokens = _md.parse(body)
    title = str(fm.get("title") or fm.get("linkTitle") or "").strip()
    if not title:
        m = _H1_RE.search(body)
        title = (
            _ANCHOR_RE.sub("", m.group(1)).strip()
            if m
            else Path(rel_path).stem.replace("-", " ").replace("_", " ")
        )
    blocks = tokens_to_blocks(tokens)
    url = map_url(rel_path, url_spec, version_ctx)
    return blocks_to_sections(blocks, title, url)
