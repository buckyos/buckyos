import type { GoalSource } from '../goals/source'
import { parseActivations, parseGoal, parseGoals, parseGoalSchedule, parseGoalState, parseOpenActivation, type ActivationReason } from '../goals/model'
import type { WorklogEntry } from '../worklog/model'
import type { WorkNode } from '../worksessions/model'
import { mockWorklogs, mockWorklogSource } from './worklog'

const AGENT = 'did:buckyos:agent:codeassistant'
const MINUTE = 60_000
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR

interface MockActivation { epoch: number; seq: number; session_id: string; started_at_ms: number; ended_at_ms: number; reasons: ActivationReason[]; outcome: string; decision: string; summary: string; works_created: string[]; cost: { runs: number; total_tokens: number }; end_reason?: string }
interface MockQuestion { text: string; at_ms: number; epoch: number; activation: number; notify_id?: string; status?: string; answer?: string; options?: Array<{ id: string; label: string }>; allow_text?: boolean }

/** A Goal as the service keeps it: `goals.*` answers with these values. */
interface MockGoal {
  definition: Record<string, unknown> & { goal_id: string; title: string; status: string; session: { epoch: number; sid: string } }
  state: { usage: { period_start_ms: number; tokens: number; work_tokens: number; activations: number; works: number }; questions: MockQuestion[]; holding?: { reason: string; since_ms: number; until_ms?: number }; last_activation?: unknown; distance?: { text: string; observed_at_ms: number }; pending: string[]; active_works: Array<{ sid: string; objective: string }> }
  schedule: Record<string, unknown>
  activations: MockActivation[]
  open: { seq: number; reasons: ActivationReason[]; started_at_ms: number; line: string } | null
  works: WorkNode[]
  /** Messages that wait for the gate. */
  waiting: ActivationReason[]
}

interface Seed {
  id: string
  title: string
  objective: string
  status?: string
  budget?: number
  builtin?: string
  achieved_when?: string
  healthy_state?: string
  updatedMinutesAgo: number
  endedMinutesAgo?: number
  used?: [number, number]
}

const sessionOf = (goalId: string, epoch = 1) => `${goalId.replace(/^goal-/, 'goalsession-')}-e${epoch}`

function goal(seed: Seed, now: number): MockGoal {
  return {
    definition: {
      schema: 'opendan.goal/1', goal_id: seed.id, revision: 2, title: seed.title, objective: seed.objective, lifecycle: seed.achieved_when ? 'achieve' : 'maintain',
      ...(seed.achieved_when ? { achieved_when: seed.achieved_when } : {}), ...(seed.healthy_state ? { healthy_state: seed.healthy_state } : {}),
      owner: 'self', created_by: seed.builtin ? { principal: 'opendan', via: 'opendan' } : { principal: 'self', via: 'ui_session:ui-session-work' },
      ...(seed.budget ? { budget: { activation: {}, period: { window_secs: 86_400, max_total_tokens: seed.budget } } } : {}),
      status: seed.status ?? 'active', status_changed_at_ms: now - (seed.endedMinutesAgo ?? seed.updatedMinutesAgo) * MINUTE,
      session: { epoch: 1, sid: sessionOf(seed.id) }, epochs: [], created_at_ms: now - 20 * DAY, updated_at_ms: now - seed.updatedMinutesAgo * MINUTE,
      extensions: seed.builtin ? { opendan: { builtin: seed.builtin, policy_version: 1 } } : {},
    },
    state: { usage: { period_start_ms: Math.floor(now / DAY) * DAY, tokens: seed.used?.[0] ?? 0, work_tokens: seed.used?.[1] ?? 0, activations: 0, works: 0 }, questions: [], pending: [], active_works: [] },
    schedule: { schema: 'opendan.goal_schedule/1', goal_id: seed.id, backend: 'local', check_errors: 0 },
    activations: [],
    open: null,
    works: [],
    waiting: [],
  }
}

const header = (item: MockGoal, seq: number): Omit<WorklogEntry, 'seq'> => ({ t: 'user_message', run_id: `run-${seq}`, brief: `goal activation ${seq}: header not repeated in history`, content: `<goal_activation goal="${item.definition.goal_id}" activation="${seq}">\n…\n</goal_activation>` })
const messageXml = (key: string, text: string, from = 'self') => `<msg key="${key}" from="feedback from ${from}" from_id="codeassistant" time="2026-11-24T08:00:00Z">\n${text.replace(/&/g, '&amp;').replace(/</g, '&lt;')}\n</msg>`

