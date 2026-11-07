//! Project name validation and generation.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::templates::{CARGO_TOML, GITIGNORE, Template};

/// Where generated projects get `kerniq-server` from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KerniqSource {
    /// A crates.io version requirement.
    Version(String),
    /// A local checkout (the repository root or the `kerniq-server` crate).
    Path(PathBuf),
    /// A git repository.
    Git(String),
}

impl KerniqSource {
    /// Renders the dependency as an inline TOML table.
    pub fn dependency(&self, features: &[&str]) -> String {
        let source = match self {
            KerniqSource::Version(v) => format!("version = {}", toml_string(v)),
            KerniqSource::Path(p) => format!("path = {}", toml_string(&p.to_string_lossy())),
            KerniqSource::Git(url) => format!("git = {}", toml_string(url)),
        };
        if features.is_empty() {
            format!("{{ {source} }}")
        } else {
            let list: Vec<String> = features.iter().map(|f| toml_string(f)).collect();
            format!("{{ {source}, features = [{}] }}", list.join(", "))
        }
    }

    /// Resolves a user-supplied path to the `kerniq-server` crate directory.
    pub fn from_path(path: &Path) -> Result<Self, ProjectError> {
        let path = path
            .canonicalize()
            .map_err(|e| ProjectError::Io(format!("cannot resolve `{}`: {e}", path.display())))?;
        for candidate in [path.join("crates/kerniq-server"), path.clone()] {
            let manifest = candidate.join("Cargo.toml");
            if fs::read_to_string(&manifest)
                .map(|text| text.contains("name = \"kerniq-server\""))
                .unwrap_or(false)
            {
                return Ok(KerniqSource::Path(candidate));
            }
        }
        Err(ProjectError::InvalidKerniqPath(path))
    }
}

fn toml_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Errors from project generation.
#[derive(Debug)]
pub enum ProjectError {
    InvalidName(String, &'static str),
    AlreadyExists(PathBuf),
    InvalidKerniqPath(PathBuf),
    Io(String),
}

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectError::InvalidName(name, reason) => {
                write!(f, "invalid project name `{name}`: {reason}")
            }
            ProjectError::AlreadyExists(path) => write!(
                f,
                "destination `{}` already exists; choose another name or remove it",
                path.display()
            ),
            ProjectError::InvalidKerniqPath(path) => write!(
                f,
                "`{}` is neither a Kerniq Server checkout nor the `kerniq-server` crate",
                path.display()
            ),
            ProjectError::Io(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ProjectError {}

const RESERVED: &[&str] = &[
    // Rust keywords (strict and reserved).
    "as",
    "async",
    "await",
    "break",
    "const",
    "continue",
    "crate",
    "dyn",
    "else",
    "enum",
    "extern",
    "false",
    "fn",
    "for",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "pub",
    "ref",
    "return",
    "self",
    "static",
    "struct",
    "super",
    "trait",
    "true",
    "type",
    "unsafe",
    "use",
    "where",
    "while",
    "abstract",
    "become",
    "box",
    "do",
    "final",
    "gen",
    "macro",
    "override",
    "priv",
    "try",
    "typeof",
    "unsized",
    "virtual",
    "yield",
    // Names that clash with the standard library, Cargo targets or Kerniq.
    "std",
    "core",
    "alloc",
    "proc_macro",
    "test",
    "build",
    "deps",
    "examples",
    "kerniq",
    "kerniq-server",
    "kerniq_server",
];

/// Validates a package name: ASCII lowercase letters, digits, `-` and `_`,
/// starting with a letter, at most 64 characters, not a keyword or a
/// reserved name.
pub fn validate_name(name: &str) -> Result<(), ProjectError> {
    let invalid = |reason| Err(ProjectError::InvalidName(name.to_owned(), reason));
    if name.is_empty() {
        return invalid("the name is empty");
    }
    if name.len() > 64 {
        return invalid("names are limited to 64 characters");
    }
    if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
        return invalid("names must start with a lowercase ASCII letter");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return invalid("use only lowercase ASCII letters, digits, `-` and `_`");
    }
    if RESERVED.contains(&name) || RESERVED.contains(&crate_ident(name).as_str()) {
        return invalid("this name is a Rust keyword or otherwise reserved");
    }
    Ok(())
}

/// The crate identifier for a package name (`customer-api` → `customer_api`).
pub fn crate_ident(name: &str) -> String {
    name.replace('-', "_")
}

/// Options for [`generate`].
#[derive(Debug, Clone)]
pub struct NewProject {
    pub name: String,
    pub template: Template,
    pub parent: PathBuf,
    pub source: KerniqSource,
}

/// Generates a project and returns its directory. Nothing is written if
/// validation fails; a partially written directory is removed on error.
pub fn generate(options: &NewProject) -> Result<PathBuf, ProjectError> {
    validate_name(&options.name)?;
    let root = options.parent.join(&options.name);
    if root.exists() {
        return Err(ProjectError::AlreadyExists(root));
    }
    let result = write_files(options, &root);
    if result.is_err() {
        let _ = fs::remove_dir_all(&root);
    }
    result.map(|()| root)
}

fn write_files(options: &NewProject, root: &Path) -> Result<(), ProjectError> {
    let template = options.template;
    let render = |text: &str| {
        text.replace("{{name}}", &options.name)
            .replace("{{crate}}", &crate_ident(&options.name))
            .replace(
                "{{kerniq_dep}}",
                &options.source.dependency(template.kerniq_features()),
            )
            .replace("{{extra_deps}}", template.extra_dependencies())
    };
    let write = |relative: &str, contents: String| -> Result<(), ProjectError> {
        let path = root.join(relative);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)
                .map_err(|e| ProjectError::Io(format!("cannot create `{}`: {e}", dir.display())))?;
        }
        fs::write(&path, contents)
            .map_err(|e| ProjectError::Io(format!("cannot write `{}`: {e}", path.display())))
    };

    write("Cargo.toml", render(CARGO_TOML))?;
    write(".gitignore", GITIGNORE.to_owned())?;
    for file in template.files() {
        write(file.path, render(file.contents))?;
    }
    write("README.md", render(&readme(template)))?;
    Ok(())
}

