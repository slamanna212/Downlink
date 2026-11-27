//! Fixture tests for every scoring rule in docs/SPORTS.md §5, using invented guide titles in
//! real formats and a small inline catalog (never the embedded file).

use std::collections::BTreeMap;
use std::sync::LazyLock;

use super::*;
use crate::service::dto::{SportsFollowDto, SportsLeagueFollowDto, SportsSeriesFollowDto};
use crate::sports::catalog::{Catalog, parse};

pub(crate) const FIXTURE: &str = r#"
[sessions]
shows = ["Pre-race", "Pre-Race Show", "Countdown", "Grid Walk", "Post-race", "Race Hub", "Highlights", "Preview", "Raceday", "Weekly"]
qualifying = ["Qualifying", "Quali", "Q1", "Q2", "Q3", "Sprint Qualifying", "Sprint Shootout", "Pole", "Pole Day", "Time Trials", "Hyperpole", "Superpole", "Top 10 Shootout", "Last Chance Qualifier", "LCQ"]
practice = ["Practice", "Free Practice", "FP1", "FP2", "FP3", "P1", "P2", "P3", "Warm-up", "Happy Hour", "Carb Day", "Shakedown", "Test", "Testing"]
race = ["Race", "Grand Prix", "GP", "Sprint", "Sprint Race", "Feature", "Feature Race", "Main Event", "Heat", "Heat Race", "24 Hours", "6 Hours", "12 Hours", "Hours of", "Duel", "Final", "500", "400", "Superpole Race"]

[words]
replay = ["Classic", "Replay", "Encore", "Rewind", "Greatest Games"]
talk = ["Live from", "Tonight", "Countdown", "Postgame", "Pregame", "Insider", "Report", "Draft"]
sports_groups = ["Sports", "Racing", "Motorsport", "NFL"]
categories = ["Sports", "Sports event", "Football", "Motorsport"]

[[league]]
key = "nfl"
name = "NFL"
kind = "team"
tokens = ["NFL", "NFL Football", "National Football League"]
exclude = ["NFL Live", "Inside the NFL", "NFL Total Access", "Good Morning Football"]

  [[league.team]]
  key = "chi"
  name = "Chicago Bears"
  short = "Bears"
  abbr = "CHI"
  city = "Chicago"

  [[league.team]]
  key = "gb"
  name = "Green Bay Packers"
  short = "Packers"
  abbr = "GB"
  city = "Green Bay"

  [[league.team]]
  key = "nyg"
  name = "New York Giants"
  short = "Giants"
  abbr = "NYG"
  city = "New York"

  [[league.team]]
  key = "ari"
  name = "Arizona Cardinals"
  short = "Cardinals"
  abbr = "ARI"
  city = "Arizona"

  [[league.team]]
  key = "sf"
  name = "San Francisco 49ers"
  short = "49ers"
  abbr = "SF"
  city = "San Francisco"
  aliases = ["Niners"]

[[league]]
key = "mlb"
name = "MLB"
kind = "team"
tokens = ["MLB", "MLB Baseball", "Major League Baseball"]
exclude = ["MLB Tonight"]

  [[league.team]]
  key = "sf"
  name = "San Francisco Giants"
  short = "Giants"
  abbr = "SF"
  city = "San Francisco"

  [[league.team]]
  key = "stl"
  name = "St. Louis Cardinals"
  short = "Cardinals"
  abbr = "STL"
  city = "St. Louis"

  [[league.team]]
  key = "tex"
  name = "Texas Rangers"
  short = "Rangers"
  abbr = "TEX"
  city = "Texas"

  [[league.team]]
  key = "hou"
  name = "Houston Astros"
  short = "Astros"
  abbr = "HOU"
  city = "Houston"

[[league]]
key = "nhl"
name = "NHL"
kind = "team"
tokens = ["NHL", "NHL Hockey"]
events = ["Winter Classic"]

  [[league.team]]
  key = "nyr"
  name = "New York Rangers"
  short = "Rangers"
  abbr = "NYR"
  city = "New York"

  [[league.team]]
  key = "lak"
  name = "Los Angeles Kings"
  short = "Kings"
  abbr = "LAK"
  city = "Los Angeles"

