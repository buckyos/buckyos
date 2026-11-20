---
name: aiworkspace_run
description: Execute a wish run of an AI Workspace that the user who owns you handed to you (a Goal naming a run and its claim token) — claim it, analyze the task, or write and trial-run the wish's macro, or run the stored macro; hand it in with what your model used, keep the lease alive — take a stage's outcome back (`run-rollback`), start an analysis run yourself when you worked out a wish's task in a conversation (such as one from the workspace's Agent panel, whose messages carry an `[aiworkspace context]` block), start a wish's stages or refresh a sourced table through the workspace's objects (`agent_tool xcall`, after asking the user), and read a workspace's tables and documents.
requires: buckyos signed in as this Agent (container /opt/buckyos/bin/agent-tools/buckyos, host $BUCKYOS_ROOT/bin/buckyos); the owner's hosted run and claim token, or permission to start a stage. Analysis requires update on the wish. For code and run, the effective macro identity needs update on the wish and write access where results go.
baseline: standard
commands: jq
---
#### Claim and keep the lease
A hosted Goal supplies `WS`, `RUN` and `TOKEN`. For an analysis agreed in conversation, `buckyos aiws run-start --workspace "$WS" --wish <wish_id> --stage analyze` returns the run and claim token. Inspect `run-get` for the stage, executor and trigger. Run from the session directory, keeping the claim outside the macro's `work/`:

```bash
buckyos aiws run-claim --workspace "$WS" --run "$RUN" --claim-token "$TOKEN" > claim.json
attempt=$(jq -er .attempt claim.json)
jq -r .stage.user claim.json > task.md
jq -r .stage.system claim.json > rules.md
```

Use `--workspace "$WS" --run "$RUN" --attempt "$attempt"` on each subsequent executor command. Call `buckyos aiws run-heartbeat` with these options at least every minute, within `lease_ms`. Stop on `RUN_REVOKED`. Claim again only when `run-get` says `interrupted`.

#### Execute the assigned stage
Read `task.md` for snapshot inputs, output contract, checks and fixed parameters. Follow `rules.md` for schemas, stage tools and the macro guide.

- **Analyze.** Inspect the snapshot with `run-tool --name ws_profile|ws_read|ws_find|ws_query`. Submit `wish.analysis.v3` with `run-tool --name submit_analysis --args-file analysis.json`. Bind existing inputs and give every check a level (`error` blocks delivery, `warning` allows it). For missing inputs submit `status: needs_input` with blockers. For external data follow the owner rule in the stage reference.
- **Code.** Fetch `run-files --dir work`, write `work/program/main.js`, then `run-program --dir work`. Read its results and `run-tool --name check_results`. Correct errors and repeat until the trial passes, then `run-tool --name finish --args '{"summary":"…","assumptions":[],"warnings":[]}'`. This stage stores only the macro.
- **Run.** Fetch `run-files --dir work` and execute the unchanged macro with `run-program --dir work`. Use the tool's sandbox for all macro execution. The service writes results, overwriting previous edits and removing obsolete rows or fields.

End with `run-report --finished`, or `--error "<reason>"` if the work cannot be done. Report the last turn's model (`--model`), totals over the stage (`--input-tokens`, `--output-tokens`) and number of model turns (`--calls`). Omit unknown values. Omit them for a stored-macro run. Verify `buckyos aiws run-get --workspace "$WS" --run "$RUN"` reaches `succeeded` with `applied.commit` and `record`. The service commits automatically.

#### Read for the task
- Analysis blockers, macro APIs, controls, checks and failures: `$OPENDAN_AGENT_ROOT/cmd_skills/aiworkspace_run/references/stages.md`.
- Panel context, object actions and rollback: `$OPENDAN_AGENT_ROOT/cmd_skills/aiworkspace_run/references/actions.md`. Ask before calling a write action. Roll back only on request and obtain agreement before `--confirm`.
