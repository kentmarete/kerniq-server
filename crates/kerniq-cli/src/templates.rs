//! Embedded project templates.
//!
//! Templates are ordinary files under `src/templates/`, embedded at compile
//! time. Placeholders: `{{name}}` (package name), `{{crate}}` (crate
//! identifier), `{{kerniq_dep}}` (dependency specification) and
//! `{{extra_deps}}` (template-specific dependencies).

use clap::ValueEnum;

/// A file in a generated project.
pub struct TemplateFile {
    pub path: &'static str,
    pub contents: &'static str,
}

macro_rules! file {
    ($path:literal, $source:literal) => {
        TemplateFile {
            path: $path,
            contents: include_str!(concat!("templates/", $source)),
        }
    };
}

/// The available project templates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Template {
    /// Minimal application with one endpoint.
    Basic,
    /// REST API with routes, handlers, models and an in-memory store.
    Rest,
    /// Feature-oriented layout: config, state, errors, models, services, routes.
    Modular,
    /// Logging, configuration, graceful shutdown, health probes, metrics, middleware.
    Production,
}

impl Template {
    /// Kerniq features the template needs.
    pub fn kerniq_features(self) -> &'static [&'static str] {
        match self {
            Template::Basic | Template::Rest | Template::Modular => &[],
            Template::Production => &["cors", "compression", "config-file"],
        }
    }

    /// Extra `[dependencies]` lines.
    pub fn extra_dependencies(self) -> &'static str {
        match self {
            Template::Production => {
                "tracing = \"0.1\"\ntracing-subscriber = { version = \"0.3\", features = [\"env-filter\", \"json\"] }\n"
            }
            _ => "",
        }
    }

    /// The template's files (excluding the shared `Cargo.toml` and `.gitignore`).
    pub fn files(self) -> Vec<TemplateFile> {
        match self {
            Template::Basic => vec![file!("src/main.rs", "basic/main.rs.tmpl")],
            Template::Rest => vec![
                file!("src/main.rs", "rest/main.rs.tmpl"),
                file!("src/handlers.rs", "rest/handlers.rs.tmpl"),
                file!("src/models.rs", "rest/models.rs.tmpl"),
                file!("src/store.rs", "rest/store.rs.tmpl"),
            ],
            Template::Modular => vec![
                file!("src/main.rs", "modular/main.rs.tmpl"),
                file!("src/config.rs", "modular/config.rs.tmpl"),
                file!("src/state.rs", "modular/state.rs.tmpl"),
                file!("src/error.rs", "modular/error.rs.tmpl"),
                file!("src/models/mod.rs", "modular/models/mod.rs.tmpl"),
                file!("src/models/user.rs", "modular/models/user.rs.tmpl"),
                file!("src/services/mod.rs", "modular/services/mod.rs.tmpl"),
                file!("src/services/users.rs", "modular/services/users.rs.tmpl"),
                file!("src/routes/mod.rs", "modular/routes/mod.rs.tmpl"),
                file!("src/routes/health.rs", "modular/routes/health.rs.tmpl"),
                file!("src/routes/users.rs", "modular/routes/users.rs.tmpl"),
            ],
            Template::Production => vec![
                file!("src/main.rs", "production/main.rs.tmpl"),
                file!("src/config.rs", "production/config.rs.tmpl"),
                file!("src/state.rs", "production/state.rs.tmpl"),
                file!("src/routes.rs", "production/routes.rs.tmpl"),
                file!("kerniq.toml", "production/kerniq.toml.tmpl"),
                file!(".env.example", "production/env.example.tmpl"),
                file!("Dockerfile", "production/Dockerfile.tmpl"),
            ],
        }
    }
}

/// `Cargo.toml` shared by every template.
pub const CARGO_TOML: &str = include_str!("templates/common/Cargo.toml.tmpl");
/// `.gitignore` shared by every template.
pub const GITIGNORE: &str = include_str!("templates/common/gitignore.tmpl");
