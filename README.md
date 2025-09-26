# Presence

Presence is Originoid's tool for getting the current Spotify status of users in our [Discord server](https://discord.gg/noid). It connects a Discord bot (with Presence Intent) to the Gateway and exposes it via REST and WebSocket.

## Endpoints

- REST snapshot: `GET /v1/{DISCORD_USER_ID}`
- WebSocket stream: `WS /ws/v1/{DISCORD_USER_ID}`

### Example Usage

```bash
# REST
curl -s https://presence.originoid.co/v1/492731761680187403

# WebSocket (You can install wscat globally via your favourite node package manager!)
wscat -c ws://presence.originoid.co/ws/v1/492731761680187403
```

### Development

```bash
# Clone the Repo
gh repo clone originoidco/presence && cd presence

# Set your Discord bot token (Presence Intent must be enabled on the bot)
DISCORD_BOT_TOKEN=YOUR_TOKEN_HERE

# Run the server (default localhost:8787)
cargo run
```

> [!NOTE]
> Presence is only available for users who share a guild with the bot and have Spotify connected and visible in Discord.
> WebSocket clients receive the latest snapshot on connect (if cached), then real-time updates.

## Why this approach?

This is heavily inspired by Lanyard. This avoids dealing with Spotify in general while still providing real-time listening data, within Discord’s limitations.

You're welcome to use this yourself! Ensure you enable the Presence Intent for your bot etc.

---

originoidco/presence is licensed under the [GNU Affero General Public License v3.0](LICENSE).

You must state all significant changes made to the original software, make the source code available to the public with credit to the original author, original source, and use the same license.

© 2025 Originoid LTD | Registered UK Company No. 15988228 | ICO Reference No. ZB857511

124 City Road, London, EC1V 2NX
