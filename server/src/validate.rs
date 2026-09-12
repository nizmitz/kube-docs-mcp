//! Manifest validation: multi-doc YAML -> JSON -> jsonschema draft-04 per resolved kind.

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use jsonschema::{Draft, Validator};
use lru::LruCache;
use rusqlite::Connection;
use serde::Serialize;
use serde_json::{json, Value};

use crate::error::AppError;
use crate::schema::store::{self, KindQuery, KindRef, SchemaCache};

pub const MAX_INPUT_BYTES: usize = 512 * 1024;
const MAX_ERRORS_PER_DOC: usize = 50;

#[derive(Debug, Serialize)]
pub struct DocResult {
    pub index: usize,
    #[serde(rename = "apiVersion", skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<SchemaInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<FieldError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SchemaInfo {
    pub project: String,
    pub version: String,
    pub pinned: bool,
    pub source: String,
    pub api_group: String,
    pub api_version: String,
    pub kind: String,
}

impl From<&KindRef> for SchemaInfo {
    fn from(k: &KindRef) -> Self {
        Self {
            project: k.project.clone(),
            version: k.version.clone(),
            pinned: k.pinned,
            source: k.source.clone(),
            api_group: k.api_group.clone(),
            api_version: k.api_version.clone(),
            kind: k.kind.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct FieldError {
    pub path: String,
    pub message: String,
    pub schema_path: String,
}

pub struct ValidatorCache {
    inner: Mutex<LruCache<(i64, bool), Arc<Validator>>>,
}

impl Default for ValidatorCache {
    fn default() -> Self {
        Self {
            inner: Mutex::new(LruCache::new(
                NonZeroUsize::new(32).unwrap_or(NonZeroUsize::MIN),
            )),
        }
    }
}

impl ValidatorCache {
    pub fn clear(&self) {
        if let Ok(mut g) = self.inner.lock() {
            g.clear();
        }
    }
}

/// Parse a YAML stream into JSON documents with DoS budgets.
pub fn parse_yaml(input: &str) -> Result<Vec<Value>, AppError> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(AppError::InvalidInput(format!(
            "manifest larger than {MAX_INPUT_BYTES} bytes"
        )));
    }
    let opts = serde_saphyr::options! {
        with_snippet: false,
        budget: serde_saphyr::budget! {
            max_depth: 64,
            max_anchors: 100,
            max_aliases: 1000,
            max_documents: 200,
            max_nodes: 200_000,
            max_total_scalar_bytes: 2 * MAX_INPUT_BYTES,
        },
    };
    serde_saphyr::from_multiple_with_options::<Value>(input, opts)
        .map_err(|e| AppError::InvalidInput(format!("YAML parse error: {e}")))
}

/// Set `additionalProperties: false` on object nodes that neither declare it nor preserve
/// unknown fields (mirrors kubeconform `-strict`).
pub fn strictify(node: &mut Value) {
    let Some(obj) = node.as_object_mut() else {
        if let Some(a) = node.as_array_mut() {
            a.iter_mut().for_each(strictify);
        }
        return;
    };
    let is_object = obj.get("type").and_then(Value::as_str) == Some("object")
        || obj
            .get("type")
            .and_then(Value::as_array)
            .is_some_and(|a| a.iter().any(|t| t == "object"))
        || obj.contains_key("properties");
    let preserve = obj
        .get("x-kubernetes-preserve-unknown-fields")
        .and_then(Value::as_bool)
        == Some(true);
    if is_object
        && !preserve
        && !obj.contains_key("additionalProperties")
        && obj.contains_key("properties")
    {
        obj.insert("additionalProperties".into(), Value::Bool(false));
    }
    for key in ["properties", "definitions", "$defs", "patternProperties"] {
        if let Some(Value::Object(children)) = obj.get_mut(key) {
            children.values_mut().for_each(strictify);
        }
    }
    for key in ["items", "additionalProperties", "not"] {
        if let Some(child) = obj.get_mut(key) {
            if child.is_object() {
                strictify(child);
            }
        }
    }
    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(Value::Array(children)) = obj.get_mut(key) {
            children.iter_mut().for_each(strictify);
        }
    }
}

fn compile(schema: &Value, strict: bool) -> Result<Validator, AppError> {
    let mut s = schema.clone();
    if strict {
        strictify(&mut s);
    }
    jsonschema::options()
        .with_draft(Draft::Draft4)
        .should_validate_formats(false)
        .should_ignore_unknown_formats(true)
        .build(&s)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("schema compile failed: {e}")))
}

fn get_validator(
    cache: &ValidatorCache,
    schemas: &SchemaCache,
    conn: &Connection,
    kind_id: i64,
    strict: bool,
) -> Result<Arc<Validator>, AppError> {
    if let Ok(mut g) = cache.inner.lock() {
        if let Some(v) = g.get(&(kind_id, strict)) {
            return Ok(Arc::clone(v));
        }
    }
    let schema = schemas.get_or_load(conn, kind_id)?;
    let v = Arc::new(compile(&schema, strict)?);
    if let Ok(mut g) = cache.inner.lock() {
        g.put((kind_id, strict), Arc::clone(&v));
    }
    Ok(v)
}

