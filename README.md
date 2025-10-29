# Presence

Presence is Originoid's service for getting the current Spotify/Listening status of users in our [Discord Server](https://discord.gg/noid). It connects a Discord Bot (with Presence Intent) to the Gateway and exposes it via REST and WebSocket.

> [!NOTE]
> Presence is only available for users who share a server with the bot and have Spotify connected and visible in Discord.
> You can try this for yourself by joining our [Discord](https://discord.gg/noid) and heading to `https://presence.originoid.co/v1/{your_discord_id}`

## Endpoints

- REST snapshot: `GET /v1/{DISCORD_USER_ID}`
- WebSocket stream: `WS /ws/v1/{DISCORD_USER_ID}`
- Check if user is in our server: `GET /v1/{DISCORD_USER_ID}/in_server`

## Usage

```bash
# REST
curl -s https://presence.originoid.co/v1/492731761680187403

# WebSocket (You can install wscat globally via your favourite node package manager!)
wscat -c wss://presence.originoid.co/ws/v1/492731761680187403
```

### Response

```json
{
  "user_id": "492731761680187403",
  "spotify": {
    "track": "Hanging Around",
    "artist": "Basement",
    "album": "Promise Everything (Deluxe)",
    "album_art_url": "https://i.scdn.co/image/ab67616d0000b273042a795b1fb9bf6efc9d5b21",
    "started_at_ms": 1758879636097,
    "ends_at_ms": 1758879815070
  },
  "timestamp_ms": 1758879633871
}
```

### Struct

```rs
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
```

## Why this approach?

This avoids dealing with Spotify in general while still providing real-time listening data, within Discord’s limitations.

This idea for the implementation is also heavily inspired by [Lanyard](https://github.com/Phineas/lanyard) but "dumbed-down" for our use-case, which I (dromzeh) personally use on my [own site](https://dromzeh.dev/) to display what I'm currently doing on Discord alongside what I'm listening to.

## License

[originoidco/presence](https://github.com/originoidco/presence) is licensed under the [GNU Affero General Public License v3.0](LICENSE). Authored by [@dromzeh](https://x.com/@dromzeh/).

You must state all significant changes made to the original software, make the source code available to the public with credit to the original author, original source, and use the same license.

> © 2025 Originoid LTD | Registered UK Company No. 15988228 | ICO Reference No. ZB857511
