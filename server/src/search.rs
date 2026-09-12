//! FTS5-backed search over docs, fields and annotations.

use std::cmp::Reverse;
use std::collections::BTreeSet;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;

use crate::db::{fts_query, like_prefix, version_key};
use crate::error::AppError;

pub const MAX_LIMIT: usize = 20;
pub const DOC_PAGE_CHARS: usize = 8 * 1024;

fn clamp_limit(limit: Option<usize>, default: usize) -> usize {
    limit.unwrap_or(default).clamp(1, MAX_LIMIT)
}

#[derive(Debug, Serialize)]
pub struct DocHit {
    pub doc_id: i64,
    pub project: String,
    pub version: String,
    pub title: String,
    pub section_path: String,
    pub url: String,
    pub snippet: String,
    pub license: String,
    pub rank: f64,
}

pub fn search_docs(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    version: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<DocHit>, AppError> {
    let limit = clamp_limit(limit, 8);
    let q = fts_query(query)?;
    // Rank content first, then pick one docs row per content (newest matching version).
    let mut stmt = conn.prepare_cached(
        "SELECT dc.id, dc.title, dc.section_path, bm25(docs_fts, 3.0, 2.0, 1.0) AS rank, \
         snippet(docs_fts, 2, '[', ']', '…', 24) FROM docs_fts JOIN doc_content dc ON dc.id = docs_fts.rowid \
         WHERE docs_fts MATCH ?1 ORDER BY rank LIMIT ?2",
    )?;
    let contents: Vec<(i64, String, String, f64, String)> = stmt
        .query_map(params![q, (limit * 4) as i64], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<Result<_, _>>()?;
    let mut rows = conn.prepare_cached(
        "SELECT d.id, p.slug, v.version, d.url, d.license FROM docs d \
         JOIN versions v ON v.id = d.version_id JOIN projects p ON p.id = v.project_id \
         WHERE d.content_id = ?1 AND (?2 IS NULL OR p.slug = ?2) AND (?3 IS NULL OR v.version = ?3)",
    )?;
    let mut hits = Vec::new();
    for (cid, title, section_path, rank, snip) in contents {
        let mut candidates: Vec<(i64, String, String, String, String)> = rows
            .query_map(params![cid, project, version], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<Result<_, _>>()?;
        if candidates.is_empty() {
            continue;
        }
        candidates.sort_by_key(|a| Reverse(version_key(&a.2)));
        let (doc_id, project, version, url, license) = candidates.swap_remove(0);
        hits.push(DocHit {
            doc_id,
            project,
            version,
            title,
            section_path,
            url,
            snippet: snip,
            license,
            rank,
        });
        if hits.len() >= limit {
            break;
        }
    }
    Ok(hits)
}

#[derive(Debug, Serialize)]
pub struct DocPage {
    pub doc_id: i64,
    pub project: String,
    pub version: String,
    pub title: String,
    pub section_path: String,
    pub url: String,
    pub license: String,
    pub offset: usize,
    pub total_chars: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
    pub text: String,
}

pub fn get_doc(conn: &Connection, doc_id: i64, offset: Option<usize>) -> Result<DocPage, AppError> {
    let row = conn
        .query_row(
            "SELECT p.slug, v.version, dc.title, dc.section_path, d.url, d.license, dc.text FROM docs d \
             JOIN doc_content dc ON dc.id = d.content_id JOIN versions v ON v.id = d.version_id \
             JOIN projects p ON p.id = v.project_id WHERE d.id = ?1",
            params![doc_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("doc {doc_id}")))?;
    let (project, version, title, section_path, url, license, text) = row;
    let total_chars = text.chars().count();
    let offset = offset.unwrap_or(0).min(total_chars);
    let page: String = text.chars().skip(offset).take(DOC_PAGE_CHARS).collect();
    let end = offset + page.chars().count();
    Ok(DocPage {
        doc_id,
        project,
        version,
        title,
        section_path,
        url,
        license,
        offset,
        total_chars,
        next_offset: (end < total_chars).then_some(end),
        text: page,
    })
}

#[derive(Debug, Serialize)]
pub struct FieldHit {
    pub project: String,
    pub version: String,
    pub api_group: String,
    pub api_version: String,
    pub kind: String,
    pub path: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

pub fn search_fields(
    conn: &Connection,
    query: &str,
    project: Option<&str>,
    version: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<FieldHit>, AppError> {
    let limit = clamp_limit(limit, 10);
    let q = fts_query(query)?;
    const COLS: &str =
        "p.slug, v.version, k.api_group, k.api_version, k.kind, f.path, f.type, f.required, \
         f.description";
    let map = |r: &rusqlite::Row<'_>| {
        Ok(FieldHit {
            project: r.get(0)?,
            version: r.get(1)?,
            api_group: r.get(2)?,
            api_version: r.get(3)?,
            kind: r.get(4)?,
            path: r.get(5)?,
            type_: r.get(6)?,
            required: r.get::<_, i64>(7)? != 0,
            description: r.get(8)?,
        })
    };
    // 1) FTS over kind/path/description (ranked).
    let sql = format!(
        "SELECT {COLS} FROM fields_fts JOIN fields f ON f.id = fields_fts.rowid \
         JOIN kinds k ON k.id = f.kind_id JOIN versions v ON v.id = k.version_id \
         JOIN projects p ON p.id = v.project_id \
         WHERE fields_fts MATCH ?1 AND (?2 IS NULL OR p.slug = ?2) AND (?3 IS NULL OR v.version = ?3) \
         ORDER BY bm25(fields_fts, 2.0, 3.0, 1.0), v.pinned DESC LIMIT ?4"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut rows: Vec<FieldHit> = stmt
        .query_map(params![q, project, version, (limit * 6) as i64], map)?
        .collect::<Result<_, _>>()?;
    // 2) Case-insensitive substring match on the dotted path (tokenizer independent), so
    //    `topologySpread*` or `containers.ports` find `spec.topologySpreadConstraints` etc.
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|t| t.trim_end_matches('*').to_ascii_lowercase())
        .filter(|t| !t.is_empty() && t.len() >= 3)
        .collect();
    if !terms.is_empty() {
        let mut where_ = String::new();
        for i in 0..terms.len() {
            where_.push_str(&format!(" AND instr(lower(f.path), ?{}) > 0", i + 4));
        }
        let sql = format!(
            "SELECT {COLS} FROM fields f JOIN kinds k ON k.id = f.kind_id \
             JOIN versions v ON v.id = k.version_id JOIN projects p ON p.id = v.project_id \
             WHERE (?1 IS NULL OR p.slug = ?1) AND (?2 IS NULL OR v.version = ?2) AND ?3 = ?3{where_} \
             ORDER BY v.pinned DESC, length(f.path) LIMIT {}",
            limit * 6
        );
        let mut stmt = conn.prepare_cached(&sql)?;
        let mut binds: Vec<rusqlite::types::Value> = vec![
            project
                .map(|s| s.to_string().into())
                .unwrap_or(rusqlite::types::Value::Null),
            version
                .map(|s| s.to_string().into())
                .unwrap_or(rusqlite::types::Value::Null),
            1i64.into(),
        ];
        binds.extend(
            terms
                .iter()
                .map(|t| rusqlite::types::Value::from(t.clone())),
        );
        let extra: Vec<FieldHit> = stmt
            .query_map(rusqlite::params_from_iter(binds), map)?
            .collect::<Result<_, _>>()?;
        rows.extend(extra);
    }
    // dedupe same (project, group, kind, path) across versions, keeping the highest-ranked
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for h in rows {
        if seen.insert((
            h.project.clone(),
            h.api_group.clone(),
            h.kind.clone(),
            h.path.clone(),
        )) {
            out.push(h);
            if out.len() >= limit {
                break;
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Serialize)]
pub struct AnnotationHit {
    pub provider: String,
    pub key: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub applies_to: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_values: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub example: Option<String>,
    pub description: String,
    pub doc_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<String>,
    pub source: String,
}

const ANN_SELECT: &str =
    "SELECT p.slug, a.key, a.type, a.applies_to_json, a.value_type, a.allowed_values_json, \
    a.example, a.description, a.doc_url, a.since, a.deprecated, a.source FROM annotations a \
    JOIN projects p ON p.id = a.project_id";

fn ann_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<AnnotationHit> {
    let applies: String = r.get(3)?;
    let allowed: Option<String> = r.get(5)?;
    Ok(AnnotationHit {
        provider: r.get(0)?,
        key: r.get(1)?,
        type_: r.get(2)?,
        applies_to: serde_json::from_str(&applies).unwrap_or(Value::Null),
        value_type: r.get(4)?,
        allowed_values: allowed.and_then(|s| serde_json::from_str(&s).ok()),
        example: r.get(6)?,
        description: r.get(7)?,
        doc_url: r.get(8)?,
        since: r.get(9)?,
        deprecated: r.get(10)?,
        source: r.get(11)?,
    })
}

pub fn search_annotations(
    conn: &Connection,
    query: &str,
    provider: Option<&str>,
    kind: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<AnnotationHit>, AppError> {
    let limit = clamp_limit(limit, 10);
    let query = query.trim();
    if query.is_empty() || query.len() > 256 {
        return Err(AppError::InvalidInput("query must be 1..256 chars".into()));
    }
    let kind_filter = kind.map(|k| format!("%\"{}\"%", k.replace(['%', '_', '"'], "")));
    let rows: Vec<AnnotationHit> = if query.contains('/')
        || query.contains('.') && !query.contains(' ')
    {
        let sql = format!(
            "{ANN_SELECT} WHERE a.key LIKE ?1 ESCAPE '\\' AND (?2 IS NULL OR p.slug = ?2) \
             AND (?3 IS NULL OR a.applies_to_json LIKE ?3 OR a.applies_to_json LIKE '%\"*\"%') \
             ORDER BY a.key LIMIT ?4"
        );
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows: Vec<AnnotationHit> = stmt
            .query_map(
                params![like_prefix(query), provider, kind_filter, limit as i64],
                ann_row,
            )?
            .collect::<Result<_, _>>()?;
        rows
    } else {
        let q = fts_query(query)?;
        let sql = "SELECT p.slug, a.key, a.type, a.applies_to_json, a.value_type, a.allowed_values_json, \
             a.example, a.description, a.doc_url, a.since, a.deprecated, a.source FROM annotations_fts \
             JOIN annotations a ON a.id = annotations_fts.rowid JOIN projects p ON p.id = a.project_id \
             WHERE annotations_fts MATCH ?1 AND (?2 IS NULL OR p.slug = ?2) \
             AND (?3 IS NULL OR a.applies_to_json LIKE ?3 OR a.applies_to_json LIKE '%\"*\"%') \
             ORDER BY bm25(annotations_fts, 3.0, 1.0) LIMIT ?4";
        let mut stmt = conn.prepare_cached(sql)?;
        let rows: Vec<AnnotationHit> = stmt
            .query_map(params![q, provider, kind_filter, limit as i64], ann_row)?
            .collect::<Result<_, _>>()?;
        rows
    };
    Ok(rows)
}

pub fn get_annotation(
    conn: &Connection,
    key: &str,
    provider: Option<&str>,
) -> Result<Vec<AnnotationHit>, AppError> {
    let key = key.trim();
    if key.is_empty() || key.len() > 256 {
        return Err(AppError::InvalidInput("key must be 1..256 chars".into()));
    }
    let sql =
        format!("{ANN_SELECT} WHERE a.key = ?1 AND (?2 IS NULL OR p.slug = ?2) ORDER BY p.slug");
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows: Vec<AnnotationHit> = stmt
        .query_map(params![key, provider], ann_row)?
        .collect::<Result<_, _>>()?;
    if rows.is_empty() {
        return Err(AppError::NotFound(format!(
            "annotation '{key}'; try search_annotations with a prefix"
        )));
    }
    Ok(rows)
}

#[derive(Debug, Serialize)]
pub struct ProjectInfo {
    pub slug: String,
    pub name: String,
    pub category: String,
    pub homepage: String,
    pub docs_license: String,
    pub versions: Vec<VersionInfo>,
}

#[derive(Debug, Serialize)]
pub struct VersionInfo {
    pub version: String,
    pub pinned: bool,
    pub source: String,
    pub stale: bool,
    pub kinds: i64,
    pub docs: i64,
}

pub fn list_projects(conn: &Connection) -> Result<Vec<ProjectInfo>, AppError> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, slug, name, category, homepage, docs_license FROM projects ORDER BY category, slug",
    )?;
    let projects: Vec<(i64, String, String, String, String, String)> = stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?
        .collect::<Result<_, _>>()?;
    let mut vstmt = conn.prepare_cached(
        "SELECT v.version, v.pinned, v.source, v.stale, \
         (SELECT COUNT(*) FROM kinds k WHERE k.version_id = v.id), \
         (SELECT COUNT(*) FROM docs d WHERE d.version_id = v.id) FROM versions v WHERE v.project_id = ?1",
    )?;
    let mut out = Vec::new();
    for (id, slug, name, category, homepage, docs_license) in projects {
        let mut versions: Vec<VersionInfo> = vstmt
            .query_map(params![id], |r| {
                Ok(VersionInfo {
                    version: r.get(0)?,
                    pinned: r.get::<_, i64>(1)? != 0,
                    source: r.get(2)?,
                    stale: r.get::<_, i64>(3)? != 0,
                    kinds: r.get(4)?,
                    docs: r.get(5)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        versions.sort_by_key(|v| Reverse(version_key(&v.version)));
        out.push(ProjectInfo {
            slug,
            name,
            category,
            homepage,
            docs_license,
            versions,
        });
    }
    Ok(out)
}

#[derive(Debug, Serialize)]
pub struct KindSummary {
    pub api_group: String,
    pub api_version: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

pub fn list_kinds(
    conn: &Connection,
    project: &str,
    version: Option<&str>,
    group: Option<&str>,
) -> Result<(String, Vec<KindSummary>), AppError> {
    let version = match version {
        Some(v) => v.to_string(),
        None => {
            let mut stmt = conn.prepare_cached(
                "SELECT v.version, v.pinned FROM versions v JOIN projects p ON p.id = v.project_id WHERE p.slug = ?1",
            )?;
            let mut vs: Vec<(String, i64)> = stmt
                .query_map(params![project], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?;
            vs.sort_by(|a, b| {
                b.1.cmp(&a.1)
                    .then_with(|| version_key(&b.0).cmp(&version_key(&a.0)))
            });
            vs.into_iter()
                .next()
                .map(|v| v.0)
                .ok_or_else(|| AppError::NotFound(format!("project '{project}'")))?
        }
    };
    let mut stmt = conn.prepare_cached(
        "SELECT k.api_group, k.api_version, k.kind, k.scope, substr(k.description, 1, 160) FROM kinds k \
         JOIN versions v ON v.id = k.version_id JOIN projects p ON p.id = v.project_id \
         WHERE p.slug = ?1 AND v.version = ?2 AND (?3 IS NULL OR k.api_group = ?3) \
         ORDER BY k.api_group, k.kind, k.api_version",
    )?;
    let kinds: Vec<KindSummary> = stmt
        .query_map(params![project, version, group], |r| {
            Ok(KindSummary {
                api_group: r.get(0)?,
                api_version: r.get(1)?,
                kind: r.get(2)?,
                scope: r.get(3)?,
                description: r.get(4)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok((version, kinds))
}
