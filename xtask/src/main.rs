use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

const APP: &str = "adequate_booru_viewer";
const WINDOW: &str = "adequate booru viewer";
const DEMO: &str = "demo/wet";
const DEFAULT_W: u32 = 1440;
const DEFAULT_H: u32 = 920;
const DEFAULT_FPS: u32 = 60;
const X264_CRF: &str = "12";
const X264_PRESET: &str = "slow";

fn main() -> Result<()> {
    match env::args().nth(1).as_deref() {
        Some("release-build") => release_build(),
        Some("wet-demo") => WetDemo::parse()?.run(),
        Some(other) => {
            bail!("unknown xtask `{other}`; try `cargo release-build` or `cargo xtask wet-demo`")
        }
        None => bail!("missing xtask; try `cargo release-build` or `cargo xtask wet-demo`"),
    }
}

/// Verify the checkout, then atomically replace the user's installed `abv`
/// with a release build from this exact source tree. `cargo install` only
/// touches the destination after a successful compile, so a broken checkout
/// cannot dislodge the previous known-good binary.
fn release_build() -> Result<()> {
    let root = workspace_root()?;
    run("./check.py", ["verify"], &[], &root)?;
    run("cargo", ["audit"], &[], &root)?;
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
    run("cargo", install, &[], &root)
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

#[derive(Debug)]
struct WetDemo {
    root: PathBuf,
    width: u32,
    height: u32,
    fps: u32,
    out: Option<PathBuf>,
    display: Option<String>,
    stage: Stage,
    build: BuildMode,
    run: RunMode,
    camp: CampPolicy,
    mode: Mode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    Xvfb,
    LiveDisplay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BuildMode {
    Fresh,
    Skip,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RunMode {
    Record,
    Dry,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CampPolicy {
    Clean,
    Keep,
}

/// Which slice of the choreography this invocation drives.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Mode {
    /// The final take: every segment back-to-back from segment 1's entry
    /// state, one continuous recording (so the waves never reset).
    Continuous,
    /// One segment in isolation from its own entry state — the fast iteration
    /// loop for tuning a single beat's mouse work.
    Segment(String),
    /// Replay every segment in order and, at each seam, snapshot the app's live
    /// slate+config into the next segment's entry state. Regenerates entry
    /// states from the app's own writer; this is the state-faithfulness test.
    Scaffold,
}

impl WetDemo {
    fn parse() -> Result<Self> {
        let mut args = env::args().skip(2);
        let root = workspace_root()?;
        let mut out = None;
        let mut display = None;
        let mut width = DEFAULT_W;
        let mut height = DEFAULT_H;
        let mut fps = DEFAULT_FPS;
        let mut stage = Stage::Xvfb;
        let mut build = BuildMode::Fresh;
        let mut run = RunMode::Record;
        let mut camp = CampPolicy::Clean;
        let mut mode = Mode::Continuous;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--out" => out = Some(PathBuf::from(take(&mut args, "--out")?)),
                "--segment" => mode = Mode::Segment(take(&mut args, "--segment")?),
                "--scaffold" => mode = Mode::Scaffold,
                "--display" => display = Some(take(&mut args, "--display")?),
                "--width" => {
                    width = take(&mut args, "--width")?
                        .parse()
                        .context("parse --width")?;
                }
                "--height" => {
                    height = take(&mut args, "--height")?
                        .parse()
                        .context("parse --height")?;
                }
                "--fps" => fps = take(&mut args, "--fps")?.parse().context("parse --fps")?,
                "--live-display" => stage = Stage::LiveDisplay,
                "--skip-build" => build = BuildMode::Skip,
                "--dry-run" => run = RunMode::Dry,
                "--keep-temp" => camp = CampPolicy::Keep,
                "--help" | "-h" => {
                    println!(
                        "cargo xtask wet-demo [--out PATH] [--segment NAME | --scaffold] [--display :97] [--live-display] [--skip-build] [--dry-run]"
                    );
                    std::process::exit(0);
                }
                _ => bail!("unknown wet-demo flag `{arg}`"),
            }
        }
        Ok(Self {
            root,
            width,
            height,
            fps,
            out,
            display,
            stage,
            build,
            run,
            camp,
            mode,
        })
    }

    fn run(self) -> Result<()> {
        tools(["cargo", "xdotool", "ffmpeg"])?;
        if self.stage == Stage::Xvfb {
            tools(["Xvfb", "xdpyinfo"])?;
        }
        let demo = self.root.join(DEMO);
        let manifest = Manifest::load(&demo.join("segments.toml"))?;
        let plan = self.plan(&manifest, &demo)?;
        if self.run == RunMode::Dry {
            self.report_plan(&plan);
            return Ok(());
        }
        if self.build == BuildMode::Fresh {
            // The demo drives named anchor targets, which the app only emits
            // under `devtools`. Shipped builds never carry this.
            run(
                "cargo",
                [
                    "build",
                    "--release",
                    "--bin",
                    "abv",
                    "--features",
                    "devtools",
                ],
                &[],
                &self.root,
            )?;
        }
        let meta = CargoMeta::read(&self.root)?;
        let binary = bin_path(&meta.target_directory);
        let entry = Self::entry_paths(&demo, &manifest, plan[0].index);
        let camp = Camp::raise(self.camp, &entry)?;
        let display = if self.stage == Stage::LiveDisplay {
            env::var("DISPLAY").context("DISPLAY is required with --live-display")?
        } else {
            self.display.clone().unwrap_or_else(|| ":97".to_owned())
        };
        let mut xvfb = if self.stage == Stage::LiveDisplay {
            None
        } else {
            Some(Xvfb::raise(&display, self.width, self.height)?)
        };
        if let Some(server) = xvfb.as_mut() {
            server.wait_ready()?;
        }
        let app = App::raise(&binary, &display, &camp)?;
        let window = App::wait_window(&display)?;
        xdotool(&display, ["windowmove", &window, "0", "0"])?;
        xdotool(
            &display,
            [
                "windowsize",
                &window,
                &self.width.to_string(),
                &self.height.to_string(),
            ],
        )?;
        xdotool(&display, ["windowfocus", "--sync", &window])?;
        let probe = Probe::new(camp.probe_path());
        match &self.mode {
            Mode::Scaffold => {
                Self::scaffold(&plan, &manifest, &demo, &camp, &display, &window, &probe)?;
            }
            Mode::Continuous | Mode::Segment(_) => {
                self.record(&plan, &meta.target_directory, &display, &window, &probe)?;
            }
        }
        app.terminate()?;
        Ok(())
    }

    /// Resolve the segments this invocation will drive: one for `--segment`,
    /// the whole manifest for continuous/scaffold.
    fn plan(&self, manifest: &Manifest, demo: &Path) -> Result<Vec<Segment>> {
        let load = |index: usize, name: &str| -> Result<Segment> {
            let timeline = Timeline::load(&fragment_path(demo, name))?;
            Ok(Segment {
                name: name.to_owned(),
                index,
                timeline,
            })
        };
        match &self.mode {
            Mode::Segment(name) => {
                let index = manifest
                    .order
                    .iter()
                    .position(|other| other == name)
                    .with_context(|| format!("no segment `{name}` in manifest"))?;
                Ok(vec![load(index, name)?])
            }
            Mode::Continuous | Mode::Scaffold => manifest
                .order
                .iter()
                .enumerate()
                .map(|(index, name)| load(index, name))
                .collect(),
        }
    }

    /// Segment 0's entry is the hand-authored base (`demo/wet/{config,slate}`);
    /// every later segment's entry is regenerated under `segments/` by scaffold.
    fn entry_paths(demo: &Path, manifest: &Manifest, index: usize) -> EntryState {
        if index == 0 {
            EntryState {
                config: demo.join("config.toml"),
                slate: demo.join("slate.toml"),
            }
        } else {
            let name = &manifest.order[index];
            EntryState {
                config: demo.join(SEGMENTS).join(format!("{name}.config.toml")),
                slate: demo.join(SEGMENTS).join(format!("{name}.slate.toml")),
            }
        }
    }

    fn report_plan(&self, plan: &[Segment]) {
        let mut total = Duration::ZERO;
        for segment in plan {
            let span = segment.timeline.duration();
            total += span;
            println!(
                "  {:>2} {:<18} {:>3} steps  {:>6} ms",
                segment.index,
                segment.name,
                segment.timeline.steps.len(),
                span.as_millis()
            );
        }
        println!(
            "wet-demo {:?}: {} segment(s), {} ms choreography",
            self.mode,
            plan.len(),
            total.as_millis()
        );
    }

    /// Play the planned segments back-to-back into one recording. Continuous
    /// mode plays the whole manifest from segment 0's entry; `--segment` plays
    /// the single planned fragment from its own entry.
    fn record(
        &self,
        plan: &[Segment],
        target: &Path,
        display: &str,
        window: &str,
        probe: &Probe,
    ) -> Result<()> {
        let out = self.out.clone().unwrap_or_else(|| {
            let stem = match &self.mode {
                Mode::Segment(name) => format!("abv-wet-segment-{name}.mp4"),
                _ => "abv-wet-demo.mp4".to_owned(),
            };
            target.join("demo").join(stem)
        });
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        let span: Duration = plan.iter().map(|segment| segment.timeline.duration()).sum();
        let runtime = span + Duration::from_secs(2);
        let mut ffmpeg =
            Recorder::raise(display, self.width, self.height, self.fps, runtime, &out)?;
        for segment in plan {
            segment.timeline.play(display, window, probe)?;
        }
        ffmpeg.finish(Duration::from_secs(5))?;
        println!("wet demo: {}", out.display());
        Ok(())
    }

    /// Play every segment continuously and, at each seam, snapshot the app's
    /// live slate+config into the next segment's entry state. The app is the
    /// sole author of those files, so a divergence between a `--segment` replay
    /// and this continuous run is a hole in our serialization.
    fn scaffold(
        plan: &[Segment],
        manifest: &Manifest,
        demo: &Path,
        camp: &Camp,
        display: &str,
        window: &str,
        probe: &Probe,
    ) -> Result<()> {
        let seam = demo.join(SEGMENTS);
        fs::create_dir_all(&seam).with_context(|| format!("create {}", seam.display()))?;
        for (slot, segment) in plan.iter().enumerate() {
            segment.timeline.play(display, window, probe)?;
            let Some(next) = manifest.order.get(slot + 1) else {
                continue;
            };
            // Outwait the app's debounced config flush before reading it.
            thread::sleep(Duration::from_millis(800));
            let _bytes = fs::copy(camp.live_config(), seam.join(format!("{next}.config.toml")))
                .with_context(|| format!("snapshot config for {next}"))?;
            let _bytes = fs::copy(camp.live_slate(), seam.join(format!("{next}.slate.toml")))
                .with_context(|| format!("snapshot slate for {next}"))?;
            println!("scaffolded entry state for `{next}`");
        }
        Ok(())
    }
}

const SEGMENTS: &str = "segments";

/// One choreography fragment plus where it sits in the manifest.
struct Segment {
    name: String,
    index: usize,
    timeline: Timeline,
}

/// A segment's entry state: the config+slate pair the app boots from.
struct EntryState {
    config: PathBuf,
    slate: PathBuf,
}

/// The ordered roster of segments under `demo/wet/segments.toml`.
#[derive(Debug, Deserialize)]
struct Manifest {
    order: Vec<String>,
}

impl Manifest {
    fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        let manifest: Self =
            toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
        if manifest.order.is_empty() {
            bail!("segment manifest {} is empty", path.display());
        }
        Ok(manifest)
    }
}

