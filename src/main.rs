use std::convert::Infallible;
use std::net::IpAddr;
use std::sync::Arc;

use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serenity::http::Http as SerenityHttp;
use serenity::model::id::GuildId;
use tokio::sync::watch;
use tokio::time::{Duration, Instant, interval_at, timeout};
use tracing::{info, warn};
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

const PRESENCE_TTL_MINUTES: i64 = 5;
const PRESENCE_TTL_MS: i64 = PRESENCE_TTL_MINUTES * 60 * 1000;
const MAX_CONNECTIONS_PER_IP: usize = 10;
const WS_SEND_TIMEOUT: Duration = Duration::from_secs(5);

pub type PresenceCache = Arc<DashMap<String, PresenceData>>;
pub type UserWatchers = Arc<DashMap<String, watch::Sender<Option<PresenceData>>>>;
type ConnectionCounter = Arc<DashMap<IpAddr, usize>>;

#[derive(Clone)]
struct AppState {
    cache: PresenceCache,
    watchers: UserWatchers,
    connections: ConnectionCounter,
    http: Arc<SerenityHttp>,
    guild_id: GuildId,
}

fn is_presence_stale(presence: &PresenceData) -> bool {
    let now = chrono::Utc::now().timestamp_millis();
    now - presence.timestamp_ms > PRESENCE_TTL_MS
}

fn validate_user_id(user_id: &str) -> bool {
    user_id.len() <= 20 && user_id.chars().all(|c| c.is_ascii_digit())
}

async fn get_presence_handler(user_id: String, state: AppState) -> Result<impl Reply, Rejection> {
    if !validate_user_id(&user_id) {
        return Ok(warp::reply::with_status(
            warp::reply::json(&serde_json::json!({"error": "invalid user id"})),
            StatusCode::BAD_REQUEST,
        ));
    }

    if let Some(presence) = state.cache.get(&user_id) {
        if !is_presence_stale(&presence) {
            return Ok(warp::reply::with_status(
                warp::reply::json(&*presence),
                StatusCode::OK,
            ));
        }
        drop(presence);
        state.cache.remove(&user_id);
    }
    Ok(warp::reply::with_status(
        warp::reply::json(&serde_json::json!({"error": "User not found"})),
        StatusCode::NOT_FOUND,
    ))
}

