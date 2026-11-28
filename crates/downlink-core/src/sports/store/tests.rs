//! Rebuild and listing against a temp database with invented channels and guide rows.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use super::*;
use crate::service::dto::{SportsLeagueFollowDto, SportsSeriesFollowDto};
use crate::sports::catalog::parse;
use crate::sports::matcher::tests::FIXTURE;
use crate::store::{ChannelUpsert, NewSource};

static CAT: LazyLock<Catalog> = LazyLock::new(|| parse(FIXTURE).unwrap());

const NOW: i64 = 1_795_780_800;

fn ch(ext: &str, name: &str, group: &str, adult: bool) -> ChannelUpsert {
    ChannelUpsert {
        ext_id: ext.into(),
        name: name.into(),
        number: ext.parse().ok(),
        group_ext_id: Some(group.into()),
        tvg_id: Some(format!("tvg.{ext}")),
        is_adult: adult,
        streams: vec![format!("s{ext}")],
        ..Default::default()
    }
}

fn prog(channel_id: i64, start: i64, title: &str, sub: Option<&str>, flags: i64) -> ProgrammeRow {
    ProgrammeRow {
        channel_id,
        start_utc: start,
        end_utc: start + 3 * 3600,
        ext_id: None,
        title: Some(title.into()),
        subtitle: sub.map(Into::into),
        description: None,
        season: None,
        episode: None,
        flags,
        icon: None,
    }
}

async fn seeded() -> (Store, HashMap<String, i64>) {
    let store = Store::open_memory().await.unwrap();
    let src = store
        .insert_source(&NewSource {
            kind: "m3u".into(),
            name: "Test".into(),
            base_url: None,
            secret_ref: None,
            username: None,
            tls_insecure: false,
            created_at: NOW,
        })
        .await
        .unwrap();
    let groups = store
        .sync_groups(
            src.id,
            &[
                ("g1".into(), "US Sports".into()),
                ("g2".into(), "Event Feeds".into()),
            ],
        )
        .await
        .unwrap();
    store
        .sync_channels(
            src.id,
            &[
                ch("1", "Sports One", "g1", false),
                ch("2", "Sports Two", "g1", false),
                ch("3", "Hidden Adult Sports", "g1", true),
                ch("4", "NFL 03: Bears vs Packers 1:00 PM ET", "g2", false),
                ch("5", "Movies", "g1", false),
            ],
            &groups,
        )
        .await
        .unwrap();
    let ids: HashMap<String, i64> =
        sqlx::query_as::<_, (String, i64)>("SELECT ext_id, id FROM channel")
            .fetch_all(store.pool())
            .await
            .unwrap()
            .into_iter()
            .collect();
    let live = flags::LIVE;
    store
        .upsert_programmes(&[
            prog(
                ids["1"],
                NOW + 3600,
                "NFL Football",
                Some("Chicago Bears at Green Bay Packers"),
                live,
            ),
            prog(
                ids["2"],
                NOW + 7200,
                "NFL Classic",
                Some("Chicago Bears at Green Bay Packers"),
                0,
            ),
            prog(
                ids["3"],
                NOW + 3600,
                "NFL Football",
                Some("Bears at Packers"),
                live,
            ),
            prog(
                ids["1"],
                NOW + 86_400,
                "Formula 1: Sprint Qualifying",
                None,
                live,
            ),
            prog(
                ids["2"],
                NOW + 86_400,
                "Formula 1: Free Practice 1",
                None,
                live,
            ),
            prog(ids["5"], NOW + 3600, "A Bears Movie", None, 0),
            // Outside the 7-day window.
            prog(
                ids["1"],
                NOW + 9 * 86_400,
                "NFL Football",
                Some("Bears at Packers"),
                live,
            ),
        ])
        .await
        .unwrap();
    (store, ids)
}

fn follow() -> SportsFollowDto {
    SportsFollowDto {
        version: 1,
        include_replays: false,
        leagues: BTreeMap::from([(
            "nfl".to_owned(),
            SportsLeagueFollowDto {
                teams: vec!["chi".into()],
            },
        )]),
        series: BTreeMap::from([(
            "f1".to_owned(),
            SportsSeriesFollowDto {
                sessions: vec!["race".into(), "qualifying".into()],
            },
        )]),
        priority: Vec::new(),
    }
}

#[tokio::test]
async fn missing_follow_doc_is_the_default() {
    let (store, _) = seeded().await;
    let f = load_follow(&store).await.unwrap();
    assert_eq!(f.version, 1);
    assert!(!f.include_replays && f.leagues.is_empty() && f.series.is_empty());
    // Nothing followed: rebuild empties the table.
    let r = rebuild(&store, &CAT, &ChannelFilter::default(), NOW)
        .await
        .unwrap();
    assert_eq!(r.matches, 0);
}

