//! In-memory aggregate counters. No query text, no client identifiers.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::json;

const BUCKETS_MS: [u64; 9] = [5, 10, 25, 50, 100, 250, 500, 1000, 2500];

#[derive(Default)]
struct Inner {
    calls: BTreeMap<(String, String), u64>,
    latency: BTreeMap<String, [u64; 10]>,
}

pub struct Metrics {
    inner: Mutex<Inner>,
    started: Instant,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            started: Instant::now(),
        }
    }
}

impl Metrics {
    pub fn record(&self, tool: &str, outcome: &str, took: Duration) {
        let Ok(mut g) = self.inner.lock() else { return };
        *g.calls
            .entry((tool.to_string(), outcome.to_string()))
            .or_insert(0) += 1;
        let ms = took.as_millis() as u64;
        let idx = BUCKETS_MS
            .iter()
            .position(|b| ms <= *b)
            .unwrap_or(BUCKETS_MS.len());
        g.latency.entry(tool.to_string()).or_insert([0; 10])[idx] += 1;
    }

    pub fn uptime_secs(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    pub fn snapshot(&self) -> serde_json::Value {
        let Ok(g) = self.inner.lock() else {
            return json!({});
        };
        let mut calls: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
        for ((tool, outcome), n) in &g.calls {
            calls
                .entry(tool.clone())
                .or_default()
                .insert(outcome.clone(), *n);
        }
        let labels: Vec<String> = BUCKETS_MS
            .iter()
            .map(|b| format!("le_{b}ms"))
            .chain(std::iter::once("gt_2500ms".into()))
            .collect();
        let latency: BTreeMap<String, BTreeMap<String, u64>> = g
            .latency
            .iter()
            .map(|(tool, buckets)| {
                (
                    tool.clone(),
                    labels
                        .iter()
                        .cloned()
                        .zip(buckets.iter().copied())
                        .collect(),
                )
            })
            .collect();
        json!({ "calls": calls, "latency_ms": latency })
    }
}
