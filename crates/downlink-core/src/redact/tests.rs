use super::*;

fn clean(s: &str) -> String {
    redact(s)
}

#[test]
fn xtream_live_movie_series_paths() {
    assert_eq!(
        clean("GET http://p.test:8080/live/alice/hunter2/1234.ts"),
        "GET http://p.test:8080/live/***/***/1234.ts"
    );
    assert_eq!(
        clean("http://p.test/movie/alice/hunter2/55.mkv"),
        "http://p.test/movie/***/***/55.mkv"
    );
    assert_eq!(
        clean("http://p.test/series/alice/hunter2/9.mp4"),
        "http://p.test/series/***/***/9.mp4"
    );
}

#[test]
fn xtream_timeshift_path() {
    let out = clean("http://p.test/timeshift/alice/hunter2/60/2025-01-01:10-00/77.ts");
    assert!(!out.contains("alice") && !out.contains("hunter2"), "{out}");
    assert!(out.contains("/timeshift/***/***/60/"), "{out}");
}

#[test]
fn xtream_path_at_end_of_string() {
    assert_eq!(clean("/live/alice/hunter2"), "/live/***/***");
}

#[test]
fn query_credentials_for_php_endpoints() {
    for u in [
        "http://p.test/player_api.php?username=alice&password=hunter2&action=get_live_streams",
        "http://p.test/get.php?username=alice&password=hunter2&type=m3u_plus",
        "http://p.test/xmltv.php?username=alice&password=hunter2",
        "http://p.test/streaming/timeshift.php?username=alice&password=hunter2&stream=1&start=x",
    ] {
        let out = clean(u);
        assert!(!out.contains("alice"), "{out}");
        assert!(!out.contains("hunter2"), "{out}");
        assert!(
            out.contains("username=***") && out.contains("password=***"),
            "{out}"
        );
    }
    // non-secret params survive
    assert!(
        clean("http://p.test/player_api.php?username=a&password=b&action=x").contains("action=x")
    );
}

#[test]
fn query_api_key_and_token() {
    let out = clean("https://d.test/api/x?api_key=SECRETKEY123&page=2");
    assert_eq!(out, "https://d.test/api/x?api_key=***&page=2");
    assert_eq!(
        clean("https://d.test/x?page=2&token=abcd1234"),
        "https://d.test/x?page=2&token=***"
    );
}

#[test]
fn userinfo_in_url() {
    assert_eq!(
        clean("http://alice:hunter2@p.test/stream"),
        "http://***@p.test/stream"
    );
}

#[test]
fn hls_token_path() {
    assert_eq!(
        clean("http://d.test/proxy/hls/abcDEF123/index.m3u8"),
        "http://d.test/proxy/hls/***/index.m3u8"
    );
}

#[test]
fn headers_api_key_authorization() {
    assert_eq!(clean("X-API-Key: sk_live_abcdef"), "X-API-Key: ***");
    assert_eq!(clean("x-api-key=sk_live_abcdef"), "x-api-key=***");
    assert_eq!(
        clean("Authorization: Bearer eyJhbGciOi.payload.sig"),
        "Authorization: ***"
    );
    assert_eq!(clean("Cookie: sessionid=abc; csrftoken=def"), "Cookie: ***");
    let dbg = r#"{"X-API-Key": "sk_live_abcdef", "Accept": "*/*"}"#;
    let out = clean(dbg);
    assert!(!out.contains("sk_live_abcdef"), "{out}");
    assert!(out.contains("Accept"), "{out}");
}

#[test]
fn http_header_fields_option_string() {
    let out = clean("http-header-fields=X-API-Key: sk_live_abcdef");
    assert!(!out.contains("sk_live_abcdef"), "{out}");
}

#[test]
fn json_secrets() {
    let out = clean(
        r#"{"username":"alice","password":"hunter2","access":"AAA.BBB.CCC","refresh":"RRR"}"#,
    );
    assert!(
        !out.contains("hunter2") && !out.contains("AAA.BBB") && !out.contains("RRR"),
        "{out}"
    );
    assert!(
        out.contains("alice"),
        "username alone in JSON is not touched: {out}"
    );
}

#[test]
fn redirect_urls_are_dropped() {
    assert_eq!(
        redact_redirect_url("http://provider.test/live/u/p/1.ts"),
        "<redirect-url>"
    );
}

#[test]
fn literal_set_redacts_known_secrets() {
    let mut set = RedactionSet::new();
    set.add("provider-token-xyz").add("ab"); // short ignored
    let out = set.redact("failed ab to reach http://h.test/a/provider-token-xyz/b");
    assert_eq!(out, "failed ab to reach http://h.test/a/***/b");
    assert!(!set.is_empty());
}

#[test]
fn redaction_is_idempotent() {
    let once = clean("http://p.test/live/alice/hunter2/1.ts?token=zzzz&x-api-key=1");
    assert_eq!(clean(&once), once);
}

#[test]
fn header_value_helper() {
    assert_eq!(redact_header_value("X-API-Key", "abc"), "***");
    assert_eq!(redact_header_value("authorization", "Bearer abc"), "***");
    assert_eq!(redact_header_value("Accept", "*/*"), "*/*");
}

#[test]
fn redacted_display_wrapper() {
    let url = "http://p.test/get.php?username=a&password=b";
    assert_eq!(
        format!("{}", Redacted(url)),
        "http://p.test/get.php?username=***&password=***"
    );
    assert!(!format!("{:?}", Redacted(url)).contains("password=b"));
}

#[test]
fn tracing_writer_redacts_events() {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);
    impl Write for Sink {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .map_err(|_| std::io::Error::other("poisoned"))?
                .extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let sink = Sink::default();
    let s2 = sink.clone();
    let sub = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(RedactingMakeWriter::new(move || s2.clone()))
        .finish();
    tracing::subscriber::with_default(sub, || {
        tracing::info!("opening http://p.test/live/alice/hunter2/1.ts with X-API-Key: sk_abcdef");
    });
    let text =
        String::from_utf8(sink.0.lock().map(|g| g.clone()).unwrap_or_default()).unwrap_or_default();
    assert!(text.contains("/live/***/***/1.ts"), "{text}");
    assert!(
        !text.contains("hunter2") && !text.contains("sk_abcdef"),
        "{text}"
    );
}