[[league]]
key = "nba"
name = "NBA"
kind = "team"
tokens = ["NBA", "NBA Basketball"]

  [[league.team]]
  key = "sac"
  name = "Sacramento Kings"
  short = "Kings"
  abbr = "SAC"
  city = "Sacramento"

  [[league.team]]
  key = "bos"
  name = "Boston Celtics"
  short = "Celtics"
  abbr = "BOS"
  city = "Boston"

[[league]]
key = "f1"
name = "Formula 1"
kind = "motorsport"
group = "open_wheel"
tokens = ["Formula 1", "Formula One"]
context = ["F1"]
events = ["Monaco Grand Prix"]
exclude = ["F1 Academy", "F1 TV"]

[[league]]
key = "f1_academy"
name = "F1 Academy"
kind = "motorsport"
group = "open_wheel"
tokens = ["F1 Academy"]

[[league]]
key = "nascar_cup"
name = "NASCAR Cup Series"
kind = "motorsport"
group = "stock_car"
tokens = ["NASCAR Cup Series", "NASCAR Cup", "NASCAR"]
context = ["Cup"]
events = ["Daytona 500", "Coca-Cola 600"]
exclude = ["Xfinity", "O'Reilly", "Truck Series"]

[[league]]
key = "nascar_oreilly"
name = "NASCAR O'Reilly Auto Parts Series"
kind = "motorsport"
group = "stock_car"
tokens = ["O'Reilly Auto Parts Series", "Xfinity Series", "NASCAR Xfinity"]

[[league]]
key = "motogp"
name = "MotoGP"
kind = "motorsport"
group = "motorcycle"
tokens = ["MotoGP"]
exclude = ["Moto2", "Moto3"]

[[league]]
key = "epl"
name = "Premier League"
kind = "team"
tokens = ["Premier League"]

  [[league.team]]
  key = "liv"
  name = "Liverpool"
  short = "Liverpool"
  abbr = "LIV"
  city = "Liverpool"

  [[league.team]]
  key = "che"
  name = "Chelsea"
  short = "Chelsea"
  abbr = "CHE"
  city = "London"

[[league]]
key = "moto2"
name = "Moto2"
kind = "motorsport"
group = "motorcycle"
tokens = ["Moto2"]
"#;

static CAT: LazyLock<Catalog> = LazyLock::new(|| parse(FIXTURE).unwrap());

fn cat() -> &'static Catalog {
    &CAT
}

fn follow(leagues: &[(&str, &[&str])], series: &[(&str, &[&str])]) -> SportsFollowDto {
    SportsFollowDto {
        version: 1,
        include_replays: false,
        leagues: leagues
            .iter()
            .map(|(k, t)| {
                (
                    (*k).to_owned(),
                    SportsLeagueFollowDto {
                        teams: t.iter().map(|s| (*s).to_owned()).collect(),
                    },
                )
            })
            .collect::<BTreeMap<_, _>>(),
        series: series
            .iter()
            .map(|(k, s)| {
                (
                    (*k).to_owned(),
                    SportsSeriesFollowDto {
                        sessions: s.iter().map(|x| (*x).to_owned()).collect(),
                    },
                )
            })
            .collect(),
    }
}

fn prog(title: &str) -> Candidate<'_> {
    Candidate {
        title,
        channel_name: "Channel 5",
        ..Default::default()
    }
}

fn leagues(v: &[Match]) -> Vec<&str> {
    v.iter().map(|m| m.league.as_str()).collect()
}

fn one(title: &str, flags: i64, trusted: bool) -> Option<Match> {
    let c = Candidate {
        flags,
        ..prog(title)
    };
    let v = score_programme(cat(), &c, trusted);
    assert!(v.len() <= 1, "{title}: {v:?}");
    v.into_iter().next()
}

const LIVE: i64 = flags::LIVE;

#[test]
fn normalize_folds_case_diacritics_ampersand_and_punctuation() {
    assert_eq!(
        normalize("CF Montréal & Brighton—Hove: vs. @Home!"),
        [
            "cf", "montreal", "and", "brighton", "hove", "vs", "@", "home"
        ]
    );
}

