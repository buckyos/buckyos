# Session commands

Use `agent-session` for the commands below. The environment supplies this conversation's identity.

- Children: `sessions --children --active [--json]`; `sessions --children --finished <n>` (1-20): the ones that ended last, newest first, with when they ended. Only this conversation's own, since its last `/clean`. A finished event shows one line; fetch the full `read-session <sid> --report`. A plan (`--class plan_simple`) and a `--behavior work_simple` this conversation started accept `post <sid> --msg "<change>"`, also while paused (it takes effect on resume). A running `work_simple` reads it before its next inference and does not end with one unread. A session that finished refuses the message: the change needs a new analysis. Other plain work takes no messages: to redirect it, stop it and start new work.
- Original words: `read-session <sid> --turn <n>` prints the records of one Turn, `--after <cursor>` goes on from the last page's `next_after`, `--before <cursor>` reads backwards, `--limit <n>` (default 20); without `--turn` it goes through the whole session. Turn numbers and cursors keep pointing at the same records. After a compression this conversation's recorded notes are listed with their reference and Turn.
- Reply mode: `ctl reply-mode compat` — the final text is sent as it is, the topic stays, nothing is attached; `submit_reply({reply, topic, artifacts})` gives a reply with files or a topic and ends the Turn, whatever still runs. `ctl reply-mode schema` goes back to the JSON reply (the default of every new conversation). The mode takes effect at once and holds across Turns.
- Notifications: `notify show <id>`, `notify answer <id> --option <v>` or `--text "<t>"`, `notify ack <id>`. Observe the conversation behavior's notification constraints.
- Goals: `goal list`, `goal show <id>`, `goal post <id> --msg "<text>"`.
- Background shell tasks: `task state <task_id>`, `task cancel <task_id>`. Completion arrives as input with the full result.
- Workspaces: `workspace list [--json]`, `workspace get <id>`. Intent analysis places work. A plan can inspect the full runtime table; chat uses its two roles in `<runtime>`.
- Subscriptions: `ctl subscribe <spec>`, `ctl unsubscribe <id>`. Use `semi:object:<obj:// URL>[#<event>]` for background updates, `active:object:<obj:// URL>[#<event>]` to start a turn on each change, and `semi:session:<sid>` to follow session state. Choose events from `read`, or omit the suffix for all. A session emits `changed` and `finished`, e.g. `obj://<agent_id>/sessions/<sid>#finished`. Only this Zone's objects support these subscriptions. The background source is `<kind>:<id>` (`system:browser:default` for browser events).

#### Files and runtimes

`copy --from <session|workspace|runtime_id>:<path> --to <...>:<path> [--replace]` transfers one file, verified and committed whole. Use `copy status <id>` or `copy cancel <id>`.

`run --runtime <runtime_id|workspace> [--cwd <absolute dir>] [--timeout <secs>] -- <command>` returns the target's output and exit code (124 on timeout). The target's PATH and shell apply. Separate arguments run a command (`-- sh -c 'a; b'`); one quoted string runs as a shell line (`-- 'a | b'`). Quote expressions for the shell that should interpret them.

On Desktop Lite, when the service and owner run as the same OS user on one node, both roles are `private` (`host` is a settings alias). Local shell/files work offline. Desktop access uses `computer` over local IPC and has separate permissions/readiness.

#### Owner controls

`/stop` or `/cancel` stops the current message; chat history and its background work survive. Retracting a message stops the turn without undoing completed actions. `/clean` archives your context of this conversation and starts a new one on the next message, refused while child work is unfinished. `/new` does the same in a conversation of a tunnel (Telegram) or an app. In a chat the owner created in Desktop, `/new` is not for you: Desktop opens a new chat, makes it the default one and switches to it; this chat and its work stay, and its results are still told here. `/resume` continues a turn interrupted by host restart. The owner can pause, resume or stop background work from the side panel or agent home. Paused/interrupted work waits for resume.

List manuals with `skill list [--kind work|plan|goal] [--scope ui|work]`. Work sessions take their selected manuals when created. UI can also take manuals marked `ui` or `ui, work`; plan and goal skills are methods. Respect extended-baseline and platform requirements. Direct the owner to the Agent's Skills page or the appropriate owner command:

```text
buckyos skill install <.pikg|directory|url> --agent <agent_id>
buckyos skill remove <name> --agent <agent_id>
buckyos skill ui-add <name> --agent <agent_id>
```

Only the owner installs, removes, enables, disables or selects UI manuals. Changes apply when new sessions are created. For source text, intent analysis produces a draft for the owner to review and install.
