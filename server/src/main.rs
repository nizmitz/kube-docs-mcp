use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use kube_docs_mcp::db::{self, SharedIndex};
use kube_docs_mcp::index_updater::{self, Updater};
use kube_docs_mcp::mcp::tools::{Caches, KubeDocs};
use kube_docs_mcp::metrics::Metrics;
use kube_docs_mcp::web::{self, AppState};
use rmcp::ServiceExt;
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(
    name = "kube-docs-mcp",
    version,
    about = "MCP server for Kubernetes/CNCF schemas, annotations and docs"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the MCP server (HTTP or stdio).
    Serve(ServeArgs),
    /// HTTP GET a health URL; exit 0 on 200 (for Docker HEALTHCHECK in a scratch image).
    Healthcheck {
        #[arg(long, default_value = "http://127.0.0.1:8080/healthz")]
        url: String,
    },
}

#[derive(clap::Args)]
struct ServeArgs {
    /// Listen address for Streamable HTTP (e.g. 0.0.0.0:8080). Mutually exclusive with --stdio.
    #[arg(long, env = "KD_HTTP", conflicts_with = "stdio")]
    http: Option<String>,
    /// Serve MCP over stdin/stdout.
    #[arg(long)]
    stdio: bool,
    /// Path of the SQLite index.
    #[arg(long, env = "KD_INDEX", default_value = "/data/index.sqlite")]
    index: PathBuf,
    /// URL of manifest.json; when set, the index is downloaded on boot and refreshed periodically.
    #[arg(long, env = "KD_MANIFEST_URL")]
    manifest_url: Option<String>,
    /// Refresh interval in seconds.
    #[arg(long, env = "KD_UPDATE_INTERVAL_SECS", default_value_t = 21_600)]
    update_interval_secs: u64,
    /// Allowed Host header values (DNS-rebinding guard). Repeatable or comma separated.
    #[arg(long = "allowed-host", env = "KD_ALLOWED_HOSTS", value_delimiter = ',', default_values_t = ["localhost:8080".to_string()])]
    allowed_hosts: Vec<String>,
    /// Serve the landing page from this directory instead of the embedded copy.
    #[arg(long, env = "KD_STATIC_DIR")]
    static_dir: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(2)
        .enable_all()
        .build()?;
    rt.block_on(async_main(cli))
}

async fn async_main(cli: Cli) -> anyhow::Result<()> {
    match cli.cmd {
        Cmd::Healthcheck { url } => healthcheck(&url).await,
        Cmd::Serve(args) => serve(args).await,
    }
}

async fn healthcheck(url: &str) -> anyhow::Result<()> {
    install_crypto();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()?;
    match client.get(url).send().await {
        Ok(r) if r.status().is_success() => Ok(()),
        Ok(r) => {
            eprintln!("healthcheck: {}", r.status());
            std::process::exit(1)
        }
        Err(e) => {
            eprintln!("healthcheck: {e}");
            std::process::exit(1)
        }
    }
}

fn install_crypto() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

async fn serve(args: ServeArgs) -> anyhow::Result<()> {
    // stdout is the MCP channel in stdio mode; logs always go to stderr.
    tracing_subscriber::fmt()
        .json()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=warn".into()),
        )
        .init();
    install_crypto();

    let shared: SharedIndex = db::new_shared();
    let caches = Arc::new(Caches::default());
    let metrics = Arc::new(Metrics::default());

    let updater = match &args.manifest_url {
        Some(url) => Some(Updater::new(
            shared.clone(),
            caches.clone(),
            args.index.clone(),
            url.clone(),
            Duration::from_secs(args.update_interval_secs.max(60)),
        )?),
        None => None,
    };

    if args.stdio {
        open_local(&shared, &args.index)?;
        let service = KubeDocs::new(shared, caches, metrics)
            .serve(rmcp::transport::stdio())
            .await?;
        service.waiting().await?;
        return Ok(());
    }

    let addr = args.http.clone().unwrap_or_else(|| "0.0.0.0:8080".into());
    let ct = CancellationToken::new();
    let state = AppState {
        idx: shared.clone(),
        caches: caches.clone(),
        metrics,
        static_dir: args.static_dir.clone(),
    };
    let router = web::router(state, args.allowed_hosts.clone(), ct.child_token());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, index = %args.index.display(), "listening");

    // Open (or download) the index without blocking the listener so /healthz answers 503 meanwhile.
    match updater {
        Some(u) => {
            tokio::spawn(async move {
                if let Err(e) = index_updater::ensure_index(&u).await {
                    tracing::error!(error = %e, "could not obtain index");
                }
                u.run().await;
            });
        }
        None => open_local(&shared, &args.index)?,
    }

    let shutdown = ct.clone();
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            shutdown.cancel();
        })
        .await?;
    Ok(())
}

fn open_local(shared: &SharedIndex, path: &std::path::Path) -> anyhow::Result<()> {
    let idx = index_updater::open_with_sidecar(path)
        .map_err(|e| anyhow::anyhow!("open index {}: {e}", path.display()))?;
    shared.store(Some(Arc::new(idx)));
    Ok(())
}
