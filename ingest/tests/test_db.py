import json
import sqlite3
from pathlib import Path

import zstandard

from ingest.db import (
    AnnotationRow,
    FieldRow,
    KindRow,
    ProjectRow,
    Section,
    ShardWriter,
    merge_shards,
)
from ingest.manifest import attribution, write_release


def _shard(path: Path, slug: str, version: str, shared_text: str) -> None:
    w = ShardWriter(path)
    pid = w.add_project(
        ProjectRow(slug, slug.title(), "cncf", "https://x", "Apache-2.0", "CC-BY-4.0", "attr")
    )
    vid = w.add_version(pid, version, f"v{version}")
    w.add_kind(
        vid,
        KindRow("g.io", "v1", "Thing", "Namespaced", "desc", {"type": "object"}),
        [FieldRow("spec.a", "string", True, "field a"), FieldRow("spec.b", "integer", False, None)],
    )
    # duplicate GVK ignored
    assert w.add_kind(vid, KindRow("g.io", "v1", "Thing", None, None, {}), []) == 0
    w.add_sections(
        vid,
        [
            Section("https://x/a/", "A", "A > s", shared_text),
            Section("https://x/b/", "B", "B", f"unique {slug}"),
        ],
        "CC-BY-4.0",
    )
    w.add_annotations(pid, [AnnotationRow("x.io/k", ["Pod"], "desc", "https://x/#k")])
    w.finalize()
    w.close()


def test_shard_and_merge(tmp_path: Path) -> None:
    _shard(tmp_path / "a.sqlite", "aa", "1.0.0", "same text")
    _shard(tmp_path / "b.sqlite", "bb", "2.0.0", "same text")
    out = tmp_path / "index.sqlite"
    merge_shards(
        [tmp_path / "a.sqlite", tmp_path / "b.sqlite"],
        out,
        {"build_date": "2026-01-01T00:00:00Z", "ingest_sha": "abc"},
    )
    c = sqlite3.connect(out)
    assert c.execute("select count(*) from projects").fetchone()[0] == 2
    assert c.execute("select count(*) from doc_content").fetchone()[0] == 3  # shared deduped
    assert c.execute("select count(*) from docs").fetchone()[0] == 4
    assert c.execute("select count(*) from kinds").fetchone()[0] == 2
    assert c.execute("select count(*) from fields").fetchone()[0] == 4
    assert c.execute("select value from meta where key='ingest_sha'").fetchone()[0] == "abc"
    hits = c.execute(
        "select dc.title, p.slug from docs_fts join doc_content dc on dc.id=docs_fts.rowid"
        " join docs d on d.content_id=dc.id join versions v on v.id=d.version_id"
        " join projects p on p.id=v.project_id where docs_fts match 'unique' order by p.slug"
    ).fetchall()
    assert hits == [("B", "aa"), ("B", "bb")]
    assert (
        c.execute("select count(*) from fields_fts where fields_fts match '\"spec.a\"'").fetchone()[
            0
        ]
        == 2
    )
    assert c.execute(
        "select key from annotations_fts where annotations_fts match '\"x.io/k\"'"
    ).fetchall()
    blob = c.execute("select schema_zstd from kinds limit 1").fetchone()[0]
    assert json.loads(zstandard.ZstdDecompressor().decompress(blob)) == {"type": "object"}
    c.close()
    dist = tmp_path / "dist"
    mpath = write_release(out, dist)
    m = json.loads(mpath.read_text())
    assert m["schema_version"] == 1 and m["ingest_sha"] == "abc"
    assert m["build_date"] == "2026-01-01T00:00:00Z"
    assert {p["slug"] for p in m["projects"]} == {"aa", "bb"}
    assert (dist / "index.sqlite.zst").stat().st_size == m["index"]["size"]
    sums = (dist / "SHA256SUMS").read_text()
    assert m["index"]["sha256"] in sums and "manifest.json" in sums
    assert "**Aa**" in attribution(out)