/** What the Turn of an Activation recorded: the header, what was said, two tool calls, the report. */
function seedTurn(item: MockGoal, seq: number, startedAt: number, messages: string[], report: string, complete: boolean) {
  const sid = item.definition.session.sid
  const run_id = `run-${seq}`
  mockWorklogs.seed(sid, seq, [
    { t: 'turn_started', run_id, at_ms: startedAt },
    header(item, seq),
    ...messages.map(content => ({ t: 'user_message', run_id, content })),
    { t: 'assistant_message', run_id, assistant: 'Looking at what changed since the last time.', tool_calls: [{ call_id: `${seq}-look`, tool: 'shell', args: { command: 'git -C project log --since=yesterday --oneline', description: 'List what changed' } }] },
    { t: 'action_result', run_id, call_id: `${seq}-look`, status: 'ok', result: '3f2c1ab Fix the import order\n9be7710 Update the release notes' },
    ...(complete ? [
      { t: 'assistant_message', run_id, assistant: '', tool_calls: [{ call_id: `${seq}-read`, tool: 'shell', args: { command: 'cat reports/latest.md', description: 'Read the last report' } }] },
      { t: 'action_result', run_id, call_id: `${seq}-read`, status: 'ok', result: '# Weekly report' },
      { t: 'outcome', run_id, kind: 'done', report },
      { t: 'turn_ended', run_id, status: 'completed', at_ms: startedAt + 2 * MINUTE },
    ] : []),
  ], complete)
}

interface ActivationSeed { minutesAgo: number; took?: number; reasons: ActivationReason[]; outcome?: string; decision: string; summary: string; tokens: number; works?: string[]; end_reason?: string; messages?: string[] }

function activate(item: MockGoal, now: number, seeds: ActivationSeed[]) {
  for (const seed of seeds) {
    const seq = item.activations.length + 1
    const started = now - seed.minutesAgo * MINUTE
    const record: MockActivation = { epoch: 1, seq, session_id: item.definition.session.sid, started_at_ms: started, ended_at_ms: started + (seed.took ?? 130) * 1000, reasons: seed.reasons, outcome: seed.outcome ?? 'completed', decision: seed.decision, summary: seed.summary, works_created: seed.works ?? [], cost: { runs: 1, total_tokens: seed.tokens }, ...(seed.end_reason ? { end_reason: seed.end_reason } : {}) }
    item.activations.push(record)
    item.state.last_activation = { epoch: 1, seq, at_ms: record.ended_at_ms, reasons: seed.reasons, outcome: record.outcome, decision: record.decision, summary: record.summary }
    seedTurn(item, seq, started, seed.messages ?? [], seed.summary, true)
  }
}

const scheduled = (key: string, why: string, missed?: number): ActivationReason => ({ kind: 'scheduled', key, note: `scheduled check (every 1h)${missed ? `; ${missed} slots passed since the last one` : ''}: ${why}`, ...(missed ? { missed } : {}) })

const REPORT = `The weekly report for **week 47** is ready.

- 14 commits merged, 3 of them in the gateway
- The build was red on Tuesday for 40 minutes; the cause was a stale lock file
- Two reviews have been waiting for more than three days

| Area | Commits | Open reviews |
| --- | --- | --- |
| gateway | 3 | 1 |
| desktop | 8 | 1 |
| docs | 3 | 0 |

I kept the wording of last week's report. The two reviews are listed at the end so you can decide whom to remind. Nothing else needs your attention before Friday, and I will look again when the next build finishes.`

