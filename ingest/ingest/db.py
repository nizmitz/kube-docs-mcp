"""SQLite index schema, shard writer, and merge.

The same DDL is used for per-project shards and the merged index. The Rust server
opens the merged file read-only. Keep this file in sync with docs/index-format.md.
"""

from __future__ import annotations

import hashlib
import json
import sqlite3
from collections.abc import Iterable
from dataclasses import dataclass, field
from datetime import UTC, datetime
from pathlib import Path

import zstandard

from ingest import SCHEMA_VERSION

DDL = """
CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS projects (
  id           INTEGER PRIMARY KEY,
  slug         TEXT NOT NULL UNIQUE,
  name         TEXT NOT NULL,
  category     TEXT NOT NULL,
  homepage     TEXT NOT NULL,
  code_license TEXT NOT NULL,
  docs_license TEXT NOT NULL,
  attribution  TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS versions (
  id         INTEGER PRIMARY KEY,
  project_id INTEGER NOT NULL REFERENCES projects(id),
  version    TEXT NOT NULL,
  git_tag    TEXT,
  pinned     INTEGER NOT NULL DEFAULT 1,
  source     TEXT NOT NULL DEFAULT 'own',
  indexed_at TEXT NOT NULL,
  stale      INTEGER NOT NULL DEFAULT 0,
  UNIQUE(project_id, version)
);
CREATE TABLE IF NOT EXISTS kinds (
  id          INTEGER PRIMARY KEY,
  version_id  INTEGER NOT NULL REFERENCES versions(id),
  api_group   TEXT NOT NULL,
  api_version TEXT NOT NULL,
  kind        TEXT NOT NULL,
  scope       TEXT,
  description TEXT,
  schema_zstd BLOB NOT NULL,
  UNIQUE(version_id, api_group, api_version, kind)
);
CREATE INDEX IF NOT EXISTS kinds_kind ON kinds(kind);
CREATE INDEX IF NOT EXISTS kinds_group_kind ON kinds(api_group, kind);
CREATE TABLE IF NOT EXISTS fields (
  id          INTEGER PRIMARY KEY,
  kind_id     INTEGER NOT NULL REFERENCES kinds(id),
  kind        TEXT NOT NULL,
  path        TEXT NOT NULL,
  type        TEXT NOT NULL,
  required    INTEGER NOT NULL DEFAULT 0,
  description TEXT,
  enum_json   TEXT
);
CREATE INDEX IF NOT EXISTS fields_kind_id ON fields(kind_id);
CREATE VIRTUAL TABLE IF NOT EXISTS fields_fts USING fts5(
  kind, path, description,
  content='fields', content_rowid='id', tokenize='porter unicode61'
);
CREATE TABLE IF NOT EXISTS doc_content (
  id           INTEGER PRIMARY KEY,
  hash         TEXT NOT NULL UNIQUE,
  title        TEXT NOT NULL,
  section_path TEXT NOT NULL,
  text         TEXT NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS docs_fts USING fts5(
  title, section_path, text,
  content='doc_content', content_rowid='id', tokenize='porter unicode61'
);
CREATE TABLE IF NOT EXISTS docs (
  id         INTEGER PRIMARY KEY,
  version_id INTEGER NOT NULL REFERENCES versions(id),
  content_id INTEGER NOT NULL REFERENCES doc_content(id),
  url        TEXT NOT NULL,
  license    TEXT NOT NULL,
  ord        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS docs_content ON docs(content_id);
CREATE INDEX IF NOT EXISTS docs_version ON docs(version_id);
CREATE TABLE IF NOT EXISTS annotations (
  id                  INTEGER PRIMARY KEY,
  project_id          INTEGER NOT NULL REFERENCES projects(id),
  key                 TEXT NOT NULL,
  type                TEXT NOT NULL DEFAULT 'annotation',
  applies_to_json     TEXT NOT NULL,
  value_type          TEXT,
  allowed_values_json TEXT,
  example             TEXT,
  description         TEXT NOT NULL,
  doc_url             TEXT NOT NULL,
  since               TEXT,
  deprecated          TEXT,
  source              TEXT NOT NULL DEFAULT 'auto',
  UNIQUE(project_id, key)
);
CREATE INDEX IF NOT EXISTS annotations_key ON annotations(key);
CREATE VIRTUAL TABLE IF NOT EXISTS annotations_fts USING fts5(
  key, description,
  content='annotations', content_rowid='id', tokenize='porter unicode61'
);
"""

