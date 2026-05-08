use tokio::time::{sleep, Duration};

/// Instance readiness probe: max retry attempts.
/// Agent polls service endpoints up to this limit after instance start; interval between retries is ENDPOINT_READY_INTERVAL_MS.
/// Can be overridden via instance params_json.probe_max_attempts.
const ENDPOINT_READY_MAX_ATTEMPTS: u32 = 5;

/// Startup readiness probe: retry interval (ms), default 5s.
/// Can be overridden via instance params_json.probe_interval_ms.
const ENDPOINT_READY_INTERVAL_MS: u64 = 5000;

/// Startup readiness probe: single HTTP request timeout (ms).
/// Can be overridden via instance params_json.probe_timeout_ms.
const ENDPOINT_READY_REQUEST_TIMEOUT_MS: u64 = 400;

/// Post-start wait for custom/script backend process initialization (ms).
pub(crate) const CUSTOM_SCRIPT_STARTUP_WAIT_MS: u64 = 500;

/// Wait before re-verifying process liveness after probe pass (ms).
pub(crate) const POST_READINESS_VERIFY_DELAY_MS: u64 = 300;

/// Buffer time for log collection after killing process on probe failure (ms).
pub(crate) const POST_KILL_LOG_WAIT_MS: u64 = 200;

/// Instance readiness probe config. Can be overridden via instance params_json; uses built-in defaults otherwise.
#[derive(Debug, Clone)]
pub(crate) struct ProbeConfig {
    /// Probe path list; None means use default paths from probe_paths(backend).
    pub(crate) paths: Option<Vec<String>>,
    pub(crate) max_attempts: u32,
    pub(crate) interval_ms: u64,
    pub(crate) timeout_ms: u64,
}

impl Default for ProbeConfig {
    fn default() -> Self {
        Self {
            paths: None,
            max_attempts: ENDPOINT_READY_MAX_ATTEMPTS,
            interval_ms: ENDPOINT_READY_INTERVAL_MS,
            timeout_ms: ENDPOINT_READY_REQUEST_TIMEOUT_MS,
        }
    }
}

impl ProbeConfig {
    pub(crate) fn from_payload(payload: &serde_json::Value) -> Self {
        let params = payload
            .get("params")
            .or_else(|| payload.get("params_json"))
            .unwrap_or(&serde_json::Value::Null);
        let parsed = if let Some(value) = params.as_str() {
            serde_json::from_str::<serde_json::Value>(value).unwrap_or(serde_json::Value::Null)
        } else {
            params.clone()
        };
        Self {
            paths: parsed
                .get("probe_paths")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .filter(|s| !s.trim().is_empty())
                        .collect()
                }),
            max_attempts: parsed
                .get("probe_max_attempts")
                .and_then(|v| v.as_u64())
                .map(|v| v.clamp(1, 60) as u32)
                .unwrap_or(ENDPOINT_READY_MAX_ATTEMPTS),
            interval_ms: parsed
                .get("probe_interval_ms")
                .and_then(|v| v.as_u64())
                .map(|v| v.clamp(100, 60_000))
                .unwrap_or(ENDPOINT_READY_INTERVAL_MS),
            timeout_ms: parsed
                .get("probe_timeout_ms")
                .and_then(|v| v.as_u64())
                .map(|v| v.clamp(50, 60_000))
                .unwrap_or(ENDPOINT_READY_REQUEST_TIMEOUT_MS),
        }
    }

    fn effective_paths(&self, backend: &str) -> Vec<String> {
        match &self.paths {
            Some(paths) if !paths.is_empty() => paths.clone(),
            _ => probe_paths(backend).iter().map(|s| s.to_string()).collect(),
        }
    }
}

pub fn build_test_urls(backend: &str, base: &str) -> Result<Vec<String>, String> {
    let trimmed = base.trim().trim_end_matches('/');
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err("instance test URL must start with http:// or https://".to_string());
    }
    let parsed =
        reqwest::Url::parse(trimmed).map_err(|_| "invalid instance test URL format".to_string())?;
    let path = parsed.path().trim_end_matches('/');
    let host = parsed
        .host_str()
        .ok_or_else(|| "instance test URL missing host".to_string())?;
    let root = match parsed.port() {
        Some(port) => format!("{}://{}:{}", parsed.scheme(), host, port),
        None => format!("{}://{}", parsed.scheme(), host),
    };
    let mut urls = Vec::new();
    if path.is_empty() {
        match backend {
            "llama_cpp" | "vllm" | "ollama" => {
                urls.push(format!("{trimmed}/v1/models"));
                urls.push(format!("{trimmed}/health"));
                urls.push(trimmed.to_string());
            }
            _ => {
                urls.push(format!("{trimmed}/health"));
                urls.push(format!("{trimmed}/v1/models"));
                urls.push(trimmed.to_string());
            }
        }
    } else {
        urls.push(trimmed.to_string());
        if !path.ends_with("/v1/models") {
            urls.push(format!("{root}/v1/models"));
        }
        if !path.ends_with("/health") {
            urls.push(format!("{root}/health"));
        }
    }
    urls.dedup();
    Ok(urls)
}

pub fn summarize_test_failures(urls: &[String], failures: &[String]) -> String {
    if failures.iter().all(|failure| failure.contains("HTTP 404")) {
        format!(
            "No available test endpoint found; tried {}. Verify the backend supports OpenAI-compatible APIs or configure the correct endpoint_url in the instance.",
            urls.join(", ")
        )
    } else {
        format!("test failed: {}", failures.join("；"))
    }
}

/// Returns readiness probe paths for backend (ordered by priority).
/// Path strategy consistent with build_test_urls.
fn probe_paths(backend: &str) -> &'static [&'static str] {
    match backend {
        "llama_cpp" | "vllm" | "ollama" => &["/v1/models", "/health", "/"],
        _ => &["/health", "/v1/models", "/"],
    }
}

pub(crate) async fn endpoint_ready(backend: &str, base_url: &str, probe: &ProbeConfig) -> bool {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_millis(probe.timeout_ms))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };
    let root = base_url.trim_end_matches('/');
    let paths = probe.effective_paths(backend);
    for _ in 0..probe.max_attempts {
        for path in &paths {
            let url = format!("{root}{path}");
            if let Ok(response) = client.get(&url).send().await {
                if response.status().is_success() || response.status().is_redirection() {
                    return true;
                }
            }
        }
        sleep(Duration::from_millis(probe.interval_ms)).await;
    }
    false
}
