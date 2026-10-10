import { OpenDanError, type OpenDanDataModel } from './datamodel'
import type {
  AgentProfile,
  ArtifactVersion,
  HostedStatus,
  KnownWorkspace,
  PerceptionRecord,
  RegistryEntry,
  SessionDetail,
  SessionState,
  WorklogEntry,
} from './types'

const WHO = 'app:jarvis@devtest'
const AGENT = 'did:bns:jarvis.devtest'
const OWNER = 'did:bns:devtest'
const ROOT = '/opt/buckyos/data/home/devtest/.local/share/jarvis/agents/jarvis'
const now = Date.now()
const ago = (seconds: number) => now - seconds * 1000

const UI = 'ui-7f3a2c91'
const WORK_RUNNING = 'work-b41d09e2'
const WORK_PENDING = 'work-5c8e17aa'
const WORK_FAILED = 'work-e02b66f4'
const CHECK = 'sc-daily'

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
    waiting_for: 'input',
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
      last_result: { kind: 'turn_open', rev: 30, turn: 2, waiting_for: { kind: 'tool', refs: ['task-55'] } },
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
        waiting_for: { kind: 'input' },
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
      binding: { schema: 'opendan.binding/4', target: { kind: 'native' }, runtime_id: 'rt-native-01', kind: 'native', workdir: `${ROOT}/sessions/${UI}/work`, bound_at_ms: ago(86000), bound_by: WHO },
      statistics: { input_tokens: 48210, output_tokens: 3922, total_tokens: 52132, rounds: 11, rounds_failed: 0, rounds_interrupted: 0, turns: 3 },
      runs: ['run-0001', 'run-0002', 'run-0003'],
      lease_holder: null,
    },
    [WORK_RUNNING]: {
      state: state(running, {
        turn_seq: 2,
        turns_completed: 1,
        open_turn: { index: 2, run_id: 'run-0002', input_seq: 3, inputs: ['queue#3'], at_ms: ago(120) },
        waiting_for: { kind: 'tool', refs: ['task-55'], deadline_ms: now + 600000 },
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
      binding: { schema: 'opendan.binding/4', target: { kind: 'tmux', session: 'od-b41d' }, runtime_id: 'rt-tmux-07', kind: 'tmux', workdir: '/home/devtest/Downloads', bound_at_ms: ago(370), bound_by: WHO },
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

  const perception: PerceptionRecord[] = [
    { seq: 7, at_ms: ago(900), session_id: WORK_PENDING, kind: 'session_finished', source: 'session', tags: ['photos', 'rename'], summary: 'Renamed 212 photos, 3 skipped' },
    { seq: 8, at_ms: ago(880), session_id: WORK_PENDING, kind: 'decision_pending', source: 'session', summary: 'Waiting for accept / discard of v2' },
    { seq: 3, at_ms: ago(5400), session_id: WORK_FAILED, kind: 'session_finished', source: 'session', tags: ['llm'], summary: 'LLM request failed' },
  ]

  return { entries, hostedList, worklogs, details, artifactVersions, perception }
}

const delay = () => new Promise((resolve) => window.setTimeout(resolve, 60))

export const createMockDataModel = (): OpenDanDataModel => {
  const fx = buildFixtures()
  const workspaces: KnownWorkspace[] = [
    {
      version: 1, runtime_host: 'mock-host',
      workspace_id: 'ws-downloads', name: 'Downloads', description: 'Weekly reports from shared downloads.',
      location: { runtime_id: 'local', directory: '/home/devtest/Downloads' },
      usage: 'collaborative', lifecycle: 'active', availability: 'available',
      revision: 2, location_revision: 1, updated_at_ms: ago(4), checked_at_ms: ago(30),
      source_session: WORK_RUNNING, conflict: null, last_error: null,
    },
    {
      version: 1, runtime_host: 'mock-host',
      workspace_id: 'ws-photos', name: 'Photos', description: 'Organize photographs and keep rename reports.',
      location: { runtime_id: 'local', directory: `${ROOT}/workspace/photos` },
      usage: 'private', lifecycle: 'active', availability: 'available',
      revision: 1, location_revision: 1, updated_at_ms: ago(900), checked_at_ms: ago(900),
      source_session: WORK_PENDING, conflict: null, last_error: null,
    },
    {
      version: 1, runtime_host: 'studio-host',
      workspace_id: 'ws-design-archive', name: 'Design archive', description: 'Long-term project on a disconnected workstation.',
      location: { runtime_id: 'studio-workstation', directory: '/projects/design/archive' },
      usage: 'collaborative', lifecycle: 'archived', availability: 'runtime_unavailable',
      revision: 4, location_revision: 2, updated_at_ms: ago(7200), checked_at_ms: ago(120),
      source_session: null, conflict: null, last_error: 'Runtime studio-workstation is unavailable; the workspace registration is retained.',
    },
    {
      version: 1, runtime_host: 'mock-host',
      workspace_id: 'ws-project-notes', name: 'Project notes', description: 'Archived notes whose directory was moved.',
      location: { runtime_id: 'local', directory: '/home/devtest/Projects/notes-old' },
      usage: 'collaborative', lifecycle: 'archived', availability: 'missing',
      revision: 2, location_revision: 1, updated_at_ms: ago(8000), checked_at_ms: ago(120),
      source_session: null, conflict: null, last_error: 'Directory does not exist; locate the moved workspace to restore access.',
    },
  ]
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
    const found = fx.entries.find((e) => e.session_id === sid)
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

  return {
    source: 'mock',

    loaderStatus: async () => {
      await delay()
      return {
        agent_did: AGENT,
        agent_id: 'jarvis',
        who: WHO,
        agent_root: ROOT,
        started_at_ms: ago(7200),
        now_ms: Date.now(),
        modules: [
          { name: 'agent_state_service', enabled: true, running: true, note: 'port 4060' },
          { name: 'webui', enabled: true, running: true },
          { name: 'ui', enabled: true, running: true, note: '1 inbox' },
          { name: 'self_check', enabled: true, running: false, note: 'next run in 6h' },
          { name: 'self_improve', enabled: false, running: false },
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
      if (location.runtime_id !== 'local') throw new OpenDanError('invalid_argument', 'Runtime is unavailable.')
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
      return structuredClone(fx.entries)
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
        entry: e,
        worklog: (fx.worklogs[sid] ?? []).slice(-worklog),
        children: fx.entries.filter((c) => c.origin?.parent_session === sid).map((c) => c.session_id),
        hosted: fx.hostedList.find((h) => h.session_id === sid) ?? null,
      })
    },

    stopSession: async (sid, reason) => {
      await delay()
      const e = find(sid)
      inputIndex += 1
      log(sid, { t: 'control_applied', input: { src: 'queue', index: inputIndex, kind: 'control' }, command: 'stop', detail: reason ? { reason } : undefined })
      if (e.status.run_state !== 'finished') {
        commit(e, { run_state: 'finished', outcome: 'stopped', waiting_for: undefined, turn_open: false, one_line_status: reason ? `Stopped: ${reason}` : 'Stopped' })
        const st = fx.details[sid]?.state
        if (st) Object.assign(st, { open_turn: undefined, waiting_for: undefined, stop_requested: true })
      }
      return { index: inputIndex }
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

    perception: async () => {
      await delay()
      return {
        cursor: { offsets: { [UI]: 2048, [WORK_PENDING]: 512 }, updated_at_ms: ago(1200) },
        backlog: [
          { session_id: WORK_PENDING, from_offset: 512, to_offset: 1388 },
          { session_id: WORK_FAILED, from_offset: 0, to_offset: 241 },
        ],
      }
    },

    perceptionRecords: async (item) => {
      await delay()
      return fx.perception.filter((r) => r.session_id === item.session_id)
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

    recallHints: async (tags, maxHints = 8) => {
      await delay()
      const all = [
        { id: 'h-101', time: '2026-09-28', sentence: 'Photos live under /home/devtest/Pictures/camera.', kind: 'fact', tags: ['photos'] },
        { id: 'h-102', time: '2026-09-30', sentence: 'The user prefers reports as markdown with a short summary first.', kind: 'preference', tags: ['report'] },
        { id: 'h-103', time: '2026-10-01', sentence: 'Renaming photos requires an accept decision before files are moved.', kind: 'rule', tags: ['photos', 'rename'] },
      ]
      return all
        .filter((h) => tags.length === 0 || h.tags.some((t) => tags.includes(t)))
        .slice(0, maxHints)
        .map((h) => ({ id: h.id, time: h.time, sentence: h.sentence, kind: h.kind }))
    },
  }
}
