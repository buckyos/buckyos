---
name: opendan_agent_dev
description: Develop OpenDAN Agents — AgentRootFS layout, agent.toml, behaviors, work and plan skills and TypeScript CLI tools on the buckyos SDK, sessions and report artifacts, debugging, packaging an Agent template PIKG, registering it and creating an Agent from it, and where skill packages and skill drafts fit.
requires: an OpenDAN session (agent-session on PATH) and the buckyos command; registering templates and creating Agents is done by the owner (Control Panel/Desktop or host CLI); host debugging needs the BuckyOS repo
baseline: standard
commands: deno, node
---
#### Start from the installed Agent
Inspect `$OPENDAN_AGENT_ROOT/agent.toml`, `behaviors/` and `agent-session skill list`. Copy the source template (`src/apps/jarvis_agent/agent/` for Jarvis) when developing a new one. A template PIKG carries an `agent` subpackage, `categories:["agent"]` and a `www` endpoint. Each Agent is a separate App instance `<agent_id>@<owner>` with its own DID. Keys, `.meta/`, `state/`, `sessions/` and `memory/` belong to the instance. Exclude them from the package.

#### Develop and verify
1. Put identity text in `role.md` / `self.md`, behaviors in `behaviors/<name>.toml` and session classes in `agent.toml`. Use `work_simple` for one bounded task and `plan_simple` for a plan with Todos. Resolve every behavior reference before testing.
2. Put work/UI manuals in `cmd_skills/<name>/SKILL.md`, plans in `plan_skills/` and goals in `goal_skills/`. Skills contain text and optional `references/`. Executables belong to the Agent's `tools/` or an extension package. For a new manual, use the `skill_authoring` manual. The Agent produces a draft in `skills_drafts/<slug>/`; only the owner installs, removes, disables, enables or selects UI manuals.
3. Exercise the changed behavior in a throw-away Work, choosing its manuals at creation. For example, `agent-session create-worksession --title "Check behavior" --objective "Exercise the changed behavior" --behavior <name> --cmd-skill <skill>`. Read `agent-session read-session <sid> --report --worklog 50`. A final explicit report requires evidence, an outcome and any deliverables.
4. From template source, build, pack and inspect the PIKG. Have the owner register it and create an Agent. Verify `ready`, the first Message Center reply, permission mode and writable AgentRoot. Updates preserve local edits and existing frozen sessions.

#### Read for the change
- AgentRootFS, session-class configuration, behavior loops, goals, skill selection and SDK commands: `$OPENDAN_AGENT_ROOT/cmd_skills/opendan_agent_dev/references/authoring.md`.
- Runtime/workspace choice, transfers, artifacts, final reports and Todo orchestration: `$OPENDAN_AGENT_ROOT/cmd_skills/opendan_agent_dev/references/sessions.md`.
- Explicit remote executors, desktop drivers, ledger recovery and OS-specific limits: `$OPENDAN_AGENT_ROOT/cmd_skills/opendan_agent_dev/references/executors.md`.
- Package metadata, owner registration and creation, debugging and recovery: `$OPENDAN_AGENT_ROOT/cmd_skills/opendan_agent_dev/references/deployment.md`. Shared release and publish steps are in `$OPENDAN_AGENT_ROOT/cmd_skills/buckyos_dev_release/SKILL.md`.
