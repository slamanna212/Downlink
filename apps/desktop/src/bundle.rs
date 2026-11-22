//! Diagnostics bundle (PLAN §11.5): a zip of redacted logs and versions the user can attach to a
//! bug report. Every text entry is passed through `redact/` on its way in (CLAUDE.md rule 8).

use std::io::{Cursor, Write};

use downlink_core::redact::redact;
use zip::write::SimpleFileOptions;

/// Cap per log file so a bundle stays small: the newest 1 MiB of each.
pub const LOG_TAIL_BYTES: usize = 1024 * 1024;

/// Build the zip from `(file name, text)` pairs. Text is redacted line by line.
pub fn build_zip(entries: &[(String, String)]) -> Result<Vec<u8>, String> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, text) in entries {
        zip.start_file(name.as_str(), opts)
            .map_err(|e| e.to_string())?;
        let redacted: String = text.lines().map(redact).collect::<Vec<_>>().join("\n");
        zip.write_all(redacted.as_bytes())
            .map_err(|e| e.to_string())?;
    }
    zip.finish()
        .map(Cursor::into_inner)
        .map_err(|e| e.to_string())
}

/// Read a bounded suffix, discarding a partial first line to keep redaction intact.
pub fn log_tail(path: &std::path::Path) -> std::io::Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let start = file.metadata()?.len().saturating_sub(LOG_TAIL_BYTES as u64);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(LOG_TAIL_BYTES as u64).read_to_end(&mut bytes)?;
    let offset = if start == 0 {
        0
    } else {
        bytes
            .iter()
            .position(|b| *b == b'\n')
            .map_or(bytes.len(), |i| i + 1)
    };
    Ok(String::from_utf8_lossy(&bytes[offset..]).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn read_all(bytes: &[u8]) -> String {
        let mut z = zip::ZipArchive::new(Cursor::new(bytes)).expect("zip");
        let mut out = String::new();
        for i in 0..z.len() {
            let mut f = z.by_index(i).expect("entry");
            f.read_to_string(&mut out).expect("read");
            out.push('\n');
        }
        out
    }

    #[test]
    fn bundle_contains_no_secrets() {
        let log = "GET http://user:hunter2@host/api\n\
                   X-API-Key: sk_live_abcdef123456\n\
                   playing http://prov.example/live/alice/s3cr3t/42.ts\n\
                   http://x/player_api.php?username=bob&password=p4ss&action=get_live\n\
                   {\"password\":\"topsecret\"}\n\
                   Authorization: Bearer eyJhbGciOi.payload.sig\n";
        let zip = build_zip(&[
            ("logs/downlink.log".into(), log.into()),
            ("app.json".into(), "{}".into()),
        ])
        .expect("build");
        let text = read_all(&zip);
        for secret in [
            "hunter2",
            "sk_live_abcdef123456",
            "alice",
            "s3cr3t",
            "bob",
            "p4ss",
            "topsecret",
            "eyJhbGciOi",
        ] {
            assert!(
                !text.contains(secret),
                "{secret} leaked into the bundle:\n{text}"
            );
        }
        assert!(text.contains("***"), "redaction masks were expected");
    }

    #[test]
    fn tail_cuts_at_a_line_boundary() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "whole log").unwrap();
        assert_eq!(log_tail(file.path()).unwrap(), "whole log");
        let mut text = "x".repeat(LOG_TAIL_BYTES);
        text.push_str("\nlast line\n");
        std::fs::write(file.path(), text).unwrap();
        assert_eq!(log_tail(file.path()).unwrap(), "last line\n");
    }
}
