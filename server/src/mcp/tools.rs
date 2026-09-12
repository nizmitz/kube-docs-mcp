//! The MCP tool surface (10 tools). Every tool runs its DB work in `spawn_blocking`.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo};
use rmcp::{schemars, tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::db::{self, SharedIndex};
use crate::error::AppError;
use crate::metrics::Metrics;
use crate::schema::explain;
use crate::schema::store::{self, SchemaCache};
use crate::search;
use crate::validate::{self, ValidatorCache};

pub const VALIDATE_TIMEOUT: Duration = Duration::from_secs(2);
const TOOL_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Default)]
pub struct Caches {
    pub schemas: SchemaCache,
    pub validators: ValidatorCache,
}

impl Caches {
    pub fn clear(&self) {
        self.schemas.clear();
        self.validators.clear();
    }
}

#[derive(Clone)]
pub struct KubeDocs {
    idx: SharedIndex,
    caches: Arc<Caches>,
    metrics: Arc<Metrics>,
}

// ---------- parameter types ----------

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ListKindsParams {
    /// Project slug from `list_projects` (e.g. `k8s`, `cilium`, `cert-manager`).
    pub project: String,
    /// Project version (e.g. `1.37.0`). Default: newest pinned version.
    #[serde(default)]
    pub version: Option<String>,
    /// Restrict to one API group (e.g. `apps`, `cilium.io`). Use `""` for the core group.
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ExplainParams {
    /// Resource kind. Accepts `Pod`, `apps/v1/Deployment`, `Certificate.cert-manager.io`,
    /// `cilium.io/v2:CiliumNetworkPolicy`. Ambiguous bare kinds return candidates.
    pub kind: String,
    /// Dotted field path like `spec.containers[].ports` (empty = top level).
    #[serde(default)]
    pub path: Option<String>,
    /// Project slug to disambiguate (e.g. `k8s`).
    #[serde(default)]
    pub project: Option<String>,
    /// Project version (e.g. `1.36.4`). Default: newest pinned.
    #[serde(default)]
    pub version: Option<String>,
    /// Nesting depth of the FIELDS listing, 1..3. Default 1.
    #[serde(default)]
    pub depth: Option<u8>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct GetSchemaParams {
    /// Resource kind (same forms as `explain`).
    pub kind: String,
    /// Dotted field path to return a subtree; large schemas are truncated unless narrowed.
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ValidateParams {
    /// One or more YAML documents (separated by `---`). Max 512 KB.
    pub yaml: String,
    /// Kubernetes version to validate core kinds against, e.g. `1.36.4`. Default: newest indexed.
    #[serde(default)]
    pub k8s_version: Option<String>,
    /// Reject unknown fields (like `kubeconform -strict`). Default true.
    #[serde(default)]
    pub strict: Option<bool>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    /// Search terms. Words are ANDed; a trailing `*` matches prefixes (e.g. `topologySpread*`).
    pub query: String,
    /// Restrict to a project slug.
    #[serde(default)]
    pub project: Option<String>,
    /// Restrict to a project version.
    #[serde(default)]
    pub version: Option<String>,
    /// Max results (1..20).
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct GetDocParams {
    /// `doc_id` from `search_docs`.
    pub doc_id: i64,
    /// Character offset for paging (use `next_offset` from the previous page).
    #[serde(default)]
    pub offset: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SearchAnnotationsParams {
    /// Key prefix (`alb.ingress.kubernetes.io/`, `cloud.google.com/neg`) or free words
    /// (`internal load balancer azure`).
    pub query: String,
    /// Provider/project slug: `k8s`, `gke`, `eks`, `aks`, `ingress-nginx`, ...
    #[serde(default)]
    pub provider: Option<String>,
    /// Only annotations that apply to this Kind (e.g. `Service`, `Ingress`).
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct GetAnnotationParams {
    /// Exact annotation/label key, e.g. `service.beta.kubernetes.io/aws-load-balancer-type`.
    pub key: String,
    #[serde(default)]
    pub provider: Option<String>,
}

// ---------- helpers ----------

fn json_result(v: &Value) -> CallToolResult {
    let text = serde_json::to_string_pretty(v).unwrap_or_else(|_| "{}".into());
    CallToolResult::success(vec![ContentBlock::text(text)])
}

fn error_result(e: &AppError) -> CallToolResult {
    let body = match e {
        AppError::Ambiguous { input, candidates } => json!({
            "error": "ambiguous", "message": e.to_string(), "input": input,
            "candidates": candidates.iter().map(|c| json!({
                "project": c.project, "version": c.version, "apiVersion": c.group_version(),
                "kind": c.kind, "use": format!("{}:{}", c.group_version(), c.kind)
            })).collect::<Vec<_>>()
        }),
        other => json!({ "error": other.kind(), "message": other.to_string() }),
    };
    CallToolResult::error(vec![ContentBlock::text(
        serde_json::to_string_pretty(&body).unwrap_or_default(),
    )])
}

impl KubeDocs {
    pub fn new(idx: SharedIndex, caches: Arc<Caches>, metrics: Arc<Metrics>) -> Self {
        Self {
            idx,
            caches,
            metrics,
        }
    }

    async fn run<F, Fut>(
        &self,
        tool: &'static str,
        project: Option<&str>,
        f: F,
    ) -> Result<CallToolResult, McpError>
    where
        F: FnOnce(Arc<db::Index>, Arc<Caches>) -> Fut,
        Fut: Future<Output = Result<CallToolResult, AppError>>,
    {
        let start = Instant::now();
        let outcome;
        let result = match db::current(&self.idx) {
            Err(e) => {
                outcome = e.kind();
                Err(McpError::internal_error(e.to_string(), None))
            }
            Ok(idx) => {
                match tokio::time::timeout(TOOL_TIMEOUT, f(idx, Arc::clone(&self.caches))).await {
                    Ok(Ok(r)) => {
                        outcome = "ok";
                        Ok(r)
                    }
                    Ok(Err(e)) => {
                        outcome = e.kind();
                        Ok(error_result(&e))
                    }
                    Err(_) => {
                        outcome = "timeout";
                        Ok(error_result(&AppError::Timeout(TOOL_TIMEOUT.as_secs())))
                    }
                }
            }
        };
        let label = match project {
            Some(p)
                if p.len() <= 40 && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') =>
            {
                format!("{tool}:{p}")
            }
            _ => tool.to_string(),
        };
        self.metrics.record(&label, outcome, start.elapsed());
        result
    }
}

async fn blocking<T: Send + 'static>(
    idx: Arc<db::Index>,
    f: impl FnOnce(&rusqlite::Connection) -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    tokio::task::spawn_blocking(move || idx.with_conn(f)).await?
}

fn resolve_kind(
    conn: &rusqlite::Connection,
    kind: &str,
    project: Option<&str>,
    version: Option<&str>,
) -> Result<store::KindRef, AppError> {
    let mut q = store::parse_kind_input(kind)?;
    q.project = project.map(String::from);
    q.version = version.map(String::from);
    let refs = store::resolve(conn, &q)?;
    store::pick(kind, refs)
}

// ---------- tools ----------

#[tool_router]
impl KubeDocs {
    #[tool(
        name = "list_projects",
        description = "List indexed projects (Kubernetes, CNCF projects, cloud providers) with their versions. \
        `pinned=true` versions are built from a git tag; `pinned=false` is an unpinned 'latest' snapshot. \
        Call this first when unsure which project slug or version to pass to other tools.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn list_projects(&self) -> Result<CallToolResult, McpError> {
        self.run("list_projects", None, |idx, _| async move {
            let build = idx.meta_str("build_date").unwrap_or("unknown").to_string();
            let projects = blocking(idx, search::list_projects).await?;
            Ok(json_result(
                &json!({ "build_date": build, "projects": projects }),
            ))
        })
        .await
    }

    #[tool(
        name = "list_kinds",
        description = "List resource kinds (apiVersion/Kind) available in a project version, optionally \
        filtered by API group. Use to discover CRD names before `explain`/`get_schema`.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn list_kinds(
        &self,
        Parameters(p): Parameters<ListKindsParams>,
    ) -> Result<CallToolResult, McpError> {
        let project = p.project.clone();
        let label = project.clone();
        self.run("list_kinds", Some(&label), |idx, _| async move {
            let (version, kinds) = blocking(idx, move |c| {
                search::list_kinds(c, &p.project, p.version.as_deref(), p.group.as_deref())
            })
            .await?;
            Ok(json_result(&json!({ "project": project, "version": version, "count": kinds.len(), "kinds": kinds })))
        })
        .await
    }

    #[tool(
        name = "explain",
        description = "Like `kubectl explain`: field-level documentation for any indexed kind (core Kubernetes \
        and CRDs such as CiliumNetworkPolicy, Certificate, ClusterPolicy). Returns type, description, \
        required flags and child fields at a dotted `path` (e.g. `spec.containers[].resources`). \
        Prefer this over docs search when the question is 'what fields does X have / what does field Y mean'.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn explain(
        &self,
        Parameters(p): Parameters<ExplainParams>,
    ) -> Result<CallToolResult, McpError> {
        let project = p.project.clone();
        self.run("explain", project.as_deref(), |idx, caches| async move {
            let text = blocking(idx, move |c| {
                let kref = resolve_kind(c, &p.kind, p.project.as_deref(), p.version.as_deref())?;
                let schema = caches.schemas.get_or_load(c, kref.kind_id)?;
                explain::render(
                    &kref,
                    &schema,
                    p.path.as_deref().unwrap_or(""),
                    p.depth.unwrap_or(1),
                )
            })
            .await?;
            Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
        })
        .await
    }

    #[tool(
        name = "get_schema",
        description = "Return the JSON Schema (draft-04 dialect) for a kind or a field subtree. Large schemas are \
        truncated; narrow with `path`. Use when you need exact constraints (enums, patterns, min/max) \
        rather than prose.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn get_schema(
        &self,
        Parameters(p): Parameters<GetSchemaParams>,
    ) -> Result<CallToolResult, McpError> {
        let project = p.project.clone();
        self.run("get_schema", project.as_deref(), |idx, caches| async move {
            let out = blocking(idx, move |c| {
                let kref = resolve_kind(c, &p.kind, p.project.as_deref(), p.version.as_deref())?;
                let schema = caches.schemas.get_or_load(c, kref.kind_id)?;
                let sub = explain::subtree(&schema, p.path.as_deref().unwrap_or(""))?;
                Ok(json!({
                    "kind": kref.kind, "apiVersion": kref.group_version(), "project": kref.project,
                    "version": kref.version, "pinned": kref.pinned, "source": kref.source,
                    "path": p.path.unwrap_or_default(), "schema": sub
                }))
            })
            .await?;
            Ok(json_result(&out))
        })
        .await
    }

    #[tool(
        name = "validate_manifest",
        description = "Validate Kubernetes YAML (multi-document) against the real schemas: core kinds for the \
        requested `k8s_version` and any indexed CRD (Cilium, cert-manager, Kyverno, Argo, ...). \
        Returns per-document status with JSON-pointer error paths. `strict` (default true) rejects \
        unknown fields. Max 512 KB.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn validate_manifest(
        &self,
        Parameters(p): Parameters<ValidateParams>,
    ) -> Result<CallToolResult, McpError> {
        self.run("validate_manifest", None, |idx, caches| async move {
            let strict = p.strict.unwrap_or(true);
            let docs = validate::parse_yaml(&p.yaml)?;
            let fut = blocking(idx, move |c| {
                validate::validate_documents(
                    c,
                    &caches.schemas,
                    &caches.validators,
                    docs,
                    p.k8s_version.as_deref(),
                    strict,
                )
            });
            let out = tokio::time::timeout(VALIDATE_TIMEOUT, fut)
                .await
                .map_err(|_| AppError::Timeout(VALIDATE_TIMEOUT.as_secs()))??;
            Ok(json_result(&out))
        })
        .await
    }

    #[tool(
        name = "search_fields",
        description = "Find which kinds have a field matching a name or description, across all projects \
        (e.g. `topologySpreadConstraints`, `dns01 cloudflare`). Answers 'where is field X used'.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn search_fields(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let project = p.project.clone();
        self.run("search_fields", project.as_deref(), |idx, _| async move {
            let hits = blocking(idx, move |c| {
                search::search_fields(
                    c,
                    &p.query,
                    p.project.as_deref(),
                    p.version.as_deref(),
                    p.limit,
                )
            })
            .await?;
            Ok(json_result(&json!({ "count": hits.len(), "fields": hits })))
        })
        .await
    }

    #[tool(
        name = "search_docs",
        description = "Full-text search over reference documentation of indexed projects (BM25). Returns \
        ranked sections with snippet, URL and `doc_id` for `get_doc`. Good for configuration options, \
        CLI flags, Helm values and behaviour questions. For field structure use `explain` instead.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn search_docs(
        &self,
        Parameters(p): Parameters<SearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let project = p.project.clone();
        self.run("search_docs", project.as_deref(), |idx, _| async move {
            let hits = blocking(idx, move |c| {
                search::search_docs(
                    c,
                    &p.query,
                    p.project.as_deref(),
                    p.version.as_deref(),
                    p.limit,
                )
            })
            .await?;
            Ok(json_result(
                &json!({ "count": hits.len(), "results": hits }),
            ))
        })
        .await
    }

    #[tool(
        name = "get_doc",
        description = "Read the full text of a documentation section returned by `search_docs` (8 KB pages; \
        follow `next_offset`).",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn get_doc(
        &self,
        Parameters(p): Parameters<GetDocParams>,
    ) -> Result<CallToolResult, McpError> {
        self.run("get_doc", None, |idx, _| async move {
            let page = blocking(idx, move |c| search::get_doc(c, p.doc_id, p.offset)).await?;
            Ok(json_result(&serde_json::to_value(page)?))
        })
        .await
    }

    #[tool(
        name = "search_annotations",
        description = "Search the annotations/labels/taints catalog: Kubernetes well-known keys plus cloud \
        provider and controller keys (GKE, EKS/AWS load balancer controller, AKS, ingress-nginx, ...). \
        Pass a key prefix (`alb.ingress.kubernetes.io/`) or words (`internal load balancer gke`). \
        Filter by `kind` (Service, Ingress) or `provider`.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn search_annotations(
        &self,
        Parameters(p): Parameters<SearchAnnotationsParams>,
    ) -> Result<CallToolResult, McpError> {
        let provider = p.provider.clone();
        self.run(
            "search_annotations",
            provider.as_deref(),
            |idx, _| async move {
                let hits = blocking(idx, move |c| {
                    search::search_annotations(
                        c,
                        &p.query,
                        p.provider.as_deref(),
                        p.kind.as_deref(),
                        p.limit,
                    )
                })
                .await?;
                Ok(json_result(
                    &json!({ "count": hits.len(), "annotations": hits }),
                ))
            },
        )
        .await
    }

    #[tool(
        name = "get_annotation",
        description = "Get one annotation/label entry by exact key: applies-to kinds, value type, allowed \
        values, example and the authoritative doc URL.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn get_annotation(
        &self,
        Parameters(p): Parameters<GetAnnotationParams>,
    ) -> Result<CallToolResult, McpError> {
        let provider = p.provider.clone();
        self.run("get_annotation", provider.as_deref(), |idx, _| async move {
            let hits = blocking(idx, move |c| {
                search::get_annotation(c, &p.key, p.provider.as_deref())
            })
            .await?;
            Ok(json_result(
                &json!({ "count": hits.len(), "annotations": hits }),
            ))
        })
        .await
    }
}

#[tool_handler]
impl ServerHandler for KubeDocs {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_instructions(
                "kube-docs-mcp: version-pinned Kubernetes + CNCF + cloud reference. \
                 Workflow: `explain`/`get_schema` for field structure, `validate_manifest` to check YAML, \
                 `search_annotations`/`get_annotation` for annotation keys, `search_docs`+`get_doc` for prose. \
                 `list_projects` shows slugs and versions; pass `project`/`version` when a cluster version matters."
                    .to_string(),
            )
    }
}
