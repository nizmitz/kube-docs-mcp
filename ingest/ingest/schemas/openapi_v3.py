"""Kubernetes core: api/openapi-spec/v3/*.json → per-kind standalone schemas."""

from __future__ import annotations

import json
from collections.abc import Iterator
from pathlib import Path
from typing import Any

from ingest.db import FieldRow, KindRow
from ingest.schemas.common import convert, set_gvk_enums, standalone, walk_fields

REF_PREFIX = "#/components/schemas/"


def _gvk_key(g: dict[str, Any]) -> tuple[str, str, str]:
    return (str(g.get("group", "")), str(g.get("version", "")), str(g.get("kind", "")))


def load_dir(files: list[Path]) -> tuple[dict[str, Any], dict[tuple[str, str, str], str]]:
    """Merge all spec files. Returns (definitions, scope-by-gvk)."""
    definitions: dict[str, Any] = {}
    scope: dict[tuple[str, str, str], str] = {}
    for f in sorted(files):
        with f.open() as fh:
            doc = json.load(fh)
        for name, node in doc.get("components", {}).get("schemas", {}).items():
            if name not in definitions:
                definitions[name] = convert(node, REF_PREFIX)
        for path, item in doc.get("paths", {}).items():
            namespaced = "{namespace}" in path
            for op in item.values():
                if not isinstance(op, dict):
                    continue
                gvk = op.get("x-kubernetes-group-version-kind")
                if isinstance(gvk, dict):
                    key = _gvk_key(gvk)
                    if namespaced:
                        scope[key] = "Namespaced"
                    else:
                        scope.setdefault(key, "Cluster")
    return definitions, scope


def kinds(files: list[Path]) -> Iterator[tuple[KindRow, list[FieldRow]]]:
    definitions, scope = load_dir(files)
    seen: set[tuple[str, str, str]] = set()
    # x-kubernetes-group-version-kind is dropped by convert(); re-read raw files for gvk mapping
    raw_gvk: dict[str, list[dict[str, Any]]] = {}
    for f in files:
        with f.open() as fh:
            doc = json.load(fh)
        for name, node in doc.get("components", {}).get("schemas", {}).items():
            g = node.get("x-kubernetes-group-version-kind")
            if g and name not in raw_gvk:
                raw_gvk[name] = g
    for name, gvks in raw_gvk.items():
        for gvk in gvks:
            key = _gvk_key(gvk)
            if key in seen or key[2].endswith("List"):
                continue
            seen.add(key)
            schema = standalone(name, definitions)
            set_gvk_enums(schema, *key)
            desc = definitions[name].get("description")
            row = KindRow(
                api_group=key[0],
                api_version=key[1],
                kind=key[2],
                scope=scope.get(key),
                description=desc,
                schema=schema,
            )
            yield row, walk_fields(schema)
