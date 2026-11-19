use std::sync::Mutex;

use super::*;
use crate::store::NewSource;

#[derive(Default)]
struct Rec {
    stopped: Mutex<Vec<(String, String)>>,
    fail_ids: Vec<String>,
}

#[async_trait]
impl SessionStopper for Rec {
    async fn stop(&self, _s: i64, kind: &str, id: &str) -> Result<()> {
        if self.fail_ids.iter().any(|f| f == id) {
            return Err(crate::AppError::network("down"));
        }
        self.stopped.lock().unwrap().push((kind.into(), id.into()));
        Ok(())
    }
}

async fn store() -> (Store, i64) {
    let s = Store::open_memory().await.unwrap();
    let src = s
        .insert_source(&NewSource {
            kind: "dispatcharr".into(),
            name: "T".into(),
            base_url: None,
            secret_ref: None,
            username: None,
            tls_insecure: false,
            created_at: 1,
        })
        .await
        .unwrap();
    (s, src.id)
}

#[tokio::test]
async fn startup_cleanup_deletes_persisted_sessions() {
    let (s, sid) = store().await;
    track(&s, sid, KIND_CATCHUP, "c1").await.unwrap();
    track(&s, sid, KIND_HLS, "h1").await.unwrap();
    let rec = Rec::default();
    let r = cleanup_on_startup(&s, &rec).await.unwrap();
    assert_eq!(
        r,
        CleanupReport {
            stopped: 2,
            failed: 0
        }
    );
    assert!(s.list_open_sessions().await.unwrap().is_empty());
    assert_eq!(rec.stopped.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn failed_stops_stay_persisted_for_next_start() {
    let (s, sid) = store().await;
    track(&s, sid, KIND_CATCHUP, "ok").await.unwrap();
    track(&s, sid, KIND_CATCHUP, "bad").await.unwrap();
    let rec = Rec {
        fail_ids: vec!["bad".into()],
        ..Default::default()
    };
    let r = stop_all(&s, &rec).await.unwrap();
    assert_eq!((r.stopped, r.failed), (1, 1));
    let left = s.list_open_sessions().await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].session_id, "bad");
}

#[tokio::test]
async fn stop_kind_only_touches_that_kind() {
    let (s, sid) = store().await;
    track(&s, sid, KIND_CATCHUP, "c1").await.unwrap();
    track(&s, sid, KIND_HLS, "h1").await.unwrap();
    let rec = Rec::default();
    let r = stop_kind(&s, &rec, KIND_CATCHUP).await.unwrap();
    assert_eq!(r.stopped, 1);
    let left = s.list_open_sessions().await.unwrap();
    assert_eq!((left.len(), left[0].kind.as_str()), (1, KIND_HLS));
}

#[tokio::test]
async fn release_removes_row_only_after_server_confirms() {
    let (s, sid) = store().await;
    track(&s, sid, KIND_HLS, "x").await.unwrap();
    let bad = Rec {
        fail_ids: vec!["x".into()],
        ..Default::default()
    };
    assert!(release(&s, &bad, sid, KIND_HLS, "x").await.is_err());
    assert_eq!(s.list_open_sessions().await.unwrap().len(), 1);
    release(&s, &Rec::default(), sid, KIND_HLS, "x")
        .await
        .unwrap();
    assert!(s.list_open_sessions().await.unwrap().is_empty());
}
