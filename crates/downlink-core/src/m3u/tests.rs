use super::*;

fn p(s: &str) -> Playlist {
    parse(s.as_bytes()).unwrap()
}

#[test]
fn basic_entry_and_header() {
    let pl = p(
        "#EXTM3U x-tvg-url=\"http://a.test/e.xml, http://b.test/e.xml.gz\" url-logo=\"http://l.test/\" tvg-shift=\"1.5\" max-conn=\"2\"\n\
#EXTINF:-1 tvg-id=\"one.test\" tvg-name=\"One\" tvg-chno=\"5.5\" tvg-logo=\"http://l.test/1.png\" group-title=\"News\" radio=\"true\" custom-x=\"keep\",Channel, One\nhttp://s.test/1.ts\n",
    );
    assert_eq!(
        pl.header.tvg_urls,
        ["http://a.test/e.xml", "http://b.test/e.xml.gz"]
    );
    assert_eq!(
        (pl.header.tvg_shift, pl.header.max_conn),
        (Some(1.5), Some(2))
    );
    assert_eq!(pl.entries.len(), 1);
    let e = &pl.entries[0];
    assert_eq!(
        e.name, "Channel, One",
        "split once on the first comma; name may contain commas"
    );
    assert_eq!(
        (e.tvg_id(), e.tvg_name(), e.tvg_chno(), e.group.as_deref()),
        (Some("one.test"), Some("One"), Some(5.5), Some("News"))
    );
    assert!(e.is_radio());
    assert_eq!(
        e.attrs.get("custom-x"),
        Some("keep"),
        "unknown attributes preserved"
    );
    assert_eq!(e.duration, Some(-1.0));
}

#[test]
fn comma_inside_quoted_attribute_is_not_the_name_separator() {
    let pl = p("#EXTINF:-1 tvg-name=\"A, B\" group-title=\"x\",Real Name\nhttp://s.test/1\n");
    assert_eq!(pl.entries[0].name, "Real Name");
    assert_eq!(pl.entries[0].tvg_name(), Some("A, B"));
}

#[test]
fn line_endings_bom_and_missing_header() {
    for sep in ["\n", "\r\n", "\r"] {
        let text = format!(
            "\u{feff}#EXTINF:-1 tvg-id=\"a\",A{sep}http://s.test/a{sep}#EXTINF:-1,B{sep}http://s.test/b"
        );
        let pl = p(&text);
        assert_eq!(pl.entries.len(), 2, "sep {sep:?}");
        assert_eq!(pl.entries[0].tvg_id(), Some("a"), "BOM stripped");
        assert_eq!(pl.entries[1].url, "http://s.test/b", "no trailing newline");
    }
}

#[test]
fn aux_lines_become_headers_and_drm_flag() {
    let pl = p(
        "#EXTM3U\n#EXTINF:-1,UA\n#EXTVLCOPT:http-user-agent=Agent/1\n#EXTVLCOPT:http-referrer=http://r.test\n#EXTVLCOPT:http-origin=http://o.test\n#EXTVLCOPT:network-caching=1000\nhttp://s.test/ua\n\
#EXTINF:-1,Drm\n#KODIPROP:inputstream=inputstream.adaptive\n#KODIPROP:inputstream.adaptive.license_type=com.widevine.alpha\nhttp://s.test/drm.mpd\n\
#EXTGRP:Grouped\n#EXTINF:-1,Grp\nhttp://s.test/g\n\
#EXTINF:-1,Http\n#EXTHTTP:{\"Authorization\":\"Bearer abc\",\"x-n\":1}\nhttp://s.test/h\n\
#EXTINF:-1,Pipe\nhttp://s.test/p.ts|User-Agent=Agent/2&Referer=http://r.test&X-Foo=bar\n",
    );
    assert_eq!(pl.entries.len(), 5);
    let ua = &pl.entries[0].aux;
    assert_eq!(
        ua.http_headers(),
        [
            ("User-Agent".into(), "Agent/1".into()),
            ("Referer".into(), "http://r.test".into()),
            ("Origin".into(), "http://o.test".into())
        ]
    );
    assert_eq!(ua.vlcopts, [("network-caching".into(), "1000".into())]);
    assert!(!ua.is_drm());
    assert!(pl.entries[1].aux.is_drm());
    assert_eq!(
        pl.entries[2].group.as_deref(),
        Some("Grouped"),
        "EXTGRP before EXTINF applies"
    );
    assert_eq!(
        pl.entries[3].aux.headers,
        [("Authorization".into(), "Bearer abc".into())],
        "non-string JSON values ignored"
    );
    let pipe = &pl.entries[4];
    assert_eq!(
        pipe.url, "http://s.test/p.ts",
        "pipe suffix split off the URL"
    );
    assert_eq!(
        pipe.aux.http_headers()[0],
        ("User-Agent".into(), "Agent/2".into())
    );
    assert_eq!(pipe.aux.headers[2], ("X-Foo".into(), "bar".into()));
}

