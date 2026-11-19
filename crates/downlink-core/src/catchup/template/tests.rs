use chrono::TimeZone;

use super::*;
use crate::m3u::parse;

fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
    chrono::Utc
        .with_ymd_and_hms(y, mo, d, h, mi, 0)
        .single()
        .unwrap()
        .timestamp()
}

fn ctx<'a>(live: &'a str, start: i64, end: i64) -> ExpandCtx<'a> {
    ExpandCtx {
        live_url: live,
        start_utc: start,
        end_utc: end,
        now_utc: 1_900_000_000,
        channel_id: "ch7",
        catchup_id: "cid9",
        xc: None,
    }
}

fn tpl(mode: Mode, source: Option<&str>, time: TimeSemantics, corr_h: i64) -> CatchupTemplate {
    CatchupTemplate {
        mode,
        source: source.map(Into::into),
        time,
        correction_secs: corr_h * 3600,
        days: 3,
    }
}

fn wall(tz: &str) -> TimeSemantics {
    TimeSemantics::LocalWallclock { tz: tz.into() }
}

#[test]
fn default_mode_epoch_and_duration() {
    let s = utc(2026, 1, 1, 10, 0);
    let t = tpl(
        Mode::Default,
        Some(
            "http://p.test/ts.php?stream=1&utc={utc}&utcend={utcend}&d={duration}&m={duration:60}&c={channel-id}&i={catchup-id}&now={lutc}",
        ),
        TimeSemantics::UtcSeconds,
        0,
    );
    let u = expand(&t, &ctx("http://p.test/live/1.ts", s, s + 5400)).unwrap();
    assert_eq!(
        u,
        format!(
            "http://p.test/ts.php?stream=1&utc={s}&utcend={}&d=5400&m=90&c=ch7&i=cid9&now=1900000000",
            s + 5400
        )
    );
}

#[test]
fn start_end_aliases_and_offset() {
    let s = utc(2026, 1, 1, 10, 0);
    let t = tpl(
        Mode::Default,
        Some("{start}-{end}-{offset}"),
        TimeSemantics::UtcSeconds,
        0,
    );
    assert_eq!(
        expand(&t, &ctx("x", s, s + 60)).unwrap(),
        format!("{s}-{}-{}", s + 60, 1_900_000_000 - s)
    );
}

#[test]
fn correction_shifts_start_and_end_before_expansion() {
    let s = utc(2026, 1, 1, 10, 0);
    let t = tpl(
        Mode::Default,
        Some("{utc}/{utcend}"),
        TimeSemantics::UtcSeconds,
        -1,
    );
    assert_eq!(
        expand(&t, &ctx("x", s, s + 3600)).unwrap(),
        format!("{}/{}", s - 3600, s)
    );
    let t = CatchupTemplate {
        correction_secs: 5400,
        ..t
    }; // +1.5 h
    assert_eq!(
        expand(&t, &ctx("x", s, s + 3600)).unwrap(),
        format!("{}/{}", s + 5400, s + 9000)
    );
}

#[test]
fn wallclock_formats_follow_the_templates_zone_not_the_machines() {
    // 2026-07-01 12:00Z: London (BST) 13:00, New York (EDT) 08:00, Berlin (CEST) 14:00, UTC 12:00.
    let s = utc(2026, 7, 1, 12, 0);
    for (tz, h) in [
        ("Europe/London", "13"),
        ("America/New_York", "08"),
        ("Europe/Berlin", "14"),
        ("UTC", "12"),
    ] {
        let t = tpl(
            Mode::Default,
            Some("{Y}{m}{d}T{H}{M}{S}|{start:%H:%M}"),
            wall(tz),
            0,
        );
        assert_eq!(
            expand(&t, &ctx("x", s, s + 60)).unwrap(),
            format!("20260701T{h}0000|{h}:00"),
            "{tz}"
        );
    }
    // Winter: London = UTC.
    let w = utc(2026, 1, 15, 12, 0);
    let t = tpl(Mode::Default, Some("{H}"), wall("Europe/London"), 0);
    assert_eq!(expand(&t, &ctx("x", w, w + 60)).unwrap(), "12");
    // Epoch placeholders are unaffected by the zone.
    let t = tpl(Mode::Default, Some("{utc}"), wall("America/New_York"), 0);
    assert_eq!(expand(&t, &ctx("x", s, s + 60)).unwrap(), s.to_string());
}

