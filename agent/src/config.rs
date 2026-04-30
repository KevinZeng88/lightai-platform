#[derive(Debug, Clone)]
pub struct Config {
    pub listen_addr: String,
    pub server_url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:8081".to_string(),
            server_url: "http://127.0.0.1:8080".to_string(),
        }
    }
}
