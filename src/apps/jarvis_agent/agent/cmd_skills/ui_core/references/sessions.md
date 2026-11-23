# Session commands

Use `agent-session` for the commands below. The environment supplies this conversation's identity.

- Children: `sessions --children --active [--json]`. A finished event shows one line; fetch the full `read-session <sid> --report`. A plan created with `--class plan_simple` accepts `post <sid> --msg "<change>"` even while paused. Explain it takes effect on resume. To redirect a plain work session, it must be stopped and new work started.
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

`/stop` or `/cancel` stops the current message; chat history and its background work survive. Retracting a message stops the turn without undoing completed actions. `/clean` or `/new` archives the conversation and starts a new one on the next message, refused while child work is unfinished. `/resume` continues a turn interrupted by host restart. The owner can pause, resume or stop background work from the side panel or agent home. Paused/interrupted work waits for resume.

List manuals with `skill list [--kind work|plan|goal] [--scope ui|work]`. Work sessions take their selected manuals when created. UI can also take manuals marked `ui` or `ui, work`; plan and goal skills are methods. Respect extended-baseline and platform requirements. Direct the owner to the Agent's Skills page or the appropriate owner command:

```text
buckyos skill install <.pikg|directory|url> --agent <agent_id>
buckyos skill remove <name> --agent <agent_id>
buckyos skill ui-add <name> --agent <agent_id>
```

Only the owner installs, removes, enables, disables or selects UI manuals. Changes apply when new sessions are created. For source text, intent analysis produces a draft for the owner to review and install.
