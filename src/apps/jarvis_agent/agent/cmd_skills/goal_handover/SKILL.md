---
name: goal_handover
description: Close work that built something by setting up, or revising, the goal that operates it from then on — its definition, a tried-out check script, its skills — or fold the duty into a goal there is. For a work whose task says it ends with a goal.
requires: an OpenDAN work session that a conversation started itself (agent-session on PATH; a Todo's or a goal's work may not change goals)
baseline: standard
scope: work
---
#### What is handed over

Your task built something that somebody has to look after. From now on a goal operates it: it wakes by a check, keeps the thing working, tells the user what matters and asks for an upgrade when more is needed. You leave it able to do that without you: it never sees this context.

1. `agent-session goal list`; `goal show <id>` of any that comes near. Goals are scarce: a duty one of them can take goes there (revise it, below). Your task names the goal to revise when it knows it. Two goals that contradict each other: do not choose — say it in the report and leave the old one as it is.
2. Write down, in the workspace, how to operate what you built: what runs where, what it looks like when it works, how to check it, how to repair the likely failures (first in the workspace, then install: the exact commands that install it again, `goal job put …`, `goal check set …`), what is not operation but an upgrade. A short file, for example `operations.md`. The goal gets it as a skill: `--skill <name>=<absolute path of the file>`.
3. What has to run again and again without judgement (fetch, rebuild, back up) becomes a job of the goal, its program in a directory of the workspace; see "Jobs" in `$OPENDAN_AGENT_ROOT/goal_skills/goal_check_script/SKILL.md`. It is installed once the goal exists.
4. Write the check script the goal starts with, so that its first run is a check and not an inference. How: `$OPENDAN_AGENT_ROOT/goal_skills/goal_check_script/SKILL.md`. What has to be told at known times the script sends itself. How results reach the user: `$OPENDAN_AGENT_ROOT/goal_skills/goal_delivery/SKILL.md`.
   Try it before there is a goal: `agent-session goal check run --script <check.ts> --every <secs> --workspace <id>` (or `--cron … --timezone …`, `--path …`): the same Deno, permissions and `ctx`, with no earlier `state`. Make the condition true once if you can.

#### A new goal

```sh
agent-session goal create --title "<short title>" --lifecycle maintain \
  --objective "<what it operates and what for, with the paths, ids and URLs>" \
  --healthy-state "<what is true when all is well>" --distance "<what to look at to judge it>" \
  --constraint "<limit>" --workspace <workspace_id> --skill <name>=<absolute path of operations.md> \
  --check <check.ts> --every <secs> --key <work session id>:goal
```

- Wall-clock times instead of `--every`: `--cron "<m h dom mon dow>" --timezone <iana>`.
- Then its jobs: `agent-session goal job put <job_id> --goal <goal_id> --dir <directory with the program only; its data by absolute path> --command "<program> <args>" --every <secs ≥ 300>`; `agent-session goal job run <job_id> --goal <goal_id> --wait` shows that it works.
- No `--start`: with a check script the goal starts with a check.
- The objective stands on its own. It says what operation covers, what the user wants to hear about and how often, and what is an upgrade. "May send work to change code" only if the user wants that.
- Leave the budget out unless your task gives one.

#### Revising a goal

`agent-session goal show <id>` for its revision, then `agent-session goal revise <id> --expected-revision <n> --reason "<what changed>" [--objective …] [--healthy-state …] [--constraint …]… [--skill <name>=<path>]…` with the whole new text of each field you give (constraints: the whole new list; skills are added). A conflict: read it again and redo. A job: `goal job put` again with the same id (a new version runs at once; its data stays). Its check: `agent-session goal check set <id> --script <check.ts> --every <secs> --reason "<why>"`. The goal sees what changed at its next activation; `agent-session goal post <id> --msg "<what to do about it>"` wakes it when that cannot wait. A changed `--workspace` takes effect after `agent-session goal restart <id> --reason "<why>"`.

Folding a goal into another: revise the one that stays so that its objective and its check cover both, then `agent-session goal archive <other id> --reason "merged into <id>"`.

#### Before you report

- `agent-session goal show <id> --state --schedule`: active, the check ran (`last_check`), the skills are there; its jobs ran (`goal job list --goal <id>`). Its host runs the first check within a minute.
- Nothing half done stays behind: when what you built does not work as it was asked for (a stand-in — a command you wrote in place of the one named, made-up data — does not), or the goal cannot be set up, `agent-session goal archive <id> --reason "<why>"` the one you created, and report the failure with what is missing.
- The report's `result` carries `goal_id` (and what else the task asked for); the text says what the goal does from now on, how often it checks, and when the user hears from it.