#[test]
fn word_boundaries_keep_f1_out_of_f150() {
    let c = Candidate {
        group_name: Some("Sports"),
        flags: LIVE,
        ..prog("F150 Truck Show: Live Race Build")
    };
    assert!(score_programme(cat(), &c, true).is_empty());
    // The same channel with the real series token does match.
    let c = Candidate {
        group_name: Some("Sports"),
        flags: LIVE,
        ..prog("F1: Monaco Practice")
    };
    let v = score_programme(cat(), &c, true);
    assert_eq!(leagues(&v), ["f1"]);
    assert_eq!(v[0].session, Some("practice"));
}

#[test]
fn context_token_needs_league_context() {
    // "F1" alone, no group, category or other token: not enough.
    assert!(one("F1 Grand Prix", LIVE, true).is_none());
}

#[test]
fn versus_between_strong_aliases_matches_without_a_league_token() {
    let m = one("Chicago Bears at Green Bay Packers", 0, true).unwrap();
    assert_eq!(m.league, "nfl");
    assert_eq!(m.teams, ["chi", "gb"]);
    assert_eq!(m.score, 2 * 30 + 20);
    let m = one("Bears vs. Packers", 0, true).unwrap();
    assert_eq!(m.teams, ["chi", "gb"]);
}

#[test]
fn abbreviations_count_only_inside_a_versus_pattern_with_context() {
    let m = one("NFL Football: CHI @ GB", LIVE, true).unwrap();
    assert_eq!(m.teams, ["chi", "gb"]);
    assert_eq!(m.score, 40 + 10 + 10 + 20 + 10);
    // An abbreviation outside a versus pattern is ignored.
    let m = one("NFL Football: CHI preview", 0, true);
    assert!(m.as_ref().is_none_or(|m| m.teams.is_empty()), "{m:?}");
}

#[test]
fn league_only_title_needs_follow_all() {
    let c = Candidate {
        flags: LIVE,
        ..prog("NFL Football")
    };
    let all = follow(&[("nfl", &[])], &[]);
    let picked = follow(&[("nfl", &["chi"])], &[]);
    let v = match_programme(cat(), &all, &c, true);
    assert_eq!(leagues(&v), ["nfl"]);
    assert!(v[0].teams.is_empty());
    assert!(match_programme(cat(), &picked, &c, true).is_empty());
}

#[test]
fn picked_team_must_match() {
    let c = Candidate {
        subtitle: Some("Chicago Bears at Green Bay Packers"),
        flags: LIVE,
        ..prog("NFL Football")
    };
    assert_eq!(
        match_programme(cat(), &follow(&[("nfl", &["gb"])], &[]), &c, true).len(),
        1
    );
    assert!(match_programme(cat(), &follow(&[("nfl", &["nyg"])], &[]), &c, true).is_empty());
    // Unknown team keys are ignored, not widened to "All games".
    assert!(match_programme(cat(), &follow(&[("nfl", &["gone"])], &[]), &c, true).is_empty());
}

#[test]
fn shared_nickname_giants_needs_league_context() {
    // "Giants at Cardinals": both nicknames are shared by NFL and MLB, so they are weak.
    assert!(one("Giants at Cardinals", LIVE, true).is_none());
    let m = one("NFL Football: Giants at Cardinals", LIVE, true).unwrap();
    assert_eq!(m.league, "nfl");
    assert_eq!(m.teams, ["nyg", "ari"]);
    let m = one("MLB Baseball: Giants at Cardinals", LIVE, true).unwrap();
    assert_eq!(m.league, "mlb");
    assert_eq!(m.teams, ["sf", "stl"]);
}

#[test]
fn shared_nickname_cardinals_full_name_is_strong() {
    let m = one("Arizona Cardinals at San Francisco 49ers", 0, true).unwrap();
    assert_eq!(m.league, "nfl");
    assert_eq!(m.teams, ["ari", "sf"]);
    let m = one("St. Louis Cardinals vs. San Francisco Giants", 0, true).unwrap();
    assert_eq!(m.league, "mlb");
    assert_eq!(m.teams, ["sf", "stl"]);
}

