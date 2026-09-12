# Contributing

## Add a project (no code)

1. Copy `registry/projects/k8s.yaml` → `registry/projects/<slug>.yaml`. Fill `source.repo`, `schemas`, `docs`,
   `annotations`. Keep docs to **reference material** (API refs, config refs, CLI refs, annotations pages) via
   `include` globs — no tutorials or blog posts.
2. Validate: `cd ingest && uv run ingest lint` (schema check) and `uv run ingest lint --remote` (checks every
   glob matches at the resolved tag).
3. Try it: `uv run ingest build <slug> --out /tmp/<slug>.sqlite --work /tmp/work` then inspect with `sqlite3`.
4. Open a PR. CI runs lint, tests, semgrep, CodeQL, trivy.

Supported docs formats: `hugo-md`, `mkdocs-md`, `docusaurus-md`, `plain-md`, `mdx`, `sphinx-rst`, `yaml-comments`.
Supported schema sources: `openapi-v3`, `crd-yaml`, `crd-release-asset`, `jsonschema-dir`, `opa-capabilities`.
Annotation sources: `k8s-wellknown`, `md-table`, `curated-yaml`. A new format needs a driver in `ingest/ingest/docs/`
plus a fixture test.

## Curated annotations (HTML-only sources like GKE)

Edit `registry/annotations/<provider>.yaml`. Every entry needs `key`, `applies_to`, `description`, `doc_url`.
Cite the exact upstream page in `doc_url`; reviewers verify against it.

## Dev setup

```sh
cd ingest && uv sync && uv run pre-commit install
cd ../server && cargo build
cd ../web && npm ci && npm run build
```

Commit style: Conventional Commits. All CI checks are required on `main`.
