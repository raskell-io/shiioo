//! A node: a worker on another machine (`docs/design/tools-and-machines.md`
//! §2, ADR-0006, ADR-0010).
//!
//! The office's record stays on the office machine. A node holds leases,
//! appends events and reads logs through the office's HTTP API
//! ([`RemoteStore`] implements `EventStore` over it), so the worker code is
//! the same as on the office machine: leases, fencing and recovery hold
//! across the wire. The node dials out; the office never connects to it.
//!
//! The loop ([`run_node`]): heartbeat, ask the office for work addressed
//! to this machine (a long poll), drive each run with a fresh worker, repeat.

use serde::Deserialize;
use serde_json::{Value, json};
use shiioo_core::{Event, EventStore, Lease, Payload, RunId, Seq, StoreError, WorkerId};
use shiioo_runtime::{RunOutcome, Worker};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The office's log, over HTTP, with the office's fencing errors intact.
pub struct RemoteStore {
    base: String,
    token: String,
    http: reqwest::Client,
}

impl RemoteStore {
    pub fn new(server_url: &str, token: &str) -> Self {
        Self {
            base: server_url.trim_end_matches('/').to_owned(),
            token: token.to_owned(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn server_url(&self) -> &str {
        &self.base
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    async fn call<T: for<'de> Deserialize<'de>>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<T, StoreError> {
        let mut req = self
            .http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.token);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let response = req
            .send()
            .await
            .map_err(|e| StoreError::Backend(format!("office unreachable: {e}")))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| StoreError::Backend(format!("reading the office's answer: {e}")))?;
        if status.is_success() {
            return serde_json::from_str(&text)
                .map_err(|e| StoreError::Backend(format!("decoding the office's answer: {e}")));
        }
        let problem: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if let Some(err) = problem.get("store").and_then(StoreError::from_json) {
            return Err(err);
        }
        Err(StoreError::Backend(format!(
            "office answered {status}: {}",
            problem["detail"]
                .as_str()
                .map_or_else(|| text.chars().take(200).collect(), str::to_owned)
        )))
    }
}

impl EventStore for RemoteStore {
    async fn create_run(&self, _spec: Value) -> Result<RunId, StoreError> {
        Err(StoreError::Backend(
            "a node does not create runs; the office does".into(),
        ))
    }

    async fn acquire_lease(
        &self,
        run_id: RunId,
        worker: WorkerId,
        ttl_ms: u64,
    ) -> Result<Lease, StoreError> {
        self.call(
            reqwest::Method::POST,
            &format!("/v1/runs/{run_id}/lease"),
            Some(json!({ "worker": worker, "ttl_ms": ttl_ms })),
        )
        .await
    }

    async fn renew_lease(&self, lease: &Lease, ttl_ms: u64) -> Result<Lease, StoreError> {
        self.call(
            reqwest::Method::POST,
            &format!("/v1/runs/{}/lease/renew", lease.run_id),
            Some(json!({ "lease": lease, "ttl_ms": ttl_ms })),
        )
        .await
    }

    async fn append(
        &self,
        lease: &Lease,
        expected_next: Seq,
        payloads: Vec<Payload>,
    ) -> Result<Vec<Event>, StoreError> {
        self.call(
            reqwest::Method::POST,
            &format!("/v1/runs/{}/events", lease.run_id),
            Some(json!({ "lease": lease, "expected_next": expected_next, "payloads": payloads })),
        )
        .await
    }

    async fn append_external(
        &self,
        _run_id: RunId,
        _payloads: Vec<Payload>,
    ) -> Result<Vec<Event>, StoreError> {
        Err(StoreError::Backend(
            "a node does not write external input; the office does".into(),
        ))
    }

    async fn read(&self, run_id: RunId, from: Seq) -> Result<Vec<Event>, StoreError> {
        self.call(
            reqwest::Method::GET,
            &format!("/v1/runs/{run_id}/events?from={}", from.0),
            None,
        )
        .await
    }

    async fn list_runs(&self) -> Result<Vec<RunId>, StoreError> {
        let views: Vec<Value> = self.call(reqwest::Method::GET, "/v1/runs", None).await?;
        Ok(views
            .iter()
            .filter_map(|v| serde_json::from_value(v["id"].clone()).ok())
            .collect())
    }
}

/// What a node tells the office about itself and how it polls.
#[derive(Clone, Debug)]
pub struct NodeConfig {
    pub name: String,
    pub version: String,
    pub capabilities: Vec<String>,
    /// How long one `work` poll may wait at the office.
    pub poll_wait_secs: u64,
    /// Runs driven at the same time.
    pub max_concurrent: usize,
}

impl NodeConfig {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            capabilities: Vec::new(),
            poll_wait_secs: 25,
            max_concurrent: 4,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error("office unreachable: {0}")]
    Unreachable(reqwest::Error),
    #[error("office refused: {0}")]
    Refused(String),
}

/// Exchanges an enrollment token for this machine's token
/// (`POST /v1/machines/enroll`, no other credential needed).
pub async fn enroll(
    server_url: &str,
    enrollment_token: &str,
    name: &str,
    labels: &BTreeMap<String, String>,
    capabilities: &[String],
    version: &str,
) -> Result<String, NodeError> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default();
    let response = http
        .post(format!("{}/v1/machines/enroll", server_url.trim_end_matches('/')))
        .json(&json!({
            "token": enrollment_token,
            "name": name,
            "labels": labels,
            "capabilities": capabilities,
            "version": version,
        }))
        .send()
        .await
        .map_err(NodeError::Unreachable)?;
    let status = response.status();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        return Err(NodeError::Refused(
            body["detail"]
                .as_str()
                .map_or_else(|| status.to_string(), str::to_owned),
        ));
    }
    body["machine_token"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| NodeError::Refused("no machine token in the answer".into()))
}

