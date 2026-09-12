//! kubectl-explain style rendering over a stored JSON Schema (draft-04 dialect, local `$ref`s).

use std::collections::BTreeSet;

use serde_json::{json, Map, Value};

use crate::error::AppError;
use crate::schema::store::KindRef;

const MAX_DEREF: usize = 16;
pub const MAX_EXPLAIN_BYTES: usize = 16 * 1024;
pub const MAX_SCHEMA_BYTES: usize = 64 * 1024;

fn definitions(root: &Value) -> Option<&Map<String, Value>> {
    root.get("definitions")
        .or_else(|| root.get("$defs"))
        .and_then(Value::as_object)
}

/// Follow local `$ref`s (`#/definitions/x`) until a concrete node; merge `allOf`.
pub fn deref<'a>(root: &'a Value, node: &'a Value) -> Value {
    let mut cur = node;
    let mut hops = 0;
    while let Some(r) = cur.get("$ref").and_then(Value::as_str) {
        hops += 1;
        if hops > MAX_DEREF {
            break;
        }
        let name = r.rsplit('/').next().unwrap_or(r);
        match definitions(root).and_then(|d| d.get(name)) {
            Some(target) => cur = target,
            None => break,
        }
    }
    if let Some(all) = cur.get("allOf").and_then(Value::as_array) {
        let mut merged = cur.as_object().cloned().unwrap_or_default();
        merged.remove("allOf");
        let mut props = merged
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut required: Vec<Value> = merged
            .get("required")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for part in all {
            let p = deref(root, part);
            if let Some(o) = p.as_object() {
                for (k, v) in o {
                    match k.as_str() {
                        "properties" => {
                            if let Some(pp) = v.as_object() {
                                props.extend(pp.iter().map(|(a, b)| (a.clone(), b.clone())));
                            }
                        }
                        "required" => {
                            if let Some(r) = v.as_array() {
                                required.extend(r.iter().cloned());
                            }
                        }
                        _ => {
                            merged.entry(k.clone()).or_insert_with(|| v.clone());
                        }
                    }
                }
            }
        }
        if !props.is_empty() {
            merged.insert("properties".into(), Value::Object(props));
        }
        if !required.is_empty() {
            merged.insert("required".into(), Value::Array(required));
        }
        return Value::Object(merged);
    }
    cur.clone()
}

fn base_type(node: &Value) -> Option<String> {
    match node.get("type") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .find(|t| *t != "null")
            .map(String::from),
        _ => None,
    }
}

/// Human type label: `string`, `integer`, `Object`, `[]Object`, `map[string]string`, `int-or-string`.
pub fn type_label(root: &Value, node: &Value) -> String {
    let n = deref(root, node);
    if n.get("x-kubernetes-int-or-string").and_then(Value::as_bool) == Some(true) {
        return "int-or-string".into();
    }
    if let Some(one) = n
        .get("oneOf")
        .or_else(|| n.get("anyOf"))
        .and_then(Value::as_array)
    {
        let ts: BTreeSet<String> = one.iter().filter_map(base_type).collect();
        if ts.contains("integer") && ts.contains("string") {
            return "int-or-string".into();
        }
    }
    match base_type(&n).as_deref() {
        Some("array") => {
            let inner = n
                .get("items")
                .map(|i| type_label(root, i))
                .unwrap_or_else(|| "Object".into());
            format!("[]{inner}")
        }
        Some("object") | None if n.get("properties").is_some() => "Object".into(),
        Some("object") => match n.get("additionalProperties") {
            Some(Value::Object(_)) => {
                let inner = n
                    .get("additionalProperties")
                    .map(|i| type_label(root, i))
                    .unwrap_or_default();
                format!("map[string]{inner}")
            }
            _ => "Object".into(),
        },
        Some(t) => t.to_string(),
        None => {
            if n.get("x-kubernetes-preserve-unknown-fields").is_some() {
                "Object".into()
            } else {
                "any".into()
            }
        }
    }
}

/// Walk a dotted path (`spec.containers[].ports`) and return the (dereffed) node.
pub fn walk(root: &Value, path: &str) -> Result<Value, AppError> {
    let mut cur = deref(root, root);
    if path.trim().is_empty() {
        return Ok(cur);
    }
    let mut seen = String::new();
    for raw in path.split('.') {
        let seg = raw.trim_end_matches("[]");
        if seg.is_empty() {
            continue;
        }
        // descend through arrays transparently
        let mut node = cur;
        for _ in 0..3 {
            if base_type(&node).as_deref() == Some("array") {
                node = deref(root, node.get("items").unwrap_or(&Value::Null));
            } else {
                break;
            }
        }
        let next = node
            .get("properties")
            .and_then(|p| p.get(seg))
            .or_else(|| {
                node.get("additionalProperties").filter(|ap| {
                    ap.is_object()
                        && (seg == "*"
                            || !node.get("properties").is_some_and(|p| p.get(seg).is_some()))
                })
            })
            .cloned();
        let Some(next) = next else {
            let available: Vec<String> = node
                .get("properties")
                .and_then(Value::as_object)
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default();
            return Err(AppError::NotFound(format!(
                "field '{seg}' not found under '{}'; available: {}",
                if seen.is_empty() {
                    "<root>"
                } else {
                    seen.as_str()
                },
                if available.is_empty() {
                    "(none)".into()
                } else {
                    available.join(", ")
                }
            )));
        };
        cur = deref(root, &next);
        if !seen.is_empty() {
            seen.push('.');
        }
        seen.push_str(seg);
    }
    // unwrap trailing arrays so FIELDS of `spec.containers` shows the element fields
    for _ in 0..3 {
        if base_type(&cur).as_deref() == Some("array") {
            cur = deref(root, cur.get("items").unwrap_or(&Value::Null));
        } else {
            break;
        }
    }
    Ok(cur)
}

