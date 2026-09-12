import json
from pathlib import Path

from ingest.fetch import sparse_dir
from ingest.models import ResolvedVersion, Versions
from ingest.versions import parse_tag, resolve, select

TAGS = ["v1.37.0", "v1.36.4", "v1.36.3", "v1.38.0-alpha.1", "v1.35.9", "v1.37.0-rc.0", "junk"]


def test_select_latest_minors() -> None:
    vs = select(
        TAGS, Versions(strategy="latest-minors", count=2, tag_pattern=r"^v(\d+)\.(\d+)\.(\d+)$")
    )
    assert [v.tag for v in vs] == ["v1.37.0", "v1.36.4"]
    assert vs[0].version == "1.37.0" and vs[0].render("release-{major}.{minor}") == "release-1.37"


def test_select_latest_releases_and_fixed() -> None:
    vs = select(
        TAGS, Versions(strategy="latest-releases", count=3, tag_pattern=r"^v(\d+)\.(\d+)\.(\d+)$")
    )
    assert [v.tag for v in vs] == ["v1.37.0", "v1.36.4", "v1.36.3"]
    fx = select([], Versions(strategy="fixed", tags=["v9.9.9", "main"]))
    assert fx[0].major == 9 and fx[1].tag == "main" and fx[1].major == 0
    assert parse_tag("nope", r"^v(\d+)\.(\d+)\.(\d+)$") is None
    assert parse_tag("0.44.1", r"^(\d+)\.(\d+)\.(\d+)$") == ResolvedVersion(
        version="0.44.1", tag="0.44.1", major=0, minor=44, patch=1
    )


def test_resolve_uses_cache(tmp_path: Path) -> None:
    cache = tmp_path / "cache" / "tags-o__r.json"
    cache.parent.mkdir(parents=True)
    cache.write_text(json.dumps(TAGS))
    vs = resolve("o/r", Versions(strategy="latest-minors", count=1), tmp_path)
    assert [v.tag for v in vs] == ["v1.37.0"]


def test_sparse_dir() -> None:
    assert sparse_dir("api/openapi-spec/v3/*.json") == "/api/openapi-spec/v3/"
    assert sparse_dir("pkg/**/*.yaml") == "/pkg/"
    assert sparse_dir("deploy/crds/file.yaml") == "/deploy/crds/file.yaml"
    assert sparse_dir("*.yaml") == "/*"
