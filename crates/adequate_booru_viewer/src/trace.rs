use std::{env, fs::OpenOptions, io::Write as _, sync::LazyLock, time::Instant};

static STARTUP_EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

pub fn startup(stage: &str) {
    let Some(path) = env::var_os("ADEQUATE_BOORU_VIEWER_STARTUP_TRACE") else {
        return;
    };
    let Ok(mut trace) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    // One write syscall per line, or concurrent threads shred each other's output.
    let line = format!(
        "{:>12.3} ms  {stage}\n",
        STARTUP_EPOCH.elapsed().as_secs_f64() * 1_000.0
    );
    let _ignored = trace.write_all(line.as_bytes());
}
