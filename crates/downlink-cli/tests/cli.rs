#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::process::Command;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY: &str = "sk_test_cli_synthetic_key_999";
const FX: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../downlink-core/tests/fixtures/dispatcharr/"
);

fn fx(name: &str) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(format!("{FX}{name}")).unwrap()).unwrap()
}

async fn server() -> MockServer {
    let s = MockServer::start().await;
    for (p, f) in [
        ("/api/core/version/", "version.json"),
        ("/api/accounts/users/me/", "me.json"),
        ("/api/core/settings/", "settings.json"),
        ("/api/core/streamprofiles/", "stream_profiles.json"),
        ("/api/channels/groups/", "groups.json"),
        ("/api/epg/grid/", "grid.json"),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_json(fx(f)))
            .mount(&s)
            .await;
    }
    // Single unpaginated page keeps the CLI test small.
    Mock::given(method("GET"))
        .and(path("/api/channels/channels/"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(fx("channels_page1.json")["results"].clone()),
        )
        .mount(&s)
        .await;
    s
}

fn run(uri: &str, args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_downlink-cli"))
        .args(["--source", uri, "--key", KEY])
        .args(args)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into(),
        String::from_utf8_lossy(&out.stderr).into(),
    )
}

async fn cli(s: &MockServer, args: &'static [&'static str]) -> (bool, String, String) {
    let uri = s.uri();
    tokio::task::spawn_blocking(move || run(&uri, args))
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn channels_and_search() {
    let s = server().await;
    let (ok, out, err) = cli(&s, &["channels"]).await;
    assert!(ok, "{err}");
    assert!(
        out.contains("Channel One") && out.contains("Channel Two"),
        "{out}"
    );
    let (_, out, _) = cli(&s, &["channels", "--search", "two"]).await;
    assert!(
        out.contains("Channel Two") && !out.contains("Channel One"),
        "{out}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn epg_runs() {
    let s = server().await;
    let (ok, out, err) = cli(&s, &["epg"]).await;
    assert!(ok, "{err}");
    assert!(out.contains("now:") && out.contains("next:"), "{out}");
}

#[tokio::test(flavor = "multi_thread")]
async fn play_plan_is_redacted_and_never_fetches_the_stream() {
    let s = server().await;
    let (ok, out, err) = cli(&s, &["play-plan", "Channel Two"]).await;
    assert!(ok, "{err}");
    assert!(!out.contains(KEY) && !err.contains(KEY), "{out}{err}");
    assert!(
        out.contains("\"needs_redirect_resolve\": true"),
        "redirect channel flagged: {out}"
    );
    assert!(out.contains("/proxy/ts/stream/"), "{out}");
    let reqs = s.received_requests().await.unwrap();
    assert!(
        reqs.iter().all(|r| !r.url.path().starts_with("/proxy/")),
        "no stream request"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_channel_fails_cleanly() {
    let s = server().await;
    let (ok, _, err) = cli(&s, &["play-plan", "zzzzqqq"]).await;
    assert!(!ok && err.contains("No channel matches"), "{err}");
}
