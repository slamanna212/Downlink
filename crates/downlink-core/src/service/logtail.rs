//! In-memory tail of redacted log lines for the diagnostics panel. Plug it in as a tracing
//! writer: `tracing_subscriber::fmt().with_writer(RedactingMakeWriter::new(tail.clone()))` (it is
//! redacted a second time on `push`, which is harmless: redaction is idempotent).

use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;

const CAP: usize = 200;

#[derive(Clone, Default)]
pub struct LogTail {
    lines: Arc<Mutex<VecDeque<String>>>,
}

impl LogTail {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, line: &str) {
        let line = crate::redact::redact(line.trim_end());
        if line.is_empty() {
            return;
        }
        if let Ok(mut g) = self.lines.lock() {
            if g.len() == CAP {
                g.pop_front();
            }
            g.push_back(line);
        }
    }

    /// Last `n` lines, oldest first.
    pub fn tail(&self, n: usize) -> Vec<String> {
        self.lines
            .lock()
            .map(|g| g.iter().skip(g.len().saturating_sub(n)).cloned().collect())
            .unwrap_or_default()
    }
}

pub struct TailWriter(LogTail);

impl Write for TailWriter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        for l in String::from_utf8_lossy(b).lines() {
            self.0.push(l);
        }
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for LogTail {
    type Writer = TailWriter;
    fn make_writer(&'a self) -> Self::Writer {
        TailWriter(self.clone())
    }
}
