---
name: goal_check_script
description: Write the check script that decides when this goal is worth an activation — a cheap TypeScript look at files, the network or Memory that returns true only when something you care about changed. Read it before the first `goal check set`, and whenever checks keep passing with nothing to do or keep failing.
requires: the goal's own commands (`agent-session goal check show|run|set`)
---
# Writing a goal's check script

A goal is woken by its check, not by the clock: the host runs the check on a timetable, before any model call, and only `true` gives the goal an activation (with your `reason`). A check costs no tokens; an activation costs tens of thousands. The first activations of a goal should build a good check; later ones should rarely need to happen.

## The contract

```ts
export default async function check(ctx, state) {
  // ctx: goal_id, title, objective, scope {paths, objects, workspace}, agent_root,
  //      now_ms, last_inference_at_ms, last_check_at_ms, last_check, pending, active_works,
  //      host.call(method, params) — read-only calls of the host (memory.status {full?})
  // state: what this function returned as `state` last time (null the first time)
  return { run: false, state };            // nothing to do: no activation
  // return { run: true, reason: "3 new files in inbox/: a.pdf, b.pdf, c.pdf", state: next };
}
```

- It may read files (`Deno.readTextFile`, `Deno.readDir`, `Deno.stat`) and use the network (`fetch`, and the `buckyos` websdk import). It cannot run commands or write files: the host refuses both. Observe here, act in the activation.
- Keep what you saw last in `state` (a size, a modification time, a list of ids, an ETag): the check compares against it and needs no write access. Return the new `state` with every answer; `null` keeps the old one. At most 64 KiB.
- `reason` is read by the next activation instead of looking again: say what changed, concretely (names, counts, links), not "something changed".
- Throwing, taking more than 30 seconds or returning anything else is an error: no activation. After errors the hard guarantee still wakes you once in a long while and `<check_errors>` in the goal header says why — fix the script then.

## How to write one

1. Decide what change deserves your attention, in terms you can test without a model: a new file in a folder, a page whose content hash changed, an issue list with a newer item, Memory material waiting (`memory.status`).
2. Write the smallest script that answers it, in the session's directory (e.g. `check.ts`). Prefer one cheap request or one directory listing; avoid reading whole large files every time — compare sizes and dates first.
3. Run it the way the host will: `agent-session goal check run --script check.ts` (the same Deno, permissions, `ctx` and stored `state`). Run it twice: the second run must see the first run's `state`. Make the condition true on purpose once if you can, and see the `reason`.
4. Install it with a timetable that matches how fast the thing changes: `agent-session goal check set --script check.ts --every 600 --reason "<why>"`, or `--cron "0 9 * * 1-5" --timezone <zone>` for wall-clock times. The timetable is at least 60 seconds; the check is cheap, but your hard interval and budget still limit how often you can be woken.
5. Watch `<gate>` in the next headers: many `checks_true` followed by `no_action` means the condition is too loose; `check_errors` means it breaks; `no_action_in_a_row` growing means you are woken for nothing. Tighten it then, and record why in your report.

## Patterns

- Folder watch: list names and sizes, keep them in `state`, `run` when a new name appears or a size changes; the `reason` lists them.
- Web page or API: keep an ETag or a hash of the part you care about; `run` when it differs.
- Deadline: keep nothing; `run` when `now_ms` passes a date you put in `state` or in `pending`.
- Combine: one check may cover several of the goal's duties; give each its own part of `state` and say in `reason` which one fired.
