# plan_do_todo_dv — TodoList-driven PLAN / DO and plan_simple, real model

Real-model DV of [notepads/opendan-plan-do-todo-list-todo.md](../../notepads/opendan-plan-do-todo-list-todo.md) §4–§6: the Jarvis PLAN reviews a small Python inventory-reservation service through a TodoList — two explorations in parallel as their own sub sessions, a wait that is no decision, a review of each report, then a serial verification Todo in DO and the final `review.md` / `decisions.md`. The model is real; files, tools, Todo scheduling and reports go through the real `xagent`.

The mock-model counterpart (state machine, recovery, concurrency, CLI) is `cargo test -p libopendan --test plan_do_todo` in `src/`.

## Prerequisites

- Rust toolchain (the run builds `xagent` from this checkout: `cargo build --manifest-path src/Cargo.toml -p libopendan --bin xagent`).
- An OpenAI-compatible endpoint with tool calling. Default: the local CLIProxyAPI at `http://127.0.0.1:8317/v1` with its key from `/opt/cli-proxy-api/keys.env` (`API_KEY=`).

| Variable | Default | Meaning |
| --- | --- | --- |
| `PLAN_DO_DV` | — | `1` enables the case when `main.py` runs without arguments (`test/run.py -p plan_do_todo_dv`); otherwise it reports NOT RUN (exit 77) |
| `PLAN_DO_BASE_URL` | `http://127.0.0.1:8317/v1` | provider endpoint |
| `PLAN_DO_API_KEY` | from `/opt/cli-proxy-api/keys.env` | credential (never written to the evidence) |
| `PLAN_DO_PLAN_MODEL` / `PLAN_DO_CODE_MODEL` | `gpt-6-sol` | models that replace the package aliases `llm.plan` / `llm.code` (and `llm.chat`) |
| `PLAN_DO_CASE_ROOT` | `$TMPDIR/plan_do_todo_dv` | where case directories are created |
| `PLAN_DO_TIMEOUT_SECS` | `3600` | total run time limit (exceeded = failed) |

A missing credential, an unreachable endpoint or a model the endpoint does not offer fails explicitly (exit 2). Nothing falls back to a mock.

## Run

```bash
cd buckyos
PLAN_DO_DV=1 python3 test/plan_do_todo_dv/main.py all --variant normal      # prepare + run + verify
PLAN_DO_DV=1 python3 test/plan_do_todo_dv/main.py all --variant failure     # idempotency.py is missing from the fixture
# or phase by phase on one case directory:
python3 test/plan_do_todo_dv/main.py prepare --case /tmp/pd/case1 [--variant failure]
python3 test/plan_do_todo_dv/main.py run --case /tmp/pd/case1
python3 test/plan_do_todo_dv/main.py verify --case /tmp/pd/case1
```

`prepare` creates a fresh case directory: an isolated AgentRoot (the Jarvis package of this checkout, with the model aliases mapped to the provider's ids — the mapping and both copies are recorded), an empty session parent and file queue, a read-only fixture copy with its hashes, `task.txt` (this directory's `task.md` with the fixture path), and `llm-context.json` (provider, `tools.enabled`, time limits).

`run` executes exactly:

```bash
export OPENDAN_AGENT_ROOT="$CASE/agent" OPENDAN_AGENT_DID="did:bns:jarvis.todo-test" \
       LIBOPENDAN_WHO="app:xagent-test@local" LIBOPENDAN_QUEUE_DIR="$CASE/kmsg" PLAN_DO_API_KEY=…
"$XAGENT" new --class work --behavior plan --objective "$(cat "$CASE/task.txt")" \
  --parent "$CASE/sessions" --llm-context "@$CASE/llm-context.json" --no-run > "$CASE/create.json"
"$XAGENT" run "$SID" --until finished --max-children 4 --max-wait 3600 \
  > "$CASE/drive.json" 2> "$CASE/drive.stderr.log"
```

then collects the evidence (also when the run failed). `$XAGENT` is the binary `prepare` built (path and sha256 in `config/case.json`).

Watching a run from a second terminal (same environment):

```bash
"$XAGENT" list --active
"$XAGENT" status "$SID" --worklog 200 --events
"$XAGENT" todo list "$SID"            # the PLAN's view; --json: Todo → attempt → child_sid | run_id → report_ref
"$XAGENT" status "$CHILD_SID" --report --worklog 200 --run
```

