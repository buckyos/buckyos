/* What "manage" of the personal site calls on the mock Agent Service: the jobs of the maintenance Goal
 * (`goals.jobs`, `goals.job_run`, `goals.job_set_enabled`) and its changes (`goals.changes`), and the record of the
 * pages the owner looked at (`browser.visits*`). The jobs are the ones the site's sources name in the mock bucket's
 * `site.json`; a run "syncs" its source by writing its feed, the way a job of the real Goal does. */

import { mockReflowControl } from './reflow'

const RUN_MS = 600
const HOUR = 3_600_000

interface MockRun { run_id: string; version: number; at_ms: number; ended_at_ms: number; status: string; kind?: string; summary: string; trigger: string; attempts: number; tokens: number }
interface MockJob {
  schema: string; goal_id: string; job_id: string; title: string; version: number; command: string[]; every_secs: number; timeout_secs: number
  enabled: boolean; updated_at_ms: number; updated_by: string
  running?: { run_id: string; version: number; at_ms: number; trigger: string }
  last_run?: MockRun
  failures: number
  next_at_ms?: number
  versions: number[]
}
interface MockChange { seq: number; at_ms: number; kind: string; who: string; via: string; reason?: string; revision?: number; epoch?: number; detail?: unknown }
interface Source { id: string; title: string; job?: string; every_secs?: number; state?: string }
interface Feed { status: Record<string, unknown> & { state?: string; checked_at?: number | null; error?: { kind?: string; message?: string } | null }; updated_at?: number }

const jobs = new Map<string, Map<string, MockJob>>()
const changes = new Map<string, MockChange[]>()
/** How the next run of a job ends instead of well: by job id. */
const failing = new Map<string, { kind: string; summary: string }>()
let runs = 0

const json = <T,>(key: string): T | null => {
  try { return JSON.parse(mockReflowControl.read(key) ?? 'null') as T | null } catch { return null }
}
const feedKey = (source: string) => `_data/feed/${source}.json`
const sourceOfJob = (jobId: string): Source | undefined => json<{ sources?: Source[] }>('_data/site.json')?.sources?.find((source) => (source.job || `src-${source.id}`) === jobId)

function notFound(what: string): Error {
  return Error(`RPC call error: not_found: ${what}`)
}

/** The Goal's jobs: one for each source of the site this Goal maintains that got as far as being installed, and the hourly summary of the reading records. */
function jobsOf(goalId: string): Map<string, MockJob> {
  const site = json<{ sources?: Source[]; maintainer?: { goal_id?: string } | null }>('_data/site.json')
  let known = jobs.get(goalId)
  if (!known) jobs.set(goalId, known = new Map())
  if (site?.maintainer?.goal_id !== goalId) return known
  const now = Date.now()
  const install = (jobId: string, title: string, command: string[], every: number, enabled: boolean, last: MockRun | undefined) => {
    if (known.has(jobId)) return
    known.set(jobId, {
      schema: 'opendan.goal_job/1', goal_id: goalId, job_id: jobId, title, version: 1, command, every_secs: every, timeout_secs: 600, enabled,
      updated_at_ms: now - 26 * HOUR, updated_by: 'work_session:ws-20261123T010203-0a1b2c3d', failures: last && last.status !== 'ok' ? 2 : 0, versions: [1],
      ...(last ? { last_run: last } : {}), ...(enabled ? { next_at_ms: now + Math.min(every, 1800) * 1000 } : {}),
    })
  }
  for (const source of site.sources ?? []) {
    if (source.state === 'failed' || source.state === 'preparing') continue
    const feed = json<Feed>(feedKey(source.id))
    const at = feed?.status.checked_at ?? feed?.updated_at ?? now - HOUR
    const failed = feed && feed.status.state !== 'ok' && feed.status.state !== 'preparing'
    const last: MockRun | undefined = feed ? {
      run_id: `run-${++runs}`, version: 1, at_ms: at - 40_000, ended_at_ms: at, trigger: 'schedule', attempts: 1, tokens: failed ? 0 : 18_400,
      status: failed ? 'failed' : 'ok', ...(failed ? { kind: feed.status.error?.kind ?? 'program' } : {}),
      summary: failed ? feed.status.error?.message ?? 'the sync failed' : 'listed 12, published 3',
    } : undefined
    install(source.job || `src-${source.id}`, `Sync ${source.title}`, ['site', 'sync', source.id, '--dir', '{dir}'], source.every_secs ?? 1800, source.state !== 'paused', last)
  }
  install('site-reading', 'Sum up the reading records', ['site', 'reading', '--compact'], 3600, true, { run_id: `run-${++runs}`, version: 1, at_ms: now - 20 * 60_000, ended_at_ms: now - 20 * 60_000 + 900, status: 'ok', summary: 'read 4 batches', trigger: 'schedule', attempts: 1, tokens: 0 })
  return known
}

