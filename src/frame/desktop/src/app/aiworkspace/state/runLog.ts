/* The wish runs under way in this window (许愿格交互升级 §3.8): what the collapsed status bar says of them and the
 * status panel's "运行中" lists, one block a run. Each block keeps a line for every change of the run's progress this
 * window sees — the run's phase, the tool the model called, the last command, the trial run, a warning. Nothing new is
 * asked of the service: the lines come from the `proc.get` polling `WishService` does for the runs it follows, so a
 * change undone between two polls is not seen. A run that ends leaves with its lines; the work log keeps its start and
 * end. The macro's own output is not followed live (Q10): it is in the run's record afterwards. */

import { useSyncExternalStore } from 'react'
import type { WishRunView, WishStage } from '../api/types'
import { aiwsT } from '../i18n'
import { Emitter } from './emitter'
import { useStore } from './hooks'
import { serverClock } from './serverClock'
import type { WorkspaceStore } from './store'

export type RunLogKind = 'phase' | 'tool' | 'command' | 'trial' | 'warning'
export interface RunLogLine {
  seq: number
  /** When this window saw it, on the service's clock (`run.created_at` is the service's). */
  at: number
  kind: RunLogKind
  value?: string
  count?: number
}
export interface RunLogRun { run: WishRunView; phase: string; lines: readonly RunLogLine[] }

const MAX_LINES = 50
/** The phases of a run's life (queued → snapshotting → running → validating → applying, `claimed` once an executor has
 * it); each change is a line. What an executor reports as its phase besides (a tool's name, waiting for the model)
 * changes with every model round and is said by the tool lines. */
const PHASES = new Set(['queued', 'claimed', 'snapshotting', 'running', 'validating', 'applying'])

/** The run's phase in its life: the one the service reports, else the run's state. */
export function runPhase(run: WishRunView): string {
  const phase = run.progress?.phase ?? ''
  return PHASES.has(phase) ? phase : run.state
}

interface Seen { phase: string; tools: number; command: string; trials: number; warning: string }
const NOTHING: Seen = { phase: '', tools: 0, command: '', trials: 0, warning: '' }

function observe(run: WishRunView): Seen {
  const p = run.progress ?? {}
  return { phase: runPhase(run), tools: p.tool_calls ?? 0, command: p.last_command ?? '', trials: p.program_runs ?? 0, warning: p.warning ?? '' }
}

/** The lines between what was seen of a run and what is seen now. A trial run is said once: the model's `run_program`
 * call and the trial it counts arrive together. */
function changes(before: Seen, now: Seen, run: WishRunView): Pick<RunLogLine, 'kind' | 'value' | 'count'>[] {
  const out: Pick<RunLogLine, 'kind' | 'value' | 'count'>[] = []
  const tool = run.progress?.last_tool
  const trial = now.trials > before.trials
  if (now.phase !== before.phase) out.push({ kind: 'phase', value: now.phase })
  if (now.tools > before.tools && !(trial && tool === 'run_program')) out.push({ kind: 'tool', value: tool, count: now.tools })
  if (now.command && now.command !== before.command) out.push({ kind: 'command', value: now.command })
  if (trial) out.push({ kind: 'trial', count: now.trials })
  if (now.warning && now.warning !== before.warning) out.push({ kind: 'warning', value: now.warning })
  return out
}

/** The active runs `WishService` follows for this window. */
function activeRuns(store: WorkspaceStore): readonly WishRunView[] {
  return store.wish.activeRuns()
}

interface Followed { run: WishRunView; seen: Seen; lines: RunLogLine[] }

export class RunLog {
  private readonly store: WorkspaceStore
  private readonly followed = new Map<string, Followed>()
  private list: readonly RunLogRun[] = []
  private seq = 0
  private version = 0
  private readonly emitter = new Emitter()
  readonly subscribe = this.emitter.subscribe
  readonly snapshot = () => this.version

  constructor(store: WorkspaceStore) {
    this.store = store
    store.wish.subscribe(this.update)
    this.update()
  }

  /** The runs under way, oldest first. */
  runs(): readonly RunLogRun[] { return this.list }

  private readonly update = () => {
    const active = activeRuns(this.store)
    // the service's clock only once there is a run (it measures itself with a few `ws.time` round trips)
    let now: number | null = null
    const ids = new Set<string>()
    let changed = false
    for (const run of active) {
      ids.add(run.run_id)
      let followed = this.followed.get(run.run_id)
      if (!followed) {
        followed = { run, seen: NOTHING, lines: [] }
        this.followed.set(run.run_id, followed)
        changed = true
      } else if (followed.run !== run) {
        changed = true
      }
      const seen = observe(run)
      const lines = changes(followed.seen, seen, run)
      if (lines.length > 0) changed = true
      for (const line of lines) followed.lines.push({ ...line, seq: ++this.seq, at: now ??= serverClock(this.store.session).now() })
      if (followed.lines.length > MAX_LINES) followed.lines.splice(0, followed.lines.length - MAX_LINES)
      followed.run = run
      followed.seen = seen
    }
    for (const id of [...this.followed.keys()]) if (!ids.has(id)) { this.followed.delete(id); changed = true }
    if (!changed) return
    this.list = [...this.followed.values()]
      .sort((a, b) => a.run.created_at.localeCompare(b.run.created_at))
      .map((f) => ({ run: f.run, phase: f.seen.phase, lines: [...f.lines] }))
    this.version += 1
    this.emitter.emit()
  }
}

