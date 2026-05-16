use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayProcessSpec {
    pub binary_path: PathBuf,
    pub config_path: PathBuf,
    pub work_dir: PathBuf,
    pub log_path: PathBuf,
    pub state_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayCommandSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub current_dir: PathBuf,
    pub log_path: PathBuf,
}

impl GatewayProcessSpec {
    pub fn validate(&self) -> Result<(), String> {
        validate_non_empty_path("binary_path", &self.binary_path)?;
        validate_non_empty_path("config_path", &self.config_path)?;
        validate_controlled_path("work_dir", &self.work_dir)?;
        validate_controlled_path("log_path", &self.log_path)?;
        validate_controlled_path("state_path", &self.state_path)?;
        if self.work_dir == Path::new("/") {
            return Err("work_dir must not be filesystem root".to_string());
        }
        Ok(())
    }

    pub fn command_spec(&self) -> Result<GatewayCommandSpec, String> {
        self.validate()?;
        Ok(GatewayCommandSpec {
            program: self.binary_path.clone(),
            args: vec![
                "--config".to_string(),
                self.config_path.to_string_lossy().into_owned(),
            ],
            current_dir: self.work_dir.clone(),
            log_path: self.log_path.clone(),
        })
    }
}

pub fn gateway_state_path_from_agent_state_path(agent_state_path: &str) -> PathBuf {
    let path = Path::new(agent_state_path);
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("agent-state.toml");
    path.with_file_name(format!("{file_name}.gateway.json"))
}

fn validate_non_empty_path(field: &str, path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    if path_contains_nul(path) {
        return Err(format!("{field} must not contain NUL bytes"));
    }
    Ok(())
}

fn validate_controlled_path(field: &str, path: &Path) -> Result<(), String> {
    validate_non_empty_path(field, path)?;
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!(
            "{field} must not contain parent directory components"
        ));
    }
    Ok(())
}

fn path_contains_nul(path: &Path) -> bool {
    path.to_string_lossy().chars().any(|value| value == '\0')
}
