//! Pure sports scoring (docs/SPORTS.md §5). No I/O: the store hands in plain candidates and the
//! catalog, and gets back the leagues each candidate matches.

use std::collections::HashSet;
use std::sync::LazyLock;

use chrono::{Duration, NaiveTime, TimeZone};
use chrono_tz::Tz;
use regex::Regex;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use super::catalog::{Catalog, Kind, League, Phrase};
use crate::service::dto::SportsFollowDto;
use crate::source::flags;

pub const THRESHOLD: i64 = 50;

/// Bump on any change to scoring or session rules here: stored matches are rebuilt at startup
/// when this (or the catalog text) changes. See `catalog::fingerprint`.
pub const MATCHER_VERSION: u32 = 3;

const TOKEN_TITLE: i64 = 40;
const TOKEN_OTHER: i64 = 25;
const EVENT: i64 = 40;
const TEAM_STRONG: i64 = 30;
const TEAM_WEAK: i64 = 10;
const MAX_TEAMS_SCORED: usize = 2;
const VERSUS: i64 = 20;
const CATEGORY: i64 = 15;
/// A motorsport session term ("Qualifying", "FP2") next to the series name.
const SESSION: i64 = 15;
const SPORTS_GROUP: i64 = 10;
const LIVE_FLAG: i64 = 10;
const NEW_FLAG: i64 = 5;
const EXCLUDE: i64 = -100;
const REPLAY: i64 = -40;
const TALK: i64 = -30;
/// League score that makes weak aliases (cities, shared nicknames, abbreviations) count.
const CONTEXT: i64 = 25;

const CONNECTORS: [&str; 4] = ["at", "vs", "v", "@"];

/// Superscript and modifier letters: the "ᴸᶦᵛᵉ" / "ᴺᵉʷ" guide markers.
fn is_superscript(c: char) -> bool {
    matches!(c,
        '\u{02B0}'..='\u{02FF}'
        | '\u{1D2C}'..='\u{1D6A}'
        | '\u{1D78}'
        | '\u{1D9B}'..='\u{1DBF}'
        | '\u{2070}'..='\u{209F}')
}

/// Put a space before each superscript run, so a marker glued to the last word
/// ("Rocketsᴸᶦᵛᵉ") folds to its own word ("Rockets Live") instead of "RocketsLive".
fn split_superscripts(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    let mut prev_super = true;
    for c in s.chars() {
        let sup = is_superscript(c);
        if sup && !prev_super {
            out.push(' ');
        }
        prev_super = sup || c.is_whitespace();
        out.push(c);
    }
    out
}

/// Lowercase, strip diacritics, fold compatibility forms (superscript "ᴸᶦᵛᵉ" -> "live", split
/// from a word it is glued to), "&" -> "and", keep "@" as its own word, punctuation -> spaces.
pub fn normalize(s: &str) -> Vec<String> {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in split_superscripts(s).nfkd() {
        if is_combining_mark(ch) {
            continue;
        }
        match ch {
            '&' => out.push_str(" and "),
            '@' => out.push_str(" @ "),
            c if c.is_alphanumeric() => out.extend(c.to_lowercase()),
            _ => out.push(' '),
        }
    }
    out.split_whitespace().map(str::to_owned).collect()
}

static LIVE_MARK_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)(^\s*live\s*[:|\-–]|[(\[]\s*live\s*[)\]]|[:|\-–]\s*live\s*$|\blive\s*$)").ok()
});

/// The guide marks this airing live in its text: a superscript "ᴸᶦᵛᵉ" suffix, "(Live)",
/// "[Live]", a "LIVE:" prefix or a trailing "Live".
pub fn live_marker(title: &str, subtitle: Option<&str>) -> bool {
    let Some(re) = LIVE_MARK_RE.as_ref() else {
        return false;
    };
    [Some(title), subtitle].into_iter().flatten().any(|t| {
        // NFKC folds "ᴸᶦᵛᵉ" to "Lɪve" (the superscript i is a small capital): fold that too.
        let folded: String = split_superscripts(t)
            .nfkc()
            .map(|c| if c == 'ɪ' { 'i' } else { c })
            .collect();
        re.is_match(folded.trim())
    })
}

