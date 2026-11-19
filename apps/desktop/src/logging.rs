//! Logging: tracing to a size-rotated file (5 x 5 MB) in the app log dir, stderr, and the
//! diagnostics log tail, all through `RedactingMakeWriter` (CLAUDE.md rule 8). The level is
//! `info` by default; `set_debug_logging` flips it to `debug` at runtime.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use downlink_core::redact::RedactingMakeWriter;
use downlink_core::service::LogTail;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry, reload};

pub const MAX_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_FILES: usize = 5;

struct Inner {
    dir: PathBuf,
    name: String,
    file: Option<File>,
    size: u64,
}

/// `downlink.log`, `downlink.log.1` ... `downlink.log.4`; rotates when the current file passes
/// [`MAX_BYTES`].
#[derive(Clone)]
pub struct RotatingFile {
    inner: Arc<Mutex<Inner>>,
    max_bytes: u64,
    max_files: usize,
}

impl RotatingFile {
    pub fn new(dir: &Path, name: &str, max_bytes: u64, max_files: usize) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let path = dir.join(name);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(Self {
            inner: Arc::new(Mutex::new(Inner {
                dir: dir.to_owned(),
                name: name.to_owned(),
                file: Some(file),
                size,
            })),
            max_bytes,
            max_files: max_files.max(1),
        })
    }

    fn rotate(&self, i: &mut Inner) -> io::Result<()> {
        i.file = None;
        let name = |n: usize| {
            if n == 0 {
                i.dir.join(&i.name)
            } else {
                i.dir.join(format!("{}.{n}", i.name))
            }
        };
        let _ = fs::remove_file(name(self.max_files - 1));
        for n in (0..self.max_files - 1).rev() {
            if name(n).exists() {
                let _ = fs::rename(name(n), name(n + 1));
            }
        }
        i.file = Some(OpenOptions::new().create(true).append(true).open(name(0))?);
        i.size = 0;
        Ok(())
    }
}

pub struct RotatingWriter(RotatingFile);

impl Write for RotatingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut g = match self.0.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if g.size + buf.len() as u64 > self.0.max_bytes && g.size > 0 {
            self.0.rotate(&mut g)?;
        }
        let n = match g.file.as_mut() {
            Some(f) => f.write(buf)?,
            None => buf.len(),
        };
        g.size += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut g = match self.0.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        match g.file.as_mut() {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }
}

impl<'a> MakeWriter<'a> for RotatingFile {
    type Writer = RotatingWriter;
    fn make_writer(&'a self) -> Self::Writer {
        RotatingWriter(self.clone())
    }
}

/// Handle to flip the log level at runtime.
#[derive(Clone)]
pub struct LogControl {
    handle: reload::Handle<EnvFilter, Registry>,
}

fn filter(debug: bool) -> EnvFilter {
    if let Ok(f) = EnvFilter::try_from_env("DOWNLINK_LOG") {
        return f;
    }
    EnvFilter::new(if debug {
        "debug,hyper=info,reqwest=info,sqlx=warn,tao=warn,wry=warn"
    } else {
        "info,sqlx=warn"
    })
}

impl LogControl {
    pub fn set_debug(&self, on: bool) {
        let _ = self.handle.reload(filter(on));
    }
}

/// Install the global subscriber. Safe to call once; returns a control handle.
pub fn init(log_dir: &Path, tail: LogTail, debug: bool) -> LogControl {
    let (filter_layer, handle) = reload::Layer::new(filter(debug));
    let file = RotatingFile::new(log_dir, "downlink.log", MAX_BYTES, MAX_FILES).ok();
    let base = Registry::default()
        .with(filter_layer)
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(RedactingMakeWriter::new(tail)),
        )
        .with(tracing_subscriber::fmt::layer().with_writer(RedactingMakeWriter::new(io::stderr)));
    match file {
        Some(f) => {
            let _ = base
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .with_writer(RedactingMakeWriter::new(f)),
                )
                .try_init();
        }
        None => {
            let _ = base.try_init();
        }
    }
    LogControl { handle }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_and_keeps_five_files() {
        let dir = std::env::temp_dir().join(format!("dl-log-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let rf = RotatingFile::new(&dir, "t.log", 100, 5).expect("create");
        let mut w = RotatingWriter(rf.clone());
        for _ in 0..40 {
            w.write_all(&[b'x'; 30]).expect("write");
        }
        let count = fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
        assert_eq!(count, 5);
        let _ = fs::remove_dir_all(&dir);
    }
}
