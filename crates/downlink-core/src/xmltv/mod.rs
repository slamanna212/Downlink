//! XMLTV (PLAN §6.4): streaming `quick-xml` parser, two passes (channels, then programmes),
//! gzip/xz sniffed by magic bytes, `YYYYMMDDHHMMSS ±HHMM` timestamps. Bad programmes are dropped
//! and counted, a truncated document yields everything before the cut.

pub mod ingest;
pub mod matching;

use std::io::BufReader;
use std::path::Path;

use chrono::{NaiveDate, TimeZone};
use chrono_tz::Tz;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::source::flags;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmltvChannel {
    pub id: String,
    pub display_names: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct XmltvProgramme {
    pub channel: String,
    pub start_utc: i64,
    pub end_utc: i64,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub flags: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct XmltvStats {
    pub programmes: u64,
    /// Dropped: unparseable times, missing channel, end <= start, missing stop.
    pub dropped: u64,
    /// Timestamps without an offset, read as the default timezone.
    pub assumed_tz: u64,
    /// The document ended abruptly or was not well formed past this point.
    pub truncated: bool,
}

/// Parse an XMLTV timestamp into UTC seconds. `Some((utc, assumed_tz))`; `assumed_tz` is true when
/// the stamp carried no offset and `default_tz` was applied.
pub fn parse_time(s: &str, default_tz: Tz) -> Option<(i64, bool)> {
    let s = s.trim();
    let digits_end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let d = &s[..digits_end];
    if !(8..=14).contains(&d.len()) || !d.len().is_multiple_of(2) {
        return None;
    }
    let num = |a: usize, b: usize| -> Option<u32> { d.get(a..b).and_then(|x| x.parse().ok()) };
    let (y, mo, da) = (num(0, 4)?, num(4, 6)?, num(6, 8)?);
    let (h, mi, se) = (
        num(8, 10).unwrap_or(0),
        num(10, 12).unwrap_or(0),
        num(12, 14).unwrap_or(0),
    );
    let naive = NaiveDate::from_ymd_opt(i32::try_from(y).ok()?, mo, da)?.and_hms_opt(h, mi, se)?;
    let tail = s[digits_end..].trim();
    if tail.is_empty() {
        let dt = default_tz.from_local_datetime(&naive).earliest()?;
        return Some((dt.timestamp(), true));
    }
    if tail.eq_ignore_ascii_case("z")
        || tail.eq_ignore_ascii_case("utc")
        || tail.eq_ignore_ascii_case("gmt")
    {
        return Some((naive.and_utc().timestamp(), false));
    }
    let sign = match tail.as_bytes().first()? {
        b'+' => 1i64,
        b'-' => -1i64,
        _ => return None,
    };
    let t: String = tail[1..].chars().filter(|c| *c != ':').collect();
    if !(2..=4).contains(&t.len()) || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let oh: i64 = t[..2].parse().ok()?;
    let om: i64 = if t.len() == 4 {
        t[2..].parse().ok()?
    } else {
        0
    };
    if oh > 14 || om > 59 {
        return None;
    }
    Some((
        naive.and_utc().timestamp() - sign * (oh * 3600 + om * 60),
        false,
    ))
}

fn attr(e: &BytesStart<'_>, name: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| {
            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .ok()
                .map(|v| v.into_owned())
        })
}

type XmlSource = quick_xml::encoding::DecodingReader<BufReader<Box<dyn std::io::Read + Send>>>;

fn open(path: &Path) -> std::io::Result<Reader<XmlSource>> {
    let r = crate::util::open_decompressed(path)?;
    // Auto-detects BOM / declared encoding (ISO-8859-1 guides are common) and transcodes to UTF-8.
    let mut reader = Reader::from_reader(quick_xml::encoding::DecodingReader::new(
        BufReader::with_capacity(64 * 1024, r),
    ));
    let cfg = reader.config_mut();
    cfg.check_end_names = false;
    cfg.allow_dangling_amp = true;
    cfg.allow_unmatched_ends = true;
    Ok(reader)
}

