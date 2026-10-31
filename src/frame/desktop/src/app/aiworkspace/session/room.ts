/* What a terminal knows of a running session, from the channel's timeline (第四期规划 §5.1, §6): the confirmed stage,
 * the presenter's pointer and free camera, the reactions flying over the stage, the presence the presenter sees, the
 * active control terminal. An external store for React (`subscribe` / `snapshot`). */

import type { SessionChannel } from './channel'
import type { MediaState, SessionEvent, SessionInfo, SessionStage } from './types'

export interface FlyingReaction { id: number; kind: 'feedback' | 'danmaku'; text: string; symbol?: string; name?: string; x?: number; y?: number; surfaceId?: string; at: number }

export interface RoomSnapshot {
  state: SessionInfo['state']
  stage: SessionStage
  /** The presenter's pointer in stage coordinates (0…1), null when it is off. */
  pointer: { x: number; y: number } | null
  /** The presenter's free camera: a world rect of a Surface, with a number per sample. */
  camera: { surfaceId: string; rect: { x: number; y: number; w: number; h: number }; nonce: number } | null
  reactions: FlyingReaction[]
  presence: Record<string, { name: string; online: boolean }>
  controller: string | null
  /** The last voice event (start / stop) and whether a recording is under way. */
  voice: 'on' | 'off'
  lastCommitSeq: number
}

const REACTION_MS = 6000

export class RoomModel {
  private snap: RoomSnapshot
  private readonly listeners = new Set<() => void>()
  private nextReaction = 1
  private sweeper = 0

  constructor(info: SessionInfo, controller: string | null) {
    this.snap = { state: info.state, stage: info.stage ?? {}, pointer: null, camera: null, reactions: [], presence: {}, controller, voice: 'off', lastCommitSeq: 0 }
  }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }
  snapshot = (): RoomSnapshot => this.snap

  private set(patch: Partial<RoomSnapshot>) {
    this.snap = { ...this.snap, ...patch }
    for (const l of [...this.listeners]) l()
  }

  attach(channel: SessionChannel): () => void {
    const offEvent = channel.onEvent((e) => this.apply(e))
    const offControl = channel.onControl((controller) => this.set({ controller }))
    this.sweeper = window.setInterval(() => {
      const now = performance.now()
      if (this.snap.reactions.some((r) => now - r.at > REACTION_MS)) this.set({ reactions: this.snap.reactions.filter((r) => now - r.at <= REACTION_MS) })
    }, 1000)
    return () => { offEvent(); offControl(); window.clearInterval(this.sweeper) }
  }

  /** The stage as the presenter's own command confirmed it (the event follows, the answer is faster). */
  confirm(stage: SessionStage) {
    if ((stage.nonce ?? 0) >= (this.snap.stage.nonce ?? 0)) this.set({ stage, ...(stage.free ? {} : { camera: null }) })
  }

  setState(state: SessionInfo['state']) { this.set({ state }) }

  media(entityId: string): MediaState | undefined {
    return this.snap.stage.media?.[entityId]
  }

  apply(e: SessionEvent) {
    const d = e.data as Record<string, unknown>
    switch (e.type) {
      case 'session.start':
        this.set({ state: 'live', stage: (d.stage as SessionStage) ?? this.snap.stage })
        break
      case 'session.end':
        this.set({ state: 'ended', pointer: null })
        break
      case 'stage':
      case 'media.state': {
        const stage = d.stage as SessionStage | undefined
        if (stage) this.set({ stage, ...(stage.free ? {} : { camera: null }) })
        break
      }
      case 'sample.pointer':
        this.set({ pointer: d && typeof d.x === 'number' && typeof d.y === 'number' ? { x: d.x, y: d.y } : null })
        break
      case 'sample.camera': {
        const rect = d.rect as { x: number; y: number; w: number; h: number } | undefined
        if (typeof d.surface_id === 'string' && rect) this.set({ camera: { surfaceId: d.surface_id, rect, nonce: (this.snap.camera?.nonce ?? 0) + 1 } })
        break
      }
      case 'reaction': {
        const r: FlyingReaction = { id: this.nextReaction++, kind: d.kind === 'danmaku' ? 'danmaku' : 'feedback', text: String(d.text ?? ''), symbol: typeof d.symbol === 'string' ? d.symbol : undefined,
                                    name: typeof d.name === 'string' ? d.name : undefined, x: typeof d.x === 'number' ? d.x : undefined, y: typeof d.y === 'number' ? d.y : undefined,
                                    surfaceId: typeof d.surface_id === 'string' ? d.surface_id : undefined, at: performance.now() }
        this.set({ reactions: [...this.snap.reactions.slice(-60), r] })
        break
      }
      case 'presence.join':
      case 'presence.leave': {
        const who = String(d.principal ?? e.actor ?? '')
        if (who) this.set({ presence: { ...this.snap.presence, [who]: { name: String(d.name ?? who), online: e.type === 'presence.join' } } })
        break
      }
      case 'control':
        this.set({ controller: (d.controller as string | null) ?? null })
        break
      case 'voice.start':
        this.set({ voice: 'on' })
        break
      case 'voice.stop':
        this.set({ voice: 'off' })
        break
      case 'commit':
        this.set({ lastCommitSeq: Math.max(this.snap.lastCommitSeq, e.commit_seq ?? 0) })
        break
    }
  }
}
