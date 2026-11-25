---
name: ui_core
description: Commands of a conversation (UI session) — media understanding, work sessions and goals it started, memory, notifications, skills, subscriptions, tasks.
requires: an OpenDAN UI session (agent-session, llm_understand_media on PATH)
baseline: standard
scope: ui
---
#### Files, media and objects

Use `shell` on the private runtime. The owner's default runtime is in `<runtime>`. For a path the user gives, try that machine first with `read` or `agent-session copy --from <owner runtime>:<path> --to session:<file>`. If unavailable or missing, try private and say where you read it. A quick read/listing may use `agent-session run --runtime <owner runtime> --cwd <dir> -- <command>`. Use `try_analyze_user_intent` for changes or desktop actions on the owner's machine, and for work on other runtimes.

Keep Agent files in `$OPENDAN_AGENT_ROOT` (owner's `home/agents/<full agent_id>/`). It stays writable with read-only user-data access. On refusal save drafts there or in the session and report the refused destination.

- Read text with `read`. Before answering about images, audio, video or documents, run `llm_understand_media <file|object id|url> --goal "<question>"` alone, without pipes or `&&`. Use the attachment ID or Zone path exactly as given. Long conversions return as tasks. Do not restart them. For a long document the result lists unseen pages; name a page in the goal to request it.
- To add to the owner's calendar, run `xcall owner.cal add --params '{"title":"…","start":"…","end":"…"}' --idempotency-key <stable-key>` in the session directory. Choose a fresh key for each request and reuse it on retries. Give the owner the returned `open_url`. To check a day, run `xcall owner.cal list --params '{"from":"…","to":"…","tz":"…"}'` from local midnight to the next local midnight. Use offset-bearing timestamps in the owner's time zone. If the date or time zone is unclear, ask before calling. These actions are ready to call directly. For other calendar operations, to-dos, contacts and notifications, see `$OPENDAN_AGENT_ROOT/cmd_skills/owner/SKILL.md`.
- `read` on an `obj://`, `cyfs:///` or Zone HTTPS URL describes its properties, actions, risks and events. Use `read uri=<url> property=<name>` for one property. Before `agent_tool xcall <obj://…> <action> [key=value …]`, ask the user if the declared risk is anything other than `read_only`.

#### Work and conversations

Use `agent-session sessions --children --active` (or `--finished <n>` for what ended lately) and `agent-session read-session <sid> --report`. Relay verified deliverables, or the failure, partial results and what remains. Forward changes to unfinished work you started (a plan, or a `work_simple`) with `agent-session post <sid> --msg "<user's words and context>"`. A paused one keeps its message until resumed; one that finished refuses it. A plan's workers take no messages.

For notifications, goals, task state/cancellation, subscriptions, workspaces, copy/run options, owner skill management or chat controls, read `$OPENDAN_AGENT_ROOT/cmd_skills/ui_core/references/sessions.md`. A shell task returns its result as input. Only the owner installs skills and chooses conversation manuals. Send skill drafting to intent analysis.

`[browser]` events arrive in the Agent Browser conversation when the user takes over, hands back or asks to continue. All sessions share tabs. Read `$OPENDAN_AGENT_ROOT/cmd_skills/browser/SKILL.md` before using `agent-browser` if the manual is absent.

Use the current behavior's `[reply]` schema. The user sees `reply` and `artifacts` attachments; `topic` updates the conversation. When a reply is rejected and you cannot give it in that format, run `agent-session ctl reply-mode compat`: your text is then sent as it is, and `submit_reply` gives a reply with files or a new topic. `agent-session ctl reply-mode schema` goes back.

## Memory: what to perceive

Use `agent-session ctl perceive "<observation>"` for the owner's lasting preferences, requirements, judgments and corrections, useful facts, names or pointers, project/tool observations, and consequential surprises, mistakes or methods. Capture lessons from a work report when relaying it, except what a plan's report lists as `Remembered (already in memory)`. Record only what will matter later or revises existing knowledge. Preserve attribution, exact values, scope, conditions and uncertainty. Use `--explicit` only when asked to remember. Leave out secrets, progress, live state and one-off task parameters.

For flags, evidence, corrections and examples read `$OPENDAN_AGENT_ROOT/cmd_skills/ui_core/references/memory.md`. Search with `agent-session recall <words or tags>... [--object <path>]` and inspect evidence with `agent-session recall --read <ref>`. Distinguish unconsolidated notes from conclusions.
