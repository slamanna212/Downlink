//! Small shared helpers.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

/// Open a file, sniffing gzip/xz magic bytes (the extension is not trusted) and returning a
/// streaming decompressed reader.
pub fn open_decompressed(path: &Path) -> std::io::Result<Box<dyn Read + Send>> {
    let mut f = File::open(path)?;
    let mut magic = [0u8; 6];
    let n = f.read(&mut magic)?;
    drop(f);
    let f = File::open(path)?;
    let r = BufReader::with_capacity(64 * 1024, f);
    if n >= 2 && magic[..2] == [0x1f, 0x8b] {
        return Ok(Box::new(flate2::read::MultiGzDecoder::new(r)));
    }
    if n >= 6 && magic == [0xfd, b'7', b'z', b'X', b'Z', 0x00] {
        // lzma-rs decodes xz from a BufRead into a Write; run it on a thread behind a pipe so
        // the result streams instead of being buffered whole.
        let (rd, wr) = std::io::pipe()?;
        std::thread::spawn(move || {
            let mut r = r;
            let mut wr = wr;
            let _ = lzma_rs::xz_decompress(&mut r, &mut wr);
        });
        return Ok(Box::new(rd));
    }
    Ok(Box::new(r))
}

/// Peak resident set size of this process in KiB (Linux `VmHWM`); `None` elsewhere.
pub fn peak_rss_kib() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = s.lines().find(|l| l.starts_with("VmHWM:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}
