use anyhow::{Context as _, Result, bail, ensure};
use egui_tester_witness::{Error as WitnessError, ObservationJournal};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

const STARTUP_LIMIT: Duration = Duration::from_secs(45);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

fn main() -> Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    let mode = arguments.next();
    ensure!(
        arguments.next().is_none(),
        "abv-portability accepts at most one mode"
    );
    match mode.as_deref() {
        None => prove_present(),
        Some(mode) if mode == "present" => prove_present(),
        Some(mode) if mode == "lifecycle" => prove_lifecycle(),
        Some(mode) => bail!(
            "unknown abv-portability mode {}; expected `present` or `lifecycle`",
            Path::new(mode).display()
        ),
    }
}

fn prove_present() -> Result<()> {
    let binary = binary()?;
    prove_identity(&binary)?;
    let cell = Cell::forge("present")?;
    let witness = cell.path().join("abv.observations");
    let frames = cell.path().join("abv.frames");
    let launch = format!(
        "abv-portability-{}-{}",
        std::env::consts::OS,
        std::process::id()
    );
    let mut command = Command::new(&binary);
    let _command = command
        .arg("--pause-mirror")
        .env("EGUI_TESTER_WITNESS", &witness)
        .env("EGUI_TESTER_FRAMES", &frames)
        .env("EGUI_TESTER_LAUNCH", &launch)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    isolate_host_paths(&mut command, cell.path())?;

    let child = command
        .spawn()
        .with_context(|| format!("launch {}", binary.display()))?;
    let mut captive = Captive::new(child);
    let verdict = await_first_present(captive.child_mut()?, &witness, &launch)?;
    let output = captive.finish()?;
    std::fs::write(cell.path().join("abv.stdout"), &output.stdout)
        .context("retain ABV portability stdout")?;
    std::fs::write(cell.path().join("abv.stderr"), &output.stderr)
        .context("retain ABV portability stderr")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    match verdict {
        Startup::Ready => {}
        Startup::Exited(status) => bail!(
            "ABV exited before its first witnessed frame with {status}\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ),
        Startup::TimedOut => bail!(
            "ABV presented no witnessed surface within {STARTUP_LIMIT:?}\nstdout:\n{stdout}\nstderr:\n{stderr}"
        ),
    }
    println!(
        "ABV portability passed: {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    Ok(())
}

fn prove_lifecycle() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .context("resolve ABV workspace root")?;
    let cell = tempfile::Builder::new()
        .prefix("abv-lifecycle-")
        .tempdir()
        .context("forge installation lifecycle cell")?;
    let prefix = cell.path().join("prefix");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());

    let mut install = Command::new(&cargo);
    let _install = install
        .arg("install")
        .arg("--path")
        .arg(root.join("crates/adequate_booru_viewer"))
        .arg("--bin")
        .arg("abv")
        .arg("--root")
        .arg(&prefix)
        .args(["--locked", "--force"]);
    run_checked(&mut install, "install ordinary ABV product")?;

    let binary = prefix
        .join("bin")
        .join(format!("abv{}", std::env::consts::EXE_SUFFIX));
    ensure!(
        binary.is_file(),
        "cargo install omitted {}",
        binary.display()
    );
    prove_identity(&binary)?;

    let mut uninstall = Command::new(cargo);
    let _uninstall = uninstall
        .arg("uninstall")
        .arg("--root")
        .arg(&prefix)
        .arg("adequate_booru_viewer");
    run_checked(&mut uninstall, "uninstall ordinary ABV product")?;
    ensure!(
        !binary.exists(),
        "cargo uninstall left {}",
        binary.display()
    );
    println!(
        "ABV lifecycle passed: {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    Ok(())
}

fn run_checked(command: &mut Command, operation: &str) -> Result<()> {
    let invocation = format!("{command:?}");
    let output = command
        .output()
        .with_context(|| format!("{operation}: {invocation}"))?;
    ensure!(
        output.status.success(),
        "{operation} failed with {}: {invocation}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn binary() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("ABV_PORTABILITY_BINARY") {
        return Ok(PathBuf::from(path));
    }
    let executable = std::env::current_exe().context("resolve portability executable")?;
    let parent = executable
        .parent()
        .context("portability executable has no parent")?;
    Ok(parent.join(format!("abv{}", std::env::consts::EXE_SUFFIX)))
}

fn prove_identity(binary: &Path) -> Result<()> {
    let version = Command::new(binary)
        .arg("--version")
        .output()
        .with_context(|| format!("run {} --version", binary.display()))?;
    let stdout = String::from_utf8_lossy(&version.stdout);
    let stderr = String::from_utf8_lossy(&version.stderr);
    ensure!(
        version.status.success(),
        "{} --version failed with {}\nstdout:\n{stdout}\nstderr:\n{stderr}",
        binary.display(),
        version.status
    );
    ensure!(
        stdout.trim().starts_with("abv "),
        "{} --version returned an alien identity: {stdout:?}",
        binary.display()
    );
    Ok(())
}

fn isolate_host_paths(command: &mut Command, root: &Path) -> Result<()> {
    let home = root.join("home");
    let roots = [
        ("HOME", home.clone()),
        ("USERPROFILE", home.clone()),
        ("APPDATA", home.join("AppData").join("Roaming")),
        ("LOCALAPPDATA", home.join("AppData").join("Local")),
        ("XDG_CACHE_HOME", root.join("cache")),
        ("XDG_CONFIG_HOME", root.join("config")),
        ("XDG_DATA_HOME", root.join("data")),
        ("XDG_STATE_HOME", root.join("state")),
        ("XDG_RUNTIME_DIR", root.join("runtime")),
    ];
    for (name, path) in roots {
        std::fs::create_dir_all(&path)
            .with_context(|| format!("create isolated {name} at {}", path.display()))?;
        let _command = command.env(name, path);
    }
    Ok(())
}

fn await_first_present(child: &mut Child, path: &Path, launch: &str) -> Result<Startup> {
    let begun = Instant::now();
    let mut journal = ObservationJournal::sealed(path, launch);
    while begun.elapsed() < STARTUP_LIMIT {
        if let Some(status) = child.try_wait().context("poll ABV process")? {
            return Ok(Startup::Exited(status));
        }
        match journal.read_new::<Value>() {
            Ok(frames) => {
                for frame in frames {
                    if presented(&frame)? {
                        return Ok(Startup::Ready);
                    }
                }
            }
            Err(WitnessError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("read ABV witness"),
        }
        thread::sleep(POLL_INTERVAL);
    }
    Ok(Startup::TimedOut)
}

fn presented(frame: &Value) -> Result<bool> {
    let Some(state) = frame.get("state") else {
        return Ok(false);
    };
    let contract = state
        .get("contract")
        .and_then(Value::as_str)
        .context("witness state has no contract")?;
    ensure!(
        contract == abv_contract::UI_FINGERPRINT,
        "ABV UI contract mismatch: expected {}, observed {contract}",
        abv_contract::UI_FINGERPRINT
    );
    Ok(frame
        .get("surface_sequence")
        .and_then(Value::as_u64)
        .is_some_and(|sequence| sequence > 0))
}

enum Startup {
    Ready,
    Exited(ExitStatus),
    TimedOut,
}

struct Cell {
    path: PathBuf,
    _temporary: Option<tempfile::TempDir>,
}

impl Cell {
    fn forge(label: &str) -> Result<Self> {
        if let Some(path) = std::env::var_os("ABV_PORTABILITY_ARTIFACTS") {
            let path = PathBuf::from(path).join(format!(
                "{}-{}-{}-{label}",
                std::env::consts::OS,
                std::env::consts::ARCH,
                std::process::id()
            ));
            std::fs::create_dir_all(&path)
                .with_context(|| format!("create portability artifacts at {}", path.display()))?;
            return Ok(Self {
                path,
                _temporary: None,
            });
        }
        let temporary = tempfile::tempdir().context("forge portability cell")?;
        Ok(Self {
            path: temporary.path().to_path_buf(),
            _temporary: Some(temporary),
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

struct Captive(Option<Child>);

impl Captive {
    const fn new(child: Child) -> Self {
        Self(Some(child))
    }

    fn child_mut(&mut self) -> Result<&mut Child> {
        self.0.as_mut().context("ABV child was already reaped")
    }

    fn finish(mut self) -> Result<Output> {
        let mut child = self.0.take().context("ABV child was already reaped")?;
        if child
            .try_wait()
            .context("poll ABV before teardown")?
            .is_none()
        {
            child
                .kill()
                .context("terminate ABV after portability proof")?;
        }
        child.wait_with_output().context("reap ABV process")
    }
}

impl Drop for Captive {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _killed = child.kill();
            let _reaped = child.wait();
        }
    }
}
