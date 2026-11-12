# tv-screen

A broadcast: one video or sound (an `asset` Block in the `screen` slot) that every viewer plays at the same position.

- The position lives in the instance's config `broadcast = { pos_ms, at_ms, playing }`: at service time `at_ms` the
  video was at `pos_ms`. Viewers play at `pos_ms + (server now − at_ms)` while `playing`, at `pos_ms` when paused. Use
  the service's clock (`ws.time`), never a device clock.
- To start or pause: `entity.set_props { key: "config", props: { broadcast: { pos_ms, at_ms: <ws.time>, playing } } }`.
- To change the video: in ONE Commit, replace the asset's keys (or the Block in `screen`) and set
  `broadcast = { pos_ms: 0, at_ms: <ws.time>, playing: true }` — never the new video at the old position.
- `loop: true` loops; otherwise the end holds the last frame. Static views and prints show the poster.
- Readers cannot change any of this; their pause, mute and "Back to live" stay on their own device.
