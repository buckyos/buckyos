---
name: skill_authoring
description: Format reference for writing OpenDAN skills and skill drafts — directory layout, front matter fields and their fixed words, mapping from agentskills.io SKILL.md files, size advice, the lint a draft must pass, install.toml and draft.json. Use it when a text, a tool's own manual or a repeated piece of work is turned into a work, ui or plan skill for the owner to install.
requires: an OpenDAN work session on the private runtime (agent-session on PATH, $OPENDAN_AGENT_ROOT writable)
baseline: standard
scope: work
---
#### What a skill is
- Work skill: the manual for one kind of work (commands, steps, checks, pitfalls), given whole to a work session when it is created. `cmd_skills/<name>/SKILL.md`.
- Ui skill: a work skill with `scope: ui` or `scope: ui, work`: short, for commands used inside a conversation. Every conversation that takes it carries all of it.
- Plan skill: a method for splitting, ordering and accepting one kind of objective, read by a plan session before it builds its TodoList. `plan_skills/<name>/SKILL.md`. No commands.
- Goal skill: a method a goal keeps for itself. `goal_skills/<name>/SKILL.md`.

You write drafts. Only the owner installs, removes, disables or enables skills and chooses the manuals of conversations — on the agent's Skills page or with `buckyos skill … --agent <agent id>`. Never write into `$OPENDAN_AGENT_ROOT/cmd_skills/`, `plan_skills/`, `goal_skills/` or `tools/`.

#### Layout
A skill package and a draft have the same layout (`<name>`: lowercase letters, digits, `_`, `-`):
```text
cmd_skills/<name>/SKILL.md            front matter + body
cmd_skills/<name>/references/*.md     optional: detail read on demand
plan_skills/<name>/SKILL.md
plan_skills/<name>/references/*.md    optional
goal_skills/<name>/SKILL.md
tools/<name>/bin/<command>            optional: commands of work skill <name>, on PATH of sessions that took it
tools/<name>/install.toml             optional: how commands not carried in bin/ get onto a runtime
```
A single `.md` file directly under `cmd_skills/` is not a skill. `tools/<name>` without `cmd_skills/<name>/SKILL.md` is refused. A command in `tools/<name>/bin` must not have the name of a standard command (`agent-session`, `buckyos`, `gen_image`, …).

#### Front matter
One line per field, between `---` lines at the top of SKILL.md:
```yaml
---
name: <the directory name>
description: <what it does and when to use it>
requires: <free text for a person: accounts, permissions, services, tools>
commands: ffmpeg, ffprobe
scope: work
baseline: standard
platforms: linux, macos
---
```
- `description` is the only thing whoever chooses skills sees. Say both what the skill does and when to use it, with the words a request would use. One or two sentences.
- `commands`: names only, comma separated — the commands the body runs that are neither shell builtins or common Unix commands, nor standard commands, nor the skill's own `tools/<name>/bin`. It tells the owner what a runtime must have; it does not block anything.
- `scope` (work skills only): `work` (default) | `ui` | `ui, work`.
- `baseline`: `standard` (default) | `extended`. `extended` when the body needs commands outside the standard layer (`ffmpeg`, `pandoc`, `python3`, `node`, …).
- `platforms`: `linux`, `macos`, `windows`; leave it out when the skill works everywhere.
- Plan and goal skills take `name`, `description`, `requires` only: `scope` or `commands` there is an error.

#### From an agentskills.io SKILL.md
| Source | Here |
| --- | --- |
| `name` | `name` (rename to our character set when needed; directory = name) |
| `description` | `description`; rewrite it when it does not say both what and when |
| `compatibility` | `requires` |
| `metadata.version`, `license`, author | a line at the end of the body ("Source: … version …, license …") and `imported_from.source` in `draft.json` |
| `allowed-tools: Bash(<cmd>:*)` | each `<cmd>` into `commands`. Its permission meaning has no counterpart: permissions belong to the runtime, say so in `notes` |
| `disable-model-invocation: true` | `scope: ui`; it is never chosen automatically, only the owner adds it to conversations |
| `scripts/` | `tools/<name>/bin/` (executable; the body calls them by name) |
| `references/` | `references/` unchanged |
| `assets/` | `references/` when text; otherwise leave out and name it in `notes` |

Most source texts mix a method with commands. Split them: the method (stages, how to cut and order the work, what counts as done) becomes a plan skill; the commands and their pitfalls become a work skill. Name each in the other's body ("the commands for these steps are in the work skill `x`"; "to plan a larger job of this kind, see the plan skill `x_plan`"). A plan skill says "give this step a manual for X", never a session-creating command or `--skill <a concrete name>`.

