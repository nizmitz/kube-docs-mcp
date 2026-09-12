"""Release artifacts: index.sqlite.zst, manifest.json, SHA256SUMS, ATTRIBUTION.md."""

from __future__ import annotations

import hashlib
import json
import sqlite3
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

import zstandard

from ingest import SCHEMA_VERSION


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def compress(index: Path, out: Path) -> None:
    cctx = zstandard.ZstdCompressor(level=19, threads=-1)
    with index.open("rb") as src, out.open("wb") as dst:
        cctx.copy_stream(src, dst)


def project_summary(index: Path) -> list[dict[str, Any]]:
    conn = sqlite3.connect(f"file:{index}?mode=ro", uri=True)
    rows = conn.execute(
        "SELECT p.slug, v.version, v.pinned, v.stale FROM versions v JOIN projects p"
        " ON p.id = v.project_id ORDER BY p.slug, v.version DESC"
    ).fetchall()
    conn.close()
    out: dict[str, list[dict[str, Any]]] = {}
    for slug, version, pinned, stale in rows:
        out.setdefault(slug, []).append(
            {"version": version, "pinned": bool(pinned), "stale": bool(stale)}
        )
    return [{"slug": s, "versions": vs} for s, vs in out.items()]


def meta(index: Path) -> dict[str, str]:
    conn = sqlite3.connect(f"file:{index}?mode=ro", uri=True)
    m = dict(conn.execute("SELECT key, value FROM meta").fetchall())
    conn.close()
    return {str(k): str(v) for k, v in m.items()}


def write_release(index: Path, out_dir: Path) -> Path:
    out_dir.mkdir(parents=True, exist_ok=True)
    zst = out_dir / "index.sqlite.zst"
    compress(index, zst)
    m = meta(index)
    manifest = {
        "schema_version": SCHEMA_VERSION,
        "build_date": m.get("build_date") or datetime.now(UTC).isoformat(timespec="seconds"),
        "index": {
            "file": zst.name,
            "sha256": sha256_file(zst),
            "size": zst.stat().st_size,
            "uncompressed_size": index.stat().st_size,
            "uncompressed_sha256": sha256_file(index),
        },
        "ingest_sha": m.get("ingest_sha", ""),
        "registry_sha": m.get("registry_sha", ""),
        "projects": project_summary(index),
    }
    mpath = out_dir / "manifest.json"
    mpath.write_text(json.dumps(manifest, indent=2) + "\n")
    sums = out_dir / "SHA256SUMS"
    sums.write_text("".join(f"{sha256_file(p)}  {p.name}\n" for p in (zst, mpath)))
    return mpath


def attribution(index: Path) -> str:
    conn = sqlite3.connect(f"file:{index}?mode=ro", uri=True)
    rows = conn.execute(
        "SELECT name, homepage, code_license, docs_license, attribution FROM projects ORDER BY name"
    ).fetchall()
    conn.close()
    lines = ["# Attribution", "", "This index redistributes documentation and schemas from:", ""]
    for name, home, code, docs, attr in rows:
        lines.append(f"- **{name}** ({home}) — code: {code}, docs: {docs}. {attr}")
    return "\n".join(lines) + "\n"