fn fragment_path(demo: &Path, name: &str) -> PathBuf {
    demo.join(SEGMENTS).join(format!("{name}.toml"))
}

fn take(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    args.next().with_context(|| format!("{flag} needs a value"))
}

fn tools<const N: usize>(names: [&str; N]) -> Result<()> {
    for name in names {
        let _tool = find_tool(name)?;
    }
    Ok(())
}

fn find_tool(name: &str) -> Result<PathBuf> {
    let path = env::var_os("PATH").context("PATH is unset")?;
    for dir in env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    bail!("required tool `{name}` is not on PATH")
}

fn bin_path(target: &Path) -> PathBuf {
    let exe = if cfg!(windows) { "abv.exe" } else { "abv" };
    target.join("release").join(exe)
}

#[derive(Debug, Deserialize)]
struct CargoMeta {
    target_directory: PathBuf,
}

impl CargoMeta {
    fn read(root: &Path) -> Result<Self> {
        let out = Command::new("cargo")
            .args(["metadata", "--format-version", "1", "--no-deps"])
            .current_dir(root)
            .output()
            .context("spawn cargo metadata")?;
        ensure(out.status, "cargo metadata")?;
        serde_json::from_slice(&out.stdout).context("parse cargo metadata")
    }
}

struct Camp {
    root: PathBuf,
    policy: CampPolicy,
}

