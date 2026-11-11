// Check of the built-in skill crystallization goal (its initial version: the
// goal may rewrite it with `agent-session goal check set`).
//
// The host runs it once a day, before any model call: it answers `run: true`
// only when enough work was done since the goal last looked at the agent's
// manuals — the usage record the host keeps (`.meta/skills_state.json`:
// every work session counts once for each manual it was given, or under
// `(none)` without one). A look costs a lot of tokens; a few new work
// sessions, or a week with at least one, are worth it.

type Usage = { frozen?: number };
type State = { seen_total?: number; looked_at?: number };

const ENOUGH_NEW = 3;
const WEEK_MS = 7 * 24 * 3600 * 1000;

export default async function check(ctx: { now_ms: number; agent_root?: string }, state: State | null) {
  const s: State = state ?? {};
  if (!ctx.agent_root) {
    return { run: false, state: s };
  }
  let total = 0;
  try {
    const record = JSON.parse(await Deno.readTextFile(`${ctx.agent_root}/.meta/skills_state.json`));
    for (const u of Object.values((record.usage ?? {}) as Record<string, Usage>)) {
      total += u.frozen ?? 0;
    }
  } catch {
    return { run: false, state: s };
  }
  const seen = s.seen_total ?? 0;
  const fresh = total - seen;
  const quiet = !s.looked_at || ctx.now_ms - s.looked_at >= WEEK_MS;
  if (fresh >= ENOUGH_NEW || (fresh > 0 && quiet)) {
    return {
      run: true,
      reason: `${fresh} work session(s) used manuals since the last look (usage record .meta/skills_state.json)`,
      state: { seen_total: total, looked_at: ctx.now_ms },
    };
  }
  return { run: false, state: { ...s, seen_total: seen > total ? total : seen } };
}