function seedGoals(now: number): Map<string, MockGoal> {
  const weekly = goal({ id: 'goal-weekly-report', title: 'Weekly project report', objective: 'Keep a report of what happened in the project each week: what was merged, what broke, what waits for a review. Send it on the day the owner reads it, and say so early when something needs a decision.', budget: 1_200_000, healthy_state: 'A report of the current week exists and is no older than a day.', updatedMinutesAgo: 60, used: [92_000, 36_500] }, now)
  weekly.state.distance = { text: 'The report of week 47 is written; the day to send it is still open.', observed_at_ms: now - 70 * MINUTE }
  weekly.state.pending = ['Ask which day the report should go out', 'Compare the build times with last week']
  weekly.works = [
    { session_id: 'work-goal-notes', shape: 'simple', title: 'Collect the release notes of week 47', state: 'succeeded', created_at_ms: now - 26 * HOUR, finished_at_ms: now - 26 * HOUR + 9 * MINUTE, line: 'Collected 14 entries', usage: { input_tokens: 31_000, output_tokens: 2_600, rounds: 7, models: [] }, model: 'gpt-6.1-sol@cliproxy', subtree_usage: null, todos: null, children: [] },
    { session_id: 'work-goal-builds', shape: 'simple', title: 'Measure the build times', state: 'running', created_at_ms: now - 68 * MINUTE, finished_at_ms: null, line: 'running cargo', usage: { input_tokens: 9_000, output_tokens: 700, rounds: 2, models: [] }, model: 'gpt-6.1-sol@cliproxy', subtree_usage: null, todos: null, children: [] },
  ]
  weekly.state.active_works = [{ sid: 'work-goal-builds', objective: 'Measure the build times' }]
  activate(weekly, now, [
    { minutesAgo: 3 * 1440, reasons: [{ kind: 'created', key: 'weekly:created', note: 'goal goal-weekly-report was created' }], decision: 'acted', summary: 'Wrote the check script: it wakes me when the week has new merged commits, at most once an hour.', tokens: 86_400, took: 250 },
    { minutesAgo: 26 * 60, reasons: [scheduled('weekly:s1', '14 new commits since the last report', 3)], decision: 'created_work', summary: 'There is enough for this week’s report. Started a work session to collect the release notes.', tokens: 31_200, works: ['work-goal-notes'] },
    { minutesAgo: 25 * 60 + 40, reasons: [{ kind: 'child', key: 'weekly:c1', ref: 'work-goal-notes', note: 'finished' }], decision: 'acted', summary: REPORT, tokens: 44_800, took: 310 },
    { minutesAgo: 24 * 60, reasons: [scheduled('weekly:s2', 'the build turned green again')], decision: 'no_action', summary: 'The build is green again and nothing else changed. The report stays as it is.', tokens: 18_900, took: 48 },
    { minutesAgo: 5 * 60, reasons: [{ kind: 'feedback', key: 'weekly:f1', note: 'feedback from self' }], decision: 'acted', summary: 'Shortened the report: one line per area, details behind a link.', tokens: 27_300, messages: [messageXml('weekly:f1', 'Please keep it shorter & skip the docs area.')] },
    { minutesAgo: 72, reasons: [scheduled('weekly:s3', '2 reviews wait for more than three days')], decision: 'needs_info', outcome: 'completed', summary: 'The report is ready, but I do not know which day you read it. Asked you, and started measuring the build times meanwhile.', tokens: 33_600, works: ['work-goal-builds'] },
  ])
  weekly.state.questions = [
    { text: 'Which day should the weekly report go out?', at_ms: now - 70 * MINUTE, epoch: 1, activation: 6, notify_id: 'nt-agent-goal', status: 'open', allow_text: true },
    { text: 'Should the report also list the open reviews by person?', at_ms: now - 69 * MINUTE, epoch: 1, activation: 6, options: [{ id: 'yes', label: 'Yes, by person' }, { id: 'no', label: 'No, only the count' }] },
  ]

  const reminders = goal({ id: 'goal-reminders', title: 'Personal reminders', objective: 'Remind the owner of fixed things at the time they asked for, without looking at anything else.', updatedMinutesAgo: 600, used: [0, 0] }, now)
  activate(reminders, now, [{ minutesAgo: 9 * 1440, reasons: [{ kind: 'created', key: 'reminders:created', note: 'goal goal-reminders was created' }], decision: 'acted', summary: 'The check script sends the reminders on time; nothing is left for me to do.', tokens: 52_100 }])
  reminders.schedule.last_check = { at_ms: now - 40 * MINUTE, outcome: 'false', notified: 'Reminder: dentist appointment tomorrow at 9:00', script_version: 1, elapsed_ms: 40 }

  const prices = goal({ id: 'goal-prices', title: 'Electricity price watch', objective: 'Watch the hourly electricity prices and say when charging the car at home is cheap.', budget: 300_000, updatedMinutesAgo: 900, used: [188_000, 113_400] }, now)
  activate(prices, now, [
    { minutesAgo: 8 * 60, reasons: [scheduled('prices:s1', 'the price file changed')], decision: 'acted', summary: 'Cheap between 01:00 and 05:00 tonight. Sent you a note.', tokens: 61_000 },
    { minutesAgo: 4 * 60, reasons: [scheduled('prices:s2', 'the price file changed')], outcome: 'budget_exhausted', decision: 'unknown', summary: '', end_reason: 'the period budget is used up', tokens: 127_000, took: 420 },
  ])
  prices.state.holding = { reason: 'period_budget', since_ms: now - 2 * HOUR, until_ms: now + 3 * HOUR }

  const self = goal({ id: 'goal-self-improve', title: 'Self improvement', objective: 'Look at how the work of this agent went and make what it repeats cheaper: manuals, shortcuts, tools.', builtin: 'self_improve', budget: 1_800_000, updatedMinutesAgo: 4 * 1440, used: [12_000, 0] }, now)
  activate(self, now, [{ minutesAgo: 17 * 60, reasons: [scheduled('self:s1', 'the check passed')], decision: 'no_action', summary: 'Too few finished work sessions to learn from.', tokens: 12_000, took: 31 }])

  const sites = goal({ id: 'goal-sites', title: 'Keep my sites reachable', objective: 'Check the three sites every hour and repair what the owner allowed.', status: 'paused', updatedMinutesAgo: 3 * 1440 }, now)
  activate(sites, now, [{ minutesAgo: 3 * 1440, reasons: [scheduled('sites:s1', 'blog.example.com timed out')], outcome: 'failed', decision: 'unknown', summary: '', end_reason: 'the model provider refused the request', tokens: 9_000 }])

  const photos = goal({ id: 'goal-photos', title: 'Move the photos to the new disk', objective: 'Copy every photo to the new disk and verify the copies.', status: 'achieved', achieved_when: 'Every photo is on the new disk with the same checksum.', updatedMinutesAgo: 6 * 1440, endedMinutesAgo: 6 * 1440 }, now)
  activate(photos, now, [{ minutesAgo: 6 * 1440, reasons: [{ kind: 'manual', key: 'photos:m1', note: 'activated by self' }], decision: 'acted', summary: 'All 18,204 photos are on the new disk; the checksums match.', tokens: 22_400 }])
  const parcel = goal({ id: 'goal-parcel', title: 'Track the parcel', objective: 'Say when the parcel arrives.', status: 'archived', achieved_when: 'The parcel arrived.', updatedMinutesAgo: 12 * 1440, endedMinutesAgo: 12 * 1440 }, now)

  return new Map([weekly, reminders, prices, self, sites, photos, parcel].map(item => [item.definition.goal_id, item]))
}

