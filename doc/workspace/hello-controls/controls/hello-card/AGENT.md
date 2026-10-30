# Hello Card

One task record as a card. Bind `task` to a `buckyos.record` of the task table (fields `title`, `status`,
`owner`). Config: `show_owner` (boolean), `done_label` (text). The Done button sets `status` to `done`
through the host; nothing else is written. Insert with `view.type: "html"`.
