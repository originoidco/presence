use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;
use tokio::sync::OnceCell;
use tracing::{info, warn};

use crate::PresenceData;

const CACHE_TTL_SECS: u64 = 300;

static REDIS_CLIENT: OnceCell<Option<ConnectionManager>> = OnceCell::const_new();

pub async fn init_redis() -> bool {
    let result = REDIS_CLIENT
        .get_or_init(|| async {
            let url = match std::env::var("REDIS_URL") {
                Ok(u) => u,
                Err(_) => {
                    info!("REDIS_URL not set, using in-memory cache");
                    return None;
                }
            };

            match redis::Client::open(url.as_str()) {
                Ok(client) => match ConnectionManager::new(client).await {
                    Ok(cm) => {
                        info!("redis connected");
                        Some(cm)
                    }
                    Err(e) => {
                        warn!(?e, "failed to connect to redis, using in-memory cache");
                        None
                    }
                },
                Err(e) => {
                    warn!(?e, "invalid redis url, using in-memory cache");
                    None
                }
            }
        })
        .await;

    result.is_some()
}

pub fn is_redis_available() -> bool {
    REDIS_CLIENT.get().map(|opt| opt.is_some()).unwrap_or(false)
}

async fn get_redis() -> Option<ConnectionManager> {
    REDIS_CLIENT.get()?.clone()
}

pub struct Cache {
    memory: Arc<DashMap<String, PresenceData>>,
}

impl Cache {
    pub fn new() -> Self {
        Self {
            memory: Arc::new(DashMap::new()),
        }
    }

    pub async fn get(&self, user_id: &str) -> Option<PresenceData> {
        if let Some(mut redis) = get_redis().await {
            let key = format!("presence:{}", user_id);
            match redis.get::<_, Option<String>>(&key).await {
                Ok(Some(json)) => {
                    if let Ok(data) = serde_json::from_str(&json) {
                        return Some(data);
                    }
                }
                Ok(None) => return None,
                Err(_) => {}
            }
        }

        self.memory.get(user_id).map(|r| r.clone())
    }

    pub async fn set(&self, user_id: &str, data: &PresenceData) {
        if let Some(mut redis) = get_redis().await {
            let key = format!("presence:{}", user_id);
            if let Ok(json) = serde_json::to_string(data) {
                let _: Result<(), _> = redis.set_ex(&key, json, CACHE_TTL_SECS).await;
            }
        }

        self.memory.insert(user_id.to_string(), data.clone());
    }

    pub async fn remove(&self, user_id: &str) {
        if let Some(mut redis) = get_redis().await {
            let key = format!("presence:{}", user_id);
            let _: Result<(), _> = redis.del(&key).await;
        }

        self.memory.remove(user_id);
    }

    pub fn get_memory(&self) -> Arc<DashMap<String, PresenceData>> {
        self.memory.clone()
    }
}

pub async fn wait_for_redis(timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    let retry_delay = Duration::from_millis(200);

    while start.elapsed() < timeout {
        if init_redis().await {
            return true;
        }
        tokio::time::sleep(retry_delay).await;
    }

    false
}
