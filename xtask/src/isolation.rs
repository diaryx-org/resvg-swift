//! `cargo xtask package-isolation` — check each crate alone, as a consumer
//! would build it. CI runs it as the `package-isolation` job in
//! `.config/ship.figl`.
//!
//! Workspace feature unification means `cargo check --workspace` can pass when
//! a crate cannot compile on its own. Each row checks one crate the way a host
//! actually builds it.

use crate::util::{cargo, run};
use anyhow::Result;

/// A new workspace member belongs in this list; the test below is what says so.
const ISOLATED: &[&[&str]] = &[
    &["-p", "usvg-flatten"],
    &["-p", "resvg-uniffi"],
    &["-p", "xtask"],
];

pub fn run_task() -> Result<()> {
    for spec in ISOLATED {
        let mut args = vec!["check"];
        args.extend_from_slice(spec);
        run(cargo().args(&args))?;
    }
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
                "workspace member `{member}` is not in isolation::ISOLATED"
            );
        }
        Ok(())
    }
}
