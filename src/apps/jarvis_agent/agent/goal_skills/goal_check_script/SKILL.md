---
name: goal_check_script
description: Write the check script that decides when this goal is worth an activation — a cheap TypeScript look at files, the network or Memory that returns true only when something you care about changed — and that sends reminders at their times without any activation. Read it before the first `goal check set`, when the goal has to tell the user something at known times, and whenever checks keep passing with nothing to do or keep failing.
requires: the goal's own commands (`agent-session goal check show|run|set`)
---
# Writing a goal's check script

A goal is woken by its check, not by the clock: the host runs the check on a timetable, before any model call, and only `true` gives the goal an activation (with your `reason`). A check costs no tokens; an activation costs tens of thousands. The first activations of a goal should build a good check; later ones should rarely need to happen. What the goal must say at known times (a reminder) the check sends itself, without waking the goal.

## The contract

```ts
export default async function check(ctx, state) {
  // ctx: goal_id, title, objective, scope {paths, objects, workspace}, agent_root,
  //      now_ms, timezone (the goal's IANA zone or null), trigger, held,
  //      last_inference_at_ms, last_check_at_ms, last_check, pending, active_works,
  //      host.call(method, params) — read-only calls of the host (memory.status {full?})
  // state: what this function returned as `state` last time (null the first time)
  return { run: false, state };            // nothing to do: no activation
  // return { run: true, reason: "3 new files in inbox/: a.pdf, b.pdf, c.pdf", state: next };
  // return { run: false, notify: { title: "Stand up", message: "…", level: "timely" }, state: next };
}
```

- It may read files (`Deno.readTextFile`, `Deno.readDir`, `Deno.stat`) and use the network (`fetch`, and the `buckyos` websdk import). It cannot run commands or write files: the host refuses both. Observe here, act in the activation.
- Keep what you saw last in `state` (a size, a modification time, a list of ids, an ETag): the check compares against it and needs no write access. Return the new `state` with every answer; `null` keeps the old one. At most 64 KiB.
- `reason` is read by the next activation instead of looking again: say what changed, concretely (names, counts, links), not "something changed".
- `notify: {title, message?, url?, level?}` is a notification the host sends the user at once, as this goal (`level`: `quiet | normal | timely | urgent`; `url`: `https://…` or a Zone path `/…` where opening it lands). It needs no activation, so return it with `run: false`. One per check; a notification that cannot be sent makes the check an error and its `state` is not kept, so the next check decides again.
- The check runs even while the hard interval or the budget keeps you from being woken: `ctx.held` then says why (`min_interval`, `period_budget`, `agent_budget`, `guarantee`), otherwise it is `null`. A `run: false` result counts as usual, notification included; a `run: true` result is set aside whole (no notification, the old `state` stays), so return a notification that must not wait with `run: false`.
- Throwing, taking more than 30 seconds or returning anything else is an error: no activation. After errors the hard guarantee still wakes you once in a long while and `<check_errors>` in the goal header says why — fix the script then.

## How to write one

1. Decide what change deserves your attention, in terms you can test without a model: a new file in a folder, a page whose content hash changed, an issue list with a newer item, Memory material waiting (`memory.status`).
2. Write the smallest script that answers it, in the session's directory (e.g. `check.ts`). Prefer one cheap request or one directory listing; avoid reading whole large files every time — compare sizes and dates first.
3. Run it the way the host will: `agent-session goal check run --script check.ts` (the same Deno, permissions, `ctx` and stored `state`). Run it twice: the second run must see the first run's `state`. Make the condition true on purpose once if you can, and see the `reason`.
4. Install it with a timetable that matches how fast the thing changes: `agent-session goal check set --script check.ts --every 600 --reason "<why>"`, or `--cron "0 9 * * 1-5" --timezone <zone>` for wall-clock times. The timetable is at least 60 seconds; the check is cheap, but your hard interval and budget still limit how often you can be woken.
5. Watch `<gate>` in the next headers: many `checks_true` followed by `no_action` means the condition is too loose; `check_errors` means it breaks; `no_action_in_a_row` growing means you are woken for nothing. Tighten it then, and record why in your report.

## Reminders at known times

The goal's first activation turns the reminders in its objective into a check script; after that no reminder needs an activation. Give the timetable a `--cron` that comes at every reminder time (more often is fine), in the user's zone, and let the script find the reminder that is due and not sent yet: a check that comes late still sends it, one that comes twice does not send it twice.

```ts
// Weekdays in the user's zone at 15:00, 15:50, 16:40 and 17:30: stand up for 10 minutes.
const TIMES = ["15:00", "15:50", "16:40", "17:30"];
const DAYS = [1, 2, 3, 4, 5]; // Monday … Friday
const LATE_MIN = 15; // a check this late still sends it; later, the reminder is dropped

function local(ms: number, tz: string) {
  const p = Object.fromEntries(
    new Intl.DateTimeFormat("en-US", {
      timeZone: tz, hourCycle: "h23", weekday: "short",
      year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit",
    }).formatToParts(new Date(ms)).map((x) => [x.type, x.value]),
  );
  return {
    date: `${p.year}-${p.month}-${p.day}`,
    day: ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"].indexOf(p.weekday),
    minute: Number(p.hour) * 60 + Number(p.minute),
  };
}

export default function check(ctx, state) {
  const now = local(ctx.now_ms, ctx.timezone ?? "UTC");
  if (!DAYS.includes(now.day)) return { run: false };
  const due = TIMES.find((t) => {
    const [h, m] = t.split(":").map(Number);
    return now.minute >= h * 60 + m && now.minute < h * 60 + m + LATE_MIN;
  });
  const key = `${now.date} ${due}`;
  if (!due || state?.sent === key) return { run: false };
  return {
    run: false,
    notify: { title: "站起来活动 10 分钟", message: `${due} 了，离开座位走一走。`, level: "timely" },
    state: { sent: key },
  };
}
```

Install it with `agent-session goal check set --script check.ts --cron "*/10 15-17 * * 1-5" --reason "stand-up reminders"` (the zone comes from the conversation that created the goal; give `--timezone` if `ctx.timezone` is wrong). Before that, `goal check run --script check.ts` shows what it would send now.

- Write the reminder text in the user's language, short enough to read on a lock screen.
- A single reminder ("at 9 tomorrow"): send it with `run: false` and keep `{sent: true}`; at the next check return `run: true` with a `reason` such as "the reminder was sent at 09:00", and mark the goal achieved in that activation.
- Something that needs judgement on the day (the user is on leave, asked to skip today) is not the script's: keep what you learned in the script's constants or let the check return `run: true` when a file or message it can read says so.

## Patterns

- Folder watch: list names and sizes, keep them in `state`, `run` when a new name appears or a size changes; the `reason` lists them.
- Web page or API: keep an ETag or a hash of the part you care about; `run` when it differs.
- Deadline: keep nothing; `run` when `now_ms` passes a date you put in `state` or in `pending`.
- Combine: one check may cover several of the goal's duties; give each its own part of `state` and say in `reason` which one fired.