/// Start positions of `phrase` in `hay`, on word boundaries.
fn positions<'a>(hay: &'a [String], phrase: &'a [String]) -> impl Iterator<Item = usize> + 'a {
    let n = phrase.len();
    let last = if n == 0 || hay.len() < n {
        0
    } else {
        hay.len() - n + 1
    };
    (0..last).filter(move |&i| hay[i..i + n] == *phrase)
}

fn has(hay: &[String], p: &Phrase) -> bool {
    positions(hay, &p.words).next().is_some()
}

fn any(hay: &[String], ps: &[Phrase]) -> bool {
    ps.iter().any(|p| has(hay, p))
}

/// A guide programme (or an event channel) to score. Plain data, so tests build it inline.
#[derive(Debug, Clone, Default)]
pub struct Candidate<'a> {
    pub title: &'a str,
    pub subtitle: Option<&'a str>,
    pub description: Option<&'a str>,
    pub channel_name: &'a str,
    pub group_name: Option<&'a str>,
    /// XMLTV categories, when the guide has them.
    pub categories: &'a [String],
    /// `crate::source::flags` bits.
    pub flags: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Programme,
    Channel,
}

/// One league a candidate matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub league: String,
    /// Team keys that matched, followed or not (catalog order).
    pub teams: Vec<String>,
    /// Motorsport only.
    pub session: Option<&'static str>,
    pub live: bool,
    pub live_inferred: bool,
    pub replay: bool,
    pub score: i64,
}

struct Hay {
    title: Vec<String>,
    subtitle: Vec<String>,
    description: Vec<String>,
    channel: Vec<String>,
    group: Vec<String>,
    categories: Vec<Vec<String>>,
}

/// Facts about the candidate that do not depend on the league.
struct Facts {
    /// "from 19xx/20xx" in the title or subtitle.
    from_year: bool,
    talk: bool,
    session: Option<&'static str>,
    sports_group: bool,
    category: bool,
}

fn from_year(words: &[String]) -> bool {
    words.windows(2).any(|w| {
        w[0] == "from"
            && w[1].len() == 4
            && w[1]
                .parse::<u32>()
                .is_ok_and(|y| (1900..=2099).contains(&y))
    })
}

/// Session of a motorsport title (§2.3). A term inside a longer matched term does not count
/// ("Superpole" inside "Superpole Race", "Race" inside "Pre-race", "Sprint" inside "Sprint
/// Qualifying", "Final" inside the neutral "Grand Final"); of the rest, the first in the order
/// shows, qualifying, practice, race wins. Neutral event names never classify.
fn classify_session(cat: &Catalog, title: &[String], subtitle: &[String]) -> Option<&'static str> {
    let s = &cat.sessions;
    let lists: [(&'static str, &[Phrase]); 5] = [
        ("shows", &s.shows),
        ("qualifying", &s.qualifying),
        ("practice", &s.practice),
        ("race", &s.race),
        ("", &s.neutral),
    ];
    let neutral = lists.len() - 1;
    // (order, field, start, end)
    let mut occ: Vec<(usize, usize, usize, usize)> = Vec::new();
    for (field_idx, field) in [title, subtitle].into_iter().enumerate() {
        for (order, (_, terms)) in lists.iter().enumerate() {
            for t in terms.iter() {
                let n = t.words.len();
                occ.extend(positions(field, &t.words).map(|st| (order, field_idx, st, st + n)));
            }
        }
    }
    occ.iter()
        .filter(|o| {
            !occ.iter()
                .any(|p| p.1 == o.1 && p.2 <= o.2 && o.3 <= p.3 && (p.3 - p.2) > (o.3 - o.2))
        })
        .map(|o| o.0)
        .filter(|&i| i != neutral)
        .min()
        .map(|i| lists[i].0)
}

/// Replay words in the title or subtitle, ignoring any that sit inside one of this league's own
/// tokens or event aliases ("Winter Classic" is a live event, not a classic).
fn replay_for(cat: &Catalog, league: &League, h: &Hay, f: &Facts) -> bool {
    if f.from_year {
        return true;
    }
    [&h.title, &h.subtitle].into_iter().any(|field| {
        let covers: Vec<(usize, usize)> = league
            .events
            .iter()
            .chain(league.tokens.iter())
            .flat_map(|p| {
                let n = p.words.len();
                positions(field, &p.words).map(move |st| (st, st + n))
            })
            .collect();
        cat.words.replay.iter().any(|r| {
            let n = r.words.len();
            positions(field, &r.words)
                .any(|st| !covers.iter().any(|&(a, b)| a <= st && st + n <= b))
        })
    })
}