let goals: Map<string, MockGoal> | null = null
const store = () => goals ??= seedGoals(Date.now())

function must(goalId: string): MockGoal {
  const found = store().get(goalId)
  if (!found) throw Error(`Goal ${goalId} not found`)
  return found
}

let posted = 0
const feedbackTexts = new Map<string, string>()

/** Scripted `goals.*` of the seeded Agent; tests drive it through `window.__mockGoals`. */
export const mockGoals = {
  /** Every call served so far, in order. */
  calls: [] as Array<{ method: string; params: Record<string, unknown> }>,
  /** The viewer is not the Agent's owner (or the Agent is offline): every call fails. */
  denied: false,
  /** How long what is said to a Goal waits before an Activation reads it, and how long that one runs. */
  wakeMs: 1_200,
  workMs: 2_500,
  get: (goalId: string) => store().get(goalId),
  activeCount: () => [...store().values()].filter(item => item.definition.status === 'active').length,
  hold(goalId: string, reason: string | null, until_ms?: number) {
    const item = must(goalId)
    item.state.holding = reason ? { reason, since_ms: Date.now(), until_ms } : undefined
    if (!reason && item.waiting.length > 0) mockGoals.open(goalId, item.waiting.splice(0))
  },
  open(goalId: string, reasons: ActivationReason[]) {
    const item = must(goalId)
    const seq = item.activations.length + 1
    item.open = { seq, reasons, started_at_ms: Date.now(), line: 'working (goal)' }
    seedTurn(item, seq, Date.now(), reasons.filter(reason => feedbackTexts.has(reason.key)).map(reason => messageXml(reason.key, feedbackTexts.get(reason.key)!)), '', false)
  },
  finish(goalId: string, result: { decision: string; summary: string; outcome?: string }) {
    const item = must(goalId)
    const open = item.open
    if (!open) return
    item.open = null
    activate(item, Date.now(), [{ minutesAgo: (Date.now() - open.started_at_ms) / MINUTE, took: (Date.now() - open.started_at_ms) / 1000, reasons: open.reasons, tokens: 21_000, messages: open.reasons.filter(reason => feedbackTexts.has(reason.key)).map(reason => messageXml(reason.key, feedbackTexts.get(reason.key)!)), ...result }])
    item.state.usage.tokens += 21_000
  },
}
if (import.meta.env.DEV && typeof window !== 'undefined') Object.assign(window, { __mockGoals: mockGoals })

