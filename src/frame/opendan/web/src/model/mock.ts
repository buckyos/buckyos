import { OpenDanError, type OpenDanDataModel } from './datamodel'
import type {
  AgentProfile,
  ArtifactVersion,
  GoalActivation,
  GoalChange,
  GoalDefinition,
  GoalSchedule,
  GoalState,
  HostedStatus,
  InstallPlan,
  KnownRuntime,
  KnownWorkspace,
  MemoryEntry,
  RuntimeImpact,
  RuntimeSettings,
  RegistryEntry,
  SessionDetail,
  SessionState,
  PlannedSkill,
  SkillAudit,
  SkillDraft,
  SkillFinding,
  SkillMeta,
  SkillSource,
  UiSkills,
  WorklogEntry,
} from './types'

const WHO = 'app:jarvis@devtest'
const AGENT = 'did:bns:jarvis.devtest'
const OWNER = 'did:bns:devtest'
const ROOT = '/opt/buckyos/data/home/devtest/agents/jarvis.test.buckyos.io'
const now = Date.now()
const ago = (seconds: number) => now - seconds * 1000

const UI = 'ui-7f3a2c91'
const WORK_RUNNING = 'work-b41d09e2'
const WORK_PENDING = 'work-5c8e17aa'
const WORK_FAILED = 'work-e02b66f4'
const CHECK = 'sc-daily'
const GOAL_BUILTIN = 'goal-memory-maintenance'
const GOAL_TIDY = 'goal-downloads-tidy'
const GOAL_ARCHIVED = 'goal-photo-backlog'
const SLOT_SECS = 1800
const slotNow = Math.floor(now / 1000 / SLOT_SECS)
const iso = (seconds: number) => new Date(now + seconds * 1000).toISOString()

const entry = (
  sid: string,
  kind: RegistryEntry['kind'],
  cls: string,
  objective: string,
  status: Partial<RegistryEntry['status']>,
  extra: Partial<RegistryEntry> = {},
): RegistryEntry => ({
  session_id: sid,
  kind,
  class: cls,
  created_by: { principal: WHO, via: 'app' },
  driver: { principal: WHO },
  location: `${ROOT}/sessions/${sid}`,
  agent_access: 'full',
  objective,
  location_rev: 1,
  ...extra,
  status: {
    rev: 12,
    run_state: 'waiting',
    acceptance: 'n/a',
    one_line_status: '',
    report_brief: '',
    activity: { summary: '', heartbeat_ms: ago(20) },
    updated_at_ms: ago(20),
    ...status,
  },
})

const state = (e: RegistryEntry, extra: Partial<SessionState>): SessionState => ({
  schema: 'opendan.session_state/5',
  rev: e.status.rev,
  run_state: e.status.run_state,
  outcome: e.status.outcome,
  acceptance: e.status.acceptance,
  turn_seq: 3,
  turns_completed: 3,
  live_run: null,
  last_run: 'run-0003',
  activity: e.status.activity,
  one_line_status: e.status.one_line_status,
  last_error: e.status.last_error,
  stop_requested: false,
  updated_at_ms: e.status.updated_at_ms,
  ...extra,
})

const hosted = (sid: string, cls: string, extra: Partial<HostedStatus>): HostedStatus => ({
  session_id: sid,
  class: cls,
  loaded: true,
  loaded_at_ms: ago(3600),
  drives: 14,
  last_result: { kind: 'idle', rev: 12, run_state: 'waiting' },
  last_result_at_ms: ago(20),
  idle_unload_secs: 600,
  reason: 'scan',
  ...extra,
})

const frozen = (behaviors: string[]) => ({
  catalog_rev: 'sha256:9a41c0de',
  frozen_at_ms: ago(7200),
  frozen_by: WHO,
  behaviors,
})

const config = (sid: string, behavior: string, behaviors: string[]) => ({
  session: { session_id: sid, agent_did: AGENT },
  runtime: { kind: 'native' },
  workspace: null,
  subscriptions: [],
  channels: { outbound: 'msg_center' },
  behavior,
  frozen: frozen(behaviors),
})

const buildFixtures = () => {
  const ui = entry(UI, 'ui', 'ui.direct', 'Talk with devtest', {
    needs_input: true,
    one_line_status: 'Waiting for the next message',
    updated_at_ms: ago(45),
  }, {
    route_key: `${AGENT}/dm:${OWNER}`,
    input_queue: `opendan/${UI}/input`,
    wake_event: `/opendan/session/${UI}/input`,
  })
  const running = entry(WORK_RUNNING, 'work', 'work.default', 'Summarize last week\'s download folder into a report', {
    rev: 31,
    run_state: 'running',
    turn_open: true,
    one_line_status: 'Reading 14 files under ~/Downloads',
    activity: {
      summary: 'Collecting file metadata',
      touching: [
        { kind: 'path', ref: '/home/devtest/Downloads', mode: 'read', since_ms: ago(300) },
        { kind: 'artifact', ref: 'art-weekly-report', mode: 'write', since_ms: ago(120) },
      ],
      heartbeat_ms: ago(4),
    },
    updated_at_ms: ago(4),
  }, {
    origin: { parent_session: UI, reason_messages: ['msg:1a2b'], created_by_call: 'call_91' },
    input_queue: `opendan/${WORK_RUNNING}/input`,
    artifact_id: 'art-weekly-report',
    workspace: { workspace_id: 'ws-downloads', access: 'read_write' },
    runtime_id: 'host',
  })
  const pending = entry(WORK_PENDING, 'work', 'work.default', 'Rename photos by capture date', {
    rev: 48,
    run_state: 'finished',
    outcome: 'succeeded',
    acceptance: 'pending',
    one_line_status: 'Renamed 212 photos, 3 skipped',
    report_brief: '212 renamed, 3 skipped (no EXIF date).',
    pending_decision: { artifact: 'art-photo-rename', ver: 'v2' },
    updated_at_ms: ago(900),
  }, {
    origin: { parent_session: UI },
    input_queue: `opendan/${WORK_PENDING}/input`,
    artifact_id: 'art-photo-rename',
    workspace: { workspace_id: 'ws-photos', access: 'read_write' },
  })
  const failed = entry(WORK_FAILED, 'work', 'work.default', 'Fetch the release notes of cyfs-gateway', {
    rev: 9,
    run_state: 'finished',
    outcome: 'failed',
    one_line_status: 'LLM request failed',
    last_error: { kind: 'llm', message: 'provider returned 529 (overloaded) after 3 attempts', at_ms: ago(5400) },
    updated_at_ms: ago(5400),
  }, {
    origin: { parent_session: WORK_RUNNING },
    driver: { principal: 'app:xagent@devtest' },
  })
  const check = entry(CHECK, 'self_check', 'self_check', 'Daily self check', {
    rev: 6,
    run_state: 'finished',
    outcome: 'succeeded',
    one_line_status: 'No issue found',
    updated_at_ms: ago(40000),
  })
  const entries = [ui, running, pending, failed, check]

  const hostedList: HostedStatus[] = [
    hosted(UI, 'ui.direct', { reason: 'ui' }),
    hosted(WORK_RUNNING, 'work.default', {
      drives: 3,
      last_result: { kind: 'turn_open', rev: 30, turn: 2, needs_input: false },
      last_result_at_ms: ago(60),
      reason: 'input',
    }),
    hosted(WORK_PENDING, 'work.default', {
      loaded: false,
      drives: 9,
      last_result: { kind: 'finished', rev: 48, outcome: 'succeeded', acceptance: 'pending' },
      last_result_at_ms: ago(900),
      reason: 'scan',
    }),
  ]

  const worklogs: Record<string, WorklogEntry[]> = {
    [UI]: [
      { seq: 1, t: 'created', session_id: UI, kind: 'ui', by: WHO, objective: ui.objective, at_ms: ago(86400) },
      { seq: 41, t: 'turn_started', run_id: 'run-0003', turn: 3, input_seq: 7, inputs: [{ src: 'queue', index: 7, kind: 'message' }], events: [], at_ms: ago(400) },
      { seq: 42, t: 'user_message', run_id: 'run-0003', turn: 3, content: 'Please summarize what I downloaded last week.' },
      { seq: 43, t: 'step', run_id: 'run-0003', turn: 3, step_index: 0, behavior: 'ui_chat', assistant: 'I will start a work session for this.', actions: [{ call_id: 'call_91', tool: 'session.create', args: { objective: running.objective }, effect: 'write' }] },
      { seq: 44, t: 'action_result', run_id: 'run-0003', turn: 3, call_id: 'call_91', status: 'ok', result: `{"session_id":"${WORK_RUNNING}"}` },
      { seq: 45, t: 'assistant_message', run_id: 'run-0003', turn: 3, assistant: 'On it. I started a task and will report back when the summary is ready.' },
      { seq: 46, t: 'turn_ended', run_id: 'run-0003', turn: 3, status: 'completed', at_ms: ago(380) },
    ],
    [WORK_RUNNING]: [
      { seq: 1, t: 'created', session_id: WORK_RUNNING, kind: 'work', by: WHO, objective: running.objective, at_ms: ago(380) },
      { seq: 2, t: 'turn_started', run_id: 'run-0001', turn: 1, input_seq: 1, inputs: [{ src: 'queue', index: 1, kind: 'message' }], events: [], at_ms: ago(370) },
      { seq: 3, t: 'step', run_id: 'run-0001', turn: 1, step_index: 0, behavior: 'work_plan', assistant: 'Listing the folder first.', actions: [{ call_id: 'call_1', tool: 'shell', args: { command: 'ls -la ~/Downloads' }, effect: 'read' }] },
      { seq: 4, t: 'action_result', run_id: 'run-0001', turn: 1, call_id: 'call_1', status: 'ok', result: 'total 14\n-rw-r--r-- 1 devtest devtest 48213 report-draft.pdf\n…' },
      { seq: 5, t: 'outcome', run_id: 'run-0001', turn: 1, kind: 'call_behavior', next_behavior: 'work_do' },
      { seq: 6, t: 'input_rejected', input: { src: 'queue', index: 2, kind: 'message' }, reason: 'supplement_only', detail: 'session accepts supplements only' },
      { seq: 7, t: 'control_applied', input: { src: 'queue', index: 3, kind: 'control' }, command: 'interrupt' },
    ],
    [WORK_PENDING]: [
      { seq: 60, t: 'outcome', run_id: 'run-0004', turn: 4, kind: 'finish', report: '212 renamed, 3 skipped.' },
      { seq: 61, t: 'turn_ended', run_id: 'run-0004', turn: 4, status: 'completed', at_ms: ago(900) },
    ],
    [WORK_FAILED]: [
      { seq: 5, t: 'turn_ended', run_id: 'run-0001', turn: 1, status: 'failed', at_ms: ago(5400) },
    ],
    [CHECK]: [
      { seq: 9, t: 'compaction', summary_start_seq: 3, made_by: 'runner' },
      { seq: 10, t: 'turn_ended', run_id: 'run-0001', turn: 1, status: 'completed', at_ms: ago(40000) },
    ],
  }

  const details: Record<string, Omit<SessionDetail, 'entry' | 'worklog' | 'children' | 'hosted'>> = {
    [UI]: {
      state: state(ui, {
        current_behavior: 'ui_chat',
        reply: { tunnel: 'msg_center', to: 'did:bns:devtest' },
        outbox: [
          { key: 'reply:t2', msg: { kind: 'chat', content: 'Done, 3 files moved.' }, turn: 2, status: 'sent', msg_id: 'msg:9f01', deliveries: ['did:bns:devtest'], attempts: 1, updated_at_ms: ago(4000) },
          { key: 'reply:t3', msg: { kind: 'chat', content: 'On it. I started a task…' }, turn: 3, status: 'failed', error: 'msg-center: post_send timeout after 30s', attempts: 3, updated_at_ms: ago(360) },
          { key: 'notice:t3', msg: { kind: 'chat', content: 'The previous reply could not be delivered.' }, turn: 3, status: 'pending', attempts: 0, updated_at_ms: ago(350) },
        ],
      }),
      report: '',
      config: config(UI, 'ui_chat', ['ui_chat']),
      binding: { schema: 'opendan.binding/6', target: { kind: 'native' }, runtime_id: 'private', connector: 'native', workdir: `${ROOT}/sessions/${UI}/work`, bound_at_ms: ago(86000), bound_by: WHO },
      statistics: { input_tokens: 48210, output_tokens: 3922, total_tokens: 52132, rounds: 11, rounds_failed: 0, rounds_interrupted: 0, turns: 3 },
      runs: ['run-0001', 'run-0002', 'run-0003'],
      lease_holder: null,
    },
    [WORK_RUNNING]: {
      state: state(running, {
        turn_seq: 2,
        turns_completed: 1,
        open_turn: { index: 2, run_id: 'run-0002', input_seq: 3, inputs: ['queue#3'], at_ms: ago(120) },
        current_behavior: 'work_do',
        process_entry: 'work_do',
        process_stack: [
          { entry: 'work_plan', role: 'caller', call: { mode: 'create_sub_context', behavior: 'work_do', trigger: { kind: 'behavior' }, task: 'collect metadata' }, run_id: 'run-0001', turns: [] },
        ],
        live_run: { run_id: 'run-0002' },
        pending_events: [
          {
            subscription_id: 'sub-task',
            source: { kind: 'task', id: 'task-55' },
            latest: { seq: 4, key: 'task-55#4', event: 'progress', summary: 'scanned 9/14 files', data_ref: null, received_at_ms: ago(15) },
            terminal: null,
            superseded: 3,
          },
        ],
        watched_tasks: ['task-55'],
      }),
      report: '# Weekly downloads\n\n- 14 files, 3 archives\n- (in progress)\n',
      config: config(WORK_RUNNING, 'work_plan', ['work_plan', 'work_do', 'work_report']),
      binding: { schema: 'opendan.binding/6', target: { node_did: 'did:web:pc.test.buckyos.io', profile_id: 'agent-3f9c1a07b2e4', workdir: '/home/devtest/Downloads' }, runtime_id: 'rt-node-07', connector: 'node_executor', workdir: '/home/devtest/Downloads', bound_at_ms: ago(370), bound_by: WHO },
      statistics: { input_tokens: 9120, output_tokens: 801, total_tokens: 9921, rounds: 4, rounds_failed: 0, rounds_interrupted: 1, turns: 1 },
      runs: ['run-0001', 'run-0002'],
      lease_holder: { resource: `session:${WORK_RUNNING}`, epoch: 5, holder: { runner_id: 'rn-51c2', principal: WHO, host: 'ood1', pid: 4411 }, acquired_at_ms: ago(120), released_at_ms: null },
    },
    [WORK_PENDING]: {
      state: state(pending, { turn_seq: 4, turns_completed: 4, pending_decision: pending.status.pending_decision, result: { renamed: 212, skipped: 3 } }),
      report: '# Photo rename\n\n212 photos renamed to `YYYY-MM-DD_HHMMSS.jpg`.\n3 skipped: no EXIF capture date.\n',
      config: config(WORK_PENDING, 'work_plan', ['work_plan', 'work_do', 'work_report']),
      binding: null,
      statistics: null,
      runs: ['run-0001', 'run-0002', 'run-0003', 'run-0004'],
      lease_holder: null,
    },
    [WORK_FAILED]: {
      state: state(failed, { turn_seq: 1, turns_completed: 1 }),
      report: '',
      config: config(WORK_FAILED, 'work_plan', ['work_plan']),
      binding: null,
      statistics: null,
      runs: ['run-0001'],
      lease_holder: null,
    },
    [CHECK]: { note: 'session directory is not readable: permission denied' },
  }

  const artifactVersions: Record<string, ArtifactVersion[]> = {
    'art-photo-rename': [
      { ver: 'v1', session: WORK_PENDING, base: null, state: 'accepted', outputs: ['rename-plan.json'], updated_at_ms: ago(5000) },
      { ver: 'v2', session: WORK_PENDING, base: 'v1', state: 'produced', outputs: ['rename-plan.json', 'report.md'], side_effects: [{ call_id: 'call_40', tool: 'shell', note: 'mv 212 files' }], updated_at_ms: ago(900) },
    ],
    'art-weekly-report': [
      { ver: 'v1', session: WORK_RUNNING, base: null, state: 'produced', outputs: ['weekly.md'], updated_at_ms: ago(100) },
    ],
  }

  return { entries, hostedList, worklogs, details, artifactVersions }
}

