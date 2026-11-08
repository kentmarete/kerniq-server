//! Service identity attached to every signal as OpenTelemetry resource
//! attributes.

use opentelemetry::KeyValue;

/// `service.name`.
pub const SERVICE_NAME: &str = "service.name";
/// `service.version`.
pub const SERVICE_VERSION: &str = "service.version";
/// `service.namespace`.
pub const SERVICE_NAMESPACE: &str = "service.namespace";
/// `service.instance.id`.
pub const SERVICE_INSTANCE_ID: &str = "service.instance.id";
/// `deployment.environment.name`.
pub const DEPLOYMENT_ENVIRONMENT: &str = "deployment.environment.name";

/// Who is emitting telemetry. Converted into an OpenTelemetry `Resource` by
/// `kerniq-observability`.
///
/// Unset fields fall back to the standard environment variables
/// (`OTEL_SERVICE_NAME`, `OTEL_RESOURCE_ATTRIBUTES`) through the SDK's
/// resource detectors.
#[derive(Debug, Clone, Default)]
pub struct ServiceInfo {
    /// `service.name`.
    pub name: Option<String>,
    /// `service.version`.
    pub version: Option<String>,
    /// `service.namespace`.
    pub namespace: Option<String>,
    /// `deployment.environment.name` (`production`, `staging`, ...).
    pub environment: Option<String>,
    /// `service.instance.id`. Unique per process; Kerniq generates one when
    /// not set.
    pub instance_id: Option<String>,
    /// Additional resource attributes (region, cluster, team ...).
    pub attributes: Vec<KeyValue>,
}

impl ServiceInfo {
    /// All configured attributes, `service.name` excluded (the SDK resource
    /// builder handles it separately so it can apply its fallback).
    pub fn to_attributes(&self) -> Vec<KeyValue> {
        let mut out = Vec::with_capacity(4 + self.attributes.len());
        if let Some(v) = &self.version {
            out.push(KeyValue::new(SERVICE_VERSION, v.clone()));
        }
        if let Some(v) = &self.namespace {
            out.push(KeyValue::new(SERVICE_NAMESPACE, v.clone()));
        }
        if let Some(v) = &self.environment {
            out.push(KeyValue::new(DEPLOYMENT_ENVIRONMENT, v.clone()));
        }
        if let Some(v) = &self.instance_id {
            out.push(KeyValue::new(SERVICE_INSTANCE_ID, v.clone()));
        }
        out.extend(self.attributes.iter().cloned());
        out
    }
}

/// A process-unique instance id: `<hostname>-<pid>-<start time>`, without
/// requiring a UUID dependency. Stable for the life of the process.
pub fn default_instance_id() -> String {
    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "localhost".to_owned());
    let started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    format!("{host}-{}-{started:x}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes() {
        let info = ServiceInfo {
            name: Some("payment-api".into()),
            version: Some("1.0.0".into()),
            environment: Some("production".into()),
            attributes: vec![KeyValue::new("cloud.region", "eu-west-1")],
            ..ServiceInfo::default()
        };
        let attrs = info.to_attributes();
        let keys: Vec<_> = attrs.iter().map(|kv| kv.key.as_str()).collect();
        assert_eq!(
            keys,
            [SERVICE_VERSION, DEPLOYMENT_ENVIRONMENT, "cloud.region"]
        );
    }

    #[test]
    fn instance_id_contains_pid() {
        assert!(default_instance_id().contains(&std::process::id().to_string()));
    }
}
