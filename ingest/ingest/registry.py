"""Load and validate registry YAML files (projects + curated annotations)."""

from __future__ import annotations

import json
from pathlib import Path

import jsonschema
import yaml

from ingest.models import CuratedCatalog, Project

REGISTRY_DIR = Path(__file__).resolve().parents[2] / "registry"


class RegistryError(ValueError):
    pass


def _load_schema(name: str) -> dict[str, object]:
    with (REGISTRY_DIR / "schema" / name).open() as fh:
        data: dict[str, object] = json.load(fh)
        return data


def _validate(data: object, schema_name: str, path: Path) -> None:
    schema = _load_schema(schema_name)
    validator = jsonschema.Draft202012Validator(schema, format_checker=jsonschema.FormatChecker())
    errors = sorted(validator.iter_errors(data), key=lambda e: list(e.path))
    if errors:
        msgs = [f"  {'/'.join(str(p) for p in e.path) or '<root>'}: {e.message}" for e in errors]
        raise RegistryError(f"{path}:\n" + "\n".join(msgs))


def load_project(path: Path) -> Project:
    with path.open() as fh:
        data = yaml.safe_load(fh)
    _validate(data, "project.schema.json", path)
    project = Project.model_validate(data)
    if project.slug != path.stem:
        raise RegistryError(f"{path}: slug '{project.slug}' must match filename")
    for a in project.annotations:
        if a.type == "curated-yaml" and a.path:
            load_curated(REGISTRY_DIR / a.path)
    return project


def load_curated(path: Path) -> CuratedCatalog:
    if not path.exists():
        raise RegistryError(f"curated annotations file missing: {path}")
    with path.open() as fh:
        data = yaml.safe_load(fh)
    _validate(data, "annotations.schema.json", path)
    return CuratedCatalog.model_validate(data)


def load_all(registry: Path = REGISTRY_DIR) -> list[Project]:
    projects = [load_project(p) for p in sorted((registry / "projects").glob("*.yaml"))]
    projects += [load_project(p) for p in sorted((registry / "bootstrap").glob("*.yaml"))]
    return projects


def find(slug: str, registry: Path = REGISTRY_DIR) -> Project:
    for sub in ("projects", "bootstrap"):
        p = registry / sub / f"{slug}.yaml"
        if p.exists():
            return load_project(p)
    raise RegistryError(f"unknown project '{slug}'")
