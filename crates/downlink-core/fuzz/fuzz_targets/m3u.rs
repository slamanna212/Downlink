#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(pl) = downlink_core::m3u::parse(data) {
        // Invariants: every entry has a URL; counts are consistent.
        assert!(pl.entries.iter().all(|e| !e.url.trim().is_empty()));
        assert_eq!(pl.stats.entries as usize, pl.entries.len());
    }
});
