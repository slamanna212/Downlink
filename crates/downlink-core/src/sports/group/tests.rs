use super::*;
use crate::service::dto::ProgrammeDto;

const NOW: i64 = 1_800_000_000;

fn channel(id: i64, number: f64, quality: Option<&'static str>, fav: bool) -> ChannelDto {
    ChannelDto {
        id,
        source_id: 1,
        uuid: None,
        number: Some(number),
        name: format!("Channel {id}"),
        group_id: None,
        logo: None,
        quality,
        is_catchup: false,
        catchup_days: 0,
        is_radio: false,
        has_drm: false,
        is_favourite: fav,
        epg_match: "matched".into(),
    }
}

fn programme(channel_id: i64, start: i64, title: &str) -> ProgrammeDto {
    ProgrammeDto {
        id: format!("{channel_id}:{start}"),
        channel_id,
        start_utc: start,
        end_utc: start + 3 * 3600,
        title: Some(title.into()),
        subtitle: None,
        description: None,
        season: None,
        episode: None,
        is_new: false,
        is_live: true,
        is_premiere: false,
        is_finale: false,
        is_scheduled: false,
    }
}

fn m(
    ch: ChannelDto,
    start: Option<i64>,
    league: &str,
    teams: &[&str],
    title: &str,
) -> SportsMatchDto {
    SportsMatchDto {
        programme: start.map(|s| programme(ch.id, s, title)),
        channel: ch,
        league: league.into(),
        league_name: league.to_uppercase(),
        teams: teams.iter().map(|t| (*t).to_owned()).collect(),
        followed_teams: vec![],
        session: None,
        live: true,
        live_inferred: false,
        source: if start.is_some() {
            "programme"
        } else {
            "channel"
        }
        .into(),
        score: 80,
        event_start_utc: None,
        alternates: vec![],
    }
}

fn motor(ch: ChannelDto, start: i64, session: &str) -> SportsMatchDto {
    let mut x = m(ch, Some(start), "f1", &[], "Formula 1");
    x.session = Some(session.into());
    x
}

#[test]
fn same_game_on_three_channels_collapses_to_the_best_one() {
    let out = group_events(
        vec![
            m(
                channel(1, 5.0, Some("SD"), false),
                Some(NOW),
                "nfl",
                &["chi", "gb"],
                "NFL Football",
            ),
            m(
                channel(2, 9.0, Some("FHD"), false),
                Some(NOW + 300),
                "nfl",
                &["gb", "chi"],
                "Bears at Packers",
            ),
            m(
                channel(3, 2.0, Some("HD"), false),
                Some(NOW),
                "nfl",
                &["chi", "gb"],
                "NFL Football",
            ),
        ],
        NOW,
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].channel.id, 2, "FHD beats HD and SD");
    let alts: Vec<i64> = out[0].alternates.iter().map(|c| c.id).collect();
    assert_eq!(alts, vec![3, 1], "alternates in preference order");
}

#[test]
fn favorite_wins_over_quality() {
    let out = group_events(
        vec![
            m(
                channel(1, 5.0, Some("4K"), false),
                Some(NOW),
                "nfl",
                &["chi", "gb"],
                "x",
            ),
            m(
                channel(2, 9.0, Some("SD"), true),
                Some(NOW),
                "nfl",
                &["chi", "gb"],
                "x",
            ),
        ],
        NOW,
    );
    assert_eq!(out[0].channel.id, 2);
}

#[test]
fn unknown_quality_ranks_between_sd_and_hd() {
    let out = group_events(
        vec![
            m(
                channel(1, 1.0, Some("SD"), false),
                Some(NOW),
                "nfl",
                &["chi"],
                "x",
            ),
            m(
                channel(2, 2.0, None, false),
                Some(NOW),
                "nfl",
                &["chi"],
                "x",
            ),
        ],
        NOW,
    );
    assert_eq!(out[0].channel.id, 2);
}

#[test]
fn different_games_and_far_apart_starts_stay_separate() {
    let out = group_events(
        vec![
            m(
                channel(1, 1.0, None, false),
                Some(NOW),
                "nfl",
                &["chi", "gb"],
                "x",
            ),
            m(
                channel(2, 2.0, None, false),
                Some(NOW),
                "nfl",
                &["kc", "buf"],
                "x",
            ),
            m(
                channel(3, 3.0, None, false),
                Some(NOW + 7 * 86400),
                "nfl",
                &["chi", "gb"],
                "x",
            ),
            m(
                channel(4, 4.0, None, false),
                Some(NOW),
                "nhl",
                &["chi"],
                "x",
            ),
        ],
        NOW,
    );
    assert_eq!(out.len(), 4);
}