/// Resolve `&amp;`, `&lt;`, `&#169;`… into `out`.
fn push_ref(out: &mut String, name: &str) {
    if let Some(r) = quick_xml::escape::resolve_predefined_entity(name) {
        out.push_str(r);
    } else if let Some(num) = name.strip_prefix('#') {
        let cp = match num.strip_prefix(['x', 'X']) {
            Some(h) => u32::from_str_radix(h, 16).ok(),
            None => num.parse().ok(),
        };
        if let Some(c) = cp.and_then(char::from_u32) {
            out.push(c);
        }
    }
}

/// Pass 1: the channel list. Tolerates truncation (returns what was read).
pub fn parse_channels(path: &Path) -> std::io::Result<Vec<XmltvChannel>> {
    let mut reader = open(path)?;
    let mut buf = Vec::new();
    let mut out: Vec<XmltvChannel> = Vec::new();
    let mut cur: Option<XmltvChannel> = None;
    let mut in_name = false;
    let mut text = String::new();
    while let Ok(ev) = reader.read_event_into(&mut buf) {
        match ev {
            Event::Decl(d) => {
                if let Some(enc) = d.encoder() {
                    reader.get_mut().set_encoding(enc);
                }
            }
            Event::Start(e) => match e.local_name().as_ref() {
                "channel" => {
                    cur = attr(&e, "id")
                        .filter(|i| !i.is_empty())
                        .map(|id| XmltvChannel {
                            id,
                            display_names: vec![],
                        });
                }
                "display-name" if cur.is_some() => {
                    in_name = true;
                    text.clear();
                }
                _ => {}
            },
            Event::Text(t) if in_name => text.push_str(&t),
            Event::CData(t) if in_name => text.push_str(&t),
            Event::GeneralRef(r) if in_name => push_ref(&mut text, &r),
            Event::End(e) => match e.local_name().as_ref() {
                "display-name" if in_name => {
                    in_name = false;
                    let n = text.trim();
                    if let (Some(c), false) = (cur.as_mut(), n.is_empty()) {
                        c.display_names.push(n.to_owned());
                    }
                }
                "channel" => out.extend(cur.take()),
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Title,
    Sub,
    Desc,
    EpNs,
    EpScreen,
}

/// Pass 2: stream programmes into `on_programme`. Missing `stop`, `stop <= start`, unparseable
/// times or a missing channel drop the programme (counted).
pub fn parse_programmes(
    path: &Path,
    default_tz: Tz,
    mut on_programme: impl FnMut(XmltvProgramme),
) -> std::io::Result<XmltvStats> {
    let mut reader = open(path)?;
    let mut buf = Vec::new();
    let mut st = XmltvStats::default();
    let mut cur: Option<XmltvProgramme> = None;
    let mut bad = false;
    let mut field: Option<Field> = None;
    let mut text = String::new();
    let mut got_title = false;
    let mut ended = false;
    loop {
        let ev = match reader.read_event_into(&mut buf) {
            Ok(e) => e,
            Err(_) => {
                st.truncated = true;
                break;
            }
        };
        match ev {
            Event::Decl(ref d) => {
                if let Some(enc) = d.encoder() {
                    reader.get_mut().set_encoding(enc);
                }
            }
            Event::Start(ref e) | Event::Empty(ref e) => {
                let is_empty = matches!(ev, Event::Empty(_));
                match e.local_name().as_ref() {
                    "programme" => {
                        bad = false;
                        got_title = false;
                        let chan = attr(e, "channel").unwrap_or_default();
                        let mut p = XmltvProgramme {
                            channel: chan,
                            ..Default::default()
                        };
                        match (
                            attr(e, "start").and_then(|s| parse_time(&s, default_tz)),
                            attr(e, "stop").and_then(|s| parse_time(&s, default_tz)),
                        ) {
                            (Some((s, a1)), Some((en, a2))) => {
                                p.start_utc = s;
                                p.end_utc = en;
                                st.assumed_tz += u64::from(a1 || a2);
                            }
                            _ => bad = true,
                        }
                        if p.channel.is_empty() || p.end_utc <= p.start_utc {
                            bad = true;
                        }
                        if is_empty {
                            st.dropped += 1; // no title: nothing to show
                        } else {
                            cur = Some(p);
                        }
                    }
                    "title" if cur.is_some() && !got_title => {
                        field = Some(Field::Title);
                        text.clear();
                    }
                    "sub-title" if cur.is_some() => {
                        field = Some(Field::Sub);
                        text.clear();
                    }
                    "desc" if cur.is_some() => {
                        field = Some(Field::Desc);
                        text.clear();
                    }
                    "episode-num" if cur.is_some() => match attr(e, "system").as_deref() {
                        Some("xmltv_ns") => {
                            field = Some(Field::EpNs);
                            text.clear();
                        }
                        Some("onscreen") => {
                            field = Some(Field::EpScreen);
                            text.clear();
                        }
                        _ => {}
                    },
                    "new" => {
                        if let Some(c) = cur.as_mut() {
                            c.flags |= flags::NEW;
                        }
                    }
                    "premiere" => {
                        if let Some(c) = cur.as_mut() {
                            c.flags |= flags::PREMIERE;
                        }
                    }
                    "live" => {
                        if let Some(c) = cur.as_mut() {
                            c.flags |= flags::LIVE;
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(t) if field.is_some() => text.push_str(&t),
            Event::CData(t) if field.is_some() => text.push_str(&t),
            Event::GeneralRef(r) if field.is_some() => push_ref(&mut text, &r),
            Event::End(e) => match e.local_name().as_ref() {
                "title" | "sub-title" | "desc" | "episode-num" if field.is_some() => {
                    let v = text.trim().to_owned();
                    if let (Some(c), Some(f)) = (cur.as_mut(), field.take()) {
                        match f {
                            Field::Title => {
                                got_title = true;
                                c.title = (!v.is_empty()).then_some(v);
                            }
                            Field::Sub => c.subtitle = (!v.is_empty()).then_some(v),
                            Field::Desc => c.description = (!v.is_empty()).then_some(v),
                            Field::EpNs => {
                                let (s, e) = parse_xmltv_ns(&v);
                                c.season = c.season.or(s);
                                c.episode = c.episode.or(e);
                            }
                            Field::EpScreen => {
                                let (s, e) = parse_onscreen(&v);
                                c.season = c.season.or(s);
                                c.episode = c.episode.or(e);
                            }
                        }
                    }
                }
                "programme" => {
                    field = None;
                    if let Some(p) = cur.take() {
                        if bad || (p.title.is_none() && p.description.is_none()) {
                            st.dropped += 1;
                        } else {
                            st.programmes += 1;
                            on_programme(p);
                        }
                    }
                }
                "tv" => ended = true,
                _ => {}
            },
            Event::Eof => {
                if cur.is_some() || !ended {
                    st.truncated = true;
                }
                break;
            }
            _ => {}
        }
        buf.clear();
    }
    if cur.is_some() {
        st.dropped += 1;
    }
    Ok(st)
}

/// `xmltv_ns`: `season.episode.part` zero-based, e.g. `0.4.0/1`.
fn parse_xmltv_ns(s: &str) -> (Option<i64>, Option<i64>) {
    let mut it = s.split('.');
    let num = |p: Option<&str>| {
        p.and_then(|x| x.split('/').next())
            .and_then(|x| x.trim().parse::<i64>().ok())
            .map(|n| n + 1)
    };
    (num(it.next()), num(it.next()))
}

/// `S01E05` / `s1e5`.
fn parse_onscreen(s: &str) -> (Option<i64>, Option<i64>) {
    let u = s.to_ascii_uppercase();
    let Some(si) = u.find('S') else {
        return (None, None);
    };
    let Some(ei) = u[si..].find('E').map(|i| i + si) else {
        return (None, None);
    };
    (
        u[si + 1..ei].trim().parse().ok(),
        u[ei + 1..].trim().parse().ok(),
    )
}

#[cfg(test)]
mod tests;
