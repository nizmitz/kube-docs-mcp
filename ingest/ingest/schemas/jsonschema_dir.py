"""imroc/kubeschemas-style dir: <group>/<kind>_<version>.json files."""

from __future__ import annotations

import json
from collections.abc import Iterator
from pathlib import Path
from typing import Any

from ingest.db import FieldRow, KindRow
from ingest.schemas.common import DRAFT4, convert, set_gvk_enums, walk_fields


def kinds(files: list[Path], root: Path) -> Iterator[tuple[KindRow, list[FieldRow]]]:
    seen: set[tuple[str, str, str]] = set()
    for f in sorted(files):
        if f.name.endswith("list_" + f.name.split("_")[-1]) and "list_" in f.name:
            continue  # *List kinds
        try:
            with f.open() as fh:
                raw: dict[str, Any] = json.load(fh)
        except (OSError, json.JSONDecodeError):
            continue
        gvks = raw.get("x-kubernetes-group-version-kind") or []
        if not gvks:
            continue
        schema = convert(raw, ref_prefix="#/definitions/")
        schema["$schema"] = DRAFT4
        schema.setdefault("type", "object")
        for g in gvks:
            key = (str(g.get("group", "")), str(g.get("version", "")), str(g.get("kind", "")))
            if key in seen or key[2].endswith("List"):
                continue
            seen.add(key)
            s = json.loads(json.dumps(schema))
            set_gvk_enums(s, *key)
            props = s.setdefault("properties", {})
            props.setdefault("metadata", {"type": "object"})
            yield (
                KindRow(
                    api_group=key[0],
                    api_version=key[1],
                    kind=key[2],
                    scope=None,
                    description=raw.get("description"),
                    schema=s,
                ),
                walk_fields(s),
            )
