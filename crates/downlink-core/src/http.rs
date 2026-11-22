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