FTS_TABLES = ("fields_fts", "docs_fts", "annotations_fts")


def content_hash(title: str, section_path: str, text: str) -> str:
    h = hashlib.sha256()
    for part in (title, section_path, text):
        h.update(part.encode("utf-8"))
        h.update(b"\x00")
    return h.hexdigest()


@dataclass(slots=True)
class ProjectRow:
    slug: str
    name: str
    category: str
    homepage: str
    code_license: str
    docs_license: str
    attribution: str


@dataclass(slots=True)
class KindRow:
    api_group: str
    api_version: str
    kind: str
    scope: str | None
    description: str | None
    schema: dict[str, object]


@dataclass(slots=True)
class FieldRow:
    path: str
    type: str
    required: bool
    description: str | None
    enum: list[str] | None = None


@dataclass(slots=True)
class Section:
    url: str
    title: str
    section_path: str
    text: str
    license: str = ""


@dataclass(slots=True)
class AnnotationRow:
    key: str
    applies_to: list[str]
    description: str
    doc_url: str
    type: str = "annotation"
    value_type: str | None = None
    allowed_values: list[str] | None = None
    example: str | None = None
    since: str | None = None
    deprecated: str | None = None
    source: str = "auto"


@dataclass(slots=True)
class BuildStats:
    kinds: int = 0
    fields: int = 0
    docs: int = 0
    annotations: int = 0
    extra: dict[str, int] = field(default_factory=dict)


def open_db(path: Path) -> sqlite3.Connection:
    conn = sqlite3.connect(path)
    conn.execute("PRAGMA journal_mode=OFF")
    conn.execute("PRAGMA synchronous=OFF")
    conn.execute("PRAGMA temp_store=MEMORY")
    conn.executescript(DDL)
    return conn