interface GoalFixture {
  def: GoalDefinition
  state: GoalState
  schedule: GoalSchedule
  activations: GoalActivation[]
  changes: GoalChange[]
}

const goalDef = (goal_id: string, title: string, objective: string, extra: Partial<GoalDefinition>): GoalDefinition => ({
  schema: 'opendan.goal/1',
  goal_id,
  revision: 1,
  change_seq: 1,
  title,
  objective,
  lifecycle: 'maintain',
  owner: OWNER,
  created_by: { principal: OWNER, via: 'owner' },
  status: 'active',
  status_changed_at_ms: ago(86400 * 3),
  status_by: OWNER,
  session: { epoch: 1, sid: `${goal_id}-e1` },
  epochs: [{ epoch: 1, sid: `${goal_id}-e1`, started_at_ms: ago(86400 * 3), start: 'created', keep: 'all' }],
  created_at_ms: ago(86400 * 3),
  updated_at_ms: ago(86400 * 3),
  ...extra,
})

const activation = (goal_id: string, epoch: number, seq: number, started: number, extra: Partial<GoalActivation>): GoalActivation => ({
  schema: 'opendan.goal_activation/1',
  goal_id,
  epoch,
  seq,
  session_id: `${goal_id}-e${epoch}`,
  goal_revision: 1,
  started_at_ms: started,
  ended_at_ms: started + 42_000,
  reasons: [{ kind: 'scheduled', key: `sched:${goal_id}:${epoch}:${seq}` }],
  outcome: 'completed',
  decision: 'no_action',
  summary: '',
  cost: { runs: 1, total_tokens: 2400 },
  ...extra,
})

const change = (seq: number, at_ms: number, kind: string, who: string, via: string, extra: Partial<GoalChange> = {}): GoalChange => ({
  seq, at_ms, kind, who, via, ...extra,
})

