---
name: aiworkspace_template_dev
description: Build and maintain AI Workspaces from the command line — create a workspace, add tables, documents and Blocks by committing operations, import table files and Word / PowerPoint documents, put mind maps (Block-controls) on a canvas from a Markdown outline, develop controls (component.json in a home folder, build, pull, events and handlers, upgrades with migrations, export to the library or a .pikg, import), save and create workspaces from templates, export and import packages, reference data across workspaces, run wish blocks, publish data by path and insert published links.
requires: the buckyos command (agent container: /opt/buckyos/bin/agent-tools/buckyos; host: $BUCKYOS_ROOT/bin/buckyos) signed in; Deno for scripts that use the websdk directly; the workspace must give you the capability each step needs (structure/update to write, manage to share, publish or write HTML control code, update on a wish to analyze, code or run it — a run writes its results as whoever starts it, so `run` alone runs nothing yet); while a workspace names nothing for you as an Agent you act with your owner's rights
baseline: standard
commands: deno, node
---
#### Find, write and verify
Use `buckyos aiws <verb> --help` for the installed CLI, or `buckyos.getAiWorkspaceClient()` in the websdk for kRPC at `/kapi/aiworkspace`. Check `buckyos aiws info --workspace <ws>` for capabilities, epoch and public origin. Find entities with `ls`, `find` (names), `grep` (content), `stat` (type, revisions, operations), `cat` (plain text) and `read` (native content). A `path_only` ancestor is unreadable. Keep entity IDs for stored references because names and paths can change.

Use `table add|set|rm|field-add|option-add`, `text append|replace|rm`, `mv`, `rename` or `set-prop` for a simple change. Read back with `cat` or `sql`. Query one SELECT using displayed field names, `"<field>#raw"` for stored values and `_id` for record ID. A table view exposes its SELECT in `meta.shows.view_sql`. Pass the accompanying `view_params` as `doc.query.params` to preserve manual row order. Materialize the smaller table in a CTE when joining large tables.

A write is a Commit: `doc.commit {protocol_version:"1.2",workspace_id,epoch,idempotency_key,session_id,operations}`. Use the current epoch and one `aiws.commit` per atomic change. The client chunks requests over 8 MiB. Split only when exceeding 32 MiB or 50,000 operations per Commit. On timeout use `doc.get_submission {epoch,idempotency_key}` and reuse the same key for retry. Read `errors[].code`, `sub_code` and `conflicts[]` on rejection. Refresh revisions before retrying conflicts and refresh the epoch after restore.

#### Choose a procedure
- Create a table, canvas, or composite instance, clone Blocks, import files or place a calendar control: `$OPENDAN_AGENT_ROOT/cmd_skills/aiworkspace_template_dev/references/content.md`.
- Build, test, migrate and package custom controls or export and create a template: `$OPENDAN_AGENT_ROOT/cmd_skills/aiworkspace_template_dev/references/controls.md`.
- Export, restore, publish or link data across workspaces, set permissions or change the shared toolbar or cover: `$OPENDAN_AGENT_ROOT/cmd_skills/aiworkspace_template_dev/references/sharing.md`.

Use `aiworkspace_run` to execute a claimed wish, `aiworkspace_sync` for external data fetched by an Agent. Ask before a write action on an object. Change ACLs only when the user asks. Read back the result and give the human-facing URL from `ws.get_info.public_base` plus `/<workspace>` (surface: `/<workspace>/<surface>?id=<surface_id>`).
