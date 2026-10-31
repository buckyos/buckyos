/* The replay of a session (第四期规划 §7.1, §7.2, TODO I12): the run Workspace as it was at any moment, rebuilt in
 * this browser — the baseline rows (`session.replay`, cut to what the viewer may read now) loaded into the WASM
 * replica, then the run Workspace's Commits (`doc.get_changes`, their row deltas) applied up to the moment; the
 * continuous states (step, pointer, camera, media, reactions) come from the time line. The Workspace session the
 * Blocks read is this replica: nothing is written, nothing runs, no request but reads of assets leaves the page. */

import { ServiceFailure } from '../api/client'
import type { WorkspaceSession } from '../api/session'
import type { AiwsCore } from '../api/wasm'
import type { ChangesPage, CommitEvent, CommitOutcome, EntityEnvelope, Json } from '../api/types'
import type { AiwsClient } from '../api/client'
import { unwrap } from '../api/client'
import type { SessionApi } from './api'
import type { FlyingReaction, RoomSnapshot } from './room'
import type { MediaState, ReplayBundle, SessionEvent, SessionStage } from './types'

type Replica = InstanceType<AiwsCore['Replica']>

function engine<T>(run: () => T): T {
  try {
    return run()
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    try {
      const parsed = JSON.parse(message) as { code?: string; detail?: string }
      if (parsed && typeof parsed.code === 'string') throw new ServiceFailure({ code: parsed.code, detail: parsed.detail })
    } catch (inner) { if (inner instanceof ServiceFailure) throw inner }
    throw error instanceof Error ? error : new Error(message)
  }
}

export interface ReplayData {
  bundle: ReplayBundle
  changes: CommitEvent[]
  events: SessionEvent[]
}

/** Everything a replay needs, read once: the baseline, the Commits after it, the visible time line. */
export async function loadReplay(api: SessionApi, client: AiwsClient): Promise<ReplayData> {
  const bundle = await api.replay()
  const events: SessionEvent[] = []
  for (let after = 0; ;) {
    const page = await api.events(after, 2000)
    events.push(...page.events)
    if (!page.more || page.events.length === 0) break
    after = page.events[page.events.length - 1].seq
  }
  const changes: CommitEvent[] = []
  for (let after = bundle.baseline_seq; after < bundle.head_seq;) {
    const page = unwrap<ChangesPage>(await client.getChanges({ workspace_id: bundle.workspace_id }, bundle.epoch, after, 500))
    if (page.changes.length === 0) break
    changes.push(...page.changes)
    after = page.changes[page.changes.length - 1].seq
    if (!page.more) break
  }
  return { bundle, changes, events }
}

export class ReplayEngine {
  readonly data: ReplayData
  private readonly core: AiwsCore
  private readonly principal: string
  private replica: Replica
  private readonly listeners = new Set<(event: CommitEvent) => void>()
  private readonly resetListeners = new Set<() => void>()
  /** `commit_seq → t_rel_ms` of the Commits the viewer may see on the time line. */
  private readonly commitTimes: { seq: number; t: number }[]
  readonly duration: number

  constructor(core: AiwsCore, principal: string, data: ReplayData) {
    this.core = core
    this.principal = principal
    this.data = data
    this.replica = this.fresh()
    this.commitTimes = data.events.filter((e) => e.type === 'commit' && e.commit_seq).map((e) => ({ seq: e.commit_seq!, t: e.t_rel_ms })).sort((a, b) => a.seq - b.seq)
    const last = data.events.reduce((m, e) => Math.max(m, e.t_rel_ms), 0)
    this.duration = Math.max(data.bundle.duration_ms ?? 0, last)
  }

  private fresh(): Replica {
    const b = this.data.bundle
    return new this.core.Replica(this.principal, b.workspace_id, b.epoch, JSON.stringify(b.baseline), b.baseline_seq, 1)
  }

  get seq(): number { return this.replica.confirmed_seq }

  /** The last Commit at or before `t` (the content of that moment). */
  seqAt(t: number): number {
    let seq = this.data.bundle.baseline_seq
    for (const c of this.commitTimes) {
      if (c.t > t) break
      seq = c.seq
    }
    return seq
  }

  /** Move the content to `seq`: forward by applying the deltas, backward by rebuilding from the baseline. */
  seek(seq: number) {
    if (seq < this.seq) {
      this.replica.free()
      this.replica = this.fresh()
      const upto = this.data.changes.filter((c) => c.seq <= seq)
      if (upto.length) engine(() => this.replica.apply_remote(JSON.stringify(upto)))
      for (const l of [...this.resetListeners]) l()
      return
    }
    const step = this.data.changes.filter((c) => c.seq > this.seq && c.seq <= seq)
    if (step.length === 0) return
    engine(() => this.replica.apply_remote(JSON.stringify(step)))
    for (const c of step) for (const l of [...this.listeners]) l(c)
  }

  onChange(listener: (event: CommitEvent) => void) { this.listeners.add(listener); return () => { this.listeners.delete(listener) } }
  onReset(listener: () => void) { this.resetListeners.add(listener); return () => { this.resetListeners.delete(listener) } }

  outline(): EntityEnvelope[] {
    return (JSON.parse(engine(() => this.replica.outline())) as { entities: EntityEnvelope[] }).entities
  }

  read<C>(entityId: string, selector?: unknown): EntityEnvelope & { content: C; head_seq: number } {
    const r = JSON.parse(engine(() => this.replica.read(entityId, selector ? JSON.stringify(selector) : undefined))) as EntityEnvelope & { content: C; head_seq: number }
    r.head_seq = this.seq
    return r
  }

  query(params: unknown): unknown {
    return JSON.parse(engine(() => this.replica.query(JSON.stringify(params))))
  }

