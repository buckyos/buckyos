---
name: goal_delivery
description: Choose how a goal's results reach the user — a notification, a page or app made for this goal, or an AIWorkspace — and build what repeated deliveries need during the first activations, so later ones only use it. Read it when a goal starts delivering, or when its deliveries keep costing the same effort each time.
requires: work sessions with the manuals named below for building pages and apps; `agent-session notify send`
---
# Delivering a goal's results

Every way a goal delivers is a command; nothing is delivered by writing it into the goal's result. Pick the lightest one that does the job, and prepare it once.

## The ways

- **Tell the user**: `agent-session notify send --title "<one line>" --message "<details>" [--url <link>] [--level quiet|normal|timely|urgent]`. The system chooses the channel. Use it for a finding or a result that fits in a few lines. `--url` is where opening it lands: a page, an app, a report. Something the user must decide goes in your result's `questions` instead (with `options` when the answers form a short list).
- **Tell the user at known times** (a reminder, a fixed daily message): the check script sends it (`notify` in its result, goal skill `goal_check_script`), with no activation for each one. Write it once, in the first activation.
- **A page or app for this goal** (a dashboard, a list the goal keeps up to date, a form): BuckyOS lets the agent make an app with its own URL, a standard TypeScript app or one built on AIWorkspace. Build it in a work session — `agent-session create-worksession ... --behavior work_simple --cmd-skill buckyos_app_dev` for an app, `--cmd-skill aiworkspace_template_dev` for an AIWorkspace page — with a task that names what the page shows and how the goal updates it. When it exists, later activations only update its data and send a short notification with `--url` pointing at it.
- **A file or report** in the goal's workspace: write it (or have a work session write it) and notify with its link.

## Prepare once, deliver cheaply

The first activations of a goal may spend budget to build its tools: the check script (goal skill `goal_check_script`), the page or app, the small scripts that collect what each delivery needs. Later activations should only run them. Keep what you built visible to yourself across restarts: `agent-session goal skills add <name>=<where it is>` (a URL, a script path) with `--note "<what it is for>"`; it is listed in `<current_skills>` of every activation.

## Choosing

- The user reads it once and moves on → a notification.
- The user comes back to it, or it grows over time → a page or app, plus short notifications that link to it.
- It needs the user's decision → `questions` in the result (it becomes an answerable notification).
- Keep the number of notifications low: one that says what changed and links to where the details are beats several partial ones.

## Asking for an upgrade

You operate; the user decides what changes. When what you look after needs more than operation — something the user would see differently, a new permission or login, a higher cost, a new feature or source — ask for an upgrade once and go on operating what there is.

1. Write `upgrades/<yyyy-mm-dd>-<short-name>.md` in the workspace (without one, in this session's directory): what happened and how you know, what it affects, what you suggest, what happens if nothing is done, and the draft of the task for the work session that would do it — objective, scope, inputs, done when, what the report gives, and the last step: revise this goal (`goal_id`) to what it operates from then on.
2. Tell the user: `agent-session notify send --title "<one line>" --message "<two or three sentences; the file's absolute path; goal <goal_id>>" --key "upgrade:<goal_id>:<short-name>" [--url <a page that shows it>]`. The key makes a repeat the same notification: one matter, one notification.
3. Keep one line in `pending` with the file's path. The user answers in a conversation, where the work session is created from your file; it revises this goal when it is done (`changed_since_last_activation`).
4. The user says no, or lets it lie: do not raise it again. A matter that really changed gets a new short name.

A small change to the goal itself (what to watch, when not to remind) is no upgrade: the user says it in a conversation and the goal is revised there.