#[test]
fn one_named_team_joins_a_two_team_listing() {
    let out = group_events(
        vec![
            m(
                channel(1, 1.0, None, false),
                Some(NOW),
                "nfl",
                &["chi", "gb"],
                "Bears at Packers",
            ),
            m(
                channel(2, 2.0, None, false),
                Some(NOW + 600),
                "nfl",
                &["chi"],
                "NFL Football",
            ),
        ],
        NOW,
    );
    assert_eq!(out.len(), 1);
}

#[test]
fn teamless_listings_group_only_on_equal_titles() {
    let out = group_events(
        vec![
            m(
                channel(1, 1.0, None, false),
                Some(NOW),
                "epl",
                &[],
                "Premier League: Matchday Live",
            ),
            m(
                channel(2, 2.0, None, false),
                Some(NOW),
                "epl",
                &[],
                "Premier League - Matchday LIVE",
            ),
            m(
                channel(3, 3.0, None, false),
                Some(NOW),
                "epl",
                &[],
                "Premier League Highlights Show",
            ),
        ],
        NOW,
    );
    assert_eq!(out.len(), 2);
}

#[test]
fn event_channel_joins_the_game_on_air() {
    let out = group_events(
        vec![
            m(
                channel(1, 1.0, Some("HD"), false),
                Some(NOW - 1800),
                "nfl",
                &["chi", "det"],
                "NFL Football",
            ),
            m(
                channel(2, 2.0, Some("SD"), false),
                None,
                "nfl",
                &["chi", "det"],
                "NFL 03: Bears vs Lions",
            ),
        ],
        NOW,
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].channel.id, 1, "guide data beats a name-only match");
    assert_eq!(out[0].alternates[0].id, 2);
}

#[test]
fn event_channel_does_not_join_next_weeks_game() {
    let out = group_events(
        vec![
            m(
                channel(1, 1.0, None, false),
                Some(NOW + 6 * 86400),
                "nfl",
                &["chi", "det"],
                "x",
            ),
            m(
                channel(2, 2.0, None, false),
                None,
                "nfl",
                &["chi", "det"],
                "x",
            ),
        ],
        NOW,
    );
    assert_eq!(out.len(), 2);
}

#[test]
fn motorsport_groups_by_session() {
    let out = group_events(
        vec![
            motor(channel(1, 1.0, None, false), NOW, "race"),
            motor(channel(2, 2.0, None, false), NOW + 120, "race"),
            motor(
                channel(3, 3.0, None, false),
                NOW - 86400 + 3600 * 25,
                "qualifying",
            ),
        ],
        NOW,
    );
    assert_eq!(out.len(), 2);
}

#[test]
fn same_channel_listed_twice_is_not_its_own_alternate() {
    let out = group_events(
        vec![
            m(
                channel(1, 1.0, None, false),
                Some(NOW),
                "nfl",
                &["chi", "gb"],
                "x",
            ),
            m(
                channel(1, 1.0, None, false),
                Some(NOW + 60),
                "nfl",
                &["chi", "gb"],
                "y",
            ),
        ],
        NOW,
    );
    assert_eq!(out.len(), 1);
    assert!(out[0].alternates.is_empty());
}

#[test]
fn display_order_is_live_then_start() {
    let mut replay = m(
        channel(1, 1.0, None, false),
        Some(NOW - 600),
        "nfl",
        &["kc"],
        "x",
    );
    replay.live = false;
    let later = m(
        channel(2, 2.0, None, false),
        Some(NOW + 3600),
        "nfl",
        &["chi"],
        "x",
    );
    let now = m(channel(3, 3.0, None, false), Some(NOW), "nfl", &["gb"], "x");
    let out = group_events(vec![replay, later, now], NOW);
    let ids: Vec<i64> = out.iter().map(|x| x.channel.id).collect();
    assert_eq!(ids, vec![3, 2, 1]);
}

