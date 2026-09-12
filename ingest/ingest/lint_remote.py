"""`ingest lint --remote`: verify globs match files at the resolved ref (GitHub tree API)."""

from __future__ import annotations

import fnmatch
from typing import Any

import httpx

from ingest.fetch import resolve_ref
from ingest.models import Project, ResolvedVersion
from ingest.versions import _client, resolve


def _tree(repo: str, ref: str, c: httpx.Client) -> list[str]:
    r = c.get(f"/repos/{repo}/git/trees/{ref}", params={"recursive": "1"})
    r.raise_for_status()
    data: dict[str, Any] = r.json()
    return [str(e["path"]) for e in data.get("tree", []) if e.get("type") == "blob"]


def _match(paths: list[str], pattern: str) -> bool:
    # translate ** glob to fnmatch semantics loosely
    pat = pattern.replace("**/", "*").replace("**", "*")
    return any(fnmatch.fnmatch(p, pat) for p in paths)


class _TreeCache:
    def __init__(self, client: httpx.Client, version: ResolvedVersion) -> None:
        self.c = client
        self.v = version
        self.trees: dict[tuple[str, str], list[str]] = {}

    def get(self, repo: str, ref_tpl: str) -> list[str]:
        ref = resolve_ref(repo, ref_tpl, self.v)
        key = (repo, ref)
        if key not in self.trees:
            self.trees[key] = _tree(repo, ref, self.c)
        return self.trees[key]


def check_remote(projects: list[Project], token: str | None) -> list[str]:
    errors: list[str] = []
    with _client(token) as c:
        for p in projects:
            try:
                versions = resolve(p.source.repo, p.source.versions, None, token)
            except httpx.HTTPError as e:
                errors.append(f"{p.slug}: cannot resolve versions: {e}")
                continue
            if not versions:
                errors.append(f"{p.slug}: strategy resolved zero versions")
                continue
            v = versions[0]
            trees = _TreeCache(c, v)
            tree_for = trees.get
            for s in p.schemas:
                if s.type == "crd-release-asset":
                    continue
                repo = s.repo or p.source.repo
                paths = tree_for(repo, s.ref)
                for g in s.paths:
                    if not _match(paths, g):
                        errors.append(
                            f"{p.slug}: schema glob '{g}' matches nothing in {repo}@{v.tag}"
                        )
            for d in p.docs:
                repo = d.repo or p.source.repo
                paths = tree_for(repo, d.ref)
                for g in d.include:
                    if not _match(paths, f"{d.root.strip('/')}/{g}"):
                        errors.append(f"{p.slug}: docs glob '{g}' matches nothing under {d.root}")
            for a in p.annotations:
                if a.type == "curated-yaml" or not a.path:
                    continue
                repo = a.repo or p.source.repo
                if a.path not in tree_for(repo, a.ref):
                    errors.append(f"{p.slug}: annotation path '{a.path}' missing in {repo}")
    return errors