fn facts(cat: &Catalog, h: &Hay) -> Facts {
    let w = &cat.words;
    Facts {
        from_year: from_year(&h.title) || from_year(&h.subtitle),
        talk: any(&h.title, &w.talk),
        session: classify_session(cat, &h.title, &h.subtitle),
        sports_group: any(&h.group, &w.sports_groups),
        category: h
            .categories
            .iter()
            .any(|c| w.categories.iter().any(|p| p.words == *c)),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    Abbr,
    Weak,
    Strong,
}

struct Occ {
    start: usize,
    end: usize,
    team: usize,
    level: Level,
}

fn occurrences(league: &League, field: &[String]) -> Vec<Occ> {
    let mut out = Vec::new();
    for (i, t) in league.teams.iter().enumerate() {
        let tagged = t
            .strong
            .iter()
            .map(|p| (p, Level::Strong))
            .chain(t.weak.iter().map(|p| (p, Level::Weak)))
            .chain(std::iter::once((&t.abbr_phrase, Level::Abbr)));
        for (p, level) in tagged {
            for start in positions(field, &p.words) {
                out.push(Occ {
                    start,
                    end: start + p.words.len(),
                    team: i,
                    level,
                });
            }
        }
    }
    out
}

/// A versus pattern (`A at B`, `A vs B`, `A v B`, `A @ B`) between two different teams whose
/// aliases are allowed to count.
fn versus(league: &League, field: &[String], with_context: bool) -> Option<(usize, usize)> {
    let occ = occurrences(league, field);
    let ok = |o: &Occ| o.level == Level::Strong || with_context;
    for a in occ.iter().filter(|o| ok(o)) {
        let Some(conn) = field.get(a.end) else {
            continue;
        };
        if !CONNECTORS.contains(&conn.as_str()) {
            continue;
        }
        if let Some(b) = occ
            .iter()
            .find(|b| b.start == a.end + 1 && b.team != a.team && ok(b))
        {
            return Some((a.team, b.team));
        }
    }
    None
}

/// Best alias level per team in the given fields (abbreviations never count here).
fn team_levels(league: &League, fields: &[&[String]]) -> Vec<Option<Level>> {
    league
        .teams
        .iter()
        .map(|t| {
            if t.strong.iter().any(|p| fields.iter().any(|f| has(f, p))) {
                Some(Level::Strong)
            } else if t.weak.iter().any(|p| fields.iter().any(|f| has(f, p))) {
                Some(Level::Weak)
            } else {
                None
            }
        })
        .collect()
}

struct LeagueScore {
    score: i64,
    teams: Vec<String>,
    /// Some league evidence beyond generic signals (token, event or a team).
    evidence: bool,
    /// That evidence is in the title or subtitle itself (league token, event or team), not only
    /// the channel name or description.
    own: bool,
    event: bool,
    replay: bool,
}

fn score_league(
    cat: &Catalog,
    league: &League,
    h: &Hay,
    f: &Facts,
    flags_bits: i64,
) -> LeagueScore {
    let others: [&[String]; 3] = [&h.subtitle, &h.description, &h.channel];
    let title_tok = any(&h.title, &league.tokens);
    let other_tok = others.iter().any(|o| any(o, &league.tokens));
    let event = std::iter::once(&h.title[..])
        .chain(others)
        .any(|o| any(o, &league.events));
    // Risky context tokens ("Cup", "Indy", "GT") need this league's own name or an event
    // somewhere. A generic sports group or category is not enough: "UCI World Cup" and "Davis
    // Cup" air on sports channels too.
    let has_context = title_tok || other_tok || event;
    let own_text: [&[String]; 2] = [&h.title, &h.subtitle];
    let own_league = own_text.iter().any(|o| {
        any(o, &league.tokens) || any(o, &league.events) || (has_context && any(o, &league.context))
    });
    let title_ctx = has_context && any(&h.title, &league.context);
    let other_ctx = has_context && others.iter().any(|o| any(o, &league.context));

    let mut league_score = 0;
    if title_tok || title_ctx {
        league_score += TOKEN_TITLE;
    }
    if other_tok || other_ctx {
        league_score += TOKEN_OTHER;
    }
    if event {
        league_score += EVENT;
    }

    let mut score = league_score;
    let mut teams = Vec::new();
    let mut team_evidence = false;
    let mut own_team = false;
    if league.kind == Kind::Team && !league.teams.is_empty() {
        let with_context = league_score >= CONTEXT;
        // Title and subtitle first; the description only when those name no team.
        let mut levels = team_levels(league, &[&h.title, &h.subtitle]);
        if levels.iter().all(Option::is_none) {
            levels = team_levels(league, &[&h.description]);
        }
        let vs = versus(league, &h.title, with_context)
            .or_else(|| versus(league, &h.subtitle, with_context));
        if let Some((a, b)) = vs {
            for i in [a, b] {
                if let Some(slot) = levels.get_mut(i)
                    && slot.is_none()
                {
                    *slot = Some(Level::Abbr);
                }
            }
        }
        let strong: Vec<usize> = (0..levels.len())
            .filter(|&i| levels[i] == Some(Level::Strong))
            .collect();
        let weak: Vec<usize> = if with_context {
            (0..levels.len())
                .filter(|&i| matches!(levels[i], Some(Level::Weak | Level::Abbr)))
                .collect()
        } else {
            Vec::new()
        };
        let n_strong = strong.len().min(MAX_TEAMS_SCORED);
        let n_weak = weak.len().min(MAX_TEAMS_SCORED - n_strong);
        score += TEAM_STRONG * n_strong as i64 + TEAM_WEAK * n_weak as i64;
        if vs.is_some() {
            score += VERSUS;
        }
        team_evidence = !strong.is_empty() || !weak.is_empty();
        // A team named in the title/subtitle (not just the description or channel name).
        let own_levels = team_levels(league, &own_text);
        own_team = vs.is_some()
            || own_levels.iter().enumerate().any(|(i, l)| match l {
                Some(Level::Strong) => true,
                Some(Level::Weak | Level::Abbr) => with_context && weak.contains(&i),
                None => false,
            });
        let matched: HashSet<usize> = strong.into_iter().chain(weak).collect();
        teams = league
            .teams
            .iter()
            .enumerate()
            .filter(|(i, _)| matched.contains(i))
            .map(|(_, t)| t.key.clone())
            .collect();
    }

    if f.category {
        score += CATEGORY;
    }
    if league.kind == Kind::Motorsport && league_score > 0 && f.session.is_some() {
        score += SESSION;
    }
    if f.sports_group {
        score += SPORTS_GROUP;
    }
    if flags_bits & flags::LIVE != 0 {
        score += LIVE_FLAG;
    }
    if flags_bits & (flags::NEW | flags::PREMIERE) != 0 {
        score += NEW_FLAG;
    }
    if any(&h.title, &league.exclude) || any(&h.subtitle, &league.exclude) {
        score += EXCLUDE;
    }
    let replay = replay_for(cat, league, h, f);
    if replay {
        score += REPLAY;
    }
    // A show is a session users can follow for motorsport, so its talk words do not count
    // against it there.
    let followable_show = league.kind == Kind::Motorsport && f.session == Some("shows");
    if f.talk && !followable_show {
        score += TALK;
    }
    LeagueScore {
        score,
        teams,
        evidence: league_score > 0 || team_evidence,
        own: own_league || own_team,
        event,
        replay,
    }
}

/// How live is decided for one candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveInfo {
    /// The source marks live airings (LIVE flag or a title marker): `marked` is this one's mark.
    Trusted { marked: bool },
    /// The source never marks live airings: live unless it looks like a replay ("likely live").
    Inferred,
}