fn readme(template: Template) -> String {
    let description = match template {
        Template::Basic => "A minimal Kerniq Server application.",
        Template::Rest => "A REST API built with Kerniq Server.",
        Template::Modular => "A modular Kerniq Server application.",
        Template::Production => "A production-ready Kerniq Server service.",
    };
    let port = if template == Template::Production {
        8080
    } else {
        3000
    };
    format!(
        "# {{{{name}}}}\n\n{description}\n\n## Run\n\n```sh\nkerniq dev      # run with automatic restart on changes\ncargo run       # or run directly\n```\n\nThe server listens on http://127.0.0.1:{port} by default.\n\n## Test\n\n```sh\ncargo test\n```\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_validation() {
        for ok in ["customer-api", "api", "a1", "billing_service"] {
            validate_name(ok).unwrap();
        }
        for bad in [
            "", "Customer", "1api", "-api", "my api", "fn", "test", "std", "kerniq", "async",
            "über",
        ] {
            assert!(validate_name(bad).is_err(), "{bad} should be rejected");
        }
        assert!(validate_name(&"a".repeat(65)).is_err());
    }

    #[test]
    fn dependency_rendering() {
        assert_eq!(
            KerniqSource::Version("0.1".into()).dependency(&[]),
            r#"{ version = "0.1" }"#
        );
        assert_eq!(
            KerniqSource::Path("/x/y".into()).dependency(&["cors"]),
            r#"{ path = "/x/y", features = ["cors"] }"#
        );
        assert_eq!(
            KerniqSource::Path(r#"C:\a"b"#.into()).dependency(&[]),
            r#"{ path = "C:\\a\"b" }"#
        );
    }

    #[test]
    fn generates_every_template_without_leftover_placeholders() {
        let dir = std::env::temp_dir().join(format!("kerniq-cli-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for (i, template) in [
            Template::Basic,
            Template::Rest,
            Template::Modular,
            Template::Production,
        ]
        .into_iter()
        .enumerate()
        {
            let name = format!("app-{i}");
            let root = generate(&NewProject {
                name: name.clone(),
                template,
                parent: dir.clone(),
                source: KerniqSource::Version("0.1".into()),
            })
            .unwrap();
            for file in template
                .files()
                .iter()
                .map(|f| f.path)
                .chain(["Cargo.toml", "README.md"])
            {
                let text = fs::read_to_string(root.join(file)).unwrap();
                assert!(
                    !text.contains("{{"),
                    "{file} in {template:?} has a placeholder"
                );
            }
            let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
            assert!(manifest.contains(&format!("name = \"{name}\"")));

            // A second attempt refuses to overwrite.
            let err = generate(&NewProject {
                name,
                template,
                parent: dir.clone(),
                source: KerniqSource::Version("0.1".into()),
            })
            .unwrap_err();
            assert!(matches!(err, ProjectError::AlreadyExists(_)));
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
