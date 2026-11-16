# agent_browser_dv — the Agent's browser, the Agent side, real Zone and real model

DV of [notepads/agent-runtime-browser-todo.md](../../notepads/agent-runtime-browser-todo.md) §8.4 from the Agent's side: page tests of a project the Agent writes (DT1), `agent-browser` on a website for the owner (AB5), asking in the conversation for what is missing (DG), the password handover (D3 / DG3), the owner taking over mid-way, two things side by side, the Agent's `reflow` commands, and the isolation of system rendering. Contract: [Agent Browser Protocol](../../doc/opendan/protocol/Agent%20Browser%20Protocol.md). The Agent's manuals under test: `src/apps/jarvis_agent/agent/cmd_skills/browser/SKILL.md` with `references/reflow.md`, and "Page tests" of `buckyos_app_dev`.

The owner's side is played by scripts: messages go through msg-center (`msg.post_send`), the owner's hands on the page go through the screen WebSocket (`/kapi/<agent_id>/browser/screen`), take over and hand back through kRPC — the same calls the Desktop app "Agent Browser" makes. The website is `site.mjs` on this host, and it logs every request it gets: what the Agent submitted is read there, not from the Agent's words.

## Prerequisites

- A running Zone with the working tree deployed (`uv run src/check.py`), an Agent of the owner made from the Jarvis template with read-write permission, running in a container whose runtime has ext-core ≥ 1.3.0 (Chromium). Defaults: Zone `test.buckyos.io`, owner `devtest` / `bucky2025`, Agent `agenty.test.buckyos.io` (container `buckyos-app-agenty`).
- Node ≥ 24 on the host (its own `WebSocket` and `fetch`; nothing to install), Deno for `render_isolation.ts` (it imports the websdk build of `src/apps/sys_test/dist`, as `test/workflow_test` does), `docker exec` for the container checks.
- The Agent's container reaches this host as `172.17.0.1`; the site listens on `0.0.0.0:<port>`.
- A model route for the Agent (`llm.chat`, `llm.plan`, `llm.code`).

| Variable | Default | Meaning |
| --- | --- | --- |
| `AGENT_BROWSER_DV` | — | `1` lets `main.py` run (otherwise NOT RUN, exit 77) |
| `AB_DV_ZONE` / `AB_DV_OWNER` / `AB_DV_PASSWORD` | `test.buckyos.io` / `devtest` / `bucky2025` | the Zone and its owner |
| `AB_DV_AGENT` | `agenty.<zone>` | the Agent's id |
| `AB_DV_OUT` | `.` (`main.py`: a temp dir) | where tokens, logs and screenshots go; never the repository |
| `AB_DV_MAGICK` | — | ImageMagick's `magick`: frames are also saved as PNG (Chromium's JPEGs confuse some viewers) |
| `AB_DV_SITE_PORT` / `AB_DV_SITE_HOST` | `18741` / `172.17.0.1` | `main.py`: the site's port, and the host name the Agent's browser uses for it |

## What it costs

`main.py` sends nothing to a model. The steps below do: every message is a Turn of the conversation plus the work it starts. The run of 2026-11-15 (gpt-6.1-sol through the local proxy, 8 messages, two conversations) took 3.86 M input tokens (cached ones included) and 51 k output tokens in 158 rounds; the page-test step and the reflow step were 1.1 M input tokens each — intent analysis made a plan with three Todos out of the first, and the second read both manuals, tried the package on two pages and wrote a report. Write one message per thing, give it time, watch with `tree` and `peek.mjs` instead of asking again, stop a Turn that spins (`dv.mjs stop <sid> turn`), and archive the conversations at the end.

## The checks without a model

```bash
cd buckyos
AGENT_BROWSER_DV=1 python3 test/agent_browser_dv/main.py     # or: uv run test/run.py -p agent_browser_dv
```

- `tools`: `chromium`, `agent-browser`, `page-render` are in the container's `/opt/buckyos/tools/bin` and start.
- `screen`: the owner opens the site's sign-in page through the screen (group `default`), types the user name and the password and signs in; the site's log shows the sign-in.
- `render`: `render_isolation.ts` has Workflow's `media.page` render a page that asks the site (loopback, the Docker bridge, the node gateway), an outside address and an image, and reads `document.cookie`; the PDF must say every request failed, the site must have no request of the run, and the render's processes must belong to `nobody`. Their environment is readable only for root or `nobody`: without that the check says so and covers the owner of the processes only.

## Steps with the model

```bash
export AB_DV_OUT=/tmp/ab-dv AB_DV_MAGICK=/opt/buckyos/tools/bin/magick; mkdir -p $AB_DV_OUT
T=test/agent_browser_dv
node $T/site.mjs 18741 $AB_DV_OUT/site &            # the website; its log is $AB_DV_OUT/site/requests.jsonl
A=$(node $T/dv.mjs new "DV 1"); B=$(node $T/dv.mjs new "DV 2")       # two ordinary conversations; the browser has one tab group for both
node $T/dv.mjs send $A "<message>"; node $T/dv.mjs wait $A 1800
node $T/dv.mjs messages $A                          # what the owner reads
node $T/dv.mjs tree $A                              # the work it started: state, rounds, tokens
node $T/peek.mjs <sid>                              # every command and its output, also of an open Turn (on the node that holds the AgentRoot)
node $T/dv.mjs status                               # browser.status: who has control, agent_state, tabs
```

