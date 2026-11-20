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

/// The newest `max` bytes of `s`, cut at a line boundary.
pub fn tail(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    match s[start..].find('\n') {
        Some(i) => &s[start + i + 1..],
        None => &s[start..],
    }
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
        let s = "aaaa\nbbbb\ncccc\n";
        assert_eq!(tail(s, 7), "cccc\n");
        assert_eq!(tail(s, 100), s);
    }
}