function changesOf(goalId: string): MockChange[] {
  let list = changes.get(goalId)
  if (!list) {
    const now = Date.now()
    const work = 'work_session:ws-20261123T010203-0a1b2c3d'
    list = [
      { seq: 1, at_ms: now - 50 * HOUR, kind: 'created', who: 'did:bns:alice', via: work, revision: 1, epoch: 1 },
      { seq: 2, at_ms: now - 50 * HOUR + 60_000, kind: 'job', who: 'did:bns:alice', via: work, epoch: 1, detail: { job_id: 'src-news-yahoo-co-jp', what: 'put', version: 1 } },
      { seq: 3, at_ms: now - 26 * HOUR, kind: 'revised', who: 'did:bns:alice', via: 'work_session:ws-20261124T080910-4e5f6a7b', reason: 'Hacker News was added to the reading site', revision: 2, epoch: 1, detail: { fields: ['objective', 'healthy_state'] } },
      { seq: 4, at_ms: now - 26 * HOUR + 60_000, kind: 'job', who: 'did:bns:alice', via: 'work_session:ws-20261124T080910-4e5f6a7b', epoch: 1, detail: { job_id: 'src-news-ycombinator-com', what: 'put', version: 1 } },
      { seq: 5, at_ms: now - 3 * HOUR, kind: 'revised', who: 'did:bns:alice', via: 'ui_session:ui-browser', reason: 'The owner does not want sports news', revision: 3, epoch: 1, detail: { fields: ['constraints'] } },
    ]
    changes.set(goalId, list)
  }
  return list
}

function must(goalId: string, jobId: string): MockJob {
  const job = jobsOf(goalId).get(jobId)
  if (!job) throw notFound(`job ${jobId} of goal ${goalId}`)
  return job
}

/** A run ends: its record, and what a sync leaves in the source's feed. */
function finish(job: MockJob) {
  const now = Date.now()
  const failure = failing.get(job.job_id)
  failing.delete(job.job_id)
  job.last_run = {
    run_id: job.running?.run_id ?? `run-${++runs}`, version: job.version, at_ms: job.running?.at_ms ?? now, ended_at_ms: now, trigger: 'manual', attempts: 1, tokens: 0,
    status: failure ? 'failed' : 'ok', ...(failure ? { kind: failure.kind } : {}), summary: failure?.summary ?? 'listed 12, published 0',
  }
  job.failures = failure ? job.failures + 1 : 0
  delete job.running
  const source = sourceOfJob(job.job_id)
  const feed = source && json<Feed>(feedKey(source.id))
  if (!source) return
  const status = failure
    ? { ...feed?.status, state: failure.kind === 'auth' ? 'needs_login' : 'failed', error: { kind: failure.kind, message: failure.summary, at: now } }
    : { ...feed?.status, state: 'ok', checked_at: now, published_at: feed?.status.published_at ?? now, error: null, pending: null, login_url: null }
  mockReflowControl.put(feedKey(source.id), JSON.stringify({ schema: 'buckyos.site.feed/1', source: source.id, items: [], ...feed, status, updated_at: now }))
}

