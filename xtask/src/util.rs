//! Shared plumbing for the tasks: where the repo is and how a subprocess is run.

use anyhow::{Result, bail};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The repo root. xtask lives at `<root>/xtask`, so every path a task builds is
/// anchored here rather than at the caller's cwd — which is why `cargo xtask`
/// behaves the same from any subdirectory.
///
/// Asked at runtime rather than baked in with `env!`: cargo sets
/// `CARGO_MANIFEST_DIR` for what it runs, and a checkout that has been moved
/// would otherwise carry a stale compile-time path along in `target/`.
pub fn root() -> PathBuf {
    let from_manifest = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .and_then(|dir| dir.parent().map(Path::to_path_buf))
        .filter(|root| is_root(root));
    from_manifest
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.ancestors().find(|d| is_root(d)).map(Path::to_path_buf))
        })
        .unwrap_or_else(|| {
            panic!("could not locate the repo root — run this through `cargo xtask`")
        })
}

fn is_root(dir: &Path) -> bool {
    dir.join("xtask/Cargo.toml").is_file()
}

/// A command whose working directory is the repo root.
pub fn cmd(program: impl AsRef<OsStr>) -> Command {
    let mut c = Command::new(program);
    c.current_dir(root());
    c
}

/// `cargo`, rooted at the workspace — the one that invoked us, so a task run
/// through `cargo +nightly xtask` doesn't switch toolchains halfway.
pub fn cargo() -> Command {
    cmd(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
}

/// Run a command to completion, inheriting stdio; an error names it.
pub fn run(command: &mut Command) -> Result<()> {
    let status = command.status()?;
    if !status.success() {
        bail!("{command:?} failed with {status}");
    }
    Ok(())
}
