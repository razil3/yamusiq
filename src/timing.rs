//! Opt-in latency diagnostics. Only stage names and milliseconds are written:
//! no track IDs, URLs, user names, tokens or HTTP error bodies.
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub fn record(stage: &str, elapsed: Duration) {
    static OUTPUT: OnceLock<Mutex<Option<File>>> = OnceLock::new();
    let output = OUTPUT.get_or_init(|| Mutex::new(
        std::env::var_os("YAMUSIQ_TIMINGS").and_then(|path| OpenOptions::new().create(true).append(true).open(path).ok())
    ));
    if let Ok(mut file) = output.lock()
        && let Some(file) = file.as_mut()
    {
        let _ = writeln!(file, "{stage} {:.3} ms", elapsed.as_secs_f64() * 1000.0);
    }
}
