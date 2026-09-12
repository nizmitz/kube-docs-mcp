"""build_project with fetch mocked to point at local fixture trees."""

import json
import sqlite3
from pathlib import Path

import pytest

from ingest import build
from ingest.models import Project, ResolvedVersion


@pytest.fixture
def fake_checkout(monkeypatch: pytest.MonkeyPatch, fixtures: Path, tmp_path: Path) -> Path:
    repo = tmp_path / "repo"
    (repo / "api").mkdir(parents=True)
    (repo / "api" / "mini.json").write_text((fixtures / "openapi_mini.json").read_text())
    (repo / "crds").mkdir()
    (repo / "crds" / "w.yaml").write_text((fixtures / "crd.yaml").read_text())
    import shutil

    shutil.copytree(fixtures / "content", repo / "content")
    (repo / "content" / "en" / "docs" / "reference" / "wk.md").write_text(
        (fixtures / "wellknown.md").read_text()
    )
    (repo / "content" / "en" / "docs" / "reference" / "skip.md").write_text(
        "---\ndraft: true\n---\n# x"
    )
    (repo / "content" / "en" / "docs" / "concepts").mkdir()
    (repo / "content" / "en" / "docs" / "concepts" / "no.md").write_text("# excluded\ntext")

    def checkout(
        repo_name: str, ref: str, v: ResolvedVersion, work: Path, paths: list[str]
    ) -> Path:
        return repo

    monkeypatch.setattr(build, "checkout", checkout)
    return repo


def test_build_project(fake_checkout: Path, tmp_path: Path, fixtures: Path) -> None:
    proj = Project.model_validate(
        {
            "slug": "demo",
            "name": "Demo",
            "category": "cncf",
            "homepage": "https://demo.io",
            "license": {"code": "Apache-2.0", "docs": "CC-BY-4.0"},
            "attribution": "Demo © authors",
            "source": {"repo": "demo/demo", "versions": {"strategy": "latest-minors"}},
            "schemas": [
                {"type": "openapi-v3", "paths": ["api/*.json"]},
                {"type": "crd-yaml", "paths": ["crds/*.yaml"]},
            ],
            "docs": [
                {
                    "format": "hugo-md",
                    "root": "content/{major}.{minor}/docs|content/en/docs",
                    "include": ["reference/**/*.md"],
                    "exclude": ["reference/skip.md"],
                    "url": {"base": "https://demo.io/docs/"},
                }
            ],
            "annotations": [
                {
                    "type": "k8s-wellknown",
                    "path": "content/en/docs/reference/wk.md",
                    "url": "https://demo.io/docs/reference/wk/",
                },
                {"type": "curated-yaml", "path": "../ingest/tests/fixtures/curated.yaml"},
            ],
        }
    )
    out = tmp_path / "demo.sqlite"
    stats = build.build_project(
        proj, tmp_path / "work", out, versions_override=["v1.2.3", "v1.1.0"]
    )
    assert stats.kinds == 2 * 4  # Pod, Node, Widget v1, Widget v1alpha1 per version
    assert stats.annotations == 5 + 2
    c = sqlite3.connect(out)
    assert c.execute("select count(*) from versions").fetchone()[0] == 2
    assert c.execute("select count(*) from doc_content").fetchone()[0] > 0
    # identical docs across two versions are deduplicated
    assert (
        c.execute("select count(*) from docs").fetchone()[0]
        == 2 * c.execute("select count(*) from doc_content").fetchone()[0]
    )
    urls = {r[0] for r in c.execute("select url from docs")}
    assert "https://demo.io/docs/reference/hugo/" in urls and not any("concepts" in u for u in urls)
    assert json.loads(
        c.execute("select value from meta where key='project:demo'").fetchone()[0]
    ) == [
        "v1.2.3",
        "v1.1.0",
    ]
    assert c.execute("select count(*) from annotations where source='curated'").fetchone()[0] == 2
