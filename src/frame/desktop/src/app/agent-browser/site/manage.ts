/* What the owner manages about the personal site on the maintaining Agent's own service, with the owner's session:
 * the jobs of its maintenance Goal (`goals.jobs`, `goals.job_run`, `goals.job_set_enabled`: Agent State Protocol
 * §9.9), the Goal's recent changes (`goals.changes`), and the record of what the owner browsed (`browser.visits*`:
 * Agent Browser Protocol §7.3). No Agent is asked and no tokens are used. */

import { getBrowserBackend, type BrowserAgent } from '../api/backend'

/** How a run of a job ended. */
export type JobRunStatus = 'ok' | 'failed' | 'timeout' | 'killed'

/** A job as `goals.jobs` shows it, as far as the App looks at it. */
export interface Job {
  job_id: string
  title: string
  enabled: boolean
  /** A run is under way. */
  running: boolean
  last_run: { at_ms: number; ended_at_ms: number; status: JobRunStatus; kind: string; summary: string } | null
  /** Runs in a row that did not succeed. */
  failures: number
  next_at_ms: number | null
}

/** One line of the Goal's `changes.jsonl`. */
export interface GoalChange {
  seq: number
  at_ms: number
  /** `created | revised | status | reopened | restarted | check | job | epoch_ended`. */
  kind: string
  /** `owner`, `opendan`, or `ui_session | work_session | goal_session` without the session's id. */
  via: string
  reason: string
  /** What the change was about: the fields revised, the states, the job. */
  detail: string
}

/** A page the owner looked at in the Agent's browser. */
export interface Visit {
  url: string
  title: string
  last_at_ms: number
  dwell_ms: number
  count: number
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === 'object' && !Array.isArray(value)
}

const text = (value: unknown): string => (typeof value === 'string' ? value : '')
const count = (value: unknown): number => (typeof value === 'number' && Number.isFinite(value) ? value : 0)

function parseJob(value: unknown): Job | null {
  if (!isRecord(value) || typeof value.job_id !== 'string') return null
  const run = isRecord(value.last_run) ? value.last_run : null
  const status = run?.status === 'ok' || run?.status === 'timeout' || run?.status === 'killed' ? run.status : 'failed'
  return {
    job_id: value.job_id,
    title: text(value.title) || value.job_id,
    enabled: value.enabled !== false,
    running: isRecord(value.running),
    last_run: run && { at_ms: count(run.at_ms), ended_at_ms: count(run.ended_at_ms), status, kind: text(run.kind), summary: text(run.summary) },
    failures: count(value.failures),
    next_at_ms: typeof value.next_at_ms === 'number' ? value.next_at_ms : null,
  }
}

function changeDetail(kind: string, detail: unknown): string {
  if (!isRecord(detail)) return ''
  if (kind === 'revised' && Array.isArray(detail.fields)) return detail.fields.filter((field) => typeof field === 'string').join(', ')
  if (kind === 'status' && typeof detail.to === 'string') return `${text(detail.from)} → ${detail.to}`
  if (kind === 'job') return [text(detail.job_id), text(detail.what)].filter(Boolean).join(': ')
  return ''
}

function parseChange(value: unknown): GoalChange | null {
  if (!isRecord(value) || typeof value.kind !== 'string') return null
  return { seq: count(value.seq), at_ms: count(value.at_ms), kind: value.kind, via: text(value.via).split(':', 1)[0], reason: text(value.reason), detail: changeDetail(value.kind, value.detail) }
}

function parseVisit(value: unknown): Visit | null {
  if (!isRecord(value) || typeof value.url !== 'string') return null
  return { url: value.url, title: text(value.title), last_at_ms: count(value.last_at_ms), dwell_ms: count(value.dwell_ms), count: count(value.count) }
}

const list = <T,>(value: unknown, parse: (item: unknown) => T | null): T[] => (Array.isArray(value) ? value : []).map(parse).filter((item): item is T => item !== null)

async function call(agent: BrowserAgent, method: string, params: Record<string, unknown>): Promise<unknown> {
  return (await getBrowserBackend()).call<unknown>(agent, method, params)
}

/** `goals.jobs`. */
export async function readJobs(agent: BrowserAgent, goalId: string): Promise<Job[]> {
  return list(await call(agent, 'goals.jobs', { goal_id: goalId }), parseJob)
}

/** `goals.job_run`: asks the host for a run now; it shows up in `goals.jobs` when it started. */
export async function runJob(agent: BrowserAgent, goalId: string, jobId: string): Promise<void> {
  await call(agent, 'goals.job_run', { goal_id: goalId, job_id: jobId })
}

/** `goals.job_set_enabled`. */
export async function setJobEnabled(agent: BrowserAgent, goalId: string, jobId: string, enabled: boolean): Promise<void> {
  await call(agent, 'goals.job_set_enabled', { goal_id: goalId, job_id: jobId, enabled })
}

/** `goals.changes`: the newest `limit`, newest first. */
export async function readChanges(agent: BrowserAgent, goalId: string, limit: number): Promise<GoalChange[]> {
  return list(await call(agent, 'goals.changes', { goal_id: goalId, limit }), parseChange).reverse()
}

/** `browser.visits`: whether pages are recorded, and the ones recorded, newest first. */
export async function readVisits(agent: BrowserAgent): Promise<{ enabled: boolean; visits: Visit[] }> {
  const answer = await call(agent, 'browser.visits', {})
  return { enabled: isRecord(answer) && answer.enabled === true, visits: list(isRecord(answer) ? answer.visits : null, parseVisit) }
}

/** `browser.visits_set_enabled`. */
export async function setVisitsEnabled(agent: BrowserAgent, enabled: boolean): Promise<void> {
  await call(agent, 'browser.visits_set_enabled', { enabled })
}

/** `browser.visits_clear`. */
export async function clearVisits(agent: BrowserAgent): Promise<void> {
  await call(agent, 'browser.visits_clear', {})
}

/** The service's own sentence in a failed call (the wire error is JSON inside the RPC error's message), for showing why. */
export function callFailure(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error ?? '')
  const start = message.indexOf('{')
  if (start >= 0) {
    try {
      const wire = JSON.parse(message.slice(start)) as { message?: unknown }
      if (typeof wire.message === 'string') return wire.message
    } catch { /* not the wire form */ }
  }
  return message
}