async function served(method: string, params: Record<string, unknown>) {
  mockGoals.calls.push({ method, params })
  await new Promise(resolve => setTimeout(resolve, 20))
  if (mockGoals.denied) throw Object.assign(Error('permission denied'), { code: 'NoPermission' })
}

const of = (agentDid: string) => agentDid === AGENT ? store() : new Map<string, MockGoal>()

/** Answers are parsed as the service's are. */
export const mockGoalSource: GoalSource = {
  async list(agentDid) {
    await served('goals.list', {})
    return parseGoals(structuredClone([...of(agentDid).values()].map(item => item.definition))) ?? []
  },
  async get(agentDid, goalId) {
    await served('goals.get', { goal_id: goalId })
    return parseGoal(structuredClone(of(agentDid).get(goalId)?.definition ?? null))
  },
  async state(agentDid, goalId) {
    await served('goals.state', { goal_id: goalId })
    return parseGoalState(structuredClone(of(agentDid).get(goalId)?.state ?? null))
  },
  async schedule(agentDid, goalId) {
    await served('goals.schedule', { goal_id: goalId })
    return parseGoalSchedule(structuredClone(of(agentDid).get(goalId)?.schedule ?? null))
  },
  async activations(agentDid, goalId, limit) {
    await served('goals.activations', { goal_id: goalId, query: { limit } })
    return parseActivations(structuredClone((of(agentDid).get(goalId)?.activations ?? []).slice(-limit))) ?? []
  },
  async open(agentDid, sid) {
    await served('session.read', { sid, worklog: 0, report: false })
    const item = [...of(agentDid).values()].find(entry => entry.definition.session.sid === sid)
    return parseOpenActivation({ entry: { session_id: sid, status: { one_line_status: item?.open?.line ?? '' } }, state: { goal: { epoch: item?.definition.session.epoch, ...(item?.open ? { activation: item.open } : {}) } } })
  },
  async works(agentDid, sid) {
    await served('session.work_tree', { sid })
    const item = [...of(agentDid).values()].find(entry => entry.definition.session.sid === sid)
    const sessions = structuredClone(item?.works ?? [])
    return { agent_did: agentDid, session_id: sid, complete: sessions.every(work => work.finished_at_ms !== null), sessions }
  },
  async opening(agentDid, sid, turn) {
    await served('session.worklog', { sid, turn, after: 0 })
    return (await mockWorklogSource({ agentDid, sessionId: sid, turn })({ after: 0 })).entries.slice(0, 8)
  },
  async post(agentDid, goalId, text) {
    await served('goals.post', { goal_id: goalId, text })
    const item = of(agentDid).get(goalId)
    if (!item) throw Error(`not_found: goal ${goalId}`)
    if (item.definition.status !== 'active' && item.definition.status !== 'paused') throw Error(`goal_conflict: goal ${goalId} is ${item.definition.status}`)
    const key = `msg:mock-post-${++posted}-${Date.now()}`
    feedbackTexts.set(key, text)
    const reason: ActivationReason = { kind: 'feedback', key, ref: key, note: 'feedback from self' }
    const held = item.state.holding && item.state.holding.reason !== 'min_interval'
    if (held || item.definition.status === 'paused') { item.waiting.push(reason); return key }
    setTimeout(() => {
      mockGoals.open(goalId, [reason])
      setTimeout(() => { if (item.open?.reasons.includes(reason)) mockGoals.finish(goalId, { decision: 'acted', summary: `Noted: ${text}` }) }, mockGoals.workMs)
    }, mockGoals.wakeMs)
    return key
  },
}
