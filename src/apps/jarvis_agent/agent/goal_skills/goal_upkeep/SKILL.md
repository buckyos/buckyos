---
name: goal_upkeep
description: Keep a goal cheap and working by its own record — read what it did and spent (`agent-session goal stats`), then tighten the check, thin out the timetable, move repeated judgement into a script, change the way it works or what its activations remember, and get help when it keeps failing. Read it when a goal is woken for nothing, its budget runs short, the hard guarantee woke it, or activations or work sessions fail in a row.
requires: the goal's own commands (`agent-session goal stats|check`)
---
# Keeping a goal cheap and working

A goal pays for every activation. Its job is to hold its state with as few of them as that takes, and to stop doing what does not work. This is upkeep of yourself, not of what you look after: do it first, then what the activation was for.

## Read the record

`agent-session goal stats` gives one entry per statistics period (the last one is not over): `activations` with their `decisions` and `reasons`, `tokens_by_goal`, `tokens_by_works`, `tokens_by_jobs`, `tokens_per_activation`, the checks (`checks_true`, `checks_false`, `checks_error`), the jobs (`job_runs_ok`, `job_runs_failed`), what the gate skipped. `--periods 14` looks further back.

| What it shows | What it means | What to change |
| --- | --- | --- |
| many `no_action`, `checks_true` close to `activations` | the check lets too much through | compare with what you saw last and return `true` only for a change you would act on |
| `checks_true` = `checks_false` + `checks_true`, or no script | the timetable wakes you, not a condition | write a check script (goal skill `goal_check_script`) |
| the same small decision again and again | judgement that became a rule | put the rule into the check script (it decides, and notifies by itself), or into a job where something has to be done |
| checks mostly `false` and nothing ever close to due | checked more often than the thing moves | a sparser timetable: `goal check set --every <secs>` or `--cron` |
| `tokens_per_activation` grows while the work stays the same | you read too much, or carry too much | read less (names, sizes, dates first); `--context fresh` |
| `tokens_by_works` far above `tokens_by_goal` | work sessions that search again what you know | give them the facts, paths and earlier reports; smaller tasks |
| `skipped_by_budget` | the period's budget is not enough for how often you run | check less, do less per activation; if it is really needed, say in your report what budget it takes |
| `checks_error`, `hard_guarantees` | the script is broken or the source is down | `agent-session goal check show`, fix and test it |

Change one thing at a time and write into the report what you changed and what you expect, so the next look at the numbers can tell. Two periods without a better number: take it back or try another.

## Ways of working

| What the activation has to do | How |
| --- | --- |
| a few steps you can check at once | do it yourself |
| one thing that needs a context of its own | a `work_simple` |
| finding out first and then acting, or several things side by side | a `plan_simple` (it lives 48 hours at most) |
| routine without judgement | the check script (looks, tells) or a job (does); no inference — goal skill `goal_check_script` |

After creating a work session the activation ends; read its report when it comes back. Never wait for it, never follow a long command from here: every round of waiting pays for the whole context again.

## What an activation remembers

`context="fresh"` (the usual): an activation starts with its header. Leave what the next one needs in the report (its text comes back as `<last_activation>`, up to 2000 characters of it), `distance`, `pending`, the check's `state`, or a file you name there. A list, a table, a longer thread belongs in a file in the workspace or in this session's directory, with its path in `pending`.

`agent-session goal check set --context carry --reason "<why>"` when activations really build on each other's details — a negotiation, an investigation over several days. Every round then pays for that history, so go back to `fresh` when the thread ends.

Looking up the same thing in every activation is a sign that `fresh` misses something: write it down once.

## When it keeps failing

`activations_failed_in_a_row` and `works_failed_in_a_row` in `<state>` count it; at three failed activations the user gets a notification that you keep failing.

1. Find out what failed: `<last_error>`, the report of the failed work (`agent-session read-session <sid> --report`), `goal show --activations 5`.
2. The same approach failed twice: do it another way — another source, a smaller step, another tool. Never a third time.
3. The other way failed too: ask the user in `questions`. Say what you tried, where it is stuck and what exactly you need from them; offer `options` when the answers form a short list; name who else could help if you know.
4. What has to change is what the user sees, what the system may do or what it costs: ask for an upgrade (goal skill `goal_delivery`).
5. The user declined or does not care: thin out the timetable, keep one line in `pending`, do not raise it again until the facts change.
6. An activation that runs out of budget does too much: split it, or hand the long part to a work session.
