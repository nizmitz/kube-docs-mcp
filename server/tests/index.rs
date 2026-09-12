#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;

use kube_docs_mcp::db::{self, Index};
use kube_docs_mcp::error::AppError;
use kube_docs_mcp::mcp::tools::Caches;
use kube_docs_mcp::schema::{explain, store};
use kube_docs_mcp::search;
use kube_docs_mcp::validate;

fn open() -> Index {
    Index::open(&common::build_fixture()).expect("open index")
}

#[test]
fn fts5_is_compiled_in() {
    let idx = open();
    let n: i64 = idx
        .with_conn(|c| {
            Ok(
                c.query_row("SELECT sqlite_compileoption_used('ENABLE_FTS5')", [], |r| {
                    r.get(0)
                })?,
            )
        })
        .expect("query");
    assert_eq!(n, 1);
    assert_eq!(idx.meta_str("schema_version"), Some("1"));
}

#[test]
fn index_is_read_only() {
    let idx = open();
    let err = idx.with_conn(|c| Ok(c.execute("INSERT INTO meta VALUES ('x','y')", [])?));
    assert!(err.is_err());
}

#[test]
fn resolves_and_prefers_newest_pinned() {
    let idx = open();
    let r = idx
        .with_conn(|c| store::pick("Pod", store::resolve(c, &store::parse_kind_input("Pod")?)?))
        .expect("pod");
    assert_eq!(
        (
            r.project.as_str(),
            r.version.as_str(),
            r.api_version.as_str()
        ),
        ("k8s", "1.37.0", "v1")
    );
    let r = idx
        .with_conn(|c| {
            let mut q = store::parse_kind_input("pods")?;
            q.version = Some("1.36.4".into());
            store::pick("pods", store::resolve(c, &q)?)
        })
        .expect("plural + version");
    assert_eq!(r.version, "1.36.4");
    // CRD with two served versions: stable wins
    let r = idx
        .with_conn(|c| {
            store::pick(
                "CiliumNetworkPolicy",
                store::resolve(c, &store::parse_kind_input("CiliumNetworkPolicy")?)?,
            )
        })
        .expect("cnp");
    assert_eq!(r.api_version, "v2");
    // unpinned only
    let r = idx
        .with_conn(|c| {
            store::pick(
                "BackendConfig",
                store::resolve(c, &store::parse_kind_input("BackendConfig")?)?,
            )
        })
        .expect("backendconfig");
    assert!(!r.pinned);
    assert_eq!(r.source, "kubeschemas");
}

#[test]
fn ambiguity_and_not_found() {
    let idx = open();
    // Insert nothing new: NetworkPolicy exists only in k8s, but a bare 'NetworkPolicy' vs
    // 'CiliumNetworkPolicy' are distinct kinds, so no ambiguity here. Simulate ambiguity by
    // asking for a kind that lives in two projects: BackendConfig is single-project as well, so
    // craft the ambiguous set manually via pick().
    let refs = idx
        .with_conn(|c| {
            let mut a = store::resolve(c, &store::parse_kind_input("Pod")?)?;
            let b = store::resolve(c, &store::parse_kind_input("BackendConfig")?)?;
            a.extend(b);
            Ok(a)
        })
        .expect("refs");
    match store::pick("Pod", refs) {
        Err(AppError::Ambiguous { candidates, .. }) => assert_eq!(candidates.len(), 2),
        other => panic!("expected ambiguous, got {other:?}"),
    }
    let missing = idx.with_conn(|c| {
        store::pick(
            "Nope",
            store::resolve(c, &store::parse_kind_input("Nope")?)?,
        )
    });
    assert!(matches!(missing, Err(AppError::NotFound(_))));
}

#[test]
fn explain_walks_real_schema() {
    let idx = open();
    let caches = Caches::default();
    let text = idx
        .with_conn(|c| {
            let r = store::pick("Pod", store::resolve(c, &store::parse_kind_input("Pod")?)?)?;
            let s = caches.schemas.get_or_load(c, r.kind_id)?;
            explain::render(&r, &s, "spec.containers[].ports", 1)
        })
        .expect("explain");
    assert!(text.contains("KIND:       Pod"));
    assert!(
        text.contains("containerPort\t<integer> -required-"),
        "{text}"
    );
    let top = idx
        .with_conn(|c| {
            let r = store::pick("Pod", store::resolve(c, &store::parse_kind_input("Pod")?)?)?;
            let s = caches.schemas.get_or_load(c, r.kind_id)?;
            explain::render(&r, &s, "", 2)
        })
        .expect("explain top");
    assert!(top.contains("spec\t<Object>"));
    assert!(top.contains("containers\t<[]Object> -required-"), "{top}");
    let sub = idx
        .with_conn(|c| {
            let r = store::pick("Pod", store::resolve(c, &store::parse_kind_input("Pod")?)?)?;
            let s = caches.schemas.get_or_load(c, r.kind_id)?;
            explain::subtree(&s, "spec.containers")
        })
        .expect("subtree");
    assert_eq!(sub["properties"]["ports"]["type"], "array");
    assert!(
        sub.get("definitions").is_none()
            || sub["definitions"].get("io.k8s.api.core.v1.Pod").is_none()
    );
}

