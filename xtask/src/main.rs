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
        Some("wet-demo") => WetDemo::parse()?.run(),
        Some(other) => bail!("unknown xtask `{other}`; try `cargo xtask wet-demo`"),
        None => bail!("missing xtask; try `cargo xtask wet-demo`"),
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

impl WetDemo {
    fn parse() -> Result<Self> {
        let mut args = env::args().skip(2);
        let root = env::current_dir().context("resolve workspace root")?;
        let mut out = None;
        let mut display = None;
        let mut width = DEFAULT_W;
        let mut height = DEFAULT_H;
        let mut fps = DEFAULT_FPS;
        let mut stage = Stage::Xvfb;
        let mut build = BuildMode::Fresh;
        let mut run = RunMode::Record;
        let mut camp = CampPolicy::Clean;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--out" => out = Some(PathBuf::from(take(&mut args, "--out")?)),
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
                        "cargo xtask wet-demo [--out PATH] [--display :97] [--live-display] [--skip-build] [--dry-run]"
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
        })
    }

    fn run(self) -> Result<()> {
        tools(["cargo", "xdotool", "ffmpeg"])?;
        if self.stage == Stage::Xvfb {
            tools(["Xvfb", "xdpyinfo"])?;
        }
        let timeline = Timeline::load(&self.root.join(DEMO).join("timeline.toml"))?;
        if self.run == RunMode::Dry {
            println!(
                "wet-demo: {} scripted steps, {} ms choreography",
                timeline.steps.len(),
                timeline.duration().as_millis()
            );
            return Ok(());
        }
        if self.build == BuildMode::Fresh {
            run(
                "cargo",
                ["build", "--release", "--bin", "abv"],
                &[],
                &self.root,
            )?;
        }
        let meta = CargoMeta::read(&self.root)?;
        let binary = bin_path(&meta.target_directory);
        let out = self
            .out
            .unwrap_or_else(|| meta.target_directory.join("demo").join("abv-wet-demo.mp4"));
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        let camp = Camp::raise(&self.root, self.camp)?;
        let display = if self.stage == Stage::LiveDisplay {
            env::var("DISPLAY").context("DISPLAY is required with --live-display")?
        } else {
            self.display.unwrap_or_else(|| ":97".to_owned())
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
        let runtime = timeline.duration() + Duration::from_secs(2);
        let mut ffmpeg =
            Recorder::raise(&display, self.width, self.height, self.fps, runtime, &out)?;
        timeline.play(&display, &window)?;
        ffmpeg.finish(Duration::from_secs(5))?;
        app.terminate()?;
        println!("wet demo: {}", out.display());
        Ok(())
    }
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
    fn raise(root: &Path, policy: CampPolicy) -> Result<Self> {
        let root_dir = env::temp_dir().join(format!("abv-wet-demo-{}", std::process::id()));
        let _stale = fs::remove_dir_all(&root_dir);
        let camp = Self {
            root: root_dir,
            policy,
        };
        fs::create_dir_all(camp.config_app()).context("create demo config dir")?;
        fs::create_dir_all(camp.state_app()).context("create demo state dir")?;
        let _bytes = fs::copy(
            root.join(DEMO).join("config.toml"),
            camp.config_app().join("config.toml"),
        )
        .context("copy demo config")?;
        let _bytes = fs::copy(
            root.join(DEMO).join("slate.toml"),
            camp.state_app().join("slate.toml"),
        )
        .context("copy demo slate")?;
        Ok(camp)
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
        let probe = camp.root.join("gui-ready");
        let mut command = Command::new(binary);
        let _command = command
            .env("DISPLAY", display)
            .env("WINIT_UNIX_BACKEND", "x11")
            .env("ADEQUATE_BOORU_VIEWER_STARTUP_PROBE", &probe)
            .envs(camp.env())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        let mut child = command.spawn().context("spawn abv")?;
        wait_probe(&mut child, &probe, Duration::from_secs(20)).context("wait for abv GUI")?;
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

    fn play(&self, display: &str, window: &str) -> Result<()> {
        let mut cursor = Cursor::default();
        for step in &self.steps {
            step.play(display, window, &mut cursor)?;
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
    },
    Drag {
        from: [i32; 2],
        to: [i32; 2],
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
            Self::Click { .. } | Self::Key { .. } => Duration::from_millis(80),
        }
    }

    fn play(&self, display: &str, window: &str, cursor: &mut Cursor) -> Result<()> {
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
            } => {
                let button = direction.button();
                for _ in 0..*clicks {
                    xdotool(display, ["click", button])?;
                    thread::sleep(Duration::from_millis(*delay_ms));
                }
            }
            Self::Drag { from, to, ms } => {
                glide(display, window, cursor, from[0], from[1], 0)?;
                xdotool(display, ["mousedown", "1"])?;
                glide(display, window, cursor, to[0], to[1], *ms)?;
                xdotool(display, ["mouseup", "1"])?;
            }
        }
        Ok(())
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

fn run<const N: usize>(
    program: &str,
    args: [&str; N],
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
