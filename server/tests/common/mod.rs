#![allow(clippy::unwrap_used, clippy::expect_used)]
#![allow(dead_code)]
//! Builds a tiny index with the exact DDL used by the Python ingest (tests/common/ddl.sql).

use std::path::PathBuf;

use rusqlite::{params, Connection};
use serde_json::json;

pub const DDL: &str = include_str!("ddl.sql");

pub fn pod_schema() -> serde_json::Value {
    json!({
        "$schema": "http://json-schema.org/draft-04/schema#",
        "$ref": "#/definitions/io.k8s.api.core.v1.Pod",
        "definitions": {
            "io.k8s.api.core.v1.Pod": {
                "type": "object", "description": "Pod is a collection of containers that can run on a host.",
                "properties": {
                    "apiVersion": {"type": "string"}, "kind": {"type": "string"},
                    "metadata": {"$ref": "#/definitions/ObjectMeta"},
                    "spec": {"$ref": "#/definitions/io.k8s.api.core.v1.PodSpec"}
                }
            },
            "ObjectMeta": {"type": "object", "properties": {"name": {"type": "string"}, "labels": {"type": "object", "additionalProperties": {"type": "string"}}}},
            "io.k8s.api.core.v1.PodSpec": {
                "type": "object", "description": "PodSpec is a description of a pod.",
                "required": ["containers"],
                "properties": {
                    "containers": {"type": "array", "description": "List of containers belonging to the pod. Cannot be updated.",
                        "items": {"$ref": "#/definitions/io.k8s.api.core.v1.Container"}},
                    "restartPolicy": {"type": "string", "enum": ["Always", "OnFailure", "Never"]},
                    "topologySpreadConstraints": {"type": "array", "items": {"type": "object", "properties": {"maxSkew": {"type": "integer"}}}}
                }
            },
            "io.k8s.api.core.v1.Container": {
                "type": "object", "required": ["name"],
                "properties": {
                    "name": {"type": "string", "description": "Name of the container specified as a DNS_LABEL. Each container in a pod must have a unique name."},
                    "image": {"type": "string"},
                    "ports": {"type": "array", "items": {"type": "object", "required": ["containerPort"],
                        "properties": {"containerPort": {"type": "integer"}, "name": {"type": "string"}}}}
                }
            }
        }
    })
}

pub fn cnp_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "description": "CiliumNetworkPolicy is a Kubernetes third-party resource for network policies.",
        "properties": {
            "apiVersion": {"type": "string"}, "kind": {"type": "string"},
            "metadata": {"type": "object", "x-kubernetes-preserve-unknown-fields": true},
            "spec": {"type": "object", "properties": {
                "endpointSelector": {"type": "object", "x-kubernetes-preserve-unknown-fields": true},
                "egress": {"type": "array", "items": {"type": "object", "properties": {"toFQDNs": {"type": "array", "items": {"type": "object", "properties": {"matchName": {"type": "string"}}}}}}}
            }}
        }
    })
}

