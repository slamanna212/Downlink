//! The one `User-Agent` Downlink sends everywhere: API calls, play-time resolves and mpv itself.
//!
//! Shape: `Downlink/<version> (<platform>; <OS>)`, e.g. `Downlink/0.1.0 (Linux; Debian GNU/Linux 13)`.
//! Dispatcharr shows it in its Stats panel next to the user. It names the OS, never the user or
//! the machine's hostname, because the same string also reaches third-party providers.

use std::sync::OnceLock;

/// The process-wide User-Agent, computed once.
pub fn user_agent() -> &'static str {
    static UA: OnceLock<String> = OnceLock::new();
    UA.get_or_init(|| {
        build(
            env!("CARGO_PKG_VERSION"),
            platform_name(),
            whoami::distro().ok().as_deref(),
        )
    })
}

fn platform_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "linux") {
        "Linux"
    } else {
        std::env::consts::OS
    }
}

/// Pure builder. The OS name is reduced to header-safe text without parentheses or `;`.
pub fn build(version: &str, platform: &str, os: Option<&str>) -> String {
    let os = os.map(clean).filter(|s| !s.is_empty() && s != platform);
    match os {
        Some(os) => format!("Downlink/{version} ({platform}; {os})"),
        None => format!("Downlink/{version} ({platform})"),
    }
}

/// Keep printable ASCII, drop parenthesised suffixes like `(trixie)`, collapse whitespace.
fn clean(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0u32;
    for c in s.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ';' => out.push(' '),
            c if depth == 0 && (c.is_ascii_graphic() || c == ' ') => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_with_os() {
        assert_eq!(
            build("1.2.3", "Linux", Some("Debian GNU/Linux 13 (trixie)")),
            "Downlink/1.2.3 (Linux; Debian GNU/Linux 13)"
        );
    }

    #[test]
    fn os_missing_or_redundant() {
        assert_eq!(build("1.0.0", "Windows", None), "Downlink/1.0.0 (Windows)");
        assert_eq!(
            build("1.0.0", "Windows", Some("  ")),
            "Downlink/1.0.0 (Windows)"
        );
        assert_eq!(
            build("1.0.0", "macOS", Some("macOS")),
            "Downlink/1.0.0 (macOS)"
        );
    }

    #[test]
    fn header_safe() {
        let ua = build("1.0.0", "Linux", Some("Fancy\u{2019}OS; v2\r\nX-Evil: 1"));
        assert_eq!(ua, "Downlink/1.0.0 (Linux; FancyOS v2X-Evil: 1)");
        assert!(reqwest::header::HeaderValue::from_str(&ua).is_ok());
    }

    #[test]
    fn process_value_is_valid_header() {
        let ua = user_agent();
        assert!(ua.starts_with(concat!("Downlink/", env!("CARGO_PKG_VERSION"), " (")));
        assert!(reqwest::header::HeaderValue::from_str(ua).is_ok());
    }
}