const visits = new Map<string, { enabled: boolean; visits: Array<{ url: string; title: string; first_at_ms: number; last_at_ms: number; dwell_ms: number; count: number }> }>()

function visitsOf(agentId: string) {
  let record = visits.get(agentId)
  if (!record) {
    const now = Date.now()
    const visit = (url: string, title: string, minutesAgo: number, dwell: number, count: number) => ({ url, title, first_at_ms: now - 5 * 24 * HOUR, last_at_ms: now - minutesAgo * 60_000, dwell_ms: dwell, count })
    record = {
      enabled: true,
      visits: [
        visit('https://www3.nhk.or.jp/news/', 'NHKニュース', 12, 540_000, 6),
        visit('https://arstechnica.com/gadgets/', 'Gadgets | Ars Technica', 95, 1_260_000, 4),
        visit('https://www.dmv.example.org/licence/renew', 'Renew a licence', 300, 380_000, 2),
        visit('https://news.ycombinator.com/item', '', 1500, 42_000, 9),
      ],
    }
    visits.set(agentId, record)
  }
  return record
}

/** `goals.*` and `browser.visits*` of the mock Agent Service; undefined: not one of them. */
export function mockManageCall(agentId: string, method: string, params: Record<string, unknown>): unknown {
  const goalId = String(params.goal_id ?? '')
  const jobId = String(params.job_id ?? '')
  switch (method) {
    case 'goals.jobs':
      return structuredClone([...jobsOf(goalId).values()])
    case 'goals.job_run': {
      const job = must(goalId, jobId)
      const requested = { request_id: `req-${++runs}`, by: 'owner', at_ms: Date.now() }
      if (!job.running) {
        job.running = { run_id: `run-${++runs}`, version: job.version, at_ms: Date.now(), trigger: 'manual' }
        window.setTimeout(() => finish(job), RUN_MS)
      }
      return requested
    }
    case 'goals.job_set_enabled': {
      const job = must(goalId, jobId)
      job.enabled = params.enabled === true
      job.updated_at_ms = Date.now()
      job.updated_by = 'owner'
      if (job.enabled) job.next_at_ms = Date.now() + job.every_secs * 1000
      else delete job.next_at_ms
      const list = changesOf(goalId)
      list.push({ seq: list.length + 1, at_ms: Date.now(), kind: 'job', who: 'did:bns:alice', via: 'owner', epoch: 1, detail: { job_id: jobId, what: job.enabled ? 'enable' : 'disable', version: job.version } })
      return structuredClone({ ...job, running: undefined, last_run: undefined })
    }
    case 'goals.changes': {
      const limit = typeof params.limit === 'number' ? params.limit : 0
      return structuredClone(limit > 0 ? changesOf(goalId).slice(-limit) : changesOf(goalId))
    }
    case 'browser.visits': {
      const record = visitsOf(agentId)
      const since = typeof params.since_ms === 'number' ? params.since_ms : 0
      return structuredClone({ enabled: record.enabled, visits: record.visits.filter((visit) => visit.last_at_ms >= since) })
    }
    case 'browser.visits_set_enabled':
      visitsOf(agentId).enabled = params.enabled === true
      return { enabled: params.enabled === true }
    case 'browser.visits_clear':
      visitsOf(agentId).visits = []
      return {}
    default:
      return undefined
  }
}

/** The Agent's side of the jobs, for the specs. */
export const mockManageControl = {
  /** The next run of this job fails: `auth` leaves its source waiting for a sign-in, anything else failed. */
  failJob(jobId: string, kind: string, summary: string) {
    failing.set(jobId, { kind, summary })
  },
  job: (goalId: string, jobId: string) => structuredClone(jobsOf(goalId).get(jobId) ?? null),
  visits: (agentId: string) => structuredClone(visitsOf(agentId)),
}