async fn user_in_server_handler(user_id: String, state: AppState) -> Result<impl Reply, Rejection> {
    let uid = match user_id.parse::<u64>() {
        Ok(v) => v,
        Err(_) => {
            return Ok(warp::reply::with_status(
                warp::reply::json(&serde_json::json!({ "error": "invalid user id" })),
                StatusCode::BAD_REQUEST,
            ));
        }
    };

    match discord::is_member(&state.http, state.guild_id, uid).await {
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

fn extract_client_ip() -> impl Filter<Extract = (IpAddr,), Error = Rejection> + Clone {
    warp::header::optional::<String>("cf-connecting-ip").map(|cf_ip: Option<String>| {
        cf_ip
            .and_then(|ip| ip.parse().ok())
            .unwrap_or_else(|| IpAddr::from([127, 0, 0, 1]))
    })
}

struct ConnectionGuard {
    connections: ConnectionCounter,
    ip: IpAddr,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        if let dashmap::mapref::entry::Entry::Occupied(mut entry) = self.connections.entry(self.ip)
        {
            *entry.get_mut() -= 1;
            if *entry.get() == 0 {
                entry.remove();
            }
        }
    }
}

fn try_acquire_connection(connections: &ConnectionCounter, ip: IpAddr) -> Option<ConnectionGuard> {
    let mut entry = connections.entry(ip).or_insert(0);
    if *entry >= MAX_CONNECTIONS_PER_IP {
        return None;
    }
    *entry += 1;
    drop(entry);

    Some(ConnectionGuard {
        connections: connections.clone(),
        ip,
    })
}

struct WatcherGuard {
    watchers: UserWatchers,
    cache: PresenceCache,
    user_id: String,
}

impl Drop for WatcherGuard {
    fn drop(&mut self) {
        if let Some(watcher) = self.watchers.get(&self.user_id)
            && watcher.receiver_count() == 0
        {
            drop(watcher);
            self.watchers.remove(&self.user_id);
            self.cache.remove(&self.user_id);
        }
    }
}

async fn ws_send_with_timeout(
    ws_tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    msg: Message,
) -> bool {
    matches!(timeout(WS_SEND_TIMEOUT, ws_tx.send(msg)).await, Ok(Ok(_)))
}

async fn ws_handler(ws: WebSocket, user_id: String, state: AppState, _conn_guard: ConnectionGuard) {
    let rx = state
        .watchers
        .entry(user_id.clone())
        .or_insert_with(|| watch::channel(None).0)
        .subscribe();

    let _watcher_guard = WatcherGuard {
        watchers: state.watchers.clone(),
        cache: state.cache.clone(),
        user_id: user_id.clone(),
    };

    let (mut ws_tx, mut ws_rx) = ws.split();

    if let Some(presence) = state.cache.get(&user_id)
        && !is_presence_stale(&presence)
        && let Ok(payload) = serde_json::to_string(&*presence)
    {
        let _ = ws_send_with_timeout(&mut ws_tx, Message::text(payload)).await;
    }

    ws_loop(&mut ws_tx, &mut ws_rx, rx).await;
}

async fn ws_loop(
    ws_tx: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    ws_rx: &mut futures_util::stream::SplitStream<WebSocket>,
    mut rx: watch::Receiver<Option<PresenceData>>,
) {
    let mut ping_interval = interval_at(
        Instant::now() + Duration::from_secs(25),
        Duration::from_secs(25),
    );

    loop {
        tokio::select! {
            _ = ping_interval.tick() => {
                if !ws_send_with_timeout(ws_tx, Message::ping(Vec::new())).await {
                    break;
                }
            }

            incoming = ws_rx.next() => {
                match incoming {
                    Some(Ok(msg)) if msg.is_close() => break,
                    Some(Ok(msg)) if msg.is_ping() => {
                        let _ = ws_send_with_timeout(ws_tx, Message::pong(msg.into_bytes())).await;
                    }
                    Some(Err(_)) | None => break,
                    _ => {}
                }
            }

            result = rx.changed() => {
                if result.is_err() {
                    break;
                }
                let presence = rx.borrow_and_update().clone();
                if let Some(ref p) = presence
                    && !is_presence_stale(p)
                    && let Ok(payload) = serde_json::to_string(p)
                    && !ws_send_with_timeout(ws_tx, Message::text(payload)).await
                {
                    break;
                }
            }
        }
    }
}

mod discord;

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    let token = std::env::var("DISCORD_BOT_TOKEN").expect("DISCORD_BOT_TOKEN not set");
    let guild_id: u64 = std::env::var("GUILD_ID")
        .expect("GUILD_ID not set")
        .parse()
        .expect("GUILD_ID must be a valid u64");

    let http = Arc::new(SerenityHttp::new(&token));
    let cache: PresenceCache = Arc::new(DashMap::new());
    let watchers: UserWatchers = Arc::new(DashMap::new());
    let connections: ConnectionCounter = Arc::new(DashMap::new());

    let state = AppState {
        cache,
        watchers,
        connections,
        http,
        guild_id: GuildId::new(guild_id),
    };

    let get_route = warp::path!("v1" / String)
        .and(warp::get())
        .and(with_state(state.clone()))
        .and_then(get_presence_handler);

    let in_server_route = warp::path!("v1" / String / "in_server")
        .and(warp::get())
        .and(with_state(state.clone()))
        .and_then(user_in_server_handler);

    let ws_route = warp::path!("ws" / "v1" / String)
        .and(warp::ws())
        .and(with_state(state.clone()))
        .and(extract_client_ip())
        .map(|user_id: String, ws: Ws, state: AppState, ip: IpAddr| {
            ws.on_upgrade(move |socket| async move {
                if !validate_user_id(&user_id) {
                    return;
                }
                match try_acquire_connection(&state.connections, ip) {
                    Some(guard) => ws_handler(socket, user_id, state, guard).await,
                    None => {
                        warn!(ip = %ip, "connection limit exceeded");
                    }
                }
            })
        });

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

    info!("starting http server on 0.0.0.0:8787");
    tokio::spawn(discord::start_discord(
        state.cache.clone(),
        state.watchers.clone(),
    ));
    warp::serve(routes).run(([0, 0, 0, 0], 8787)).await;
}
