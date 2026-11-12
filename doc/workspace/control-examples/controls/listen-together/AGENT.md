# listen-together

An HTML control (host API 4) that plays the audio files of the bound folder `playlist` for everyone at the same moment.

- Shared state is the instance's config: `track` (a file name of the folder; empty: the first), `mode` (`order` or
  `shuffle`) and `anchor = { pos_ms, at_ms, playing }` — at service time `at_ms` the track was at `pos_ms`. Use the
  service's clock (`ws.time`), never a device clock. Change `track` and `anchor` in ONE Commit
  (`entity.set_props { key: "config", props: { track, anchor: { pos_ms: 0, at_ms: <ws.time>, playing: true } } }`).
- The toolbar offers the owner (whoever has `update`) Start, Pause, Resume, Next and the play mode, also before the
  control runs. A running player follows a change at once.
- A song's lyrics are a `.lrc` file of the same name in the folder (`[mm:ss.xx] line`).
- A listener's volume and their own listening ("On my own": track and position) are their own user state
  (`block:<instance>/volume`, `block:<instance>/own`), not the document: do not write them for anyone.
