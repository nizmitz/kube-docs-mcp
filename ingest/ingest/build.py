"""Build one project shard: resolve versions → checkout → schemas/docs/annotations → sqlite."""

from __future__ import annotations

import fnmatch
import json
import logging
from pathlib import Path

from rich.logging import RichHandler

from ingest import registry as reg
from ingest.annotations import curated_yaml, k8s_wellknown, md_headings, md_table
from ingest.db import BuildStats, ProjectRow, Section, ShardWriter
from ingest.docs import DRIVERS, extensions
from ingest.fetch import checkout, download_release_asset, sparse_dir
from ingest.models import DocsSource, Project, ResolvedVersion, SchemaSource
from ingest.schemas import crd_yaml, openapi_v3
from ingest.versions import parse_tag, resolve

log = logging.getLogger("ingest")


def setup_logging(verbose: bool = False) -> None:
    logging.basicConfig(
        level=logging.DEBUG if verbose else logging.INFO,
        format="%(message)s",
        handlers=[RichHandler(show_path=False, rich_tracebacks=True)],
    )
    logging.getLogger("httpx").setLevel(logging.WARNING)


def _glob(root: Path, patterns: list[str], excludes: list[str] | None = None) -> list[Path]:
    out: list[Path] = []
    for pat in patterns:
        for p in sorted(root.glob(pat)):
            if not p.is_file():
                continue
            rel = p.relative_to(root).as_posix()
            if excludes and any(fnmatch.fnmatch(rel, e) for e in excludes):
                continue
            out.append(p)
    return out


def _ctx(v: ResolvedVersion) -> dict[str, str]:
    return {
        "tag": v.tag,
        "major": str(v.major),
        "minor": str(v.minor),
        "patch": str(v.patch),
        "version": v.version,
    }


def _schema_source(
    project: Project, src: SchemaSource, v: ResolvedVersion, work: Path, w: ShardWriter, vid: int
) -> None:
    repo = src.repo or project.source.repo
    if src.type == "crd-release-asset":
        assert src.asset
        f = download_release_asset(repo, v.tag, v.render(src.asset), work)
        for kind, fields in crd_yaml.kinds([f]):
            w.add_kind(vid, kind, fields)
        return
    co = checkout(repo, src.ref, v, work, [sparse_dir(p) for p in src.paths])
    files = _glob(co, src.paths)
    if not files:
        log.warning("%s: schema paths %s matched no files", project.slug, src.paths)
    if src.type == "openapi-v3":
        for kind, fields in openapi_v3.kinds(files):
            w.add_kind(vid, kind, fields)
    elif src.type == "crd-yaml":
        for kind, fields in crd_yaml.kinds(files):
            w.add_kind(vid, kind, fields)
    elif src.type == "jsonschema-dir":
        from ingest.schemas import jsonschema_dir

        for kind, fields in jsonschema_dir.kinds(files, co):
            w.add_kind(vid, kind, fields)
    elif src.type == "opa-capabilities":
        from ingest.schemas import opa_capabilities

        for kind, fields in opa_capabilities.kinds(files):
            w.add_kind(vid, kind, fields)


def _docs_source(
    project: Project, src: DocsSource, v: ResolvedVersion, work: Path, w: ShardWriter, vid: int
) -> int:
    repo = src.repo or project.source.repo
    roots = [v.render(r.strip()).strip("/") for r in src.root.split("|") if r.strip()]
    co = checkout(repo, src.ref, v, work, [f"/{r}/" for r in roots])
    root = next((co / r for r in roots if (co / r).is_dir()), None)
    if root is None:
        log.warning("%s: none of docs roots %s exist in %s", project.slug, roots, repo)
        return 0
    exts = extensions(src.format)
    files = [f for f in _glob(root, src.include, src.exclude) if f.suffix in exts]
    driver = DRIVERS[src.format]
    ctx = _ctx(v)
    sections: list[Section] = []
    for f in files:
        rel = f.relative_to(root).as_posix()
        try:
            text = f.read_text(encoding="utf-8", errors="replace")
            sections.extend(driver(text, rel, src.url, ctx, f))
        except Exception as e:  # one bad page must not kill the shard
            log.warning("%s: failed to parse %s: %s", project.slug, rel, e)
    n = w.add_sections(vid, sections, src.license or project.license.docs)
    log.info("%s %s docs: %d files → %d sections", project.slug, v.version, len(files), n)
    return n


def _annotations(
    project: Project, v: ResolvedVersion, work: Path, w: ShardWriter, pid: int
) -> None:
    for src in project.annotations:
        if src.type == "curated-yaml":
            assert src.path
            cat = reg.load_curated(reg.REGISTRY_DIR / src.path)
            w.add_annotations(pid, curated_yaml.rows(cat))
            continue
        assert src.path and src.url
        repo = src.repo or project.source.repo
        co = checkout(repo, src.ref, v, work, [f"/{src.path}"])
        f = co / src.path
        if not f.is_file():
            log.warning("%s: annotation source %s missing", project.slug, src.path)
            continue
        text = f.read_text(encoding="utf-8", errors="replace")
        if src.type == "k8s-wellknown":
            w.add_annotations(pid, k8s_wellknown.parse(text, src.url))
        elif src.type == "md-table":
            assert src.columns
            w.add_annotations(pid, md_table.parse(text, src.columns, src.url, src.applies_to))
        elif src.type == "md-headings":
            w.add_annotations(
                pid, md_headings.parse(text, src.url, src.heading_level, src.applies_to)
            )


def build_project(
    project: Project,
    work_dir: Path,
    out_shard: Path,
    versions_override: list[str] | None = None,
    github_token: str | None = None,
) -> BuildStats:
    work_dir.mkdir(parents=True, exist_ok=True)
    out_shard.parent.mkdir(parents=True, exist_ok=True)
    if versions_override:
        versions = [
            parse_tag(t, project.source.versions.tag_pattern)
            or ResolvedVersion(version=t, tag=t, major=0, minor=0, patch=0)
            for t in versions_override
        ]
    else:
        versions = resolve(project.source.repo, project.source.versions, work_dir, github_token)
    if not versions:
        raise RuntimeError(f"{project.slug}: no versions resolved")
    log.info("%s: versions %s", project.slug, [v.tag for v in versions])
    w = ShardWriter(out_shard)
    try:
        pid = w.add_project(
            ProjectRow(
                slug=project.slug,
                name=project.name,
                category=project.category,
                homepage=project.homepage,
                code_license=project.license.code,
                docs_license=project.license.docs,
                attribution=project.attribution,
            )
        )
        bootstrap = project.category == "bootstrap"
        for v in versions:
            vid = w.add_version(
                pid,
                v.version,
                v.tag,
                pinned=not bootstrap,
                source=project.slug if bootstrap else "own",
            )
            for s in project.schemas:
                _schema_source(project, s, v, work_dir, w, vid)
            for d in project.docs:
                _docs_source(project, d, v, work_dir, w, vid)
        _annotations(project, versions[0], work_dir, w, pid)
        w.set_meta(f"project:{project.slug}", json.dumps([v.tag for v in versions]))
        w.finalize()
    finally:
        w.close()
    log.info("%s: %s", project.slug, w.stats)
    return w.stats
