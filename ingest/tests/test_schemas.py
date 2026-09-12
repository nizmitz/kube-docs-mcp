from pathlib import Path

import jsonschema

from ingest.schemas import crd_yaml, openapi_v3
from ingest.schemas.common import convert, standalone, type_name, walk_fields


def test_openapi_kinds_and_fields(fixtures: Path) -> None:
    out = {k.kind: (k, f) for k, f in openapi_v3.kinds([fixtures / "openapi_mini.json"])}
    assert set(out) == {"Pod", "Node"}  # PodList skipped
    pod, fields = out["Pod"]
    assert pod.scope == "Namespaced" and out["Node"][0].scope == "Cluster"
    assert pod.api_group == "" and pod.api_version == "v1"
    assert pod.schema["$schema"].startswith("http://json-schema.org/draft-04")
    defs = pod.schema["definitions"]
    assert "io.k8s.api.core.v1.Container" in defs and "io.k8s.JSONSchemaProps" not in defs
    root = defs["io.k8s.api.core.v1.Pod"]
    assert root["properties"]["kind"]["enum"] == ["Pod"]
    assert root["properties"]["apiVersion"]["enum"] == ["v1"]
    by_path = {f.path: f for f in fields}
    assert by_path["spec.containers"].type == "[]Container"
    assert by_path["spec.containers"].required is True
    assert by_path["spec.containers[].name"].required is True
    assert by_path["spec.containers[].ports[].containerPort"].type == "integer"
    assert by_path["spec.containers[].targetPort"].type == "int-or-string"
    assert by_path["spec.nodeSelector"].type == "map[string]string"
    assert by_path["spec.restartPolicy"].enum == ["Always", "OnFailure", "Never"]
    assert by_path["spec"].description == "Spec of the pod"
    # draft-04 validity + validation works with local refs
    jsonschema.Draft4Validator.check_schema(pod.schema)
    v = jsonschema.Draft4Validator(pod.schema)
    errs = list(v.iter_errors({"apiVersion": "v1", "kind": "Pod", "spec": {"containers": [{}]}}))
    assert any("name" in e.message for e in errs)
    assert not list(
        v.iter_errors(
            {
                "apiVersion": "v1",
                "kind": "Pod",
                "spec": {"containers": [{"name": "c", "targetPort": "http"}]},
            }
        )
    )


def test_openapi_cycle_is_cut(fixtures: Path) -> None:
    out = {k.kind: f for k, f in openapi_v3.kinds([fixtures / "openapi_mini.json"])}
    paths = [f.path for f in out["Node"]]
    assert "spec.not" in paths and "spec.not.title" not in paths  # cycle stops at revisit
    assert all(p.count(".") <= 7 for p in paths)


def test_convert_nullable_and_x_keys() -> None:
    c = convert(
        {
            "type": "string",
            "nullable": True,
            "x-kubernetes-list-type": "set",
            "x-kubernetes-preserve-unknown-fields": True,
        }
    )
    assert c["type"] == ["string", "null"]
    assert "x-kubernetes-list-type" not in c and c["x-kubernetes-preserve-unknown-fields"] is True


def test_standalone_only_reachable() -> None:
    defs = {"A": {"$ref": "#/definitions/B"}, "B": {"type": "string"}, "C": {"type": "integer"}}
    s = standalone("A", defs)
    assert set(s["definitions"]) == {"A", "B"} and s["$ref"] == "#/definitions/A"
    assert type_name({"$ref": "#/definitions/B"}, defs) == "string"


def test_crd_versions(fixtures: Path) -> None:
    rows = list(crd_yaml.kinds([fixtures / "crd.yaml"]))
    assert [(k.api_version, k.kind) for k, _ in rows] == [("v1", "Widget"), ("v1alpha1", "Widget")]
    k, fields = rows[0]
    assert k.api_group == "example.io" and k.scope == "Namespaced"
    assert k.description == "Widget does things."
    assert k.schema["properties"]["apiVersion"]["enum"] == ["example.io/v1"]
    assert k.schema["properties"]["metadata"] == {"type": "object"}
    by = {f.path: f for f in fields}
    assert by["spec.size"].required and by["spec.size"].type == "integer"
    assert by["spec.mode"].enum == ["fast", "slow"]
    assert by["spec.port"].type == "int-or-string"
    assert k.schema["properties"]["spec"]["properties"]["extra"][
        "x-kubernetes-preserve-unknown-fields"
    ]
    jsonschema.Draft4Validator.check_schema(k.schema)
    assert list(walk_fields(k.schema)) == fields