## plan_simple mode

`--mode plan_simple` runs the same review with the Jarvis `plan_simple` behavior ([notepads/opendan-plan-simple-todo.md](../../notepads/opendan-plan-simple-todo.md) §8.2): native function calls with an `intent` on every call, one Turn, every Todo in a sub session of its own, waits as `agent-session wait` calls, the background after tool results. The task is `task_simple.md`; it names the deliverables' place (the session's workspace) but not how Todos get their runtime / workspace, so the plan's own assignment is checked.

```bash
PLAN_DO_DV=1 python3 test/plan_do_todo_dv/main.py all --mode plan_simple --variant normal
PLAN_DO_DV=1 python3 test/plan_do_todo_dv/main.py all --mode plan_simple --variant failure
```

`run` registers `$CASE/workspace` (`xagent workspace import --runtime container`) and creates the session with `xagent new --class plan_simple --workspace <id>`. `verify` then checks (`verify_simple`): the `plan_simple` class with `explicit_report`; one `turn_started` / `turn_ended`; no behavior steps and no DO; every tool call with a non-empty intent (calls refused for a missing one are counted), intents recorded in `parent/intents.json`; waits as standalone `agent-session wait <sid>` calls, no polling; ≥ 2 explorations with overlapping windows; ≥ 2 serial Todos each in its own `work_simple` sub session, one at a time, after their prerequisites were accepted; explorations without a workspace and the deliverable writer with the inherited one; `review.md` / `decisions.md` in the workspace, tracing findings to Todo ids and sub session ids; every latest result reviewed after a report read; ≥ 2 background blocks (`parent/backgrounds.json`) without child output; markers A / B / C; provider refusals / 403s recorded (`provider.refusals_recorded`, informational).

## Exit codes

`all` / `verify`: 0 every check passed, 1 a check failed (see `verification.json`), 2 configuration / provider error, 77 not enabled. `run` returns the exit code of `xagent run` (recorded in `run.json`).

## Evidence (`$CASE/`)

```text
create.json / drive.json / drive.stderr.log / run.json
config/case.json             variant, provider (no key), model map, fixture hashes, xagent path + sha256, behavior hashes
config/behaviors.package/    the package's behaviors;  config/behaviors.case/  the ones the run froze
parent/session/              the parent session directory (todos.json, worklog, runs, reports, review.md, …)
parent/todos.json / todo_view.xml / status.json / views.json (the TodoList view of every decision boundary)
children/<child_sid>/        session/, status.json, binding.json (its Todo and attempt)
serial/<run_id>/             sub_task.json (DO's input), process_done.json, report.json, binding.json
timeline.json                dispatches, child windows (start / first round / first tool / end), waits, reviews
review.md / decisions.md     the parent's final deliverables
verification.json            every check with its detail, token usage
```

## What `verify` checks

From the committed Todo list, sessions, worklogs and reports — never from words in the final report alone:

- two or more explorations with distinct sub sessions, each with real model rounds and tool calls; their execution windows overlap (the second had begun tool execution before the first ended);
- each exploration's first input carries its Todo-only acceptance marker (`ACCEPT-A-7Q2`, `ACCEPT-B-5M8`) and its tool output prints it; the serial Todo's `<sub_task>` carries `ACCEPT-C-3K9`, reviewed-result references, and the output prints it;
- the parent waited by not deciding (`outcome.kind = wait`), was woken by `_child` inputs, never polled (`sleep`, `agent-session wait`);
- every exploration was reviewed; the serial Todo started after the reviews; the list converged; the session succeeded;
- the PLAN's context never held a child's tool output, and a child report's body appeared first as the result of `todo show … --report` / `read-session`; the views hold only the compact fields;
- `review.md` names `inventory.py` and `idempotency.py` (tenant) and traces every accepted result to its Todo and execution; `decisions.md` exists.

The failure variant instead checks that the branch without its input was rejected / needs evidence or followed up, and that the outcome says so.

## Runs

Recorded runs, with the manual review, are listed in the TODOs' implementation records: PLAN / DO in [opendan-plan-do-todo-list-todo.md](../../notepads/opendan-plan-do-todo-list-todo.md) §8, plan_simple in [opendan-plan-simple-todo.md](../../notepads/opendan-plan-simple-todo.md) §10.4.
