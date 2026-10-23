use anyhow::{Context as _, Result, bail};
use std::{
    env,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::Command,
};

fn main() -> Result<()> {
    match env::args().nth(1).as_deref() {
        Some("release-build") => release_build(),
        Some(other) => bail!("unknown xtask `{other}`; try `cargo release-build`"),
        None => bail!("missing xtask; try `cargo release-build`"),
    }
}

/// Verify the checkout, then atomically replace the user's installed `abv`
/// with a release build from this exact source tree. `cargo install` only
/// touches the destination after a successful compile, so a broken checkout
/// cannot dislodge the previous known-good binary.
fn release_build() -> Result<()> {
    let root = workspace_root()?;
    run("./check.py", ["verify"], &root)?;
    run("cargo", ["audit"], &root)?;
    let mut install = [
        "install",
        "--path",
        "crates/adequate_booru_viewer",
        "--locked",
        "--offline",
        "--force",
        "--bin",
        "abv",
    ]
    .map(OsString::from)
    .to_vec();
    if let Some(prefix) = developer_install_prefix()? {
        install.extend([OsString::from("--root"), prefix.into_os_string()]);
    }
    run("cargo", install, &root)
}

fn workspace_root() -> Result<PathBuf> {
    let output = Command::new("cargo")
        .args(["locate-project", "--workspace", "--message-format", "plain"])
        .output()
        .context("locate Cargo workspace")?;
    if !output.status.success() {
        bail!(
            "cargo locate-project exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let manifest = String::from_utf8(output.stdout).context("workspace path is not UTF-8")?;
    PathBuf::from(manifest.trim())
        .parent()
        .map(Path::to_path_buf)
        .context("Cargo workspace manifest has no parent")
}

/// Linux user executables canonically live in `~/.local/bin`. Other hosts
/// retain Cargo's native install prefix unless explicitly overridden.
fn developer_install_prefix() -> Result<Option<PathBuf>> {
    if let Some(prefix) = env::var_os("ABV_INSTALL_ROOT") {
        let prefix = PathBuf::from(prefix);
        if !prefix.is_absolute() {
            bail!("ABV_INSTALL_ROOT must be absolute");
        }
        return Ok(Some(prefix));
    }
    #[cfg(target_os = "linux")]
    {
        let home = env::var_os("HOME").context("HOME is required for the local abv install")?;
        Ok(Some(PathBuf::from(home).join(".local")))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(None)
    }
}

fn run(program: &str, args: impl IntoIterator<Item = impl AsRef<OsStr>>, cwd: &Path) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .status()
        .with_context(|| format!("spawn {program}"))?;
    if status.success() {
        Ok(())
    } else {
        bail!("{program} exited with {status}")
    }
}
