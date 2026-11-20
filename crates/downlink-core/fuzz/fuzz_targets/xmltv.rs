#![no_main]
use std::io::Write;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(mut f) = tempfile::NamedTempFile::new() else { return };
    if f.write_all(data).is_err() || f.flush().is_err() {
        return;
    }
    let _ = downlink_core::xmltv::parse_channels(f.path());
    let _ = downlink_core::xmltv::parse_programmes(f.path(), chrono_tz::Tz::UTC, |_| {});
});
