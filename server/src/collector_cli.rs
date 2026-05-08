//! Server-side collector CLI helpers.
//!
//! These functions support `lightai-server collector sync` and
//! `lightai-server collector register`.  They read collector directories,
//! compute script hashes, and upsert records into the collector_registry table.

use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

/// Parsed collector.toml (subset of agent's CollectorManifest).
#[derive(Debug, serde::Deserialize)]
struct CollectorManifest {
    id: String,
    vendor: String,
    name: String,
    version: String,
    #[serde(default)]
    description: String,
    discover: String,
    metrics: String,
}

/// Outcome for a single collector directory.
#[derive(Debug)]
pub enum RegisterOutcome {
    Registered {
        id: String,
        version: String,
        discover_sha256: String,
        metrics_sha256: String,
    },
    Updated {
        id: String,
        version: String,
        discover_sha256: String,
        metrics_sha256: String,
    },
    Skipped {
        dir: String,
        reason: String,
    },
}

/// Upsert a single collector directory into the registry.
pub async fn register_one(
    pool: &SqlitePool,
    dir: &std::path::Path,
) -> anyhow::Result<RegisterOutcome> {
    let manifest = read_manifest(dir)?;
    let discover_path = dir.join(&manifest.discover);
    let metrics_path = dir.join(&manifest.metrics);
    if !discover_path.is_file() {
        return Ok(RegisterOutcome::Skipped {
            dir: dir.display().to_string(),
            reason: format!("discover script not found: {}", discover_path.display()),
        });
    }
    if !metrics_path.is_file() {
        return Ok(RegisterOutcome::Skipped {
            dir: dir.display().to_string(),
            reason: format!("metrics script not found: {}", metrics_path.display()),
        });
    }

    let discover_bytes = std::fs::read(&discover_path)?;
    let metrics_bytes = std::fs::read(&metrics_path)?;
    let discover_sha256 = sha256_hex(&discover_bytes);
    let metrics_sha256 = sha256_hex(&metrics_bytes);
    let now = now_unix_secs();

    // Check whether this id+version already exists.
    let existing: Option<String> =
        sqlx::query_scalar("SELECT id FROM collector_registry WHERE id = ? AND version = ?")
            .bind(&manifest.id)
            .bind(&manifest.version)
            .fetch_optional(pool)
            .await?;

    let is_new = existing.is_none();

    // Upsert via the existing repository function (which does INSERT OR REPLACE via
    // the same path as the API endpoint).  We go directly to SQL here to keep the
    // CLI self-contained.
    sqlx::query(
        r#"
        INSERT INTO collector_registry (
            id, vendor, name, version, description,
            discover_sha256, metrics_sha256, enabled,
            created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?, ?)
        ON CONFLICT(id, version) DO UPDATE SET
            vendor = excluded.vendor,
            name = excluded.name,
            description = excluded.description,
            discover_sha256 = excluded.discover_sha256,
            metrics_sha256 = excluded.metrics_sha256,
            -- preserve existing enabled status on update
            enabled = CASE
                WHEN collector_registry.enabled = 0 THEN 0
                ELSE collector_registry.enabled
            END,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(&manifest.id)
    .bind(&manifest.vendor)
    .bind(&manifest.name)
    .bind(&manifest.version)
    .bind(&manifest.description)
    .bind(&discover_sha256)
    .bind(&metrics_sha256)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    if is_new {
        Ok(RegisterOutcome::Registered {
            id: manifest.id,
            version: manifest.version,
            discover_sha256,
            metrics_sha256,
        })
    } else {
        Ok(RegisterOutcome::Updated {
            id: manifest.id,
            version: manifest.version,
            discover_sha256,
            metrics_sha256,
        })
    }
}

/// Scan a root directory for collector subdirectories and upsert all valid ones.
pub async fn sync_root(
    pool: &SqlitePool,
    root: &std::path::Path,
) -> anyhow::Result<Vec<RegisterOutcome>> {
    let mut outcomes = Vec::new();
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(e) => {
            anyhow::bail!("cannot read collector root {}: {e}", root.display());
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.') || name.ends_with('~') || name.ends_with(".tmp") {
            continue;
        }
        // Must contain collector.toml.
        if !path.join("collector.toml").is_file() {
            outcomes.push(RegisterOutcome::Skipped {
                dir: path.display().to_string(),
                reason: "missing collector.toml".to_string(),
            });
            continue;
        }

        match register_one(pool, &path).await {
            Ok(outcome) => outcomes.push(outcome),
            Err(e) => outcomes.push(RegisterOutcome::Skipped {
                dir: path.display().to_string(),
                reason: format!("{e:#}"),
            }),
        }
    }

    Ok(outcomes)
}

fn read_manifest(dir: &std::path::Path) -> anyhow::Result<CollectorManifest> {
    let path = dir.join("collector.toml");
    if !path.is_file() {
        anyhow::bail!("collector.toml not found in {}", dir.display());
    }
    let content = std::fs::read_to_string(&path)?;
    let manifest: CollectorManifest = toml::from_str(&content)?;
    if manifest.id.is_empty() {
        anyhow::bail!("id must not be empty");
    }
    if manifest.discover.is_empty() {
        anyhow::bail!("discover must not be empty");
    }
    if manifest.metrics.is_empty() {
        anyhow::bail!("metrics must not be empty");
    }
    if manifest.discover.contains('/') || manifest.discover.contains('\\') {
        anyhow::bail!("discover must be a filename, not a path");
    }
    if manifest.metrics.contains('/') || manifest.metrics.contains('\\') {
        anyhow::bail!("metrics must be a filename, not a path");
    }
    Ok(manifest)
}

/// Inspect a single collector directory: read metadata, compute hashes,
/// return a JSON-serialisable result.  Does NOT write to the database.
#[derive(Debug, serde::Serialize)]
pub struct InspectResult {
    pub id: String,
    pub vendor: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub discover_sha256: String,
    pub metrics_sha256: String,
    pub enabled: bool,
}

pub fn inspect_one(dir: &std::path::Path) -> anyhow::Result<InspectResult> {
    let manifest = read_manifest(dir)?;
    let discover_path = dir.join(&manifest.discover);
    let metrics_path = dir.join(&manifest.metrics);
    if !discover_path.is_file() {
        anyhow::bail!("discover script not found: {}", discover_path.display());
    }
    if !metrics_path.is_file() {
        anyhow::bail!("metrics script not found: {}", metrics_path.display());
    }
    let discover_bytes = std::fs::read(&discover_path)?;
    let metrics_bytes = std::fs::read(&metrics_path)?;
    Ok(InspectResult {
        id: manifest.id,
        vendor: manifest.vendor,
        name: manifest.name,
        version: manifest.version,
        description: manifest.description,
        discover_sha256: sha256_hex(&discover_bytes),
        metrics_sha256: sha256_hex(&metrics_bytes),
        enabled: true,
    })
}

pub fn inspect_root(root: &std::path::Path) -> anyhow::Result<Vec<InspectResult>> {
    let mut results = Vec::new();
    let entries = std::fs::read_dir(root)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", root.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.') || name.ends_with('~') || name.ends_with(".tmp") {
            continue;
        }
        if !path.join("collector.toml").is_file() {
            continue;
        }
        match inspect_one(&path) {
            Ok(r) => results.push(r),
            Err(e) => eprintln!("skipped {}: {e}", path.display()),
        }
    }
    Ok(results)
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
