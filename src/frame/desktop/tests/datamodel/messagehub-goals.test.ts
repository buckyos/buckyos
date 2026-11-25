import { activationResult, arrangeGoals, budgetHold, goalMessages, goalRowOf, messageInTurn, parseActivations, parseGoal, parseGoalSchedule, parseGoalState, parseOpenActivation, periodUsage, reasonDetail, unansweredQuestions, type Goal, type GoalTimeline } from '../../src/components/conversation/goals/model.ts'

function equal(actual: unknown, expected: unknown) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`Expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`)
}

const HOUR = 3_600_000
const DAY = 24 * HOUR
const definition = (extra: Record<string, unknown> = {}) => ({ goal_id: 'g1', revision: 2, title: 'Report', objective: 'Keep a report', status: 'active', session: { epoch: 1, sid: 's1' }, created_at_ms: 1, updated_at_ms: 10, ...extra })
const goalOf = (extra: Record<string, unknown> = {}) => parseGoal(definition(extra)) as Goal
const activation = (seq: number, extra: Record<string, unknown> = {}) => ({ epoch: 1, seq, session_id: 's1', started_at_ms: seq * 1000, ended_at_ms: seq * 1000 + 500, outcome: 'completed', decision: 'acted', summary: `look ${seq}`, cost: { runs: 1, total_tokens: 7 }, ...extra })
const timeline = (extra: Partial<GoalTimeline> = {}): GoalTimeline => ({ goal: goalOf(), state: null, schedule: null, activations: parseActivations([activation(1), activation(2)])!, hasOlder: false, open: null, posts: [], notices: [], ...extra })
const dids = { agent: 'did:a', self: 'did:me' }
const rows = (value: GoalTimeline) => goalMessages(value, dids).map(message => goalRowOf(message)?.type ?? message.kind)

Deno.test('a goal is read without the fields it leaves out', () => {
  equal(parseGoal(null), null)
  equal(parseGoal({ goal_id: 'g1' }), null)
  equal(goalOf().period, { window_secs: 86_400 })
  equal(goalOf({ budget: { period: { window_secs: 3600, max_total_tokens: 500 } } }).period, { window_secs: 3600, max_total_tokens: 500 })
  equal([goalOf().builtin, goalOf({ extensions: { opendan: { builtin: 'self_improve' } } }).builtin], [false, true])
  equal(goalOf({ title: ' ' }).title, 'g1')
})

Deno.test('active goals come first, dormant and switched off ones next, the ended apart', () => {
  const goals = [goalOf({ goal_id: 'ended', status: 'archived', status_changed_at_ms: 5 }), goalOf({ goal_id: 'dormant', status: 'paused', updated_at_ms: 99 }), goalOf({ goal_id: 'off', updated_at_ms: 98 }), goalOf({ goal_id: 'old', updated_at_ms: 1 }), goalOf({ goal_id: 'new', updated_at_ms: 2 }), goalOf({ goal_id: 'done', status: 'achieved', status_changed_at_ms: 6 })]
  const { live, ended } = arrangeGoals(goals, goal => goal.goal_id === 'off')
  equal(live.map(goal => goal.goal_id), ['new', 'old', 'dormant', 'off'])
  equal(ended.map(goal => goal.goal_id), ['done', 'ended'])
})

Deno.test('usage counts the goal and its work, and starts over with the period', () => {
  const now = 5 * DAY + 2 * HOUR
  const state = parseGoalState({ usage: { period_start_ms: 5 * DAY, tokens: 30, work_tokens: 12 } })
  equal(periodUsage(goalOf({ budget: { period: { max_total_tokens: 100 } } }), state, now), { used: 42, budget: 100 })
  equal(periodUsage(goalOf(), state, now + DAY), { used: 0 })
  equal(periodUsage(goalOf(), null, now), { used: 0 })
})

Deno.test('an unanswered question is one that is open or was never sent', () => {
  const state = parseGoalState({ questions: [{ text: 'a', status: 'open' }, { text: 'b' }, { text: 'c', status: 'answered', answer: 'yes' }, { text: 'd', status: 'expired' }, { text: ' ' }] })
  equal(unansweredQuestions(state), 2)
  equal(unansweredQuestions(null), 0)
})

Deno.test('only a budget that still holds is shown', () => {
  const held = (reason: string, until_ms?: number) => budgetHold(parseGoalState({ holding: { reason, since_ms: 1, until_ms } }), 100)
  equal(held('period_budget', 200), { reason: 'period_budget', until_ms: 200 })
  equal(held('agent_budget'), { reason: 'agent_budget' })
  equal(held('period_budget', 100), null)
  equal(held('min_interval', 200), null)
})

Deno.test('how an activation ended is its decision, or why it made none', () => {
  equal(activationResult({ outcome: 'completed', decision: 'no_action' }), 'no_action')
  equal(activationResult({ outcome: 'needs_input', decision: 'unknown' }), 'needs_info')
  equal(activationResult({ outcome: 'failed', decision: 'unknown' }), 'failed')
  equal(activationResult({ outcome: 'budget_exhausted', decision: 'acted' }), 'budget_exhausted')
})

