#[derive(Debug, Clone)]
pub struct Config {
    pub listen_addr: String,
    pub database_url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:8080".to_string(),
            database_url: "sqlite://data/lightai.db".to_string(),
        }
    }
}
