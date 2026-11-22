use std::path::PathBuf;

use super::*;

fn fx(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/fixtures/xmltv")
        .join(name)
}

fn all(path: &Path, tz: Tz) -> (Vec<XmltvProgramme>, XmltvStats) {
    let mut v = vec![];
    let st = parse_programmes(path, tz, |p| v.push(p)).unwrap();
    (v, st)
}

#[test]
fn timestamp_formats_and_offsets() {
    let utc = |s: &str| parse_time(s, Tz::UTC).map(|x| x.0);
    let base = 1_767_268_800; // 2026-01-01T12:00:00Z
    assert_eq!(utc("20260101120000 +0000"), Some(base));
    assert_eq!(utc("20260101130000 +0100"), Some(base));
    assert_eq!(utc("20260101070000 -0500"), Some(base));
    assert_eq!(utc("20260101173000 +0530"), Some(base));
    assert_eq!(
        utc("20260101120000+0000"),
        Some(base),
        "no space before the offset"
    );
    assert_eq!(utc("202601011200 +0000"), Some(base), "no seconds");
    assert_eq!(utc("20260101120000 Z"), Some(base));
    assert_eq!(utc("20260101130000 +01:00"), Some(base), "colon offset");
    assert_eq!(
        utc("20260101"),
        Some(base - 12 * 3600),
        "date only = midnight"
    );
    for bad in [
        "",
        "notadate",
        "2026",
        "20261301120000 +0000",
        "20260101120000 +9900",
        "20260101120000 junk",
        "2026010112000",
    ] {
        assert_eq!(utc(bad), None, "{bad:?}");
    }
}

#[test]
fn missing_offset_uses_default_tz_and_is_flagged() {
    let berlin: Tz = "Europe/Berlin".parse().unwrap();
    let (t, assumed) = parse_time("20260101120000", berlin).unwrap();
    assert!(assumed);
    assert_eq!(t, 1_767_268_800 - 3600, "12:00 Berlin winter = 11:00Z");
    // DST: 2026-07-01 12:00 Berlin = 10:00Z
    assert_eq!(
        parse_time("20260701120000", berlin).unwrap().0,
        1_782_907_200 - 2 * 3600
    );
    // an explicit offset wins over the default zone
    assert_eq!(
        parse_time("20260101120000 +0000", berlin),
        Some((1_767_268_800, false))
    );
}

#[test]
fn timezones_fixture() {
    let (v, st) = all(&fx("timezones.xml"), Tz::UTC);
    assert_eq!(v.len(), 7);
    let at = |title: &str| {
        v.iter()
            .find(|p| p.title.as_deref() == Some(title))
            .unwrap()
            .start_utc
    };
    let noon = 1_767_268_800;
    assert_eq!(at("UTC Offset"), noon);
    assert_eq!(at("Plus One Offset"), noon);
    assert_eq!(
        at("Minus Five Offset"),
        noon + 3600,
        "08:00 at -0500 is 13:00Z"
    );
    assert_eq!(at("No Offset (assume UTC)"), noon + 3 * 3600);
    assert_eq!(at("No Seconds"), noon + 4 * 3600);
    assert_eq!(at("No Space Before Offset"), noon + 5 * 3600);
    assert_eq!(
        at("Half Hour Offset"),
        noon + 1800,
        "18:00 at +0530 is 12:30Z"
    );
    assert_eq!(st.assumed_tz, 1);
}

#[test]
fn channels_edge_cases() {
    let ch = parse_channels(&fx("edge_cases.xml")).unwrap();
    assert_eq!(ch.len(), 3);
    assert_eq!(ch[0].id, "a.test");
    assert_eq!(ch[0].display_names, ["Channel A", "A Alias", "101"]);
    assert_eq!(ch[1].display_names, ["Channel B & Co"], "entity resolved");
    assert!(ch[2].display_names.is_empty(), "empty display-name skipped");
}

#[test]
fn programme_edge_cases() {
    let (v, st) = all(&fx("edge_cases.xml"), Tz::UTC);
    let first = &v[0];
    assert_eq!(
        first.title.as_deref(),
        Some("Entities & <Tags> \"Quotes\" \u{a9}"),
        "first <title> wins; entities resolved"
    );
    assert_eq!(first.subtitle.as_deref(), Some("Sub Title"));
    assert_eq!(
        first.description.as_deref(),
        Some("CDATA <b>description</b> & more")
    );
    assert_eq!(
        (first.season, first.episode),
        (Some(1), Some(5)),
        "xmltv_ns 0.4.0/1"
    );
    assert!(
        first.flags & flags::NEW != 0
            && first.flags & flags::PREMIERE != 0
            && first.flags & flags::LIVE != 0
    );
    // dropped: missing stop, zero length, stop before start, unparseable, empty-title-and-desc
    let titles: Vec<_> = v.iter().filter_map(|p| p.title.as_deref()).collect();
    assert!(
        !titles.contains(&"Missing Stop")
            && !titles.contains(&"Zero Length")
            && !titles.contains(&"Stop Before Start")
            && !titles.contains(&"Unparseable Times")
    );
    assert!(
        titles.contains(&"Overlap One") && titles.contains(&"Overlap Two"),
        "overlaps are kept (the guide decides)"
    );
    assert!(st.dropped >= 5, "{st:?}");
    assert!(!st.truncated);
    let long = v
        .iter()
        .find(|p| p.title.as_deref() == Some("Long Description"))
        .unwrap();
    assert!(long.description.as_ref().unwrap().len() > 20_000);
}

