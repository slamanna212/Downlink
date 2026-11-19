//! Client-side failover and retry policy (PLAN §4.5), as pure testable logic.
//!
//! On `end-file` with an error, or 8 s without a decoded frame: reconnect the same URL once, then
//! (sources with `Caps::FAILOVER`, i.e. Dispatcharr) ask for the channel's stream list and surface
//! the failure with the last error. Three attempts maximum, with backoff, never a loop. Sources
//! without server-side failover get the same two-attempt retry and then a sentence.

use std::time::Duration;

/// Seconds without a decoded frame before an attempt counts as failed.
pub const NO_FRAME_TIMEOUT: Duration = Duration::from_secs(8);
pub const MAX_ATTEMPTS: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureCause {
    /// `end-file` with an error; carries mpv's error text (already free of URLs).
    EndFileError(String),
    /// The stream ended while live (server closed the connection).
    UnexpectedEof,
    /// No decoded frame within [`NO_FRAME_TIMEOUT`].
    NoFrameTimeout,
}

impl FailureCause {
    pub fn describe(&self) -> String {
        match self {
            Self::EndFileError(e) => e.clone(),
            Self::UnexpectedEof => "the stream ended unexpectedly".to_owned(),
            Self::NoFrameTimeout => "no video arrived in time".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryDecision {
    /// Reload the same URL after `after`. `attempt` is the number of the upcoming attempt.
    RetrySame { after: Duration, attempt: u32 },
    /// Ask the catalogue for the channel's other streams (the caller then calls
    /// `Player::play_alternate`, which counts as the final attempt).
    AskForStreamList { last_error: String },
    /// Stop trying and surface the failure.
    GiveUp { last_error: String },
}

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    failover_capable: bool,
    /// Attempts made so far (the initial load is attempt 1).
    attempts: u32,
    escalated: bool,
}

impl RetryPolicy {
    pub fn new(failover_capable: bool) -> Self {
        Self {
            failover_capable,
            attempts: 1,
            escalated: false,
        }
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// Backoff before upcoming attempt number `attempt` (2 or 3): 250 ms, 500 ms, ... capped at 2 s.
    pub fn backoff(attempt: u32) -> Duration {
        let exp = attempt.saturating_sub(2).min(4);
        Duration::from_millis((250u64 << exp).min(2000))
    }

    /// Record a failed attempt and decide what to do next.
    pub fn on_failure(&mut self, cause: &FailureCause) -> RetryDecision {
        let last_error = cause.describe();
        let limit = if self.failover_capable {
            MAX_ATTEMPTS
        } else {
            2
        };
        if self.attempts >= limit || self.escalated && self.attempts >= MAX_ATTEMPTS {
            return RetryDecision::GiveUp { last_error };
        }
        // Second failure on a failover-capable source: the same URL has been retried once.
        if self.failover_capable && self.attempts == 2 && !self.escalated {
            return RetryDecision::AskForStreamList { last_error };
        }
        self.attempts += 1;
        RetryDecision::RetrySame {
            after: Self::backoff(self.attempts),
            attempt: self.attempts,
        }
    }

    /// The caller is loading an alternate stream: the third and final attempt.
    pub fn on_escalated(&mut self) {
        self.escalated = true;
        self.attempts = self.attempts.max(2) + 1;
    }

    /// A frame was shown: a later failure starts a fresh budget.
    pub fn on_playing(&mut self) {
        self.attempts = 1;
        self.escalated = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err() -> FailureCause {
        FailureCause::EndFileError("boom".into())
    }

    #[test]
    fn failover_source_retries_once_then_asks_then_gives_up() {
        let mut p = RetryPolicy::new(true);
        assert_eq!(
            p.on_failure(&err()),
            RetryDecision::RetrySame {
                after: Duration::from_millis(250),
                attempt: 2
            }
        );
        assert_eq!(
            p.on_failure(&err()),
            RetryDecision::AskForStreamList {
                last_error: "boom".into()
            }
        );
        p.on_escalated();
        assert_eq!(p.attempts(), 3);
        assert_eq!(
            p.on_failure(&err()),
            RetryDecision::GiveUp {
                last_error: "boom".into()
            }
        );
    }

    #[test]
    fn non_failover_source_gets_two_attempts() {
        let mut p = RetryPolicy::new(false);
        assert!(matches!(
            p.on_failure(&err()),
            RetryDecision::RetrySame { attempt: 2, .. }
        ));
        assert!(matches!(p.on_failure(&err()), RetryDecision::GiveUp { .. }));
    }

    #[test]
    fn never_loops_forever() {
        for failover in [false, true] {
            let mut p = RetryPolicy::new(failover);
            let mut steps = 0;
            loop {
                steps += 1;
                assert!(steps <= 5, "policy did not terminate");
                match p.on_failure(&FailureCause::NoFrameTimeout) {
                    RetryDecision::GiveUp { .. } => break,
                    RetryDecision::AskForStreamList { .. } => p.on_escalated(),
                    RetryDecision::RetrySame { .. } => {}
                }
            }
        }
    }

    #[test]
    fn playing_resets_the_budget() {
        let mut p = RetryPolicy::new(false);
        let _ = p.on_failure(&err());
        p.on_playing();
        assert!(matches!(
            p.on_failure(&err()),
            RetryDecision::RetrySame { .. }
        ));
    }

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(RetryPolicy::backoff(2), Duration::from_millis(250));
        assert_eq!(RetryPolicy::backoff(3), Duration::from_millis(500));
        assert!(RetryPolicy::backoff(99) <= Duration::from_secs(2));
    }

    #[test]
    fn timeout_is_eight_seconds() {
        assert_eq!(NO_FRAME_TIMEOUT, Duration::from_secs(8));
    }
}
