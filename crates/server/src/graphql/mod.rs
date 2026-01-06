pub mod schema;

pub use schema::*;

use async_graphql::http::{playground_source, GraphQLPlaygroundConfig};
use async_graphql_axum::{GraphQLRequest, GraphQLResponse, GraphQLSubscription};
use axum::{
    extract::State,
    response::{Html, IntoResponse},
    Extension,
};
use std::sync::Arc;

use crate::config::AppState;

/// GraphQL query/mutation handler
pub async fn graphql_handler(
    State(state): State<Arc<AppState>>,
    Extension(schema): Extension<ShiiooSchema>,
    req: GraphQLRequest,
) -> GraphQLResponse {
    schema.execute(req.into_inner()).await.into()
}

/// GraphQL subscription handler (WebSocket)
pub async fn graphql_subscription_handler(
    Extension(schema): Extension<ShiiooSchema>,
    protocol: async_graphql_axum::GraphQLProtocol,
    websocket: axum::extract::WebSocketUpgrade,
) -> impl IntoResponse {
    websocket
        .protocols(async_graphql::http::ALL_WEBSOCKET_PROTOCOLS)
        .on_upgrade(move |stream| {
            async_graphql_axum::GraphQLWebSocket::new(stream, schema, protocol).serve()
        })
}

/// GraphQL Playground UI
pub async fn graphql_playground() -> impl IntoResponse {
    Html(playground_source(GraphQLPlaygroundConfig::new(
        "/api/graphql",
    ).subscription_endpoint("/api/graphql/ws")))
}
