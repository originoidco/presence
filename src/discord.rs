use std::sync::Arc;

use dashmap::DashMap;
use serenity::all::{ActivityType, Client, Context, EventHandler, GatewayIntents, Presence};
use serenity::async_trait;
use tokio::sync::broadcast::Sender;
use tracing::{error, info};

use crate::{PresenceData, SpotifyActivity};

pub type PresenceCache = Arc<DashMap<String, PresenceData>>;

pub struct Handler {
    pub cache: PresenceCache,
    pub tx: Sender<String>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn presence_update(&self, _ctx: Context, new: Presence) {
        let user_id = new.user.id.to_string();

        let raw_spotify_activity = new
            .activities
            .iter()
            .find(|a| a.kind == ActivityType::Listening);

        if let Some(a) = raw_spotify_activity {
            info!(user_id = %user_id, activity = ?a, "Raw Spotify activity (presence_update)");
        }

        let spotify: Option<SpotifyActivity> = raw_spotify_activity.map(|a| {
            let album_art_hash = a
                .assets
                .as_ref()
                .and_then(|asst| asst.large_image.as_ref())
                .map(|li| li.strip_prefix("spotify:").unwrap_or(li).to_string());

            let album_art_url = album_art_hash
                .as_ref()
                .map(|hash| format!("https://i.scdn.co/image/{}", hash));

            SpotifyActivity {
                track: a.details.clone(),
                artist: a.state.clone(),
                album: a.assets.as_ref().and_then(|asst| asst.large_text.clone()),
                album_art_url,
                started_at_ms: a
                    .timestamps
                    .as_ref()
                    .and_then(|t| t.start.map(|v| v as i64)),
                ends_at_ms: a.timestamps.as_ref().and_then(|t| t.end.map(|v| v as i64)),
            }
        });

        let presence = PresenceData {
            user_id: user_id.clone(),
            spotify,
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
        };

        self.cache.insert(user_id.clone(), presence);
        let _ = self.tx.send(user_id);
    }
}

pub async fn start_discord(cache: PresenceCache, tx: Sender<String>) {
    let token = std::env::var("DISCORD_BOT_TOKEN").expect("DISCORD_BOT_TOKEN not set");
    let intents = GatewayIntents::GUILDS | GatewayIntents::GUILD_PRESENCES;
    let handler = Handler { cache, tx };

    match Client::builder(token, intents).event_handler(handler).await {
        Ok(mut client) => {
            info!("Discord client starting");
            if let Err(err) = client.start().await {
                error!(?err, "Discord client error");
            }
        }
        Err(err) => {
            error!(?err, "Error creating Discord client");
        }
    }
}
