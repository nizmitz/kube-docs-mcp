"""Sphinx-flavoured reStructuredText → Sections via docutils with stub directives/roles."""

from __future__ import annotations

import io
from collections.abc import Iterator
from pathlib import Path
from typing import Any

from docutils import nodes
from docutils.core import publish_doctree
from docutils.parsers.rst import Directive, directives, roles

from ingest.db import Section
from ingest.docs.base import Block, blocks_to_sections, map_url
from ingest.models import UrlSpec


class _AnyOptions(dict[str, Any]):
    def __missing__(self, key: str) -> Any:
        return directives.unchanged


class _KeepContent(Directive):
    """Render directive body as ordinary nested content (note, tabs, only, …)."""

    has_content = True
    optional_arguments = 100
    final_argument_whitespace = True
    option_spec = _AnyOptions()

    def run(self) -> list[nodes.Node]:
        node = nodes.container()
        if self.content:
            self.state.nested_parse(self.content, self.content_offset, node)
        return list(node.children)


class _Literal(Directive):
    """code-block / parsed-literal: body becomes a literal block with language info."""

    has_content = True
    optional_arguments = 100
    final_argument_whitespace = True
    option_spec = _AnyOptions()

    def run(self) -> list[nodes.Node]:
        lang = self.arguments[0] if self.arguments else ""
        lit = nodes.literal_block("\n".join(self.content), "\n".join(self.content))
        lit["language"] = lang
        return [lit]


class _ListTable(Directive):
    """list-table: nested bullet lists → rows of ' | ' separated cells."""

    has_content = True
    optional_arguments = 100
    final_argument_whitespace = True
    option_spec = _AnyOptions()

    def run(self) -> list[nodes.Node]:
        container = nodes.container()
        self.state.nested_parse(self.content, self.content_offset, container)
        rows: list[str] = []
        for outer in container.findall(nodes.bullet_list):
            if outer.parent is not container:
                continue
            for item in outer.children:
                cells = [
                    " ".join(cell.astext().split())
                    for inner in item.children
                    if isinstance(inner, nodes.bullet_list)
                    for cell in inner.children
                ]
                rows.append(" | ".join(cells))
        text = "\n".join(rows)
        return [nodes.literal_block(text, text, language="table")]


class _Drop(Directive):
    has_content = True
    optional_arguments = 100
    final_argument_whitespace = True
    option_spec = _AnyOptions()

    def run(self) -> list[nodes.Node]:
        return []


KEEP = (
    "tabs", "tab", "group-tab", "code-tab", "note", "warning", "tip", "important", "caution",
    "attention", "danger", "hint", "versionadded", "versionchanged", "deprecated", "only",
    "seealso", "admonition", "glossary", "rubric", "envvar", "option", "confval", "cmdoption",
    "program", "describe", "topic", "sidebar", "container", "div", "list-table", "csv-table",
    "table", "collapse", "dropdown", "card", "grid", "grid-item", "grid-item-card", "tabbed",
    "toggle", "cssclass", "rst-class", "contents", "compound", "epigraph", "highlights",
)  # fmt: skip
LITERAL = ("code-block", "sourcecode", "code", "parsed-literal", "literalinclude")
DROP = (
    "toctree", "image", "figure", "include", "highlight", "mermaid", "graphviz", "raw",
    "meta", "index", "spelling", "video", "youtube", "target-notes", "footer", "header",
    "default-domain", "currentmodule", "module", "tabularcolumns", "sectionauthor", "codeauthor",
    "openapi", "swaggerv2doc", "api-reference", "restapi", "jinja", "cilium-version", "github-file",
)  # fmt: skip
ROLES = (
    "ref", "doc", "term", "file", "code", "option", "envvar", "program", "command", "kbd", "abbr",
    "math", "guilabel", "menuselection", "samp", "dfn", "download", "numref", "eq", "any",
    "py:class", "py:func", "py:mod", "class", "func", "mod", "meth", "attr", "obj", "data",
    "confval", "cmdoption", "spelling:ignore", "spelling:word", "git-tree", "github-project",
    "github-issue", "github-pull", "github-backport", "prev", "next", "gh-file", "kubectl",
)  # fmt: skip

