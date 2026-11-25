# Handing a reading site to its goal

The method is the manual `goal_handover` (`$OPENDAN_AGENT_ROOT/cmd_skills/goal_handover/SKILL.md`): look at the goals there are, try the check before there is a goal, create or revise, see it work, report. This file adds only what a reading site fixes.

#### One site, one goal

`site config get maintainer` and `agent-session goal list` decide what you do:

| `maintainer` | Do |
| --- | --- |
| `null`, or a goal of this Agent that is archived or gone | create the goal (below) and enter it as maintainer |
| a live goal of this Agent | revise it: the new source in its objective, the source's job added |
| a goal of another Agent | "Taking over" below |

Never a second goal for the site, never a goal per source.

#### What the goal starts with

- Its method is the goal skill `reading_site_ops`, installed with this Agent: a name is enough.
- `SITE.md` in the workspace: for each source what the owner asked, how it is read in one line (the detail is its `NOTES.md`), what the owner was told, what failed. The goal gets it as a skill.
- Its check script: the template beside that skill. Copy it, fill in the two constants at its top (`AGENT`: this Agent's DID, `echo "$OPENDAN_AGENT_DID"`; `TEXT`: its two notifications, sign-in needed and no model for translation, in the owner's language), and try it:

```sh
cp "$OPENDAN_AGENT_ROOT/goal_skills/reading_site_ops/references/check.ts" check.ts
agent-session goal check run --script check.ts --every 300 --workspace "$OPENDAN_WORKSPACE_ID"
```

  Without jobs it answers `run: false`. What it wakes the goal for is in its header and in `reading_site_ops`.

#### A new goal

```sh
agent-session goal create --title "<the site's title>" --lifecycle maintain \
  --objective "<objective>" --healthy-state "<healthy state>" --distance "<distance>" \
  --constraint "<1>" --constraint "<2>" --constraint "<3>" \
  --workspace "$OPENDAN_WORKSPACE_ID" --skill reading_site_ops --skill site_notes="$OPENDAN_WORKSPACE_DIR/SITE.md" \
  --check check.ts --every 300 --key "$OPENDAN_SESSION_ID:goal"
```

No `--start` and no budget. The texts, with the site's own values in them:

- Objective: "Keep <owner>'s reading site (https://reflow.<zone>/ and Agent Browser's home; defined by site.json in the owner's bucket, `site status`; the sources of its programs in workspace <workspace_id>) showing what its definition says, and improve what it shows by what the owner reads. Sources: <id> (<title>, <home_url>, every <n> minutes, <full text | list only | with the owner's sign-in>); … Operation: every source keeps syncing through its job src-<id>; a source that breaks after a small change of the original site is repaired, in the workspace first and then installed; the job site-reading keeps the reading summary; once a day at most the template parameters and the order of columns follow what the owner reads; articles on one matter are grouped into events, and follow-ups are added to the events the owner reads. The owner hears from this goal only when a source needs their sign-in, when an upgrade is asked for, and at most once a day about an event they said they follow; routine updates are silent. An upgrade — a new source or domain, a changed layout, a source moved to the browser or to a sign-in, more than twice the frequency — is asked for once and done by a work session of the owner's."
- Healthy state: "Every active source synced within 3 of its periods; the articles the owner opens are translated and have their pictures; no failure is left alone: it is repaired, or an upgrade is asked for and that source's job is stopped."
- Distance: "`site status` (each source's state, last check, last error, articles waiting), the last runs of the jobs, open upgrade requests (`site upgrade list`), `site reading --days 7`."
- Constraints: "Hidden topics, pausing a source and removing a source are the owner's: never change them unasked." "No way around a paywall, a sign-in wall or a block against programs; no source synced more often than site.json says." "Translation stays within limits.translate_tokens_per_day of site.json; what does not fit waits for the next day."

Then the site learns who keeps it, and the jobs go in:

```sh
site config set maintainer '{"agent":"<this Agent DID>","goal_id":"<goal_id>"}'
site config set limits.translate_tokens_per_day <tokens>
mkdir -p reading
printf 'Reading summary of the site: the job runs "site reading --compact".\n' > reading/README.md
agent-session goal job put site-reading --goal <goal_id> --dir reading --command "site reading --compact" --every 3600 --title "Reading summary"
```

- `<tokens>` is the smaller of 1000000 and half of the goal's budget for a period: `goal.budget.period.max_total_tokens` in `agent-session goal show <goal_id>`, or, when the goal states none, `max_total_tokens` under `[session.goal.budget.period]` in `$OPENDAN_AGENT_ROOT/agent.toml`. Translation and the goal's own activations are paid from that one budget; half of it stays for repairs.
- A job's directory needs at least one file; the reading summary has no program of its own, hence the one line of text.
- Each source's job as the manual shows (`src-<source>`, `site sync <source> --dir {dir}`, `--every` the source's `every_secs`, `--timeout 600`), after `site source put sources/<source> --state active`.

#### A goal there is

`agent-session goal show <goal_id>` for the revision, then `agent-session goal revise <goal_id> --expected-revision <n> --objective "<the whole text, the new source in its list>" --reason "<source> added"`, the source's job, and a line in `SITE.md`. Its check needs no change: it reads the jobs. `agent-session goal post <goal_id> --msg "<what to do now>"` only when something cannot wait for its next activation.

#### See it work

- `agent-session goal job run src-<source> --goal <goal_id> --wait`: `status: ok` with a summary such as `20 listed, 20 new, 6 published, 14 waiting (time: …)`. Waiting articles are published by the next syncs; `waiting (translate_budget …)` by tomorrow's.
- `site status`: the source is `ok`, checked now, with articles. Look at the real thing as in the template reference: `site preview` without `--data` serves the bucket's data.
- `agent-session goal show <goal_id> --state --schedule`: active, `last_check` ran and said false, both skills are there; `agent-session goal job list --goal <goal_id>`: every job, none failing.

#### Taking over

The site outlives its maintainer: packages, template and content are in the owner's bucket. When `maintainer` names a goal that is not this Agent's (another Agent kept the site, or was removed):

1. For every source of `site source list`: `site source get <id> sources/<id>` into this workspace, then `site try sources/<id> --fixtures --no-translate`. Read its `NOTES.md`; write `SITE.md` from them.
2. `site template list`: when the version in use is not the default one of `site init` but one the earlier maintainer installed, `site template get <version> template` brings its files into this workspace, so that a later change of the layout starts from what the owner sees.
3. Create the goal and the jobs as above; the first syncs find the articles already there and add only new ones. Enter the new maintainer.
4. The old goal is not yours to archive. While it lives, both sync every source and both spend on translation: say in your report, first, that the owner has to archive it with the Agent that owns it.

The template in use stays in use; nothing is installed again for a take-over.
