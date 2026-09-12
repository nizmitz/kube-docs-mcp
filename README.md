# kube-docs-mcp

Public [MCP](https://modelcontextprotocol.io) server that gives AI agents **structured** Kubernetes knowledge:

- `explain` — `kubectl explain`-style field lookup from the real OpenAPI / CRD schemas, version-pinned
- `validate_manifest` — validate YAML against those schemas (core + every indexed CRD), strict or lenient
- `search_annotations` / `get_annotation` — catalog of annotations, labels and taints: Kubernetes well-known keys, GKE / EKS / AKS, ingress and controller annotations
- `search_docs` / `get_doc` — full-text search over *reference* documentation of CNCF projects
- `list_projects` / `list_kinds` / `get_schema` / `search_fields`

Endpoint: `https://kubedocs.nizmitz.com/mcp` (Streamable HTTP, no auth, rate-limited).

```sh
claude mcp add --transport http kubedocs https://kubedocs.nizmitz.com/mcp
```

Cursor / Codex / other clients: point them at the same URL. Local/offline: download `index.sqlite.zst` from the
latest `index-*` release, decompress, run `kube-docs-mcp serve --stdio --index index.sqlite`.

## Projects

The registry lives in [`registry/projects`](registry/projects). Adding a project is a YAML file + PR — see
[CONTRIBUTING.md](CONTRIBUTING.md). Schemas come from each project's git tags (pinned versions); an unpinned
"latest" tier for hundreds more CRDs is bootstrapped from [imroc/kubeschemas](https://github.com/imroc/kubeschemas).

## Layout

| dir | what |
|---|---|
| `registry/` | project definitions, curated annotation catalogs, JSON Schemas for both |
| `ingest/` | Python pipeline (uv). Runs in GitHub Actions, produces the SQLite index |
| `server/` | Rust MCP server (rmcp + axum + rusqlite). Serves the index read-only |
| `web/` | Astro landing page, embedded into the server binary |
| `deploy/` | docker-compose, nginx, Cloudflare and droplet runbook |
| `docs/` | contracts: [index format](docs/index-format.md) |

## Privacy

The server keeps only aggregate counters (tool name, project, outcome, latency). It does not store query text,
manifests you validate, or IP addresses. The reverse proxy keeps standard access logs for 7 days.

## Licenses

Code: Apache-2.0. Indexed documentation keeps its upstream license; see `ATTRIBUTION.md` in each index release
and the `license` column on every doc row.