#[test]
fn latin1_declared_encoding() {
    let ch = parse_channels(&fx("latin1.xml")).unwrap();
    assert_eq!(ch[0].display_names, ["Caf\u{e9} Cha\u{ee}ne"]);
    let (v, _) = all(&fx("latin1.xml"), Tz::UTC);
    assert_eq!(v[0].title.as_deref(), Some("Cr\u{e8}me Br\u{fb}l\u{e9}e"));
}

#[test]
fn truncated_keeps_everything_before_the_cut() {
    let (v, st) = all(&fx("truncated.xml"), Tz::UTC);
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].title.as_deref(), Some("Complete"));
    assert!(st.truncated);
    assert_eq!(parse_channels(&fx("truncated.xml")).unwrap().len(), 1);
}

#[test]
fn no_xml_declaration_and_gzip_sniffing() {
    let (v, st) = all(&fx("no_header.xml"), Tz::UTC);
    assert_eq!(v.len(), 1);
    assert!(!st.truncated);
    // gzip is detected by magic bytes, not by the extension.
    let gz = fx("basic.xml.gz");
    let dir = tempfile::tempdir().unwrap();
    let renamed = dir.path().join("looks-plain.xml");
    std::fs::copy(&gz, &renamed).unwrap();
    assert!(!parse_channels(&renamed).unwrap().is_empty());
    assert!(!all(&renamed, Tz::UTC).0.is_empty());
}

#[test]
fn xz_is_sniffed_too() {
    use std::io::Write;
    let xml = b"<tv><channel id=\"x.test\"><display-name>X</display-name></channel><programme start=\"20260101120000 +0000\" stop=\"20260101130000 +0000\" channel=\"x.test\"><title>Xz</title></programme></tv>";
    let mut encoder = liblzma::write::XzEncoder::new(Vec::new(), 0);
    encoder.write_all(xml).unwrap();
    let out = encoder.finish().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("guide");
    std::fs::File::create(&p).unwrap().write_all(&out).unwrap();
    assert_eq!(all(&p, Tz::UTC).0[0].title.as_deref(), Some("Xz"));
}

#[test]
fn episode_number_helpers() {
    assert_eq!(parse_xmltv_ns("0.4.0/1"), (Some(1), Some(5)));
    assert_eq!(parse_xmltv_ns(" . 2 . "), (None, Some(3)));
    assert_eq!(parse_xmltv_ns("garbage"), (None, None));
    assert_eq!(parse_onscreen("S02E10"), (Some(2), Some(10)));
    assert_eq!(parse_onscreen("nothing"), (None, None));
}

#[test]
fn compression_errors_are_not_reported_as_successful_xmltv() {
    use std::io::Write;

    let mut encoder = liblzma::write::XzEncoder::new(Vec::new(), 0);
    encoder
        .write_all(b"<tv><channel id=\"test\"/></tv>")
        .unwrap();
    let mut bytes = encoder.finish().unwrap();
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), &bytes).unwrap();
    assert!(
        !parse_programmes(file.path(), Tz::UTC, |_| {})
            .unwrap()
            .truncated
    );

    let footer_byte = bytes.len() - 1;
    bytes[footer_byte] ^= 1;
    std::fs::write(file.path(), bytes).unwrap();
    assert!(parse_channels(file.path()).is_err());
    assert!(parse_programmes(file.path(), Tz::UTC, |_| {}).is_err());
}

#[test]
fn xmltv_limits_fields_events_and_nesting() {
    let path = tempfile::NamedTempFile::new().unwrap();
    let field = format!(
        "<tv><channel id=\"test\"><display-name>{}</display-name></channel></tv>",
        "x".repeat(MAX_FIELD_BYTES + 1)
    );
    let event = format!("<tv><!--{}--></tv>", "x".repeat(MAX_EVENT_BYTES + 1));
    let nested = format!(
        "<tv>{}{}</tv>",
        "<n>".repeat(MAX_NESTING),
        "</n>".repeat(MAX_NESTING)
    );

    std::fs::write(path.path(), field).unwrap();
    assert_eq!(
        parse_channels(path.path()).unwrap_err().kind(),
        io::ErrorKind::FileTooLarge
    );

    for payload in [event, nested] {
        std::fs::write(path.path(), payload).unwrap();
        assert_eq!(
            parse_channels(path.path()).unwrap_err().kind(),
            io::ErrorKind::FileTooLarge
        );
        assert_eq!(
            parse_programmes(path.path(), Tz::UTC, |_| {})
                .unwrap_err()
                .kind(),
            io::ErrorKind::FileTooLarge
        );
    }
}
