# The owner's reading site in a conversation

`site status` first: the sources with their states, open upgrade requests, and `maintainer` — the goal that keeps the site (`goal_id`) and its Agent. `site --help` has every command. A value given to `site config set` is JSON: text in double quotes.

#### Add a website

1. `site match <URL>`: an address a source already has is an answer ("it is in your site, under <source>"), not work.
2. A work session of this conversation that is still adding sources takes the next one: `agent-session post <sid> --msg "<the whole message>"`. One at a time: they write the same workspace and the same goal.
3. Otherwise hand the message to intent analysis, with this settled so that it asks the owner nothing about it:
   - one `work_simple` on the private runtime with the manuals `reading_site` and `goal_handover`;
   - in the site's workspace: the registered one named `reading-site` (`agent-session workspace list`). When there is none it is created without asking: `agent-session workspace create --runtime private --directory "$OPENDAN_AGENT_ROOT/workspace/reading-site" --name reading-site --description "Source programs of the owner's reading site" --key reading-site`;
   - the task: add this website as a source (`URL`, `Title`, the owner's `Note` in their words; build the site first when `site status` says there is none), and end by setting up the goal that keeps the site, or by revising the maintainer goal when it is this Agent's;
   - the report: `result.goal_id`, each source with its state, what it gives (full text, list only, behind the owner's sign-in), what failed and why.
4. Ask the owner only what changes the source: which section of a large site, when the note leaves it open.
5. Reply that it shows in the site as being set up and that you tell them when the first articles are there. When the work ends, relay what the source gives and what failed; a source the site sells or refuses to programs comes as list only, and the owner hears that from you.

#### Go ahead with an upgrade

`File` is the note the goal wrote, `Goal` its id, `Upgrade` the request's id. Read the file, then hand it to intent analysis: a work session from the note's draft of the task, in the goal's workspace, with `reading_site` and `goal_handover`, ending with `site upgrade close <Upgrade>` and the revision of goal `<Goal>`. Never start it for a request the owner has not agreed to. The owner declines in their own words ("not now", "leave it"): `site upgrade dismiss <Upgrade>` (the id is in `site status` when the message is not there), then `agent-session goal post <Goal> --msg "<their words>"`. The goal does not ask again.

#### Remove a website

The goal is `goal_id` of `site config get maintainer`. In this order, so that no sync runs for a source that is gone:

```sh
agent-session goal job rm src-<Source> --goal <goal_id>
site source rm <Source>
agent-session goal show <goal_id>
agent-session goal revise <goal_id> --expected-revision <n> --objective "<its whole text without that source>" --reason "the owner removed <Source>"
agent-session goal post <goal_id> --msg "The owner removed the source <Source>; its job and its package are gone."
```

Its articles stay readable until they expire. A maintainer that is another Agent's goal: only `site source rm`, and tell the owner that the other Agent's goal still has the job.

#### Small changes

| The owner wants | Do |
| --- | --- |
| a topic gone ("no more sports"), or back | `site config get template.params.hide_topics`, then `site config set template.params.hide_topics '["体育"]'` with the whole list; the names are the site's `topics` (`site config get topics`) |
| another title; other columns or their order | `site config set title '"…"'`, `site config set sources.<id>.title '"…"'`, `site config set columns '[{"id":"jp","title":"日本","sources":["news-yahoo-co-jp"]}]'` |
| a source paused | `site config set sources.<id>.state '"paused"'` and `agent-session goal job disable src-<id> --goal <goal_id>` |
| a source resumed | `site config set sources.<id>.state '"active"'` and `agent-session goal job enable src-<id> --goal <goal_id>` |
| fewer or more articles a sync, comments off, kept longer | `site config set sources.<id>.max_new <n>`, `sources.<id>.comments 0`, `sources.<id>.keep_days <n>`: the next sync reads them |
| another frequency | `site config set sources.<id>.every_secs <secs, at least 300>`, then the job's timetable, with the package as it is installed: `site source get <id> pkg-<id>` and `agent-session goal job put src-<id> --goal <goal_id> --dir pkg-<id> --command "site sync <id> --dir {dir}" --every <secs> --timeout 600` |
| no record of what they read | `site config set reading.enabled false` |
| the goal to watch or tell something else | `agent-session goal show <goal_id>`, `agent-session goal revise <goal_id> --expected-revision <n> --objective "<the whole new text>" --reason "<the owner's words>"`, then `agent-session goal post <goal_id> --msg "<what changed>"` |

Hidden topics are the owner's: the goal never changes them, so the owner's words are the only way. Say in one sentence what you changed. What needs a program, a template or another way of fetching is not a small change: it goes to a work session with `reading_site`.
