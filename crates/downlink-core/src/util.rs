//! Small shared helpers.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;

/// Open a file, sniffing gzip/xz magic bytes (the extension is not trusted) and returning a
/// streaming decompressed reader.
pub fn open_decompressed(path: &Path) -> std::io::Result<Box<dyn Read + Send>> {
    let file = open_import_file(path)?;
    let mut r =
        BufReader::with_capacity(64 * 1024, BoundedReader::new(file, MAX_IMPORT_FILE_BYTES));
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

pub(crate) const MAX_IMPORT_FILE_BYTES: u64 = 512 * 1024 * 1024;

pub(crate) fn open_import_file(path: &Path) -> io::Result<File> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_IMPORT_FILE_BYTES {
        return Err(import_limit());
    }
    Ok(file)
}

pub(crate) fn import_limit() -> io::Error {
    io::Error::new(
        io::ErrorKind::FileTooLarge,
        "That file is too large to import.",
    )
}

/// Limit bytes read, distinguishing a file that ends at the limit from one that exceeds it.
pub(crate) struct BoundedReader<R> {
    pub(crate) remaining: u64,
    inner: R,
}

impl<R> BoundedReader<R> {
    pub(crate) fn new(inner: R, remaining: u64) -> Self {
        Self { inner, remaining }
    }
}

impl<R: Read> Read for BoundedReader<R> {
    fn read(&mut self, mut buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            return match self.inner.read(&mut buf[..1])? {
                0 => Ok(0),
                _ => Err(import_limit()),
            };
        }
        if self.remaining < buf.len() as u64 {
            let len = self.remaining as usize;
            buf = &mut buf[..len];
        }
        let n = self.inner.read(buf)?;
        self.remaining -= n as u64;
        Ok(n)
    }
}

/// Peak resident set size of this process in KiB (Linux `VmHWM`); `None` elsewhere.
pub fn peak_rss_kib() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = s.lines().find(|l| l.starts_with("VmHWM:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}
