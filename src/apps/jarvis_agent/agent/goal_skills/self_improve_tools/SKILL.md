---
name: self_improve_tools
description: Duty `tools` of the self improvement goal — make the command work keeps lacking, or the short cut for what it keeps composing again, put it where every work session finds it, and write the manual that makes work use it. Read it when a reason names `tools`, or when the same commands are composed again and again.
requires: the self improvement goal's session, or work it sent (`agent-session skill add`); the private runtime
---
# Making a tool

A tool pays when it lets work finish that could not, or saves every later work the rounds of finding out the same thing. It costs: somebody has to keep it working. Make few, make them small, make sure they are used.

## Is there a gap

- Evidence first: at least two work sessions that show the same gap, read with `agent-session read-session <sid> --report` (add `--worklog 15` only where the report does not say what was run: a worklog stays in your context for the rest of the activation). A command not found (exit 127) that the work then did another way in two rounds is no gap; a typo is none; a tool of another operating system is none.
- `agent-session skill list --all --json` and `command -v <name>`: is there something already that does it, under another name or in a manual that work was not given? Then the manual's `description` is the fault: goal skill `self_improve_manuals`.

## The cheapest way that closes it, in this order

1. What is there does it, work just does not know: say it in a manual (revise one of your own, draft for a package's).
2. A **short cut**: existing commands and interfaces behind one short name with agreed parameters, so that work stops composing them again (what `xcall owner.cal add` does for the calendar). Usually a few lines.
3. A package manager that installs without root into the agent's own place (`uv tool install`, `deno install`, `npm install -g --prefix`), then a short cut to it.
4. An open source program built here, from its source, at a fixed version.
5. A program of your own, in Deno (TypeScript) or Python.

Scripts of your own: `#!/usr/bin/env -S deno run --allow-…` with only the permissions it needs, or `#!/usr/bin/env -S uv run --script` / `python3`. `--help` says what it does; results on stdout (JSON where a work reads it), errors on stderr, exit 0 only when it did what it says; no state outside its arguments; nothing secret inside; no interactive questions.

## Put it where work finds it

1. The command: `$OPENDAN_AGENT_ROOT/tools/<name>`, executable (`chmod 755`). The directory is on the `PATH` of every session of the private runtime. One file where that is possible; what it needs besides goes to `$OPENDAN_AGENT_ROOT/tools/.<name>/`.
2. Try it the way work will, in one command: from another directory, by its name, with the inputs of the sessions that lacked it, and once wrong on purpose.
3. The manual that makes work use it: a SKILL.md in your working directory with front matter `name`, `description` (what for and when — this line is all that intent analysis sees), `requires`, `commands: <name>`, `scope: work`; the body: when to use it, the call with one real example, what it prints, how to check the result, what it does not do. Under 60 lines. Then
   `agent-session skill add <skill name> --file SKILL.md --reason "<the sessions that lacked it and what they did instead>"`.
   The name must be new; the host checks it like any package and refuses `scope` with `ui` (a draft then). Improving it later is `skill revise`.
4. Keep it in the Goal's skills, so it is looked after: `agent-session goal skills add <name>=$OPENDAN_AGENT_ROOT/tools/<name> --note "made for <gap>; check with <command>"`.
5. Tell the owner, once: `agent-session notify send --key self_improve_tool:<name> --title "<what the agent can do now>" --message "<what the command does, which work it was made for, that it is on the Skills page (disable it there if unwanted)>"`.

Who does it. An activation is for the judgement: is there a gap, what closes it. Only a short cut of a few lines is made right here. A program is made by a work session (`--behavior work_simple`), with a budget of its own: give it what the command takes and prints, the inputs and the results the sessions had (to try it with), where it goes, points 1–3 above, and "done when the command works by its name and `agent-session skill add` took its manual". End with `created_work`; at the activation its end brings, read its report, run the command once yourself, then points 4 and 5.

## Afterwards

`usage["work:<skill name>"]` in `.meta/skills_state.json` says whether work was given the manual; the reports say whether it helped. Not used after ten pieces of work that could have used it: make the `description` better once, then recommend disabling it. It broke: repair it in place. What is not used and not worth repairing goes: remove your own file from `tools/`, and recommend to the owner that the manual be disabled.

Boundaries: only `$OPENDAN_AGENT_ROOT/tools/` and your own manuals are yours. Never replace a command of the system or of another manual by using its name. Never install as root, never change the runtime's own packages, never send data anywhere a tool does not need to.