Deno.test('a reason keeps what the goal said and drops the host\'s wording', () => {
  const detail = (kind: string, note?: string) => reasonDetail({ kind, key: 'k', note } as Parameters<typeof reasonDetail>[0])
  equal(detail('scheduled', 'scheduled check (every 1h): 2 new issues: one is old'), '2 new issues: one is old')
  equal(detail('scheduled', 'scheduled check (cron 0 9 * * * @ Europe/Berlin); 3 slots passed since the last one: late'), 'late')
  equal(detail('scheduled', 'scheduled check (every 1h): the check passed'), '')
  equal(detail('scheduled', 'scheduled check (every 1h)'), '')
  equal(detail('created', 'goal g1 was created; check: first run'), 'first run')
  equal(detail('restarted', 'epoch 2 started (restarted): it got lost'), 'it got lost')
  equal(detail('manual', 'activated by alice'), '')
  equal(detail('manual', 'try the new source'), 'try the new source')
  equal(detail('child', 'finished'), '')
})

Deno.test('what was said is found in the opening of a turn, never the header the runtime wrote', () => {
  const entries = [
    { seq: 1, t: 'turn_started' },
    { seq: 2, t: 'user_message', brief: 'header', content: '<goal_activation><msg key="k1">not this</msg></goal_activation>' },
    { seq: 3, t: 'user_message', content: '<msg key="k0" from="a">\nfirst\n</msg>\n<msg key="k1" from="feedback from alice" time="t">\nkeep it &lt;short&gt; &amp; sweet\nplease\n</msg>' },
  ]
  equal(messageInTurn(entries, 'k1'), 'keep it <short> & sweet\nplease')
  equal(messageInTurn(entries, 'k0'), 'first')
  equal(messageInTurn(entries, 'k2'), undefined)
})

Deno.test('the open activation is read from its session', () => {
  equal(parseOpenActivation({ entry: { session_id: 's1', status: { one_line_status: 'working (goal)' } }, state: { goal: { epoch: 2, activation: { seq: 4, started_at_ms: 9, reasons: [{ kind: 'manual', key: 'm' }, { kind: 'nonsense', key: 'x' }] } } } }), { epoch: 2, seq: 4, session_id: 's1', started_at_ms: 9, reasons: [{ kind: 'manual', key: 'm' }], line: 'working (goal)' })
  equal(parseOpenActivation({ entry: { session_id: 's1' }, state: { goal: { epoch: 2 } } }), null)
})

Deno.test('a message waits until an activation names it, and only in the session it was posted to', () => {
  const post = { key: 'p1', sid: 's1', text: 'hi', at_ms: 1800 }
  const pending = (value: GoalTimeline) => goalMessages(value, dids).map(goalRowOf).flatMap(row => row?.type === 'post' ? [row.pending] : [])
  equal(rows(timeline({ posts: [post] })), ['activation', 'post', 'activation'])
  equal(pending(timeline({ posts: [post] })), [true])
  equal(pending(timeline({ posts: [post], open: { epoch: 1, seq: 3, session_id: 's1', started_at_ms: 3000, reasons: [{ kind: 'feedback', key: 'p1' }], line: '' } })), [false])
  equal(pending(timeline({ posts: [{ ...post, sid: 'before' }] })), [false])
  equal(pending(timeline({ posts: [post], goal: goalOf({ status: 'archived' }) })), [false])
  // Older than what is loaded: it was read long ago.
  equal(rows(timeline({ posts: [{ ...post, at_ms: 10 }], hasOlder: true })), ['activation', 'activation'])
})

Deno.test('what was said elsewhere stands before the activation that read it', () => {
  const activations = parseActivations([activation(1), activation(2, { reasons: [{ kind: 'feedback', key: 'f', note: 'feedback from alice' }] })])!
  equal(rows(timeline({ activations })), ['activation', 'feedback', 'activation'])
})

Deno.test('a question is one row: its card, or the question itself when there is no card', () => {
  const state = parseGoalState({ questions: [{ text: 'sent', at_ms: 2500, epoch: 1, activation: 2, notify_id: 'n1' }, { text: 'unsent', at_ms: 2500, epoch: 1, activation: 2 }] })
  const notice = { from: 'did:a', to: ['did:me'], kind: 'notify', created_at_ms: 2400, content: { content: 'sent', machine: { intent: 'buckyos.notify', data: { notify_id: 'n1' } } }, ui_message_id: 'notify:n1' } as unknown as NonNullable<GoalTimeline['notices']>[number]
  equal(rows(timeline({ state, notices: [notice] })), ['activation', 'activation', 'notify', 'question'])
  equal(rows(timeline({ state, notices: null })), ['activation', 'activation', 'question', 'question'])
})

Deno.test('what a check sent is only told from the schedule when the notifications cannot be read', () => {
  const schedule = parseGoalSchedule({ last_check: { at_ms: 9000, outcome: 'false', notified: 'Stand up' }, check_errors: 2, last_check_error: 'boom' })
  equal([schedule?.check_errors, schedule?.last_check_error], [2, 'boom'])
  equal(rows(timeline({ schedule })), ['activation', 'activation'])
  equal(rows(timeline({ schedule, notices: null })), ['activation', 'activation', 'notified'])
})