impl Camp {
    fn raise(policy: CampPolicy, entry: &EntryState) -> Result<Self> {
        let root_dir = env::temp_dir().join(format!("abv-wet-demo-{}", std::process::id()));
        let _stale = fs::remove_dir_all(&root_dir);
        let camp = Self {
            root: root_dir,
            policy,
        };
        fs::create_dir_all(camp.config_app()).context("create demo config dir")?;
        fs::create_dir_all(camp.state_app()).context("create demo state dir")?;
        let _bytes = fs::copy(&entry.config, camp.live_config())
            .with_context(|| format!("copy entry config {}", entry.config.display()))?;
        let _bytes = fs::copy(&entry.slate, camp.live_slate())
            .with_context(|| format!("copy entry slate {}", entry.slate.display()))?;
        Ok(camp)
    }

    /// The slate the running app writes back; scaffold snapshots it at seams.
    fn live_slate(&self) -> PathBuf {
        self.state_app().join("slate.toml")
    }

    fn live_config(&self) -> PathBuf {
        self.config_app().join("config.toml")
    }

    fn config_home(&self) -> PathBuf {
        self.root.join("config")
    }

    fn state_home(&self) -> PathBuf {
        self.root.join("state")
    }

    fn config_app(&self) -> PathBuf {
        self.config_home().join(APP)
    }