fn first_sentence(desc: &str) -> String {
    let d = desc.trim().replace('\n', " ");
    let end = d.find(". ").map(|i| i + 1).unwrap_or(d.len());
    let mut s: String = d[..end].to_string();
    if s.len() > 160 {
        s.truncate(157);
        s.push_str("...");
    }
    s
}

fn wrap(text: &str, indent: &str, width: usize) -> String {
    let mut out = String::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            if !line.is_empty() && line.len() + word.len() + 1 > width {
                out.push_str(indent);
                out.push_str(&line);
                out.push('\n');
                line.clear();
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        if !line.is_empty() {
            out.push_str(indent);
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

fn render_fields(root: &Value, node: &Value, depth: u8, indent: usize, out: &mut String) {
    let Some(props) = node.get("properties").and_then(Value::as_object) else {
        if let Some(Value::Object(ap)) = node.get("additionalProperties") {
            let pad = " ".repeat(indent);
            out.push_str(&format!(
                "{pad}<key> <{}>\n",
                type_label(root, &Value::Object(ap.clone()))
            ));
        }
        return;
    };
    let required: BTreeSet<&str> = node
        .get("required")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let pad = " ".repeat(indent);
    for (name, sub) in props {
        let d = deref(root, sub);
        let req = if required.contains(name.as_str()) {
            " -required-"
        } else {
            ""
        };
        out.push_str(&format!("{pad}{name}\t<{}>{req}\n", type_label(root, sub)));
        if let Some(desc) = d.get("description").and_then(Value::as_str) {
            out.push_str(&format!("{pad}  {}\n", first_sentence(desc)));
        }
        if depth > 1 && out.len() < MAX_EXPLAIN_BYTES {
            let mut inner = d.clone();
            for _ in 0..3 {
                if base_type(&inner).as_deref() == Some("array") {
                    inner = deref(root, inner.get("items").unwrap_or(&Value::Null));
                } else {
                    break;
                }
            }
            if inner.get("properties").is_some() {
                render_fields(root, &inner, depth - 1, indent + 4, out);
            }
        }
        out.push('\n');
        if out.len() > MAX_EXPLAIN_BYTES {
            out.push_str(&format!("{pad}... output truncated; narrow with `path`\n"));
            return;
        }
    }
}

pub fn render(kref: &KindRef, root: &Value, path: &str, depth: u8) -> Result<String, AppError> {
    let node = walk(root, path)?;
    let mut out = String::new();
    out.push_str(&format!("KIND:       {}\n", kref.kind));
    out.push_str(&format!("VERSION:    {}\n", kref.group_version()));
    out.push_str(&format!(
        "PROJECT:    {} {} ({}, source: {})\n",
        kref.project,
        kref.version,
        if kref.pinned {
            "pinned"
        } else {
            "unpinned latest"
        },
        kref.source
    ));
    if let Some(scope) = &kref.scope {
        out.push_str(&format!("SCOPE:      {scope}\n"));
    }
    if !path.trim().is_empty() {
        out.push_str(&format!(
            "\nFIELD:      {}\t<{}>\n",
            path.trim(),
            type_label(root, &node)
        ));
    }
    let desc = node
        .get("description")
        .and_then(Value::as_str)
        .or(kref.description.as_deref())
        .unwrap_or("<no description>");
    out.push_str("\nDESCRIPTION:\n");
    out.push_str(&wrap(desc, "    ", 88));
    if node.get("properties").is_some() || node.get("additionalProperties").is_some() {
        out.push_str("\nFIELDS:\n");
        render_fields(root, &node, depth.clamp(1, 3), 2, &mut out);
    }
    if let Some(e) = node.get("enum").and_then(Value::as_array) {
        let vals: Vec<String> = e.iter().map(|v| v.to_string()).collect();
        out.push_str(&format!("\nENUM: {}\n", vals.join(", ")));
    }
    Ok(out)
}

/// Collect names of definitions transitively referenced by `node`.
fn collect_refs(root: &Value, node: &Value, acc: &mut BTreeSet<String>) {
    match node {
        Value::Object(o) => {
            if let Some(r) = o.get("$ref").and_then(Value::as_str) {
                let name = r.rsplit('/').next().unwrap_or(r).to_string();
                if acc.insert(name.clone()) {
                    if let Some(def) = definitions(root).and_then(|d| d.get(&name)) {
                        collect_refs(root, def, acc);
                    }
                }
            }
            for v in o.values() {
                collect_refs(root, v, acc);
            }
        }
        Value::Array(a) => a.iter().for_each(|v| collect_refs(root, v, acc)),
        _ => {}
    }
}

/// Self-contained JSON Schema for the subtree at `path`. Capped at `MAX_SCHEMA_BYTES`.
pub fn subtree(root: &Value, path: &str) -> Result<Value, AppError> {
    let node = walk(root, path)?;
    let mut names = BTreeSet::new();
    collect_refs(root, &node, &mut names);
    let mut out = Map::new();
    out.insert(
        "$schema".into(),
        json!("http://json-schema.org/draft-04/schema#"),
    );
    if let Some(o) = node.as_object() {
        out.extend(o.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    if !names.is_empty() {
        if let Some(defs) = definitions(root) {
            let picked: Map<String, Value> = names
                .iter()
                .filter_map(|n| defs.get(n).map(|v| (n.clone(), v.clone())))
                .collect();
            out.insert("definitions".into(), Value::Object(picked));
        }
    }
    let v = Value::Object(out);
    let size = serde_json::to_vec(&v)?.len();
    if size > MAX_SCHEMA_BYTES {
        let props: Vec<String> = node
            .get("properties")
            .and_then(Value::as_object)
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        return Ok(json!({
            "truncated": true,
            "size_bytes": size,
            "hint": "schema too large; pass a deeper `path` (e.g. spec.containers) or use `explain`",
            "properties": props,
        }));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod() -> Value {
        json!({
            "$ref": "#/definitions/Pod",
            "definitions": {
                "Pod": {"type":"object","description":"Pod is a collection of containers.",
                    "properties": {"spec": {"$ref":"#/definitions/PodSpec"}, "metadata": {"type":"object"}},
                    "required":["spec"]},
                "PodSpec": {"type":"object","description":"PodSpec is a description of a pod.",
                    "properties": {"containers": {"type":"array","items":{"$ref":"#/definitions/Container"},"description":"List of containers."},
                                   "nodeSelector":{"type":"object","additionalProperties":{"type":"string"}},
                                   "priority":{"type":["integer","null"]}},
                    "required":["containers"]},
                "Container": {"type":"object","properties":{"name":{"type":"string","description":"Name of the container. Must be DNS_LABEL."},
                    "ports":{"type":"array","items":{"type":"object","properties":{"containerPort":{"type":"integer"},"name":{"type":"string"}},"required":["containerPort"]}},
                    "targetPort":{"x-kubernetes-int-or-string":true,"oneOf":[{"type":"integer"},{"type":"string"}]}},
                    "required":["name"]}
            }
        })
    }

    #[test]
    fn walks_paths_and_types() {
        let r = pod();
        let n = walk(&r, "spec.containers").ok();
        assert!(n.as_ref().and_then(|n| n.get("properties")).is_some());
        assert_eq!(
            type_label(&r, &json!({"$ref":"#/definitions/PodSpec"})),
            "Object"
        );
        let spec = walk(&r, "spec").ok().unwrap_or_default();
        assert_eq!(
            type_label(
                &r,
                spec.get("properties")
                    .and_then(|p| p.get("containers"))
                    .unwrap_or(&Value::Null)
            ),
            "[]Object"
        );
        assert_eq!(
            type_label(
                &r,
                spec.get("properties")
                    .and_then(|p| p.get("nodeSelector"))
                    .unwrap_or(&Value::Null)
            ),
            "map[string]string"
        );
        assert_eq!(
            type_label(
                &r,
                spec.get("properties")
                    .and_then(|p| p.get("priority"))
                    .unwrap_or(&Value::Null)
            ),
            "integer"
        );
        let c = walk(&r, "spec.containers[].targetPort")
            .ok()
            .unwrap_or_default();
        assert_eq!(type_label(&r, &c), "int-or-string");
        assert!(walk(&r, "spec.nope").is_err());
    }

    #[test]
    fn renders_and_subtree() {
        let r = pod();
        let kref = KindRef {
            kind_id: 1,
            project: "k8s".into(),
            version: "1.37.0".into(),
            pinned: true,
            source: "own".into(),
            api_group: "".into(),
            api_version: "v1".into(),
            kind: "Pod".into(),
            scope: Some("Namespaced".into()),
            description: None,
        };
        let text = render(&kref, &r, "spec.containers[].ports", 1).unwrap_or_default();
        assert!(
            text.contains("containerPort\t<integer> -required-"),
            "{text}"
        );
        assert!(text.contains("VERSION:    v1"));
        let sub = subtree(&r, "spec").unwrap_or_default();
        assert!(sub
            .get("definitions")
            .and_then(|d| d.get("Container"))
            .is_some());
        assert!(sub.get("definitions").and_then(|d| d.get("Pod")).is_none());
    }
}
