//! downlink-platform: OS integration. M1/M2 scope: sleep/resume notifications and quit hooks so
//! the app can pause and release the stream (CLAUDE.md rule 6, PLAN §4.9). Tray, media keys,
//! SMTC / Now Playing / MPRIS, deep links and single instance arrive later.

use std::sync::{Arc, Mutex};

pub mod power;

pub use power::{PowerCallback, PowerEvent, PowerWatcher, watch_power};

/// Errors from platform integration. Never carries URLs or credentials.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("platform service unavailable: {0}")]
    Unavailable(String),
}

type Hook = Box<dyn FnOnce() + Send>;

/// Ordered one-shot cleanup callbacks run on quit (`RunEvent::Exit`): stop the player, delete
/// catch-up / HLS / TS sessions. Running twice is harmless: each hook runs at most once.
#[derive(Default, Clone)]
pub struct QuitHooks {
    hooks: Arc<Mutex<Vec<Hook>>>,
}

impl QuitHooks {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, f: impl FnOnce() + Send + 'static) {
        if let Ok(mut h) = self.hooks.lock() {
            h.push(Box::new(f));
        }
    }

    /// Run every registered hook in registration order, then clear them.
    pub fn run_all(&self) {
        let hooks = match self.hooks.lock() {
            Ok(mut h) => std::mem::take(&mut *h),
            Err(p) => std::mem::take(&mut *p.into_inner()),
        };
        for h in hooks {
            h();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn hooks_run_in_order_once() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let q = QuitHooks::new();
        for i in 0..3 {
            let o = order.clone();
            q.register(move || {
                if let Ok(mut v) = o.lock() {
                    v.push(i);
                }
            });
        }
        let count = Arc::new(AtomicU32::new(0));
        let c = count.clone();
        q.register(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });
        q.run_all();
        q.run_all();
        assert_eq!(
            order.lock().map(|v| v.clone()).unwrap_or_default(),
            vec![0, 1, 2]
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
