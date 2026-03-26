Use the session todo CLI to initialize the todo list during the planning phase.

- Build the todo list in one pass when the task scope is already clear.
- Prefer multiple `todo add` commands to define the full initial plan.
- Use the todo list to manage the later P-D-C-A workflow.

```bash
todo add "title" [--type=Task|Bench] [--priority=N] [--deps=T001,T003|--no-deps]
```

Parameters:
- `--type`: task type, usually `Task` or `Bench`
- `--priority`: priority level, usually 1-5
- `--deps`: dependency ids such as `T001,T003`
- `--no-deps`: explicitly create the todo without dependencies
