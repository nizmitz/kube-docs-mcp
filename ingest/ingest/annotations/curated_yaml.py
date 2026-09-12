"""Curated annotation catalogs from registry/annotations/*.yaml."""

from __future__ import annotations

from ingest.db import AnnotationRow
from ingest.models import CuratedCatalog


def rows(catalog: CuratedCatalog) -> list[AnnotationRow]:
    out: list[AnnotationRow] = []
    for e in catalog.entries:
        dep = e.deprecated
        out.append(
            AnnotationRow(
                key=e.key,
                type=e.type,
                applies_to=e.applies_to,
                description=e.description,
                doc_url=e.doc_url,
                value_type=e.value_type,
                allowed_values=e.allowed_values,
                example=e.example,
                since=e.since,
                deprecated=(dep if isinstance(dep, str) else ("true" if dep else None)),
                source="curated",
            )
        )
    return out
