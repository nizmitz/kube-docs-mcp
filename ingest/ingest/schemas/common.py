"""Shared OpenAPI→draft-04 conversion and field-tree flattening."""

from __future__ import annotations

import copy
from typing import Any

from ingest.db import FieldRow

DRAFT4 = "http://json-schema.org/draft-04/schema#"
MAX_DEPTH = 6
KEEP_X = {"x-kubernetes-preserve-unknown-fields"}

JsonObj = dict[str, Any]


DROP_KEYS = {"nullable", "example", "discriminator", "xml", "externalDocs"}


def _dropped(key: str) -> bool:
    if key in DROP_KEYS:
        return True
    return key.startswith("x-kubernetes-") and key not in KEEP_X


def convert(node: Any, ref_prefix: str = "#/components/schemas/") -> Any:
    """Recursively convert an OpenAPI 3.0 schema node into draft-04 dialect.

    - `#/components/schemas/X` refs become `#/definitions/X`
    - `nullable: true` → type gains "null"
    - `x-kubernetes-int-or-string` → oneOf integer|string
    - other `x-kubernetes-*` dropped except preserve-unknown-fields
    """
    if isinstance(node, list):
        return [convert(n, ref_prefix) for n in node]
    if not isinstance(node, dict):
        return node
    out: JsonObj = {}
    for k, v in node.items():
        if k == "$ref" and isinstance(v, str) and v.startswith(ref_prefix):
            out["$ref"] = "#/definitions/" + v[len(ref_prefix) :]
        elif _dropped(k):
            continue
        elif k in ("properties", "definitions", "patternProperties"):
            out[k] = {pk: convert(pv, ref_prefix) for pk, pv in v.items()}
        elif k in ("items", "additionalProperties", "not") and isinstance(v, dict | bool):
            out[k] = convert(v, ref_prefix) if isinstance(v, dict) else v
        elif k in ("allOf", "anyOf", "oneOf"):
            out[k] = [convert(x, ref_prefix) for x in v]
        else:
            out[k] = v
    if node.get("x-kubernetes-int-or-string"):
        out.pop("type", None)
        out["oneOf"] = [{"type": "integer"}, {"type": "string"}]
    if node.get("nullable") and "type" in out:
        t = out["type"]
        out["type"] = [t, "null"] if isinstance(t, str) else [*t, "null"]
    return out


def collect_refs(node: Any, acc: set[str]) -> None:
    if isinstance(node, dict):
        ref = node.get("$ref")
        if isinstance(ref, str) and ref.startswith("#/definitions/"):
            acc.add(ref[len("#/definitions/") :])
        for v in node.values():
            collect_refs(v, acc)
    elif isinstance(node, list):
        for v in node:
            collect_refs(v, acc)


def standalone(root_name: str, definitions: dict[str, JsonObj]) -> JsonObj:
    """Build a self-contained draft-04 schema with only reachable definitions."""
    reachable: set[str] = set()
    todo = [root_name]
    while todo:
        name = todo.pop()
        if name in reachable or name not in definitions:
            continue
        reachable.add(name)
        refs: set[str] = set()
        collect_refs(definitions[name], refs)
        todo.extend(refs - reachable)
    return {
        "$schema": DRAFT4,
        "$ref": f"#/definitions/{root_name}",
        "definitions": {n: definitions[n] for n in sorted(reachable)},
    }


def set_gvk_enums(schema: JsonObj, group: str, version: str, kind: str) -> None:
    """Pin apiVersion/kind on the root definition (or root object)."""
    root = schema
    ref = schema.get("$ref")
    if isinstance(ref, str) and ref.startswith("#/definitions/"):
        root = schema["definitions"][ref[len("#/definitions/") :]]
    props = root.setdefault("properties", {})
    api_version = f"{group}/{version}" if group else version
    props["apiVersion"] = {**props.get("apiVersion", {"type": "string"}), "enum": [api_version]}
    props["kind"] = {**props.get("kind", {"type": "string"}), "enum": [kind]}


def _resolve(node: JsonObj, definitions: dict[str, JsonObj]) -> tuple[JsonObj, str | None]:
    """Follow $ref / single allOf wrapper. Returns (node, ref_name)."""
    name: str | None = None
    for _ in range(8):
        ref = node.get("$ref")
        if isinstance(ref, str) and ref.startswith("#/definitions/"):
            name = ref[len("#/definitions/") :]
            merged = copy.copy(definitions.get(name, {}))
            for k, v in node.items():
                if k != "$ref":
                    merged.setdefault(k, v)
            node = merged
            continue
        all_of = node.get("allOf")
        if isinstance(all_of, list) and len(all_of) == 1 and isinstance(all_of[0], dict):
            merged = {k: v for k, v in node.items() if k != "allOf"}
            for k, v in all_of[0].items():
                merged.setdefault(k, v)
            node = merged
            continue
        break
    return node, name


def type_name(node: JsonObj, definitions: dict[str, JsonObj]) -> str:
    node, name = _resolve(node, definitions)
    if "oneOf" in node and {t.get("type") for t in node["oneOf"]} == {"integer", "string"}:
        return "int-or-string"
    t = node.get("type")
    if isinstance(t, list):
        t = next((x for x in t if x != "null"), "object")
    if t == "array":
        items = node.get("items")
        return "[]" + (type_name(items, definitions) if isinstance(items, dict) else "object")
    if t == "object" or (t is None and ("properties" in node or "additionalProperties" in node)):
        ap = node.get("additionalProperties")
        if isinstance(ap, dict) and "properties" not in node:
            return f"map[string]{type_name(ap, definitions)}"
        if name:
            return name.rsplit(".", 1)[-1]
        return "object"
    if t is None:
        return name.rsplit(".", 1)[-1] if name else "object"
    return str(t)


def walk_fields(schema: JsonObj) -> list[FieldRow]:
    """Flatten a standalone schema into FieldRows (depth-limited, cycle-safe)."""
    definitions: dict[str, JsonObj] = schema.get("definitions", {})
    rows: list[FieldRow] = []

    def visit(node: JsonObj, prefix: str, depth: int, stack: tuple[str, ...]) -> None:
        node, name = _resolve(node, definitions)
        if name and name in stack:
            return
        if depth > MAX_DEPTH:
            return
        nstack = (*stack, name) if name else stack
        t = node.get("type")
        if t == "array" and isinstance(node.get("items"), dict):
            visit(node["items"], prefix + "[]", depth, nstack)
            return
        props = node.get("properties")
        if not isinstance(props, dict):
            ap = node.get("additionalProperties")
            if isinstance(ap, dict) and depth < MAX_DEPTH:
                visit(ap, prefix + "{}", depth + 1, nstack)
            return
        required = set(node.get("required", []))
        for pname, pnode in props.items():
            if not isinstance(pnode, dict):
                continue
            path = f"{prefix}.{pname}" if prefix else pname
            enum = pnode.get("enum")
            if enum is None:
                rnode, _ = _resolve(pnode, definitions)
                enum = rnode.get("enum")
            rows.append(
                FieldRow(
                    path=path,
                    type=type_name(pnode, definitions),
                    required=pname in required,
                    description=pnode.get("description")
                    or _resolve(pnode, definitions)[0].get("description"),
                    enum=[str(e) for e in enum] if isinstance(enum, list) else None,
                )
            )
            visit(pnode, path, depth + 1, nstack)

    visit(schema, "", 0, ())
    return rows
