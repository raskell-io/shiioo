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
struct StaticAssets;

/// Serve the dashboard (Phase 10)
pub async fn serve_dashboard() -> Response {
    // Debug: List all embedded files
    tracing::debug!("StaticAssets embedded files:");
    for file in StaticAssets::iter() {
        tracing::debug!("  - {}", file);
    }

    // Try embedded assets first
    if let Some(content) = StaticAssets::get("dashboard.html") {
        tracing::info!("✅ Serving dashboard from embedded assets");
        return serve_file("dashboard.html", content.data.as_ref());
    }

    tracing::warn!("⚠️  Dashboard not found in embedded assets, trying filesystem fallback");

    // Fallback: try to read from filesystem (development mode)
    let dashboard_paths = [
        "crates/server/static/dashboard.html",
        "static/dashboard.html",
        "./static/dashboard.html",
    ];

    for path in &dashboard_paths {
        if let Ok(content) = std::fs::read(path) {
            tracing::info!("✅ Serving dashboard from filesystem: {}", path);
            return serve_file("dashboard.html", &content);
        }
    }

    // Last resort: dashboard not found
    tracing::error!("❌ Dashboard not found in embedded assets or filesystem");
    (
        StatusCode::NOT_FOUND,
        Html("<h1>Dashboard not found</h1><p>Tried embedded assets and filesystem fallback</p>"),
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
