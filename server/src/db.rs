//! Read-only SQLite index access: small connection pool, hot-swappable handle, FTS sanitizer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwapOption;
use rusqlite::{Connection, OpenFlags};

use crate::error::AppError;

pub const MAX_QUERY_CHARS: usize = 256;
const POOL_SIZE: usize = 2;

pub struct Index {
    path: PathBuf,
    pool: Mutex<Vec<Connection>>,
    pub meta: HashMap<String, String>,
    pub size_bytes: u64,
}

/// `None` until the first index is opened (healthz reports 503 meanwhile).
pub type SharedIndex = Arc<ArcSwapOption<Index>>;

pub fn new_shared() -> SharedIndex {
    Arc::new(ArcSwapOption::from(None))
}

pub fn current(shared: &SharedIndex) -> Result<Arc<Index>, AppError> {
    shared.load_full().ok_or(AppError::NotReady)
}

impl Index {
    pub fn open(path: &Path) -> Result<Self, AppError> {
        let size_bytes = std::fs::metadata(path)?.len();
        let first = Self::connect(path)?;
        let mut meta = HashMap::new();
        {
            let mut stmt = first.prepare("SELECT key, value FROM meta")?;
            let rows =
                stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (k, v) = row?;
                meta.insert(k, v);
            }
        }
        let mut pool = Vec::with_capacity(POOL_SIZE);
        pool.push(first);
        while pool.len() < POOL_SIZE {
            pool.push(Self::connect(path)?);
        }
        Ok(Self {
            path: path.to_path_buf(),
            pool: Mutex::new(pool),
            meta,
            size_bytes,
        })
    }

    fn connect(path: &Path) -> Result<Connection, AppError> {
        let uri = format!("file:{}?immutable=1", path.display());
        let conn = Connection::open_with_flags(
            uri,
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.execute_batch("PRAGMA cache_size=-16000; PRAGMA mmap_size=0; PRAGMA query_only=1;")?;
        Ok(conn)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Run a closure with a pooled connection (blocking; call inside `spawn_blocking`).
    pub fn with_conn<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let conn = {
            let mut pool = self
                .pool
                .lock()
                .map_err(|_| anyhow::anyhow!("pool poisoned"))?;
            pool.pop()
        };
        let conn = match conn {
            Some(c) => c,
            None => Self::connect(&self.path)?,
        };
        let out = f(&conn);
        if let Ok(mut pool) = self.pool.lock() {
            if pool.len() < POOL_SIZE {
                pool.push(conn);
            }
        }
        out
    }

    pub fn meta_str(&self, key: &str) -> Option<&str> {
        self.meta.get(key).map(String::as_str)
    }
}

/// Convert a user query into a safe FTS5 MATCH expression.
///
/// Each whitespace-separated term is stripped to `[A-Za-z0-9._/-]`, double-quoted (so
/// FTS operators cannot be injected) and joined with implicit AND. A trailing `*` keeps
/// prefix semantics.
pub fn fts_query(user: &str) -> Result<String, AppError> {
    if user.chars().count() > MAX_QUERY_CHARS {
        return Err(AppError::InvalidInput(format!(
            "query longer than {MAX_QUERY_CHARS} chars"
        )));
    }
    let mut terms = Vec::new();
    for raw in user.split_whitespace() {
        let prefix = raw.ends_with('*');
        let cleaned: String = raw
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
            .collect();
        if cleaned.is_empty() {
            continue;
        }
        let quoted = format!("\"{}\"", cleaned.replace('"', ""));
        terms.push(if prefix { format!("{quoted}*") } else { quoted });
    }
    if terms.is_empty() {
        return Err(AppError::InvalidInput(
            "query has no searchable terms".into(),
        ));
    }
    Ok(terms.join(" "))
}

/// Escape `%` and `_` for a LIKE prefix match with `ESCAPE '\'`.
pub fn like_prefix(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 1);
    for c in s.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// Natural version ordering key: numeric segments of `1.37.0` / `v0.94.1`.
pub fn version_key(v: &str) -> Vec<u64> {
    v.trim_start_matches('v')
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_fts() {
        assert_eq!(fts_query("foo bar").ok(), Some("\"foo\" \"bar\"".into()));
        assert_eq!(
            fts_query("topo* \"OR\" x(y)").ok(),
            Some("\"topo\"* \"OR\" \"xy\"".into())
        );
        assert_eq!(
            fts_query("alb.ingress.kubernetes.io/scheme").ok(),
            Some("\"alb.ingress.kubernetes.io/scheme\"".into())
        );
        assert!(fts_query("   ").is_err());
        assert!(fts_query(&"a".repeat(300)).is_err());
    }

    #[test]
    fn versions_sort_naturally() {
        assert!(version_key("1.10.0") > version_key("1.9.11"));
        assert!(version_key("v0.94.0") > version_key("0.93.1"));
    }
}
