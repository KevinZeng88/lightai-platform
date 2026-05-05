use tokio::time::{sleep, Duration};

/// 实例就绪探测：最大重试次数。
/// 实例启动后 Agent 按此上限轮询服务端点；每次重试之间间隔 ENDPOINT_READY_INTERVAL_MS。
/// 可由实例 params_json.probe_max_attempts 覆盖。
const ENDPOINT_READY_MAX_ATTEMPTS: u32 = 5;

/// 启动就绪探测：重试间隔（毫秒），默认 5 秒。
/// 可由实例 params_json.probe_interval_ms 覆盖。
const ENDPOINT_READY_INTERVAL_MS: u64 = 5000;

/// 启动就绪探测：单次 HTTP 请求超时（毫秒）。
/// 可由实例 params_json.probe_timeout_ms 覆盖。
const ENDPOINT_READY_REQUEST_TIMEOUT_MS: u64 = 400;

/// 启动后等待 custom+script 后端进程初始化的时间（毫秒）。
pub(crate) const CUSTOM_SCRIPT_STARTUP_WAIT_MS: u64 = 500;

/// 就绪探测通过后再次验证进程存活前的等待（毫秒）。
pub(crate) const POST_READINESS_VERIFY_DELAY_MS: u64 = 300;

/// 探测失败 kill 进程后等待日志收集的缓冲时间（毫秒）。
pub(crate) const POST_KILL_LOG_WAIT_MS: u64 = 200;

/// 实例就绪探测配置。可通过 instance params_json 覆盖，未配置时使用内置默认值。
#[derive(Debug, Clone)]
pub(crate) struct ProbeConfig {
    /// 探测路径列表；None 表示使用 probe_paths(backend) 的默认路径。
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
        return Err("实例测试地址必须是 http:// 或 https://".to_string());
    }
    let parsed = reqwest::Url::parse(trimmed).map_err(|_| "实例测试地址格式非法".to_string())?;
    let path = parsed.path().trim_end_matches('/');
    let host = parsed
        .host_str()
        .ok_or_else(|| "实例测试地址缺少主机".to_string())?;
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
            "未找到可用测试接口；已尝试 {}。请确认后端是否启用 OpenAI 兼容接口，或在实例中配置正确 endpoint_url。",
            urls.join(", ")
        )
    } else {
        format!("测试失败：{}", failures.join("；"))
    }
}

/// 返回 backend 对应的就绪探测路径（优先级从高到低）。
/// 与 build_test_urls 保持一致的路径策略。
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