#[test]
fn shared_nickname_rangers_resolves_by_context() {
    assert!(one("Rangers at Kings", LIVE, true).is_none());
    let m = one("NHL Hockey: Rangers at Kings", LIVE, true).unwrap();
    assert_eq!(m.league, "nhl");
    assert_eq!(m.teams, ["nyr", "lak"]);
    let m = one("Texas Rangers at Houston Astros", 0, true).unwrap();
    assert_eq!(m.league, "mlb");
    assert_eq!(m.teams, ["tex", "hou"]);
}

#[test]
fn shared_nickname_kings_resolves_by_context() {
    let m = one("NBA Basketball: Celtics at Kings", LIVE, true).unwrap();
    assert_eq!(m.league, "nba");
    assert_eq!(m.teams, ["sac", "bos"]);
    // League + weak city, but a talk show.
    let m = one("NBA Basketball: Live From Sacramento", LIVE, true);
    assert!(m.is_none(), "{m:?}");
}

#[test]
fn exclude_phrase_vetoes_the_league() {
    assert!(one("NFL Live", LIVE, true).is_none());
    assert!(one("Inside the NFL", LIVE, true).is_none());
}

#[test]
fn replay_words_penalize_and_mark_replay() {
    let m = one("NFL Football: Chicago Bears at Green Bay Packers", 0, true).unwrap();
    assert!(!m.replay);
    assert!(!m.live, "a source with flags and no LIVE flag is a replay");
    assert_eq!(m.score, 40 + 60 + 20);
    let r = one("NFL Classic: Chicago Bears at Green Bay Packers", 0, true).unwrap();
    assert!(r.replay && !r.live);
    assert_eq!(r.score, 40 + 60 + 20 - 40);
    let r = one("Chicago Bears at Green Bay Packers from 1985", LIVE, true).unwrap();
    assert!(r.replay && !r.live);
    assert!(one("NFL Replay", LIVE, true).is_none());
}

#[test]
fn talk_words_reject_shows() {
    assert!(one("NFL Pregame", LIVE, true).is_none());
    assert!(one("NFL Draft Report", LIVE, true).is_none());
    assert!(one("MLB Tonight", LIVE, true).is_none());
}

#[test]
fn live_flag_trusted_vs_inferred() {
    let title = "NFL Football: Chicago Bears at Green Bay Packers";
    let m = one(title, LIVE, true).unwrap();
    assert!(m.live && !m.live_inferred);
    // A guide that never flags LIVE: live unless replay words, marked inferred.
    let m = one(title, 0, false).unwrap();
    assert!(m.live && m.live_inferred);
    let m = one("NFL Classic: Chicago Bears at Green Bay Packers", 0, false).unwrap();
    assert!(!m.live && m.live_inferred && m.replay);
}

#[test]
fn category_and_sports_group_add_points() {
    let cats = ["Sports event".to_owned()];
    let c = Candidate {
        categories: &cats,
        group_name: Some("US | Sports"),
        flags: LIVE | flags::NEW,
        ..prog("NFL Football")
    };
    let v = score_programme(cat(), &c, true);
    assert_eq!(v[0].score, 40 + 15 + 10 + 10 + 5);
}

#[test]
fn channel_name_token_counts_as_other_field() {
    let c = Candidate {
        channel_name: "NFL Network",
        ..prog("Chicago Bears at Green Bay Packers")
    };
    let v = score_programme(cat(), &c, true);
    assert_eq!(v[0].score, 25 + 60 + 20);
}

fn session_of(title: &str) -> Option<&'static str> {
    let v = score_programme(
        cat(),
        &Candidate {
            flags: LIVE,
            ..prog(title)
        },
        true,
    );
    assert_eq!(leagues(&v), ["f1"], "{title}");
    v[0].session
}

#[test]
fn every_motorsport_session_term_classifies() {
    let c = cat();
    for (name, terms) in [
        ("shows", &c.sessions.shows),
        ("qualifying", &c.sessions.qualifying),
        ("practice", &c.sessions.practice),
        ("race", &c.sessions.race),
    ] {
        for t in terms.iter() {
            let title = format!("Formula 1: {}", t.raw);
            assert_eq!(session_of(&title), Some(name), "{title}");
        }
    }
}

