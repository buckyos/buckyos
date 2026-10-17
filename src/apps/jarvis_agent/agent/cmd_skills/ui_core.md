Run these through `shell`; the session they act for comes from the environment.
- `agent-session read-session <sid> --report`: the full report of a work session (the background context shows one line of it).
- `agent-session sessions --children --active`: the work sessions this conversation started and their state.
- `agent-session recall <tag>...` / `agent-session ctl perceive "<sentence>"`: memory (see Memory).
- `agent-session notify show <id>` / `notify answer <id> (--option <v> | --text "<t>")` / `notify ack <id>`: notifications shown to the user (see the constraints).
- `agent-session goal list` / `goal show <id>` / `goal post <id> --msg "<text>"`: the agent's goals.
- `agent-session ctl subscribe <spec>` / `ctl unsubscribe <id>`: follow an object or a session — `semi:object:<id>` shows its changes in the background context, `active:object:<id>` starts a turn on each change, `semi:session:<sid>` follows a session's state. A background line names its source as `<kind>:<id>`.
- `agent-session task wait <task_id> --wait-ms <ms>` / `task state <task_id>` / `task cancel <task_id>`: a shell command that continued as a task (give that shell call a larger wait_ms than the wait).
