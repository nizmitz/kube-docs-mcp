from pathlib import Path

import pytest
import yaml

from ingest import registry as reg


def test_load_real_registry() -> None:
    k8s = reg.find("k8s")
    assert k8s.docs[0].format == "hugo-md" and k8s.schemas[0].type == "openapi-v3"


def test_lint_rejects_bad(tmp_path: Path) -> None:
    (tmp_path / "projects").mkdir()
    bad = tmp_path / "projects" / "bad.yaml"
    bad.write_text(yaml.safe_dump({"slug": "bad", "name": "x"}))
    with pytest.raises(reg.RegistryError, match="required"):
        reg.load_project(bad)
    mismatch = tmp_path / "projects" / "other.yaml"
    good = yaml.safe_load((reg.REGISTRY_DIR / "projects" / "k8s.yaml").read_text())
    mismatch.write_text(yaml.safe_dump(good))
    with pytest.raises(reg.RegistryError, match="must match filename"):
        reg.load_project(mismatch)
    with pytest.raises(reg.RegistryError, match="unknown project"):
        reg.find("nope", tmp_path)
    with pytest.raises(reg.RegistryError, match="missing"):
        reg.load_curated(tmp_path / "nope.yaml")