fn resolve_for_doc(
    conn: &Connection,
    api_version: &str,
    kind: &str,
    k8s_version: Option<&str>,
) -> Result<Option<KindRef>, AppError> {
    let (group, version) = store::split_api_version(api_version);
    let q = KindQuery {
        kind: kind.to_string(),
        group: Some(group),
        api_version: Some(version),
        project: None,
        version: None,
    };
    let refs = store::resolve(conn, &q)?;
    if refs.is_empty() {
        return Ok(None);
    }
    if let Some(kv) = k8s_version {
        if let Some(r) = refs.iter().find(|r| r.project == "k8s" && r.version == kv) {
            return Ok(Some(r.clone()));
        }
        if refs.iter().any(|r| r.project == "k8s") {
            // requested k8s version not indexed: fall back to best available but say so via version field
        }
    }
    Ok(refs.into_iter().next())
}

pub fn validate_documents(
    conn: &Connection,
    schemas: &SchemaCache,
    validators: &ValidatorCache,
    docs: Vec<Value>,
    k8s_version: Option<&str>,
    strict: bool,
) -> Result<Value, AppError> {
    let mut results = Vec::with_capacity(docs.len());
    let (mut valid, mut invalid, mut unknown, mut errors) = (0, 0, 0, 0);
    for (index, doc) in docs.into_iter().enumerate() {
        if doc.is_null() {
            continue;
        }
        let Some(obj) = doc.as_object() else {
            errors += 1;
            results.push(DocResult {
                index,
                api_version: None,
                kind: None,
                name: None,
                status: "error",
                schema: None,
                errors: vec![],
                message: Some("document is not a mapping".into()),
            });
            continue;
        };
        let api_version = obj
            .get("apiVersion")
            .and_then(Value::as_str)
            .map(String::from);
        let kind = obj.get("kind").and_then(Value::as_str).map(String::from);
        let name = obj
            .get("metadata")
            .and_then(|m| m.get("name"))
            .and_then(Value::as_str)
            .map(String::from);
        let (Some(av), Some(k)) = (api_version.clone(), kind.clone()) else {
            errors += 1;
            results.push(DocResult {
                index,
                api_version,
                kind,
                name,
                status: "error",
                schema: None,
                errors: vec![],
                message: Some("missing apiVersion or kind".into()),
            });
            continue;
        };
        let Some(kref) = resolve_for_doc(conn, &av, &k, k8s_version)? else {
            unknown += 1;
            results.push(DocResult {
                index,
                api_version,
                kind,
                name,
                status: "unknown_kind",
                schema: None,
                errors: vec![],
                message: Some(format!(
                    "no schema indexed for {av}/{k}; try list_kinds or search_fields"
                )),
            });
            continue;
        };
        let validator = get_validator(validators, schemas, conn, kref.kind_id, strict)?;
        let mut errs: Vec<FieldError> = validator
            .iter_errors(&doc)
            .take(MAX_ERRORS_PER_DOC)
            .map(|e| FieldError {
                path: e.instance_path().to_string(),
                message: e.to_string(),
                schema_path: e.schema_path().to_string(),
            })
            .collect();
        errs.sort_by(|a, b| a.path.cmp(&b.path));
        let status = if errs.is_empty() {
            valid += 1;
            "valid"
        } else {
            invalid += 1;
            "invalid"
        };
        let mut message = None;
        if let Some(kv) = k8s_version {
            if kref.project == "k8s" && kref.version != kv {
                message = Some(format!(
                    "k8s version {kv} not indexed; validated against {}",
                    kref.version
                ));
            }
        }
        results.push(DocResult {
            index,
            api_version,
            kind,
            name,
            status,
            schema: Some(SchemaInfo::from(&kref)),
            errors: errs,
            message,
        });
    }
    Ok(json!({
        "strict": strict,
        "summary": {"valid": valid, "invalid": invalid, "unknown_kind": unknown, "error": errors},
        "documents": results,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multi_doc_and_rejects_big() {
        let docs = parse_yaml("a: 1\n---\nb: [1,2]\n---\n").unwrap_or_default();
        assert_eq!(docs.len(), 2);
        assert!(parse_yaml(&"x".repeat(MAX_INPUT_BYTES + 1)).is_err());
        assert!(parse_yaml("a: [unclosed").is_err());
    }

    #[test]
    fn strictify_adds_additional_properties() {
        let mut s = json!({"type":"object","properties":{"a":{"type":"object","properties":{"b":{"type":"string"}}},
            "free":{"type":"object","x-kubernetes-preserve-unknown-fields":true,"properties":{}}}});
        strictify(&mut s);
        assert_eq!(s["additionalProperties"], json!(false));
        assert_eq!(s["properties"]["a"]["additionalProperties"], json!(false));
        assert!(s["properties"]["free"]
            .get("additionalProperties")
            .is_none());
    }

    #[test]
    fn compile_and_validate_draft4() {
        let s = json!({"$schema":"http://json-schema.org/draft-04/schema#","$ref":"#/definitions/X",
            "definitions":{"X":{"type":"object","required":["a"],"properties":{"a":{"type":"integer"}}}}});
        let v = compile(&s, true).ok();
        assert!(v.is_some());
        let v = v.unwrap_or_else(|| unreachable!());
        assert!(v.is_valid(&json!({"a":1})));
        let errs: Vec<String> = v
            .iter_errors(&json!({"a":"x","zz":1}))
            .map(|e| e.instance_path().to_string())
            .collect();
        assert!(errs.iter().any(|p| p == "/a"), "{errs:?}");
        assert!(errs.iter().any(|p| p.is_empty()), "{errs:?}"); // additionalProperties at root
    }
}
