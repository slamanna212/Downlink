//! Small shared helpers.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;

/// Open a file, sniffing gzip/xz magic bytes (the extension is not trusted) and returning a
/// streaming decompressed reader.
pub fn open_decompressed(path: &Path) -> std::io::Result<Box<dyn Read + Send>> {
    let mut r = BufReader::with_capacity(64 * 1024, File::open(path)?);
    let magic = r.fill_buf()?;
    if magic.starts_with(&[0x1f, 0x8b]) {
        return Ok(Box::new(flate2::bufread::MultiGzDecoder::new(r)));
    }
    if magic.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]) {
        use liblzma::stream::{CONCATENATED, Stream};

        // Keep the dictionary bounded and return decoder failures through Read.
        let stream =
            Stream::new_stream_decoder(64 * 1024 * 1024, CONCATENATED).map_err(io::Error::other)?;
        return Ok(Box::new(liblzma::bufread::XzDecoder::new_stream(r, stream)));
    }
    Ok(Box::new(r))
}

/// Peak resident set size of this process in KiB (Linux `VmHWM`); `None` elsewhere.
pub fn peak_rss_kib() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = s.lines().find(|l| l.starts_with("VmHWM:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}
