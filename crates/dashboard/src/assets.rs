//! Embeds the three static frontend files (`index.html`, `app.js`,
//! `styles.css`) into the binary so the shipped artifact stays a single
//! static executable, with no runtime dependency on the source tree.
//!
//! The files under `static/` are byte-identical copies of
//! `tradebot/dashboard/static/{index.html,app.js,styles.css}` and must stay
//! that way: the frontend is not changing as part of this port.

use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "static/"]
pub struct Assets;

/// Serves the embedded `index.html` at `/`. Mirrors `index()` in server.py.
pub async fn index_handler() -> Response {
    match Assets::get("index.html") {
        Some(file) => Html(file.data.into_owned()).into_response(),
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "index.html missing from build",
        )
            .into_response(),
    }
}

/// Serves any other embedded static asset (currently `app.js` and
/// `styles.css`) at `/static/<path>`. Mirrors the `StaticFiles` mount in
/// server.py.
pub async fn static_handler(axum::extract::Path(path): axum::extract::Path<String>) -> Response {
    match Assets::get(&path) {
        Some(file) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            (
                [(header::CONTENT_TYPE, mime.as_ref().to_string())],
                file.data.into_owned(),
            )
                .into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}
