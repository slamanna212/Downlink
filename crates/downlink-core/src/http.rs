//! HTTP client settings for requests carrying credentials.

use reqwest::{ClientBuilder, redirect::Policy};

/// Keep authenticated redirects on the original scheme, host and port.
/// Reqwest's default header filtering does not cover custom credentials or request bodies.
pub fn authenticated(builder: ClientBuilder) -> ClientBuilder {
    builder.referer(false).redirect(Policy::custom(|attempt| {
        if attempt
            .previous()
            .first()
            .is_some_and(|original| original.origin() != attempt.url().origin())
        {
            return attempt.error("authenticated redirect changed origin");
        }
        Policy::default().redirect(attempt)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::{Client, StatusCode};
    use wiremock::matchers::{body_string, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn authenticated_redirect_stays_on_origin() {
        let origin = MockServer::start().await;
        let other = MockServer::start().await;
        Mock::given(path("/start"))
            .respond_with(ResponseTemplate::new(307).insert_header("location", "/next"))
            .expect(2)
            .mount(&origin)
            .await;
        Mock::given(method("POST"))
            .and(path("/next"))
            .and(header("x-api-key", "test-key"))
            .and(body_string("test-credential"))
            .respond_with(ResponseTemplate::new(307).insert_header("location", other.uri()))
            .expect(1)
            .mount(&origin)
            .await;
        Mock::given(method("GET"))
            .and(path("/next"))
            .and(header("x-plex-token", "test-token"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&origin)
            .await;

        let client = authenticated(Client::builder()).build().expect("client");
        let start = format!("{}/start", origin.uri());
        let result = client
            .post(&start)
            .header("x-api-key", "test-key")
            .body("test-credential")
            .send()
            .await;
        assert!(result.expect_err("cross-origin redirect").is_redirect());
        assert_eq!(
            client
                .get(start)
                .header("x-plex-token", "test-token")
                .send()
                .await
                .expect("same-origin redirect")
                .status(),
            StatusCode::OK
        );
        assert!(
            other
                .received_requests()
                .await
                .expect("requests")
                .is_empty()
        );
    }
}

/// Message of the error [`read_body`] returns when a reply exceeds its bound.
pub(crate) const TOO_LARGE: &str = "The server response is too large.";

/// True when `e` is [`read_body`]'s size-bound error.
pub(crate) fn is_too_large(e: &crate::AppError) -> bool {
    e.code == crate::ErrorCode::Server && e.message == TOO_LARGE
}

/// Read a response using an upper bound independent of the server's framing.
pub async fn read_body(
    mut response: reqwest::Response,
    max_bytes: usize,
    on_error: impl Fn(reqwest::Error) -> crate::AppError,
) -> crate::Result<Vec<u8>> {
    fn too_large() -> crate::AppError {
        crate::AppError::server(TOO_LARGE)
    }

    if response
        .content_length()
        .is_some_and(|len| len > max_bytes as u64)
    {
        return Err(too_large());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(&on_error)? {
        if chunk.len() > max_bytes - body.len() {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod body_tests {
    #![allow(clippy::unwrap_used)]
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use crate::AppError;

    async fn serve(response: &'static str) -> reqwest::Response {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                peer.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let _ = peer.write_all(response.as_bytes());
        });
        reqwest::get(format!("http://{address}")).await.unwrap()
    }

    #[tokio::test]
    async fn body_limit_covers_declared_and_streamed_sizes() {
        let known = serve("HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nabcde").await;
        assert!(super::read_body(known, 4, AppError::from).await.is_err());

        let chunked =
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nab\r\n3\r\ncde\r\n0\r\n\r\n";
        let response = serve(chunked).await;
        assert!(response.content_length().is_none());
        assert!(super::read_body(response, 4, AppError::from).await.is_err());

        let response = serve(chunked).await;
        assert_eq!(
            super::read_body(response, 5, AppError::from).await.unwrap(),
            b"abcde"
        );
    }
}
