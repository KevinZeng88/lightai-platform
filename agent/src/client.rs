use crate::models::{HeartbeatRequest, RegisterRequest, RegisterResponse};

#[derive(Clone)]
pub struct ServerClient {
    base_url: String,
    client: reqwest::Client,
}

impl ServerClient {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
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

    pub async fn heartbeat(&self, token: &str, request: &HeartbeatRequest) -> anyhow::Result<()> {
        self.client
            .post(format!("{}/api/agent/heartbeat", self.base_url))
            .bearer_auth(token)
            .json(request)
            .send()
            .await?
            .error_for_status()?;

        Ok(())
    }
}
