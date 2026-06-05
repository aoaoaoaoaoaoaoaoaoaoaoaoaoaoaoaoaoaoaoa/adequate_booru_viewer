use std::{env, fs::OpenOptions, io::Write as _, sync::LazyLock, time::Instant};

static STARTUP_EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

pub fn startup(stage: &str) {
    let Some(path) = env::var_os("BOORU_BAYONET_STARTUP_TRACE") else {
        return;
    };
    let Ok(mut trace) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ignored = writeln!(
        trace,
        "{:>12.3} ms  {stage}",
        STARTUP_EPOCH.elapsed().as_secs_f64() * 1_000.0
    );
}
