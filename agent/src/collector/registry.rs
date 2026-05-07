use serde::{Deserialize, Serialize};

/// Server-side collector registry entry.
///
/// The Agent pulls the registry list from the Server and uses it to validate
/// local collector scripts before execution.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RegistryEntry {
    pub id: String,
    pub vendor: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub discover_sha256: String,
    pub metrics_sha256: String,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Request body for registering a collector on the Server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterCollectorRequest {
    pub id: String,
    pub vendor: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub discover_sha256: String,
    pub metrics_sha256: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

/// Response wrapper for the registry list API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryListResponse {
    pub collectors: Vec<RegistryEntry>,
}