pub fn build_fixture() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kd-fixture-{}-{}",
        std::process::id(),
        rand_suffix()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("index.sqlite");
    let conn = Connection::open(&path).expect("open");
    conn.execute_batch(DDL).expect("ddl");
    conn.execute("INSERT INTO meta(key,value) VALUES ('schema_version','1'),('build_date','2026-09-12T00:00:00Z')", [])
        .expect("meta");
    conn.execute(
        "INSERT INTO projects(id,slug,name,category,homepage,code_license,docs_license,attribution) VALUES \
         (1,'k8s','Kubernetes','core','https://kubernetes.io','Apache-2.0','CC-BY-4.0','k8s authors'), \
         (2,'cilium','Cilium','cncf','https://cilium.io','Apache-2.0','Apache-2.0','cilium authors'), \
         (3,'gke','GKE','cloud','https://cloud.google.com/kubernetes-engine','','CC-BY-4.0','google')",
        [],
    )
    .expect("projects");
    conn.execute(
        "INSERT INTO versions(id,project_id,version,git_tag,pinned,source,indexed_at,stale) VALUES \
         (1,1,'1.37.0','v1.37.0',1,'own','2026-09-12T00:00:00Z',0), \
         (2,1,'1.36.4','v1.36.4',1,'own','2026-09-12T00:00:00Z',0), \
         (3,2,'1.20.1','v1.20.1',1,'own','2026-09-12T00:00:00Z',0), \
         (4,3,'latest',NULL,0,'kubeschemas','2026-09-12T00:00:00Z',0)",
        [],
    )
    .expect("versions");
    let z = |v: &serde_json::Value| {
        zstd::encode_all(serde_json::to_vec(v).expect("json").as_slice(), 3).expect("zstd")
    };
    let pod = z(&pod_schema());
    let cnp = z(&cnp_schema());
    // Pod in both k8s versions; NetworkPolicy in core; CiliumNetworkPolicy in cilium; a GKE BackendConfig unpinned.
    for (id, vid, group, apiv, kind, blob) in [
        (1, 1, "", "v1", "Pod", &pod),
        (2, 2, "", "v1", "Pod", &pod),
        (3, 1, "networking.k8s.io", "v1", "NetworkPolicy", &cnp),
        (4, 3, "cilium.io", "v2", "CiliumNetworkPolicy", &cnp),
        (5, 3, "cilium.io", "v2alpha1", "CiliumNetworkPolicy", &cnp),
        (6, 4, "cloud.google.com", "v1", "BackendConfig", &cnp),
        (7, 4, "cloud.google.com", "v1beta1", "BackendConfig", &cnp),
    ] {
        conn.execute(
            "INSERT INTO kinds(id,version_id,api_group,api_version,kind,scope,description,schema_zstd) VALUES (?,?,?,?,?,'Namespaced','desc',?)",
            params![id, vid, group, apiv, kind, blob],
        )
        .expect("kind");
    }
    conn.execute(
        "INSERT INTO fields(kind_id,kind,path,type,required,description) VALUES \
         (1,'Pod','spec.containers','[]Object',1,'List of containers belonging to the pod.'), \
         (1,'Pod','spec.topologySpreadConstraints','[]Object',0,'TopologySpreadConstraints describes how a group of pods ought to spread across topology domains.'), \
         (4,'CiliumNetworkPolicy','spec.egress[].toFQDNs','[]Object',0,'toFQDNs is a list of rules matching fully qualified domain names.')",
        [],
    )
    .expect("fields");
    conn.execute(
        "INSERT INTO doc_content(id,hash,title,section_path,text) VALUES \
         (1,'h1','Pod Topology Spread Constraints','Concepts > Scheduling','You can use topology spread constraints to control how Pods are spread across your cluster among failure-domains such as regions, zones, nodes.'), \
         (2,'h2','DNS based egress policy','Network Policy > Egress','Cilium toFQDNs rules allow egress to fully qualified domain names resolved via DNS proxy.')",
        [],
    )
    .expect("content");
    conn.execute(
        "INSERT INTO docs(id,version_id,content_id,url,license,ord) VALUES \
         (1,1,1,'https://kubernetes.io/docs/concepts/scheduling-eviction/topology-spread-constraints/','CC-BY-4.0',0), \
         (2,2,1,'https://kubernetes.io/docs/concepts/scheduling-eviction/topology-spread-constraints/','CC-BY-4.0',0), \
         (3,3,2,'https://docs.cilium.io/en/v1.20/security/policy/language/#dns-based','Apache-2.0',0)",
        [],
    )
    .expect("docs");
    conn.execute(
        "INSERT INTO annotations(project_id,key,type,applies_to_json,value_type,description,doc_url,source) VALUES \
         (3,'cloud.google.com/neg','annotation','[\"Service\"]','json','Enables container-native load balancing with network endpoint groups for a Service.','https://docs.cloud.google.com/kubernetes-engine/docs/how-to/standalone-neg','curated'), \
         (3,'networking.gke.io/load-balancer-type','annotation','[\"Service\"]','enum','Set to Internal to create an internal passthrough load balancer.','https://docs.cloud.google.com/kubernetes-engine/docs/concepts/service-load-balancer-parameters','curated'), \
         (1,'kubernetes.io/ingress.class','annotation','[\"Ingress\"]','string','Deprecated: selects the ingress controller. Use ingressClassName instead.','https://kubernetes.io/docs/reference/labels-annotations-taints/','auto')",
        [],
    )
    .expect("annotations");
    for t in ["fields_fts", "docs_fts", "annotations_fts"] {
        conn.execute(&format!("INSERT INTO {t}({t}) VALUES('rebuild')"), [])
            .expect("fts");
    }
    drop(conn);
    path
}

fn rand_suffix() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}
