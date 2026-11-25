---
name: reading_site_ops
description: Operate the owner's personal reading site from the goal that keeps it — the check script that wakes you only for a source that broke or went stale and for a changed reading profile, and tells the owner by itself when a source needs their sign-in; diagnosing and repairing a source (workspace first, then install); what is operation and what is an upgrade, and how to ask for one once; the daily look at what the owner reads; events and follow-ups; when a notification is allowed. Read it in the first activation of a goal that keeps a reading site, and whenever a source fails or the reading changed.
requires: the goal's own commands (`agent-session goal check|job`, `agent-session notify send`) and the `site` command on the Agent's private runtime; read_write Agent permission
---
# Operating a reading site

The site is the websites the owner reads, each fetched by a program, translated and shown on their phone (`https://reflow.<zone>/`, Agent Browser's home). You keep it working and close to what they read; you do not build it. Three things, never mixed up:

- the workspace (`cd "$OPENDAN_WORKSPACE_DIR"`): the source of everything — `sources/<source>/` (a package: `source.json`, `extract.ts`, `NOTES.md`, `fixtures/`), `check.ts`, `SITE.md` (what the owner asked and was told), `upgrades/`;
- what runs, installed from it: your jobs `src-<source>` (`site sync <source>` on the source's timetable) and `site-reading` (the hourly reading summary), the packages in the bucket, your check;
- the content: the owner's, in their bucket. `site status` shows it; `site --help` is the reference for every command.

Routine costs you nothing: syncs are jobs, watching is the check. An activation is for a repair, the daily look at the reading, an event worth following, an upgrade to ask for.

## Your check

It was installed from `$OPENDAN_AGENT_ROOT/goal_skills/reading_site_ops/references/check.ts`; its source is `check.ts` in the workspace. Every 5 minutes it reads `ctx.jobs` and decides by `ctx.slot_ms`:

| What it sees | What it does |
| --- | --- |
| `src-<source>` failed twice in a row, kind `program` | wakes you, once per installed version of the job: a repair that does not work wakes you again |
| `src-<source>` without an `ok` run for 3 of its periods, for any other reason (unreachable, timing out, not run) | wakes you once, until it syncs again |
| `src-<source>` failed with kind `auth` | no activation: one notification per episode that the source needs the owner's sign-in, opening its `home_url` in Agent Browser |
| a sync that passed with `metrics.pending_reason` `translate_config` (articles fetched, no model to translate them) | no activation: one notification that the Zone has no model for translation, until a sync publishes again |
| a sync with articles waiting for another reason (`translate_budget`, `time`, `images`) | nothing: that is normal |
| `site-reading` reports another `profile` (the three topics and sources most read in 7 days, in any order) than the one you last looked at, with 5 opens or more | wakes you, once a day at most |
| another job failed 3 times in a row | wakes you once per version |
| a job that is switched off (paused by the owner, stopped for an upgrade) | nothing |

The thresholds are constants at its top. To change it: edit `check.ts` in the workspace, `agent-session goal check run --script check.ts`, then `agent-session goal check set --script check.ts --reason "<why>"`. A notification is returned alone with `run: false`, so it goes out while you are held; the host drops `kind` from a run that exits 0, so what a passed sync left waiting is read from `last_run.metrics.pending` and `pending_reason`. Writing checks in general: goal skill `goal_check_script`; numbers that say the check is too loose: `goal_upkeep`.

## A source woke you

1. Look: `site status` (state, last check, last error, waiting), `agent-session goal job show src-<source>`, `agent-session goal job log src-<source>`. A `program` failure names what was expected and is not there; the source's `NOTES.md` ("What goes wrong") says what that usually means.
2. A work session of the owner's in this workspace (`agent-session sessions --active`): touch nothing, keep the matter in `pending`.
3. Reproduce it in the workspace, against the live site: `site try sources/<source> --no-translate --limit 2 --out "$OPENDAN_SESSION_DIR/try"`. It still passes against its old pages (`--fixtures`) and fails live: the site changed. Compare a live page with the fixture to see what moved.
4. Decide: operation or upgrade (next section). Operation:
   - repair `extract.ts` in the workspace; `site try sources/<source> --no-translate --record` until it passes and two articles read right; put what changed and the date into `NOTES.md`;
   - install: `site source put sources/<source>`, then `agent-session goal job put src-<source> --dir sources/<source> --command "site sync <source> --dir {dir}" --every <the job's every_secs> --timeout 600`, then `agent-session goal job run src-<source> --wait` and `site status`. Report it repaired only after that run was `ok`.
   - Never edit the installed copies (a job's `versions/`, the bucket's `_src/`): the next install overwrites them. `agent-session goal job rollback src-<source>` when the new version is worse.
5. The same repair failed twice: stop, ask for an upgrade.

By kind of failure:

- `transient` for 3 periods: the site is down, slow, or limits this address (429). Operation is to wait, or to sync less often: `site config set sources.<source>.every_secs <more>`, and the job's timetable with it (`goal job put` again with `--every <more>`). A site gone for good, or one that now refuses programs (403), is the owner's to decide: an upgrade request.
- A run that times out does too much: `site config set sources.<source>.max_new <fewer>`.
- `auth`: the check told the owner. Nothing to repair, and never sign in yourself. The job goes on trying and recovers after their sign-in.
- Articles `waiting (translate_budget)`: translation is paid from your period budget (`tokens_used_by_jobs` in `<budget>`) and capped a day by `limits.translate_tokens_per_day`; one of them is used up. Untranslated articles are not published: they come tomorrow. When it happens every day, lower `max_new` of the sources the owner reads least, and say in your report what budget the site would take. `waiting (translate_config)`: the Zone has no model for translation and nothing is published. The check told the owner; there is nothing for you to repair.

## Operation or upgrade

What the owner sees, what the system may do and what it costs all stay as they are: operation, yours. Anything else: an upgrade, the owner's.

| Operation | Upgrade |
| --- | --- |
| a selector, a field or an address that moved within the same site | fetching from a new domain |
| retrying; waiting for a site to come back | a new source |
| syncing less often; fewer articles per run | syncing more than twice as often |
| template parameters and the order of columns | a new layout (template code) |
| the check script and its timetable | fetching through the browser, or with the owner's sign-in |
| events and follow-ups within their budget | turning a source into list only, or dropping one |

## Asking for an upgrade

1. Write `upgrades/<yyyy-mm-dd>-<name>.md` in the workspace as goal skill `goal_delivery` says (what happened and how you know, what it affects, what you suggest, what happens otherwise, the draft of the task, ending with revising this goal). `<name>`: lower-case letters, digits and `-`, not used before (`site upgrade list`).
2. `site upgrade open <name> --title "<one line>" --summary "<what happened; what you suggest>" --file "$OPENDAN_WORKSPACE_DIR/upgrades/<yyyy-mm-dd>-<name>.md" --source <source> --goal <goal_id>`, title and summary in the owner's language: the source shows as needing an upgrade, with "Upgrade" and "Not now". Leave `--source` out for a matter of the whole site.
3. One notification: `agent-session notify send --title "<one line>" --message "<two sentences>" --key "upgrade:<goal_id>:<name>" --url "/?agent_browser=$OPENDAN_AGENT_DID&mode=home&upgrade=<name>"`.
4. Stop that source's job, `agent-session goal job disable src-<source>`: no more failing runs. What it published stays readable; every other source goes on.
5. One line in `pending` with the file's path.

Never twice. Declined (`dismissed` in `site upgrade list`: the owner tapped "Not now" or said so in a conversation) or left lying: say nothing more and keep the job stopped. Only a problem that itself changed gets a new request under a new `<name>`. When the owner agrees, a work session of theirs does it, closes the request, starts the job again and revises you: `changed_since_last_activation` tells you.

## Once a day: what the owner reads

When the check says the reading changed: `site reading --days 7` (what was opened in the site, for how long, how far; by source and by topic) and `site visits --days 7` (what the owner looked at in this Agent's browser, per host and per page: visits, time spent, last time; it says so when they switched the record off, which is theirs to do; `no_browser` means this session does not reach the Agent's browser: go on without it).

- Yours to change, one small step a day, each with its reason in the report: `site config set template.params.home_order|per_column|collapsed <json>` and `site config set columns '<json>'` (which columns, in which order, with which sources). What is read most goes up; a column nobody opens is folded, not removed.
- Not yours: `template.params.hide_topics` (only the owner's own words change it, in a conversation), removing or pausing a source, the template's code.
- A website they keep visiting that is not in the site (`site match <url>` says neither), or a source they never open, is a question for the owner: put it as an upgrade request ("Add <site>?"), at most one such suggestion a week. Nothing is added or dropped silently.
- Both records are the owner's private ones. To the owner, in a note or a notification, say no more than the site ("you often read example.com"): never a page's address or title, never how long or when. Nothing of them goes anywhere else.

## Events and follow-ups

- `site events get --out events.json` writes the file as `site events put` takes it. Group the articles on one matter into an event, change the file and `site events put events.json`: it fills in each entry's title, source and time from its article and refuses an article that is not in the bucket. Texts are in the site's language; `adds` says what this article adds to the earlier ones. Only matters with two articles or more; drop events nobody opened for a month.

```json
{ "schema": "buckyos.site.events/1", "updated_at": 1795000000000,
  "events": [ { "id": "e-20261124-chip-export", "title": "…", "summary": "…", "updated_at": 1795000000000,
                "articles": [ { "id": "<article id>", "adds": "…" } ], "supplements": [] } ] }
```

- A follow-up or a primary document for an event the owner read: fetch it, write it as one article in a JSON file (`{ "url", "title", "lang", "blocks": [ { "type": "p", "text" } ], … }`, the shape `extract` returns) and `site supplement put <file> --event <id> --adds "<what it adds to the earlier reports>"`; the event has to be in events.json first, and its `supplements` are kept as they are when you rewrite the file. Every supplement carries where it comes from (its `url`) and what it adds; one that adds nothing is not stored.
- Budget: at most 3 searches and 2 supplements an activation, and only for events the owner opened. From public pages only, as a source would: nothing behind a paywall or a sign-in.

## Notifications

Routine updates are silent: the owner opens the site when they want to read. Allowed are only: a source needs their sign-in (the check sends it); an upgrade request (once); a Zone without a model for translation (the check sends it too); at most one a day about an event the owner said they follow, when something important happened — `agent-session notify send --title … --message … --url "/?agent_browser=$OPENDAN_AGENT_DID&mode=home&route=e%3D<event id>"`. Everything else goes into your report and `distance`.