class ShardWriter:
    """Writes one project (all its versions) into a fresh sqlite file."""

    def __init__(self, path: Path) -> None:
        if path.exists():
            path.unlink()
        self.path = path
        self.conn = open_db(path)
        self._zstd = zstandard.ZstdCompressor(level=19)
        self.stats = BuildStats()

    def close(self) -> None:
        self.conn.commit()
        self.conn.close()

    def set_meta(self, key: str, value: str) -> None:
        self.conn.execute("INSERT OR REPLACE INTO meta(key, value) VALUES (?, ?)", (key, value))

    def add_project(self, p: ProjectRow) -> int:
        cur = self.conn.execute(
            "INSERT INTO projects(slug, name, category, homepage, code_license, docs_license,"
            " attribution) VALUES (?,?,?,?,?,?,?)",
            (p.slug, p.name, p.category, p.homepage, p.code_license, p.docs_license, p.attribution),
        )
        return int(cur.lastrowid or 0)

    def add_version(
        self,
        project_id: int,
        version: str,
        git_tag: str | None,
        *,
        pinned: bool = True,
        source: str = "own",
    ) -> int:
        cur = self.conn.execute(
            "INSERT INTO versions(project_id, version, git_tag, pinned, source, indexed_at)"
            " VALUES (?,?,?,?,?,?)",
            (
                project_id,
                version,
                git_tag,
                int(pinned),
                source,
                datetime.now(UTC).isoformat(timespec="seconds"),
            ),
        )
        return int(cur.lastrowid or 0)

    def add_kind(self, version_id: int, k: KindRow, fields: Iterable[FieldRow]) -> int:
        blob = self._zstd.compress(
            json.dumps(k.schema, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
        )
        cur = self.conn.execute(
            "INSERT OR IGNORE INTO kinds(version_id, api_group, api_version, kind, scope,"
            " description, schema_zstd) VALUES (?,?,?,?,?,?,?)",
            (version_id, k.api_group, k.api_version, k.kind, k.scope, k.description, blob),
        )
        if cur.rowcount == 0:
            return 0
        kind_id = int(cur.lastrowid or 0)
        self.stats.kinds += 1
        rows = [
            (
                kind_id,
                k.kind,
                f.path,
                f.type,
                int(f.required),
                f.description,
                json.dumps(f.enum) if f.enum else None,
            )
            for f in fields
        ]
        self.conn.executemany(
            "INSERT INTO fields(kind_id, kind, path, type, required, description, enum_json)"
            " VALUES (?,?,?,?,?,?,?)",
            rows,
        )
        self.stats.fields += len(rows)
        return kind_id

    def add_sections(
        self, version_id: int, sections: Iterable[Section], default_license: str
    ) -> int:
        n = 0
        for ord_, s in enumerate(sections):
            h = content_hash(s.title, s.section_path, s.text)
            row = self.conn.execute("SELECT id FROM doc_content WHERE hash=?", (h,)).fetchone()
            if row:
                content_id = int(row[0])
            else:
                cur = self.conn.execute(
                    "INSERT INTO doc_content(hash, title, section_path, text) VALUES (?,?,?,?)",
                    (h, s.title, s.section_path, s.text),
                )
                content_id = int(cur.lastrowid or 0)
            self.conn.execute(
                "INSERT INTO docs(version_id, content_id, url, license, ord) VALUES (?,?,?,?,?)",
                (version_id, content_id, s.url, s.license or default_license, ord_),
            )
            n += 1
        self.stats.docs += n
        return n

    def add_annotations(self, project_id: int, rows: Iterable[AnnotationRow]) -> int:
        n = 0
        for a in rows:
            cur = self.conn.execute(
                "INSERT OR IGNORE INTO annotations(project_id, key, type, applies_to_json,"
                " value_type, allowed_values_json, example, description, doc_url, since,"
                " deprecated, source) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
                (
                    project_id,
                    a.key,
                    a.type,
                    json.dumps(a.applies_to),
                    a.value_type,
                    json.dumps(a.allowed_values) if a.allowed_values else None,
                    a.example,
                    a.description,
                    a.doc_url,
                    a.since,
                    a.deprecated,
                    a.source,
                ),
            )
            n += cur.rowcount
        self.stats.annotations += n
        return n

    def finalize(self) -> None:
        self.set_meta("schema_version", str(SCHEMA_VERSION))
        rebuild_fts(self.conn)
        self.conn.commit()
        self.conn.execute("VACUUM")
        self.conn.commit()


def rebuild_fts(conn: sqlite3.Connection) -> None:
    for t in FTS_TABLES:
        conn.execute(f"INSERT INTO {t}({t}) VALUES('rebuild')")
        conn.execute(f"INSERT INTO {t}({t}) VALUES('optimize')")


def merge_shards(shards: list[Path], out: Path, meta: dict[str, str]) -> None:
    """Union all shards into a single index, remapping ids and deduplicating content."""
    if out.exists():
        out.unlink()
    dst = open_db(out)
    dst.execute("PRAGMA cache_size=-262144")
    content_ids: dict[str, int] = {}
    for shard in shards:
        src = sqlite3.connect(f"file:{shard}?mode=ro", uri=True)
        src.row_factory = sqlite3.Row
        pmap: dict[int, int] = {}
        for p in src.execute("SELECT * FROM projects"):
            cur = dst.execute(
                "INSERT OR IGNORE INTO projects(slug, name, category, homepage, code_license,"
                " docs_license, attribution) VALUES (?,?,?,?,?,?,?)",
                (
                    p["slug"],
                    p["name"],
                    p["category"],
                    p["homepage"],
                    p["code_license"],
                    p["docs_license"],
                    p["attribution"],
                ),
            )
            if cur.rowcount == 0:
                row = dst.execute("SELECT id FROM projects WHERE slug=?", (p["slug"],)).fetchone()
                pmap[p["id"]] = int(row[0])
            else:
                pmap[p["id"]] = int(cur.lastrowid or 0)
        vmap: dict[int, int] = {}
        for v in src.execute("SELECT * FROM versions"):
            cur = dst.execute(
                "INSERT INTO versions(project_id, version, git_tag, pinned, source, indexed_at,"
                " stale) VALUES (?,?,?,?,?,?,?)",
                (
                    pmap[v["project_id"]],
                    v["version"],
                    v["git_tag"],
                    v["pinned"],
                    v["source"],
                    v["indexed_at"],
                    v["stale"],
                ),
            )
            vmap[v["id"]] = int(cur.lastrowid or 0)
        kmap: dict[int, int] = {}
        for k in src.execute("SELECT * FROM kinds"):
            cur = dst.execute(
                "INSERT INTO kinds(version_id, api_group, api_version, kind, scope, description,"
                " schema_zstd) VALUES (?,?,?,?,?,?,?)",
                (
                    vmap[k["version_id"]],
                    k["api_group"],
                    k["api_version"],
                    k["kind"],
                    k["scope"],
                    k["description"],
                    k["schema_zstd"],
                ),
            )
            kmap[k["id"]] = int(cur.lastrowid or 0)
        dst.executemany(
            "INSERT INTO fields(kind_id, kind, path, type, required, description, enum_json)"
            " VALUES (?,?,?,?,?,?,?)",
            (
                (
                    kmap[f["kind_id"]],
                    f["kind"],
                    f["path"],
                    f["type"],
                    f["required"],
                    f["description"],
                    f["enum_json"],
                )
                for f in src.execute("SELECT * FROM fields")
            ),
        )
        cmap: dict[int, int] = {}
        for c in src.execute("SELECT * FROM doc_content"):
            cid = content_ids.get(c["hash"])
            if cid is None:
                cur = dst.execute(
                    "INSERT INTO doc_content(hash, title, section_path, text) VALUES (?,?,?,?)",
                    (c["hash"], c["title"], c["section_path"], c["text"]),
                )
                cid = int(cur.lastrowid or 0)
                content_ids[c["hash"]] = cid
            cmap[c["id"]] = cid
        dst.executemany(
            "INSERT INTO docs(version_id, content_id, url, license, ord) VALUES (?,?,?,?,?)",
            (
                (vmap[d["version_id"]], cmap[d["content_id"]], d["url"], d["license"], d["ord"])
                for d in src.execute("SELECT * FROM docs")
            ),
        )
        dst.executemany(
            "INSERT OR IGNORE INTO annotations(project_id, key, type, applies_to_json, value_type,"
            " allowed_values_json, example, description, doc_url, since, deprecated, source)"
            " VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
            (
                (
                    pmap[a["project_id"]],
                    a["key"],
                    a["type"],
                    a["applies_to_json"],
                    a["value_type"],
                    a["allowed_values_json"],
                    a["example"],
                    a["description"],
                    a["doc_url"],
                    a["since"],
                    a["deprecated"],
                    a["source"],
                )
                for a in src.execute("SELECT * FROM annotations")
            ),
        )
        src.close()
        dst.commit()
    for k, v in meta.items():
        dst.execute("INSERT OR REPLACE INTO meta(key, value) VALUES (?, ?)", (k, v))
    dst.execute(
        "INSERT OR REPLACE INTO meta(key, value) VALUES ('schema_version', ?)",
        (str(SCHEMA_VERSION),),
    )
    rebuild_fts(dst)
    dst.commit()
    dst.execute("VACUUM")
    dst.commit()
    dst.close()
