# work_simple_dv — asynchronous simple Work Session, real model

Real-model DV of [notepads/done/opendan-simple-worksession-async-todo.md](../../notepads/done/opendan-simple-worksession-async-todo.md) §6: a `work_simple` session of the Jarvis package of this checkout builds two slow crates (`fixture/alpha`: build script 60 s, `fixture/beta`: 100 s) in parallel. The model is real; tools, tasks, Turns and ticks go through the real `xagent`.

The mock-model counterpart is `cargo test -p libopendan --test work_simple_async -- --test-threads=1` in `src/`.

What a run shows (each is a check of `verify`):

- both `shell` calls outlive the 25 s in-call wait and continue as tasks (`local:shell:<call_id>`);
- the model closes Turns with progress replies (worklog `outcome kind=wait` with the reply) and the session keeps the run;
- a tick (`timer:task_watch`, every 30 s from `[session.work] tick_secs`) opens Turns while the builds run; the new build output is in `<<background_context>>`;
- each build's end comes back as `<<task_result … status="finished">>` with the whole result;
- the model never calls `wait_task` / `agent-session task wait`;
- calls carry `intent`; the session ends with `report(is_end=true)`.

## Run

```bash
cd buckyos
python3 test/work_simple_dv/main.py all --variant normal    # one xagent run to the end
python3 test/work_simple_dv/main.py all --variant restart   # kill the driving process after Turn 1, drive again
```

Phases on one case: `prepare --case <dir>`, `run --case <dir>`, `verify --case <dir>`.

| Variable | Default | Meaning |
| --- | --- | --- |
| `WORK_SIMPLE_DV_BASE_URL` | `http://127.0.0.1:8317/v1` | OpenAI-compatible endpoint (the local CLIProxyAPI) |
| `WORK_SIMPLE_DV_API_KEY` | `API_KEY=` of `/opt/cli-proxy-api/keys.env` | credential (never written to the evidence) |
| `WORK_SIMPLE_DV_MODEL` | `gpt-6-sol` | model id replacing the package's `llm.*` aliases |
| `WORK_SIMPLE_DV_CASE_ROOT` | `/home/lzc/project/work_simple_dv_runs` | where case directories are created |
| `WORK_SIMPLE_DV_TIMEOUT_SECS` | `1500` | run time limit |

The `restart` variant kills `xagent run` with SIGKILL once Turn 1 has closed while a build runs (the commands survive: they run in their own process groups and record their end in the execution directory). The second `xagent run` is a new process whose task manager does not know them: a build that ended is read from its execution directory (`exited with code … started by a previous executor`), one still running comes back as `status="unknown"` and the model decides.

Evidence in the case directory: `verify.json` (checks, inferences, tokens, Turns, ticks), `evidence.json` (every input the model got, every call with its intent, results, outcomes), `drive*.json` / `drive*.stderr.log`, the session under `sessions/`.
