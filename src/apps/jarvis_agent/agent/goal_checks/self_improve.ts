// Check of the built-in self improvement goal (its initial version: the goal
// may rewrite it with `agent-session goal check set`).
//
// The host runs it before any model call. It answers `run: true` only when
// there is material for a duty the owner has not switched off, and `reason`
// names the duty:
// - memory: Memory holds material for a consolidation (new or due
//   perceptions, cognitions due for review). The cheap version vector is read
//   first; the full check runs only when something moved, a deadline or a
//   review came, a backlog was seen, or every 2 hours. Mechanical work
//   (unfinished receipts, cleanup) is the host's and needs no `true`.
// - manuals: enough work was given manuals since the goal last looked (the
//   usage record the host keeps, `.meta/skills_state.json`: every work
//   session counts once for each manual it was given, or under `none`).
// - tools: the same command was not found by two work sessions or more
//   (their worklogs; finished ones only, each read once). A command they only
//   looked for (`which`, `command -v`) counts when their task names it.
// A look at manuals or tools costs a lot of tokens: a few new work sessions,
// or a week with at least one, are worth it; their material is gathered every
// 6 hours.

type Usage = { frozen?: number };
type Pending = { new?: number; due?: number; waiting?: number; next_deadline?: string; oldest_at_ms?: number };
type Status = { digest: string; pending?: Pending; due_reviews?: number; next_review_at?: string };
type MemoryState = {
  digest?: string;
  full_check_at?: number;
  next_deadline?: string | null;
  next_review_at?: string | null;
  backlog?: boolean;
};
type State = {
  memory?: MemoryState;
  gathered_at?: number;
  manuals?: { seen_total?: number; looked_at?: number };
  tools?: { since_ms?: number; missing?: Record<string, string[]>; told?: string[] };
};
type Ctx = {
  now_ms: number;
  agent_root?: string | null;
  host: { call(method: string, params?: Record<string, unknown>): Promise<unknown> };
};

const BASELINE_MS = 2 * 3600 * 1000;
const GATHER_MS = 6 * 3600 * 1000;

const ENOUGH_NEW = 3;
const DAY_MS = 24 * 3600 * 1000;
const FIRST_LOOK_DAYS = 14;
const MAX_SESSIONS = 60;
const MAX_BYTES = 4 * 1024 * 1024;
const MAX_COMMANDS = 64;
const PROBE = /(?:\bwhich|\bcommand -v|\btype)((?:[ \t]+[A-Za-z0-9_.+-]+)+)/g;
const NOT_FOUND = /(?:bash|sh|zsh|dash)(?::[^:\n]*)?: ([A-Za-z0-9_.+-]+): (?:command )?not found/g;

// deno-lint-ignore no-explicit-any
async function readJson(path: string): Promise<any> {
  try {
    return JSON.parse(await Deno.readTextFile(path));
  } catch {
    return null;
  }
}

function due(at: string | null | undefined, now: number): boolean {
  return !!at && Date.parse(at) <= now;
}

async function memory(ctx: Ctx, s: MemoryState) {
  const now = ctx.now_ms;
  const quick = (await ctx.host.call("memory.status")) as Status;
  const moved = quick.digest !== s.digest;
  const baseline = !s.full_check_at || now - s.full_check_at >= BASELINE_MS;
  if (!moved && !baseline && !s.backlog && !due(s.next_deadline, now) && !due(s.next_review_at, now)) {
    return { state: s };
  }
  const full = (await ctx.host.call("memory.status", { full: true })) as Status;
  const p = full.pending ?? {};
  const reviews = full.due_reviews ?? 0;
  const work = (p.new ?? 0) > 0 || (p.due ?? 0) > 0 || reviews > 0;
  const state: MemoryState = {
    digest: full.digest,
    full_check_at: now,
    next_deadline: p.next_deadline ?? null,
    next_review_at: full.next_review_at ?? null,
    backlog: work,
  };
  if (!work) return { state };
  let reason = `memory: ${p.new ?? 0} new, ${p.due ?? 0} due, ${p.waiting ?? 0} waiting, ${reviews} cognitions due for review`;
  if (p.oldest_at_ms) reason += `; oldest material ${new Date(p.oldest_at_ms).toISOString()}`;
  return { reason, state };
}

function manuals(record: { usage?: Record<string, Usage> }, now: number, s: NonNullable<State["manuals"]>) {
  const total = Object.values(record.usage ?? {}).reduce((n, u) => n + (u.frozen ?? 0), 0);
  const seen = Math.min(s.seen_total ?? 0, total);
  const fresh = total - seen;
  const quiet = !s.looked_at || now - s.looked_at >= 7 * DAY_MS;
  if (fresh >= ENOUGH_NEW || (fresh > 0 && quiet)) {
    return { reason: `manuals: ${fresh} new work session(s) since the last look (usage record .meta/skills_state.json)`, state: { seen_total: total, looked_at: now } };
  }
  return { state: { ...s, seen_total: seen } };
}