_registered = False


def _text_role(
    name: str,
    rawtext: str,
    text: str,
    lineno: int,
    inliner: Any,
    options: Any = None,
    content: Any = None,
) -> tuple[list[nodes.Node], list[nodes.Node]]:
    # `Label <target>` → Label
    label = text.rsplit("<", 1)[0].strip() if text.endswith(">") and "<" in text else text
    return [nodes.Text(label)], []


def _register() -> None:
    global _registered
    if _registered:
        return
    for name in KEEP:
        directives.register_directive(name, _KeepContent)
    directives.register_directive("list-table", _ListTable)
    for name in LITERAL:
        directives.register_directive(name, _Literal)
    for name in DROP:
        directives.register_directive(name, _Drop)
    for name in ROLES:
        roles.register_local_role(name, _text_role)  # type: ignore[arg-type]
    _registered = True


def _node_text(node: nodes.Node) -> str:
    if isinstance(node, nodes.literal_block):
        lang = node.get("language", "")
        if lang == "table":
            return node.astext()
        return f"```{lang}\n{node.astext().rstrip()}\n```"
    if isinstance(node, nodes.bullet_list | nodes.enumerated_list):
        return "\n".join("- " + " ".join(li.astext().split()) for li in node.children)
    if isinstance(node, nodes.definition_list):
        items = []
        for item in node.children:
            kids = list(item.children) if isinstance(item, nodes.Element) else []
            term = " ".join(kids[0].astext().split()) if kids else ""
            body = " ".join(kids[-1].astext().split()) if len(kids) > 1 else ""
            items.append(f"{term}: {body}")
        return "\n".join(items)
    if isinstance(node, nodes.table):
        rows = []
        for row in node.findall(nodes.row):
            rows.append(" | ".join(" ".join(c.astext().split()) for c in row.children))
        return "\n".join(rows)
    if isinstance(node, nodes.field_list | nodes.option_list):
        return " ".join(node.astext().split())
    if isinstance(node, nodes.image | nodes.figure | nodes.comment | nodes.substitution_definition):
        return ""
    if isinstance(node, nodes.system_message | nodes.target | nodes.problematic):
        return node.astext() if isinstance(node, nodes.problematic) else ""
    return " ".join(node.astext().split())


def _walk(node: nodes.Element, level: int) -> Iterator[Block]:
    block = Block(level=level, heading="")
    if level > 0:
        title = next((c for c in node.children if isinstance(c, nodes.title)), None)
        block.heading = " ".join(title.astext().split()) if title else ""
    yield block
    for child in node.children:
        if isinstance(child, nodes.title):
            continue
        if isinstance(child, nodes.section):
            yield from _walk(child, level + 1)
            continue
        text = _node_text(child)
        if text:
            block.paragraphs.append(text)


def render_document(
    text: str, rel_path: str, url_spec: UrlSpec, version_ctx: dict[str, str] | None = None
) -> list[Section]:
    _register()
    doctree = publish_doctree(
        text,
        settings_overrides={
            "report_level": 5,
            "halt_level": 5,
            "warning_stream": io.StringIO(),
            "file_insertion_enabled": False,
            "raw_enabled": False,
        },
    )
    blocks = list(_walk(doctree, 0))
    title = ""
    # docutils promotes a lone top-level section title to document title
    if doctree.get("title"):
        title = str(doctree["title"])
    else:
        first = next((b for b in blocks if b.level >= 1 and b.heading), None)
        if first:
            title = first.heading
            first.level = 1
    if not title:
        title = Path(rel_path).stem.replace("-", " ").replace("_", " ")
    url = map_url(rel_path, url_spec, version_ctx)
    return blocks_to_sections(blocks, title, url)