const buildGoalFixtures = (): GoalFixture[] => {
  const builtin: GoalFixture = {
    def: goalDef(GOAL_BUILTIN, 'Memory maintenance', 'Consolidate what the agent perceived into cognition, review what is due and route what was discovered.', {
      healthy_state: 'Perceptions that reached their processing condition are disposed reliably; cognition is traceable and reviewed on time; discovered items have a destination.',
      constraints: ['Never record perceptions of its own', 'Small batches: about 20 materials, at most 3 per activation'],
      created_by: { principal: AGENT, via: 'opendan' },
      budget: { activation: { max_total_tokens: 32000, max_runs: 4 }, period: { window_secs: 86400, max_total_tokens: 200000, max_activations: 48 } },
      schedule: { every_secs: SLOT_SECS },
      behavior: 'memory_maintenance',
      status_by: AGENT,
      idempotency_key: 'builtin:memory_maintenance',
      updated_at_ms: ago(5400),
      extensions: { opendan: { builtin: 'memory_maintenance', policy_version: 1, capabilities: ['memory_consolidation'], records_perception: false } },
    }),
    state: {
      schema: 'opendan.goal_state/1',
      goal_id: GOAL_BUILTIN,
      rev: 23,
      epoch: 1,
      goal_revision_seen: 1,
      last_activation: {
        epoch: 1, seq: 7, at_ms: ago(5400),
        reasons: [{ kind: 'scheduled', key: `sched:${GOAL_BUILTIN}:1:7` }],
        outcome: 'completed', decision: 'acted',
        summary: 'Consolidated 2 batches (14 materials); 1 candidate needs confirmation.',
      },
      distance: { text: '3 materials wait for their processing condition; nothing is due for review.', observed_at_ms: ago(5400), unknowns: [] },
      questions: [{
        text: 'You mentioned a weekly release report three times. Should I keep a schedule for it (Fridays 16:00)?',
        at_ms: ago(5400), epoch: 1, activation: 7, notify_id: 'ntf-7c2e', status: 'open', candidate_id: 'cand-weekly-report',
      }],
      usage: { period_start_ms: ago(30000), tokens: 41200, activations: 5, works: 0, work_tokens: 0 },
      maintenance: {
        receipts: [
          { operation_id: 'op-01HX1', epoch: 1, activation: 7, batch_id: 'batch-0a3f', graph_seq: 1180, refs: ['mat-1', 'mat-2'], batch: { materials: 8 }, stage: 'completed', operations: 9, dispositions: 8, cleanup_pending: false, created_at_ms: ago(5500), updated_at_ms: ago(5450) },
          { operation_id: 'op-01HX2', epoch: 1, activation: 7, batch_id: 'batch-0a40', graph_seq: 1189, refs: ['mat-3'], batch: { materials: 6 }, stage: 'completed', operations: 4, dispositions: 6, cleanup_pending: false, created_at_ms: ago(5440), updated_at_ms: ago(5400) },
        ],
        candidates: [{
          candidate_id: 'cand-weekly-report', revision: 1, kind: 'schedule', semantic_key: 'schedule:weekly-release-report',
          title: 'Weekly release report, Fridays 16:00', action: 'create_schedule', scope: [OWNER], intent_basis: 'inferred',
          status: 'needs_confirmation', question_id: 'q-1', notify_id: 'ntf-7c2e', created_at_ms: ago(5400), updated_at_ms: ago(5400),
        }],
        last_successful_consolidation_at_ms: ago(5400),
        consolidations: 9,
        operations: 61,
        dispositions: 118,
        abandoned_batches: 1,
      },
      updated_at_ms: ago(5400),
    },
    schedule: {
      schema: 'opendan.goal_schedule/1',
      goal_id: GOAL_BUILTIN,
      backend: 'local',
      every_secs: SLOT_SECS,
      last_slot: slotNow,
      last_fire_at_ms: ago(5400),
      start_posted_epoch: 1,
      skipped: [
        { slot: slotNow - 1, at_ms: ago(1800), reason: 'no_change' },
        { slot: slotNow, at_ms: ago(60), reason: 'no_change' },
      ],
      gate: {
        status: 'no_change',
        last_checked_slot: slotNow,
        last_baseline_slot: slotNow - 3,
        last_checked_at_ms: ago(60),
        last_full_check_at_ms: ago(7200),
        last_skip_reason: 'no_change',
        last_inputs_digest: 'sha256:1f9c2b7e',
        pending: { new: 0, due: 0, waiting: 3, retry_cleanup: 0, next_deadline: iso(4 * 3600), oldest_at_ms: ago(86400) },
        due_reviews: 0,
        next_review_at: iso(2 * 86400),
        unfinished_receipts: 0,
        last_run_at_ms: ago(5400),
        last_run_slot: slotNow - 3,
        backoff_level: 0,
      },
      updated_at_ms: ago(60),
    },
    activations: [5, 6, 7].map((seq) =>
      activation(GOAL_BUILTIN, 1, seq, ago(5400 + (7 - seq) * 7200), seq === 7
        ? { decision: 'acted', summary: 'Consolidated 2 batches (14 materials); 1 candidate needs confirmation.', cost: { runs: 2, total_tokens: 18700 } }
        : { summary: 'Checked: nothing reached its processing condition.' }),
    ),
    changes: [change(1, ago(86400 * 3), 'created', AGENT, 'opendan', { revision: 1, epoch: 1, reason: 'seeded builtin:memory_maintenance' })],
  }

  const tidy: GoalFixture = {
    def: goalDef(GOAL_TIDY, 'Keep Downloads tidy', 'Keep ~/Downloads free of stale installers and archives; move what is worth keeping into the Downloads workspace.', {
      revision: 2,
      change_seq: 4,
      healthy_state: 'No file older than 30 days sits in ~/Downloads unsorted.',
      constraints: ['Never delete without asking', 'Keep PDFs'],
      scope: { paths: ['/home/devtest/Downloads'], workspace: 'ws-downloads' },
      budget: { activation: { max_total_tokens: 60000, max_runs: 3 }, period: { window_secs: 86400, max_total_tokens: 300000 } },
      schedule: { every_secs: 6 * 3600 },
      created_by: { principal: OWNER, via: `ui_session:${UI}` },
      session: { epoch: 2, sid: `${GOAL_TIDY}-e2` },
      epochs: [
        { epoch: 1, sid: `${GOAL_TIDY}-e1`, started_at_ms: ago(86400 * 3), start: 'created', keep: 'all', ended_at_ms: ago(86400), end: 'restarted', end_reason: 'scope narrowed to the Downloads workspace' },
        { epoch: 2, sid: `${GOAL_TIDY}-e2`, started_at_ms: ago(86400), start: 'restarted', keep: 'facts', reason: 'scope narrowed to the Downloads workspace' },
      ],
      updated_at_ms: ago(86400),
    }),
    state: {
      schema: 'opendan.goal_state/1',
      goal_id: GOAL_TIDY,
      rev: 9,
      epoch: 2,
      goal_revision_seen: 2,
      last_activation: {
        epoch: 2, seq: 1, at_ms: ago(43200),
        reasons: [{ kind: 'restarted', key: `restart:${GOAL_TIDY}:2` }],
        outcome: 'needs_input', decision: 'needs_info',
        summary: 'Three installers older than 30 days: delete or archive?',
      },
      distance: { text: '3 stale installers remain in ~/Downloads.', observed_at_ms: ago(43200), unknowns: ['whether the .dmg files are still needed'] },
      pending: ['Decide what happens to installers older than 30 days'],
      questions: [{
        text: 'Should installers older than 30 days be deleted, or moved to an archive folder?',
        at_ms: ago(43200), epoch: 2, activation: 1, notify_id: 'ntf-4a1b', status: 'open',
      }],
      recent_works: [{ sid: 'work-tidy-01', outcome: 'succeeded', one_line_status: 'Moved 12 archives into the workspace', absorbed_in: [1, 2] }],
      usage: { period_start_ms: ago(50000), tokens: 12800, activations: 1, works: 0, work_tokens: 0 },
      updated_at_ms: ago(43200),
    },
    schedule: {
      schema: 'opendan.goal_schedule/1',
      goal_id: GOAL_TIDY,
      backend: 'local',
      every_secs: 6 * 3600,
      last_slot: Math.floor(now / 1000 / (6 * 3600)),
      last_fire_at_ms: ago(43200),
      start_posted_epoch: 2,
      skipped: [{ slot: Math.floor(now / 1000 / (6 * 3600)), at_ms: ago(600), reason: 'min_interval' }],
      updated_at_ms: ago(600),
    },
    activations: [
      activation(GOAL_TIDY, 1, 1, ago(86400 * 3 - 60), {
        reasons: [{ kind: 'created', key: `create:${GOAL_TIDY}` }],
        decision: 'created_work', summary: 'Started a Work to sort the archives.', works_created: ['work-tidy-01'], cost: { runs: 1, total_tokens: 5100 },
      }),
      activation(GOAL_TIDY, 1, 2, ago(86400 * 2), {
        reasons: [{ kind: 'child', key: 'child:work-tidy-01', ref: 'work-tidy-01', note: 'Work finished: succeeded' }],
        decision: 'acted', summary: 'Absorbed the Work result; 12 archives moved.', works_absorbed: ['work-tidy-01'], cost: { runs: 1, total_tokens: 3900 },
      }),
      activation(GOAL_TIDY, 2, 1, ago(43200), {
        reasons: [{ kind: 'restarted', key: `restart:${GOAL_TIDY}:2` }],
        outcome: 'needs_input', decision: 'needs_info', summary: 'Three installers older than 30 days: delete or archive?', cost: { runs: 1, total_tokens: 3800 },
      }),
    ],
    changes: [
      change(1, ago(86400 * 3), 'created', OWNER, `ui_session:${UI}`, { revision: 1, epoch: 1 }),
      change(2, ago(86400 * 2), 'revised', OWNER, 'owner', { revision: 2, reason: 'keep PDFs' }),
      change(3, ago(86400), 'epoch_ended', OWNER, 'owner', { epoch: 1, reason: 'scope narrowed to the Downloads workspace' }),
      change(4, ago(86400), 'restarted', OWNER, 'owner', { epoch: 2, reason: 'scope narrowed to the Downloads workspace' }),
    ],
  }

  const archived: GoalFixture = {
    def: goalDef(GOAL_ARCHIVED, 'Clear the photo rename backlog', 'Rename every photo under Pictures/camera by capture date.', {
      lifecycle: 'achieve',
      achieved_when: 'No photo under Pictures/camera keeps its camera file name.',
      change_seq: 2,
      status: 'archived',
      status_changed_at_ms: ago(86400 * 2),
      status_reason: 'Superseded by the Photos workspace',
      epochs: [{ epoch: 1, sid: `${GOAL_ARCHIVED}-e1`, started_at_ms: ago(86400 * 6), start: 'created', keep: 'all', ended_at_ms: ago(86400 * 2), end: 'archived', end_reason: 'Superseded by the Photos workspace' }],
      created_at_ms: ago(86400 * 6),
      updated_at_ms: ago(86400 * 2),
    }),
    state: {
      schema: 'opendan.goal_state/1',
      goal_id: GOAL_ARCHIVED,
      rev: 4,
      epoch: 1,
      goal_revision_seen: 1,
      last_activation: { epoch: 1, seq: 1, at_ms: ago(86400 * 6), reasons: [{ kind: 'created', key: `create:${GOAL_ARCHIVED}` }], outcome: 'completed', decision: 'created_work', summary: 'Started the rename Work.' },
      distance: { text: '212 photos renamed, 3 skipped.', observed_at_ms: ago(86400 * 2) },
      recent_works: [{ sid: WORK_PENDING, outcome: 'succeeded', one_line_status: 'Renamed 212 photos, 3 skipped', absorbed_in: [1, 1] }],
      usage: { period_start_ms: ago(86400 * 2), tokens: 2600, activations: 1, works: 1, work_tokens: 9800 },
      last_error: { kind: 'llm', message: 'provider returned 529 (overloaded) after 3 attempts', at_ms: ago(86400 * 4) },
      updated_at_ms: ago(86400 * 2),
    },
    schedule: { schema: 'opendan.goal_schedule/1', goal_id: GOAL_ARCHIVED, backend: 'local', start_posted_epoch: 1, updated_at_ms: ago(86400 * 6) },
    activations: [
      activation(GOAL_ARCHIVED, 1, 1, ago(86400 * 6), { reasons: [{ kind: 'created', key: `create:${GOAL_ARCHIVED}` }], decision: 'created_work', summary: 'Started the rename Work.', works_created: [WORK_PENDING], cost: { runs: 1, total_tokens: 2600 } }),
    ],
    changes: [
      change(1, ago(86400 * 6), 'created', OWNER, 'owner', { revision: 1, epoch: 1 }),
      change(2, ago(86400 * 2), 'status', OWNER, 'owner', { reason: 'Superseded by the Photos workspace', detail: { status: 'archived' } }),
    ],
  }

  return [builtin, tidy, archived]
}

const delay = () => new Promise((resolve) => window.setTimeout(resolve, 60))

/**
 * `runtime=native`: Desktop Lite. `scene=restart`: sessions the host's restart interrupted. `memory=busy`: the
 * consolidation holds Memory; `memory=empty`: nothing remembered, no consolidation goal.
 */
