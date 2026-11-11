// Check of the built-in Memory maintenance goal (its initial version: the
// goal may rewrite it with `agent-session goal check set`).
//
// The host runs it every 30 minutes, before any model call: it answers
// `run: true` only when Memory holds material for a consolidation (new or due
// perceptions, cognitions due for review). The cheap version vector of Memory
// is read first; the full check runs only when something moved, a deadline or
// review came, a backlog was seen, or every 2 hours as a baseline.
// Mechanical work (unfinished receipts, cleanup) is the host's and needs no
// `true` here.

type Pending = { new?: number; due?: number; waiting?: number; next_deadline?: string; oldest_at_ms?: number };
type Status = { digest: string; pending?: Pending; due_reviews?: number; next_review_at?: string };
type State = {
  digest?: string;
  full_check_at?: number;
  next_deadline?: string | null;
  next_review_at?: string | null;
  backlog?: boolean;
};

const BASELINE_MS = 2 * 3600 * 1000;

function due(at: string | null | undefined, now: number): boolean {
  return !!at && Date.parse(at) <= now;
}

export default async function check(
  ctx: { now_ms: number; host: { call(method: string, params?: Record<string, unknown>): Promise<unknown> } },
  state: State | null,
) {
  const s: State = state ?? {};
  const now = ctx.now_ms;
  const quick = (await ctx.host.call("memory.status")) as Status;
  const moved = quick.digest !== s.digest;
  const baseline = !s.full_check_at || now - s.full_check_at >= BASELINE_MS;
  if (!moved && !baseline && !s.backlog && !due(s.next_deadline, now) && !due(s.next_review_at, now)) {
    return { run: false, state: s };
  }
  const full = (await ctx.host.call("memory.status", { full: true })) as Status;
  const p = full.pending ?? {};
  const reviews = full.due_reviews ?? 0;
  const work = (p.new ?? 0) > 0 || (p.due ?? 0) > 0 || reviews > 0;
  const next: State = {
    digest: full.digest,
    full_check_at: now,
    next_deadline: p.next_deadline ?? null,
    next_review_at: full.next_review_at ?? null,
    backlog: work,
  };
  if (!work) {
    return { run: false, state: next };
  }
  let reason = `memory maintenance: ${p.new ?? 0} new, ${p.due ?? 0} due, ${p.waiting ?? 0} waiting, ${reviews} cognitions due for review`;
  if (p.oldest_at_ms) {
    reason += `; oldest material ${new Date(p.oldest_at_ms).toISOString()}`;
  }
  return { run: true, reason, state: next };
}