#[test]
fn a_channel_joining_a_session_late_is_still_the_same_session() {
    // 40 minutes behind (its own build-up first) but on air at the same time.
    let out = group_events(
        vec![
            motor(channel(1, 1.0, None, false), NOW, "qualifying"),
            motor(channel(2, 2.0, None, false), NOW + 40 * 60, "qualifying"),
        ],
        NOW,
    );
    assert_eq!(out.len(), 1);
}

#[test]
fn copies_must_overlap_in_time() {
    // Fixtures run 3 hours: back-to-back airings of the same game do not overlap.
    let out = group_events(
        vec![
            m(
                channel(1, 1.0, None, false),
                Some(NOW),
                "nfl",
                &["chi", "gb"],
                "x",
            ),
            m(
                channel(2, 2.0, None, false),
                Some(NOW + 3 * 3600),
                "nfl",
                &["chi", "gb"],
                "x",
            ),
        ],
        NOW,
    );
    assert_eq!(out.len(), 2);
}

#[test]
fn event_channel_joins_the_airing_its_named_time_falls_in() {
    let mut tonight = m(
        channel(2, 2.0, None, false),
        None,
        "nfl",
        &["chi", "gb"],
        "x",
    );
    tonight.event_start_utc = Some(NOW + 5 * 3600);
    let out = group_events(
        vec![
            m(
                channel(1, 1.0, None, false),
                Some(NOW),
                "nfl",
                &["chi", "gb"],
                "x",
            ),
            m(
                channel(3, 3.0, None, false),
                Some(NOW + 4 * 3600),
                "nfl",
                &["chi", "gb"],
                "x",
            ),
            tonight,
        ],
        NOW,
    );
    assert_eq!(out.len(), 2);
    let later = out.iter().find(|g| g.channel.id == 3).unwrap();
    assert_eq!(later.alternates[0].id, 2);
}

#[test]
fn tape_delayed_live_session_folds_into_the_live_one() {
    // Live at 17:15 on one channel; a free-to-air delay at 21:30 the guide also marks live.
    let out = group_events(
        vec![
            motor(channel(1, 1.0, None, false), NOW, "qualifying"),
            motor(
                channel(2, 2.0, None, false),
                NOW + 4 * 3600 + 15 * 60,
                "qualifying",
            ),
        ],
        NOW,
    );
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].channel.id, 1);
}

#[test]
fn a_different_race_the_same_day_is_not_a_tape_delay() {
    let mut support = motor(channel(1, 1.0, None, false), NOW, "race");
    support.programme.as_mut().unwrap().title = Some("Supercars Adelaide: Supports".into());
    support.programme.as_mut().unwrap().end_utc = NOW + 3600;
    let mut main = motor(channel(2, 2.0, None, false), NOW + 2 * 3600, "race");
    main.programme.as_mut().unwrap().title = Some("Supercars: Race 36: Adelaide ᴸᶦᵛᵉ".into());
    let out = group_events(vec![support, main], NOW);
    assert_eq!(out.len(), 2);
}

#[test]
fn tape_delay_titles_may_differ_by_words_and_live_marker() {
    let mut live = motor(channel(1, 1.0, None, false), NOW, "qualifying");
    live.programme.as_mut().unwrap().title =
        Some("Formula 1 : Qatar Grand Prix: Qualifying ᴸᶦᵛᵉ".into());
    let mut delay = motor(channel(2, 2.0, None, false), NOW + 4 * 3600, "qualifying");
    delay.programme.as_mut().unwrap().title = Some("Formula 1: Qatar Qualifying".into());
    let out = group_events(vec![live, delay], NOW);
    assert_eq!(out.len(), 1);
}

#[test]
fn next_days_session_is_not_a_tape_delay() {
    // Sprint qualifying Friday, qualifying Saturday: same session kind, a day apart.
    let out = group_events(
        vec![
            motor(channel(1, 1.0, None, false), NOW, "qualifying"),
            motor(channel(2, 2.0, None, false), NOW + 24 * 3600, "qualifying"),
        ],
        NOW,
    );
    assert_eq!(out.len(), 2);
}

#[test]
fn a_replay_is_not_folded_as_a_tape_delay() {
    let mut replay = motor(channel(2, 2.0, None, false), NOW + 4 * 3600, "qualifying");
    replay.live = false;
    let out = group_events(
        vec![
            motor(channel(1, 1.0, None, false), NOW, "qualifying"),
            replay,
        ],
        NOW,
    );
    assert_eq!(out.len(), 2);
}
