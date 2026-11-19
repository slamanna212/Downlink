//! Joining playlist channels to XMLTV channels (PLAN §6.4). Order: manual remap, `tvg-id` ==
//! `<channel id>`, `tvg-name` == `<display-name>` (also spaces to underscores), then the channel's
//! own name. Case-insensitive, quality suffix stripped. Every non-id match is recorded so the UI
//! can say "guide matched by name".

use std::collections::HashMap;

use super::XmltvChannel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Manual,
    Id,
    Name,
    Display,
}

impl Method {
    /// Value stored in `channel.epg_match`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Id => "id",
            Self::Name => "name",
            Self::Display => "display",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChanInfo {
    pub id: i64,
    pub ext_id: String,
    pub tvg_id: Option<String>,
    pub tvg_name: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub channel_id: i64,
    pub xmltv_id: String,
    pub method: Method,
}

/// Lowercase, quality suffix stripped, whitespace collapsed.
pub fn normalise_name(s: &str) -> String {
    let (stripped, _) = crate::normalise::split_quality(s);
    stripped
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn underscored(s: &str) -> String {
    s.replace(' ', "_")
}

/// Match as many channels as possible. Unmatched channels are simply absent from the result.
pub fn match_channels(
    chans: &[ChanInfo],
    xmltv: &[XmltvChannel],
    remap: &HashMap<String, String>,
) -> Vec<Match> {
    let mut by_id: HashMap<String, &str> = HashMap::new();
    let mut by_display: HashMap<String, &str> = HashMap::new();
    for c in xmltv {
        by_id.entry(c.id.to_lowercase()).or_insert(&c.id);
        for d in &c.display_names {
            let n = normalise_name(d);
            if !n.is_empty() {
                by_display.entry(underscored(&n)).or_insert(&c.id);
                by_display.entry(n).or_insert(&c.id);
            }
        }
    }
    let mut out = Vec::new();
    for ch in chans {
        let m = if let Some(x) = remap
            .get(&ch.ext_id)
            .and_then(|r| by_id.get(&r.to_lowercase()))
        {
            Some((*x, Method::Manual))
        } else if let Some(x) = ch
            .tvg_id
            .as_deref()
            .and_then(|t| by_id.get(&t.trim().to_lowercase()))
        {
            Some((*x, Method::Id))
        } else if let Some(x) = ch.tvg_name.as_deref().and_then(|t| {
            let n = normalise_name(t);
            by_display
                .get(&n)
                .or_else(|| by_display.get(&underscored(&n)))
                .or_else(|| by_id.get(&underscored(&n)))
        }) {
            Some((*x, Method::Name))
        } else {
            let n = normalise_name(&ch.name);
            by_display
                .get(&n)
                .or_else(|| by_display.get(&underscored(&n)))
                .map(|x| (*x, Method::Display))
        };
        if let Some((xid, method)) = m {
            out.push(Match {
                channel_id: ch.id,
                xmltv_id: xid.to_owned(),
                method,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xc(id: &str, names: &[&str]) -> XmltvChannel {
        XmltvChannel {
            id: id.into(),
            display_names: names.iter().map(|s| (*s).into()).collect(),
        }
    }

    fn ch(id: i64, tvg: Option<&str>, tvg_name: Option<&str>, name: &str) -> ChanInfo {
        ChanInfo {
            id,
            ext_id: format!("e{id}"),
            tvg_id: tvg.map(Into::into),
            tvg_name: tvg_name.map(Into::into),
            name: name.into(),
        }
    }

    #[test]
    fn precedence_and_methods() {
        let x = [
            xc("one.test", &["Channel One"]),
            xc("News_Two", &["News Two"]),
            xc("c3", &["Channel Three"]),
            xc("man", &["Manual Target"]),
        ];
        let chans = [
            ch(1, Some("ONE.TEST"), None, "Anything"), // id, case-insensitive
            ch(2, Some("missing"), Some("News Two HD"), "N2"), // tvg-name vs display (quality stripped)
            ch(3, None, None, "Channel  Three FHD"),           // display name, whitespace + quality
            ch(4, Some("zzz"), Some("News_Two"), "Four"), // tvg-name underscore form matches the id
            ch(5, None, None, "Nobody"),                  // unmatched
            ch(6, Some("one.test"), None, "Manual Wins"), // remapped below
        ];
        let mut remap = HashMap::new();
        remap.insert("e6".to_owned(), "MAN".to_owned());
        let m = match_channels(&chans, &x, &remap);
        let get = |id| m.iter().find(|x| x.channel_id == id);
        assert_eq!(
            get(1).map(|m| (m.xmltv_id.as_str(), m.method)),
            Some(("one.test", Method::Id))
        );
        assert_eq!(
            get(2).map(|m| (m.xmltv_id.as_str(), m.method)),
            Some(("News_Two", Method::Name))
        );
        assert_eq!(
            get(3).map(|m| (m.xmltv_id.as_str(), m.method)),
            Some(("c3", Method::Display))
        );
        assert_eq!(
            get(4).map(|m| (m.xmltv_id.as_str(), m.method)),
            Some(("News_Two", Method::Name))
        );
        assert!(get(5).is_none());
        assert_eq!(
            get(6).map(|m| (m.xmltv_id.as_str(), m.method)),
            Some(("man", Method::Manual))
        );
        assert_eq!(Method::Name.as_str(), "name");
    }

    #[test]
    fn remap_to_unknown_xmltv_id_falls_through() {
        let x = [xc("a", &["Alpha"])];
        let mut remap = HashMap::new();
        remap.insert("e1".to_owned(), "gone".to_owned());
        let m = match_channels(&[ch(1, Some("a"), None, "Alpha")], &x, &remap);
        assert_eq!(m[0].method, Method::Id);
    }

    #[test]
    fn normalisation() {
        assert_eq!(normalise_name("  Channel   One  HD "), "channel one");
        assert_eq!(normalise_name("HD"), "hd");
    }
}