/// Works for the office until the future is dropped: heartbeats, takes the
/// runs addressed to this machine, drives them. Network errors are logged
/// and retried with backoff; a run that cannot be driven is left for the
/// next poll.
pub async fn run_node(
    config: NodeConfig,
    store: Arc<RemoteStore>,
    workers: Arc<dyn Fn() -> Worker<RemoteStore> + Send + Sync>,
) {
    let in_flight: Arc<Mutex<HashSet<RunId>>> = Arc::new(Mutex::new(HashSet::new()));
    let mut backoff = Duration::from_secs(1);
    eprintln!(
        "node {}: working for {} (v{}, up to {} runs at once)",
        config.name, store.base, config.version, config.max_concurrent
    );
    loop {
        let running: Vec<String> = lock(&in_flight).iter().map(ToString::to_string).collect();
        if let Err(e) = heartbeat(&store, &config, &running).await {
            eprintln!("node: heartbeat failed: {e}; retrying in {backoff:?}");
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(30));
            continue;
        }
        backoff = Duration::from_secs(1);
        let free = config.max_concurrent.saturating_sub(lock(&in_flight).len());
        if free == 0 {
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }
        let runs = match work(&store, &config).await {
            Ok(runs) => runs,
            Err(e) => {
                eprintln!("node: asking for work failed: {e}");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        for run_id in runs.into_iter().take(free) {
            if !lock(&in_flight).insert(run_id) {
                continue;
            }
            let worker = workers();
            let in_flight = Arc::clone(&in_flight);
            tokio::spawn(async move {
                for attempt in 0..3 {
                    match worker.drive(run_id).await {
                        Ok(RunOutcome::LostLease) if attempt < 2 => continue,
                        Ok(outcome) => {
                            eprintln!("node: run {run_id} {outcome:?}");
                            break;
                        }
                        Err(e) => {
                            eprintln!("node: run {run_id}: worker stopped: {e}");
                            break;
                        }
                    }
                }
                lock(&in_flight).remove(&run_id);
            });
        }
    }
}

async fn heartbeat(store: &RemoteStore, config: &NodeConfig, in_flight: &[String]) -> Result<(), StoreError> {
    let _: Value = store
        .call(
            reqwest::Method::POST,
            &format!("/v1/machines/{}/heartbeat", config.name),
            Some(json!({
                "version": config.version,
                "capabilities": config.capabilities,
                "in_flight": in_flight,
            })),
        )
        .await?;
    Ok(())
}

async fn work(store: &RemoteStore, config: &NodeConfig) -> Result<Vec<RunId>, StoreError> {
    let v: Value = store
        .call(
            reqwest::Method::GET,
            &format!(
                "/v1/machines/{}/work?wait={}",
                config.name, config.poll_wait_secs
            ),
            None,
        )
        .await?;
    Ok(v["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| serde_json::from_value(r.clone()).ok())
        .collect())
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}