A tool's own manual is a source too: an npm package may ship `node_modules/<pkg>/lib/tools/skills/*/SKILL.md`, and some CLIs print theirs (`<cli> skills list`, `<cli> skills get <name>`). Prefer it over a manual written from memory: it matches the installed version.

#### Size
- Ui skill: SKILL.md at most 16 KB (every conversation carries it; shorter is better). Plan skill: at most 3 KB.
- Work skill: keep SKILL.md to what every use needs. Long tables, per-command detail and rarely used procedures go into `references/<topic>.md`; the body names each file and when to read it: `read $OPENDAN_AGENT_ROOT/cmd_skills/<name>/references/<topic>.md`.

#### Verify the commands
For every command the body runs, on the runtime the skill is for:
```sh
command -v <cmd> && <cmd> --help | head -5
agent-session run --runtime <runtime id> -- command -v <cmd>
agent-session runtime list
```
The first line is the private runtime; the second is another runtime, when the skill is for that machine. Check the options the body uses against the command's own `--help`. A command that exists goes into `commands`. A command that is missing goes into `requires` ("needs `x`: not found on `<runtime id>` when this was written") and is `false` in `draft.json`. Any command outside the standard layer means `baseline: extended`. `platforms` is the OS of the runtime you verified on (`runtime list` shows it).

#### Lint a draft must pass
- Every `SKILL.md` has front matter with a `description`; `name` equals its directory; fixed words as above.
- Plan skill: no `scope`, no `commands`, no shell code blocks, no concrete commands or named work skills.
- Work skill: every command at the start of a line (or after `|`, `;`, `&&`, `$(`) in its shell code blocks, in SKILL.md and `references/`, is in `commands` — unless it is a shell builtin or common Unix command, a standard command, or in the skill's own `tools/<name>/bin`, or was not found and is named in `requires`. A name in `commands` that the text never mentions is dropped. Mark blocks that are not shell (`text`, `json`, `toml`, `yaml`) so they are not read as commands.
- No secrets, tokens or personal data copied from the source or the sessions.

#### install.toml
Only for a work skill whose commands must be installed onto a runtime and are not carried in `bin/`:
```toml
[check]
commands = ["mytool"]                 # must exist afterwards; absent: the manual's `commands`
[install.linux]
script = "setup/linux.sh"             # inside tools/<name>/; needs a runtime that grants system installs
writes = ["<instance_volume>/tools/bin"]   # outside <agent_root> / <instance_volume> it is lost when a container restarts
[uninstall.linux]
script = "setup/uninstall.sh"         # optional
```
Keys under `install` / `uninstall`: `linux`, `macos`, `windows`. Commands an extension package already provides (`ffmpeg`, `pandoc`, …) are not installed by a skill: declare `baseline: extended`. An install script is the riskiest part of a skill; describe exactly what it does in `risk`.

#### The draft
`$OPENDAN_AGENT_ROOT/skills_drafts/<slug>/` holds the files in the layout above plus `draft.json`:
```json
{
  "schema": "opendan.skill_draft/1",
  "slug": "<slug>",
  "created_by": "<this session's id: $OPENDAN_SESSION_ID>",
  "imported_from": {"digest": "sha256:<of the source text>", "source": "<file name | url | chat>", "runtime_id": "private", "platform": "linux"},
  "split": [{"kind": "work | plan | goal", "name": "<name>", "why": "<which part of the source it carries>"}],
  "verified": {"runtime_id": "private", "commands": {"ffmpeg": true, "mytool": false}},
  "risk": "<one paragraph: what this manual makes the agent do, which steps cannot be undone>",
  "notes": "<what was left out or changed, what the owner should check, which skill it replaces and why>"
}
```
```sh
mkdir -p "$OPENDAN_AGENT_ROOT/skills_drafts/<slug>/cmd_skills/<name>"
sha256sum "<source file>"
find "$OPENDAN_AGENT_ROOT/skills_drafts/<slug>" -type f
```
- `<slug>`: lowercase letters, digits, `-`, `_`; the name the owner asked for, else the main skill's name. One draft per source text, even when it becomes several skills.
- `risk` is read by the owner before installing. Write what the agent will do because of this manual: what it runs, what it changes or deletes, what it sends out of the Zone, what costs money, what cannot be undone. No reassurance, no summary of the format.
- A draft that replaces an installed skill uses the same skill name and says so in `notes`.
- Read every file back after writing it. The owner sees the draft on the Skills page and installs it there, or with `buckyos skill install --draft <slug> --agent <agent id>`.
