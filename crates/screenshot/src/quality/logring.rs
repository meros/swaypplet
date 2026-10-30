//! The last lines this process logged, kept in memory for a report.
//!
//! A report is filed from the panel process, and the lines that explain a
//! problem are the ones it logged just before. The journal has them too, but
//! reading it back means a subprocess, a unit name that differs between the
//! session and the harness, and nothing at all when the process runs outside
//! systemd. So the logger keeps its own tail.
//!
//! `RUST_LOG` still decides what reaches stderr, exactly as `env_logger` did;
//! the ring takes info and above regardless, because the default filter is
//! errors only and a report of errors alone leaves out what led to them.

use std::collections::VecDeque;
use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};

/// How many lines the ring holds. A report takes the last [`super::LOG_LINES`].
const CAPACITY: usize = 200;

static LINES: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

struct Ring {
    inner: env_logger::Logger,
}

impl Log for Ring {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Info || self.inner.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        if self.inner.matches(record) {
            self.inner.log(record);
        }
        if record.level() <= Level::Info {
            push(line(record));
        }
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

fn line(record: &Record) -> String {
    let time = glib::DateTime::now_local()
        .and_then(|t| t.format("%H:%M:%S"))
        .map(|s| s.to_string())
        .unwrap_or_default();
    format!(
        "{time} {:<5} {}: {}",
        record.level(),
        record.target(),
        record.args()
    )
}

fn push(line: String) {
    let Ok(mut lines) = LINES.lock() else {
        return;
    };
    lines.push_back(line);
    while lines.len() > CAPACITY {
        lines.pop_front();
    }
}

/// Install the logger: `env_logger` as before, plus the ring.
pub fn init() {
    let inner = env_logger::Builder::from_default_env().build();
    let max = inner.filter().max(LevelFilter::Info);
    if log::set_boxed_logger(Box::new(Ring { inner })).is_ok() {
        log::set_max_level(max);
    }
}

/// The last `n` lines, oldest first.
pub fn tail(n: usize) -> Vec<String> {
    let Ok(lines) = LINES.lock() else {
        return Vec::new();
    };
    lines
        .iter()
        .skip(lines.len().saturating_sub(n))
        .cloned()
        .collect()
}
