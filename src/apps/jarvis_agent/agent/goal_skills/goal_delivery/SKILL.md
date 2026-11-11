---
name: goal_delivery
description: Choose how a goal's results reach the user — a notification, a page or app made for this goal, or an AIWorkspace — and build what repeated deliveries need during the first activations, so later ones only use it. Read it when a goal starts delivering, or when its deliveries keep costing the same effort each time.
requires: work sessions with the manuals named below for building pages and apps; `agent-session notify send`
---
# Delivering a goal's results

Every way a goal delivers is a command; nothing is delivered by writing it into the goal's result. Pick the lightest one that does the job, and prepare it once.

## The ways

- **Tell the user**: `agent-session notify send --title "<one line>" --message "<details>" [--url <link>] [--level quiet|normal|timely|urgent]`. The system chooses the channel. Use it for a finding, a reminder or a result that fits in a few lines. `--url` is where opening it lands: a page, an app, a report. Something the user must decide goes in your result's `questions` instead.
- **A page or app for this goal** (a dashboard, a list the goal keeps up to date, a form): BuckyOS lets the agent make an app with its own URL, a standard TypeScript app or one built on AIWorkspace. Build it in a work session — `agent-session create-worksession ... --behavior work_simple --cmd-skill buckyos_app_dev` for an app, `--cmd-skill aiworkspace_template_dev` for an AIWorkspace page — with a task that names what the page shows and how the goal updates it. When it exists, later activations only update its data and send a short notification with `--url` pointing at it.
- **A file or report** in the goal's workspace: write it (or have a work session write it) and notify with its link.

## Prepare once, deliver cheaply

The first activations of a goal may spend budget to build its tools: the check script (goal skill `goal_check_script`), the page or app, the small scripts that collect what each delivery needs. Later activations should only run them. Keep what you built visible to yourself across restarts: `agent-session goal skills add <name>=<where it is>` (a URL, a script path) with `--note "<what it is for>"`; it is listed in `<current_skills>` of every activation.

## Choosing

- The user reads it once and moves on → a notification.
- The user comes back to it, or it grows over time → a page or app, plus short notifications that link to it.
- It needs the user's decision → `questions` in the result (it becomes an answerable notification).
- Keep the number of notifications low: one that says what changed and links to where the details are beats several partial ones.
