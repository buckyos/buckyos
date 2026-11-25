// What the owner looked at in this Agent's browser: the records of
// `browser.visits` (Agent Browser Protocol §7.3), for the Agent that keeps the
// reading site to see what else the owner reads.

export interface Visit {
  /** The address without query and fragment. */
  url: string;
  title: string;
  first_at_ms: number;
  last_at_ms: number;
  /** How long it was the tab the owner was looking at. */
  dwell_ms: number;
  /** How many times the owner arrived at it. */
  count: number;
}

export interface VisitsAnswer {
  /** false: the owner switched the recording off; what was recorded before is still there. */
  enabled: boolean;
  visits: Visit[];
}

export interface HostVisits {
  host: string;
  pages: number;
  count: number;
  dwell_ms: number;
  last_at_ms: number;
}

const PAGES_SHOWN = 50;
const when = (ms: number) => new Date(ms).toISOString();
const seconds = (ms: number) => `${Math.round(ms / 1000)}s`;

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/** The records of the last `days` as the command shows them: by host (most time first), then page by page (latest first). */
export function describeVisits(answer: VisitsAnswer, days: number) {
  const visits = answer.visits;
  const byHost = new Map<string, HostVisits>();
  for (const visit of visits) {
    const host = hostOf(visit.url);
    const entry = byHost.get(host) ?? { host, pages: 0, count: 0, dwell_ms: 0, last_at_ms: 0 };
    entry.pages += 1;
    entry.count += visit.count;
    entry.dwell_ms += visit.dwell_ms;
    entry.last_at_ms = Math.max(entry.last_at_ms, visit.last_at_ms);
    byHost.set(host, entry);
  }
  const hosts = [...byHost.values()].sort((a, b) => b.dwell_ms - a.dwell_ms || (a.host < b.host ? -1 : 1));
  const recording = answer.enabled
    ? "recording is on"
    : "recording is switched off (the owner turned it off): nothing new is recorded, what was recorded before is listed";
  const lines = [
    visits.length
      ? `${recording}; ${visits.length} page(s) on ${hosts.length} host(s) in the last ${days} day(s)`
      : `${recording}; nothing was recorded in the last ${days} day(s)`,
    ...hosts.map((entry) =>
      `host ${entry.host}  ${entry.pages} page(s), ${entry.count} visit(s), ${seconds(entry.dwell_ms)}, last ${when(entry.last_at_ms)}`
    ),
    ...visits.slice(0, PAGES_SHOWN).map((visit) =>
      `${visit.url}  ${visit.title ? JSON.stringify(visit.title) : "(no title)"}  ${visit.count} visit(s), ${
        seconds(visit.dwell_ms)
      }, last ${when(visit.last_at_ms)}`
    ),
    ...(visits.length > PAGES_SHOWN ? [`… and ${visits.length - PAGES_SHOWN} more page(s): --json has all of them`] : []),
  ];
  return { summary: lines.join("\n"), detail: { days, enabled: answer.enabled, hosts, visits } };
}
