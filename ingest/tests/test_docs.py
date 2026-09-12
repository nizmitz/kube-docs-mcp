from pathlib import Path

from ingest.db import Section
from ingest.docs import DRIVERS, extensions
from ingest.docs.base import Block, blocks_to_sections, map_url
from ingest.models import UrlSpec

URL = UrlSpec(base="https://example.io/docs/")


def run(fmt: str, fixtures: Path, name: str, rel: str | None = None) -> list[Section]:
    f = fixtures / name
    return DRIVERS[fmt](f.read_text(), rel or name, URL, None, f)


def test_map_url() -> None:
    assert map_url("reference/foo.md", URL) == "https://example.io/docs/reference/foo/"
    assert map_url("reference/_index.md", URL) == "https://example.io/docs/reference/"
    assert (
        map_url(
            "a/index.md",
            UrlSpec(base="https://x/{major}.{minor}", index_file="index"),
            {"major": "1", "minor": "2"},
        )
        == "https://x/1.2/a/"
    )
    spec = UrlSpec(base="https://x/", strip_ext=False, trailing_slash=False)
    assert map_url("a/b.html", spec) == "https://x/a/b.html"


def test_blocks_split_long() -> None:
    blocks = [Block(1, "T", ["x" * 3000, "y" * 3000]), Block(2, "H2", ["short"])]
    secs = blocks_to_sections(blocks, "Title", "u", limit=4000)
    assert [s.section_path for s in secs] == ["Title > T", "Title > T (cont. 1)", "Title > T > H2"]
    assert all(len(s.text) <= 4000 for s in secs)


def test_hugo(fixtures: Path) -> None:
    secs = run("hugo-md", fixtures, "content/en/docs/reference/hugo.md", "reference/hugo.md")
    assert secs[0].title == "Hugo Page"
    assert secs[0].url == "https://example.io/docs/reference/hugo/"
    full = "\n".join(s.text for s in secs)
    assert "Pods inside" in full and "glossary_tooltip" not in full
    assert "Feature state: stable" in full
    assert "Note: " in full and "This is a note body." in full
    assert "link and `code`" not in full and "link" in full
    assert "```yaml\napiVersion: v1\nkind: Pod\n```" in full
    assert "name: simple" in full  # code_sample inlined
    assert "a | 1" in full and "- item one" in full
    assert "x.png" not in full
    assert "topologyKey | Key of node labels <x>" in full
    paths = [s.section_path for s in secs]
    assert "Hugo Page > Section One > Sub A" in paths and "Hugo Page > Section Two" in paths


def test_draft_skipped(fixtures: Path) -> None:
    assert run("plain-md", fixtures, "draft.md") == []


def test_mkdocs(fixtures: Path) -> None:
    secs = run("mkdocs-md", fixtures, "mkdocs.md")
    assert secs[0].title == "MkDocs Page"
    full = "\n".join(s.text for s in secs)
    assert "Important Admonition body line one. Line two." in full
    assert "Tab A Content in tab A." in full and "!!!" not in full and "===" not in full


def test_docusaurus(fixtures: Path) -> None:
    secs = run("docusaurus-md", fixtures, "docusaurus.md")
    full = "\n".join(s.text for s in secs)
    assert secs[0].title == "Docusaurus Page"
    assert "import" not in full and "<Tabs>" not in full and "TabItem" not in full
    assert "Tab A content." in full and "Be careful Warning body." in full
    assert "jsx comment" not in full and "Usage text" in full and "here." in full


def test_sphinx_rst(fixtures: Path) -> None:
    secs = run("sphinx-rst", fixtures, "sphinx.rst")
    assert secs[0].title == "My RST Page"
    full = "\n".join(s.text for s in secs)
    assert "a ref and Pod and literal" in full
    assert "A note body." in full
    assert "Install with helm." in full and "```shell-session\n$ helm install cilium\n```" in full
    assert "other/page" not in full  # toctree dropped
    assert "Key | Value" in full and "- bullet one" in full and "term: definition text" in full
    assert "Added feature." in full and "docs words" in full
    paths = [s.section_path for s in secs]
    assert "My RST Page > Section One > Sub A" in paths


def test_yaml_comments(fixtures: Path) -> None:
    secs = run("yaml-comments", fixtures, "rules.yaml")
    paths = [s.section_path for s in secs]
    assert (
        "rules.yaml > open_write" in paths and "rules.yaml > Terminal shell in container" in paths
    )
    assert extensions("yaml-comments") == (".yaml", ".yml") and extensions("hugo-md") == (".md",)