const logs = new WeakMap<WorkspaceStore, RunLog>()

/** The window's run log; it follows the runs from the first time it is asked for (the status bar always is). */
export function runLog(store: WorkspaceStore): RunLog {
  let log = logs.get(store)
  if (!log) { log = new RunLog(store); logs.set(store, log) }
  return log
}

export function useRunLog(): readonly RunLogRun[] {
  const log = runLog(useStore())
  useSyncExternalStore(log.subscribe, log.snapshot)
  return log.runs()
}

// ---- in the reader's language

const STAGE: Record<WishStage, string> = { analyze: 'aiworkspace.runlog.stage.analyze', code: 'aiworkspace.runlog.stage.code', run: 'aiworkspace.runlog.stage.run' }
const PHASE: Record<string, string> = {
  queued: 'aiworkspace.runlog.phase.queued', claimed: 'aiworkspace.runlog.phase.claimed', snapshotting: 'aiworkspace.runlog.phase.snapshotting',
  running: 'aiworkspace.runlog.phase.running', validating: 'aiworkspace.runlog.phase.validating', applying: 'aiworkspace.runlog.phase.applying',
}
const ACTIVITY: Record<string, string> = {
  reading: 'aiworkspace.runlog.doing.reading', writing_program: 'aiworkspace.runlog.doing.writingProgram', shell: 'aiworkspace.runlog.doing.shell',
  writing: 'aiworkspace.runlog.doing.writing', checking: 'aiworkspace.runlog.doing.checking', submitting: 'aiworkspace.runlog.doing.submitting',
  working: 'aiworkspace.runlog.doing.working',
}

export function runStageText(stage: WishStage): string {
  return STAGE[stage] ? aiwsT(STAGE[stage]) : stage
}

function phaseText(phase: string): string {
  return PHASE[phase] ? aiwsT(PHASE[phase]) : phase
}

/** What the run is doing now: the phase around the work (queued, snapshot, validation, writing), else the step of the
 * work — waiting for the model, the macro running, the kind of tool the model called. The phase of an executor's run
 * stays `claimed` while it works. Empty when nothing more than "running" is known. */
export function runDoingText(run: WishRunView): string {
  const p = run.progress ?? {}
  const phase = runPhase(run)
  if (phase !== 'running' && phase !== 'claimed') return phaseText(phase)
  if (p.waiting_model || p.phase === 'waiting_model') return aiwsT('aiworkspace.runlog.doing.waitingModel')
  if (p.activity === 'running_program' || p.phase === 'running_program') return aiwsT(run.stage === 'code' ? 'aiworkspace.runlog.doing.trial' : 'aiworkspace.runlog.doing.program')
  const activity = ACTIVITY[p.activity ?? '']
  return activity ? aiwsT(activity) : phase === 'claimed' ? phaseText(phase) : ''
}

/** The model the run uses, when the executor said (its progress, or the usage it reported). */
export function runModel(run: WishRunView): string {
  const usage = run.usage as { model?: unknown } | null | undefined
  return run.progress?.model || (typeof usage?.model === 'string' ? usage.model : '')
}

/** A duration for people: 42 秒, 3 分 12 秒, 1 小时 5 分. */
export function elapsedText(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000))
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const s = total % 60
  if (h > 0) return aiwsT('aiworkspace.runlog.elapsed.hm', { h, m })
  if (m > 0) return aiwsT('aiworkspace.runlog.elapsed.ms', { m, s })
  return aiwsT('aiworkspace.runlog.elapsed.s', { s })
}

/** A line's offset from the start of its run, as a clock reads it (0:42, 1:05:09). */
export function lineOffset(line: RunLogLine, run: WishRunView): string {
  const total = Math.max(0, Math.floor((line.at - Date.parse(run.created_at)) / 1000))
  const h = Math.floor(total / 3600)
  const mm = Math.floor((total % 3600) / 60)
  const ss = String(total % 60).padStart(2, '0')
  return h > 0 ? `${h}:${String(mm).padStart(2, '0')}:${ss}` : `${mm}:${ss}`
}

export function describeRunLine(line: RunLogLine, run: WishRunView): string {
  switch (line.kind) {
    case 'phase': return phaseText(line.value ?? '')
    case 'tool': return line.value ? aiwsT('aiworkspace.runlog.line.tool', { tool: line.value }) : aiwsT('aiworkspace.runlog.line.toolUnnamed')
    case 'command': return aiwsT('aiworkspace.runlog.line.command', { command: line.value ?? '' })
    case 'trial': return run.stage === 'code' ? aiwsT('aiworkspace.runlog.line.trial', { count: line.count ?? 1 }) : aiwsT('aiworkspace.runlog.line.program')
    case 'warning': return aiwsT('aiworkspace.runlog.line.warning', { warning: line.value ?? '' })
  }
}
