//! Event grouping (docs/SPORTS.md §5.1): the same game or session on several channels becomes one
//! match. The best channel leads; the rest ride along as `alternates`.

use std::cmp::Ordering;

use super::matcher::normalize;
use crate::service::dto::{ChannelDto, SportsMatchDto};

/// Collapse matches that are the same event into one, best channel first, and return them in
/// display order (live first, then start, then score).
pub fn group_events(mut matches: Vec<SportsMatchDto>, now: i64) -> Vec<SportsMatchDto> {
    matches.sort_by(channel_preference);
    let mut groups: Vec<SportsMatchDto> = Vec::new();
    for m in matches {
        match groups.iter_mut().find(|g| same_event(g, &m, now)) {
            Some(g) => {
                let seen = g.channel.id == m.channel.id
                    || g.alternates.iter().any(|c| c.id == m.channel.id);
                if !seen {
                    g.alternates.push(m.channel);
                }
            }
            None => groups.push(m),
        }
    }
    groups.sort_by(|a, b| display_order(a, b, now));
    groups
}

/// Which copy of an event leads: favorite, then quality, then real guide data over a name-only
/// match, then score, then the lowest channel number.
fn channel_preference(a: &SportsMatchDto, b: &SportsMatchDto) -> Ordering {
    b.channel
        .is_favourite
        .cmp(&a.channel.is_favourite)
        .then(quality_rank(&b.channel).cmp(&quality_rank(&a.channel)))
        .then(a.programme.is_none().cmp(&b.programme.is_none()))
        .then(b.score.cmp(&a.score))
        .then(number_order(&a.channel, &b.channel))
        .then(a.channel.id.cmp(&b.channel.id))
}

/// Unknown quality ranks above SD: an unlabeled channel is usually HD.
fn quality_rank(c: &ChannelDto) -> u8 {
    match c.quality {
        Some("4K") => 4,
        Some("FHD") => 3,
        Some("HD") => 2,
        Some("SD") => 0,
        _ => 1,
    }
}

fn number_order(a: &ChannelDto, b: &ChannelDto) -> Ordering {
    match (a.number, b.number) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn display_order(a: &SportsMatchDto, b: &SportsMatchDto, now: i64) -> Ordering {
    let start = |m: &SportsMatchDto| m.programme.as_ref().map_or(now, |p| p.start_utc);
    b.live
        .cmp(&a.live)
        .then(start(a).cmp(&start(b)))
        .then(b.score.cmp(&a.score))
        .then(a.channel.id.cmp(&b.channel.id))
}

/// `group` is the leading match of an existing group; `m` is a candidate copy.
fn same_event(group: &SportsMatchDto, m: &SportsMatchDto, now: i64) -> bool {
    if group.league != m.league || group.session != m.session {
        return false;
    }
    match (&group.programme, &m.programme) {
        // Copies of one event air at the same time: their airtimes overlap. No fixed windows,
        // so a channel that opens with its own build-up still lines up with the others.
        (Some(a), Some(b)) => {
            a.start_utc < b.end_utc && b.start_utc < a.end_utc && same_identity(group, m)
        }
        // An event channel joins the airing its named start falls in; with no time in its
        // name, the airing that's on now.
        (Some(p), None) | (None, Some(p)) => {
            let ev = if group.programme.is_none() { group } else { m };
            let at = ev.event_start_utc.unwrap_or(now);
            p.start_utc <= at && at < p.end_utc && same_identity_untimed(group, m)
        }
        (None, None) => same_identity_untimed(group, m),
    }
}

/// Team leagues: overlapping teams (equal sets when both name two or more). Without teams on
/// either side, the normalized titles must match. Motorsport (no teams): league + session + time
/// already pin the event.
fn same_identity(a: &SportsMatchDto, b: &SportsMatchDto) -> bool {
    match (a.teams.is_empty(), b.teams.is_empty()) {
        (false, false) => teams_compatible(&a.teams, &b.teams),
        (true, true) => a.session.is_some() || title_key(a) == title_key(b),
        _ => false,
    }
}

/// Untimed copies need teams (team leagues) or a session (motorsport) to tie them together.
fn same_identity_untimed(a: &SportsMatchDto, b: &SportsMatchDto) -> bool {
    if !a.teams.is_empty() && !b.teams.is_empty() {
        return teams_compatible(&a.teams, &b.teams);
    }
    a.teams.is_empty() && b.teams.is_empty() && a.session.is_some()
}

fn teams_compatible(a: &[String], b: &[String]) -> bool {
    if a.len() >= 2 && b.len() >= 2 {
        let mut x = a.to_vec();
        let mut y = b.to_vec();
        x.sort();
        y.sort();
        return x == y;
    }
    a.iter().any(|t| b.contains(t))
}

fn title_key(m: &SportsMatchDto) -> String {
    let p = m.programme.as_ref();
    let title = p.and_then(|p| p.title.as_deref()).unwrap_or_default();
    let sub = p.and_then(|p| p.subtitle.as_deref()).unwrap_or_default();
    normalize(&format!("{title} {sub}")).join(" ")
}

#[cfg(test)]
mod tests;
