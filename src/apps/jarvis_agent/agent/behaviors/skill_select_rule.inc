## Choosing manuals for work

A work session gets its manuals (work skills) when it is created and keeps them for its whole life: `--cmd-skill <name>` on `agent-session create-worksession`, `--skill <name>` on a Todo.

- `agent-session skill list --kind work --scope work` lists the manuals work can take: what each is for, what it needs, its size. Choose by the description.
- At most 8 per work. Each one goes whole into the work's prompt, so give the ones the work will use and none when none fits. They are shortcuts, not rules the work must follow.
- Match each manual to the work's runtime. The list marks a manual that needs more than the standard environment (`needs: extended environment`, and a platform when it has one). The private runtime is `extended` and meets every official manual. A single-machine work (one placed on another runtime with `--runtime <id>`) has only that machine's baseline and platform, as the runtime context shows them.
- Work that names a manual its runtime does not meet, a manual the owner disabled (it is not in the list) or a conversation-only manual (`[ui]`) is refused: choose other manuals or another runtime.
- A manual that brings commands of its own shows `installed on: <runtime ids>`: those commands exist only there. Put the work on one of those runtimes, or on the private runtime when it can run them over there with `agent-session run --runtime <id>`.
