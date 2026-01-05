use axum::{
    http::{header, StatusCode, Uri},
    response::{Html, IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "ui/dist"]
#[prefix = "/"]
struct UiAssets;

/// Serve the embedded UI
pub async fn serve_ui(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    // Try to serve the requested file
    if let Some(content) = UiAssets::get(path) {
        return serve_file(path, content.data.as_ref());
    }

    // For SPA routing, serve index.html for non-API routes
    if !path.starts_with("api/") {
        if let Some(content) = UiAssets::get("index.html") {
            return serve_file("index.html", content.data.as_ref());
        }
    }

    // Fallback: show a placeholder page
    (
        StatusCode::OK,
        Html(include_str!("ui_placeholder.html")),
    )
        .into_response()
}

fn serve_file(path: &str, content: &[u8]) -> Response {
    let mime_type = mime_guess::from_path(path).first_or_octet_stream();

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, mime_type.as_ref())],
        content.to_vec(),
    )
        .into_response()
}
