use std::fs::File;
use std::io::{BufWriter, IsTerminal, Write};
use std::sync::{Mutex, OnceLock, atomic::AtomicBool};

static LOG: OnceLock<Mutex<BufWriter<File>>> = OnceLock::new();
static IS_TERMINAL: AtomicBool = AtomicBool::new(false);

pub fn init() {
    let file = File::create("bot.log").expect("Failed to create bot.log");

    IS_TERMINAL.store(
        std::io::stdout().is_terminal(),
        std::sync::atomic::Ordering::Relaxed,
    );
    LOG.set(Mutex::new(BufWriter::new(file)))
        .expect("Logging is already initialized");

    info("Logging initialized");
}

fn log(level: &str, msg: &str) {
    let ts_seconds = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_secs() as i64,
        Err(_) => 0,
    };

    let time = time_format::strftime_local("%H:%M:%S", ts_seconds)
        .unwrap_or_else(|_| "00:00:00".to_string());

    if let Some(log) = LOG.get() {
        if let Ok(mut guard) = log.lock() {
            let _ = writeln!(guard, "[{time} {level}] {msg}");
            let _ = guard.flush();
        }
    }

    if IS_TERMINAL.load(std::sync::atomic::Ordering::Relaxed) {
        let color = level_color(level);
        println!("{color}[{time} {level}]\x1b[0m {msg}");
    }
}

fn level_color(level: &str) -> &'static str {
    match level.trim() {
        "INFO" => "\x1b[32m",
        "WARN" => "\x1b[33m",
        "ERROR" => "\x1b[31m",
        _ => "",
    }
}

pub fn info(msg: &str) {
    log(" INFO", msg);
}
#[allow(dead_code)]
pub fn warn(msg: &str) {
    log(" WARN", msg);
}
pub fn error(msg: &str) {
    log("ERROR", msg);
}
