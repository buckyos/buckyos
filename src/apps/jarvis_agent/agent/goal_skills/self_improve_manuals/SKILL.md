---
name: self_improve_manuals
description: Duty `manuals` of the self improvement goal — read how recent work went with and without manuals, then revise a manual of the agent's own, draft a replacement or a new one for the owner, or recommend a cleanup. Read it when a reason names `manuals`, or when the owner asks about the agent's manuals.
requires: the self improvement goal's session (`agent-session skill list|revise`, `read-session`, `notify send`)
---
# Keeping the manuals in step with the work

The material is how work really went: the work sessions that were given a manual, and the ones that were given none. The output is small and checked: a better text for a manual, a draft for the owner, or a recommendation. Changing nothing is a valid result.

## Read the signals

- `cat $OPENDAN_AGENT_ROOT/.meta/skills_state.json`: `usage["work:<name>"]` = `{frozen, last_ms, recent_sessions}` for every manual given to work, and `usage["none"]` for work created without any manual. `recent_sessions` are session ids, newest last.
- `agent-session skill list --all --stats --json`: every skill with its `origin` (`package` | `installed` | `imported` | `local`), scope, size, whether it is off, and its usage.
- `ls $OPENDAN_AGENT_ROOT/skills_drafts/`: drafts already waiting for the owner. Do not draft the same thing twice.

Pick what to read, at most 6 sessions per activation: the newest ones you have not read yet (`pending` and the report of the last activation name the ones read) — a few of one manual, or a few from `usage.none`. Read each with `agent-session read-session <sid> --report --worklog 40`: its objective, what it ran, where it failed or went around, how it ended. Read the manual it was given (`$OPENDAN_AGENT_ROOT/cmd_skills/<name>/SKILL.md`).

## Judge from what the sessions show, never from the manual's text alone

- Did the work use the manual's commands and steps, or ignore them?
- Where did it lose rounds: a command or option the manual gives that no longer exists, a missing step it had to find out, a pitfall met again, a check the manual does not name?
- Did the manual lead it wrong — the work followed it and failed, or did something the task did not ask for?
- For work without a manual: is the same kind of work there three times or more, with steps that were found out again each time?
- Is the same sequence of commands composed again and again? That is a tool to make (goal skill `self_improve_tools`), not a longer manual: keep it in `pending` when you do not make it now.

One session is an incident, not a pattern: act on what at least two sessions show, or on one where the manual's text is plainly wrong (a command that does not exist, checked with `command -v` / `--help`).

## Decide: at most one change per manual, at most two per activation

- **Revise** a manual whose origin is `local` or `imported`, when only its body or `description` has to change. Write the complete new SKILL.md to a file in your working directory, keeping the front matter's `scope`, `commands`, `baseline` and `platforms` exactly as they are, then `agent-session skill revise <name> --file <new SKILL.md> --reason "<what the sessions showed: session ids and the fault>"`. The host keeps the old text under the skill's `revisions/`. Change what the sessions support and nothing else; keep the manual's size or make it smaller.
- **Draft a replacement** for a manual whose origin is `package` or `installed`: you never edit it. Write the corrected manual as a draft with the same skill name, and say in `draft.json` `notes` which manual it replaces and what the sessions showed.
- **Extract** the same kind of work recurring in `usage.none`: a new work skill as a draft from what those sessions did that worked — the commands in the order that succeeded, the checks, the pitfalls. Leave out what only one of them needed. When it is a method for splitting work rather than commands, draft a plan skill.
- **Recommend a cleanup**: a manual not given to any work for 60 days (`last_ms`), or one the sessions were given and did not use, or were misled by, more than once. Recommend that the owner disable it; you never do.
- Nothing: say what you read and why it supports no change.

## A draft

A directory `$OPENDAN_AGENT_ROOT/skills_drafts/<slug>/` laid out like a skill package: `cmd_skills/<name>/SKILL.md` (or `plan_skills/<name>/SKILL.md`), optional `references/`, and `draft.json`:

`{"schema": "opendan.skill_draft/1", "slug": "<slug>", "created_by": "<$OPENDAN_SESSION_ID>", "imported_from": {"digest": "sha256:<of the SKILL.md you wrote>", "source": "sessions: <the session ids it was drawn from>", "runtime_id": "private", "platform": "<os of the private runtime>"}, "split": [{"kind": "work | plan", "name": "<name>", "why": "<what it carries>"}], "verified": {"runtime_id": "private", "commands": {"<cmd>": true}}, "risk": "<one paragraph: what this manual makes the agent do, which steps cannot be undone>", "notes": "<the manual it replaces, if any; what the sessions showed; what the owner should check>"}`

The format reference is `$OPENDAN_AGENT_ROOT/cmd_skills/skill_authoring/SKILL.md`: read it before your first draft. Read every file back after writing it.

## Tell the owner

Once per draft and per recommendation, with a stable `--key` so that the same notice is not sent again, in the owner's language, understandable without this context, at the default level:

- a draft: `agent-session notify send --key skill_draft:<slug> --title "<Skill draft: what it is for>" --message "<what it is, which sessions it comes from, what it replaces if anything. It is on this agent's Skills page: read it there and install it, or run: buckyos skill install --draft <slug> --agent <agent id>>"`
- a cleanup: `agent-session notify send --key skill_cleanup:<name> --title "<Skill <name>: consider disabling>" --message "<the evidence: unused since when, or which sessions it misled and how. To disable: the Skills page, or buckyos skill disable <name> --agent <agent id>>"`
- a revision needs no notice of its own: name it in your report.

## Report

In `distance`: the session ids read; per manual what they showed; what was revised, drafted or recommended; what could not be judged. In `pending`: a pattern seen once that needs another look, a draft the owner has not decided on, the sessions read last. Say what you did not do: sessions left unread, a `skill revise` that was refused and why.

## Boundaries

- Never install, remove, enable or disable a skill; never change which manuals conversations get, nor a manual's `scope` towards `ui`.
- Never edit a manual whose origin is `package` or `installed`, and never write into `cmd_skills/`, `plan_skills/` or `goal_skills/` yourself: `skill revise`, `skill add` (with a tool you made) and drafts are the only ways.
- A manual records what works on this agent's runtimes: no secrets, tokens, personal data or one task's parameters.