    fn state_app(&self) -> PathBuf {
        self.state_home().join(APP)
    }

    /// Where the running app drops its anchor-probe JSON (devtools build).
    fn probe_path(&self) -> PathBuf {
        self.root.join("anchors.json")
    }

    fn env(&self) -> [(OsString, OsString); 2] {
        [
            (OsString::from("XDG_CONFIG_HOME"), self.config_home().into()),
            (OsString::from("XDG_STATE_HOME"), self.state_home().into()),
        ]
    }
}

impl Drop for Camp {
    fn drop(&mut self) {
        if self.policy == CampPolicy::Clean {
            let _removed = fs::remove_dir_all(&self.root);
        }
    }
}

struct Xvfb {
    display: String,
    child: Child,
}

impl Xvfb {
    fn raise(display: &str, width: u32, height: u32) -> Result<Self> {
        let geometry = format!("{width}x{height}x24");
        let child = Command::new("Xvfb")
            .args([display, "-screen", "0", &geometry, "-nolisten", "tcp"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("spawn Xvfb")?;
        Ok(Self {
            display: display.to_owned(),
            child,
        })
    }

    fn wait_ready(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.child.try_wait().context("poll Xvfb")?.is_some() {
                bail!("Xvfb exited before accepting clients on {}", self.display);
            }
            if Command::new("xdpyinfo")
                .args(["-display", &self.display])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
            {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(40));
        }
        bail!("Xvfb did not become ready on {}", self.display)
    }
}

impl Drop for Xvfb {
    fn drop(&mut self) {
        let _killed = self.child.kill();
        let _reaped = self.child.wait();
    }
}

struct App {
    child: Child,
}

impl App {
    fn raise(binary: &Path, display: &str, camp: &Camp) -> Result<Self> {
        let mut command = Command::new(binary);
        let _command = command
            .env("DISPLAY", display)
            .env("WINIT_UNIX_BACKEND", "x11")
            .env("ABV_ANCHOR_PROBE", camp.probe_path())
            .envs(camp.env())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        let mut child = command.spawn().context("spawn abv")?;
        wait_probe(&mut child, &camp.probe_path(), Duration::from_secs(20))
            .context("wait for abv GUI")?;
        let app = Self { child };
        Ok(app)
    }

