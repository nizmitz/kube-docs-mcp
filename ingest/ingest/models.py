"""Typed registry models. Mirrors registry/schema/project.schema.json."""

from __future__ import annotations

from typing import Literal

from pydantic import BaseModel, ConfigDict, Field

Strategy = Literal["latest-minors", "latest-releases", "fixed"]
SchemaType = Literal[
    "openapi-v3", "crd-yaml", "crd-release-asset", "jsonschema-dir", "opa-capabilities"
]
DocsFormat = Literal[
    "hugo-md", "mkdocs-md", "docusaurus-md", "plain-md", "mdx", "sphinx-rst", "yaml-comments"
]
AnnotationType = Literal["k8s-wellknown", "md-table", "md-headings", "curated-yaml"]


class _Strict(BaseModel):
    model_config = ConfigDict(extra="forbid")


class License(_Strict):
    code: str
    docs: str


class Versions(_Strict):
    strategy: Strategy
    count: int = 2
    tag_pattern: str = r"^v?(\d+)\.(\d+)\.(\d+)$"
    tags: list[str] | None = None
    include_prerelease: bool = False


class Source(_Strict):
    repo: str
    versions: Versions


class SchemaSource(_Strict):
    type: SchemaType
    repo: str | None = None
    ref: str = "{tag}"
    paths: list[str] = Field(default_factory=list)
    asset: str | None = None
    strict: bool = True


class UrlSpec(_Strict):
    base: str
    strip_ext: bool = True
    index_file: str = "_index"
    trailing_slash: bool = True


class DocsSource(_Strict):
    repo: str | None = None
    ref: str = "{tag}"
    format: DocsFormat
    root: str
    include: list[str]
    exclude: list[str] = Field(default_factory=list)
    url: UrlSpec
    license: str | None = None


class Columns(_Strict):
    key: str
    description: str | None = None
    values: str | None = None
    applies_to: str | None = None
    since: str | None = None
    type: str | None = None
    default: str | None = None


class AnnotationSource(_Strict):
    type: AnnotationType
    repo: str | None = None
    ref: str = "{tag}"
    path: str | None = None
    url: str | None = None
    columns: Columns | None = None
    applies_to: list[str] = Field(default_factory=list)
    heading_level: int = 2


class Project(_Strict):
    slug: str
    name: str
    category: Literal["core", "cncf", "cloud", "controller", "vendor", "bootstrap"]
    homepage: str
    license: License
    attribution: str
    source: Source
    schemas: list[SchemaSource] = Field(default_factory=list)
    docs: list[DocsSource] = Field(default_factory=list)
    annotations: list[AnnotationSource] = Field(default_factory=list)


class CuratedEntry(_Strict):
    key: str
    type: Literal["annotation", "label", "taint"] = "annotation"
    applies_to: list[str]
    value_type: str | None = None
    allowed_values: list[str] | None = None
    example: str | None = None
    description: str
    doc_url: str
    since: str | None = None
    deprecated: bool | str = False


class CuratedCatalog(_Strict):
    provider: str
    license: str
    attribution: str
    entries: list[CuratedEntry]


class ResolvedVersion(BaseModel):
    """A concrete version chosen by the version strategy."""

    model_config = ConfigDict(frozen=True)
    version: str  # display, e.g. "1.37.0"
    tag: str  # git tag, e.g. "v1.37.0"
    major: int
    minor: int
    patch: int

    def render(self, template: str) -> str:
        return template.format(
            tag=self.tag, major=self.major, minor=self.minor, patch=self.patch, version=self.version
        )
