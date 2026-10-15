//! The workspace's task runner — the [cargo-xtask] pattern: a plain Rust binary,
//! invoked as `cargo xtask <task>` through the alias in `.cargo/config.toml`, so
//! every developer already has the only toolchain it needs.
//!
//! CI and releasing are not here. Both are `ship`, configured by
//! `.config/ship.figl`: `ship ci` runs the checks (one of them calls
//! `cargo xtask package-isolation`), and `ship` cuts a release.
//!
//! [cargo-xtask]: https://github.com/matklad/cargo-xtask

mod isolation;
mod util;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "cargo xtask",
    about = "The steps that leave Rust, and the checks too big for one command",
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    task: Task,
}

#[derive(Subcommand)]
enum Task {
    /// Check each crate alone, as a consumer would build it.
    PackageIsolation,
    /// (Re)generate the committed UniFFI Swift binding from crates/resvg-uniffi.
    Bindings,
    /// Moved: CI is `ship ci` now, configured by `.config/ship.figl`.
    #[command(hide = true)]
    Ci {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        rest: Vec<String>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().task {
        Task::PackageIsolation => isolation::run_task(),
        Task::Bindings => util::run(util::cmd("bash").arg("scripts/gen-bindings.sh")),
        Task::Ci { .. } => {
            anyhow::bail!(
                "`cargo xtask ci` is `ship ci` now (`ship ci <job>` for one job, `ship ci --list` for the jobs)"
            )
        }
    }
}
