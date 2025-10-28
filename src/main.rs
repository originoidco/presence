use std::convert::Infallible;
use std::sync::Arc;

use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::info;
use warp::ws::{Message, WebSocket, Ws};
use warp::{Filter, Rejection, Reply, http::StatusCode};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpotifyActivity {
    pub track: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_art_url: Option<String>,
    pub started_at_ms: Option<i64>,
    pub ends_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceData {
    pub user_id: String,
    pub spotify: Option<SpotifyActivity>,
    pub timestamp_ms: i64,
}

type PresenceCache = Arc<DashMap<String, PresenceData>>;

type PresenceBroadcast = broadcast::Sender<String>;

#[derive(Clone)]
struct AppState {
    cache: PresenceCache,
    tx: PresenceBroadcast,
}

async fn get_presence_handler(user_id: String, state: AppState) -> Result<impl Reply, Rejection> {
    if let Some(presence) = state.cache.get(&user_id) {
        Ok(warp::reply::with_status(
            warp::reply::json(&*presence),
            StatusCode::OK,
        ))
    } else {
        Ok(warp::reply::with_status(
            warp::reply::json(&serde_json::json!({"error": "User not found"})),
            StatusCode::NOT_FOUND,
        ))
    }
}

async fn user_in_server_handler(
    user_id: String,
    _state: AppState,
) -> Result<impl Reply, Rejection> {
    let uid = match user_id.parse::<u64>() {
        Ok(v) => v,
        Err(_) => {
            return Ok(warp::reply::with_status(
                warp::reply::json(&serde_json::json!({ "error": "invalid user id" })),
                StatusCode::BAD_REQUEST,
            ));
        }
    };

    let guild_id = match std::env::var("GUILD_ID") {
        Ok(s) => match s.parse::<u64>() {
            Ok(v) => v,
            Err(_) => {
                return Ok(warp::reply::with_status(
                    warp::reply::json(&serde_json::json!({ "error": "invalid GUILD_ID in env" })),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ));
            }
        },
        Err(_) => {
            return Ok(warp::reply::with_status(
                warp::reply::json(&serde_json::json!({ "error": "GUILD_ID not set in env" })),
                StatusCode::INTERNAL_SERVER_ERROR,
            ));
        }
    };

    match discord::is_member(guild_id, uid).await {
        Ok(in_server) => Ok(warp::reply::with_status(
            warp::reply::json(&serde_json::json!({ "in_server": in_server })),
            StatusCode::OK,
        )),
        Err(e) => Ok(warp::reply::with_status(
            warp::reply::json(&serde_json::json!({ "error": e })),
            StatusCode::INTERNAL_SERVER_ERROR,
        )),
    }
}

fn with_state(state: AppState) -> impl Filter<Extract = (AppState,), Error = Infallible> + Clone {
    warp::any().map(move || state.clone())
}

async fn ws_handler(ws: WebSocket, user_id: String, state: AppState) {
    let mut rx = state.tx.subscribe();

    let (mut ws_tx, mut ws_rx) = ws.split();

    if let Some(presence) = state.cache.get(&user_id) {
        let _ = ws_tx
            .send(Message::text(
                serde_json::to_string(&*presence).unwrap_or_else(|_| "{}".into()),
            ))
            .await;
    }

    let user_id_for_task = user_id.clone();
    let cache = Arc::clone(&state.cache);

    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = ws_rx.next().await {
            if msg.is_close() {
                break;
            }
        }
    });

    let send_task = tokio::spawn(async move {
        while let Ok(changed_user_id) = rx.recv().await {
            #[allow(clippy::collapsible_if)]
            if changed_user_id == user_id_for_task {
                if let Some(presence) = cache.get(&changed_user_id) {
                    let payload = serde_json::to_string(&*presence).unwrap_or_else(|_| "{}".into());
                    if ws_tx.send(Message::text(payload)).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    let _ = tokio::try_join!(recv_task, send_task);
}

mod discord;

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    let cache: PresenceCache = Arc::new(DashMap::new());
    let (tx, _rx) = broadcast::channel::<String>(1024);
    let state = AppState { cache, tx };

    // GET: /v1/{userid}
    let get_route = warp::path!("v1" / String)
        .and(warp::get())
        .and(with_state(state.clone()))
        .and_then(get_presence_handler);

    // GET: /v1/{userid}/in_server
    let in_server_route = warp::path!("v1" / String / "in_server")
        .and(warp::get())
        .and(with_state(state.clone()))
        .and_then(user_in_server_handler);

    // WS: /ws/v1/{userid}
    let ws_route = warp::path!("ws" / "v1" / String)
        .and(warp::ws())
        .and(with_state(state.clone()))
        .map(|user_id: String, ws: Ws, state: AppState| {
            ws.on_upgrade(move |socket| ws_handler(socket, user_id, state))
        });

    // GET: /
    let root = warp::path::end().and(warp::get()).map(|| {
        warp::reply::json(&serde_json::json!({
            "endpoints": [
                {"method": "GET", "path": "/v1/{userid}"},
                {"method": "WS",  "path": "/ws/v1/{userid}"},
                {"method": "GET", "path": "/v1/{userid}/in_server"}
            ]
        }))
    });

    let routes = root
        .or(get_route)
        .or(in_server_route)
        .or(ws_route)
        .with(warp::cors().allow_any_origin());

    info!("Starting HTTP server on 0.0.0.0:8787");
    tokio::spawn(discord::start_discord(
        state.cache.clone(),
        state.tx.clone(),
    ));
    warp::serve(routes).run(([0, 0, 0, 0], 8787)).await;
}