#[test]
fn dst_transition_days_render_correct_local_hours() {
    // Berlin 2026-03-29: clocks jump 02:00 -> 03:00 (01:00Z). 00:30Z = 01:30 CET, 01:30Z = 03:30 CEST.
    let t = tpl(Mode::Default, Some("{H}:{M}"), wall("Europe/Berlin"), 0);
    let before = utc(2026, 3, 29, 0, 30);
    let after = utc(2026, 3, 29, 1, 30);
    assert_eq!(expand(&t, &ctx("x", before, before + 60)).unwrap(), "01:30");
    assert_eq!(expand(&t, &ctx("x", after, after + 60)).unwrap(), "03:30");
    // Fall back 2026-10-25: 02:00 CEST -> 02:00 CET (01:00Z). Both 00:30Z and 01:30Z map to
    // *different* instants even though the wall clock repeats 02:xx: 00:30Z = 02:30 CEST, 01:30Z = 02:30 CET.
    let a = utc(2026, 10, 25, 0, 30);
    let b = utc(2026, 10, 25, 1, 30);
    assert_eq!(expand(&t, &ctx("x", a, a + 60)).unwrap(), "02:30");
    assert_eq!(expand(&t, &ctx("x", b, b + 60)).unwrap(), "02:30");
    // The epoch value disambiguates the repeated hour.
    let e = tpl(Mode::Default, Some("{utc}"), wall("Europe/Berlin"), 0);
    assert_ne!(
        expand(&e, &ctx("x", a, a + 60)).unwrap(),
        expand(&e, &ctx("x", b, b + 60)).unwrap()
    );
}

#[test]
fn append_and_shift_modes() {
    let s = utc(2026, 1, 1, 10, 0);
    let a = tpl(
        Mode::Append,
        Some("?offset=-{duration}&start={start}"),
        TimeSemantics::UtcSeconds,
        0,
    );
    assert_eq!(
        expand(&a, &ctx("http://p.test/live/1.ts", s, s + 60)).unwrap(),
        format!("http://p.test/live/1.ts?offset=-60&start={s}")
    );
    assert_eq!(
        expand(&a, &ctx("http://p.test/live/1.ts?tok=1", s, s + 60)).unwrap(),
        format!("http://p.test/live/1.ts?tok=1&offset=-60&start={s}"),
        "existing query: ? becomes &"
    );
    let sh = tpl(Mode::Shift, None, TimeSemantics::UtcSeconds, 0);
    assert_eq!(
        expand(&sh, &ctx("http://p.test/c/3.ts", s, s + 60)).unwrap(),
        format!("http://p.test/c/3.ts?utc={s}&lutc=1900000000")
    );
    assert_eq!(
        expand(&sh, &ctx("http://p.test/c/3.ts?a=b", s, s + 60)).unwrap(),
        format!("http://p.test/c/3.ts?a=b&utc={s}&lutc=1900000000")
    );
}

#[test]
fn xc_mode_uses_server_wallclock_in_the_declared_zone() {
    // 2026-07-01 12:00Z with a London server (BST): the panel wants 13:00 local.
    let s = utc(2026, 7, 1, 12, 0);
    let t = tpl(Mode::Xc, None, wall("Europe/London"), 0);
    let u = expand(
        &t,
        &ctx("http://p.test:8080/live/alice/pw/42.m3u8", s, s + 3600),
    )
    .unwrap();
    assert_eq!(
        u,
        "http://p.test:8080/timeshift/alice/pw/60/2026-07-01:13-00/42.ts"
    );
    // Winter: no offset.
    let w = utc(2026, 1, 15, 12, 0);
    assert!(
        expand(&t, &ctx("http://p.test/live/alice/pw/42.ts", w, w + 3600))
            .unwrap()
            .contains("/2026-01-15:12-00/")
    );
    // A UTC-minus server (New York, EDT).
    let ny = tpl(Mode::Xc, None, wall("America/New_York"), 0);
    assert!(
        expand(&ny, &ctx("http://p.test/live/a/b/1.ts", s, s + 1800))
            .unwrap()
            .contains("/30/2026-07-01:08-00/1.ts")
    );
    // Minutes round up; correction applies first.
    let c = tpl(Mode::Xc, None, wall("Europe/London"), -1);
    assert_eq!(
        expand(&c, &ctx("http://p.test/live/a/b/1.ts", s, s + 3601)).unwrap(),
        "http://p.test/timeshift/a/b/61/2026-07-01:12-00/1.ts"
    );
    // Explicit parts (Xtream source): no live URL needed.
    let mut cx = ctx("", s, s + 3600);
    cx.xc = Some(XcParts {
        base: "https://x.test".into(),
        username: "u".into(),
        password: "p".into(),
        stream_id: "7".into(),
    });
    assert_eq!(
        expand(&t, &cx).unwrap(),
        "https://x.test/timeshift/u/p/60/2026-07-01:13-00/7.ts"
    );
    // Not an XC URL.
    assert!(expand(&t, &ctx("http://p.test/stream.ts", s, s + 60)).is_err());
}