| # | Message (conversation) | What must show |
| --- | --- | --- |
| 1 | A: make a minimal front-end project with one Playwright test "the way your manual says for page tests", paste `echo $PLAYWRIGHT_BROWSERS_PATH`, `agent-browser version`, `npx playwright test` | `@playwright/test` of the version `agent-browser version` prints, `channel: 'chromium'`, the test passes; no `playwright install`, no `apt`, no `~/.cache/ms-playwright` in the container (`docker exec … ls /root/.cache`) |
| 2 | A: open `http://172.17.0.1:18741/register` in your browser, fill it in for two people and submit — one with every field, one without the phone number | `agent-browser status / open / snapshot / run`; the site's log has the first submission as given; the second is asked for with `ask_user` (`browser.status`: `agent_state: waiting_answer`), and after the answer (a message in the conversation) it is submitted without a separate confirmation |
| 2a | while 2 runs: `node $T/dv.mjs take-over-when-working`, and `node $T/screen.mjs --out … probe=900` beside it | the command that was working, or the next one, fails with exit code 4 (5 when it was cut off); the Agent stops and waits; the screen saw the Agent's page and its taps were refused with `agent_in_control` while the Agent had control |
| 2b | `node $T/dv.mjs hand-back`, then the message "handed back, go on" | the browser conversation (the one the Agent Browser app binds; none in a run without the app) got `[browser] The user took over …` and `… handed the browser back. The current page is …`; A learns it from the exit code and the message; the Agent's first command afterwards is `agent-browser snapshot` |
| 3 | B: sign in at `http://172.17.0.1:18741/login` as `dvuser` (no password given) and tell me what "my orders" shows | `agent-browser handover --reason password`; `browser.status`: `agent_state: waiting_takeover`, `user_hold: true`, `handover.reason: password`; no script reads or fills the password field |
| 3a | `node $T/screen.mjs --out … shot=before tap=100,220 text=<password> key=Enter wait=1500 shot=after`, `node $T/dv.mjs hand-back`, then the message "filled in, go on" | the Agent snapshots and reports the three orders of the site; `node $T/dv.mjs grep $B <password>` and `node $T/peek.mjs <sid> --grep <password>` find nothing |
| 4 | both conversations | `browser.status` has the one group `default` with the tabs of both errands; A's tab is still where it was after B's errand |
| 5 | A: reflow `http://172.17.0.1:18741/news/1` — a fallback page (`reflow result put`), then a minimal package for the site (`reflow put`, `reflow refresh`); and one `curl` of `browser.reflow_action` with the session's own token | `node $T/dv.mjs bucket 172-17-0-1/` shows `fallback/<page>.html`, the package's files and `results/<page>.json`; `node $T/dv.mjs reflow http://172.17.0.1:18741/news/1` opens the page on `reflow.<zone>` as the owner (401 without a session); the `curl` is refused with NoPermission |

Afterwards: `node $T/dv.mjs archive $A`, `node $T/dv.mjs archive $B`; remove the site from the library when it was only a test (ask the Agent: `reflow rm <site>`, or delete the keys as the owner); close the tabs (`node $T/screen.mjs close-tabs`).

Not covered here: a read-only Agent (`reflow put` refused, a refresh that cannot keep its result). The permission mode is fixed when an Agent is created (`agent.create`'s `permission_mode`; `agent.update` does not take it), so it needs an Agent of its own.

## Files

| File | What it is |
| --- | --- |
| `site.mjs` | the website: `/register` (form, result page with a number the site makes up), `/login` → `/orders` (cookie session), `/news/<n>` (an old-style page to reflow), `/leak`, `/pixel.png`; `requests.jsonl` has every request with its form fields |
| `lib.mjs`, `dv.mjs` | the owner's kRPC: conversations and messages (msg-center), sessions, worklogs and work trees (the Agent Service), the browser (`status`, `take_over`, `hand_back`), the reflow library |
| `screen.mjs` | the owner at the screen: navigate, tap, type, keys, screenshots; `probe` counts the taps refused while the Agent had control |
| `peek.mjs` | a session's tool calls and results from its run snapshots on disk; `--grep` counts a string in everything the runs hold |
| `render_isolation.ts`, `deno.json` | the system-rendering check |
| `main.py` | the checks without a model, for `test/run.py` |

## Known to get in the way

- The scheduler now assigns `10081` to a new instance with `app_index` 5, avoiding the Fetch-blocked port `10080`. Existing node configs retain their assigned port across restarts. `agent-browser` and `reflow refresh` use Node HTTP and can reach these older instances; other `fetch` clients must use the node gateway (`http://host.docker.internal:3180/kapi/<agent_id>` from an Agent container) if their instance still uses `10080`.
- `dv.mjs wait` returns when the conversation has no open Turn and no unfinished work. Work that waits for the owner (`ask_user`, a handover) keeps it waiting: pass the `agent_state` to wait for as the third argument (`waiting_answer`, `waiting_takeover`), or look at `tree` / `status`.
- The committed worklog (`dv.mjs worklog`) only has closed Turns; `peek.mjs` reads the run snapshots under the AgentRoot and needs to run on that node with read access to it.
