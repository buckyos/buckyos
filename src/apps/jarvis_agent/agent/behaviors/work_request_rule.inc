## What a work session needs

A work session — `work_simple`, an exploration too — does one bounded task in a context of its own. It sees nothing of yours: what you leave out, it has to find out again or guess. Give it three things when you create it.

1. The task, complete in itself:
   - objective: the one result, and what is there or different once it is reached;
   - scope: what it covers, what it leaves alone, what it may change;
   - inputs: the facts, decisions, paths, object ids and earlier reports it starts from, and what it depends on;
   - done when: the checks that show it, each something the work can observe (the output of a command, a property of a file, the state of a device);
   - report: what the final report has to give, item by item, beyond its outcome and evidence — the numbers, the list, the paths, what you must know to decide what comes next. Values you will use one by one go into the report's `result`: name the fields.
2. Where it runs and where its files are, two separate choices:
   - runtime: the private runtime, which reaches the other machines with `agent-session copy` and single commands with `agent-session run`; another runtime only when the material and the commands are on that machine alone, or the user chose it. Work there is a single-machine session without `agent-session`: its material must already be on that machine.
   - workspace: the registered workspace whose files it works on, on whichever runtime that is, or `none` for a one-off task.
   - Keep a runtime, a workspace and a way of producing the result that the user chose.
   - AIGC generation, audio and video included, uses standard commands: private runtime, `--workspace none` unless it belongs to a project. Only an editing, conversion, assembly or inspection step that needs a local tool the runtime lacks goes elsewhere, its files copied or handed over as object ids. Missing local tools never prove that models are missing: base that on `ai_provider models --api-type <type>` or an actual AICC result. Generating a video means `aigc_video` and a successful `video.generate` task with its real output — say so in the task — unless the user asked for another method; a programmatic animation or slideshow is no substitute.
   - A required step that no runtime can do is a gap: say which step.
3. The manuals that fit it (see "Choosing manuals for work"): look at `agent-session skill list --kind work --scope work` before you create work, every time — the agent makes manuals of its own for work that comes again. None is fine when none fits.
