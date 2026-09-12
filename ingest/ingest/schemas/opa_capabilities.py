"""OPA capabilities.json → pseudo-kind `Rego` whose fields are the built-in functions."""

from __future__ import annotations

import json
from collections.abc import Iterator
from pathlib import Path
from typing import Any

from ingest.db import FieldRow, KindRow
from ingest.schemas.common import DRAFT4


def _type(t: dict[str, Any] | None) -> str:
    if not t:
        return "any"
    kind = str(t.get("type", "any"))
    if kind == "array":
        return "[]" + _type(t.get("dynamic") or (t.get("static") or [None])[0])
    if kind == "set":
        return "set[" + _type(t.get("of")) + "]"
    if kind == "object":
        return "object"
    if kind == "any" and isinstance(t.get("of"), list):
        return "|".join(_type(x) for x in t["of"])
    return kind


def kinds(files: list[Path]) -> Iterator[tuple[KindRow, list[FieldRow]]]:
    for f in files:
        with f.open() as fh:
            caps = json.load(fh)
        fields: list[FieldRow] = []
        props: dict[str, Any] = {}
        for b in caps.get("builtins", []):
            decl = b.get("decl", {})
            args = ", ".join(_type(a) for a in decl.get("args", []))
            res = _type(decl.get("result"))
            sig = f"{b['name']}({args}) -> {res}"
            desc = sig + (" [deprecated]" if b.get("deprecated") else "")
            fields.append(
                FieldRow(path=b["name"], type="function", required=False, description=desc)
            )
            props[b["name"]] = {"type": "string", "description": desc}
        schema: dict[str, Any] = {
            "$schema": DRAFT4,
            "type": "object",
            "description": "OPA built-in functions (from capabilities.json)",
            "properties": props,
        }
        yield (
            KindRow(
                api_group="openpolicyagent.org",
                api_version="builtins",
                kind="Rego",
                scope=None,
                description=f"{len(fields)} Rego built-in functions",
                schema=schema,
            ),
            fields,
        )
