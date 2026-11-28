//! The embedded sports catalog (docs/SPORTS.md §2): parsed and validated once, then compiled into
//! normalized phrases the matcher can compare word by word.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use serde::Deserialize;

use super::matcher::normalize;
use crate::error::{AppError, Result};

const EMBEDDED: &str = include_str!("catalog.toml");

/// Motorsport groups (§2.2), in display order.
pub const GROUPS: [&str; 6] = [
    "open_wheel",
    "stock_car",
    "sports_car",
    "motorcycle",
    "rally",
    "drag_dirt",
];

/// Session types (§2.3), in the order the settings page lists them.
pub const SESSIONS: [&str; 4] = ["race", "qualifying", "practice", "shows"];

#[derive(Debug, Deserialize)]
struct RawCatalog {
    #[serde(default)]
    sessions: RawSessions,
    #[serde(default)]
    words: RawWords,
    #[serde(default, rename = "league")]
    leagues: Vec<RawLeague>,
}

#[derive(Debug, Default, Deserialize)]
struct RawSessions {
    #[serde(default)]
    shows: Vec<String>,
    #[serde(default)]
    qualifying: Vec<String>,
    #[serde(default)]
    practice: Vec<String>,
    #[serde(default)]
    race: Vec<String>,
    #[serde(default)]
    neutral: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawWords {
    #[serde(default)]
    replay: Vec<String>,
    #[serde(default)]
    talk: Vec<String>,
    #[serde(default)]
    sports_groups: Vec<String>,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    filler: Vec<String>,
    #[serde(default)]
    feeds: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawLeague {
    key: String,
    name: String,
    kind: String,
    #[serde(default)]
    group: Option<String>,
    #[serde(default)]
    tokens: Vec<String>,
    #[serde(default)]
    context: Vec<String>,
    #[serde(default)]
    events: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    #[serde(default, rename = "team")]
    teams: Vec<RawTeam>,
}

#[derive(Debug, Deserialize)]
struct RawTeam {
    key: String,
    name: String,
    short: String,
    abbr: String,
    #[serde(default)]
    city: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    weak: Vec<String>,
}

/// A catalog phrase: the original text (for FTS queries) and its normalized words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phrase {
    pub raw: String,
    pub words: Vec<String>,
}

impl Phrase {
    pub fn new(raw: &str) -> Self {
        Self {
            raw: raw.to_owned(),
            words: normalize(raw),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Team,
    Motorsport,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Team => "team",
            Self::Motorsport => "motorsport",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Team {
    pub key: String,
    pub name: String,
    pub short: String,
    pub abbr: String,
    /// Unique enough to count on their own.
    pub strong: Vec<Phrase>,
    /// Cities, shared nicknames, `weak` aliases: only count with league context.
    pub weak: Vec<Phrase>,
    /// Only used inside a versus pattern.
    pub abbr_phrase: Phrase,
}

#[derive(Debug, Clone)]
pub struct League {
    pub key: String,
    pub name: String,
    pub kind: Kind,
    pub group: Option<String>,
    pub tokens: Vec<Phrase>,
    pub context: Vec<Phrase>,
    pub events: Vec<Phrase>,
    pub exclude: Vec<Phrase>,
    pub teams: Vec<Team>,
}

#[derive(Debug, Clone, Default)]
pub struct SessionTerms {
    pub shows: Vec<Phrase>,
    pub qualifying: Vec<Phrase>,
    pub practice: Vec<Phrase>,
    pub race: Vec<Phrase>,
    /// Event names that contain a session term but are not a session ("Grand Final"): terms
    /// inside them do not count, and they classify as nothing themselves.
    pub neutral: Vec<Phrase>,
}

#[derive(Debug, Clone, Default)]
pub struct Words {
    pub replay: Vec<Phrase>,
    pub talk: Vec<Phrase>,
    pub sports_groups: Vec<Phrase>,
    pub categories: Vec<Phrase>,
    /// Placeholder listings that are never an event ("No Game Today", "No Event").
    pub filler: Vec<Phrase>,
    /// Secondary feeds of an event channel (on-board cameras, timing): never their own match.
    pub feeds: Vec<Phrase>,
}

#[derive(Debug, Clone)]
pub struct Catalog {
    pub leagues: Vec<League>,
    pub sessions: SessionTerms,
    pub words: Words,
}

impl Catalog {
    pub fn league(&self, key: &str) -> Option<&League> {
        self.leagues.iter().find(|l| l.key == key)
    }
}

fn phrases(v: &[String]) -> Vec<Phrase> {
    v.iter()
        .map(|s| Phrase::new(s))
        .filter(|p| !p.words.is_empty())
        .collect()
}

/// Parse and validate a catalog document.
pub fn parse(text: &str) -> std::result::Result<Catalog, String> {
    let raw: RawCatalog = toml::from_str(text).map_err(|e| format!("catalog.toml: {e}"))?;
    if raw.leagues.is_empty() {
        return Err("catalog has no leagues".into());
    }
    let mut keys = HashSet::new();
    for l in &raw.leagues {
        if l.key.trim().is_empty() || l.name.trim().is_empty() {
            return Err("a league has an empty key or name".into());
        }
        if !keys.insert(l.key.as_str()) {
            return Err(format!("duplicate league key `{}`", l.key));
        }
        match l.kind.as_str() {
            "team" => {
                if l.group.is_some() {
                    return Err(format!("team league `{}` has a group", l.key));
                }
            }
            "motorsport" => match l.group.as_deref() {
                Some(g) if GROUPS.contains(&g) => {}
                Some(g) => return Err(format!("league `{}` has unknown group `{g}`", l.key)),
                None => return Err(format!("motorsport league `{}` has no group", l.key)),
            },
            k => return Err(format!("league `{}` has unknown kind `{k}`", l.key)),
        }
        if phrases(&l.tokens).is_empty() {
            return Err(format!("league `{}` has no tokens", l.key));
        }
        if l.kind == "motorsport" && !l.teams.is_empty() {
            return Err(format!("motorsport league `{}` has teams", l.key));
        }
        let mut team_keys = HashSet::new();
        for t in &l.teams {
            if t.key.trim().is_empty() || normalize(&t.name).is_empty() {
                return Err(format!("league `{}` has a team with no key or name", l.key));
            }
            if !team_keys.insert(t.key.as_str()) {
                return Err(format!("duplicate team key `{}` in `{}`", t.key, l.key));
            }
        }
    }

    // Strong aliases (name, short, aliases): a duplicate inside one league is a data error; one
    // found in more than one league is a shared nickname and becomes weak (§2.1).
    let mut alias_leagues: HashMap<Vec<String>, HashSet<&str>> = HashMap::new();
    for l in &raw.leagues {
        let mut seen: HashMap<Vec<String>, &str> = HashMap::new();
        for t in &l.teams {
            let mut own = HashSet::new();
            for a in std::iter::once(&t.name)
                .chain(std::iter::once(&t.short))
                .chain(t.aliases.iter())
            {
                let w = normalize(a);
                if w.is_empty() || !own.insert(w.clone()) {
                    continue;
                }
                if let Some(other) = seen.insert(w.clone(), t.key.as_str()) {
                    return Err(format!(
                        "alias `{a}` is used by both `{other}` and `{}` in `{}`",
                        t.key, l.key
                    ));
                }
                alias_leagues.entry(w).or_default().insert(l.key.as_str());
            }
            // Explicit weak nicknames count toward "shared across leagues" too (cities do not).
            for a in &t.weak {
                let w = normalize(a);
                if !w.is_empty() {
                    alias_leagues.entry(w).or_default().insert(l.key.as_str());
                }
            }
        }
    }
    let shared = |w: &Vec<String>| alias_leagues.get(w).is_some_and(|s| s.len() > 1);

    let leagues = raw
        .leagues
        .iter()
        .map(|l| League {
            key: l.key.clone(),
            name: l.name.clone(),
            kind: if l.kind == "team" {
                Kind::Team
            } else {
                Kind::Motorsport
            },
            group: l.group.clone(),
            tokens: phrases(&l.tokens),
            context: phrases(&l.context),
            events: phrases(&l.events),
            exclude: phrases(&l.exclude),
            teams: l
                .teams
                .iter()
                .map(|t| {
                    let mut strong = Vec::new();
                    let mut weak = Vec::new();
                    for a in std::iter::once(&t.name)
                        .chain(std::iter::once(&t.short))
                        .chain(t.aliases.iter())
                    {
                        let p = Phrase::new(a);
                        if p.words.is_empty()
                            || strong.contains(&p)
                            || weak.iter().any(|w: &Phrase| w.words == p.words)
                        {
                            continue;
                        }
                        if shared(&p.words) {
                            weak.push(p);
                        } else {
                            strong.push(p);
                        }
                    }
                    // A city or weak alias that is also a strong alias stays strong
                    // ("Liverpool" as both city and nickname).
                    for a in t.city.iter().chain(t.weak.iter()) {
                        let p = Phrase::new(a);
                        if !p.words.is_empty()
                            && !weak.iter().any(|w| w.words == p.words)
                            && !strong.iter().any(|s: &Phrase| s.words == p.words)
                        {
                            weak.push(p);
                        }
                    }
                    Team {
                        key: t.key.clone(),
                        name: t.name.clone(),
                        short: t.short.clone(),
                        abbr: t.abbr.clone(),
                        strong,
                        weak,
                        abbr_phrase: Phrase::new(&t.abbr),
                    }
                })
                .collect(),
        })
        .collect();

    Ok(Catalog {
        leagues,
        sessions: SessionTerms {
            shows: phrases(&raw.sessions.shows),
            qualifying: phrases(&raw.sessions.qualifying),
            practice: phrases(&raw.sessions.practice),
            race: phrases(&raw.sessions.race),
            neutral: phrases(&raw.sessions.neutral),
        },
        words: Words {
            replay: phrases(&raw.words.replay),
            talk: phrases(&raw.words.talk),
            sports_groups: phrases(&raw.words.sports_groups),
            categories: phrases(&raw.words.categories),
            filler: phrases(&raw.words.filler),
            feeds: phrases(&raw.words.feeds),
        },
    })
}

static CATALOG: LazyLock<std::result::Result<Catalog, String>> = LazyLock::new(|| parse(EMBEDDED));

/// Identifies the stored `sports_match` rows' rules: the catalog text plus the matcher version.
/// When it changes (an app update with new aliases or scoring), the rows are rebuilt at startup
/// instead of waiting for the next guide refresh. FNV-1a: stable across builds and toolchains.
pub fn fingerprint() -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in EMBEDDED.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{}:{h:016x}", super::matcher::MATCHER_VERSION)
}

/// The embedded catalog, parsed once.
pub fn catalog() -> Result<&'static Catalog> {
    CATALOG.as_ref().map_err(|e| {
        tracing::error!(error = %e, "sports catalog is invalid");
        AppError::local("The sports catalog could not be loaded.")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = r#"
[sessions]
race = ["Race"]
[words]
replay = ["Classic"]
"#;

    fn with(leagues: &str) -> std::result::Result<Catalog, String> {
        parse(&format!("{BASE}\n{leagues}"))
    }

    #[test]
    fn embedded_catalog_parses_and_validates() {
        let c = catalog().unwrap();
        assert!(!c.leagues.is_empty());
        for l in &c.leagues {
            assert!(!l.tokens.is_empty(), "{}", l.key);
        }
        assert!(!c.sessions.race.is_empty());
        // Every league the spec lists (docs/SPORTS.md §2.1, §2.2) is present with the right kind.
        for k in ["nfl", "mlb", "nba", "nhl", "mls", "epl"] {
            let l = c.league(k).unwrap_or_else(|| panic!("missing {k}"));
            assert_eq!(l.kind, Kind::Team, "{k}");
            assert!(!l.teams.is_empty(), "{k} has no teams");
        }
        for k in [
            "f1",
            "f2",
            "f3",
            "f1_academy",
            "indycar",
            "indy_nxt",
            "formula_e",
            "super_formula",
            "nascar_cup",
            "nascar_oreilly",
            "nascar_truck",
            "arca",
            "wec",
            "imsa",
            "supercars",
            "btcc",
            "dtm",
            "gtwc",
            "super_gt",
            "motogp",
            "moto2",
            "moto3",
            "wsbk",
            "supercross",
            "pro_motocross",
            "motoamerica",
            "wrc",
            "dakar",
            "nrx",
            "extreme_h",
            "nhra",
            "woo",
            "high_limit",
        ] {
            let l = c.league(k).unwrap_or_else(|| panic!("missing {k}"));
            assert_eq!(l.kind, Kind::Motorsport, "{k}");
        }
    }

    /// Behavior the real data must keep (invented titles, real catalog).
    #[test]
    fn embedded_catalog_spot_checks() {
        use crate::source::flags::LIVE;
        use crate::sports::matcher::{Candidate, score_programme};
        let c = catalog().unwrap();
        let top = |title: &str| -> Vec<(String, Option<&'static str>)> {
            let cand = Candidate {
                title,
                channel_name: "Channel 5",
                flags: LIVE,
                ..Default::default()
            };
            score_programme(c, &cand, true)
                .into_iter()
                .map(|m| (m.league, m.session))
                .collect()
        };
        assert_eq!(
            top("F1 Academy: Race 1"),
            [("f1_academy".into(), Some("race"))]
        );
        assert_eq!(
            top("Formula 1: Sprint Qualifying"),
            [("f1".into(), Some("qualifying"))]
        );
        assert_eq!(
            top("Formula 1: Final Practice"),
            [("f1".into(), Some("practice"))]
        );
        assert_eq!(top("Formula 1: Pre-race"), [("f1".into(), Some("shows"))]);
        assert_eq!(top("MotoGP: Sprint"), [("motogp".into(), Some("race"))]);
        assert_eq!(top("Moto2: Race"), [("moto2".into(), Some("race"))]);
        assert!(top("F150 Showdown").is_empty());
        assert!(top("Giants at Cardinals").is_empty());
        assert_eq!(top("NFL Football: Giants at Cardinals")[0].0, "nfl");
        assert_eq!(top("MLB Baseball: Giants at Cardinals")[0].0, "mlb");
    }

    /// Listing shapes seen in real guides (invented titles and channels): live markers, filler
    /// on team channels, pre/post shows, "Best of" repeats and event-channel feeds.
    #[test]
    fn grand_final_is_an_event_name_not_a_session() {
        use crate::sports::matcher::{Candidate, score_programme};
        let c = catalog().unwrap();
        let on = |title: &str| {
            let cand = Candidate {
                title,
                ..Default::default()
            };
            score_programme(c, &cand, true)
        };
        let q = on("Supercars Harbor Grand Final : Qualifying & Top 10 Shootout ᴸᶦᵛᵉ");
        assert_eq!(q[0].session, Some("qualifying"));
        let r = on("Supercars Championship : Race 36: Harbor Grand Final ᴸᶦᵛᵉ");
        assert_eq!(r[0].session, Some("race"));
        // The pre-race show is a show, and the support categories are not Supercars.
        let g = on("Supercars Harbor Grand Final : The Grid ᴸᶦᵛᵉ");
        assert_eq!(g[0].session, Some("shows"));
        assert!(on("Supercars Harbor Grand Final : Supports ᴸᶦᵛᵉ").is_empty());
    }

    #[test]
    fn embedded_catalog_guide_shapes() {
        use crate::sports::matcher::{Candidate, live_marker, score_channel, score_programme};
        let c = catalog().unwrap();
        let on = |title: &str, channel: &'static str, trusted: bool| {
            let cand = Candidate {
                title,
                channel_name: channel,
                description: Some("The Eagles host Arizona at the stadium."),
                ..Default::default()
            };
            score_programme(c, &cand, trusted)
        };

        // Superscript and bracketed live markers.
        assert!(live_marker(
            "Formula 1 : Desert Grand Prix: Qualifying ᴸᶦᵛᵉ",
            None
        ));
        assert!(live_marker("NFL Football (Live)", None));
        assert!(live_marker("LIVE: Premier League", None));
        assert!(!live_marker(
            "Formula 1 : Desert Grand Prix: Qualifying",
            None
        ));
        assert!(!live_marker("Live from the Paddock", None));

        // Filler on a team channel never matches, even with the team in the channel name.
        assert!(on("No Game Today", "Philadelphia Eagles", false).is_empty());
        // The listing must name the league or a team itself; the channel name is not enough.
        assert!(on("Classic Rewind Hour", "Philadelphia Eagles", false).is_empty());
        assert!(on("Team Practice Report", "Philadelphia Eagles", false).is_empty());
        // A real game on that channel still matches.
        let game = on(
            "NFL Football : Arizona Cardinals at Philadelphia Eagles ᴸᶦᵛᵉ",
            "Philadelphia Eagles",
            true,
        );
        assert_eq!(game[0].league, "nfl");
        assert!(game[0].live && !game[0].live_inferred);

        // A trusted source without the marker on this airing: a replay.
        let rerun = on(
            "Formula 1 : Desert Grand Prix: 2nd Practice",
            "Racing One",
            true,
        );
        assert_eq!(rerun[0].session, Some("practice"));
        assert!(!rerun[0].live && !rerun[0].live_inferred);
        let live = on(
            "Formula 1 : Desert Grand Prix: 2nd Practice ᴸᶦᵛᵉ",
            "Racing One",
            true,
        );
        assert!(live[0].live);

        // Pre- and post-race shows are shows, not the race.
        let s = |t: &str| on(t, "Racing One", true).first().and_then(|m| m.session);
        assert_eq!(
            s("Formula 1 : Desert Grand Prix: Grand Prix Sunday ᴸᶦᵛᵉ"),
            Some("shows")
        );
        assert_eq!(
            s("F1 Chequered Flag : Desert Grand Prix ᴸᶦᵛᵉ"),
            Some("shows")
        );
        assert_eq!(
            s("Formula 1 Pre Qualifying : Desert Grand Prix ᴸᶦᵛᵉ"),
            Some("shows")
        );
        assert_eq!(s("Formula 1 : Desert Grand Prix: Race ᴸᶦᵛᵉ"), Some("race"));

        // "Best of" is a replay; a bare series title is only the race when marked live.
        let best = on("Formula 1 : The Best of Lakeside 2026", "Racing One", false);
        assert!(best.iter().all(|m| !m.live));
        assert!(on("Formula 1", "Racing Mix", false).is_empty());
        assert_eq!(
            on("Formula 1 ᴸᶦᵛᵉ", "Racing Mix", true)[0].session,
            Some("race")
        );

        // Event channels: placeholder and secondary feeds are dropped; the main event matches.
        assert!(score_channel(c, "Sport+ 50: NO EVENT", None).is_empty());
        assert!(score_channel(c, "Sport+ 14: Formula 1 On-Board Camera: Driver A - Desert Grand Prix Practice #2 @ 27 Nov 11:50 AM ET", None).is_empty());
        assert!(
            score_channel(
                c,
                "Sport+ 15: Formula 1 Timing Channel - Desert Grand Prix Practice #2",
                None
            )
            .is_empty()
        );
        assert_eq!(
            score_channel(
                c,
                "Sport+ 09: Formula 2 Qualifying: Lakeside @ 27 Nov 09:55 AM ET",
                None
            )[0]
            .league,
            "f2"
        );
    }

    #[test]
    fn duplicate_league_keys_are_rejected() {
        let e = with(
            r#"
[[league]]
key = "x"
name = "X"
kind = "team"
tokens = ["X League"]
[[league]]
key = "x"
name = "Y"
kind = "team"
tokens = ["Y League"]
"#,
        )
        .unwrap_err();
        assert!(e.contains("duplicate league key"), "{e}");
    }

    #[test]
    fn unknown_kind_and_group_are_rejected() {
        let e = with(
            r#"
[[league]]
key = "x"
name = "X"
kind = "ball"
tokens = ["X"]
"#,
        )
        .unwrap_err();
        assert!(e.contains("unknown kind"), "{e}");
        let e = with(
            r#"
[[league]]
key = "x"
name = "X"
kind = "motorsport"
group = "boats"
tokens = ["X"]
"#,
        )
        .unwrap_err();
        assert!(e.contains("unknown group"), "{e}");
        let e = with(
            r#"
[[league]]
key = "x"
name = "X"
kind = "motorsport"
tokens = ["X"]
"#,
        )
        .unwrap_err();
        assert!(e.contains("no group"), "{e}");
    }

    #[test]
    fn duplicate_team_keys_and_alias_collisions_are_rejected() {
        let e = with(
            r#"
[[league]]
key = "x"
name = "X"
kind = "team"
tokens = ["X"]
  [[league.team]]
  key = "a"
  name = "Alpha Owls"
  short = "Owls"
  abbr = "ALP"
  [[league.team]]
  key = "a"
  name = "Beta Bats"
  short = "Bats"
  abbr = "BET"
"#,
        )
        .unwrap_err();
        assert!(e.contains("duplicate team key"), "{e}");
        let e = with(
            r#"
[[league]]
key = "x"
name = "X"
kind = "team"
tokens = ["X"]
  [[league.team]]
  key = "a"
  name = "Alpha Owls"
  short = "Owls"
  abbr = "ALP"
  [[league.team]]
  key = "b"
  name = "Beta Owls"
  short = "Owls"
  abbr = "BET"
"#,
        )
        .unwrap_err();
        assert!(e.contains("used by both"), "{e}");
    }

    #[test]
    fn nickname_shared_across_leagues_becomes_weak() {
        let c = with(
            r#"
[[league]]
key = "x"
name = "X"
kind = "team"
tokens = ["X League"]
  [[league.team]]
  key = "a"
  name = "Alpha Owls"
  short = "Owls"
  abbr = "ALP"
  city = "Alpha"
[[league]]
key = "y"
name = "Y"
kind = "team"
tokens = ["Y League"]
  [[league.team]]
  key = "b"
  name = "Beta Owls"
  short = "Owls"
  abbr = "BET"
  weak = ["Night Birds"]
"#,
        )
        .unwrap();
        let a = &c.leagues[0].teams[0];
        let words = |v: &[Phrase]| v.iter().map(|p| p.words.join(" ")).collect::<Vec<_>>();
        assert_eq!(words(&a.strong), ["alpha owls"]);
        assert_eq!(words(&a.weak), ["owls", "alpha"]);
        let b = &c.leagues[1].teams[0];
        assert_eq!(words(&b.strong), ["beta owls"]);
        assert_eq!(words(&b.weak), ["owls", "night birds"]);
    }
}
