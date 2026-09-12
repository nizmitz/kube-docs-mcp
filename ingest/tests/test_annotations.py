from pathlib import Path

from ingest.annotations import curated_yaml, k8s_wellknown, md_table
from ingest.models import Columns
from ingest.registry import load_curated


def test_wellknown(fixtures: Path) -> None:
    rows = k8s_wellknown.parse((fixtures / "wellknown.md").read_text(), "https://k8s.io/x/")
    by = {r.key: r for r in rows}
    assert set(by) == {
        "app.kubernetes.io/component",
        "app.kubernetes.io/name",
        "kubernetes.io/limit-ranger",
        "service.kubernetes.io/topology-mode",
        "node.kubernetes.io/not-ready",
    }
    c = by["app.kubernetes.io/component"]
    assert c.type == "label" and c.applies_to == ["*"]
    assert c.example == 'app.kubernetes.io/component: "database"'
    assert c.description.startswith("The component within")
    assert "recommended labels" in c.description and "](" not in c.description
    lr = by["kubernetes.io/limit-ranger"]
    assert lr.type == "annotation" and lr.applies_to == ["Pod"]
    assert "glossary_tooltip" not in lr.description and "LimitRanger does this" in lr.description
    assert by["service.kubernetes.io/topology-mode"].applies_to == [
        "Service",
        "Endpoints",
        "EndpointSlice",
    ]
    nr = by["node.kubernetes.io/not-ready"]
    assert nr.type == "taint" and nr.doc_url == "https://k8s.io/x/#node-kubernetes-io-not-ready"
    assert c.doc_url == "https://k8s.io/x/#appkubernetesio-component"


def test_md_table(fixtures: Path) -> None:
    cols = Columns(key="Name", description="Description", type="Type")
    rows = md_table.parse((fixtures / "table.md").read_text(), cols, "https://d/", ["Ingress"])
    by = {r.key: r for r in rows}
    assert set(by) == {"alb.ingress.kubernetes.io/scheme", "alb.ingress.kubernetes.io/tags"}
    assert by["alb.ingress.kubernetes.io/scheme"].value_type == "string"
    assert by["alb.ingress.kubernetes.io/scheme"].applies_to == ["Ingress"]
    rows2 = md_table.parse(
        (fixtures / "table.md").read_text(),
        Columns(key="Annotation", description="Description"),
        "https://d/",
        [],
    )
    assert [r.key for r in rows2] == ["service.beta.kubernetes.io/aws-load-balancer-type"]
    assert rows2[0].applies_to == ["*"]


def test_curated(fixtures: Path) -> None:
    cat = load_curated(fixtures / "curated.yaml")
    rows = curated_yaml.rows(cat)
    assert rows[0].source == "curated" and rows[0].value_type == "json"
    assert rows[1].deprecated == "use spec.loadBalancerClass"
    assert rows[1].allowed_values == ["Internal"] and rows[0].deprecated is None


def test_md_table_anchor_description(fixtures: Path) -> None:
    cols = Columns(key="Name", type="Type", default="Default", applies_to="Location")
    rows = md_table.parse((fixtures / "table2.md").read_text(), cols, "https://d/", ["Ingress"])
    assert [r.key for r in rows] == [
        "alb.ingress.kubernetes.io/scheme",
        "alb.ingress.kubernetes.io/tags",
    ]
    s = rows[0]
    assert s.doc_url == "https://d/#scheme" and s.description == "Paragraph via heading slug."
    assert s.applies_to == ["Ingress", "IngressGroup"]
    assert s.value_type == "string (default: internet-facing)"
    assert rows[1].value_type == "stringMap" and rows[1].applies_to == ["Ingress"]
    # first-table-only + html anchors + description fallback via <a name>
    rows3 = md_table.parse(
        (fixtures / "table.md").read_text(),
        Columns(key="Name", type="Type"),
        "https://d/",
        [],
    )
    assert rows3[0].description == "The scheme paragraph describes the annotation in detail."
    assert rows3[1].description == "Tags paragraph."


def test_md_headings(fixtures: Path) -> None:
    from ingest.annotations import md_headings

    rows = md_headings.parse(
        (fixtures / "headings.md").read_text(), "https://c/", 2, ["Certificate"]
    )
    assert [r.key for r in rows] == ["cert-manager.io/issuer", "cert-manager.io/cluster-issuer"]
    a = rows[0]
    assert a.applies_to == ["Ingress", "Gateway"]
    assert a.description.startswith("The name of an Issuer") and "More details." in a.description
    assert "annotations:" in a.description and "Skip this" not in a.description
    assert a.doc_url == "https://c/#cert-managerioissuer"
    b = rows[1]
    assert b.applies_to == ["Certificate"] and b.doc_url == "https://c/#cluster-issuer"
