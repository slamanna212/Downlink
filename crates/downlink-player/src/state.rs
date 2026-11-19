//! Session state machine (PLAN §4.4) and the throttled event stream (`player://state`, <= 10 Hz).

use std::collections::VecDeque;
use std::time::Duration;

use serde::Serialize;

/// `Idle -> Resolving -> Connecting -> Buffering -> Playing <-> Paused -> Stopped | Failed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionState {
    Idle,
    Resolving,
    Connecting,
    Buffering,
    Playing,
    Paused,
    Stopped,
    Failed,
}

impl SessionState {
    /// Whether `self -> to` is a legal transition.
    pub fn can_go(self, to: SessionState) -> bool {
        use SessionState::*;
        if self == to {
            return false;
        }
        match self {
            Idle => matches!(to, Resolving | Connecting | Stopped),
            Resolving => matches!(to, Connecting | Failed | Stopped | Idle),
            Connecting => matches!(to, Buffering | Playing | Failed | Stopped | Resolving),
            Buffering => matches!(
                to,
                Playing | Paused | Connecting | Failed | Stopped | Resolving
            ),
            Playing => matches!(
                to,
                Paused | Buffering | Connecting | Failed | Stopped | Resolving
            ),
            Paused => matches!(
                to,
                Playing | Buffering | Connecting | Stopped | Failed | Resolving
            ),
            Stopped | Failed => matches!(to, Idle | Resolving | Connecting),
        }
    }
}

/// One recorded transition.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    /// Milliseconds since the Unix epoch (UTC).
    pub at_unix_ms: i64,
    pub from: SessionState,
    pub to: SessionState,
    pub note: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidTransition {
    pub from: SessionState,
    pub to: SessionState,
}

pub const HISTORY_LEN: usize = 50;

/// State plus the last [`HISTORY_LEN`] timestamped transitions.
#[derive(Debug)]
pub struct StateMachine {
    state: SessionState,
    history: VecDeque<Transition>,
}

impl Default for StateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl StateMachine {
    pub fn new() -> Self {
        Self {
            state: SessionState::Idle,
            history: VecDeque::with_capacity(HISTORY_LEN),
        }
    }

    pub fn state(&self) -> SessionState {
        self.state
    }

    pub fn history(&self) -> impl Iterator<Item = &Transition> {
        self.history.iter()
    }

    /// Attempt a transition. `now_unix_ms` is injected so tests are deterministic.
    pub fn transition(
        &mut self,
        to: SessionState,
        note: Option<String>,
        now_unix_ms: i64,
    ) -> Result<(), InvalidTransition> {
        if !self.state.can_go(to) {
            return Err(InvalidTransition {
                from: self.state,
                to,
            });
        }
        if self.history.len() == HISTORY_LEN {
            self.history.pop_front();
        }
        self.history.push_back(Transition {
            at_unix_ms: now_unix_ms,
            from: self.state,
            to,
            note,
        });
        self.state = to;
        Ok(())
    }
}

/// Coalesces state updates to at most one emission per `min_interval`, with a trailing flush.
#[derive(Debug)]
pub struct Throttle {
    min_interval: Duration,
    last_emit: Option<Duration>,
    dirty: bool,
}

impl Throttle {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            last_emit: None,
            dirty: false,
        }
    }

    /// Record that new state is waiting to be emitted.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// `now` is a monotonic clock reading. Returns true if the caller should emit right now.
    pub fn poll(&mut self, now: Duration) -> bool {
        if !self.dirty {
            return false;
        }
        let due = match self.last_emit {
            None => true,
            Some(t) => now.saturating_sub(t) >= self.min_interval,
        };
        if due {
            self.last_emit = Some(now);
            self.dirty = false;
        }
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SessionState::*;

    #[test]
    fn happy_path() {
        let mut m = StateMachine::new();
        for (i, s) in [
            Resolving, Connecting, Buffering, Playing, Paused, Playing, Stopped,
        ]
        .into_iter()
        .enumerate()
        {
            m.transition(s, None, i as i64).unwrap();
        }
        assert_eq!(m.state(), Stopped);
        assert_eq!(m.history().count(), 7);
    }

    #[test]
    fn zap_from_playing_to_connecting_is_legal() {
        let mut m = StateMachine::new();
        m.transition(Connecting, None, 0).unwrap();
        m.transition(Playing, None, 1).unwrap();
        m.transition(Connecting, Some("zap".into()), 2).unwrap();
    }

    #[test]
    fn illegal_transitions_are_rejected() {
        let mut m = StateMachine::new();
        assert_eq!(
            m.transition(Playing, None, 0),
            Err(InvalidTransition {
                from: Idle,
                to: Playing
            })
        );
        m.transition(Connecting, None, 0).unwrap();
        m.transition(Failed, None, 1).unwrap();
        assert!(m.transition(Playing, None, 2).is_err());
        assert!(m.transition(Paused, None, 2).is_err());
        assert!(m.transition(Failed, None, 2).is_err(), "self transition");
        m.transition(Connecting, None, 3).unwrap();
    }

    #[test]
    fn history_keeps_last_fifty() {
        let mut m = StateMachine::new();
        m.transition(Connecting, None, 0).unwrap();
        for i in 0..200 {
            let to = if m.state() == Connecting {
                Buffering
            } else {
                Connecting
            };
            m.transition(to, None, i + 1).unwrap();
        }
        assert_eq!(m.history().count(), HISTORY_LEN);
        assert_eq!(m.history().last().map(|t| t.at_unix_ms), Some(200));
        assert_eq!(m.history().next().map(|t| t.at_unix_ms), Some(151));
    }

    #[test]
    fn throttle_caps_at_ten_hz_with_trailing_flush() {
        let mut t = Throttle::new(Duration::from_millis(100));
        assert!(!t.poll(Duration::from_millis(0)), "nothing dirty");
        t.mark_dirty();
        assert!(t.poll(Duration::from_millis(1)));
        t.mark_dirty();
        assert!(!t.poll(Duration::from_millis(50)));
        assert!(!t.poll(Duration::from_millis(100)));
        assert!(t.poll(Duration::from_millis(101)), "trailing flush");
        assert!(!t.poll(Duration::from_millis(500)), "clean again");
        let mut emitted = 0;
        for ms in 0..1000u64 {
            t.mark_dirty();
            if t.poll(Duration::from_millis(1000 + ms)) {
                emitted += 1;
            }
        }
        assert!(emitted <= 10, "{emitted}");
    }
}
