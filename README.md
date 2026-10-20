# Emperor's Great Battle

A compact, browser-based 1v1 medieval battlefield prototype. The web server and authoritative game simulation are written in Rust (Axum + Tokio); Three.js renders a fixed medieval battlefield with a cinematic, tilted perspective camera.

## What's included

- Guest-only play: enter a display name, no sign-up, account table, or player database.
- Ephemeral rooms for two players, optional room password, case-insensitive duplicate room-name detection, lobby search, and WebSocket room updates.
- A bounded 120 × 80 playable meadow arena with a shallow central river, wooden fords, and two keeps with ramparts; the surrounding visual meadow extends beyond the play area so hard map edges do not intrude into the camera view.
- Toon-shaded cartoon soldiers with articulated arms and legs, visible faces, helmets, armor, heraldic capes, role-specific weapons, horses, an elephant howdah, catapult, general, and gold mines. Walking units animate their limbs.
- Server-side gold economy, unit placement restrictions, HP/damage/defense, nearest-enemy targeting, unit levels 1–5, upgrades, area damage, burn/poison effects, cavalry charge, general summoning, potion cooldowns, and 5,000-HP castles.
- Automatic cinematic camera using a Three.js perspective view (tilted like the supplied Tower Battles reference), smoothly panning between your side and the distant enemy side every ~10 seconds. The in-game camera button can pause/resume this automatic sweep. Camera focus stays within the arena, and the view remains fixed to prevent unrestricted scrolling.
- Render Blueprint at `render.yaml`.

## Run locally

Install a recent stable Rust toolchain, then run from this directory:

```bash
cargo run
```

Open `http://localhost:10000`. The server reads the `PORT` environment variable and defaults to port `10000`.

The browser loads Three.js and the optional display fonts from public CDNs, so a network connection is needed for those assets.

## Deploy to Render

1. Create a Git repository and upload the contents of this folder (the files inside the folder, not the ZIP itself).
2. In Render, choose **New → Blueprint** for the repository, or create a **Web Service** and select the Rust runtime.
3. With the Blueprint, Render reads `render.yaml`. If configuring manually, use:
   - Build command: `cargo build --release`
   - Start command: `./target/release/emperors-great-battle`
   - Health check path: `/`
4. Deploy the service and open the `onrender.com` URL. The app serves its page and its `/ws` WebSocket endpoint from the same service; Render's public WebSockets use `wss://` automatically on HTTPS pages.

Official Render references: [Rust/native runtimes](https://render.com/docs/native-runtimes), [Web Services](https://render.com/docs/web-services), [WebSockets](https://render.com/docs/websocket).

## Important prototype limitations

- **Rooms and matches are in RAM only.** No guest account, room, or match record is saved. Restarting/redeploying the service clears all active rooms and matches.
- This is a playable prototype, not a production-hardened competitive game. It has no anti-cheat/rate-limit system, reconnect-to-match flow, persistent matchmaking, database, or cross-instance room broker. Run a single service instance for room consistency.
- Balancing values for unit HP and potion duration are implementation choices because the initial brief specified HP as designable and did not specify potion durations. The requested ATK/DEF/SPEED/ATK SPEED values are used for the named troop types; levels scale stats and HP.
- A real compile/deploy should still be run in a Rust environment. The authoring environment used to prepare this archive did not include `rustc` or `cargo`, so Rust compilation could not be verified here. The JavaScript module syntax was checked with Node.js.
