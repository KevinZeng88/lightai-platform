use std::time::Duration;

#[derive(Debug)]
pub struct CheckResult {
    pub status: String,
    pub message: String,
}

pub async fn check_url(url: &str) -> CheckResult {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return CheckResult {
                status: "failed".to_string(),
                message: error.to_string(),
            };
        }
    };

    match client.get(url).send().await {
        Ok(response) if response.status().is_success() || response.status().is_redirection() => {
            CheckResult {
                status: "running".to_string(),
                message: format!("HTTP {}", response.status()),
            }
        }
        Ok(response) => CheckResult {
            status: "failed".to_string(),
            message: format!("HTTP {}", response.status()),
        },
        Err(error) => CheckResult {
            status: "failed".to_string(),
            message: error.to_string(),
        },
    }
}
