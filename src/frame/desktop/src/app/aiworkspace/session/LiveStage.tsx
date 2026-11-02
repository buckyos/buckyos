/* The stage of a session (第四期规划 §5.1, §5.2, §6.1): the run Workspace's current step as the service confirmed it,
 * for every terminal. The presenter browses, points and writes on it — the pointer and the free camera are samples
 * on the time line, a finished stroke is an ink Block of the run Workspace (so a late joiner and the replay see it);
 * a follower's camera goes where the presenter's goes; reactions fly over it. Nothing here decides the stage. */

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { randomId } from '../api/ids'
import type { CellPayload, Operation, ShowPathPayload } from '../api/types'
import { encodeStroke } from '../ink/model'
import { fitStage, worldOf, type ResolvedStep } from '../presentation/model'
import { InkLayer, PointerLayer, type InkStroke, type MarkTool } from '../presentation/overlays'
import { StageCanvas } from '../presentation/StageCanvas'
import { useAiwsText } from '../useAiwsText'
import type { WorkspaceStore } from '../state/store'
import type { Camera } from '../ui/canvas/render/camera'
import type { SessionChannel } from './channel'
import type { RoomSnapshot } from './room'
import { stepOf } from './steps'

const POINTER_MS = 100
const CAMERA_MS = 200

export interface LiveStageProps {
  store: WorkspaceStore
  path: ShowPathPayload
  steps: ResolvedStep[]
  snap: RoomSnapshot
  channel: SessionChannel | null
  presenter: boolean
  /** An audience member browsing on its own (presenter: never). */
  free?: boolean
  onFree?: () => void
  /** Bumped to fly back to the presenter's view. */
  back?: number
  tool?: MarkTool
  inkColor?: string
  /** A click while giving position feedback: the world point of the Surface on stage. */
  onPick?: (p: { surfaceId: string; x: number; y: number }) => void
  editing?: string | null
  onEditingChange?: (id: string | null) => void
  /** The Surface on stage and its camera (feedback markers, the board). */
  onCamera?: (camera: Camera, surfaceId: string) => void
  pure?: boolean
}


export function LiveStage(props: LiveStageProps) {
  const { store, path, steps, snap, channel, presenter, free = false, back = 0, tool = 'none' } = props
  const index = stepOf(steps, snap.stage)
  const current = steps[index] ?? null
  const next = steps[index + 1] ?? null
  const rootRef = useRef<HTMLDivElement>(null)
  const [onStage, setOnStage] = useState<{ camera: Camera; surfaceId: string } | null>(null)
  const [ink, setInk] = useState<InkStroke[]>([])
  const lastPointer = useRef(0)
  const lastCamera = useRef(0)
  const stageFree = snap.stage.free === true
  // the presenter browsing: its camera is sampled while free
  useEffect(() => {
    if (!presenter || !stageFree || !onStage || !channel) return
    const { camera, surfaceId } = onStage
    return camera.onChange(() => {
      const now = performance.now()
      if (now - lastCamera.current < CAMERA_MS) return
      lastCamera.current = now
      const el = rootRef.current
      if (!el) return
      const s = fitStage({ x: 0, y: 0, w: el.clientWidth, h: el.clientHeight }, path.stage)
      const rect = worldOf(s, camera.viewport)
      void channel.call('session.sample', { kind: 'camera', data: { surface_id: surfaceId, rect: { x: rect.x, y: rect.y, w: rect.w, h: rect.h } }, t_rel_ms: channel.tRel() }).catch(() => undefined)
    })
  }, [presenter, stageFree, onStage, channel, path.stage])

  const onPointer = (p: { x: number; y: number } | null) => {
    if (!channel) return
    const now = performance.now()
    if (p && now - lastPointer.current < POINTER_MS) return
    lastPointer.current = now
    const el = rootRef.current
    let data: { x: number; y: number } | null = null
    if (p && el) {
      const s = fitStage({ x: 0, y: 0, w: el.clientWidth, h: el.clientHeight }, path.stage)
      data = { x: Math.max(0, Math.min(1, (p.x - s.x) / s.w)), y: Math.max(0, Math.min(1, (p.y - s.y) / s.h)) }
    }
    void channel.call('session.sample', { kind: 'pointer', data, t_rel_ms: channel.tRel() }).catch(() => undefined)
  }

  // a finished stroke becomes ink Blocks of the run Workspace (one commit); the local copy goes when it is in
  const onStrokes = useCallback((next: InkStroke[]) => {
    setInk(next)
    const added = next.filter((s) => !ink.some((o) => o.id === s.id))
    for (const stroke of added) {
      const segments = encodeStroke(stroke.points, { color: stroke.color, width: stroke.width, pressure: stroke.points.some((p) => p.p !== undefined) }, randomId('k'))
      let after = store.outline.childrenOf(stroke.surfaceId).at(-1)?.order_key ?? undefined
      const operations: Operation[] = segments.map((s) => {
        after = store.core.order_key_between(after, undefined)
        const payload: CellPayload = { view: { type: 'ink', version: 1 }, config: s.config as unknown as CellPayload['config'] }
        return { op: 'entity.create', entity_id: randomId('c'), type_id: 'buckyos.cell', parent_id: stroke.surfaceId, order_key: after, placement: s.rect, payload: payload as unknown as Record<string, never> }
      })
      void store.submit({ editId: `board:${stroke.id}`, label: 'board', operations, undoable: false }).then(() => setInk((list) => list.filter((s) => s.id !== stroke.id)))
    }
  }, [ink, store])

  // feedback: a click at a world point of the Surface on stage
  const pick = (event: React.PointerEvent) => {
    if (!props.onPick || !onStage) return
    const r = rootRef.current?.getBoundingClientRect()
    const p = onStage.camera.toWorld(event.clientX - (r?.left ?? 0), event.clientY - (r?.top ?? 0))
    props.onPick({ surfaceId: onStage.surfaceId, x: p.x, y: p.y })
  }

  const follow = !presenter && !free && stageFree ? snap.camera : null
  const nonce = (snap.stage.nonce ?? 0) * 1000 + back
  const pointer = presenter ? null : snap.pointer
  const { t } = useAiwsText()
  const reactions = props.pure ? [] : snap.reactions
  return (
    <div ref={rootRef} className="aiws-session-stage" data-testid="aiws-session-stage" data-step={current?.step.id} data-index={index} data-black={snap.stage.black ? 'true' : undefined}
      data-free={stageFree ? 'true' : undefined} data-following={!presenter && !free ? 'true' : undefined}>
      {current ? (
        <StageCanvas stage={path.stage} background={path.background} current={current} next={next} transition={free ? 'fly' : snap.stage.transition ?? 'fade'} nonce={nonce}
          free={presenter ? stageFree : free || stageFree} black={snap.stage.black === true} onFree={props.onFree}
          follow={follow} editing={props.editing ?? null} onEditingChange={props.onEditingChange}
          onCamera={(camera, surfaceId) => { setOnStage((prev) => (prev?.camera === camera ? prev : { camera, surfaceId })); props.onCamera?.(camera, surfaceId) }}
          underMask={presenter ? <InkLayer camera={onStage?.camera ?? null} surfaceId={onStage?.surfaceId ?? null} tool={tool} color={props.inkColor ?? '#ef4444'} width={4} strokes={ink} onStrokes={onStrokes} /> : null}
          overMask={<>
            {presenter && <PointerLayer active={tool === 'pointer'} onMove={onPointer} />}
            {pointer && <StagePointer pointer={pointer} stage={path.stage} />}
            {props.onPick && <div className="aiws-session-pick" data-testid="aiws-session-pick" onPointerDown={pick} />}
            <ReactionLayer reactions={reactions} camera={onStage?.camera ?? null} surfaceId={onStage?.surfaceId ?? null} />
          </>} />
      ) : (
        <div className="aiws-session-empty aiws-muted" data-testid="aiws-session-empty">{t('aiworkspace.session.stage.noSteps')}</div>
      )}
    </div>
  )
}

