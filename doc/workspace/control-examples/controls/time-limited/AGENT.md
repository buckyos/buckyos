# time-limited

Blocks in the `content` slot are shown to each viewer for `limit_ms` from that viewer's first opening of the canvas
(recorded once in their own user state, by the service's clock); then they are hidden for that viewer only.

- Put Blocks in by creating them under the instance with `slot: "content"`; change the time with
  `entity.set_props { key: "config", props: { limit_ms } }`.
- This is presentation, not access: a reader can still read the Blocks through the API, design mode shows them all
  (marked "Time-limited content"), and prints and exports show them all. Do not use it to keep anything secret.