    fn wait_window(display: &str) -> Result<String> {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            let out = Command::new("xdotool")
                .env("DISPLAY", display)
                .args(["search", "--name", WINDOW])
                .output()
                .context("spawn xdotool search")?;
            if out.status.success()
                && let Some(id) = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .next()
                    .filter(|line| !line.is_empty())
            {
                return Ok(id.to_owned());
            }
            thread::sleep(Duration::from_millis(40));
        }
        bail!("could not find `{WINDOW}` window")
    }

    fn terminate(mut self) -> Result<()> {
        let pid = self.child.id().to_string();
        let _status = Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .context("spawn kill -TERM abv")?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.child.try_wait().context("poll abv")?.is_some() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(80));
        }
        self.child.kill().context("kill wedged abv")?;
        let _wait = self.child.wait();
        Ok(())
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _killed = self.child.kill();
            let _reaped = self.child.wait();
        }
    }
}

struct Recorder {
    child: Child,
    raw: PathBuf,
    out: PathBuf,
    fps: u32,
}

impl Recorder {
    fn raise(
        display: &str,
        width: u32,
        height: u32,
        fps: u32,
        runtime: Duration,
        out: &Path,
    ) -> Result<Self> {
        let size = format!("{width}x{height}");
        let fps_text = fps.to_string();
        let input = format!("{display}.0");
        let seconds = format!("{:.3}", runtime.as_secs_f64());
        let raw = capture_path(out);
        if raw.exists() {
            fs::remove_file(&raw).with_context(|| format!("remove {}", raw.display()))?;
        }
        let raw_arg = raw.as_os_str().to_str().context("non-utf8 capture path")?;
        let child = Command::new("ffmpeg")
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-thread_queue_size",
                "1024",
                "-video_size",
                &size,
                "-framerate",
                &fps_text,
                "-f",
                "x11grab",
                "-draw_mouse",
                "1",
                "-i",
                &input,
                "-t",
                &seconds,
                "-an",
                "-c:v",
                "rawvideo",
                "-f",
                "nut",
                raw_arg,
            ])
            .spawn()
            .context("spawn ffmpeg")?;
        thread::sleep(Duration::from_millis(250));
        Ok(Self {
            child,
            raw,
            out: out.to_owned(),
            fps,
        })
    }

    fn finish(&mut self, grace: Duration) -> Result<()> {
        let deadline = Instant::now() + grace;
        let status = loop {
            if let Some(status) = self.child.try_wait().context("poll ffmpeg capture")? {
                break status;
            }
            if Instant::now() >= deadline {
                self.child.kill().context("kill ffmpeg capture")?;
                let _wait = self.child.wait();
                bail!("ffmpeg capture did not finish within {:?}", grace);
            }
            thread::sleep(Duration::from_millis(80));
        };
        ensure(status, "ffmpeg capture")?;
        self.transcode()?;
        fs::remove_file(&self.raw).with_context(|| format!("remove {}", self.raw.display()))?;
        Ok(())
    }

    fn transcode(&self) -> Result<()> {
        let fps = self.fps.to_string();
        let raw = self
            .raw
            .as_os_str()
            .to_str()
            .context("non-utf8 capture path")?;
        let out = self
            .out
            .as_os_str()
            .to_str()
            .context("non-utf8 output path")?;
        let status = Command::new("ffmpeg")
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-i",
                raw,
                "-an",
                "-fps_mode",
                "cfr",
                "-r",
                &fps,
                "-c:v",
                "libx264",
                "-preset",
                X264_PRESET,
                "-crf",
                X264_CRF,
                "-tune",
                "animation",
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
                out,
            ])
            .status()
            .context("spawn ffmpeg transcode")?;
        ensure(status, "ffmpeg transcode")
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _killed = self.child.kill();
            let _reaped = self.child.wait();
        }
        let _removed = fs::remove_file(&self.raw);
    }
}