#[tokio::test]
async fn follow_doc_keeps_unknown_keys() {
    let (store, _) = seeded().await;
    store
        .kv_set(
            FOLLOW_KEY,
            r#"{"version":1,"includeReplays":false,"futureKey":7,
                "leagues":{"gone":{"teams":["x"]},"nfl":{"teams":[]}},"series":{}}"#,
            NOW,
        )
        .await
        .unwrap();
    save_follow(&store, &CAT, &follow(), NOW).await.unwrap();
    let raw: Value =
        serde_json::from_str(&store.kv_get(FOLLOW_KEY).await.unwrap().unwrap().0).unwrap();
    assert_eq!(raw["futureKey"], 7);
    assert_eq!(raw["leagues"]["gone"]["teams"][0], "x");
    assert_eq!(raw["leagues"]["nfl"]["teams"][0], "chi");
    let back = load_follow(&store).await.unwrap();
    assert!(back.leagues.contains_key("gone"));
    assert_eq!(back.series["f1"].sessions, ["race", "qualifying"]);
}

#[tokio::test]
async fn follow_priority_round_trips_and_defaults_empty() {
    let (store, _) = seeded().await;
    // A document written before priority existed loads with an empty ranking.
    store
        .kv_set(
            FOLLOW_KEY,
            r#"{"version":1,"leagues":{"nfl":{"teams":["chi"]}},"series":{}}"#,
            NOW,
        )
        .await
        .unwrap();
    assert!(load_follow(&store).await.unwrap().priority.is_empty());
    let ranked = SportsFollowDto {
        priority: vec!["f1".into(), "nfl:chi".into()],
        ..follow()
    };
    save_follow(&store, &CAT, &ranked, NOW).await.unwrap();
    assert_eq!(
        load_follow(&store).await.unwrap().priority,
        ["f1", "nfl:chi"]
    );
}

#[tokio::test]
async fn rebuild_matches_visible_programmes_and_event_channels() {
    let (store, ids) = seeded().await;
    save_follow(&store, &CAT, &follow(), NOW).await.unwrap();
    rebuild(&store, &CAT, &ChannelFilter::default(), NOW)
        .await
        .unwrap();

    let all: Vec<StoredMatch> = sqlx::query_as(
        "SELECT channel_id, start_utc, end_utc, league, teams, session, live, live_inferred,
                score, source FROM sports_match ORDER BY channel_id, start_utc",
    )
    .fetch_all(store.pool())
    .await
    .unwrap();
    // Adult channel hidden by the catalog filter; the movie and the far-future game never match.
    assert!(
        all.iter()
            .all(|m| m.channel_id != ids["3"] && m.channel_id != ids["5"])
    );
    assert!(
        all.iter()
            .all(|m| m.start_utc.is_none_or(|s| s < NOW + WINDOW_SECS))
    );
    let game = all
        .iter()
        .find(|m| m.channel_id == ids["1"] && m.league == "nfl")
        .unwrap();
    assert_eq!(game.teams, "chi,gb");
    assert!(
        game.live && !game.live_inferred,
        "this source sets LIVE flags"
    );
    let replay = all
        .iter()
        .find(|m| m.channel_id == ids["2"] && m.league == "nfl")
        .unwrap();
    assert!(!replay.live);
    let event = all.iter().find(|m| m.channel_id == ids["4"]).unwrap();
    assert_eq!(event.source, "channel");
    assert_eq!(event.start_utc, Some(NOW + 6 * 3600));
    assert!(event.live && event.live_inferred);

    // Listing: replays and unfollowed sessions are left out; live first, then start.
    let f = load_follow(&store).await.unwrap();
    let listed = list(&store, &CAT, &f, NOW, NOW + 2 * 86_400, NOW)
        .await
        .unwrap();
    let got: Vec<(i64, &str, Option<&str>)> = listed
        .iter()
        .map(|m| (m.channel_id, m.league.as_str(), m.session.as_deref()))
        .collect();
    assert_eq!(
        got,
        [
            (ids["1"], "nfl", None),
            (ids["4"], "nfl", None),
            (ids["1"], "f1", Some("qualifying")),
        ]
    );
    let with_replays = SportsFollowDto {
        include_replays: true,
        ..f
    };
    let listed = list(&store, &CAT, &with_replays, NOW, NOW + 2 * 86_400, NOW)
        .await
        .unwrap();
    assert!(listed.iter().any(|m| m.channel_id == ids["2"] && !m.live));

    // Programme rows come back for the DTO in one query.
    let rows = programmes_for(&store, &[(ids["1"], NOW + 3600)])
        .await
        .unwrap();
    assert_eq!(
        rows[&(ids["1"], NOW + 3600)].title.as_deref(),
        Some("NFL Football")
    );
}

#[tokio::test]
async fn followed_team_filter_applies_at_rebuild() {
    let (store, _) = seeded().await;
    let mut f = follow();
    f.leagues.insert(
        "nfl".into(),
        SportsLeagueFollowDto {
            teams: vec!["nyg".into()],
        },
    );
    save_follow(&store, &CAT, &f, NOW).await.unwrap();
    rebuild(&store, &CAT, &ChannelFilter::default(), NOW)
        .await
        .unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM sports_match WHERE league = 'nfl'")
        .fetch_one(store.pool())
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[test]
fn fts_phrases_are_quoted_and_safe() {
    assert_eq!(
        fts_phrase("O'Reilly \"Auto\"").as_deref(),
        Some("\"O'Reilly Auto\"")
    );
    assert_eq!(fts_phrase("NOT OR AND").as_deref(), Some("\"NOT OR AND\""));
    assert_eq!(fts_phrase("&&"), None);
}
