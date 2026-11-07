use kerniq_server::prelude::*;

/// Application configuration: Kerniq's server settings (from `KERNIQ_*`
/// variables) plus application-specific values.
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub service_name: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            service_name: env!("CARGO_PKG_NAME").to_owned(),
        }
    }
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            server: ServerConfig::from_env()?,
            service_name: std::env::var("SERVICE_NAME")
                .unwrap_or_else(|_| env!("CARGO_PKG_NAME").to_owned()),
        })
    }
}