#[test]
fn bad_templates_fail_with_a_plain_sentence_never_a_wrong_hour() {
    let s = utc(2026, 1, 1, 10, 0);
    let c = ctx("x", s, s + 60);
    assert!(
        expand(
            &tpl(
                Mode::Default,
                Some("{nonsense}"),
                TimeSemantics::UtcSeconds,
                0
            ),
            &c
        )
        .is_err()
    );
    assert!(expand(&tpl(Mode::Default, None, TimeSemantics::UtcSeconds, 0), &c).is_err());
    assert!(
        expand(
            &tpl(
                Mode::Default,
                Some("{duration:0}"),
                TimeSemantics::UtcSeconds,
                0
            ),
            &c
        )
        .is_err()
    );
    assert!(
        expand(
            &tpl(
                Mode::Default,
                Some("{duration:x}"),
                TimeSemantics::UtcSeconds,
                0
            ),
            &c
        )
        .is_err()
    );
    assert!(
        expand(
            &tpl(Mode::Default, Some("{utc}"), TimeSemantics::UtcSeconds, 0),
            &ctx("x", s, s)
        )
        .is_err(),
        "empty range"
    );
    // An unterminated brace is left as literal text, not a panic.
    assert_eq!(
        expand(
            &tpl(Mode::Default, Some("a{utc"), TimeSemantics::UtcSeconds, 0),
            &c
        )
        .unwrap(),
        "a{utc"
    );
    // Unknown zone name degrades to UTC rendering rather than failing.
    assert_eq!(
        expand(&tpl(Mode::Default, Some("{H}"), wall("Not/AZone"), 0), &c).unwrap(),
        "10"
    );
}

#[test]
fn from_m3u_header_and_entry_attributes() {
    let pl = parse(
        std::fs::File::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/fixtures/m3u/catchup.m3u"
        ))
        .unwrap(),
    )
    .unwrap();
    let get = |id: &str| pl.entries.iter().find(|e| e.tvg_id() == Some(id)).unwrap();
    let t = |id: &str| CatchupTemplate::from_m3u(&get(id).attrs, &pl.header, None);

    let d = t("cu1.test").unwrap();
    assert_eq!(
        (d.mode, d.days, &d.time),
        (Mode::Default, 7, &TimeSemantics::UtcSeconds),
        "header says catchup-timezone=UTC"
    );
    assert!(d.source.as_deref().unwrap().contains("{utcend}"));
    let a = t("cu2.test").unwrap();
    assert_eq!((a.mode, a.days), (Mode::Append, 3));
    assert_eq!(t("cu3.test").unwrap().mode, Mode::Shift);
    let x = t("cu4.test").unwrap();
    assert_eq!(
        (x.mode, x.correction_secs, &x.time),
        (Mode::Xc, -3600, &wall("Europe/London"))
    );
    assert!(t("cu6.test").is_none(), "flussonic is unsupported");
    // cu5: Dispatcharr tv_archive attrs but no mode: header default mode applies with the header source.
    let d5 = t("cu5.test").unwrap();
    assert_eq!(d5.days, 3);
    // cu7: catchup-days="abc", empty source -> falls back to the header's source and days.
    let d7 = t("cu7.test").unwrap();
    assert_eq!(d7.days, 7);
}

#[test]
fn json_roundtrip() {
    let t = tpl(Mode::Xc, None, wall("Europe/London"), -1);
    assert_eq!(CatchupTemplate::from_json(&t.to_json()), Some(t));
    assert!(CatchupTemplate::from_json("garbage").is_none());
    assert!(t_json_has_semantics());
}

fn t_json_has_semantics() -> bool {
    tpl(Mode::Xc, None, wall("UTC"), 0)
        .to_json()
        .contains("local_wallclock")
}

#[test]
fn xc_parts_from_live_url() {
    let p = XcParts::from_live_url("https://p.test:8443/live/al/pw/123.ts?x=1").unwrap();
    assert_eq!(
        (p.base.as_str(), p.username.as_str(), p.stream_id.as_str()),
        ("https://p.test:8443", "al", "123")
    );
    assert!(XcParts::from_live_url("http://p.test/other/1.ts").is_none());
}
