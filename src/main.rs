mod agents;
mod db;
mod llm;
mod tools;

use axum::{
    extract::State,
    http::StatusCode,
    response::Html,
    routing::{get, post},
    Json, Router,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

#[derive(Clone)]
struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    db: Mutex<Connection>,
    llm: Option<llm::LlmClient>,
}

#[derive(Deserialize)]
struct ChatRequest {
    user_name: String,
    message: String,
}

#[derive(Serialize)]
struct ChatResponse {
    reply: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db_path =
        std::env::var("SCHEDULER_DB").unwrap_or_else(|_| "scheduler.db".to_string());
    let conn = db::init_db(&db_path)?;
    let llm = llm::LlmClient::new();
    let mode = match &llm {
        Some(c) => format!("LLM router enabled: {}", c.label()),
        None => "offline keyword router (no LLM configured)".to_string(),
    };
    println!("Scheduler agent starting — {mode}. DB: {db_path}");

    let state = AppState {
        inner: Arc::new(Inner {
            db: Mutex::new(conn),
            llm,
        }),
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/api/chat", post(chat))
        .route("/health", get(health))
        .with_state(state);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);
    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr).await?;
    println!("Listening on http://localhost:{port}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

async fn health() -> &'static str {
    "ok"
}

async fn chat(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> (StatusCode, Json<ChatResponse>) {
    let reply = agents::route_and_handle(&state.inner.db, state.inner.llm.as_ref(), &req.user_name, &req.message).await;
    (StatusCode::OK, Json(ChatResponse { reply }))
}
