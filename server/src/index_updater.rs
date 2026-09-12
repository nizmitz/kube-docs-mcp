//! Downloads the index release artifact described by `manifest.json`, verifies hashes and
//! hot-swaps the open index. Never panics; every failure is logged and retried next tick.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::db::{Index, SharedIndex};
use crate::mcp::tools::Caches;

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    #[serde(default)]
    pub build_date: String,
    pub index: ManifestIndex,
}

#[derive(Debug, Deserialize)]
pub struct ManifestIndex {
    pub file: String,
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub uncompressed_size: u64,
    pub uncompressed_sha256: String,
}

pub struct Updater {
    pub shared: SharedIndex,
    pub caches: Arc<Caches>,
    pub index_path: PathBuf,
    pub manifest_url: String,
    pub interval: Duration,
    client: reqwest::Client,
}

struct HashingWriter<W: Write> {
    inner: W,
    hasher: Sha256,
    written: u64,
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        self.written += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl Updater {
    pub fn new(
        shared: SharedIndex,
        caches: Arc<Caches>,
        index_path: PathBuf,
        manifest_url: String,
        interval: Duration,
    ) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("kube-docs-mcp/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(600))
            .connect_timeout(Duration::from_secs(20))
            .build()?;
        Ok(Self {
            shared,
            caches,
            index_path,
            manifest_url,
            interval,
            client,
        })
    }

    /// Resolve relative asset names against the manifest URL.
    fn asset_url(&self, file: &str) -> String {
        if file.starts_with("http://") || file.starts_with("https://") {
            return file.to_string();
        }
        match self.manifest_url.rfind('/') {
            Some(i) => format!("{}/{}", &self.manifest_url[..i], file),
            None => file.to_string(),
        }
    }

    pub async fn fetch_manifest(&self) -> anyhow::Result<Manifest> {
        let bytes = self
            .client
            .get(&self.manifest_url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        let m: Manifest = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(
            m.schema_version == 1,
            "unsupported index schema_version {}",
            m.schema_version
        );
        Ok(m)
    }

    fn current_sha(&self) -> Option<String> {
        self.shared
            .load_full()
            .and_then(|i| i.meta_str("uncompressed_sha256").map(String::from))
    }

    /// Returns true when a new index was installed.
    pub async fn check_once(&self) -> anyhow::Result<bool> {
        let m = self.fetch_manifest().await?;
        let want = m.index.uncompressed_sha256.to_ascii_lowercase();
        if self.current_sha().as_deref() == Some(want.as_str()) {
            tracing::debug!("index up to date");
            return Ok(false);
        }
        let url = self.asset_url(&m.index.file);
        tracing::info!(url, build_date = %m.build_date, "downloading index");
        let download = self.index_path.with_extension("sqlite.download");
        let tmp = self.index_path.with_extension("sqlite.tmp");
        let got = self.download(&url, &download).await?;
        anyhow::ensure!(
            got == m.index.sha256.to_ascii_lowercase(),
            "sha256 mismatch for {url}: {got}"
        );
        let tmp2 = tmp.clone();
        let dl2 = download.clone();
        let (uncompressed_sha, size) =
            tokio::task::spawn_blocking(move || decompress(&dl2, &tmp2)).await??;
        let _ = tokio::fs::remove_file(&download).await;
        if uncompressed_sha != want {
            let _ = tokio::fs::remove_file(&tmp).await;
            anyhow::bail!("uncompressed sha256 mismatch: {uncompressed_sha}");
        }
        // Stamp the hash into a sidecar so `current_sha` works after restart.
        tokio::fs::rename(&tmp, &self.index_path).await?;
        tokio::fs::write(self.index_path.with_extension("sqlite.sha256"), &want).await?;
        self.reopen().await?;
        tracing::info!(size, sha = %want, "index installed");
        Ok(true)
    }

    async fn download(&self, url: &str, dest: &Path) -> anyhow::Result<String> {
        let mut resp = self.client.get(url).send().await?.error_for_status()?;
        let mut file = tokio::fs::File::create(dest).await?;
        let mut hasher = Sha256::new();
        use tokio::io::AsyncWriteExt;
        while let Some(chunk) = resp.chunk().await? {
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        Ok(hex::encode(hasher.finalize()))
    }

    pub async fn reopen(&self) -> anyhow::Result<()> {
        let path = self.index_path.clone();
        let idx = tokio::task::spawn_blocking(move || open_with_sidecar(&path)).await??;
        self.shared.store(Some(Arc::new(idx)));
        self.caches.clear();
        Ok(())
    }

    pub async fn run(self) {
        let mut tick = tokio::time::interval(self.interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await; // first tick fires immediately; boot already handled by caller
        loop {
            tick.tick().await;
            match self.check_once().await {
                Ok(true) => {}
                Ok(false) => {}
                Err(e) => tracing::warn!(error = %e, "index update failed"),
            }
        }
    }
}

fn decompress(src: &Path, dst: &Path) -> anyhow::Result<(String, u64)> {
    let input = std::fs::File::open(src)?;
    let out = std::fs::File::create(dst)?;
    let mut w = HashingWriter {
        inner: std::io::BufWriter::new(out),
        hasher: Sha256::new(),
        written: 0,
    };
    zstd::stream::copy_decode(std::io::BufReader::new(input), &mut w)?;
    w.flush()?;
    Ok((hex::encode(w.hasher.finalize()), w.written))
}

/// Open the index and, if a `.sha256` sidecar exists, expose it via `meta["uncompressed_sha256"]`.
pub fn open_with_sidecar(path: &Path) -> anyhow::Result<Index> {
    let mut idx = Index::open(path)?;
    if let Ok(s) = std::fs::read_to_string(path.with_extension("sqlite.sha256")) {
        idx.meta
            .insert("uncompressed_sha256".into(), s.trim().to_ascii_lowercase());
    }
    Ok(idx)
}

/// Block until an index exists on disk (downloading if needed) and open it.
pub async fn ensure_index(updater: &Updater) -> anyhow::Result<()> {
    if updater.index_path.exists() {
        updater.reopen().await?;
        // opportunistic refresh in background happens on the first interval tick
        return Ok(());
    }
    let mut delay = Duration::from_secs(5);
    loop {
        match updater.check_once().await {
            Ok(_) => return Ok(()),
            Err(e) => {
                tracing::warn!(error = %e, retry_in = ?delay, "initial index download failed");
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(300));
            }
        }
    }
}
