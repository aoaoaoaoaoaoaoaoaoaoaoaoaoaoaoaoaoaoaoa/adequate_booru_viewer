#![expect(
    unused_crate_dependencies,
    reason = "startup integration test launches the binary instead of linking app dependencies"
)]

use anyhow::{Context as _, Result, bail};
use std::{
    fs,
    io::Read as _,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const FIRST_FRAME_BUDGET: Duration = Duration::from_millis(250);

#[test]
fn first_egui_frame_within_250ms() -> Result<()> {
    let case = TempCase::new()?;
    let probe = case.root.join("gui-ready");
    let started = Instant::now();
    let mut app = ChildGuard::spawn(
        Command::new(env!("CARGO_BIN_EXE_booru_bayonet"))
            .env("XDG_CONFIG_HOME", &case.config)
            .env("XDG_DATA_HOME", &case.data)
            .env("XDG_CACHE_HOME", &case.cache)
            .env("BOORU_BAYONET_STARTUP_PROBE", &probe)
            .env("BOORU_BAYONET_STARTUP_PROBE_HEADLESS", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::piped()),
    )
    .context("spawn booru_bayonet")?;

    wait_for_probe(&probe, &mut app, started)?;
    Ok(())
}

fn wait_for_probe(path: &Path, child: &mut ChildGuard, started: Instant) -> Result<()> {
    loop {
        if path.exists() {
            let elapsed = started.elapsed();
            if elapsed > FIRST_FRAME_BUDGET {
                bail!(
                    "first egui frame took {:?}, budget {:?}",
                    elapsed,
                    FIRST_FRAME_BUDGET
                );
            }
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            bail!(
                "booru_bayonet exited before startup probe: {status}\n{}",
                child.stderr_tail()
            );
        }
        if started.elapsed() > FIRST_FRAME_BUDGET {
            bail!(
                "first egui frame exceeded {:?}; probe {} absent",
                FIRST_FRAME_BUDGET,
                path.display()
            );
        }
        thread::sleep(Duration::from_millis(2));
    }
}

struct ChildGuard {
    child: Child,
}

impl ChildGuard {
    fn spawn(command: &mut Command) -> Result<Self> {
        command
            .spawn()
            .map(|child| Self { child })
            .context("spawn child")
    }

    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>> {
        self.child.try_wait().context("poll child")
    }

    fn stderr_tail(&mut self) -> String {
        let Some(mut stderr) = self.child.stderr.take() else {
            return String::new();
        };
        let mut text = String::new();
        let _read = stderr.read_to_string(&mut text);
        text.chars()
            .rev()
            .take(2_000)
            .collect::<String>()
            .chars()
            .rev()
            .collect()
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _killed = self.child.kill();
            let _waited = self.child.wait();
        }
    }
}

struct TempCase {
    root: PathBuf,
    config: PathBuf,
    data: PathBuf,
    cache: PathBuf,
}

impl TempCase {
    fn new() -> Result<Self> {
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = std::env::temp_dir().join(format!(
            "booru-bayonet-startup-{}-{suffix}",
            std::process::id()
        ));
        let config = root.join("config");
        let data = root.join("data");
        let cache = root.join("cache");
        fs::create_dir_all(&config).context("create config tempdir")?;
        fs::create_dir_all(&data).context("create data tempdir")?;
        fs::create_dir_all(&cache).context("create cache tempdir")?;
        Ok(Self {
            root,
            config,
            data,
            cache,
        })
    }
}

impl Drop for TempCase {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.root);
    }
}
