//! `kerniq new`.

use std::path::PathBuf;

use crate::project::{self, KerniqSource, NewProject};
use crate::templates::Template;

/// The `kerniq-server` version requirement used by default.
pub const DEFAULT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct NewArgs {
    pub name: String,
    pub template: Template,
    pub dir: Option<PathBuf>,
    pub kerniq_path: Option<PathBuf>,
    pub kerniq_git: Option<String>,
}

pub fn run(args: NewArgs) -> Result<(), Box<dyn std::error::Error>> {
    let source = match (args.kerniq_path, args.kerniq_git) {
        (Some(path), _) => KerniqSource::from_path(&path)?,
        (None, Some(url)) => KerniqSource::Git(url),
        (None, None) => match std::env::var_os("KERNIQ_SERVER_PATH") {
            Some(path) => KerniqSource::from_path(&PathBuf::from(path))?,
            None => KerniqSource::Version(DEFAULT_VERSION.to_owned()),
        },
    };
    let parent = match args.dir {
        Some(dir) => dir,
        None => std::env::current_dir()?,
    };
    let root = project::generate(&NewProject {
        name: args.name.clone(),
        template: args.template,
        parent,
        source,
    })?;

    println!(
        "Created {:?} project `{}` in {}",
        args.template,
        args.name,
        root.display()
    );
    println!();
    println!("Next steps:");
    println!("  cd {}", root.display());
    println!("  kerniq dev");
    Ok(())
}
