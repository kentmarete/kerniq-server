//! `kerniq build`, `kerniq check` and `kerniq test`: thin wrappers around
//! Cargo so the whole workflow lives under one command.

use std::process::{Command, ExitStatus};

/// One Cargo invocation: `cargo <args> <user extra args> <trailing>`.
pub struct Step {
    pub args: &'static [&'static str],
    /// Whether the user's extra arguments are passed to this step.
    pub accepts_extra: bool,
    /// Arguments placed after the user's (e.g. `-- -D warnings`).
    pub trailing: &'static [&'static str],
}

/// Runs `cargo <args>` with inherited stdio.
pub fn cargo(args: &[&str], extra: &[String]) -> std::io::Result<ExitStatus> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    Command::new(cargo).args(args).args(extra).status()
}

/// Runs each step in order, stopping at the first failure.
pub fn run_steps(steps: &[Step], extra: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    for step in steps {
        let mut args: Vec<String> = step.args.iter().map(|s| (*s).to_owned()).collect();
        if step.accepts_extra {
            args.extend(extra.iter().cloned());
        }
        args.extend(step.trailing.iter().map(|s| (*s).to_owned()));
        eprintln!("==> cargo {}", args.join(" "));
        let status = cargo(&[], &args)?;
        if !status.success() {
            return Err(format!("`cargo {}` failed ({status})", args.join(" ")).into());
        }
    }
    Ok(())
}

pub const BUILD: &[Step] = &[Step {
    args: &["build", "--release"],
    accepts_extra: true,
    trailing: &[],
}];

pub const CHECK: &[Step] = &[
    Step {
        args: &["fmt", "--", "--check"],
        accepts_extra: false,
        trailing: &[],
    },
    Step {
        args: &["check", "--all-targets"],
        accepts_extra: true,
        trailing: &[],
    },
    Step {
        args: &["clippy", "--all-targets"],
        accepts_extra: true,
        trailing: &["--", "-D", "warnings"],
    },
];

pub const TEST: &[Step] = &[Step {
    args: &["test"],
    accepts_extra: true,
    trailing: &[],
}];