fn score_all(
    cat: &Catalog,
    h: &Hay,
    origin: Origin,
    flags_bits: i64,
    live: LiveInfo,
) -> Vec<Match> {
    // Placeholder listings ("No Game Today", "NO EVENT") are never an event.
    if any(&h.title, &cat.words.filler) || any(&h.subtitle, &cat.words.filler) {
        return Vec::new();
    }
    let f = facts(cat, h);
    let mut out: Vec<Match> = Vec::new();
    for league in &cat.leagues {
        let s = score_league(cat, league, h, &f, flags_bits);
        if !s.evidence || s.score < THRESHOLD {
            continue;
        }
        // The listing itself must name the league, an event or a team; a sports channel's name
        // or a description alone does not make its filler a match.
        if origin == Origin::Programme && !s.own {
            continue;
        }
        let (live, live_inferred) = match live {
            LiveInfo::Trusted { marked } => (marked && !s.replay, false),
            LiveInfo::Inferred => (!s.replay, true),
        };
        let session = match league.kind {
            Kind::Team => {
                // An event channel names the game; "NFL Network" alone is not an event.
                if origin == Origin::Channel && s.teams.is_empty() {
                    continue;
                }
                None
            }
            Kind::Motorsport => match f.session {
                Some(x) => Some(x),
                None if origin == Origin::Channel && !s.event => continue,
                // A series match with no session term is the race only when the guide itself
                // marks it live; an inferred "likely live" bare title is not enough.
                None if live && !live_inferred => Some("race"),
                None => continue,
            },
        };
        out.push(Match {
            league: league.key.clone(),
            teams: s.teams,
            session,
            live,
            live_inferred,
            replay: s.replay,
            score: s.score,
        });
    }
    // Ties across leagues: the higher score wins; an exact tie keeps both.
    if let Some(best) = out.iter().map(|m| m.score).max() {
        out.retain(|m| m.score == best);
    }
    out
}