#[test]
fn session_order_puts_sprint_qualifying_and_pre_race_first() {
    assert_eq!(
        session_of("Formula 1: Sprint Qualifying"),
        Some("qualifying")
    );
    assert_eq!(session_of("Formula 1: Pre-race"), Some("shows"));
    assert_eq!(session_of("Formula 1 Grand Prix Countdown"), Some("shows"));
    assert_eq!(session_of("Formula 1: Sprint"), Some("race"));
}

#[test]
fn live_series_title_without_session_counts_as_race() {
    let m = one("Formula 1: Las Vegas", LIVE, true).unwrap();
    assert_eq!(m.session, Some("race"));
    assert!(one("Formula 1: Las Vegas", 0, true).is_none());
}

#[test]
fn marquee_event_is_strong() {
    let m = one("Monaco Grand Prix", LIVE, true).unwrap();
    assert_eq!(m.league, "f1");
    assert_eq!(m.session, Some("race"));
    let m = one("Daytona 500", LIVE, true).unwrap();
    assert_eq!(m.league, "nascar_cup");
}

#[test]
fn exclude_hands_off_to_the_more_specific_series() {
    let v = score_programme(
        cat(),
        &Candidate {
            flags: LIVE,
            ..prog("F1 Academy: Race 1")
        },
        true,
    );
    assert_eq!(leagues(&v), ["f1_academy"]);
    let v = score_programme(
        cat(),
        &Candidate {
            flags: LIVE,
            ..prog("NASCAR Xfinity Series: Race at the Speedway")
        },
        true,
    );
    assert_eq!(leagues(&v), ["nascar_oreilly"]);
    let v = score_programme(
        cat(),
        &Candidate {
            flags: LIVE,
            ..prog("NASCAR O'Reilly Auto Parts Series Qualifying")
        },
        true,
    );
    assert_eq!(leagues(&v), ["nascar_oreilly"]);
    assert_eq!(v[0].session, Some("qualifying"));
    let v = score_programme(
        cat(),
        &Candidate {
            flags: LIVE,
            ..prog("Moto2: Grand Prix of Spain")
        },
        true,
    );
    assert_eq!(leagues(&v), ["moto2"]);
    // Followers of the parent series get nothing from the handed-off one.
    let f = follow(&[], &[("f1", &["race"]), ("motogp", &["race"])]);
    assert!(
        match_programme(
            cat(),
            &f,
            &Candidate {
                flags: LIVE,
                ..prog("F1 Academy: Race 1")
            },
            true
        )
        .is_empty()
    );
}

#[test]
fn motorsport_show_is_not_penalized_as_talk() {
    let m = one("NASCAR RaceDay Countdown", LIVE, true);
    // "Raceday"/"Countdown" are show terms; still the series' show.
    let m = m.unwrap();
    assert_eq!(m.league, "nascar_cup");
    assert_eq!(m.session, Some("shows"));
}

#[test]
fn ties_keep_both_and_higher_score_wins() {
    // "Kings" on a generic sports channel with both league tokens: exact tie keeps both.
    let v = score_programme(cat(), &prog("NBA and NHL Tonight Kings Special"), true);
    assert!(v.is_empty(), "talk show: {v:?}");
    let v = score_programme(
        cat(),
        &Candidate {
            flags: LIVE,
            ..prog("NBA Basketball / NHL Hockey Doubleheader")
        },
        true,
    );
    assert_eq!(leagues(&v), ["nhl", "nba"]);
}

#[test]
fn event_channel_names_match() {
    let f = follow(&[("nfl", &["chi"])], &[("f1", &[])]);
    let v = match_channel(
        cat(),
        &f,
        "NFL 03: Bears vs Packers 1:00 PM ET",
        Some("NFL Game Pass"),
    );
    assert_eq!(leagues(&v), ["nfl"]);
    assert_eq!(v[0].teams, ["chi", "gb"]);
    assert!(v[0].live && v[0].live_inferred);
    // A network channel is not an event.
    assert!(match_channel(cat(), &follow(&[("nfl", &[])], &[]), "NFL Network", None).is_empty());
    assert!(match_channel(cat(), &f, "NFL 04:", Some("NFL Game Pass")).is_empty());
    // Motorsport event channels need a session or event.
    let v = match_channel(cat(), &f, "Formula 1 | Qualifying", Some("Sports"));
    assert_eq!(v[0].session, Some("qualifying"));
    assert!(match_channel(cat(), &f, "Formula 1 TV", Some("Sports")).is_empty());
}

