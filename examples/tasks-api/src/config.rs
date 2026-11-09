use kerniq_server::prelude::*;

/// Application configuration: Kerniq's server settings (from `KERNIQ_*`
/// variables) plus application-specific values.
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub service_name: String,
    /// Key clients must send in the `x-api-key` header (env `API_KEY`).
    pub api_key: String,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .ok_or_else(|| Error::internal("API_KEY environment variable must be set"))?;
        Ok(Self {
            server: ServerConfig::from_env()?,
            service_name: std::env::var("SERVICE_NAME")
                .unwrap_or_else(|_| env!("CARGO_PKG_NAME").to_owned()),
            api_key,
        })
    }
}