#[test]
fn malformed_input_is_isolated_and_counted() {
    let pl = p(
        "#EXTM3U\n#EXTINF:-1,NoUrl\n#EXTINF:-1,After\nhttp://s.test/after\nhttp://s.test/orphan\n\
#EXTINF:-1 tvg-id=\"unclosed group-title=\"Bad\",Unclosed Quote\nhttp://s.test/u\n\
#EXTINF:-1 tvg-id=nq group-title=Bad,Unquoted\nhttp://s.test/q\n#EXTINF:-1,Truncated",
    );
    let names: Vec<&str> = pl.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["After", "Unclosed Quote", "Unquoted"]);
    assert_eq!(pl.stats.dropped, 3, "no-url + orphan + truncated");
    assert_eq!(pl.entries[2].tvg_id(), Some("nq"));
}

#[test]
fn windows_1252_fallback() {
    let mut bytes = b"#EXTINF:-1,Caf".to_vec();
    bytes.push(0xE9); // é in Windows-1252, invalid UTF-8
    bytes.extend_from_slice(b"\nhttp://s.test/c\n");
    let pl = parse(&bytes[..]).unwrap();
    assert_eq!(pl.entries[0].name, "Café");
    assert_eq!(pl.stats.non_utf8_lines, 1);
}

#[test]
fn oversized_line_is_skipped_with_bounded_memory() {
    let mut data = b"#EXTINF:-1,Good\nhttp://s.test/good\n#EXTINF:-1,Huge\n".to_vec();
    data.extend(std::iter::repeat_n(b'x', MAX_LINE * 3)); // a 3 MiB URL line
    data.extend_from_slice(b"\n#EXTINF:-1,Also good\nhttp://s.test/ag\n");
    let pl = parse(&data[..]).unwrap();
    assert_eq!(pl.stats.oversized_lines, 1);
    let names: Vec<&str> = pl.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["Good", "Also good"]);
    assert_eq!(pl.stats.dropped, 1);
}

#[test]
fn includes_are_flagged() {
    let pl = p(
        "#EXTINF:-1 type=\"playlist\",Sub\nhttp://s.test/sub.m3u\n#EXTINF:-1,Plain\nhttp://s.test/p\n",
    );
    assert!(pl.entries[0].is_include() && !pl.entries[1].is_include());
}

#[test]
fn attrs_edge_cases() {
    let a = parse_attrs(" a=\"1\" B = 2 c='x y' d=\"unterminated rest");
    assert_eq!(
        (a.get("a"), a.get("b"), a.get("c"), a.get("d")),
        (Some("1"), Some("2"), Some("x y"), Some("unterminated rest"))
    );
    assert!(parse_attrs("").is_empty());
    assert!(parse_attrs("=\"\"==").is_empty() || parse_attrs("=\"\"==").len() <= 1);
    assert_eq!(parse_attrs("k=").get("k"), Some(""));
}

#[test]
fn split_pipe() {
    assert_eq!(split_pipe_headers("http://x/y").1.len(), 0);
    let (u, h) = split_pipe_headers("http://x/y|useragent=Z&junk&=v");
    assert_eq!(u, "http://x/y");
    assert_eq!(h, [("User-Agent".to_owned(), "Z".to_owned())]);
}

#[test]
fn parse_budget_is_shared_between_files() {
    let two_lines = b"#EXTINF:-1,A\r\nhttp://stream.test/a\r\n";
    let mut budget = ParseBudget {
        bytes: (two_lines.len() * 2) as u64,
        entries: 2,
    };
    assert_eq!(
        parse_with_budget(&two_lines[..], &mut budget)
            .unwrap()
            .entries
            .len(),
        1
    );
    assert_eq!(
        parse_with_budget(&two_lines[..], &mut budget)
            .unwrap()
            .entries
            .len(),
        1
    );
    assert_eq!(budget.bytes, 0);
    assert_eq!(budget.entries, 0);
    assert_eq!(
        parse_with_budget(&two_lines[..], &mut budget)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::FileTooLarge
    );
}
