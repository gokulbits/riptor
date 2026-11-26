//! The browser interface: a few static files built into Riptor, talking to the same API.

use axum::extract::Path;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

/// No inline code, nothing from other sites; the page can only call this server.
pub const CSP: &str =
    "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; \
connect-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'self'";

const INDEX: &str = include_str!("../web/index.html");
const FILES: &[(&str, &str, &str)] = &[
    (
        "app.js",
        "text/javascript; charset=utf-8",
        include_str!("../web/app.js"),
    ),
    (
        "app.css",
        "text/css; charset=utf-8",
        include_str!("../web/app.css"),
    ),
    ("icon.svg", "image/svg+xml", include_str!("../web/icon.svg")),
];

fn page(kind: &'static str, body: &'static str) -> Response {
    let mut r = (StatusCode::OK, body).into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(kind));
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    r
}

pub async fn index() -> Response {
    page("text/html; charset=utf-8", INDEX)
}

pub async fn asset(Path(name): Path<String>) -> Response {
    match FILES.iter().find(|(n, _, _)| *n == name) {
        Some(&(_, kind, body)) => page(kind, body),
        None => (StatusCode::NOT_FOUND, "Not Found").into_response(),
    }
}