/// Every league a guide programme matches (before follow filtering). `live_trusted`: the
/// programme's source marks live airings (LIVE flags or title markers) in this window, so an
/// unmarked airing is a replay.
pub fn score_programme(cat: &Catalog, c: &Candidate<'_>, live_trusted: bool) -> Vec<Match> {
    let h = Hay {
        title: normalize(c.title),
        subtitle: c.subtitle.map(normalize).unwrap_or_default(),
        description: c.description.map(normalize).unwrap_or_default(),
        channel: normalize(c.channel_name),
        group: c.group_name.map(normalize).unwrap_or_default(),
        categories: c.categories.iter().map(|s| normalize(s)).collect(),
    };
    let marked = c.flags & flags::LIVE != 0 || live_marker(c.title, c.subtitle);
    let live = if live_trusted {
        LiveInfo::Trusted { marked }
    } else {
        LiveInfo::Inferred
    };
    // A title marker counts like the LIVE flag for scoring.
    let bits = if marked {
        c.flags | flags::LIVE
    } else {
        c.flags
    };
    score_all(cat, &h, Origin::Programme, bits, live)
}

/// Every league an event channel's name matches: the name is scored as a title.
pub fn score_channel(cat: &Catalog, channel_name: &str, group_name: Option<&str>) -> Vec<Match> {
    let title = normalize(channel_name);
    // On-board cameras, timing and tracker feeds ride along with the main event; never a card.
    if any(&title, &cat.words.feeds) {
        return Vec::new();
    }
    let h = Hay {
        title,
        subtitle: Vec::new(),
        description: Vec::new(),
        channel: Vec::new(),
        group: group_name.map(normalize).unwrap_or_default(),
        categories: Vec::new(),
    };
    score_all(cat, &h, Origin::Channel, 0, LiveInfo::Inferred)
}

/// The team-league follow rule (§5): followed league, and when teams are picked at least one of
/// them matched. Motorsport: the series is followed (session filters apply when listing).
/// Unknown keys in the follow document are ignored.
pub fn followed(cat: &Catalog, follow: &SportsFollowDto, m: &Match) -> bool {
    let Some(league) = cat.league(&m.league) else {
        return false;
    };
    match league.kind {
        Kind::Motorsport => follow.series.contains_key(&league.key),
        Kind::Team => {
            let Some(lf) = follow.leagues.get(&league.key) else {
                return false;
            };
            if lf.teams.is_empty() {
                return true; // "All games"
            }
            lf.teams.iter().any(|t| m.teams.contains(t))
        }
    }
}

pub fn match_programme(
    cat: &Catalog,
    follow: &SportsFollowDto,
    c: &Candidate<'_>,
    live_trusted: bool,
) -> Vec<Match> {
    let mut v = score_programme(cat, c, live_trusted);
    v.retain(|m| followed(cat, follow, m));
    v
}

pub fn match_channel(
    cat: &Catalog,
    follow: &SportsFollowDto,
    channel_name: &str,
    group_name: Option<&str>,
) -> Vec<Match> {
    let mut v = score_channel(cat, channel_name, group_name);
    v.retain(|m| followed(cat, follow, m));
    v
}

