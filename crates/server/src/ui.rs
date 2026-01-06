use axum::{
    http::{header, StatusCode, Uri},
    response::{Html, IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "ui/dist"]
#[prefix = "/"]
struct UiAssets;

#[derive(RustEmbed)]
#[folder = "static"]
#[prefix = "/"]
struct StaticAssets;

/// Serve the dashboard (Phase 10)
pub async fn serve_dashboard() -> Response {
    if let Some(content) = StaticAssets::get("dashboard.html") {
        return serve_file("dashboard.html", content.data.as_ref());
    }

    // Fallback if dashboard not found
    (
        StatusCode::NOT_FOUND,
        Html("<h1>Dashboard not found</h1>"),
    )
        .into_response()
}

/// Serve the embedded UI
pub async fn serve_ui(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    // Serve root as dashboard
    if path.is_empty() || path == "/" {
        return serve_dashboard().await;
    }

    // Try to serve from static assets first (Phase 10 dashboard assets)
    if let Some(content) = StaticAssets::get(path) {
        return serve_file(path, content.data.as_ref());
    }

    // Try to serve the requested file from UI assets
    if let Some(content) = UiAssets::get(path) {
        return serve_file(path, content.data.as_ref());
    }

    // For SPA routing, serve index.html for non-API routes
    if !path.starts_with("api/") {
        if let Some(content) = UiAssets::get("index.html") {
            return serve_file("index.html", content.data.as_ref());
        }
    }

    // Fallback: serve dashboard as the default page
    serve_dashboard().await
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
