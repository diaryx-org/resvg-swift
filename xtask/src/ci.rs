//! `cargo xtask ci` — the checks a release has to pass, as one program.
//!
//! `.github/workflows/ci.yml` runs this same binary, so the table below is the
//! only statement of what "green" means. Ordered cheapest-first, so a run that
//! is going to fail fails in seconds.

use crate::util::{cargo, cmd, run};
use anyhow::{Result, bail};

/// One check: what to call it, and the work itself.
pub struct Job {
    /// `cargo xtask ci <id>`.
    pub id: &'static str,
    /// The heading printed above it in a full run.
    pub name: &'static str,
    /// One line, for `cargo xtask ci --list`.
    pub about: &'static str,
    run: fn() -> Result<()>,
}

/// The whole of CI, in the order [`run_all`] runs it.
pub const JOBS: &[Job] = &[
    Job {
        id: "fmt",
        name: "Format",
        about: "rustfmt, in check mode",
        run: fmt,
    },
    Job {
        id: "clippy",
        name: "Clippy",
        about: "clippy over every target, warnings denied",
        run: clippy,
    },
    Job {
        id: "test",
        name: "Test",
        about: "the workspace test suite",
        run: test,
    },
    Job {
        id: "package-isolation",
        name: "Package isolation",
        about: "check each crate alone, as a consumer would build it",
        run: package_isolation,
    },
    Job {
        id: "bindings",
        name: "Swift bindings",
        about: "the committed UniFFI binding is what crates/resvg-swift produces",
        run: bindings,
    },
];

fn fmt() -> Result<()> {
    run(cargo().args(["fmt", "--all", "--check"]))
}

/// Warnings are errors here because they are errors in review.
fn clippy() -> Result<()> {
    run(cargo().args([
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ]))
}

fn test() -> Result<()> {
    run(cargo().args(["test", "--workspace"]))
}

/// Workspace feature unification means `cargo check --workspace` can pass when
/// a crate cannot compile on its own. Each row checks one crate the way a host
/// actually builds it. A new workspace member belongs in this list; the test
/// below is what says so.
const ISOLATED: &[&[&str]] = &[
    &["-p", "usvg-flatten"],
    &["-p", "resvg-swift"],
    &["-p", "xtask"],
];

fn package_isolation() -> Result<()> {
    for spec in ISOLATED {
        let mut args = vec!["check"];
        args.extend_from_slice(spec);
        run(cargo().args(&args))?;
    }
    Ok(())
}

fn bindings() -> Result<()> {
    run(cmd("bash").args(["scripts/gen-bindings.sh", "--check"]))
}

#[derive(clap::Args)]
pub struct Args {
    /// Run one job by id instead of every job.
    job: Option<String>,

    /// List the jobs and exit.
    #[arg(long)]
    list: bool,
}

pub fn run_task(args: Args) -> Result<()> {
    if args.list {
        for job in JOBS {
            println!("  {:<18}{}", job.id, job.about);
        }
        return Ok(());
    }
    match args.job {
        Some(id) => match JOBS.iter().find(|job| job.id == id) {
            Some(job) => (job.run)(),
            None => bail!("no CI job `{id}`; `cargo xtask ci --list` names them"),
        },
        None => run_all(),
    }
}

fn run_all() -> Result<()> {
    for job in JOBS {
        println!("▸ {}", job.name);
        (job.run)()?;
    }
    println!("✓ CI green");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ISOLATED;
    use anyhow::{Context, Result};

    /// Every workspace member is checked in isolation, so adding a crate to
    /// Cargo.toml without adding it here fails here.
    #[test]
    fn every_member_is_isolated() -> Result<()> {
        let manifest = std::fs::read_to_string(crate::util::root().join("Cargo.toml"))
            .context("reading the workspace manifest")?;
        let members = manifest
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("\"crates/") || line.starts_with("\"xtask\""))
            .map(|line| line.trim_matches(|c| c == '"' || c == ','))
            .map(|path| path.rsplit('/').next().unwrap_or(path))
            .collect::<Vec<_>>();
        assert!(!members.is_empty(), "no members parsed from Cargo.toml");
        for member in members {
            assert!(
                ISOLATED.iter().any(|spec| spec.contains(&member)),
                "workspace member `{member}` is not in ci::ISOLATED"
            );
        }
        Ok(())
    }
}
