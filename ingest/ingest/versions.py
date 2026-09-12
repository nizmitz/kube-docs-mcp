"""Resolve concrete versions for a project from GitHub tags/releases."""

from __future__ import annotations

import json
import os
import re
from pathlib import Path
from typing import Any

import httpx

from ingest.models import ResolvedVersion, Versions

GITHUB_API = "https://api.github.com"
MAX_TAGS = 300


def _client(token: str | None = None) -> httpx.Client:
    headers = {"Accept": "application/vnd.github+json", "User-Agent": "kube-docs-ingest"}
    tok = token or os.environ.get("GITHUB_TOKEN")
    if tok:
        headers["Authorization"] = f"Bearer {tok}"
    return httpx.Client(base_url=GITHUB_API, headers=headers, timeout=30.0)


def list_tags(repo: str, token: str | None = None, limit: int = MAX_TAGS) -> list[str]:
    names: list[str] = []
    with _client(token) as c:
        page = 1
        while len(names) < limit:
            r = c.get(f"/repos/{repo}/tags", params={"per_page": 100, "page": page})
            r.raise_for_status()
            batch: list[dict[str, Any]] = r.json()
            if not batch:
                break
            names.extend(str(t["name"]) for t in batch)
            page += 1
    return names[:limit]


def list_releases(repo: str, token: str | None = None, limit: int = 100) -> list[dict[str, Any]]:
    with _client(token) as c:
        r = c.get(f"/repos/{repo}/releases", params={"per_page": limit})
        r.raise_for_status()
        rel: list[dict[str, Any]] = r.json()
        return rel


def parse_tag(tag: str, pattern: str) -> ResolvedVersion | None:
    m = re.match(pattern, tag)
    if not m or len(m.groups()) < 2:
        return None
    major = int(m.group(1))
    minor = int(m.group(2))
    patch = int(m.group(3)) if len(m.groups()) >= 3 and m.group(3) is not None else 0
    return ResolvedVersion(
        version=f"{major}.{minor}.{patch}", tag=tag, major=major, minor=minor, patch=patch
    )


def select(tags: list[str], spec: Versions) -> list[ResolvedVersion]:
    """Pure selection from a tag list. Newest first."""
    parsed = [v for t in tags if (v := parse_tag(t, spec.tag_pattern)) is not None]
    parsed.sort(key=lambda v: (v.major, v.minor, v.patch), reverse=True)
    if spec.strategy == "fixed":
        out: list[ResolvedVersion] = []
        for t in spec.tags or []:
            v = parse_tag(t, spec.tag_pattern)
            out.append(v or ResolvedVersion(version=t, tag=t, major=0, minor=0, patch=0))
        return out
    if spec.strategy == "latest-releases":
        return parsed[: spec.count]
    # latest-minors: newest patch of each of the newest N minor lines
    seen: set[tuple[int, int]] = set()
    out = []
    for v in parsed:
        key = (v.major, v.minor)
        if key in seen:
            continue
        seen.add(key)
        out.append(v)
        if len(out) >= spec.count:
            break
    return out


def resolve(
    repo: str, spec: Versions, work_dir: Path | None = None, token: str | None = None
) -> list[ResolvedVersion]:
    if spec.strategy == "fixed":
        return select([], spec)
    cache = work_dir / "cache" / f"tags-{repo.replace('/', '__')}.json" if work_dir else None
    tags: list[str] | None = None
    if cache and cache.exists():
        tags = json.loads(cache.read_text())
    if tags is None:
        tags = list_tags(repo, token)
        if not any(parse_tag(t, spec.tag_pattern) for t in tags):
            tags = [
                str(r["tag_name"])
                for r in list_releases(repo, token)
                if not (r.get("prerelease") and not spec.include_prerelease)
            ]
        if cache:
            cache.parent.mkdir(parents=True, exist_ok=True)
            cache.write_text(json.dumps(tags))
    return select(tags, spec)