/** The commands one worklog shows as missing. */
function missingIn(worklog: string): Set<string> {
  const missing = new Set<string>();
  const probed = new Map<string, string[]>();
  let asked = "";
  for (const line of worklog.split("\n")) {
    const said = line.includes('"user_message"');
    const called = line.includes('"assistant_message"') && /which|command -v|type /.test(line);
    const ended = line.includes('"action_result"');
    if (!said && !called && !ended) continue;
    const entry = JSON.parse(line);
    if (entry.t === "user_message" && asked.length < 64 * 1024) asked += `\n${entry.content ?? ""}`;
    if (entry.t === "assistant_message") {
      for (const c of entry.tool_calls ?? []) {
        const names = [...String(c.args?.command ?? "").matchAll(PROBE)].flatMap((m) => m[1].trim().split(/\s+/)).filter((n) => /^[A-Za-z_]/.test(n));
        if (names.length > 0) probed.set(c.call_id, names);
      }
    }
    if (entry.t !== "action_result") continue;
    const result = String(entry.result ?? "");
    for (const m of result.matchAll(NOT_FOUND)) missing.add(m[1]);
    for (const name of probed.get(entry.call_id) ?? []) {
      const word = new RegExp(`(^|[^A-Za-z0-9_.+-])${name.replace(/[.+]/g, "\\$&")}($|[^A-Za-z0-9_.+-])`);
      const there = result.split("\n").some((l) => l.trim().endsWith(`/${name}`) || l.includes(`${name} is `));
      if (!there && word.test(asked)) missing.add(name);
    }
  }
  return missing;
}

async function tools(root: string, now: number, s: NonNullable<State["tools"]>) {
  const since = s.since_ms ?? now - FIRST_LOOK_DAYS * DAY_MS;
  const missing: Record<string, string[]> = { ...(s.missing ?? {}) };
  const ended: { sid: string; at: number; location: string }[] = [];
  try {
    for await (const e of Deno.readDir(`${root}/state/sessions`)) {
      if (!e.isFile || !e.name.startsWith("work-") || !e.name.endsWith(".json")) continue;
      const path = `${root}/state/sessions/${e.name}`;
      const changed = (await Deno.stat(path)).mtime?.getTime() ?? now;
      if (changed < since) continue;
      const entry = await readJson(path);
      const at = entry?.status?.updated_at_ms ?? 0;
      if (entry?.kind === "work" && entry.status?.run_state === "finished" && at > since) {
        ended.push({ sid: entry.session_id, at, location: entry.location });
      }
    }
  } catch {
    return { state: s };
  }
  ended.sort((a, b) => a.at - b.at);
  const look = ended.slice(0, MAX_SESSIONS);
  for (const w of look) {
    const path = `${w.location}/.opendan_agent_session/worklog.jsonl`;
    try {
      if ((await Deno.stat(path)).size > MAX_BYTES) continue;
      for (const cmd of missingIn(await Deno.readTextFile(path))) {
        const sids = missing[cmd] ?? [];
        if (!sids.includes(w.sid)) missing[cmd] = [...sids, w.sid].slice(-8);
      }
    } catch {
      continue;
    }
  }
  const told = s.told ?? [];
  const due = Object.entries(missing).filter(([cmd, sids]) => sids.length >= 2 && !told.includes(cmd));
  const state = {
    since_ms: look.length > 0 ? look[look.length - 1].at : since,
    missing: Object.fromEntries(Object.entries(missing).slice(-MAX_COMMANDS)),
    told: [...told, ...due.map(([cmd]) => cmd)].slice(-MAX_COMMANDS),
  };
  if (due.length === 0) return { state };
  const names = due.map(([cmd, sids]) => `\`${cmd}\` not found by ${sids.length} work sessions (${sids.join(", ")})`);
  return { reason: `tools: ${names.join("; ")}`, state };
}

export default async function check(ctx: Ctx, state: State | null) {
  const s: State = state ?? {};
  if (!ctx.agent_root) return { run: false, state: s };
  const record = (await readJson(`${ctx.agent_root}/.meta/skills_state.json`)) ?? {};
  const on = (duty: string) => record.self_improve?.[duty] !== false;
  const reasons: string[] = [];
  const next: State = { ...s };
  if (on("memory")) {
    const m = await memory(ctx, s.memory ?? {});
    next.memory = m.state;
    if (m.reason) reasons.push(m.reason);
  }
  if (!s.gathered_at || ctx.now_ms - s.gathered_at >= GATHER_MS) {
    next.gathered_at = ctx.now_ms;
    if (on("manuals")) {
      const m = manuals(record, ctx.now_ms, s.manuals ?? {});
      next.manuals = m.state;
      if (m.reason) reasons.push(m.reason);
    }
    if (on("tools")) {
      const t = await tools(ctx.agent_root, ctx.now_ms, s.tools ?? {});
      next.tools = t.state;
      if (t.reason) reasons.push(t.reason);
    }
  }
  return { run: reasons.length > 0, reason: reasons.join("\n"), state: next };
}
