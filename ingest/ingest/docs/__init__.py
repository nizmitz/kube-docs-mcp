"""Docs drivers keyed by registry `format`."""

from __future__ import annotations

from collections.abc import Callable
from pathlib import Path

from ingest.db import Section
from ingest.docs import (
    docusaurus_md,
    hugo_md,
    markdown,
    mkdocs_md,
    plain_md,
    sphinx_rst,
    yaml_comments,
)
from ingest.models import UrlSpec

Driver = Callable[[str, str, UrlSpec, dict[str, str] | None, Path | None], list[Section]]


def _md(pre: Callable[[str, Path], str]) -> Driver:
    def run(
        text: str,
        rel: str,
        url: UrlSpec,
        ctx: dict[str, str] | None,
        src: Path | None,
    ) -> list[Section]:
        return markdown.render_document(text, rel, url, ctx, pre, src)

    return run


def _rst(
    text: str, rel: str, url: UrlSpec, ctx: dict[str, str] | None, src: Path | None
) -> list[Section]:
    return sphinx_rst.render_document(text, rel, url, ctx)


def _yaml(
    text: str, rel: str, url: UrlSpec, ctx: dict[str, str] | None, src: Path | None
) -> list[Section]:
    return yaml_comments.render_document(text, rel, url, ctx)


DRIVERS: dict[str, Driver] = {
    "hugo-md": _md(hugo_md.preprocess),
    "mkdocs-md": _md(mkdocs_md.preprocess),
    "docusaurus-md": _md(docusaurus_md.preprocess),
    "mdx": _md(docusaurus_md.preprocess),
    "plain-md": _md(plain_md.preprocess),
    "sphinx-rst": _rst,
    "yaml-comments": _yaml,
}

EXTENSIONS: dict[str, tuple[str, ...]] = {
    "sphinx-rst": (".rst",),
    "mdx": (".mdx", ".md"),
    "yaml-comments": (".yaml", ".yml"),
}


def extensions(fmt: str) -> tuple[str, ...]:
    return EXTENSIONS.get(fmt, (".md",))
