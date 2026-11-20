---
name: skill_authoring
description: Format reference for writing OpenDAN skills and skill drafts — directory layout, front matter fields and their fixed words, mapping from agentskills.io SKILL.md files, size advice, the lint a draft must pass, install.toml and draft.json. Use it when a text, a tool's own manual or a repeated piece of work is turned into a work, ui or plan skill for the owner to install.
requires: an OpenDAN work session on the private runtime (agent-session on PATH, $OPENDAN_AGENT_ROOT writable)
baseline: standard
scope: work
---
#### Write a draft

Create one draft per source under `$OPENDAN_AGENT_ROOT/skills_drafts/<slug>/`. Keep installed skill directories owner-managed. The owner reviews, installs, removes, enables, disables and chooses conversation manuals on the Skills page or via `buckyos skill … --agent <agent id>`.

Choose a kind and create `<kind directory>/<name>/SKILL.md` inside the draft. Names and slugs use lowercase letters, digits, underscores and hyphens. Use the requested slug or the main skill name.

| Kind directory | What the reader needs |
| --- | --- |
| `cmd_skills` | Work session: commands, steps, checks and pitfalls. For conversation commands include `ui` in scope. The selected session receives all of SKILL.md. |
| `plan_skills` | Plan session: how to split, order and accept the work before writing a TodoList. Describe the capability each step needs. Omit shell commands and concrete work skill names. |
| `goal_skills` | A method the goal keeps for itself. |

Use UTF-8 text only. Each skill directory contains `SKILL.md` and optional `references/`. Long tables, unusual procedures and complete examples belong in references. Name each file in the main manual with its trigger and installed read path, e.g. `read $OPENDAN_AGENT_ROOT/cmd_skills/<name>/references/<topic>.md`. Keep UI manuals within 16 KB and plan manuals within 3 KB. Tools and binaries are supplied by the runtime or an extension package.

#### Front matter and lint

Write one field per line between `---` lines. Example work manual:

```yaml
---
name: inspect_media
description: Inspect media metadata with ffprobe when checking a deliverable's duration or codec.
requires: a runtime with ffprobe
commands: ffprobe
scope: work
baseline: extended
platforms: linux, macos
---
```

- Match `name` to the directory and write a one- or two-sentence `description` with both purpose and trigger. This is what the selector sees. `requires` describes prerequisites for a person.
- Work skills use `scope: work` (default), `ui` or `ui, work`. List external command names comma-separated, excluding shell builtins, common Unix and standard commands. The baseline defaults to `standard`. Set `baseline: extended` for dependencies such as ffmpeg or python3. Platforms are `linux`, `macos`, `windows`; omit for portable skills. Plan and goal skills use only `name`, `description`, `requires`.
- Check each command and option on the target runtime (`command -v <cmd>`, `<cmd> --help`; remotely, `agent-session run --runtime <id> -- command -v <cmd>`). Use `agent-session runtime list` for its OS. Name missing dependencies and the tested runtime in `requires`; mark them false in the draft.
- Lint shell blocks in both SKILL.md and references, including piped or chained commands. Drop unused command declarations. Label non-shell blocks with their language. Exclude copied secrets, tokens and personal data.

When importing an existing manual, read `$OPENDAN_AGENT_ROOT/cmd_skills/skill_authoring/references/draft.md` for mapping and splitting rules. For every draft use that file's `draft.json` schema and completion checks. Read back the draft files, give the owner the slug, and leave installation to them.