static TIME_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(\d{1,2})(?::([0-5]\d))?\s*(?:([ap])\.?m\b\.?)?\s*(et|est|edt|ct|cst|cdt|mt|mst|mdt|pt|pst|pdt|gmt|bst|uk|utc|cet|cest)?\b",
    )
    .ok()
});

fn zone(abbr: &str) -> Option<Tz> {
    Some(match abbr.to_ascii_lowercase().as_str() {
        "et" | "est" | "edt" => chrono_tz::America::New_York,
        "ct" | "cst" | "cdt" => chrono_tz::America::Chicago,
        "mt" | "mst" | "mdt" => chrono_tz::America::Denver,
        "pt" | "pst" | "pdt" => chrono_tz::America::Los_Angeles,
        "gmt" | "bst" | "uk" => chrono_tz::Europe::London,
        "cet" | "cest" => chrono_tz::Europe::Paris,
        "utc" => chrono_tz::UTC,
        _ => return None,
    })
}

static DATE_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:(\d{1,2})(?:st|nd|rd|th)?\s+(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.?|(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.?\s+(\d{1,2})(?:st|nd|rd|th)?)\b",
    )
    .ok()
});

fn month_num(m: &str) -> Option<u32> {
    let i = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ]
    .iter()
    .position(|x| m.eq_ignore_ascii_case(x))?;
    u32::try_from(i + 1).ok()
}

/// A day + month in the name ("27 Nov", "Nov 27th"), as the date nearest to `today`.
fn channel_date(name: &str, today: chrono::NaiveDate) -> Option<chrono::NaiveDate> {
    use chrono::Datelike;
    let c = DATE_RE.as_ref()?.captures(name)?;
    let (day, month) = match (c.get(1), c.get(2), c.get(3), c.get(4)) {
        (Some(d), Some(m), _, _) | (_, _, Some(m), Some(d)) => {
            (d.as_str().parse::<u32>().ok()?, month_num(m.as_str())?)
        }
        _ => return None,
    };
    [today.year() - 1, today.year(), today.year() + 1]
        .into_iter()
        .filter_map(|y| chrono::NaiveDate::from_ymd_opt(y, month, day))
        .min_by_key(|d| (*d - today).num_days().abs())
}

/// Best-effort start time from an event channel name (`... 1:00 PM ET`, `... @ 27 Nov 11:50 AM
/// ET`, `... 19:30`), in the named zone or else `source_tz`. With a date in the name that date
/// is used; without one, the next occurrence of the time (more than six hours past = tomorrow).
pub fn parse_channel_time(name: &str, source_tz: Tz, now: i64) -> Option<i64> {
    let re = TIME_RE.as_ref()?;
    for c in re.captures_iter(name) {
        let Some(hour) = c.get(1).and_then(|m| m.as_str().parse::<u32>().ok()) else {
            continue;
        };
        let minute = c.get(2).and_then(|m| m.as_str().parse::<u32>().ok());
        let meridiem = c.get(3).map(|m| m.as_str().to_ascii_lowercase());
        if minute.is_none() && meridiem.is_none() {
            continue; // a bare number ("NFL 03") is not a time
        }
        let hour = match meridiem.as_deref() {
            Some(m) => {
                if !(1..=12).contains(&hour) {
                    continue;
                }
                (hour % 12) + if m == "p" { 12 } else { 0 }
            }
            None if hour <= 23 => hour,
            None => continue,
        };
        let tz = c.get(4).and_then(|m| zone(m.as_str())).unwrap_or(source_tz);
        let time = NaiveTime::from_hms_opt(hour, minute.unwrap_or(0), 0)?;
        let today = tz.timestamp_opt(now, 0).single()?.date_naive();
        if let Some(date) = channel_date(name, today) {
            return tz
                .from_local_datetime(&date.and_time(time))
                .earliest()
                .map(|t| t.timestamp());
        }
        for day in 0..2 {
            let date = today + Duration::days(day);
            let Some(t) = tz.from_local_datetime(&date.and_time(time)).earliest() else {
                continue;
            };
            let t = t.timestamp();
            if t >= now - 6 * 3600 {
                return Some(t);
            }
        }
        return None;
    }
    None
}

#[cfg(test)]
pub(crate) mod tests;
