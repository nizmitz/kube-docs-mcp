//! Kind resolution and schema loading.

use std::cmp::Ordering;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use lru::LruCache;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;

use crate::db::version_key;
use crate::error::AppError;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct KindRef {
    pub kind_id: i64,
    pub project: String,
    pub version: String,
    pub pinned: bool,
    pub source: String,
    pub api_group: String,
    pub api_version: String,
    pub kind: String,
    pub scope: Option<String>,
    pub description: Option<String>,
}

impl KindRef {
    pub fn group_version(&self) -> String {
        if self.api_group.is_empty() {
            self.api_version.clone()
        } else {
            format!("{}/{}", self.api_group, self.api_version)
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KindQuery {
    pub kind: String,
    pub group: Option<String>,
    pub api_version: Option<String>,
    pub project: Option<String>,
    pub version: Option<String>,
}

/// Parse `Kind`, `Kind.group`, `group/version/Kind`, `version/Kind` (core), `apiVersion:Kind`.
pub fn parse_kind_input(input: &str) -> Result<KindQuery, AppError> {
    let s = input.trim();
    if s.is_empty() || s.len() > 200 {
        return Err(AppError::InvalidInput("kind must be 1..200 chars".into()));
    }
    let mut q = KindQuery::default();
    if let Some((api_version, kind)) = s.split_once(':') {
        let (g, v) = split_api_version(api_version);
        q.kind = kind.to_string();
        q.group = Some(g);
        q.api_version = Some(v);
        return Ok(q);
    }
    let parts: Vec<&str> = s.split('/').collect();
    match parts.as_slice() {
        [kind] => {
            if let Some((k, g)) = kind.split_once('.') {
                q.kind = k.to_string();
                q.group = Some(g.to_string());
            } else {
                q.kind = kind.to_string();
            }
        }
        [version, kind] => {
            q.kind = kind.to_string();
            q.group = Some(String::new());
            q.api_version = Some((*version).to_string());
        }
        [group, version, kind] => {
            q.kind = kind.to_string();
            q.group = Some((*group).to_string());
            q.api_version = Some((*version).to_string());
        }
        _ => return Err(AppError::InvalidInput(format!("cannot parse kind '{s}'"))),
    }
    if q.kind.is_empty() {
        return Err(AppError::InvalidInput(format!("cannot parse kind '{s}'")));
    }
    Ok(q)
}

pub fn split_api_version(api_version: &str) -> (String, String) {
    match api_version.split_once('/') {
        Some((g, v)) => (g.to_string(), v.to_string()),
        None => (String::new(), api_version.to_string()),
    }
}

const SELECT: &str =
    "SELECT k.id, p.slug, v.version, v.pinned, v.source, k.api_group, k.api_version, k.kind, \
    k.scope, k.description FROM kinds k JOIN versions v ON v.id = k.version_id \
    JOIN projects p ON p.id = v.project_id";

fn row_to_ref(r: &rusqlite::Row<'_>) -> rusqlite::Result<KindRef> {
    Ok(KindRef {
        kind_id: r.get(0)?,
        project: r.get(1)?,
        version: r.get(2)?,
        pinned: r.get::<_, i64>(3)? != 0,
        source: r.get(4)?,
        api_group: r.get(5)?,
        api_version: r.get(6)?,
        kind: r.get(7)?,
        scope: r.get(8)?,
        description: r.get(9)?,
    })
}

/// All kinds matching the query, best candidate first.
pub fn resolve(conn: &Connection, q: &KindQuery) -> Result<Vec<KindRef>, AppError> {
    let sql = format!(
        "{SELECT} WHERE k.kind = ?1 COLLATE NOCASE AND (?2 IS NULL OR k.api_group = ?2) \
         AND (?3 IS NULL OR k.api_version = ?3) AND (?4 IS NULL OR p.slug = ?4) \
         AND (?5 IS NULL OR v.version = ?5)"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut refs: Vec<KindRef> = stmt
        .query_map(
            params![q.kind, q.group, q.api_version, q.project, q.version],
            row_to_ref,
        )?
        .collect::<Result<_, _>>()?;
    if refs.is_empty() && q.kind.len() > 3 && q.kind.ends_with('s') {
        // "pods" -> "Pod"
        let mut q2 = q.clone();
        q2.kind = q.kind[..q.kind.len() - 1].to_string();
        let mut stmt = conn.prepare_cached(&sql)?;
        refs = stmt
            .query_map(
                params![q2.kind, q.group, q.api_version, q.project, q.version],
                row_to_ref,
            )?
            .collect::<Result<_, _>>()?;
    }
    refs.sort_by(rank_cmp);
    Ok(refs)
}

/// Sort: pinned first, newest project version, then API version priority (v2 > v1 > v1beta1).
fn rank_cmp(a: &KindRef, b: &KindRef) -> Ordering {
    b.pinned
        .cmp(&a.pinned)
        .then_with(|| version_key(&b.version).cmp(&version_key(&a.version)))
        .then_with(|| {
            api_version_priority(&b.api_version).cmp(&api_version_priority(&a.api_version))
        })
}

/// Kubernetes API version priority: GA > beta > alpha, higher number first.
pub fn api_version_priority(v: &str) -> (u8, u64, u8, u64) {
    let rest = v.trim_start_matches('v');
    let num_end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    let major: u64 = rest[..num_end].parse().unwrap_or(0);
    let tail = &rest[num_end..];
    if tail.is_empty() {
        return (3, major, 0, 0);
    }
    let (stage, n) = if let Some(n) = tail.strip_prefix("beta") {
        (2u8, n)
    } else if let Some(n) = tail.strip_prefix("alpha") {
        (1u8, n)
    } else {
        (0u8, "")
    };
    (stage, major, 0, n.parse().unwrap_or(0))
}

/// Pick one candidate or return `Ambiguous` when candidates span different groups/projects.
pub fn pick(input: &str, refs: Vec<KindRef>) -> Result<KindRef, AppError> {
    let Some(first) = refs.first().cloned() else {
        return Err(AppError::NotFound(format!("kind '{input}' not in index")));
    };
    let distinct: std::collections::BTreeSet<(&str, &str)> = refs
        .iter()
        .map(|r| (r.project.as_str(), r.api_group.as_str()))
        .collect();
    if distinct.len() > 1 {
        // one representative per (project, group)
        let mut seen = std::collections::BTreeSet::new();
        let candidates = refs
            .into_iter()
            .filter(|r| seen.insert((r.project.clone(), r.api_group.clone())))
            .collect();
        return Err(AppError::Ambiguous {
            input: input.to_string(),
            candidates,
        });
    }
    Ok(first)
}

pub fn get_ref(conn: &Connection, kind_id: i64) -> Result<Option<KindRef>, AppError> {
    let sql = format!("{SELECT} WHERE k.id = ?1");
    Ok(conn
        .query_row(&sql, params![kind_id], row_to_ref)
        .optional()?)
}

pub fn load_schema_raw(conn: &Connection, kind_id: i64) -> Result<Value, AppError> {
    let blob: Vec<u8> = conn
        .query_row(
            "SELECT schema_zstd FROM kinds WHERE id = ?1",
            params![kind_id],
            |r| r.get(0),
        )
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("kind id {kind_id}")))?;
    let bytes = zstd::decode_all(blob.as_slice())?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub struct SchemaCache {
    inner: Mutex<LruCache<i64, Arc<Value>>>,
}

impl Default for SchemaCache {
    fn default() -> Self {
        Self::new(64)
    }
}

impl SchemaCache {
    pub fn new(cap: usize) -> Self {
        let cap = NonZeroUsize::new(cap.max(1)).unwrap_or(NonZeroUsize::MIN);
        Self {
            inner: Mutex::new(LruCache::new(cap)),
        }
    }

    pub fn get_or_load(&self, conn: &Connection, kind_id: i64) -> Result<Arc<Value>, AppError> {
        if let Ok(mut g) = self.inner.lock() {
            if let Some(v) = g.get(&kind_id) {
                return Ok(Arc::clone(v));
            }
        }
        let v = Arc::new(load_schema_raw(conn, kind_id)?);
        if let Ok(mut g) = self.inner.lock() {
            g.put(kind_id, Arc::clone(&v));
        }
        Ok(v)
    }

    /// Drop everything (called after an index swap; kind ids are not stable across builds).
    pub fn clear(&self) {
        if let Ok(mut g) = self.inner.lock() {
            g.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kind_forms() {
        let q = parse_kind_input("Pod").ok();
        assert_eq!(q.as_ref().map(|q| q.kind.as_str()), Some("Pod"));
        let q = parse_kind_input("apps/v1/Deployment").ok();
        assert_eq!(
            q.as_ref().and_then(|q| q.group.clone()),
            Some("apps".into())
        );
        assert_eq!(
            q.as_ref().and_then(|q| q.api_version.clone()),
            Some("v1".into())
        );
        let q = parse_kind_input("v1/Pod").ok();
        assert_eq!(
            q.as_ref().and_then(|q| q.group.clone()),
            Some(String::new())
        );
        let q = parse_kind_input("Certificate.cert-manager.io").ok();
        assert_eq!(
            q.as_ref().and_then(|q| q.group.clone()),
            Some("cert-manager.io".into())
        );
        let q = parse_kind_input("cilium.io/v2:CiliumNetworkPolicy").ok();
        assert_eq!(
            q.as_ref().map(|q| q.kind.as_str()),
            Some("CiliumNetworkPolicy")
        );
        assert!(parse_kind_input("a/b/c/d").is_err());
    }

    #[test]
    fn api_version_order() {
        assert!(api_version_priority("v1") > api_version_priority("v1beta1"));
        assert!(api_version_priority("v2") > api_version_priority("v1"));
        assert!(api_version_priority("v1beta2") > api_version_priority("v1beta1"));
        assert!(api_version_priority("v1beta1") > api_version_priority("v2alpha1"));
    }
}
