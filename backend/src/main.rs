//! `ryu-token-table` — the standalone Token Table sidecar.

mod paths;

use std::net::{Ipv4Addr, SocketAddr};

use axum::extract::Request;
use axum::http::{header::AUTHORIZATION, StatusCode};
use axum::middleware::{from_fn, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use ryu_token_table::{routes, TableStore};
use serde_json::json;

const DEFAULT_PORT: u16 = 8019;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let port = std::env::var("RYU_TOKEN_TABLE_PORT")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(DEFAULT_PORT);
    let token = std::env::var("RYU_EXT_TOKEN")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if token.is_none() {
        tracing::warn!("ryu-token-table: RYU_EXT_TOKEN is unset; API routes are fail-closed");
    }

    let store = TableStore::open(paths::database_path())?;
    let gated_token = token.clone();
    let mounted = Router::new()
        .nest("/api/token-table", routes(store.clone()))
        .route("/openapi.json", get(openapi))
        .layer(from_fn(move |request: Request, next: Next| {
            let expected = gated_token.clone();
            async move { require_token(request, next, expected.as_deref()).await }
        }));
    let health_store = store;
    let app = Router::new()
        .route(
            "/health",
            get(move || {
                let store = health_store.clone();
                async move {
                    match store.list_tables() {
                        Ok(tables) => {
                            Json(json!({ "ok": true, "table_count": tables.len() })).into_response()
                        }
                        Err(error) => (
                            StatusCode::SERVICE_UNAVAILABLE,
                            Json(json!({ "ok": false, "error": error.to_string() })),
                        )
                            .into_response(),
                    }
                }
            }),
        )
        .merge(mounted);

    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!("ryu-token-table sidecar listening on http://{address}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn require_token(request: Request, next: Next, expected: Option<&str>) -> Response {
    let provided = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if ryu_sidecar_runtime::token_ok(provided, expected) {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
    }
}

async fn openapi() -> Json<serde_json::Value> {
    Json(json!({
        "openapi": "3.0.3",
        "info": { "title": "Token Table", "version": "0.1.0" },
        "paths": {
            "/api/token-table/tables": { "get": {}, "post": {} },
            "/api/token-table/tables/{table_id}": { "get": {} },
            "/api/token-table/tables/{table_id}/join": { "post": {} },
            "/api/token-table/tables/{table_id}/seat": { "post": {} },
            "/api/token-table/tables/{table_id}/leave": { "post": {} },
            "/api/token-table/tables/{table_id}/start": { "post": {} },
            "/api/token-table/tables/{table_id}/action": { "post": {} },
            "/api/token-table/tables/{table_id}/events": { "get": {} }
        }
    }))
}
