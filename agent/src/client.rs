use crate::collector::registry::RegistryEntry;
use crate::models::{
    AgentTaskPollRequest, AgentTaskPollResponse, AgentTaskResultRequest, HeartbeatRequest,
    HeartbeatResponse, RegisterRequest, RegisterResponse,
};

/// Structured error for collector registry fetch.
#[derive(Debug)]
pub enum RegistryFetchError {
    /// Network or deserialization error.
    FetchFailed(String),
    /// Agent token invalid / not registered.
    Unauthorized,
    /// Token valid but insufficient permissions.
    Forbidden,
    /// Server returned a non-2xx status code.
    ServerError(u16),
}

impl std::fmt::Display for RegistryFetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FetchFailed(msg) => write!(f, "registry fetch failed: {msg}"),
            Self::Unauthorized => write!(f, "registry fetch unauthorized — agent token invalid"),
            Self::Forbidden => write!(f, "registry fetch forbidden"),
            Self::ServerError(code) => write!(f, "registry fetch server error (HTTP {code})"),
        }
    }
}

#[derive(Clone)]
pub struct ServerClient {
    base_url: String,
    client: reqwest::Client,
}

impl ServerClient {
    pub fn new(
        base_url: String,
        ca_cert_path: Option<&str>,
        insecure_skip_tls_verify: bool,
    ) -> anyhow::Result<Self> {
        let mut client = reqwest::Client::builder();

        if insecure_skip_tls_verify {
            client = client.danger_accept_invalid_certs(true);
        } else if let Some(path) = ca_cert_path {
            if std::path::Path::new(path).exists() {
                let cert = std::fs::read(path)?;
                let cert = reqwest::tls::Certificate::from_pem(&cert)?;
                client = client.add_root_certificate(cert);
            } else {
                anyhow::bail!("CA certificate not found at '{}'. Run 'lightai-agent ca fetch' to download it, or set insecure_skip_tls_verify=true for diagnostics.", path);
            }
        } else {
            anyhow::bail!("TLS verification requires ca_cert_path or insecure_skip_tls_verify. Configure [server].ca_cert_path in agent.toml.");
        }

        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: client.build()?,
        })
    }

    pub async fn register(&self, request: &RegisterRequest) -> anyhow::Result<RegisterResponse> {
        let response = self
            .client
            .post(format!("{}/api/agent/register", self.base_url))
            .json(request)
            .send()
            .await?
            .error_for_status()?;

        Ok(response.json().await?)
    }

    /// Fetch the collector registry from Server, authenticated by agent token.
    /// Returns a structured error for each failure mode so callers can distinguish
    /// "fetch failed" from "registry empty".
    pub async fn fetch_collector_registry(
        &self,
        token: &str,
    ) -> Result<Vec<RegistryEntry>, RegistryFetchError> {
        let response = match self
            .client
            .get(format!("{}/api/agent/collector-registry", self.base_url))
            .bearer_auth(token)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return Err(RegistryFetchError::FetchFailed(e.to_string()));
            }
        };

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(RegistryFetchError::Unauthorized);
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(RegistryFetchError::Forbidden);
        }
        if !status.is_success() {
            return Err(RegistryFetchError::ServerError(status.as_u16()));
        }

        match response.json::<serde_json::Value>().await {
            Ok(payload) => {
                let entries: Vec<RegistryEntry> = payload
                    .get("collectors")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                Ok(entries)
            }
            Err(e) => Err(RegistryFetchError::FetchFailed(e.to_string())),
        }
    }

    pub async fn heartbeat(
        &self,
        token: &str,
        request: &HeartbeatRequest,
    ) -> anyhow::Result<HeartbeatResponse> {
        let response = self
            .client
            .post(format!("{}/api/agent/heartbeat", self.base_url))
            .bearer_auth(token)
            .json(request)
            .send()
            .await?
            .error_for_status()?;

        Ok(response.json().await?)
    }

    pub async fn poll_task(
        &self,
        token: &str,
        request: &AgentTaskPollRequest,
    ) -> anyhow::Result<AgentTaskPollResponse> {
        let response = self
            .client
            .post(format!("{}/api/agent/tasks/poll", self.base_url))
            .bearer_auth(token)
            .json(request)
            .send()
            .await?
            .error_for_status()?;

        Ok(response.json().await?)
    }

    pub async fn report_task_result(
        &self,
        token: &str,
        task_id: &str,
        request: &AgentTaskResultRequest,
    ) -> anyhow::Result<()> {
        self.client
            .post(format!(
                "{}/api/agent/tasks/{task_id}/result",
                self.base_url
            ))
            .bearer_auth(token)
            .json(request)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}