#[test]
fn channel_time_parsing_is_best_effort() {
    // 2026-11-27 12:00:00 UTC
    let now = 1_795_780_800;
    let ny = chrono_tz::America::New_York;
    // 1:00 PM ET = 18:00 UTC.
    assert_eq!(
        parse_channel_time("NFL 03: Bears vs Packers 1:00 PM ET", chrono_tz::UTC, now),
        Some(now + 6 * 3600)
    );
    assert_eq!(
        parse_channel_time("NFL 03: Bears vs Packers 1pm", ny, now),
        Some(now + 6 * 3600)
    );
    assert_eq!(
        parse_channel_time("EPL 2 | Arsenal v Chelsea 19:30", chrono_tz::UTC, now),
        Some(now + 7 * 3600 + 1800)
    );
    // A bare number is a feed number, not a time.
    assert_eq!(parse_channel_time("NFL 03: Bears", ny, now), None);
    // Long past today means tomorrow.
    assert_eq!(
        parse_channel_time("Race 02:00", chrono_tz::UTC, now),
        Some(now + 14 * 3600)
    );
}

#[test]
fn event_alias_containing_a_replay_word_is_not_a_replay() {
    let m = one("NHL Winter Classic: Rangers at Kings", LIVE, true).unwrap();
    assert_eq!(m.league, "nhl");
    assert!(!m.replay && m.live);
    assert_eq!(m.score, 40 + 40 + 10 + 10 + 20 + 10);
    // A replay word outside the event still counts.
    let m = one("NHL Winter Classic Rewind: Rangers at Kings", LIVE, true).unwrap();
    assert!(m.replay && !m.live);
}

#[test]
fn longer_session_term_beats_a_term_inside_it() {
    assert_eq!(session_of("Formula 1: Superpole Race"), Some("race"));
    assert_eq!(session_of("Formula 1: Superpole"), Some("qualifying"));
    assert_eq!(session_of("Formula 1: Pre-Race Show"), Some("shows"));
}

#[test]
fn short_name_equal_to_city_stays_strong() {
    let liv = &cat().league("epl").unwrap().teams[0];
    assert!(liv.strong.iter().any(|p| p.words == ["liverpool"]));
    assert!(liv.weak.is_empty());
    let m = one("Liverpool v Chelsea", 0, true).unwrap();
    assert_eq!(m.league, "epl");
    assert_eq!(m.teams, ["liv", "che"]);
}

#[test]
fn channel_time_uses_a_date_in_the_name() {
    use chrono::TimeZone;
    let ny = chrono_tz::America::New_York;
    // 2026-11-27 21:00 UTC (16:00 ET).
    let now = chrono::Utc
        .with_ymd_and_hms(2026, 11, 27, 21, 0, 0)
        .unwrap()
        .timestamp();
    let at = |y, mo, d, h, mi| ny.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap().timestamp();
    // Earlier today: kept as today (the store drops it as over), not pushed to tomorrow.
    assert_eq!(
        parse_channel_time(
            "Sport+ 16: Formula 1 - Desert GP Practice #2 @ 27 Nov 11:50 AM ET",
            chrono_tz::UTC,
            now
        ),
        Some(at(2026, 11, 27, 11, 50))
    );
    assert_eq!(
        parse_channel_time(
            "NBA: Nuggets vs. Jazz @ Nov 28th 09:30 PM ET",
            chrono_tz::UTC,
            now
        ),
        Some(at(2026, 11, 28, 21, 30))
    );
    // Across the new year.
    let dec = chrono::Utc
        .with_ymd_and_hms(2026, 12, 31, 20, 0, 0)
        .unwrap()
        .timestamp();
    assert_eq!(
        parse_channel_time(
            "NHL: Kings vs. Wild @ 1 Jan 7:00 PM ET",
            chrono_tz::UTC,
            dec
        ),
        Some(at(2027, 1, 1, 19, 0))
    );
}