fn capture_path(out: &Path) -> PathBuf {
    let stem = out
        .file_stem()
        .map_or_else(|| OsString::from("capture"), OsStr::to_os_string);
    let mut name = stem;
    name.push(".capture.nut");
    out.with_file_name(name)
}

#[derive(Debug, Deserialize)]
struct Timeline {
    steps: Vec<Step>,
}

impl Timeline {
    fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parse {}", path.display()))
    }

    fn duration(&self) -> Duration {
        self.steps
            .iter()
            .map(Step::duration)
            .fold(Duration::ZERO, |a, b| a + b)
    }

    fn play(&self, display: &str, window: &str, probe: &Probe) -> Result<()> {
        let mut cursor = Cursor::default();
        for step in &self.steps {
            step.play(display, window, &mut cursor, probe)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Cursor {
    x: i32,
    y: i32,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Step {
    Wait {
        ms: u64,
    },
    Move {
        x: i32,
        y: i32,
        #[serde(default)]
        ms: u64,
    },
    Click {
        #[serde(default = "primary")]
        button: u8,
    },
    Key {
        value: String,
    },
    Type {
        text: String,
        #[serde(default = "type_delay")]
        delay_ms: u64,
    },
    Scroll {
        clicks: u32,
        direction: Scroll,
        #[serde(default = "scroll_delay")]
        delay_ms: u64,
        /// A modifier held down for the whole burst, e.g. `ctrl` for the
        /// grid-density zoom. Absent ⇒ a plain wheel scroll.
        #[serde(default)]
        hold: Option<String>,
    },
    Drag {
        from: [i32; 2],
        to: [i32; 2],
        #[serde(default = "drag_ms")]
        ms: u64,
    },
    /// Glide the pointer to a named anchor's center, resolved live from the
    /// probe — no baked coordinates.
    Point {
        target: String,
        #[serde(default)]
        ms: u64,
    },
    /// Glide to a named anchor and click it.
    Tap {
        target: String,
        #[serde(default = "primary")]
        button: u8,
        #[serde(default)]
        ms: u64,
    },
    /// Closed loop: defocus the tag field if it holds focus, then Tab until the
    /// reference-query active group equals `path`.
    Nav {
        path: Vec<usize>,
    },
    /// Drag from one named anchor onto another — both resolved live. Used to
    /// rearrange query atoms between groups.
    DragTo {
        from: String,
        to: String,
        #[serde(default = "drag_ms")]
        ms: u64,
    },
}

impl Step {
    fn duration(&self) -> Duration {
        match self {
            Self::Wait { ms } | Self::Move { ms, .. } | Self::Drag { ms, .. } => {
                Duration::from_millis(*ms)
            }
            Self::Type { text, delay_ms } => {
                Duration::from_millis(text.chars().count().saturating_mul(*delay_ms as usize) as u64)
            }
            Self::Scroll {
                clicks, delay_ms, ..
            } => Duration::from_millis(u64::from(*clicks) * *delay_ms),
            Self::Point { ms, .. } => Duration::from_millis(*ms),
            Self::Tap { ms, .. } => Duration::from_millis(*ms + 80),
            Self::DragTo { ms, .. } => Duration::from_millis(*ms),
            Self::Nav { .. } => Duration::from_millis(1500),
            Self::Click { .. } | Self::Key { .. } => Duration::from_millis(80),
        }
    }

    fn play(&self, display: &str, window: &str, cursor: &mut Cursor, probe: &Probe) -> Result<()> {
        match self {
            Self::Wait { ms } => thread::sleep(Duration::from_millis(*ms)),
            Self::Move { x, y, ms } => glide(display, window, cursor, *x, *y, *ms)?,
            Self::Click { button } => xdotool(display, ["click", &button.to_string()])?,
            Self::Key { value } => xdotool(display, ["key", value])?,
            Self::Type { text, delay_ms } => {
                xdotool(
                    display,
                    ["type", "--delay", &delay_ms.to_string(), "--", text],
                )?;
            }
            Self::Scroll {
                clicks,
                direction,
                delay_ms,
                hold,
            } => {
                let button = direction.button();
                if let Some(modifier) = hold {
                    xdotool(display, ["keydown", modifier])?;
                }
                for _ in 0..*clicks {
                    xdotool(display, ["click", button])?;
                    thread::sleep(Duration::from_millis(*delay_ms));
                }
                if let Some(modifier) = hold {
                    xdotool(display, ["keyup", modifier])?;
                }
            }
            Self::Drag { from, to, ms } => {
                glide(display, window, cursor, from[0], from[1], 0)?;
                xdotool(display, ["mousedown", "1"])?;
                glide(display, window, cursor, to[0], to[1], *ms)?;
                xdotool(display, ["mouseup", "1"])?;
            }
            Self::Point { target, ms } => {
                let (x, y) = probe.resolve(target)?;
                glide(display, window, cursor, x, y, *ms)?;
            }
            Self::Tap { target, button, ms } => {
                let (x, y) = probe.resolve(target)?;
                glide(display, window, cursor, x, y, *ms)?;
                xdotool(display, ["click", &button.to_string()])?;
                probe.wait_fresh();
            }
            Self::Nav { path } => probe.nav_group(display, path)?,
            Self::DragTo { from, to, ms } => {
                let (fx, fy) = probe.resolve(from)?;
                glide(display, window, cursor, fx, fy, 0)?;
                xdotool(display, ["mousedown", "1"])?;
                // Resolve the drop target only after the grab, so a layout that
                // shifts under the held atom is read at release time.
                let (tx, ty) = probe.resolve(to)?;
                glide(display, window, cursor, tx, ty, *ms)?;
                xdotool(display, ["mouseup", "1"])?;
                probe.wait_fresh();
            }
        }
        Ok(())
    }
}

/// Reader for the app's `devtools` anchor-probe file: named anchor → center,
/// plus the live state closed-loop steps watch.
struct Probe {
    path: PathBuf,
    last: std::cell::Cell<u64>,
}

#[derive(Deserialize)]
struct ProbeFrame {
    frame: u64,
    anchors: Vec<ProbeAnchor>,
    state: ProbeState,
}

#[derive(Deserialize)]
struct ProbeAnchor {
    name: String,
    rect: [f32; 4],
}

#[derive(Deserialize)]
struct ProbeState {
    active_group: Vec<usize>,
    text_edit_focused: bool,
}

impl Probe {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            last: std::cell::Cell::new(0),
        }
    }

    fn read(&self) -> Option<ProbeFrame> {
        serde_json::from_slice(&fs::read(&self.path).ok()?).ok()
    }

    fn center(&self, target: &str) -> Option<(i32, i32)> {
        let frame = self.read()?;
        let anchor = frame.anchors.iter().find(|anchor| anchor.name == target)?;
        let [x0, y0, x1, y1] = anchor.rect;
        Some((
            f32::midpoint(x0, x1).round() as i32,
            f32::midpoint(y0, y1).round() as i32,
        ))
    }

    /// Resolve a target's center, retrying while the app paints it in — a tile
    /// that wants a scroll, or a panel that just opened.
    fn resolve(&self, target: &str) -> Result<(i32, i32)> {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(center) = self.center(target) {
                return Ok(center);
            }
            if Instant::now() >= deadline {
                bail!("probe anchor `{target}` never appeared");
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Block until a strictly newer frame lands, so a read reflects the last
    /// input. Best-effort, bounded.
    fn wait_fresh(&self) {
        let prev = self.last.get();
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if let Some(frame) = self.read()
                && frame.frame > prev
            {
                self.last.set(frame.frame);
                return;
            }
            thread::sleep(Duration::from_millis(8));
        }
        if let Some(frame) = self.read() {
            self.last.set(frame.frame);
        }
    }

    fn nav_group(&self, display: &str, path: &[usize]) -> Result<()> {
        if self
            .read()
            .is_some_and(|frame| frame.state.text_edit_focused)
        {
            xdotool(display, ["key", "Escape"])?;
            self.wait_fresh();
        }
        for _ in 0..16 {
            let here = self.read().map(|frame| frame.state.active_group);
            if here.as_deref() == Some(path) {
                return Ok(());
            }
            xdotool(display, ["key", "Tab"])?;
            self.wait_fresh();
        }
        bail!("could not navigate to active group {path:?}")
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Scroll {
    Up,
    Down,
}

impl Scroll {
    fn button(self) -> &'static str {
        match self {
            Self::Up => "4",
            Self::Down => "5",
        }
    }
}

fn primary() -> u8 {
    1
}

fn type_delay() -> u64 {
    24
}

fn scroll_delay() -> u64 {
    18
}

fn drag_ms() -> u64 {
    650
}

fn glide(display: &str, window: &str, cursor: &mut Cursor, x: i32, y: i32, ms: u64) -> Result<()> {
    if ms == 0 {
        cursor.x = x;
        cursor.y = y;
        return xdotool(
            display,
            [
                "mousemove",
                "--window",
                window,
                &x.to_string(),
                &y.to_string(),
            ],
        );
    }
    let steps = (ms / 16).clamp(1, 90);
    let start = *cursor;
    for step in 1..=steps {
        let t = step as f32 / steps as f32;
        let e = t * t * (3.0 - 2.0 * t);
        let sx = (start.x as f32 + (x - start.x) as f32 * e).round() as i32;
        let sy = (start.y as f32 + (y - start.y) as f32 * e).round() as i32;
        xdotool(
            display,
            [
                "mousemove",
                "--window",
                window,
                &sx.to_string(),
                &sy.to_string(),
            ],
        )?;
        thread::sleep(Duration::from_millis(ms / steps));
    }
    cursor.x = x;
    cursor.y = y;
    Ok(())
}

fn xdotool<const N: usize>(display: &str, args: [&str; N]) -> Result<()> {
    run(
        "xdotool",
        args,
        &[("DISPLAY", OsStr::new(display))],
        Path::new("."),
    )
}

fn wait_probe(child: &mut Child, path: &Path, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() {
            return Ok(());
        }
        if let Some(status) = child.try_wait().context("poll abv startup")? {
            bail!("abv exited before GUI probe with {status}");
        }
        thread::sleep(Duration::from_millis(40));
    }
    bail!("timed out waiting for {}", path.display())
}

fn run(
    program: &str,
    args: impl IntoIterator<Item = impl AsRef<OsStr>>,
    envs: &[(&str, &OsStr)],
    cwd: &Path,
) -> Result<()> {
    let mut command = Command::new(program);
    let _command = command.args(args).current_dir(cwd);
    for (key, value) in envs {
        let _command = command.env(key, value);
    }
    let status = command
        .status()
        .with_context(|| format!("spawn {program}"))?;
    ensure(status, program)
}

fn ensure(status: ExitStatus, what: &str) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        bail!("{what} exited with {status}")
    }
}