fn validate(yaml: &str, k8s: Option<&str>, strict: bool) -> serde_json::Value {
    let idx = open();
    let caches = Caches::default();
    let docs = validate::parse_yaml(yaml).expect("yaml");
    idx.with_conn(|c| {
        validate::validate_documents(c, &caches.schemas, &caches.validators, docs, k8s, strict)
    })
    .expect("validate")
}

#[test]
fn validate_manifest_paths() {
    let good = "apiVersion: v1\nkind: Pod\nmetadata:\n  name: x\nspec:\n  containers:\n  - name: c\n    image: nginx\n";
    let out = validate(good, None, true);
    assert_eq!(out["summary"]["valid"], 1, "{out}");
    assert_eq!(out["documents"][0]["schema"]["version"], "1.37.0");

    let out = validate(good, Some("1.36.4"), true);
    assert_eq!(out["documents"][0]["schema"]["version"], "1.36.4");

    let missing = "apiVersion: v1\nkind: Pod\nspec: {}\n";
    let out = validate(missing, None, true);
    assert_eq!(out["summary"]["invalid"], 1);
    assert_eq!(out["documents"][0]["errors"][0]["path"], "/spec");
    assert!(out["documents"][0]["errors"][0]["message"]
        .as_str()
        .unwrap_or("")
        .contains("containers"));

    let unknown_field = "apiVersion: v1\nkind: Pod\nspec:\n  containers: []\n  bogus: 1\n";
    let strict = validate(unknown_field, None, true);
    assert_eq!(strict["summary"]["invalid"], 1, "{strict}");
    assert_eq!(strict["documents"][0]["errors"][0]["path"], "/spec");
    let lenient = validate(unknown_field, None, false);
    assert_eq!(lenient["summary"]["valid"], 1, "{lenient}");

    let multi = "apiVersion: v1\nkind: Pod\nspec:\n  containers: []\n---\napiVersion: cilium.io/v2\nkind: CiliumNetworkPolicy\nspec:\n  egress:\n  - toFQDNs:\n    - matchName: 1\n---\napiVersion: foo/v1\nkind: Nope\n---\n";
    let out = validate(multi, None, true);
    assert_eq!(out["summary"]["valid"], 1);
    assert_eq!(out["summary"]["invalid"], 1);
    assert_eq!(out["summary"]["unknown_kind"], 1);
    assert_eq!(
        out["documents"][1]["errors"][0]["path"],
        "/spec/egress/0/toFQDNs/0/matchName"
    );
    assert_eq!(out["documents"][2]["status"], "unknown_kind");

    assert!(validate::parse_yaml("a: [1,\n").is_err());
    assert!(validate::parse_yaml("x: &a {k: v}\nb: *a\nc: *a\n").is_ok()); // small alias use is fine
}

#[test]
fn searches() {
    let idx = open();
    let hits = idx
        .with_conn(|c| search::search_docs(c, "topology spread", None, None, None))
        .expect("docs");
    assert_eq!(
        hits.len(),
        1,
        "dedupes same content across versions: {hits:?}"
    );
    assert_eq!(hits[0].version, "1.37.0");
    assert!(
        hits[0].snippet.contains("[topology]") || hits[0].snippet.contains("[spread]"),
        "{}",
        hits[0].snippet
    );
    let doc_id = hits[0].doc_id;
    let page = idx
        .with_conn(|c| search::get_doc(c, doc_id, None))
        .expect("get_doc");
    assert!(page.text.starts_with("You can use"));
    assert!(page.next_offset.is_none());

    let hits = idx
        .with_conn(|c| search::search_docs(c, "toFQDNs", Some("cilium"), None, Some(5)))
        .expect("docs");
    assert_eq!(hits.len(), 1);
    let none = idx
        .with_conn(|c| search::search_docs(c, "toFQDNs", Some("k8s"), None, None))
        .expect("docs");
    assert!(none.is_empty());

    let f = idx
        .with_conn(|c| search::search_fields(c, "topologySpread*", None, None, None))
        .expect("fields");
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].path, "spec.topologySpreadConstraints");

    let a = idx
        .with_conn(|c| search::search_annotations(c, "cloud.google.com/", None, None, None))
        .expect("ann");
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].key, "cloud.google.com/neg");
    let a = idx
        .with_conn(|c| {
            search::search_annotations(
                c,
                "internal load balancer",
                Some("gke"),
                Some("Service"),
                None,
            )
        })
        .expect("ann");
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].key, "networking.gke.io/load-balancer-type");
    let a = idx
        .with_conn(|c| search::search_annotations(c, "ingress", None, Some("Service"), None))
        .expect("ann");
    assert!(
        a.is_empty(),
        "kind filter excludes Ingress-only keys: {a:?}"
    );
    let g = idx
        .with_conn(|c| search::get_annotation(c, "kubernetes.io/ingress.class", None))
        .expect("get");
    assert_eq!(g[0].provider, "k8s");
    assert!(idx
        .with_conn(|c| search::get_annotation(c, "nope/x", None))
        .is_err());

    let (v, kinds) = idx
        .with_conn(|c| search::list_kinds(c, "cilium", None, None))
        .expect("kinds");
    assert_eq!(v, "1.20.1");
    assert_eq!(kinds.len(), 2);
    let projects = idx.with_conn(search::list_projects).expect("projects");
    assert_eq!(projects.len(), 3);
    assert!(projects
        .iter()
        .any(|p| p.slug == "gke" && !p.versions[0].pinned));
    assert!(db::fts_query("' OR 1=1 --").is_ok());
}
