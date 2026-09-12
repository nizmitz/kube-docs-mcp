# Index format (schema_version 1)

Single SQLite file, opened read-only by the server (`immutable=1`). Built by `ingest` in CI.
DDL source of truth: `ingest/ingest/db.py`.

## Tables

| table | purpose |
|---|---|
| `meta(key,value)` | `schema_version`, `build_date` (RFC3339), `ingest_sha`, `registry_sha` |
| `projects` | one per registry slug. `category` ∈ core/cncf/cloud/controller/vendor/bootstrap |
| `versions` | per project version. `pinned=1` = built from a git tag; `pinned=0` = unpinned "latest" (bootstrap). `source` ∈ own/kubeschemas. `stale=1` = shard reused from previous release because this build failed |
| `kinds` | one per (version, group, apiVersion, kind). `schema_zstd` = zstd-compressed JSON Schema (draft-04 dialect, self-contained, may contain `definitions` + local `$ref`s). `api_group=''` for core group |
| `fields` | flattened field tree per kind: `path` like `spec.containers[].ports[].containerPort`; `type` like `string`, `integer`, `object`, `[]object`, `map[string]string`, `int-or-string`; depth-limited (6), cycles cut |
| `fields_fts` | FTS5 external-content on `fields(kind, path, description)`, `rowid = fields.id` |
| `doc_content` | deduplicated section bodies. `hash = sha256(title\0section_path\0text\0)` |
| `docs_fts` | FTS5 external-content on `doc_content(title, section_path, text)`, `rowid = doc_content.id`, porter stemming |
| `docs` | (version, content, url, license, ord). Many `docs` rows may share one `content_id` (same text in two versions) |
| `annotations` | catalog rows per project. `applies_to_json` = JSON array of Kind names (`["Service"]`, `["*"]`). `source` ∈ auto/curated. `type` ∈ annotation/label/taint |
| `annotations_fts` | FTS5 external-content on `annotations(key, description)` |

## Query patterns (server)

```sql
-- docs search, optional project/version filter
SELECT d.id, dc.title, dc.section_path, d.url, bm25(docs_fts) AS rank,
       snippet(docs_fts, 2, '[', ']', '…', 24) AS snip
FROM docs_fts JOIN doc_content dc ON dc.id = docs_fts.rowid
JOIN docs d ON d.content_id = dc.id JOIN versions v ON v.id = d.version_id
JOIN projects p ON p.id = v.project_id
WHERE docs_fts MATCH ?1 AND (?2 IS NULL OR p.slug = ?2) AND (?3 IS NULL OR v.version = ?3)
ORDER BY rank LIMIT ?4;

-- kind lookup (prefer pinned + newest version)
SELECT k.*, v.version, v.pinned, v.source, p.slug FROM kinds k
JOIN versions v ON v.id = k.version_id JOIN projects p ON p.id = v.project_id
WHERE k.kind = ?1 COLLATE NOCASE AND (?2 IS NULL OR k.api_group = ?2)
ORDER BY v.pinned DESC, v.version DESC;
```

FTS query strings from users must be sanitized: wrap each term in double quotes, join with space (implicit AND),
support trailing `*` prefix. Never pass raw user input to `MATCH`.

## manifest.json (GitHub Release asset)

```json
{
  "schema_version": 1,
  "build_date": "2026-09-12T10:00:00Z",
  "index": { "file": "index.sqlite.zst", "sha256": "<hex of the .zst>", "size": 123, "uncompressed_size": 456, "uncompressed_sha256": "<hex>" },
  "ingest_sha": "<git sha>", "registry_sha": "<git sha>",
  "projects": [ { "slug": "k8s", "versions": [ { "version": "1.37.0", "pinned": true, "stale": false } ] } ]
}
```
Server flow: GET manifest → compare `uncompressed_sha256` with current → download `.zst` → verify `sha256` → decompress
to `index.sqlite.tmp` → verify `uncompressed_sha256` → rename over `index.sqlite` → reopen pool.
