use std::time::Duration;

use serenity::all::{ActivityType, Client, Context, EventHandler, GatewayIntents, Presence, Ready, ResumedEvent};
use serenity::async_trait;
use serenity::http::Http as SerenityHttp;
use serenity::model::id::{GuildId, UserId};
use tracing::{debug, error, info, warn};

use crate::{PresenceCache, PresenceData, SpotifyActivity, UserWatchers};

pub struct Handler {
    pub cache: PresenceCache,
    pub watchers: UserWatchers,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        info!(user = %ready.user.name, "discord gateway connected");
    }

    async fn resume(&self, _ctx: Context, _: ResumedEvent) {
        info!("discord gateway resumed");
    }

    async fn presence_update(&self, _ctx: Context, new: Presence) {
        let user_id = new.user.id.to_string();

        let watcher = match self.watchers.get(&user_id) {
            Some(w) => w,
            None => return,
        };

        let raw_spotify_activity = new
            .activities
            .iter()
            .find(|a| a.kind == ActivityType::Listening);

        if let Some(a) = raw_spotify_activity {
            debug!(user_id = %user_id, activity = ?a, "spotify activity");
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

        if watcher.send(Some(presence.clone())).is_ok() {
            self.cache.insert(user_id, presence);
        }
    }
}

pub async fn start_discord(cache: PresenceCache, watchers: UserWatchers) -> ! {
    let token = std::env::var("DISCORD_BOT_TOKEN").expect("DISCORD_BOT_TOKEN not set");
    let intents = GatewayIntents::GUILDS | GatewayIntents::GUILD_PRESENCES;

    let mut attempt: u32 = 0;

    loop {
        let handler = Handler {
            cache: cache.clone(),
            watchers: watchers.clone(),
        };

        match Client::builder(&token, intents).event_handler(handler).await {
            Ok(mut client) => {
                attempt = 0;
                info!("discord client starting");
                if let Err(err) = client.start().await {
                    warn!(?err, "discord client stopped, will restart");
                }
            }
            Err(err) => {
                error!(?err, "failed to create discord client, will retry");
            }
        }

        attempt = attempt.saturating_add(1);
        let backoff_secs = 2_u64.saturating_pow(attempt.min(6)).min(60);
        warn!(attempt, backoff_secs, "reconnecting after backoff");
        tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
    }
}

pub async fn is_member(http: &SerenityHttp, guild_id: GuildId, user_id: u64) -> Result<bool, String> {
    match http.get_member(guild_id, UserId::new(user_id)).await {
        Ok(_) => Ok(true),
        Err(err) => {
            if let serenity::Error::Http(http_err) = &err
                && http_err.status_code().map(|s| s.as_u16()) == Some(404)
            {
                return Ok(false);
            }
            Err(format!("discord api error: {:?}", err))
        }
    }
}
