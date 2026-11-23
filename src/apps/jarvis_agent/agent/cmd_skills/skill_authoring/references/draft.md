# Import and finish a draft

Use a tool's version-matched manual when available (e.g. `node_modules/<pkg>/lib/tools/skills/*/SKILL.md`, `<cli> skills list` and `<cli> skills get <name>`).

| agentskills.io source | OpenDAN draft |
| --- | --- |
| `name`, `description` | Match the directory's allowed spelling. Say what the skill does and when to use it. |
| `compatibility` | `requires` |
| version, license, author | End the body with source, version and license/attribution. Put provenance in `imported_from.source`. |
| `allowed-tools: Bash(<cmd>:*)` | Put external commands in `commands`. Record the permission difference in `notes`: the runtime grants access. |
| `disable-model-invocation: true` | `scope: ui` for owner-selected conversation manuals. |
| `scripts/` | Put a short snippet in a reference code block. For a program, declare it in `commands`/`requires` and identify its extension package in `notes`. |
| `references/`, `assets/` | Preserve references and text assets under `references/`. Omit other assets and name them in `notes`. |

Split a mixed source if it contains both a planning method and operational commands. In the work manual name the related plan skill. In the plan describe the capability needed for each step. Include all resulting skills in one draft.

Put `draft.json` beside the kind directories under `$OPENDAN_AGENT_ROOT/skills_drafts/<slug>/`. Example (replace placeholders, choose actual kinds and runtime):

```json
{
  "schema": "opendan.skill_draft/1",
  "slug": "<slug>",
  "created_by": "<current OPENDAN_SESSION_ID>",
  "imported_from": {"digest": "sha256:<digest of source text>", "source": "<file, URL or chat>", "runtime_id": "private", "platform": "linux"},
  "split": [{"kind": "work", "name": "<name>", "why": "<part of the source carried here>"}],
  "verified": {"runtime_id": "private", "commands": {"ffmpeg": true, "mytool": false}},
  "risk": "<what it runs, changes, deletes, sends out of the Zone or spends, and irreversible steps>",
  "notes": "<omissions, changes, checks for the owner and any replaced skill>"
}
```

Use `work`, `plan` or `goal` for each `split[].kind`. A replacement keeps the installed skill's name and explains why in `notes`. Write concrete effects in `risk`, which the owner reads before installing. Verify the source digest and read every output file.

```sh
mkdir -p "$OPENDAN_AGENT_ROOT/skills_drafts/<slug>/cmd_skills/<name>"
sha256sum "<source file>"
find "$OPENDAN_AGENT_ROOT/skills_drafts/<slug>" -type f
```

The owner can review on the Skills page or install with `buckyos skill install --draft <slug> --agent <agent id>`.