export const createMockDataModel = (params = new URLSearchParams()): OpenDanDataModel => {
  const nativeRuntime = params.get('runtime') === 'native'
  const memoryScene = params.get('memory')
  const fx = buildFixtures()
  if (params.get('scene') === 'restart') {
    const queue = (sid: string) => ({ input_queue: `opendan/${sid}/input` })
    fx.entries[1].status.paused = 'interrupted'
    fx.entries.push(
      entry('work-plan-77', 'work', 'plan_simple', 'Plan the photo archive migration', { run_state: 'waiting', paused: 'interrupted', one_line_status: 'Waiting for 1 todo', updated_at_ms: ago(200) },
        { origin: { parent_session: UI }, ...queue('work-plan-77') }),
      entry('work-todo-a1', 'work', 'work.default', 'Copy the 2019 albums', { run_state: 'running', turn_open: true, paused: 'interrupted', updated_at_ms: ago(210) },
        { origin: { parent_session: 'work-plan-77' }, ...queue('work-todo-a1') }),
      entry('sc-weekly', 'self_check', 'self_check', 'Weekly self check', { run_state: 'running', turn_open: true, paused: 'interrupted', updated_at_ms: ago(220) }, queue('sc-weekly')),
      entry('work-user-paused', 'work', 'work.default', 'Index the music library', { run_state: 'waiting', paused: 'user', updated_at_ms: ago(230) },
        { origin: { parent_session: UI }, ...queue('work-user-paused') }),
      entry('work-goal-child', 'work', 'work.default', 'Tidy the downloads folder', { run_state: 'waiting', paused: 'goal', updated_at_ms: ago(240) },
        { origin: { goal: { goal_id: GOAL_TIDY } }, ...queue('work-goal-child') }),
      entry('ui-interrupted', 'ui', 'ui.direct', 'Talk with alice', { run_state: 'running', turn_open: true, paused: 'interrupted', updated_at_ms: ago(250) },
        { route_key: `${AGENT}/dm:did:bns:alice.buckyos`, ...queue('ui-interrupted') }),
    )
  }
  const goals = buildGoalFixtures()
  const goalEntries: RegistryEntry[] = goals.flatMap((g) =>
    (g.def.epochs ?? []).map((ep) =>
      entry(ep.sid, 'goal', 'goal', g.def.title, {
        run_state: ep.end ? 'finished' : 'waiting',
        outcome: ep.end ? 'succeeded' : undefined,
        one_line_status: ep.end ? `Epoch ${ep.epoch} ended: ${ep.end}` : `Epoch ${ep.epoch}: waiting for the next activation`,
        updated_at_ms: g.def.updated_at_ms,
      }, { created_by: { principal: g.def.created_by.principal, via: g.def.created_by.via }, input_queue: `opendan/${ep.sid}/input` }),
    ),
  )
  const findGoal = (goalId: string): GoalFixture => {
    const found = goals.find((g) => g.def.goal_id === goalId)
    if (!found) throw new OpenDanError('not_found', `goal ${goalId} is not registered`)
    return found
  }
  const goalChange = (g: GoalFixture, kind: string, extra: Partial<GoalChange>) => {
    g.def.change_seq = (g.def.change_seq ?? 0) + 1
    g.changes.push(change(g.def.change_seq, Date.now(), kind, OWNER, 'owner', extra))
    g.def.updated_at_ms = Date.now()
  }
  const openEpoch = (g: GoalFixture, start: 'reopened' | 'restarted', reason: string, keep: 'all' | 'facts') => {
    const previous = g.def.epochs?.find((ep) => ep.epoch === g.def.session.epoch)
    if (previous && !previous.end) Object.assign(previous, { ended_at_ms: Date.now(), end: 'restarted', end_reason: reason })
    const epoch = g.def.session.epoch + 1
    const sid = `${g.def.goal_id}-e${epoch}`
    g.def.session = { epoch, sid }
    ;(g.def.epochs ??= []).push({ epoch, sid, started_at_ms: Date.now(), start, keep, reason })
    g.def.status = 'active'
    g.def.status_changed_at_ms = Date.now()
    g.def.status_reason = reason
    g.state.epoch = epoch
    g.state.rev += 1
    g.state.updated_at_ms = Date.now()
    if (keep === 'facts') Object.assign(g.state, { distance: undefined, pending: undefined, questions: undefined, last_activation: undefined })
    if (g.schedule.gate) g.schedule.gate.status = 'unknown'
    goalEntries.push(entry(sid, 'goal', 'goal', g.def.title, { run_state: 'waiting', one_line_status: `Epoch ${epoch}: waiting for the next activation` }, { input_queue: `opendan/${sid}/input` }))
    goalChange(g, start, { epoch, reason })
    return structuredClone(g.def)
  }
  const workspaces: KnownWorkspace[] = [
    {
      version: 1, runtime_host: 'mock-host',
      workspace_id: 'ws-downloads', name: 'Downloads', description: 'Weekly reports from shared downloads.', url: null,
      location: { runtime_id: 'local', directory: '/home/devtest/Downloads' },
      usage: 'collaborative', lifecycle: 'active', availability: 'available',
      revision: 2, location_revision: 1, updated_at_ms: ago(4), checked_at_ms: ago(30),
      source_session: WORK_RUNNING, conflict: null, last_error: null,
    },
    {
      version: 1, runtime_host: 'mock-host',
      workspace_id: 'ws-photos', name: 'Photos', description: 'Organize photographs and keep rename reports.', url: 'https://github.com/example/photos',
      location: { runtime_id: 'local', directory: `${ROOT}/workspace/photos` },
      usage: 'private', lifecycle: 'active', availability: 'available',
      revision: 1, location_revision: 1, updated_at_ms: ago(900), checked_at_ms: ago(900),
      source_session: WORK_PENDING, conflict: null, last_error: null,
    },
    {
      version: 1, runtime_host: 'studio-host',
      workspace_id: 'ws-design-archive', name: 'Design archive', description: 'Long-term project on a disconnected workstation.', url: null,
      location: { runtime_id: 'studio-workstation', directory: '/projects/design/archive' },
      usage: 'collaborative', lifecycle: 'archived', availability: 'runtime_unavailable',
      revision: 4, location_revision: 2, updated_at_ms: ago(7200), checked_at_ms: ago(120),
      source_session: null, conflict: null, last_error: 'Runtime studio-workstation is unavailable; the workspace registration is retained.',
    },
    {
      version: 1, runtime_host: 'mock-host',
      workspace_id: 'ws-project-notes', name: 'Project notes', description: 'Archived notes whose directory was moved.', url: null,
      location: { runtime_id: 'local', directory: '/home/devtest/Projects/notes-old' },
      usage: 'collaborative', lifecycle: 'archived', availability: 'missing',
      revision: 2, location_revision: 1, updated_at_ms: ago(8000), checked_at_ms: ago(120),
      source_session: null, conflict: null, last_error: 'Directory does not exist; locate the moved workspace to restore access.',
    },
    {
      version: 1, runtime_host: 'mock-host',
      workspace_id: 'ws-site', name: 'Family site', description: 'Static site of the family album.', url: 'https://github.com/example/family-site',
      location: { runtime_id: 'host', directory: '/Users/devtest/Sites/family' },
      usage: 'collaborative', lifecycle: 'active', availability: 'available',
      revision: 1, location_revision: 1, updated_at_ms: ago(20000), checked_at_ms: ago(20000),
      source_session: null, conflict: null, last_error: null,
    },
    {
      version: 1, runtime_host: 'mock-host',
      workspace_id: 'ws-scratch', name: 'Scratch', description: '', url: null,
      location: { runtime_id: 'local', directory: `${ROOT}/workspace/scratch` },
      usage: 'private', lifecycle: 'active', availability: 'available',
      revision: 1, location_revision: 1, updated_at_ms: ago(90000), checked_at_ms: ago(90000),
      source_session: null, conflict: null, last_error: null,
    },
  ]
  const allOps = ['open_stream', 'describe', 'exec', 'read', 'write', 'edit', 'mkdir', 'stat', 'rename', 'remove', 'computer_inspect', 'computer_observe', 'computer_act', 'query', 'cancel']
  const runtimes: KnownRuntime[] = [
    {
      version: 5, runtime_id: 'private', name: 'Private runtime',
      desc: "Your own environment: the agent's container. Commands run directly; paths here are not the owner's host paths.",
      runtime_host: 'mock-host', connector: { type: 'native' }, enabled: true,
      caps: { os: 'linux', arch: 'x86_64', shell: 'bash', desktop: false, baseline: 'extended', baseline_check: { checked_at_ms: ago(60), level: 'extended', missing: ['pandoc'], ok: false, ext: { status: 'partial', missing: ['pandoc'], system: ['ffmpeg', 'python3'] } } },
      grant: { ops: allOps, roots: [], computer: 'none' },
      availability: 'available', builtin: true, default: false, lifecycle: 'active',
      usage: { sessions_total: 12, last_used_at_ms: ago(300), last_session_id: UI, workspaces: 2 },
      revision: 1, registered_by: 'app:jarvis@devtest', registered_at_ms: ago(86400), updated_at_ms: ago(86400),
      checked_at_ms: ago(60), last_error: null,
    },
    {
      version: 5, runtime_id: 'host', name: "Owner's desktop",
      desc: "The owner's own machine (node `desktop`), shared with the owner: the owner's files and tools are here; commands and files go through its node executor.",
      runtime_host: 'mock-host', connector: { type: 'node_executor', node_did: 'did:dev:mock', profile_id: 'agent-0123456789ab' }, enabled: true,
      caps: { os: 'macos', arch: 'aarch64', shell: 'zsh', desktop: true, identity_mode: 'current_user', baseline: 'standard',
        baseline_check: { checked_at_ms: ago(60), level: 'standard', missing: ['aicc-tool', 'gen_image'], ok: false, ext: { status: 'partial', missing: ['pandoc', 'tesseract'], system: ['ffmpeg'] } } },
      grant: { ops: allOps.filter((op) => op !== 'computer_act'), roots: ['/Users/devtest'], computer: 'observe' },
      availability: 'available', builtin: true, default: true, lifecycle: 'active',
      usage: { sessions_total: 1, last_used_at_ms: ago(370), last_session_id: WORK_RUNNING, workspaces: 1 },
      revision: 3, registered_by: 'app:jarvis@devtest', registered_at_ms: ago(86400), updated_at_ms: ago(600),
      checked_at_ms: ago(60), last_error: null,
    },
    {
      version: 5, runtime_id: 'build-box', name: 'Build box',
      desc: 'A Linux machine in the closet, assigned to this agent for long builds.',
      runtime_host: 'build-host', connector: { type: 'node_executor', node_did: 'did:dev:build', executor_id: 'exec-build', profile_id: 'agent-0123456789ab' }, enabled: true,
      caps: { os: 'linux', arch: 'x86_64', shell: 'bash', desktop: false, baseline: 'standard' },
      grant: { ops: allOps, roots: ['/srv/build'], computer: 'none' },
      availability: 'available', builtin: false, default: false, lifecycle: 'active',
      usage: { sessions_total: 4, last_used_at_ms: ago(86400), last_session_id: WORK_PENDING, workspaces: 0 },
      revision: 2, registered_by: 'app:jarvis@devtest', registered_at_ms: ago(86400 * 5), updated_at_ms: ago(86400),
      checked_at_ms: ago(600), last_error: null,
    },
    {
      version: 5, runtime_id: 'old-laptop', name: 'Old laptop',
      desc: 'Removed from this agent.',
      runtime_host: 'laptop-host', connector: { type: 'node_executor', node_did: 'did:dev:laptop', profile_id: 'agent-0123456789ab' }, enabled: false,
      caps: { os: 'windows', arch: 'x86_64', shell: 'powershell', desktop: true, baseline: 'standard' },
      grant: { ops: allOps, roots: ['C:\\Users\\devtest'], computer: 'none' },
      availability: 'runtime_unavailable', builtin: false, default: false, lifecycle: 'retired',
      usage: { sessions_total: 7, last_used_at_ms: ago(86400 * 30), last_session_id: null, workspaces: 0 },
      revision: 6, registered_by: 'app:jarvis@devtest', registered_at_ms: ago(86400 * 90), updated_at_ms: ago(86400 * 30),
      checked_at_ms: ago(86400 * 30), last_error: 'the `old-laptop` runtime is no longer assigned to this agent',
    },
  ]
  const runtimeSettings: RuntimeSettings = {
    runtimes: {
      host: { enabled: true, target: { kind: 'node', node_id: 'desktop', node_did: 'did:dev:mock' } },
      'build-box': { enabled: true, target: { kind: 'executor', executor_id: 'exec-build' } },
    },
    default_runtime_id: 'host',
    configured_default_runtime_id: 'host',
  }
  const findRuntime = (runtimeId: string) => {
    const runtime = runtimes.find((r) => r.runtime_id === runtimeId || (r.connector.type === 'native' && r.connector.aliases?.includes(runtimeId)))
    if (!runtime) throw new OpenDanError('not_found', `runtime ${runtimeId} is not registered`)
    return runtime
  }
  const impactOf = (runtimeId: string): RuntimeImpact => {
    const runtime = findRuntime(runtimeId)
    const open = fx.entries.filter((e) => e.status.run_state !== 'finished' && e.runtime_id === runtime.runtime_id)
    const todos = open.some((e) => e.session_id === WORK_RUNNING)
      ? [{ session_id: WORK_RUNNING, todo_id: 'todo-2', title: 'Write the weekly report', status: 'pending' }]
      : []
    return {
      runtime_id: runtime.runtime_id,
      owner_default: runtime.default,
      blocking: runtime.availability === 'available' && open.some((e) => e.status.run_state !== 'created'),
      running_sessions: runtime.availability === 'available' ? open.filter((e) => e.status.run_state !== 'created').map((e) => e.session_id) : [],
      active_sessions: open.map((e) => ({ session_id: e.session_id, kind: e.kind, class: e.class, title: e.objective, run_state: e.status.run_state })),
      pending_todos: todos,
      workspaces: workspaces.filter((w) => w.location.runtime_id === runtime.runtime_id)
        .map((w) => ({ workspace_id: w.workspace_id, name: w.name, directory: w.location.directory, lifecycle: w.lifecycle })),
      usage: structuredClone(runtime.usage),
    }
  }
  const notInUse = (runtimeId: string, verb: string) => {
    const impact = impactOf(runtimeId)
    if (!impact.blocking) return
    throw new OpenDanError('invalid_argument', `sessions_in_progress: runtime \`${impact.runtime_id}\` has ${impact.running_sessions.length} session(s) in progress (${impact.running_sessions.join(', ')}); stop them or let them finish, then ${verb}`)
  }
  const withRuntimeMark = (e: RegistryEntry): RegistryEntry => {
    const runtime = e.status.run_state !== 'finished' && runtimes.find((r) => r.runtime_id === e.runtime_id)
    return runtime && runtime.availability !== 'available'
      ? { ...e, runtime_unavailable: runtime.last_error ?? `runtime \`${runtime.runtime_id}\` is not usable` }
      : e
  }
  if (nativeRuntime) {
    const runtime = runtimes[0]
    runtime.connector = { type: 'native', aliases: ['host'] }
    runtime.name = "Owner's computer"
    runtime.desc = 'Private and owner default share this native runtime. Shell and files execute locally; desktop operations use local IPC.'
    runtime.default = true
    runtime.caps = { ...runtimes[1].caps, baseline_check: { checked_at_ms: ago(60), level: 'standard', missing: [], ok: true, ext: { status: 'ok', missing: [], system: [] } } }
    runtime.grant.computer = 'observe'
    runtimes.splice(1)
    delete runtimeSettings.runtimes['build-box']
  }
  const projectRuntimes = () => {
    const asSetting = (r: KnownRuntime) => (r.connector.type === 'native' ? (r.connector.aliases?.[0] ?? 'container') : r.runtime_id)
    const configured = runtimeSettings.configured_default_runtime_id
    runtimeSettings.default_runtime_id = configured && runtimeSettings.runtimes[configured]?.enabled ? configured : 'container'
    for (const r of runtimes) {
      const chosen = runtimeSettings.runtimes[asSetting(r)]
      r.default = runtimeSettings.default_runtime_id === asSetting(r)
      if (chosen) {
        r.enabled = chosen.enabled
        r.availability = chosen.enabled ? 'available' : 'runtime_unavailable'
        r.last_error = chosen.enabled ? null : `the \`${r.runtime_id}\` runtime is not enabled for this agent`
      }
    }
  }
  const availableRuntimeIds = () => runtimes.filter((r) => r.availability === 'available').map((r) => r.runtime_id).concat('local', 'container')
  const findWorkspace = (workspaceId: string) => {
    const workspace = workspaces.find((record) => record.workspace_id === workspaceId)
    if (!workspace) throw new OpenDanError('not_found', `workspace ${workspaceId} is not registered`)
    return workspace
  }
  let inputIndex = 8
  const profile: AgentProfile = {
    agent_did: AGENT,
    agent_id: 'jarvis',
    display_name: 'Jarvis',
    avatar: null,
    bio: 'Your personal agent. I keep an eye on your files and get things done while you are away.',
    owner_did: OWNER,
    desktop_url: 'https://test.buckyos.io',
    editable: true,
  }
  const tokens = (input: number, output: number) => ({ input, output, total: input + output })
  let worklogSeq = 1000

  const find = (sid: string): RegistryEntry => {
    const found = fx.entries.find((e) => e.session_id === sid) ?? goalEntries.find((e) => e.session_id === sid)
    if (!found) throw new OpenDanError('not_found', `session ${sid} is not registered`)
    return found
  }
  const log = (sid: string, body: Record<string, unknown> & { t: string }) => {
    worklogSeq += 1
    ;(fx.worklogs[sid] ??= []).push({ seq: worklogSeq, ...body })
  }
  const commit = (e: RegistryEntry, status: Partial<RegistryEntry['status']>) => {
    e.status = { ...e.status, ...status, rev: e.status.rev + 1, updated_at_ms: Date.now() }
    const st = fx.details[e.session_id]?.state
    if (st) {
      Object.assign(st, {
        rev: e.status.rev,
        run_state: e.status.run_state,
        outcome: e.status.outcome,
        acceptance: e.status.acceptance,
        one_line_status: e.status.one_line_status,
        updated_at_ms: e.status.updated_at_ms,
      })
    }
  }

  const skill = (kind: SkillMeta['kind'], name: string, description: string, extra: Partial<SkillMeta> = {}): SkillMeta => ({
    kind, name, description, origin: 'package', baseline: 'standard', size: 2400, enabled: true, ...extra,
  })
  const skills: SkillMeta[] = [
    skill('work', 'ui_core', 'What a conversation can do: sessions, reports, notifications.', { scope: 'ui', size: 6700, usage: { frozen: 31, last_ms: ago(400), recent_sessions: [UI] } }),
    skill('work', 'share', 'Publish a file or a folder and hand out the link.', { scope: 'ui,work', size: 1900, references: ['references/acl.md'], usage: { frozen: 3, last_ms: ago(86400 * 2) } }),
    skill('work', 'aigc_audio', 'Speech and music: TTS, transcription, cutting and joining audio.', {
      requires: 'an AICC provider with a speech model; ffmpeg only for cutting and joining', commands: ['ffmpeg', 'ffprobe'], size: 4100,
      usage: { frozen: 2, last_ms: ago(86400), recent_sessions: [WORK_RUNNING] },
    }),
    skill('work', 'doc_convert', 'Convert documents between Markdown, DOCX and PDF; OCR for scans.', {
      requires: 'the extended environment (pandoc, tesseract)', baseline: 'extended', commands: ['pandoc', 'tesseract'], size: 3300,
    }),
    skill('work', 'win_registry', 'Read and repair Windows registry settings.', { platforms: ['windows'], requires: 'a Windows runtime', size: 1500 }),
    skill('work', 'yt_fetch', 'Download a video and its subtitles for offline summarising.', {
      origin: 'installed', commands: ['yt-dlp'], size: 2100,
      install: { package: 'media-skills', version: '0.2.0', digest: 'sha256:9a41c0de', installed_at_ms: ago(86400 * 3), installed_by: OWNER },
      install_state: { risk: { level: 'medium', accepted_by: OWNER, accepted_at_ms: ago(86400 * 3), plan_digest: 'sha256:11aa' } },
    }),
    skill('work', 'old_invoice_flow', 'How invoices were filed before the accounting tool changed.', {
      origin: 'imported', size: 900, enabled: false, disabled: { at_ms: ago(86400 * 9), by: OWNER, reason: 'the accounting tool changed' },
      install: { package: 'imported-old-invoice-flow', version: '', digest: 'sha256:77b2', installed_at_ms: ago(86400 * 30), installed_by: OWNER, imported_from: { digest: 'sha256:5d10', runtime_id: 'private', platform: 'linux' } },
    }),
    skill('plan', 'research_report', 'How to split a research question into parallel work and one written report.', { size: 2800, usage: { frozen: 5, last_ms: ago(86400 * 4) } }),
  ]
  const skillAudit: SkillAudit[] = [
    { at_ms: ago(86400 * 9), by: OWNER, action: 'disable', skill: 'work:old_invoice_flow', note: 'the accounting tool changed' },
    { at_ms: ago(86400 * 3), by: OWNER, action: 'install', skill: 'work:yt_fetch', note: 'media-skills 0.2.0' },
    { at_ms: ago(7200), by: 'goal:skill_crystallize', action: 'revise', skill: 'work:old_invoice_flow', note: 'body rewritten from 3 sessions; previous text in revisions/' },
  ]
  const audit = (action: string, skillKey?: string, note?: string) => {
    skillAudit.push({ at_ms: Date.now(), by: OWNER, action, ...(skillKey ? { skill: skillKey } : {}), ...(note ? { note } : {}) })
  }
  const findSkill = (kind: string, name: string): SkillMeta => {
    const found = skills.find((s) => s.kind === kind && s.name === name)
    if (!found) throw new OpenDanError('not_found', `no ${kind} skill ${name}`)
    return found
  }
  let uiNames: string[] | null = null
  const uiCandidates = () => skills.filter((s) => s.kind === 'work' && s.enabled && s.scope?.split(',').includes('ui')).map((s) => s.name)
  const uiSkills = (): UiSkills => {
    const names = uiNames ?? ['ui_core']
    return {
      names: [...names],
      source: uiNames ? 'state' : 'template',
      total_bytes: names.reduce((sum, n) => sum + (skills.find((s) => s.kind === 'work' && s.name === n)?.size ?? 0), 0),
      available: uiCandidates().filter((n) => !names.includes(n)),
    }
  }
  const drafts: (Omit<SkillDraft, 'files'> & { texts: Record<string, string> })[] = [
    {
      slug: 'meeting-minutes',
      draft: {
        imported_from: { digest: 'sha256:c41f09', runtime_id: 'private', platform: 'linux' },
        split: 'one work skill; the template section became references/template.md',
        verified: [{ command: 'pandoc', present: false }, { command: 'ffmpeg', present: true }],
        risk: 'The manual makes the agent transcribe recordings and mail the minutes to every attendee. Sending the mail cannot be undone.',
      },
      skills: [skill('work', 'meeting_minutes', 'Turn a meeting recording into minutes and send them round.', { origin: 'local', commands: ['ffmpeg', 'pandoc'], baseline: 'extended', platforms: ['linux'], size: 1800 })],
      texts: {
        'draft.json': '{\n  "imported_from": {"digest": "sha256:c41f09", "runtime_id": "private", "platform": "linux"}\n}',
        'cmd_skills/meeting_minutes/SKILL.md': '---\nname: meeting_minutes\ndescription: Turn a meeting recording into minutes and send them round.\ncommands: ffmpeg, pandoc\n---\n\n1. Transcribe the recording.\n2. Write the minutes.\n3. Mail them to the attendees.\n',
      },
      updated_at_ms: ago(1800),
    },
    { slug: 'broken-notes', draft: null, skills: [], texts: { 'notes.txt': 'unfinished' }, updated_at_ms: ago(86400), error: 'draft.json is missing' },
  ]
  // What the real service works out from the files: one skill per manual.
  const planOf = (source: SkillSource, runtimeId?: string): InstallPlan => {
    const draft = 'draft' in source ? drafts.find((d) => d.slug === source.draft) : undefined
    if ('draft' in source && !draft) throw new OpenDanError('not_found', `no draft ${source.draft}`)
    const paths = draft ? Object.keys(draft.texts) : Object.keys('package' in source ? source.package.files : {})
    const tool = paths.find((path) => path.split('/')[0] === 'tools')
    if (tool) throw new OpenDanError('invalid_argument', `\`${tool}\`: a skill is text only and brings no command or install script; publish the commands as an extension package and name them in the manual's \`commands\``)
    const packageName = draft ? `imported-${draft.slug}` : 'package' in source ? source.package.package : ''
    const version = 'package' in source ? source.package.version : ''
    const dirs: Record<string, SkillMeta['kind']> = { cmd_skills: 'work', plan_skills: 'plan', goal_skills: 'goal' }
    const planned = paths.flatMap((path): PlannedSkill[] => {
      const [dir, name, file, ...rest] = path.split('/')
      if (!dirs[dir] || !name || rest.length || file !== 'SKILL.md') return []
      const kind = dirs[dir]
      const existing = skills.find((s) => s.kind === kind && s.name === name)
      const known = draft?.skills.find((s) => s.kind === kind && s.name === name)
      return [{
        ...(known ?? skill(kind, name, `Skill ${name} of ${packageName}`, { size: 1200 })),
        origin: draft ? 'imported' : 'installed',
        ...(existing ? { replaces: existing.origin } : {}),
      }]
    })
    if (!planned.length) throw new OpenDanError('invalid_argument', 'the package holds no skill')
    const findings: SkillFinding[] = [
      ...planned.filter((s) => s.replaces).map((s): SkillFinding => ({ code: 'replaces_skill', level: 'medium', where: `${s.kind}:${s.name}`, note: `takes the place of the ${s.replaces} skill of the same name` })),
      ...planned.filter((s) => s.scope?.split(',').includes('ui')).map((s): SkillFinding => ({ code: 'ui_scope', level: 'medium', where: `${s.kind}:${s.name}`, note: 'its scope includes `ui`: once the owner adds it to the UI set it goes into the prompt of every conversation' })),
      ...(draft ? [{ code: 'imported_text', level: 'medium', where: packageName, note: 'written by the agent from a text the owner gave it: nobody signed it' } satisfies SkillFinding] : []),
    ]
    const digest = `sha256:mock-${paths.length}-${paths.join('|').length}`
    return {
      package: packageName,
      version,
      digest,
      plan_digest: `sha256:plan-${packageName}-${version}-${paths.length}`,
      skills: planned,
      risk: { level: findings.some((f) => f.level === 'high') ? 'high' : findings.some((f) => f.level === 'medium') ? 'medium' : 'low', findings },
      command_status: planned.filter((s) => s.kind === 'work').map((s) => ({
        name: s.name,
        commands: s.commands ?? [],
        status: {
          state: s.commands?.includes('pandoc') ? 'partial' as const : 'ready' as const,
          commands: Object.fromEntries((s.commands ?? []).map((c) => [c, { present: c !== 'pandoc' }])),
          ...(s.commands?.includes('pandoc') ? { recommended_packages: [{ id: 'ext-docs', version: '0.3.1', provides: ['pandoc'] }] } : {}),
        },
      })),
      checked_runtime_id: findRuntime(runtimeId ?? 'private').runtime_id,
    }
  }

  const cognitions: (MemoryEntry & { revision: number })[] = memoryScene === 'empty' ? [] : Array.from({ length: 23 }, (_, i) => ({
    layer: 'cognition' as const,
    reference: `cog:${String(23 - i).padStart(3, '0')}`,
    summary: i === 0 ? 'The owner prefers weekly reports as a single PDF, sent on Monday morning.'
      : i === 1 ? 'Photos are archived by capture date, never by import date.'
      : `Fact ${23 - i} the agent concluded about the owner's files and habits.`,
    kind: i % 4 === 0 ? 'preference' : 'fact',
    state: i === 2 ? 'disputed' : 'active',
    at_ms: ago(600 * (i + 1)),
    ...(i === 1 ? { revised_at_ms: ago(300) } : {}),
    subjects: i % 3 === 0 ? [OWNER] : ['workspace:ws-photos'],
    deletable: true,
    revision: i === 1 ? 2 : 1,
  }))
  const perceptionStates = ['pending', 'pending', 'deferred', 'absorbed', 'absorbed', 'pending', 'absorbed', 'absorbed']
  const perceptions: MemoryEntry[] = memoryScene === 'empty' ? [] : perceptionStates.map((state, i) => ({
    layer: 'perception' as const,
    reference: `per:${UI}:${40 - i}`,
    summary: i === 0 ? 'The owner asked for the report before 9 am.' : `Observation ${40 - i} noted during a conversation.`,
    kind: 'observation',
    state,
    at_ms: ago(120 * (i + 1)),
    subjects: [OWNER],
    session_id: UI,
    deletable: state === 'pending' || state === 'deferred',
    ...(state === 'absorbed' ? { absorbed_into: ['cog:023'] } : {}),
  }))
  const memoryEntry = (e: MemoryEntry): MemoryEntry => e
  const descendants = (sid: string): RegistryEntry[] =>
    fx.entries.filter((e) => e.origin?.parent_session === sid).flatMap((e) => [e, ...descendants(e.session_id)])

  return {
    source: 'mock',
    login: { lost: () => false, subscribe: () => () => {} },

    loaderStatus: async () => {
      await delay()
      return {
        agent_did: AGENT,
        agent_id: 'jarvis',
        who: WHO,
        agent_root: ROOT,
        started_at_ms: ago(7200),
        now_ms: Date.now(),
        authentication: { state: 'ready', last_error: null, error_code: null, since: null, next_retry_time: null },
        modules: [
          { name: 'agent_state_service', enabled: true, running: true, note: 'port 4060' },
          { name: 'webui', enabled: true, running: true },
          { name: 'ui', enabled: true, running: true, note: '1 inbox' },
          { name: 'self_check', enabled: true, running: false, note: 'next run in 6h' },
          { name: 'memory_maintenance', enabled: true, running: true, note: 'built-in goal seeded by the goals module; decision pre-check off' },
        ],
        hosted: structuredClone(fx.hostedList),
        ui: {
          scans: 412,
          last_scan_ms: ago(3),
          last_error: 'msg-center: list_box_by_time timeout',
          inboxes: [
            { route_key: `${AGENT}/dm:${OWNER}`, session_id: UI, delivered: 7, dropped: 0, last_at_ms: ago(400) },
            { route_key: `${AGENT}/dm:did:bns:alice.buckyos`, delivered: 0, dropped: 2, held: 'session input queue is full', last_at_ms: ago(1800) },
          ],
        },
        errors: [
          { at_ms: ago(1800), source: 'ui', message: `${AGENT}/dm:did:bns:alice.buckyos: session input queue is full` },
          { at_ms: ago(5400), source: 'supervisor', message: `${WORK_FAILED}: drive ended with error (llm)` },
        ],
      }
    },

    profile: async () => {
      await delay()
      return { ...profile }
    },

    setProfile: async (patch) => {
      await delay()
      if (patch.display_name !== undefined) profile.display_name = patch.display_name.trim() || 'jarvis'
      if (patch.bio !== undefined) profile.bio = patch.bio.trim()
      if (patch.avatar !== undefined) profile.avatar = patch.avatar || null
      return { ...profile }
    },

    usageModels: async () => {
      await delay()
      return {
        now_ms: Date.now(),
        since_ms: ago(86400 * 6),
        models: [
          { model: 'claude-sonnet-5-5', hour: tokens(41200, 3800), day: tokens(612000, 48100), all: tokens(3804000, 301500) },
          { model: 'gpt-5.6', hour: tokens(0, 0), day: tokens(88400, 9100), all: tokens(1210300, 96400) },
          { model: 'qwen3-local', hour: tokens(5100, 420), day: tokens(20900, 1800), all: tokens(64000, 5200) },
          { model: 'llm.chat', hour: tokens(0, 0), day: tokens(0, 0), all: tokens(9100, 800) },
        ],
      }
    },

    uiBindings: async () => {
      await delay()
      return [{ session_id: UI, to: OWNER, to_session: null, kind: 'chat' }]
    },

    workspaces: async () => {
      await delay()
      return structuredClone(workspaces)
    },

    runtimes: async () => {
      await delay()
      return structuredClone(runtimes)
    },

    createWorkspace: async (request) => {
      await delay()
      if (!availableRuntimeIds().includes(request.location.runtime_id)) throw new OpenDanError('invalid_argument', 'Runtime is unavailable.')
      if (!request.location.directory.startsWith('/')) throw new OpenDanError('invalid_argument', 'Workspace directory must be absolute.')
      if (!request.name.trim()) throw new OpenDanError('invalid_argument', 'A name is required.')
      const workspace: KnownWorkspace = {
        version: 2, runtime_host: 'mock-host', workspace_id: `ws-${crypto.randomUUID()}`,
        name: request.name, description: request.description, url: request.url || null,
        location: { ...request.location }, usage: 'collaborative', lifecycle: 'active', availability: 'available',
        revision: 1, location_revision: 1, updated_at_ms: Date.now(), checked_at_ms: Date.now(),
        source_session: null, conflict: null, last_error: null,
      }
      workspaces.push(workspace)
      return structuredClone(workspace)
    },

    importWorkspace: async (request) => {
      await delay()
      if (!availableRuntimeIds().includes(request.location.runtime_id)) throw new OpenDanError('invalid_argument', 'Runtime is unavailable.')
      if (!request.location.directory.startsWith('/')) throw new OpenDanError('invalid_argument', 'Workspace directory must be absolute.')
      const workspace: KnownWorkspace = {
        version: 2, runtime_host: 'mock-host', workspace_id: `ws-${crypto.randomUUID()}`,
        name: request.name || request.location.directory.split('/').filter(Boolean).pop() || 'workspace',
        description: request.description, url: request.url || null,
        location: { ...request.location }, usage: 'collaborative', lifecycle: 'active', availability: 'available',
        revision: 1, location_revision: 1, updated_at_ms: Date.now(), checked_at_ms: Date.now(),
        source_session: null, conflict: null, last_error: null,
      }
      workspaces.push(workspace)
      return structuredClone(workspace)
    },

    setWorkspaceUrl: async (expected, url) => {
      await delay()
      const workspace = findWorkspace(expected.workspace_id)
      if (workspace.revision !== expected.revision) throw new OpenDanError('conflict', 'Workspace registration changed; reload before retrying.')
      if (url && !/^https?:\/\/[^\s@]+$/.test(url)) throw new OpenDanError('invalid_argument', 'The project URL must be an http(s) link without credentials.')
      workspace.url = url || null
      workspace.revision += 1
      workspace.updated_at_ms = Date.now()
      return structuredClone(workspace)
    },

    runtimeExtTools: async (runtimeId) => ({
      runtime_id: runtimeId,
      tools_root: '/opt/buckyos/tools',
      managed_by: runtimeId === 'private' ? 'node_volume' : 'node',
      packages: { 'ext-core': { version: '1.1.0', origin: 'seed', commands: { ffmpeg: 'ffmpeg version 9.0.2' } } },
    }),
    checkRuntime: async (runtimeId) => {
      await delay()
      const runtime = findRuntime(runtimeId)
      runtime.checked_at_ms = Date.now()
      if (runtime.runtime_id === 'private') {
        runtime.caps.baseline_check = { checked_at_ms: Date.now(), level: 'extended', missing: [], ok: true, ext: { status: 'system', missing: [], system: ['ffmpeg', 'python3'] } }
      }
      return structuredClone(runtime)
    },

    runtimeSettings: async () => {
      await delay()
      return structuredClone(runtimeSettings)
    },

    setRuntimeSetting: async (patch) => {
      await delay()
      const record = runtimes.find((r) => r.runtime_id === patch.runtime_id)
      const settingId = record?.connector.type === 'native' ? (record.connector.aliases?.[0] ?? 'container') : patch.runtime_id
      const entry = runtimeSettings.runtimes[settingId]
      if (settingId !== 'container' && !entry) throw new OpenDanError('invalid_argument', `runtime ${patch.runtime_id} is not one of this agent's runtimes`)
      if (entry && patch.enabled !== undefined) {
        if (!patch.enabled && entry.enabled) notInUse(patch.runtime_id, 'switch it off')
        entry.enabled = patch.enabled
      }
      if (patch.default === true) runtimeSettings.configured_default_runtime_id = settingId
      if (patch.default === false && runtimeSettings.configured_default_runtime_id === settingId) runtimeSettings.configured_default_runtime_id = null
      projectRuntimes()
      return structuredClone(runtimeSettings)
    },

    runtimeImpact: async (runtimeId) => {
      await delay()
      return impactOf(runtimeId)
    },

    removeRuntime: async (runtimeId) => {
      await delay()
      const record = findRuntime(runtimeId)
      if (record.connector.type === 'native') throw new OpenDanError('invalid_argument', 'the private runtime cannot be removed')
      notInUse(runtimeId, 'remove it')
      delete runtimeSettings.runtimes[record.runtime_id]
      if (runtimeSettings.configured_default_runtime_id === record.runtime_id) runtimeSettings.configured_default_runtime_id = null
      record.lifecycle = 'retired'
      record.enabled = false
      record.availability = 'runtime_unavailable'
      projectRuntimes()
      return structuredClone(runtimeSettings)
    },

    checkWorkspace: async (workspaceId) => {
      await delay()
      const workspace = findWorkspace(workspaceId)
      workspace.checked_at_ms = Date.now()
      workspace.revision += 1
      return structuredClone(workspace)
    },

    relocateWorkspace: async (expected, location) => {
      await delay()
      const workspace = findWorkspace(expected.workspace_id)
      if (workspace.revision !== expected.revision) throw new OpenDanError('conflict', 'Workspace registration changed; reload before retrying.')
      if (!location.directory.startsWith('/')) throw new OpenDanError('invalid_argument', 'Workspace directory must be absolute.')
      if (!availableRuntimeIds().includes(location.runtime_id)) throw new OpenDanError('invalid_argument', 'Runtime is unavailable.')
      if (workspace.location.runtime_id !== 'local') throw new OpenDanError('conflict', 'Cannot verify relocation while the original runtime is unavailable.')
      if (workspaces.some((record) => record.workspace_id !== workspace.workspace_id && record.location.directory === location.directory)) {
        throw new OpenDanError('conflict', 'Directory identity does not match the selected workspace.')
      }
      workspace.location = { ...location }
      workspace.location_revision += 1
      workspace.revision += 1
      workspace.availability = 'available'
      workspace.last_error = null
      workspace.checked_at_ms = Date.now()
      workspace.updated_at_ms = Date.now()
      return structuredClone(workspace)
    },

    sessions: async () => {
      await delay()
      return structuredClone(fx.entries.map(withRuntimeMark))
    },

    activeSessions: async () => {
      await delay()
      return fx.entries
        .filter((e) => e.status.run_state === 'running' || e.status.run_state === 'waiting')
        .map((e) => ({
          session_id: e.session_id,
          kind: e.kind,
          run_state: e.status.run_state,
          objective: e.objective,
          activity: e.status.activity,
          relation: 'other' as const,
          overlap: [],
          possibly_interrupted: false,
          driver: e.driver.principal,
        }))
    },

    session: async (sid, worklog = 40) => {
      await delay()
      const e = find(sid)
      return structuredClone({
        ...fx.details[sid],
        entry: withRuntimeMark(e),
        worklog: (fx.worklogs[sid] ?? []).slice(-worklog),
        children: fx.entries.filter((c) => c.origin?.parent_session === sid).map((c) => c.session_id),
        hosted: fx.hostedList.find((h) => h.session_id === sid) ?? null,
      })
    },

    stopSession: async (sid, scope, reason) => {
      await delay()
      const e = find(sid)
      if (e.status.run_state === 'finished') throw new OpenDanError('session_finished', `session ${sid} is finished`)
      if (scope === 'session' && e.kind === 'ui') {
        throw new OpenDanError('invalid_argument', `session ${sid} is a UI session: it is not stopped; start a new conversation to clear its context`)
      }
      inputIndex += 1
      log(sid, { t: 'control_applied', input: { src: 'queue', index: inputIndex, kind: 'control' }, command: 'stop', detail: { scope, ...(reason ? { reason } : {}) } })
      const st = fx.details[sid]?.state
      if (scope === 'session' && e.kind === 'goal') {
        const g = goals.find((x) => x.def.session.sid === sid)
        if (g) Object.assign(g.def, { status: 'paused', status_changed_at_ms: Date.now(), status_reason: reason, status_by: OWNER })
      } else if (scope === 'turn') {
        // An interrupted Turn that is given up clears the interruption with it.
        commit(e, { run_state: 'waiting', turn_open: false, needs_input: e.kind === 'ui', paused: e.status.paused === 'interrupted' ? undefined : e.status.paused, one_line_status: 'Stopped the current message' })
        if (st) Object.assign(st, { open_turn: undefined })
      } else {
        for (const s of [e, ...descendants(sid).filter((c) => c.status.run_state !== 'finished')]) {
          commit(s, { run_state: 'finished', outcome: 'stopped', needs_input: false, turn_open: false, paused: undefined, one_line_status: reason ? `Stopped: ${reason}` : 'Stopped' })
        }
        if (st) Object.assign(st, { open_turn: undefined, wait_children: undefined, stop_requested: true })
      }
      return { index: inputIndex }
    },

    resumeSession: async (sid) => {
      await delay()
      const e = find(sid)
      if (e.status.run_state === 'finished') throw new OpenDanError('session_finished', `session ${sid} is finished`)
      inputIndex += 1
      const source = e.status.paused
      log(sid, { t: 'control_applied', input: { src: 'queue', index: inputIndex, kind: 'control' }, command: 'resume', applied: Boolean(source) })
      if (source) {
        // A resume cascades from a paused session only, to the sub sessions paused for the same reason.
        const follows = source === 'user' ? 'parent' : source
        commit(e, { paused: undefined })
        for (const child of descendants(sid)) if (child.status.paused === follows) commit(child, { paused: undefined })
      }
      return { index: inputIndex }
    },

    deleteSession: async (sid) => {
      await delay()
      const e = find(sid)
      if (e.kind !== 'work' && e.kind !== 'self_check') throw new OpenDanError('invalid_argument', `session ${sid} is a ${e.kind} session: only work and self-check sessions are deleted`)
      const all = [e, ...descendants(sid)]
      const open = all.filter((x) => x.status.run_state !== 'finished')
      if (open.length) throw new OpenDanError('invalid_argument', `session ${sid} has unfinished sessions (${open.map((x) => x.session_id).join(', ')})`)
      const deleted = all.map((x) => x.session_id)
      fx.entries.splice(0, fx.entries.length, ...fx.entries.filter((x) => !deleted.includes(x.session_id)))
      return { deleted }
    },

    memoryOverview: async () => {
      await delay()
      const maintenance = memoryScene === 'empty' ? undefined : goals.find((g) => g.def.goal_id === GOAL_BUILTIN)
      return structuredClone({
        cognitions: { current: cognitions.length },
        perceptions: { total: perceptions.length, pending: perceptions.filter((p) => p.state === 'pending' || p.state === 'deferred').length },
        last_consolidation_at_ms: maintenance ? ago(5400) : null,
        maintenance_goal_id: maintenance?.def.goal_id ?? null,
        recent: { cognitions: cognitions.slice(0, 5).map(memoryEntry), perceptions: perceptions.slice(0, 3) },
      })
    },

    memoryList: async (layer, limit, cursor) => {
      await delay()
      const all = layer === 'cognition' ? cognitions.map(memoryEntry) : perceptions
      const start = cursor ? Math.max(0, all.findIndex((e) => e.reference === cursor) + 1) : 0
      const entries = all.slice(start, start + limit)
      return structuredClone({ entries, next_cursor: start + limit < all.length ? entries[entries.length - 1].reference : null })
    },

    memoryRead: async (reference) => {
      await delay()
      const cognition = cognitions.find((c) => c.reference === reference)
      if (cognition) {
        return structuredClone({
          detail: { statement: cognition.summary, confidence: 0.8, sources: [{ perception: `per:${UI}:33`, session_id: UI }] },
        })
      }
      const perception = perceptions.find((p) => p.reference === reference)
      if (!perception) throw new OpenDanError('not_found', `memory ${reference} does not exist`)
      return structuredClone({ detail: { seq: Number(reference.split(':').pop()), content: perception.summary, by: `session:${UI}` } })
    },

    memoryForget: async (reference, expectedRevision) => {
      await delay()
      if (memoryScene === 'busy') throw new OpenDanError('busy', 'memory consolidation is running; try again later')
      const cognition = cognitions.findIndex((c) => c.reference === reference)
      if (cognition >= 0) {
        if (expectedRevision != null && expectedRevision !== cognitions[cognition].revision) throw new OpenDanError('conflict', `${reference} was revised; reload before retrying`)
        cognitions.splice(cognition, 1)
        return { reference, state: 'deleted' }
      }
      const perception = perceptions.findIndex((p) => p.reference === reference)
      if (perception < 0) throw new OpenDanError('conflict', `${reference} is already deleted`)
      if (!perceptions[perception].deletable) {
        throw new OpenDanError('invalid_argument', `${reference} is already consolidated into ${perceptions[perception].absorbed_into}`)
      }
      perceptions.splice(perception, 1)
      return { reference, state: 'discarded' }
    },

    decideSession: async (sid, decision, note) => {
      await delay()
      const e = find(sid)
      if (e.status.run_state !== 'finished' || e.status.acceptance !== 'pending') {
        throw new OpenDanError('invalid_argument', `session ${sid} has no pending decision`)
      }
      inputIndex += 1
      log(sid, { t: 'decide', decision, by: WHO, report: note ? { note } : undefined })
      commit(e, { acceptance: decision === 'accept' ? 'accepted' : 'discarded', pending_decision: undefined })
      return { index: inputIndex }
    },

    postMessage: async (sid, text) => {
      await delay()
      const e = find(sid)
      if (!e.input_queue) throw new OpenDanError('session_readonly', `session ${sid} has no input queue`)
      if (e.status.run_state === 'finished') throw new OpenDanError('session_finished', `session ${sid} is finished`)
      inputIndex += 1
      log(sid, { t: 'user_message', run_id: fx.details[sid]?.state?.last_run ?? '', turn: (fx.details[sid]?.state?.turn_seq ?? 0) + 1, content: text })
      return { index: inputIndex, key: `msg:mock-${inputIndex}` }
    },

    artifacts: async () => {
      await delay()
      return [
        { aid: 'art-photo-rename', workspace: { workspace_id: 'ws-photos' }, head: 'v1', rev: 3, updated_at_ms: ago(900) },
        { aid: 'art-weekly-report', workspace: { workspace_id: 'ws-downloads' }, head: null, rev: 1, updated_at_ms: ago(100) },
      ]
    },

    artifactVersions: async (aid) => {
      await delay()
      return fx.artifactVersions[aid] ?? []
    },

    behaviors: async () => {
      await delay()
      return {
        revision: 'sha256:9a41c0de',
        behaviors: [
          { name: 'ui_chat', objective: 'Talk with the user and dispatch work', next: [] },
          { name: 'work_plan', objective: 'Plan the task', next: ['work_do'] },
          { name: 'work_do', objective: 'Execute the plan', next: ['work_report'] },
          { name: 'work_report', objective: 'Write the report', next: [] },
        ],
      }
    },

    behavior: async (name) => {
      await delay()
      return {
        meta: { name, objective: `Objective of ${name}` },
        loop: 'function_call',
        tools: ['shell', 'read_file', 'session.create'],
        prompt: { system: `You are Jarvis running the ${name} behavior.` },
        limits: { max_steps: 24 },
      }
    },

    identity: async () => {
      await delay()
      return { role: 'Jarvis, the personal agent of devtest.', self: 'I am careful, concise and I report what I did.' }
    },

    queryMemory: async (_session, words) => {
      await delay()
      const all = [
        { id: 'item_photos_dir', time: '2026-09-28T00:00:00Z', sentence: 'Photos live under /home/devtest/Pictures/camera.', kind: 'fact', tags: ['photos'] },
        { id: 'item_report_style', time: '2026-09-30T00:00:00Z', sentence: 'The user prefers reports as markdown with a short summary first.', kind: 'preference', tags: ['report'] },
        { id: 'item_rename_accept', time: '2026-10-01T00:00:00Z', sentence: 'Renaming photos requires an accept decision before files are moved.', kind: 'rule', tags: ['photos', 'rename'] },
      ]
      const cognitions = all
        .filter((h) => words.length > 0 && h.tags.some((t) => words.includes(t)))
        .map((h) => ({
          layer: 'cognition' as const,
          reference: `${h.id}@1`,
          id: h.id,
          summary: h.sentence,
          kind: h.kind,
          state: 'active',
          observed_at: h.time,
        }))
      return {
        status: words.length ? ('recalled' as const) : ('not_triggered' as const),
        cognitions,
        recent_perceptions: [],
        pending_in_scope: 0,
      }
    },

    goals: async () => {
      await delay()
      return structuredClone(goals.map((g) => g.def))
    },

    goal: async (goalId) => {
      await delay()
      const g = goals.find((x) => x.def.goal_id === goalId)
      return g ? structuredClone(g.def) : null
    },

    goalRevision: async (goalId, revision) => {
      await delay()
      const g = goals.find((x) => x.def.goal_id === goalId)
      if (!g || revision > g.def.revision || revision < 1) return null
      return structuredClone({ ...g.def, revision, constraints: revision === g.def.revision ? g.def.constraints : (g.def.constraints ?? []).slice(0, -1) })
    },

    goalChanges: async (goalId, limit = 0) => {
      await delay()
      const g = findGoal(goalId)
      return structuredClone(limit ? g.changes.slice(-limit) : g.changes)
    },

    goalState: async (goalId) => {
      await delay()
      const g = goals.find((x) => x.def.goal_id === goalId)
      return g ? structuredClone(g.state) : null
    },

    goalSchedule: async (goalId) => {
      await delay()
      const g = goals.find((x) => x.def.goal_id === goalId)
      return g ? structuredClone(g.schedule) : null
    },

    goalActivations: async (goalId, epoch, limit = 0) => {
      await delay()
      const g = findGoal(goalId)
      const all = g.activations.filter((a) => epoch == null || a.epoch === epoch)
      return structuredClone(limit ? all.slice(-limit) : all)
    },

    activateGoal: async (goalId, reason) => {
      await delay()
      const g = findGoal(goalId)
      if (g.def.status === 'archived' || g.def.status === 'achieved') throw new OpenDanError('invalid_argument', `goal ${goalId} is ${g.def.status}`)
      const epoch = g.def.session.epoch
      const seq = Math.max(0, ...g.activations.filter((a) => a.epoch === epoch).map((a) => a.seq)) + 1
      const note = reason || `activated by ${OWNER}`
      const started = Date.now()
      const reasons: GoalActivation['reasons'] = [{ kind: 'manual', key: `manual:${goalId}:${epoch}:${seq}`, note }]
      g.activations.push(activation(goalId, epoch, seq, started, {
        reasons, decision: 'no_action', summary: `Manual activation: looked, nothing to do (${note}).`, cost: { runs: 1, total_tokens: 1900 },
      }))
      g.state.last_activation = { epoch, seq, at_ms: started, reasons, outcome: 'completed', decision: 'no_action', summary: `Manual activation: looked, nothing to do (${note}).` }
      g.state.usage.tokens = (g.state.usage.tokens ?? 0) + 1900
      g.state.usage.activations = (g.state.usage.activations ?? 0) + 1
      g.state.rev += 1
      g.state.updated_at_ms = started
      if (g.schedule.gate) Object.assign(g.schedule.gate, { last_run_at_ms: started, last_checked_at_ms: started })
      inputIndex += 1
      return { index: inputIndex, key: `goal:${goalId}:activate:${seq}` }
    },

    setGoalStatus: async (goalId, status, reason) => {
      await delay()
      const g = findGoal(goalId)
      if (status === 'achieved') throw new OpenDanError('forbidden', 'achieved is reported by the Goal Session itself')
      if (!(g.def.status === 'active' || g.def.status === 'paused')) throw new OpenDanError('invalid_argument', `goal ${goalId} is ${g.def.status}`)
      g.def.status = status
      g.def.status_changed_at_ms = Date.now()
      g.def.status_reason = reason
      g.def.status_by = OWNER
      if (g.schedule.gate) {
        g.schedule.gate.status = status === 'paused' ? 'paused' : status === 'active' ? 'no_change' : 'disabled'
        if (status === 'paused') g.schedule.gate.last_skip_reason = 'paused'
      }
      if (status === 'paused') (g.schedule.skipped ??= []).push({ slot: slotNow, at_ms: Date.now(), reason: 'paused' })
      if (status === 'archived') {
        const current = g.def.epochs?.find((ep) => ep.epoch === g.def.session.epoch)
        if (current) Object.assign(current, { ended_at_ms: Date.now(), end: 'archived', end_reason: reason })
        goalChange(g, 'epoch_ended', { epoch: g.def.session.epoch, reason })
      }
      goalChange(g, 'status', { reason, detail: { status } })
      return structuredClone(g.def)
    },

    reopenGoal: async (goalId, reason, freshState = false) => {
      await delay()
      const g = findGoal(goalId)
      if (!(g.def.status === 'archived' || g.def.status === 'achieved')) throw new OpenDanError('invalid_argument', `goal ${goalId} is ${g.def.status}: only an archived or achieved goal reopens`)
      return openEpoch(g, 'reopened', reason, freshState ? 'facts' : 'all')
    },

    restartGoal: async (goalId, reason, carryState = false) => {
      await delay()
      const g = findGoal(goalId)
      if (!(g.def.status === 'active' || g.def.status === 'paused')) throw new OpenDanError('invalid_argument', `goal ${goalId} is ${g.def.status}: only a live goal restarts`)
      return openEpoch(g, 'restarted', reason, carryState ? 'all' : 'facts')
    },

    skills: async () => {
      await delay()
      return structuredClone(skills)
    },

    skillsState: async () => {
      await delay()
      return {
        disabled: Object.fromEntries(skills.filter((s) => s.disabled).map((s) => [`${s.kind}:${s.name}`, s.disabled!])),
        ...(uiNames ? { ui_cmd_skills: [...uiNames] } : {}),
        usage: { ...Object.fromEntries(skills.filter((s) => s.usage).map((s) => [`${s.kind}:${s.name}`, s.usage!])), none: { frozen: 4, last_ms: ago(5400), recent_sessions: [WORK_FAILED] } },
        audit: structuredClone(skillAudit),
      }
    },

    uiSkills: async () => {
      await delay()
      return uiSkills()
    },

    skillsStatus: async (runtimeId = 'private') => {
      await new Promise((resolve) => window.setTimeout(resolve, 400))
      const runtime = findRuntime(runtimeId)
      const onPrivate = runtime.connector.type === 'native'
      return {
        runtime_id: runtime.runtime_id,
        skills: skills.filter((s) => s.kind === 'work').map((s) => {
          const unmet = s.platforms?.length && !s.platforms.includes(runtime.caps.os)
            ? `works on ${s.platforms.join(', ')} only (this runtime is ${runtime.caps.os})`
            : s.baseline === 'extended' && runtime.caps.baseline !== 'extended' ? 'needs the extended environment, and this runtime declares standard' : undefined
          const commands = Object.fromEntries((s.commands ?? []).map((c) => [c, c === 'pandoc' || (!onPrivate && c === 'tesseract')
            ? { present: false }
            : { present: true, provided_by: c === 'ffmpeg' ? 'system' : 'exttools' }]))
          const missing = Object.entries(commands).filter(([, c]) => !c.present).map(([name]) => name)
          return {
            name: s.name,
            ...(s.scope ? { scope: s.scope } : {}),
            enabled: s.enabled,
            off: !s.enabled,
            ...(s.requires ? { requires: s.requires } : {}),
            ...(unmet ? { unmet } : {}),
            commands: {
              state: missing.length ? 'partial' as const : Object.values(commands).some((c) => c.present && c.provided_by === 'system') ? 'ready_system' as const : 'ready' as const,
              commands,
              ...(missing.includes('pandoc') ? { recommended_packages: [{ id: 'ext-docs', version: '0.3.1', requires_chain: ['ext-core'], provides: ['pandoc'] }] } : {}),
              ...(missing.includes('tesseract') ? { no_package: ['tesseract'] } : {}),
            },
          }
        }),
      }
    },

    skillInstallPlan: async (source, runtimeId) => {
      await delay()
      return planOf(source, runtimeId)
    },

    skillDrafts: async () => {
      await delay()
      return drafts.map(({ texts, ...draft }) => structuredClone({ ...draft, files: Object.keys(texts) }))
    },

    skillDraftFile: async (slug, file) => {
      await delay()
      const text = drafts.find((d) => d.slug === slug)?.texts[file]
      if (text === undefined) throw new OpenDanError('not_found', `draft ${slug} has no file ${file}`)
      return { text }
    },

    installSkills: async (source, planDigest, replace = false) => {
      await delay()
      const plan = planOf(source)
      if (plan.plan_digest !== planDigest) throw new OpenDanError('invalid_argument', 'the package is not the one that was planned (plan_digest differs)')
      if (plan.skills.some((s) => s.replaces) && !replace) throw new OpenDanError('invalid_argument', 'a skill of the same name is installed (replace)')
      const origin = 'draft' in source ? 'imported' as const : 'installed' as const
      for (const planned of plan.skills) {
        const meta: SkillMeta & Partial<PlannedSkill> = { ...planned }
        delete meta.replaces
        const at = skills.findIndex((s) => s.kind === meta.kind && s.name === meta.name)
        const installed: SkillMeta = {
          ...meta,
          origin,
          install: { package: plan.package, version: plan.version, digest: plan.digest, installed_at_ms: Date.now(), installed_by: OWNER },
          install_state: { risk: { level: plan.risk.level, accepted_by: OWNER, accepted_at_ms: Date.now(), plan_digest: plan.plan_digest } },
        }
        if (at >= 0) skills[at] = installed
        else skills.push(installed)
        audit('install', `${meta.kind}:${meta.name}`, `${plan.package} ${plan.version}`.trim())
      }
      if ('draft' in source) drafts.splice(drafts.findIndex((d) => d.slug === source.draft), 1)
      return { package: plan.package, version: plan.version, digest: plan.digest, skills: plan.skills, plan }
    },

    removeSkill: async (kind, name) => {
      await delay()
      const skill = findSkill(kind, name)
      if (skill.origin !== 'installed' && skill.origin !== 'imported') throw new OpenDanError('invalid_argument', `${name} is a ${skill.origin} skill: disable it instead`)
      skills.splice(skills.indexOf(skill), 1)
      if (uiNames) uiNames = uiNames.filter((n) => n !== name)
      audit('remove', `${kind}:${name}`)
      return structuredClone(skill)
    },

    setSkillEnabled: async (kind, name, enabled, reason) => {
      await delay()
      const skill = findSkill(kind, name)
      skill.enabled = enabled
      skill.disabled = enabled ? undefined : { at_ms: Date.now(), by: OWNER, ...(reason ? { reason } : {}) }
      audit(enabled ? 'enable' : 'disable', `${kind}:${name}`, reason)
      return structuredClone(skill)
    },

    setUiSkills: async (names) => {
      await delay()
      const candidates = uiCandidates()
      const unknown = names?.find((n) => !candidates.includes(n))
      if (unknown) throw new OpenDanError('invalid_argument', `\`${unknown}\` is not an enabled work skill whose scope includes ui`)
      uiNames = names ? [...names] : null
      audit('ui_set', undefined, names ? names.join(', ') : 'back to the template')
      return uiSkills()
    },

    importSkill: async (request) => {
      await delay()
      if (!request.text.trim()) throw new OpenDanError('invalid_argument', 'no text to import')
      const slug = request.name?.trim() || `import-${drafts.length + 1}`
      const scope = request.scope && request.scope !== 'work' ? request.scope : undefined
      const kind = scope === 'plan' ? 'plan' as const : 'work' as const
      const dir = kind === 'plan' ? 'plan_skills' : 'cmd_skills'
      drafts.push({
        slug,
        draft: { imported_from: { digest: 'sha256:mock', runtime_id: 'private', platform: 'linux' }, risk: 'Mock import: nothing was analysed.', verified: [] },
        skills: [{ kind, name: slug, ...(scope && kind === 'work' ? { scope } : {}), description: request.text.trim().split('\n')[0].slice(0, 80), origin: 'local', size: request.text.length, enabled: true }],
        texts: { 'draft.json': '{}', [`${dir}/${slug}/SKILL.md`]: request.text },
        updated_at_ms: Date.now(),
      })
      return { session_id: WORK_RUNNING, inbox: { index: 1 } }
    },

    discardSkillDraft: async (slug) => {
      await delay()
      const at = drafts.findIndex((d) => d.slug === slug)
      if (at < 0) throw new OpenDanError('not_found', `no draft ${slug}`)
      drafts.splice(at, 1)
      audit('draft_discard', undefined, slug)
      return {}
    },
  }
}
