CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS projects (
  id           INTEGER PRIMARY KEY,
  slug         TEXT NOT NULL UNIQUE,
  name         TEXT NOT NULL,
  category     TEXT NOT NULL,
  homepage     TEXT NOT NULL,
  code_license TEXT NOT NULL,
  docs_license TEXT NOT NULL,
  attribution  TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS versions (
  id         INTEGER PRIMARY KEY,
  project_id INTEGER NOT NULL REFERENCES projects(id),
  version    TEXT NOT NULL,
  git_tag    TEXT,
  pinned     INTEGER NOT NULL DEFAULT 1,
  source     TEXT NOT NULL DEFAULT 'own',
  indexed_at TEXT NOT NULL,
  stale      INTEGER NOT NULL DEFAULT 0,
  UNIQUE(project_id, version)
);
CREATE TABLE IF NOT EXISTS kinds (
  id          INTEGER PRIMARY KEY,
  version_id  INTEGER NOT NULL REFERENCES versions(id),
  api_group   TEXT NOT NULL,
  api_version TEXT NOT NULL,
  kind        TEXT NOT NULL,
  scope       TEXT,
  description TEXT,
  schema_zstd BLOB NOT NULL,
  UNIQUE(version_id, api_group, api_version, kind)
);
CREATE INDEX IF NOT EXISTS kinds_kind ON kinds(kind);
CREATE INDEX IF NOT EXISTS kinds_group_kind ON kinds(api_group, kind);
CREATE TABLE IF NOT EXISTS fields (
  id          INTEGER PRIMARY KEY,
  kind_id     INTEGER NOT NULL REFERENCES kinds(id),
  kind        TEXT NOT NULL,
  path        TEXT NOT NULL,
  type        TEXT NOT NULL,
  required    INTEGER NOT NULL DEFAULT 0,
  description TEXT,
  enum_json   TEXT
);
CREATE INDEX IF NOT EXISTS fields_kind_id ON fields(kind_id);
CREATE VIRTUAL TABLE IF NOT EXISTS fields_fts USING fts5(
  kind, path, description,
  content='fields', content_rowid='id', tokenize='porter unicode61'
);
CREATE TABLE IF NOT EXISTS doc_content (
  id           INTEGER PRIMARY KEY,
  hash         TEXT NOT NULL UNIQUE,
  title        TEXT NOT NULL,
  section_path TEXT NOT NULL,
  text         TEXT NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS docs_fts USING fts5(
  title, section_path, text,
  content='doc_content', content_rowid='id', tokenize='porter unicode61'
);
CREATE TABLE IF NOT EXISTS docs (
  id         INTEGER PRIMARY KEY,
  version_id INTEGER NOT NULL REFERENCES versions(id),
  content_id INTEGER NOT NULL REFERENCES doc_content(id),
  url        TEXT NOT NULL,
  license    TEXT NOT NULL,
  ord        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS docs_content ON docs(content_id);
CREATE INDEX IF NOT EXISTS docs_version ON docs(version_id);
CREATE TABLE IF NOT EXISTS annotations (
  id                  INTEGER PRIMARY KEY,
  project_id          INTEGER NOT NULL REFERENCES projects(id),
  key                 TEXT NOT NULL,
  type                TEXT NOT NULL DEFAULT 'annotation',
  applies_to_json     TEXT NOT NULL,
  value_type          TEXT,
  allowed_values_json TEXT,
  example             TEXT,
  description         TEXT NOT NULL,
  doc_url             TEXT NOT NULL,
  since               TEXT,
  deprecated          TEXT,
  source              TEXT NOT NULL DEFAULT 'auto',
  UNIQUE(project_id, key)
);
CREATE INDEX IF NOT EXISTS annotations_key ON annotations(key);
CREATE VIRTUAL TABLE IF NOT EXISTS annotations_fts USING fts5(
  key, description,
  content='annotations', content_rowid='id', tokenize='porter unicode61'
);