/** The presenter's pointer on a follower's stage (stage coordinates, fitted into the box it is drawn in). */
function StagePointer({ pointer, stage }: { pointer: { x: number; y: number }; stage: { w: number; h: number } }) {
  const ref = useRef<HTMLDivElement>(null)
  const [size, setSize] = useState<{ w: number; h: number } | null>(null)
  useLayoutEffect(() => {
    const box = ref.current
    if (!box) return
    const observer = new ResizeObserver(() => setSize({ w: box.clientWidth, h: box.clientHeight }))
    observer.observe(box)
    return () => observer.disconnect()
  }, [])
  const s = size ? fitStage({ x: 0, y: 0, w: size.w, h: size.h }, stage) : null
  return (
    <div ref={ref} className="aiws-session-pointer-box" aria-hidden="true">
      {s && <div className="aiws-session-pointer" data-testid="aiws-session-pointer" style={{ left: s.x + pointer.x * s.w, top: s.y + pointer.y * s.h }} />}
    </div>
  )
}

/** Reactions over the stage (§5.2): comments fly across the top band, position feedback appears where it was given. */
function ReactionLayer({ reactions, camera, surfaceId }: { reactions: RoomSnapshot['reactions']; camera: Camera | null; surfaceId: string | null }) {
  const [, setTick] = useState(0)
  useEffect(() => camera?.onChange(() => setTick((n) => n + 1)), [camera])
  return (
    <div className="aiws-session-reactions" aria-hidden="true">
      {reactions.map((r) => {
        if (r.kind === 'danmaku') {
          return <div key={r.id} className="aiws-session-danmaku" data-testid="aiws-session-danmaku" style={{ top: `${6 + (r.id % 6) * 7}%` }}>{r.name ? <b>{r.name}</b> : null}{r.text}</div>
        }
        if (camera && r.surfaceId === surfaceId && r.x !== undefined && r.y !== undefined) {
          const p = camera.toScreen(r.x, r.y)
          return <div key={r.id} className="aiws-session-feedback" data-testid="aiws-session-feedback" style={{ left: p.x, top: p.y }}>{r.symbol ?? '●'}</div>
        }
        return <div key={r.id} className="aiws-session-feedback is-corner" data-testid="aiws-session-feedback">{r.symbol ?? '●'}{r.text ? ` ${r.text}` : ''}</div>
      })}
    </div>
  )
}
