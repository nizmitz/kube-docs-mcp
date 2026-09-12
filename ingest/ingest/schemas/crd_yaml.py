"""CustomResourceDefinition YAML → per served version standalone schemas."""

from __future__ import annotations

from collections.abc import Iterator
from pathlib import Path
from typing import Any

import yaml

from ingest.db import FieldRow, KindRow
from ingest.schemas.common import DRAFT4, convert, walk_fields


def _docs(path: Path) -> Iterator[dict[str, Any]]:
    with path.open() as fh:
        for doc in yaml.safe_load_all(fh):
            if isinstance(doc, dict) and doc.get("kind") == "CustomResourceDefinition":
                yield doc


def kinds_from_doc(crd: dict[str, Any]) -> Iterator[tuple[KindRow, list[FieldRow]]]:
    spec = crd.get("spec", {})
    group = str(spec.get("group", ""))
    kind = str(spec.get("names", {}).get("kind", ""))
    scope = str(spec.get("scope", "Namespaced"))
    versions = spec.get("versions") or [
        {"name": spec.get("version"), "served": True, "schema": spec.get("validation", {})}
    ]
    for v in versions:
        if not v.get("served", True):
            continue
        version = str(v["name"])
        raw = (v.get("schema") or {}).get("openAPIV3Schema") or {"type": "object"}
        schema = convert(raw, ref_prefix="#/definitions/")
        schema.setdefault("type", "object")
        props = schema.setdefault("properties", {})
        props["apiVersion"] = {"type": "string", "enum": [f"{group}/{version}"]}
        props["kind"] = {"type": "string", "enum": [kind]}
        props.setdefault("metadata", {"type": "object"})
        schema["$schema"] = DRAFT4
        desc = raw.get("description") or (raw.get("properties", {}).get("spec", {}) or {}).get(
            "description"
        )
        yield (
            KindRow(
                api_group=group,
                api_version=version,
                kind=kind,
                scope=scope,
                description=desc,
                schema=schema,
            ),
            walk_fields(schema),
        )


def kinds(files: list[Path]) -> Iterator[tuple[KindRow, list[FieldRow]]]:
    for f in sorted(files):
        for crd in _docs(f):
            yield from kinds_from_doc(crd)
