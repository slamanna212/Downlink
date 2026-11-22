//! HLS playlist helpers. Dispatcharr builds the absolute segment URLs of an in-progress
//! recording's playlist from the request `Host` (`docs/dispatcharr-endpoints.md`, DVR), so behind
//! a reverse proxy or under another name they point somewhere the client cannot, or should not,
//! reach. [`rewrite_playlist`] moves such URLs onto the configured server origin. Pure text work:
//! nothing is fetched here.

use url::Url;

/// First absolute http(s) URI in a playlist (segment lines or `URI="…"` attributes).
pub fn playlist_origin(body: &str) -> Option<Url> {
    let mut found = None;
    map_uris(body, &mut |uri| {
        if found.is_none()
            && let Some(u) = absolute(uri)
        {
            found = Some(u);
        }
        None
    });
    found
}

/// Whether URIs from `playlist_origin` must be moved onto `base` (different scheme, host or port).
pub fn needs_playlist_rewrite(base: &Url, playlist_origin: &Url) -> bool {
    base.origin() != playlist_origin.origin()
}

/// Rewrite every absolute http(s) URI whose origin differs from `base` onto `base`'s origin.
/// Path and query are kept as they are; when `base` carries a path prefix (server under a
/// sub-path) the prefix is added if the path lacks it. Relative URIs, other schemes, comments and
/// line endings are untouched.
pub fn rewrite_playlist(body: &str, base: &Url) -> String {
    map_uris(body, &mut |uri| {
        let u = absolute(uri)?;
        if !needs_playlist_rewrite(base, &u) {
            return None;
        }
        let mut origin = base.clone();
        origin.set_path("/");
        origin.set_query(None);
        origin.set_fragment(None);
        let prefix = base.path().trim_end_matches('/');
        let path = if !prefix.is_empty() && !u.path().starts_with(prefix) {
            format!("{prefix}{}", u.path())
        } else {
            u.path().to_owned()
        };
        let mut out = format!("{}{}", origin.as_str().trim_end_matches('/'), path);
        if let Some(q) = u.query() {
            out.push('?');
            out.push_str(q);
        }
        if let Some(f) = u.fragment() {
            out.push('#');
            out.push_str(f);
        }
        Some(out)
    })
}

fn absolute(uri: &str) -> Option<Url> {
    let u = Url::parse(uri.trim()).ok()?;
    matches!(u.scheme(), "http" | "https").then_some(u)
}

/// Replace segment lines and `URI="…"` attributes while preserving playlist formatting.
pub fn map_uris(body: &str, f: &mut dyn FnMut(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(body.len());
    for raw in body.split_inclusive('\n') {
        let (line, eol) = match raw.strip_suffix("\r\n") {
            Some(l) => (l, "\r\n"),
            None => match raw.strip_suffix('\n') {
                Some(l) => (l, "\n"),
                None => (raw, ""),
            },
        };
        if line.starts_with('#') {
            out.push_str(&visit_attrs(line, f));
        } else if line.trim().is_empty() {
            out.push_str(line);
        } else {
            match f(line) {
                Some(r) => out.push_str(&r),
                None => out.push_str(line),
            }
        }
        out.push_str(eol);
    }
    out
}

fn visit_attrs(line: &str, f: &mut dyn FnMut(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find("URI=\"") {
        let start = i + 5;
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let Some(end) = tail.find('"') else {
            out.push_str(tail);
            return out;
        };
        let uri = &tail[..end];
        match f(uri) {
            Some(r) => out.push_str(&r),
            None => out.push_str(uri),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const PL: &str = "#EXTM3U\r\n#EXT-X-VERSION:6\r\n#EXT-X-MAP:URI=\"https://dispatcharr.test/api/channels/recordings/31/hls/init.mp4?x=1\"\r\n#EXTINF:4.0,\r\nhttps://dispatcharr.test/api/channels/recordings/31/hls/seg_00000.ts\r\n#EXTINF:4.0,\r\nseg_00001.ts\r\n#EXTINF:4.0,\r\nhttp://other.test:8080/api/channels/recordings/31/hls/seg_00002.ts?token=t#f";

    fn base(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn origin_is_detected_and_compared() {
        let o = playlist_origin(PL).unwrap();
        assert_eq!(o.host_str(), Some("dispatcharr.test"));
        assert!(needs_playlist_rewrite(&base("http://192.0.2.10:9191/"), &o));
        assert!(!needs_playlist_rewrite(
            &base("https://dispatcharr.test/"),
            &o
        ));
        assert!(
            needs_playlist_rewrite(&base("https://dispatcharr.test:8443/"), &o),
            "a different port is a different origin"
        );
        assert!(playlist_origin("#EXTM3U\nseg.ts\n").is_none());
    }

    #[test]
    fn absolute_uris_move_to_the_configured_origin_keeping_path_and_query() {
        let out = rewrite_playlist(PL, &base("http://192.0.2.10:9191/"));
        assert!(out.contains(
            "URI=\"http://192.0.2.10:9191/api/channels/recordings/31/hls/init.mp4?x=1\""
        ));
        assert!(out.contains(
            "\r\nhttp://192.0.2.10:9191/api/channels/recordings/31/hls/seg_00000.ts\r\n"
        ));
        assert!(
            out.ends_with(
                "http://192.0.2.10:9191/api/channels/recordings/31/hls/seg_00002.ts?token=t#f"
            ),
            "{out}"
        );
        assert!(
            out.contains("\r\nseg_00001.ts\r\n"),
            "relative URIs are left alone"
        );
        assert!(out.starts_with("#EXTM3U\r\n#EXT-X-VERSION:6\r\n"));
        assert!(!out.contains("dispatcharr.test") && !out.contains("other.test"));
    }

    #[test]
    fn a_playlist_already_on_the_base_origin_is_unchanged() {
        let b = base("https://dispatcharr.test/");
        assert_eq!(
            rewrite_playlist(PL, &b).matches("dispatcharr.test").count(),
            3
        );
        let only = "#EXTM3U\nhttps://dispatcharr.test/a/seg.ts\n";
        assert_eq!(rewrite_playlist(only, &b), only);
    }

    #[test]
    fn a_server_under_a_sub_path_gets_the_prefix_once() {
        let b = base("https://media.test/dispatcharr/");
        let out = rewrite_playlist(
            "#EXTM3U\nhttps://x.test/api/r/seg.ts\nhttps://x.test/dispatcharr/api/r/seg2.ts\n",
            &b,
        );
        assert_eq!(
            out,
            "#EXTM3U\nhttps://media.test/dispatcharr/api/r/seg.ts\nhttps://media.test/dispatcharr/api/r/seg2.ts\n"
        );
    }

    #[test]
    fn non_http_uris_and_comments_are_untouched() {
        let s = "#EXTM3U\n# https://x.test/in-a-comment\n#EXT-X-KEY:METHOD=AES-128,URI=\"skd://key\"\ndata:text/plain,hi\n";
        assert_eq!(rewrite_playlist(s, &base("https://a.test/")), s);
    }
}
