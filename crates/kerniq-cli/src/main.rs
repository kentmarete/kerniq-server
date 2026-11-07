//! `kerniq`: project scaffolding and development workflow for Kerniq Server.
//!
//! ```text
//! kerniq new customer-api --template production
//! kerniq dev
//! kerniq check
//! kerniq test
//! kerniq build
//! ```

mod commands;
mod project;
mod templates;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::templates::Template;

#[derive(Parser)]
#[command(
    name = "kerniq",
    version,
    about = "Create and develop Kerniq Server applications"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new project.
    New {
        /// Package name (lowercase letters, digits, `-`, `_`).
        name: String,
        /// Project template.
        #[arg(long, short, value_enum, default_value_t = Template::Basic)]
        template: Template,
        /// Directory in which to create the project (default: current directory).
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Depend on a local Kerniq Server checkout instead of crates.io
        /// (also settable with KERNIQ_SERVER_PATH).
        #[arg(long, value_name = "PATH", conflicts_with = "kerniq_git")]
        kerniq_path: Option<PathBuf>,
        /// Depend on a Kerniq Server git repository instead of crates.io.
        #[arg(long, value_name = "URL")]
        kerniq_git: Option<String>,
    },
    /// Run the project and restart it when sources change.
    Dev {
        /// Arguments passed to `cargo run` (after `--`).
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Build the project in release mode.
    Build {
        /// Extra arguments for `cargo build`.
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Check formatting, compile all targets and run Clippy with warnings denied.
    Check {
        /// Extra arguments for `cargo check` and `cargo clippy`.
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Run the test suite.
    Test {
        /// Extra arguments for `cargo test`.
        #[arg(last = true)]
        args: Vec<String>,
    },
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Command::New {
            name,
            template,
            dir,
            kerniq_path,
            kerniq_git,
        } => commands::new::run(commands::new::NewArgs {
            name,
            template,
            dir,
            kerniq_path,
            kerniq_git,
        }),
        Command::Dev { args } => commands::dev::run(&args),
        Command::Build { args } => commands::cargo::run_steps(commands::cargo::BUILD, &args),
        Command::Check { args } => commands::cargo::run_steps(commands::cargo::CHECK, &args),
        Command::Test { args } => commands::cargo::run_steps(commands::cargo::TEST, &args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