  /** The room at `t`: the stage, the pointer and camera of the moment, reactions of the last seconds. */
  roomAt(t: number, state: RoomSnapshot['state']): RoomSnapshot {
    let stage: SessionStage = {}
    let pointer: RoomSnapshot['pointer'] = null
    let camera: RoomSnapshot['camera'] = null
    const reactions: FlyingReaction[] = []
    let voice: 'on' | 'off' = 'off'
    for (const e of this.data.events) {
      if (e.t_rel_ms > t) break
      const d = e.data as Record<string, Json>
      if ((e.type === 'stage' || e.type === 'media.state' || e.type === 'session.start') && d.stage) stage = d.stage as unknown as SessionStage
      else if (e.type === 'sample.pointer') pointer = d && typeof d.x === 'number' && typeof d.y === 'number' && t - e.t_rel_ms < 2000 ? { x: d.x, y: d.y } : null
      else if (e.type === 'sample.camera' && typeof d.surface_id === 'string') camera = { surfaceId: d.surface_id, rect: d.rect as unknown as { x: number; y: number; w: number; h: number }, nonce: e.seq }
      else if (e.type === 'reaction' && t - e.t_rel_ms < 6000) reactions.push({ id: e.seq, kind: d.kind === 'danmaku' ? 'danmaku' : 'feedback', text: String(d.text ?? ''), symbol: typeof d.symbol === 'string' ? d.symbol : undefined,
                                                                             name: typeof d.name === 'string' ? d.name : undefined, x: typeof d.x === 'number' ? d.x : undefined, y: typeof d.y === 'number' ? d.y : undefined,
                                                                             surfaceId: typeof d.surface_id === 'string' ? d.surface_id : undefined, at: performance.now() - (t - e.t_rel_ms) })
      else if (e.type === 'voice.start') voice = 'on'
      else if (e.type === 'voice.stop') voice = 'off'
    }
    if (!stage.free) camera = null
    return { state, stage, pointer, camera, reactions, presence: {}, controller: null, voice, lastCommitSeq: this.seq }
  }

  mediaAt(t: number, entityId: string): MediaState | undefined {
    return this.roomAt(t, 'archived').stage.media?.[entityId]
  }

  /** The recorded voice (§7.3.3): Ogg Opus segments on the time line. */
  voiceSegments(): { objectId: string; start: number; end: number }[] {
    return this.data.events.filter((e) => e.type === 'voice.segment').map((e) => {
      const d = e.data as Record<string, Json>
      return { objectId: String(d.segment_id), start: Number(d.t_rel_start), end: Number(d.t_rel_end) }
    }).sort((a, b) => a.start - b.start)
  }

  /** Navigation (§7.1): the steps and the tasks on the time line. */
  marks(): { t: number; kind: 'step' | 'task'; label: string }[] {
    const out: { t: number; kind: 'step' | 'task'; label: string }[] = []
    let last: string | null = null
    for (const e of this.data.events) {
      const d = e.data as Record<string, Json>
      if ((e.type === 'stage' || e.type === 'session.start') && d.stage) {
        const s = d.stage as unknown as SessionStage
        if (s.step_id && s.step_id !== last) { last = s.step_id; out.push({ t: e.t_rel_ms, kind: 'step', label: String(s.title ?? s.step_id) }) }
      }
    }
    return out
  }

  dispose() { this.replica.free() }
}

const READ_ONLY = 'the replay shows what was recorded: nothing can be changed here'

/** The Workspace session of a replay: reads from the engine at its playhead, writes refused, the rest (assets, the
 * Workspace's info) from the run Workspace itself. */
export function replaySession(direct: WorkspaceSession, replay: ReplayEngine): WorkspaceSession {
  const refused = async (): Promise<CommitOutcome> => ({ status: 'rejected', code: 'PERMISSION_DENIED', detail: READ_ONLY, idempotency_key: '' } as unknown as CommitOutcome)
  const overrides: Partial<Record<keyof WorkspaceSession, unknown>> = {
    outline: async () => replay.outline(),
    read: async (entityId: string, selector?: unknown) => replay.read(entityId, selector),
    readMany: async (targets: { entity_id: string; selector?: unknown }[]) => targets.map((t) => {
      try { return replay.read(t.entity_id, t.selector) } catch (error) { return { error: { code: error instanceof ServiceFailure ? error.code : 'NOT_FOUND', detail: String(error) } } }
    }),
    query: async (params: unknown) => replay.query(params),
    listAnnotations: async () => [],
    freshness: async () => [],
    relations: async (entityId: string) => ({ entity_id: entityId, outgoing: [], incoming: [], hidden_incoming: 0, shown_in: [], produced: [] }),
    appliedSeq: () => replay.seq,
    whenApplied: async () => undefined,
    subscribeChanges: (listener: (event: CommitEvent) => void) => replay.onChange(listener),
    showLock: () => null,
    noteShowLock: () => undefined,
    commit: refused,
    retryUnknown: refused,
    undoCommit: refused,
    prepare: async () => { throw new ServiceFailure({ code: 'PERMISSION_DENIED', detail: READ_ONLY }) },
    getUserState: async () => ({}),
    setUserState: async () => undefined,
    lockAcquire: async () => { throw new ServiceFailure({ code: 'PERMISSION_DENIED', detail: READ_ONLY }) },
  }
  return new Proxy(direct, {
    get(target, prop, receiver) {
      if (Object.prototype.hasOwnProperty.call(overrides, prop)) return overrides[prop as keyof WorkspaceSession]
      const v = Reflect.get(target, prop, receiver) as unknown
      return typeof v === 'function' ? (v as (...args: unknown[]) => unknown).bind(target) : v
    },
  })
}
