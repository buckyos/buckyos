# plan_do_todo_dv — the plan session (plan_simple), real model

Real-model DV of [notepads/opendan-plan-simple-async-todo.md](../../notepads/opendan-plan-simple-async-todo.md) §6: the Jarvis plan session reviews a small Python inventory-reservation service through a TodoList — it adds the whole list in its first Turn (two read-only investigations in parallel, a verification Todo after them, a Todo that writes `review.md` / `decisions.md` into the workspace after it), the host starts every Todo in a work session of its own as soon as its prerequisites completed, and the plan only steers with what comes back until it judges the review done. The model is real; files, tools, Todo scheduling and reports go through the real `xagent`.

The mock-model counterpart is `cargo test -p libopendan --test plan_simple_async` (and `--test plan_simple`) in `src/`. The former PLAN / DO mode (`next_behavior = do`) and its task are gone with the behavior loop.

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
PLAN_DO_DV=1 python3 test/plan_do_todo_dv/main.py all --variant normal        # prepare + run + verify
PLAN_DO_DV=1 python3 test/plan_do_todo_dv/main.py all --variant user-update   # the user changes investigation B while it runs
PLAN_DO_DV=1 python3 test/plan_do_todo_dv/main.py all --variant failure       # idempotency.py is missing from the fixture
# or phase by phase on one case directory:
python3 test/plan_do_todo_dv/main.py prepare --case /tmp/pd/case1 [--variant …]
python3 test/plan_do_todo_dv/main.py run --case /tmp/pd/case1
python3 test/plan_do_todo_dv/main.py verify --case /tmp/pd/case1
```

`prepare` creates a fresh case directory: an isolated AgentRoot (the Jarvis package of this checkout, with the model aliases mapped to the provider's ids — the mapping and both copies are recorded), an empty session parent and file queue, a read-only fixture copy with its hashes, `task.txt` (`task_simple.md` with the fixture path), and `llm-context.json` (provider, `tools.enabled`, time limits).

`run` registers `$CASE/workspace` (`xagent workspace import --runtime container`), creates the plan with `xagent new --class plan_simple --workspace <id> --no-run` and drives it with `xagent run <sid> --until finished --max-children 4`, then collects the evidence (also when the run failed). In the `user-update` variant it watches the plan's `todos.json` and, as soon as investigation B (the Todo whose body names `ACCEPT-B-5M8`) runs, posts the user's change to the plan's queue (`xagent post <sid> --msg …`, what the conversation does when it forwards a message): B's reproduction must also print `ACCEPT-B-EXTRA-9Z1 <tenant count>`.

Watching a run from a second terminal (same environment):

```bash
"$XAGENT" list --active
"$XAGENT" status "$SID" --worklog 200 --events
"$XAGENT" todo list "$SID"            # the plan's view; --json: Todo → attempt → child_sid → report_ref
"$XAGENT" status "$CHILD_SID" --report --worklog 200 --run
```

## Exit codes

`all` / `verify`: 0 every check passed, 1 a check failed (see `verification.json`), 2 configuration / provider error, 77 not enabled. `run` returns the exit code of `xagent run` (recorded in `run.json`).

## Evidence (`$CASE/`)

```text
create.json / drive.json / drive.stderr.log / run.json / user_update.json (user-update)
config/case.json             variant, provider (no key), model map, fixture hashes, xagent path + sha256, behavior hashes
config/behaviors.package/    the package's behaviors;  config/behaviors.case/  the ones the run froze
parent/session/              the plan session directory (todos.json, worklog, runs, …)
parent/todos.json / todo_view.xml / status.json / intents.json / backgrounds.json
children/<child_sid>/        session/, status.json, binding.json (its Todo and attempt)
timeline.json                attempts started / ended, updates, child windows (start / first round / first tool / end), the plan's batches and rounds
review.md / decisions.md     the deliverables (from the workspace)
verification.json            every check with its detail, token usage
```

## What `verify` checks

From the committed Todo list, sessions, worklogs and reports — never from words in the final report alone:

- the plan session: class `plan_simple`, `explicit_report`, an input queue, its workspace binding read-only; every Turn closed; no behavior steps; every call with a non-empty intent (`parent/intents.json`); no `todo dispatch`, `todo review`, `agent-session wait`, no `sleep`, no writing commands;
- the whole list in the first Turn (≥ 4 Todos, ≥ 3 in the failure variant); ≥ 2 explorations with distinct work sessions whose windows overlap; work Todos in `work_simple`; explorations without a workspace;
- every Todo with prerequisites started before the plan's next input batch after the last of them completed (the host schedules, the plan does not); a verification Todo after the investigations;
- normal / user-update: the deliverables in the workspace written by a Todo that inherited it, naming `inventory.py` and `idempotency.py` (tenant), tracing the investigations' Todo ids; the markers A / B / C in the work sessions' output; the plan succeeded;
- user-update: the message was posted, B was updated (`updates`), the update reached its work session (an `#updated@` input) or B was retried, and `ACCEPT-B-EXTRA-9Z1` is in B's output or report;
- failure: the final report or `review.md` says what is missing; a verification Todo exists;
- Turns ≤ work sessions + user messages + ticks + 3; the list summary recorded at the end (`control_applied{todo_summary}`); ≥ 2 background blocks; provider refusals / 403s recorded (informational).

## Runs

Recorded runs, with the manual review, are in the TODO's implementation record: [opendan-plan-simple-async-todo.md](../../notepads/opendan-plan-simple-async-todo.md) §8.
