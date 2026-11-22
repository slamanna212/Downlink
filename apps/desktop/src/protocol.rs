//! `dlimg://<hash>/<size>` custom protocol: images come from the core cache/fetcher, never from
//! remote origins (PLAN §3.6). On Windows and Android the webview reaches custom protocols as
//! `http://dlimg.localhost/<hash>/<size>`, and wry hands that to the handler rewritten as
//! `dlimg://localhost/<hash>/<size>`; all three forms are accepted.

use std::sync::Arc;

use downlink_core::service::Service;
use tauri::http::{Response, StatusCode, header};
use tauri::{Builder, Manager, Runtime};

use crate::state::AppState;

/// Normalise a request URI to the `dlimg://hash/size` form core parses.
pub fn normalise_uri(uri: &str) -> Option<String> {
    let u = url::Url::parse(uri).ok()?;
    let host = u.host_str()?;
    if host == "dlimg.localhost" {
        let path = u.path().trim_start_matches('/');
        Some(format!("dlimg://{path}"))
    } else if u.scheme() == "dlimg" && host == "localhost" {
        // wry's Windows/Android workaround reverts `http://dlimg.localhost/x` to `dlimg://localhost/x`.
        let path = u.path().trim_start_matches('/');
        Some(format!("dlimg://{path}"))
    } else if u.scheme() == "dlimg" {
        Some(format!("dlimg://{host}{}", u.path()))
    } else {
        None
    }
}

fn error(status: StatusCode) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .body(Vec::new())
        .unwrap_or_default()
}

pub fn register<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    builder.register_asynchronous_uri_scheme_protocol("dlimg", move |ctx, request, responder| {
        let app = ctx.app_handle().clone();
        let uri = request.uri().to_string();
        tauri::async_runtime::spawn(async move {
            let service: Option<Arc<Service>> = match app.try_state::<AppState>() {
                Some(s) => s.svc().await.ok(),
                None => None,
            };
            let resp = match (normalise_uri(&uri), service) {
                (Some(u), Some(service)) => match service.image(&u).await {
                    Ok(img) => {
                        if img.stale {
                            // Old copy shown now; a newer picture appears on the next load.
                            let (svc, u) = (service.clone(), u.clone());
                            tauri::async_runtime::spawn(async move {
                                if let Err(e) = svc.refresh_image(&u).await {
                                    tracing::debug!("image re-check failed: {}", e.message);
                                }
                            });
                        }
                        Response::builder()
                            .status(StatusCode::OK)
                            .header(header::CONTENT_TYPE, img.mime)
                            .header(header::CACHE_CONTROL, "max-age=86400")
                            .body(img.bytes)
                            .unwrap_or_default()
                    }
                    Err(e) => {
                        tracing::debug!("image request failed: {}", e.message);
                        error(StatusCode::NOT_FOUND)
                    }
                },
                (None, _) => error(StatusCode::BAD_REQUEST),
                (_, None) => error(StatusCode::SERVICE_UNAVAILABLE),
            };
            responder.respond(resp);
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_both_scheme_forms() {
        let h = "a".repeat(64);
        assert_eq!(
            normalise_uri(&format!("dlimg://{h}/160")),
            Some(format!("dlimg://{h}/160"))
        );
        assert_eq!(
            normalise_uri(&format!("http://dlimg.localhost/{h}/160")),
            Some(format!("dlimg://{h}/160"))
        );
        assert_eq!(
            normalise_uri(&format!("dlimg://localhost/{h}/160")),
            Some(format!("dlimg://{h}/160"))
        );
        assert_eq!(normalise_uri("https://evil.example/x/y"), None);
    }
}
