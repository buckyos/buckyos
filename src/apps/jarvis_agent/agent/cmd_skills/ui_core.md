---
name: ui_core
description: Commands of a conversation (UI session) — media understanding, work sessions and goals it started, memory, notifications, skills, subscriptions, tasks.
requires: an OpenDAN UI session (agent-session, llm_understand_media on PATH)
---
Run these through `shell`; the session they act for comes from the environment.
The owner chooses read-write (default) or read-only access when creating the Agent. User-data writes follow that choice; `$OPENDAN_AGENT_ROOT` always stays writable, including sessions, memory, workspaces and skills. For a read-only refusal, keep drafts and generated files in your session or AgentRoot and report the refused destination.
- `llm_understand_media <file | object id | url> [--goal "<what to find out>"]`: understand an image, a document (PDF …), audio or video — an attachment of a message (its id exactly as the attachment line gives it), a file, a link — through a model that can perceive it; the result is text. Use it before you answer about media; plain text files and text objects you read with `read`.
- `agent-session read-session <sid> --report`: the full report of a work session (the background context shows one line of it).
- `agent-session sessions --children --active`: the work sessions this conversation started and their state.
- `agent-session recall <tag>...` / `agent-session ctl perceive "<sentence>"`: memory (see Memory).
- `agent-session notify show <id>` / `notify answer <id> (--option <v> | --text "<t>")` / `notify ack <id>`: notifications shown to the user (see the constraints).
- `agent-session goal list` / `goal show <id>` / `goal post <id> --msg "<text>"`: the agent's goals.
- `agent-session skill list` / `skill install <x.pikg | dir>` / `skill remove <name>`: the agent's skills — command manuals a work session can be given when it is created, and methods for goals. Install or remove one when the user asks; sessions already running keep the manuals they started with.
- `agent-session ctl subscribe <spec>` / `ctl unsubscribe <id>`: follow an object or a session — `semi:object:<id>` shows its changes in the background context, `active:object:<id>` starts a turn on each change, `semi:session:<sid>` follows a session's state. A background line names its source as `<kind>:<id>`.
- `agent-session task wait <task_id> --wait-ms <ms>` / `task state <task_id>` / `task cancel <task_id>`: a shell command that continued as a task (give that shell call a larger wait_ms than the wait).
