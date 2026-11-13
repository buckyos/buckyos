# photo-vote

Everyone picks up to `picks` photos of the bound folder `photos`; the cover shows the three with the most votes (ties in
folder order). Run mode: a tap on a photo picks or unpicks it.

- Place it with `buckyos aiws place <definition> --surface <canvas> --bind photos=<folder>`: its `on_bind` creates the
  `votes` table — a `text` field `voter` marked `submitter: true` (the service fills it with the caller; each person has
  one record and changes only theirs) and a `multi_select` field `picks` with `max_items` 3 — and once placed the action
  **Prepare the ballot** adds one option per photo, its label the file name. Run that action again after adding photos.
- Votes are records: insert yours with `table.insert_records` without the submitter field; a second insert answers
  `SUBMITTER_EXISTS` with your record id — change that one with `table.set_values`.
- The cover is computed by whoever reads — count each pick, most first: `SELECT j.value AS pick, count(*) AS n FROM "<votes
  table path>" v, json_each(v."<picks field name>#raw") j GROUP BY 1 ORDER BY n DESC, 1 LIMIT 3` (`doc.query { sql }`,
  `buckyos aiws sql`); never write it back. Names of voters are visible to readers of the table (no anonymous vote in v1).
