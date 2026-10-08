/* Path editing, the canvas sub-mode "路径编辑" (第三期规划 §7.1): the paths of the workspace, the steps of one
 * path, and the properties of a step and of its target — on the left; on the canvas, every step of this Surface as the
 * rectangle a show will see, linked in order. Selecting a step flies the camera to it (that is the preview). A
 * Viewport's rectangle is moved and scaled right on the canvas, a Frame's moved and resized at the stage's aspect.
 * This mode creates and adjusts Frames and Viewports and edits paths, nothing else; every action is one commit and
 * one undo step, and every edit of the steps replays itself on a newer version (pathOps). */

import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import { ArrowDown, ArrowUp, Crosshair, Frame as FrameIcon, Play, Plus, Power, Trash2 } from 'lucide-react'
import { describeError } from '../api/session'
import type { EntityEnvelope, Json, KeyedContent, Operation, PresentationProps, ShowPathPayload, ShowPurpose, ShowStep, StageSize, StepTransition, ViewportPayload } from '../api/types'
import { useLoad, useOutlineVersion, useStore, useUserState, useVersion } from '../state/hooks'
import type { WorkspaceStore } from '../state/store'
import type { Laid } from '../ui/canvas/layout'
import type { Camera } from '../ui/canvas/render/camera'
import { useShell } from '../ui/shell/shellContext'
import {
  clampZoom, END, frameAtRatio, pathsOf, resolveSteps, sameRatio, STAGE_PRESETS, STEP_PROBLEM_TEXT, viewportOf, viewportRect, worldRectOf,
  type Rect, type ResolvedStep, type StepCommand,
} from './model'
import { commitSteps, createFrameOp, createPathOp, createViewportOp, fitFrameOps, newStep, okOutcome, readPath, setPathKeys, setStage, setTargetText } from './pathOps'
import './presentation.css'

const TRANSITIONS: { value: StepTransition; label: string }[] = [
  { value: 'auto', label: '自动' }, { value: 'fly', label: '飞行' }, { value: 'fade', label: '淡入淡出' }, { value: 'cut', label: '直接切换' },
]
/** The viewfinder of "添加当前视角" takes this share of the clear area. */
const FINDER_SHARE = 0.72
const HANDLE = 6

function useCameraVersion(camera: Camera): number {
  const [version, setVersion] = useState(0)
  useEffect(() => camera.onChange(() => setVersion((n) => n + 1)), [camera])
  return version
}

/** One path's payload, re-read when it changes. */
function usePathPayload(store: WorkspaceStore, pathId: string | null) {
  const version = useVersion(`e:${pathId ?? ''}`)
  const load = useCallback(() => (pathId ? readPath(store, pathId) : Promise.resolve(null)), [store, pathId])
  const loaded = useLoad(load, `${pathId}:${version}`).data ?? null
  return { version, path: loaded && pathId ? loaded.payload : null }
}

/** Every path's payload (for "a Frame shared by paths of different aspects"). */
function useAllPaths(store: WorkspaceStore, ids: string[]) {
  const any = useVersion('any')
  const key = ids.join(',')
  const load = useCallback(async () => {
    if (!key) return new Map<string, ShowPathPayload>()
    const list = key.split(',')
    const read = await store.session.readMany(list.map((entity_id) => ({ entity_id })))
    const out = new Map<string, ShowPathPayload>()
    read.forEach((r, i) => { if (!('error' in r)) out.set(list[i], (r.content as KeyedContent<ShowPathPayload>).payload) })
    return out
  }, [store, key])
  return useLoad(load, any).data ?? new Map<string, ShowPathPayload>()
}

export function PathEditor({ surface, camera, laid }: { surface: EntityEnvelope; camera: Camera; laid: Map<string, Laid> }) {
  const store = useStore()
  const shell = useShell()
  const outlineVersion = useOutlineVersion()
  const paths = pathsOf(store.outline)
  const remembered = useUserState<string>('ui:show-path')
  const pathId = paths.find((p) => p.entity_id === remembered)?.entity_id ?? paths[0]?.entity_id ?? null
  const selectPath = (id: string | null) => store.userState.set('ui:show-path', id)
  const pathEntity = pathId ? store.outline.get(pathId) : undefined
  const { version: pathVersion, path } = usePathPayload(store, pathId)
  const allPaths = useAllPaths(store, paths.map((p) => p.entity_id))
  const resolved = useMemo(() => (path ? resolveSteps(store.outline, path) : []),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- outlineVersion is the invalidation signal
    [path, store, outlineVersion])
  const [selectedStep, setSelectedStep] = useState<string | null>(null)
  const selected = resolved.find((s) => s.step.id === selectedStep) ?? null
  const [finder, setFinder] = useState(false)
  const [busy, setBusy] = useState<string | null>(null)
  const [pendingStage, setPendingStage] = useState<StageSize | null>(null)
  const [dropAt, setDropAt] = useState<string | null>(null)
  const canWrite = store.session.info().capabilities.includes('structure') && !shell.phone
  const canEditPath = Boolean(pathEntity?.capabilities.includes('update')) && !shell.phone

  /** Fly to a step (the preview); on another Surface, go there first. */
  const showStep = useCallback((s: ResolvedStep) => {
    setSelectedStep(s.step.id)
    if (!s.rect || !s.surfaceId) return
    if (s.surfaceId !== surface.entity_id) { shell.selectSurface(s.surfaceId); return }
    camera.animateTo(camera.fitted(s.rect, 48), 420)
  }, [camera, shell, surface.entity_id])
  // arriving on this Surface for a selected step (picked on another one): show it
  const arrived = useRef(false)
  useEffect(() => {
    if (arrived.current || !selected?.rect || selected.surfaceId !== surface.entity_id) return
    arrived.current = true
    camera.fit(selected.rect, 48)
  }, [selected, surface.entity_id, camera])

  const run = async (label: string, work: () => Promise<unknown>) => {
    setBusy(label)
    try { await work() } catch (error) { store.notify('error', `${label}失败：${describeError(error)}`) } finally { setBusy(null) }
  }
  const steps = (commands: StepCommand[], label: string, extra?: (next: ShowStep[], p: ShowPathPayload) => Operation[]) => {
    if (!pathId) return Promise.resolve(null)
    return commitSteps(store, pathId, commands, label, extra)
  }
  const afterSelected = () => (selected ? selected.step.id : END)

  // ---- creating paths and steps
  const newPath = (purpose: ShowPurpose) => run('新建路径', async () => {
    const n = pathsOf(store.outline, purpose).length + 1
    const { op, id } = createPathOp(store, purpose === 'guide' ? `使用引导 ${n}` : `演讲路径 ${n}`, purpose)
    const outcome = await store.submit({ editId: `path:new:${id}`, label: purpose === 'guide' ? '新建使用引导' : '新建演讲路径', operations: [op] })
    if (okOutcome(outcome)) { selectPath(id); setSelectedStep(null) }
  })
  const finderRect = (): Rect | null => {
    if (!path) return null
    const area = camera.clearArea
    const k = Math.min((area.w * FINDER_SHARE) / path.stage.w, (area.h * FINDER_SHARE) / path.stage.h)
    const w = path.stage.w * k
    const h = path.stage.h * k
    return { x: area.x + (area.w - w) / 2, y: area.y + (area.h - h) / 2, w, h }
  }
  const addCurrentView = () => run('添加当前视角', async () => {
    const box = finderRect()
    if (!box || !path || !pathId) return
    const world = { ...camera.toWorld(box.x, box.y), w: box.w / camera.zoom, h: box.h / camera.zoom }
    const view = viewportOf(world, path.stage)
    const n = store.outline.childrenOf('shows').filter((e) => e.type_id === 'buckyos.viewport').length + 1
    const { op, id } = createViewportOp(store, surface.entity_id, view, `视角 ${n}`)
    const step = newStep('viewport', id)
    const outcome = await steps([{ kind: 'insert', after: afterSelected(), steps: [step] }], '添加当前视角', () => [op])
    if (outcome && okOutcome(outcome)) { setSelectedStep(step.id); setFinder(false) }
  })
  const addFrame = (frameId: string) => run('添加 Frame', async () => {
    if (!path) return
    const step = newStep('frame', frameId)
    // a Frame joining the path takes the stage's aspect (centre and width kept), in the same commit (§4.1)
    const outcome = await steps([{ kind: 'insert', after: afterSelected(), steps: [step] }], '添加 Frame', (_next, p) => fitFrameOps(store, frameId, p.stage))
    if (outcome && okOutcome(outcome)) setSelectedStep(step.id)
  })
  const newFrame = () => run('新建 Frame 并加入', async () => {
    if (!path) return
    const area = camera.clearArea
    const c = camera.toWorld(area.x + area.w / 2, area.y + area.h / 2)
    const w = Math.round((area.w * 0.6) / camera.zoom)
    const h = Math.round((w * path.stage.h) / path.stage.w)
    const n = [...laid.values()].filter((l) => l.entity.view_type === 'frame').length + 1
    const { op, id } = createFrameOp(store, surface.entity_id, { x: c.x - w / 2, y: c.y - h / 2, w, h }, `第 ${n} 页`)
    const step = newStep('frame', id)
    const outcome = await steps([{ kind: 'insert', after: afterSelected(), steps: [step] }], '新建 Frame 并加入', () => [op])
    if (outcome && okOutcome(outcome)) setSelectedStep(step.id)
  })

  // ---- step list commands
  const move = (id: string, delta: -1 | 1) => {
    const i = resolved.findIndex((s) => s.step.id === id)
    const j = i + delta
    if (i < 0 || j < 0 || j >= resolved.length) return
    const after = delta < 0 ? (j === 0 ? null : resolved[j - 1].step.id) : resolved[j].step.id
    void run('调整步骤顺序', () => steps([{ kind: 'move', id, after }], '调整步骤顺序'))
  }
  const dropOn = (dragged: string, before: string | null) => {
    setDropAt(null)
    if (dragged === before) return
    const order = resolved.map((s) => s.step.id).filter((id) => id !== dragged)
    const at = before === null ? order.length : order.indexOf(before)
    const after = at <= 0 ? null : order[at - 1]
    void run('调整步骤顺序', () => steps([{ kind: 'move', id: dragged, after }], '调整步骤顺序'))
  }
  const removeStep = (id: string) => run('移除步骤', async () => {
    const outcome = await steps([{ kind: 'remove', id }], '移除步骤')
    if (outcome && okOutcome(outcome) && selectedStep === id) setSelectedStep(null)
  })
  const updateStep = (id: string, patch: Partial<Omit<ShowStep, 'id'>>, label: string) => run(label, () => steps([{ kind: 'update', id, patch }], label))

  // ---- geometry edits on the canvas (one commit per gesture)
  const commitViewport = (s: ResolvedStep, rect: Rect) => run('调整 Viewport', async () => {
    if (!path) return
    const read = await store.session.read<KeyedContent<ViewportPayload>>(s.targetId)
    const view = viewportOf(rect, path.stage)
    const revs = read.content.key_revs ?? {}
    await store.submit({ editId: `viewport:${s.targetId}`, label: `调整 ${s.title}`, operations: [{ op: 'entity.set_keys', entity_id: s.targetId, keys: [
      { key: 'center', value: view.center, expect: { rev: revs.center ?? 0 } }, { key: 'zoom', value: view.zoom, expect: { rev: revs.zoom ?? 0 } },
    ] }] })
  })
  const commitFrame = (s: ResolvedStep, rect: Rect) => run('调整 Frame', async () => {
    const frame = store.outline.get(s.targetId)
    const placed = worldRectOf(store.outline, s.targetId)
    if (!frame?.placement || !placed) return
    const placement = { ...frame.placement, x: Math.round(frame.placement.x + rect.x - placed.rect.x), y: Math.round(frame.placement.y + rect.y - placed.rect.y), w: Math.max(1, Math.round(rect.w)), h: Math.max(1, Math.round(rect.h)) }
    store.noteLayoutIntent(s.targetId, placement)
    store.outline.patchLocal(s.targetId, { placement })
    await store.submit({ editId: `layout:${s.targetId}`, label: `调整 ${s.title}`, operations: [{ op: 'tree.place', entity_id: s.targetId, placement }] })
  })

  const framesHere = [...laid.values()].filter((l) => l.entity.view_type === 'frame' && !l.isGroup)
  const stagePreset = path ? STAGE_PRESETS.find((p) => p.stage.w === path.stage.w && p.stage.h === path.stage.h)?.id ?? 'custom' : '16:9'
  // a Frame used by paths of different aspects can be right for one of them only
  const sharedMismatch = (frameId: string) => {
    if (!path) return false
    for (const [id, other] of allPaths) if (id !== pathId && other.steps.some((s) => s.target.entity_id === frameId) && !sameRatio(other.stage, path.stage)) return true
    return false
  }
  const problems = resolved.filter((s) => s.problem && s.problem !== 'rotated').length
  return (
    <>
      <div className="aiws-path-editor aiws-panel" data-testid="aiws-path-editor" onPointerDown={(event) => event.stopPropagation()}>
        <h4>演讲路径</h4>
        <div className="aiws-path-row">
          <select aria-label="选择路径" data-testid="aiws-path-select" value={pathId ?? ''} onChange={(event) => { selectPath(event.target.value || null); setSelectedStep(null) }}>
            {paths.length === 0 && <option value="">（还没有路径）</option>}
            {paths.map((p) => <option key={p.entity_id} value={p.entity_id}>{p.title ?? p.name ?? p.entity_id}{p.show_path?.purpose === 'guide' ? '（使用引导）' : ''}</option>)}
          </select>
        </div>
        <div className="aiws-path-row">
          <button type="button" disabled={!canWrite || busy !== null} data-testid="aiws-path-new" onClick={() => { void newPath('presentation') }}><Plus size={14} aria-hidden="true" /> 演讲路径</button>
          <button type="button" disabled={!canWrite || busy !== null} data-testid="aiws-path-new-guide" onClick={() => { void newPath('guide') }}><Plus size={14} aria-hidden="true" /> 使用引导</button>
        </div>
        {path && pathEntity && (
          <PathProps key={`${pathId}:${pathEntity.content_rev}`} pathId={pathId!} path={path} canEdit={canEditPath} stagePreset={stagePreset}
            onStage={(stage) => (resolved.some((s) => s.kind === 'frame') ? setPendingStage(stage) : void run('舞台尺寸', () => setStage(store, pathId!, stage)))}
            onDelete={() => run('删除路径', async () => {
              if (!window.confirm(`删除路径「${path.title ?? ''}」？步骤引用的 Frame 和 Viewport 不会被删除。`)) return
              const outcome = await store.submit({ editId: `path:delete:${pathId}`, label: '删除演讲路径', operations: [{ op: 'entity.delete', entity_id: pathId!, expect: { rev: pathEntity.life_rev } }] })
              if (okOutcome(outcome)) selectPath(null)
            })} />
        )}
        {pendingStage && path && (
          <div className="aiws-warning" data-testid="aiws-path-stage-confirm">
            舞台改为 {pendingStage.w} × {pendingStage.h}：路径中的 {new Set(resolved.filter((s) => s.kind === 'frame').map((s) => s.targetId)).size} 个 Frame 会改成新比例（保持中心和宽度），一次提交，可以撤销。
            <div className="aiws-dialog-actions">
              <button type="button" className="is-primary" data-testid="aiws-path-stage-apply" onClick={() => { const stage = pendingStage; setPendingStage(null); void run('舞台尺寸', () => setStage(store, pathId!, stage)) }}>调整</button>
              <button type="button" onClick={() => setPendingStage(null)}>取消</button>
            </div>
          </div>
        )}
        {path && (
          <>
            <h4>步骤{problems > 0 ? `（${problems} 个失效项，放映时跳过）` : ''}</h4>
            <div className="aiws-path-steps" data-testid="aiws-path-steps" role="listbox" aria-label="步骤"
              onDragOver={(event) => { event.preventDefault(); setDropAt('$end') }} onDrop={(event) => { const id = event.dataTransfer.getData('text/x-aiws-step'); if (id) dropOn(id, null) }}>
              {resolved.length === 0 && <div className="aiws-muted">用下面的按钮添加 Frame 或当前视角。</div>}
              {resolved.map((s, i) => (
                <button key={s.step.id} type="button" role="option" aria-selected={s.step.id === selectedStep} draggable={canEditPath}
                  className={`aiws-path-step${s.step.id === selectedStep ? ' is-selected' : ''}${s.step.enabled === false ? ' is-off' : ''}${dropAt === s.step.id ? ' is-drop' : ''}`}
                  data-testid={`aiws-path-step-${s.step.id}`} data-problem={s.problem ?? undefined}
                  onClick={() => showStep(s)}
                  onKeyDown={(event) => {
                    if (event.altKey && (event.key === 'ArrowUp' || event.key === 'ArrowDown')) { event.preventDefault(); move(s.step.id, event.key === 'ArrowUp' ? -1 : 1) }
                    else if (event.key === 'Delete' && canEditPath) { event.preventDefault(); void removeStep(s.step.id) }
                  }}
                  onDragStart={(event) => { event.dataTransfer.setData('text/x-aiws-step', s.step.id); event.dataTransfer.effectAllowed = 'move' }}
                  onDragOver={(event) => { event.preventDefault(); event.stopPropagation(); setDropAt(s.step.id) }}
                  onDrop={(event) => { event.preventDefault(); event.stopPropagation(); const id = event.dataTransfer.getData('text/x-aiws-step'); if (id) dropOn(id, s.step.id) }}
                  onDragEnd={() => setDropAt(null)}>
                  <span className="aiws-path-step-n">{i + 1}</span>
                  <span className="aiws-path-step-title">{s.title}</span>
                  {s.surfaceId && s.surfaceId !== surface.entity_id && <span className="aiws-path-badge" title="在另一张画布上">{store.outline.get(s.surfaceId)?.title ?? '其他画布'}</span>}
                  <span className="aiws-path-badge">{s.kind === 'frame' ? 'Frame' : 'Viewport'}</span>
                  {s.problem && <span className="aiws-path-badge is-problem" title={STEP_PROBLEM_TEXT[s.problem]}>{s.problem === 'rotated' ? '已旋转' : '失效'}</span>}
                  {!s.problem && s.ratioMismatch && <span className="aiws-path-badge is-problem" title="Frame 比例与舞台不一致：放映时补边">比例</span>}
                </button>
              ))}
            </div>
            {selected && canEditPath && (
              <div className="aiws-path-row">
                <button type="button" className="aiws-icon" aria-label="上移" title="上移（Alt+↑）" disabled={busy !== null} onClick={() => move(selected.step.id, -1)}><ArrowUp size={14} /></button>
                <button type="button" className="aiws-icon" aria-label="下移" title="下移（Alt+↓）" disabled={busy !== null} onClick={() => move(selected.step.id, 1)}><ArrowDown size={14} /></button>
                <button type="button" className="aiws-icon" aria-label={selected.step.enabled === false ? '启用' : '停用'} title={selected.step.enabled === false ? '启用这一步' : '停用（放映时跳过，不删除对象）'} data-testid="aiws-path-step-toggle"
                  onClick={() => { void updateStep(selected.step.id, { enabled: selected.step.enabled === false ? undefined : false }, selected.step.enabled === false ? '启用步骤' : '停用步骤') }}><Power size={14} /></button>
                <button type="button" className="aiws-icon" aria-label="移除步骤" title="移除这一步（Delete；不删除 Frame 或 Viewport）" data-testid="aiws-path-step-remove" onClick={() => { void removeStep(selected.step.id) }}><Trash2 size={14} /></button>
              </div>
            )}
            {canEditPath && (
              <div className="aiws-path-row" style={{ flexWrap: 'wrap' }}>
                <button type="button" disabled={busy !== null} data-testid="aiws-path-add-view" aria-pressed={finder} onClick={() => setFinder(!finder)}><Crosshair size={14} aria-hidden="true" /> 添加当前视角</button>
                <select aria-label="添加已有 Frame" data-testid="aiws-path-add-frame" value="" disabled={busy !== null || framesHere.length === 0} onChange={(event) => { if (event.target.value) void addFrame(event.target.value) }}>
                  <option value="">添加 Frame…</option>
                  {framesHere.map((l) => <option key={l.entity.entity_id} value={l.entity.entity_id}>{l.entity.title ?? '框'}{resolved.some((s) => s.targetId === l.entity.entity_id) ? '（已在路径中）' : ''}</option>)}
                </select>
                <button type="button" disabled={busy !== null || !surface.capabilities.includes('structure')} data-testid="aiws-path-new-frame" onClick={() => { void newFrame() }}><FrameIcon size={14} aria-hidden="true" /> 新建 Frame 并加入</button>
              </div>
            )}
            {selected && (
              <StepProps key={`${selected.step.id}:${pathVersion}`} step={selected} canEdit={canEditPath} laid={laid} surfaceId={surface.entity_id} stage={path.stage}
                shared={selected.kind === 'frame' && sharedMismatch(selected.targetId)}
                onUpdate={(patch, label) => { void updateStep(selected.step.id, patch, label) }}
                onRetarget={(id) => { void updateStep(selected.step.id, { target: { kind: selected.kind, entity_id: id } }, '重新绑定步骤') }} />
            )}
            {pathEntity && (
              <div className="aiws-dialog-actions">
                {path.purpose === 'presentation'
                  ? <button type="button" className="is-primary" data-testid="aiws-path-play" onClick={() => shell.startShow({ pathId: pathId!, stepId: selected?.step.id ?? null })}><Play size={14} aria-hidden="true" /> {selected ? '从这一步放映' : '开始放映'}</button>
                  : <button type="button" className="is-primary" data-testid="aiws-path-guide" onClick={() => shell.setGuide({ pathId: pathId!, index: Math.max(0, resolved.filter((s) => s.step.enabled !== false && s.rect).findIndex((s) => s.step.id === selectedStep)) })}><Play size={14} aria-hidden="true" /> 预览引导</button>}
              </div>
            )}
          </>
        )}
        {busy && <div className="aiws-muted" role="status">{busy}…</div>}
      </div>
      <PathOverlay camera={camera} steps={resolved} surfaceId={surface.entity_id} selected={selectedStep} canEdit={canEditPath && surface.capabilities.includes('structure')}
        stage={path?.stage ?? null} onCommitViewport={(s, r) => { void commitViewport(s, r) }} onCommitFrame={(s, r) => { void commitFrame(s, r) }} />
      {finder && path && <Viewfinder camera={camera} rect={finderRect} onConfirm={() => { void addCurrentView() }} onCancel={() => setFinder(false)} busy={busy !== null} />}
    </>
  )
}

function PathProps({ pathId, path, canEdit, stagePreset, onStage, onDelete }: { pathId: string; path: ShowPathPayload; canEdit: boolean; stagePreset: string; onStage: (stage: StageSize) => void; onDelete: () => void }) {
  const store = useStore()
  const [title, setTitle] = useState(path.title ?? '')
  const [custom, setCustom] = useState({ w: String(path.stage.w), h: String(path.stage.h) })
  const save = (values: Partial<Omit<ShowPathPayload, 'steps' | 'stage'>>, label: string) => { void setPathKeys(store, pathId, values, label).catch((error: unknown) => store.notify('error', `${label}失败：${describeError(error)}`)) }
  return (
    <div className="aiws-path-props" data-testid="aiws-path-props">
      <label>名称<input aria-label="路径名称" data-testid="aiws-path-title" value={title} disabled={!canEdit} onChange={(event) => setTitle(event.target.value)} onBlur={() => { if (title.trim() !== (path.title ?? '')) save({ title: title.trim() }, '路径名称') }} /></label>
      <label>用途
        <select aria-label="用途" data-testid="aiws-path-purpose" value={path.purpose} disabled={!canEdit} onChange={(event) => save({ purpose: event.target.value as ShowPurpose }, '路径用途')}>
          <option value="presentation">演讲（开始放映）</option><option value="guide">使用引导</option>
        </select>
      </label>
      <label>舞台尺寸
        <select aria-label="舞台尺寸" data-testid="aiws-path-stage" value={stagePreset} disabled={!canEdit} onChange={(event) => { const preset = STAGE_PRESETS.find((p) => p.id === event.target.value); if (preset) onStage(preset.stage) }}>
          {STAGE_PRESETS.map((p) => <option key={p.id} value={p.id}>{p.label}</option>)}
          <option value="custom" disabled>自定义（{path.stage.w} × {path.stage.h}）</option>
        </select>
      </label>
      <div className="aiws-path-row">
        <input aria-label="舞台宽" inputMode="numeric" value={custom.w} disabled={!canEdit} onChange={(event) => setCustom({ ...custom, w: event.target.value })} />
        <span>×</span>
        <input aria-label="舞台高" inputMode="numeric" value={custom.h} disabled={!canEdit} onChange={(event) => setCustom({ ...custom, h: event.target.value })} />
        <button type="button" disabled={!canEdit} onClick={() => {
          const w = Math.round(Number(custom.w)), h = Math.round(Number(custom.h))
          if (w >= 16 && h >= 16 && w <= 16384 && h <= 16384 && (w !== path.stage.w || h !== path.stage.h)) onStage({ w, h })
        }}>应用</button>
      </div>
      <label>背景（补边与 Viewport 步骤）
        <span className="aiws-path-row">
          <input type="color" aria-label="路径背景" value={path.background ?? '#ffffff'} disabled={!canEdit} onChange={(event) => save({ background: event.target.value }, '路径背景')} />
          <span className="aiws-grow" />
          <button type="button" className="aiws-link" disabled={!canEdit} data-testid="aiws-path-delete" onClick={onDelete}>删除路径</button>
        </span>
      </label>
    </div>
  )
}

/** A step and its target: step title, transition, enabled, hidden Blocks; target title, notes, caption, a Frame's page background. */
function StepProps({ step, canEdit, laid, surfaceId, stage, shared, onUpdate, onRetarget }: {
  step: ResolvedStep; canEdit: boolean; laid: Map<string, Laid>; surfaceId: string; stage: StageSize; shared: boolean
  onUpdate: (patch: Partial<Omit<ShowStep, 'id'>>, label: string) => void
  onRetarget: (id: string) => void
}) {
  const store = useStore()
  const targetVersion = useVersion(`e:${step.targetId}`)
  const load = useCallback(() => (step.target ? store.session.read<KeyedContent<Record<string, Json>>>(step.targetId) : Promise.resolve(null)), [store, step.target, step.targetId])
  const read = useLoad(load, targetVersion).data
  const payload = read?.content.payload ?? null
  const props: PresentationProps & { title?: string } = step.kind === 'viewport'
    ? { title: payload?.title as string | undefined, notes: payload?.notes as string | undefined, caption: payload?.caption as string | undefined }
    : { title: payload?.title as string | undefined, ...((payload?.presentation as PresentationProps | undefined) ?? {}) }
  const [stepTitle, setStepTitle] = useState(step.step.title ?? '')
  const [notes, setNotes] = useState<string | null>(null)
  const [caption, setCaption] = useState<string | null>(null)
  const [targetTitle, setTargetTitle] = useState<string | null>(null)
  const editTarget = canEdit && Boolean(step.target?.capabilities.includes('update'))
  const text = (key: 'notes' | 'caption' | 'title' | 'background', value: string) => { void setTargetText(store, step.targetId, key, value).catch((error: unknown) => store.notify('error', `没有保存：${describeError(error)}`)) }
  const hidden = new Set(step.step.hide ?? [])
  const inside = step.rect && step.surfaceId === surfaceId
    ? [...laid.values()].filter((l) => !l.isGroup && l.entity.view_type !== 'frame' && l.entity.entity_id !== step.targetId && l.bounds.x < step.rect!.x + step.rect!.w && l.bounds.x + l.bounds.w > step.rect!.x && l.bounds.y < step.rect!.y + step.rect!.h && l.bounds.y + l.bounds.h > step.rect!.y)
    : []
  const viewports = store.outline.childrenOf('shows').filter((e) => e.type_id === 'buckyos.viewport')
  const frames = [...laid.values()].filter((l) => l.entity.view_type === 'frame')
  return (
    <div className="aiws-path-props" data-testid="aiws-step-props">
      {step.problem && (
        <div className="aiws-warning" data-testid="aiws-step-problem">
          {STEP_PROBLEM_TEXT[step.problem]}。{step.problem === 'rotated' ? '' : '放映时跳过；可以重新绑定或移除。'}
          {step.problem !== 'rotated' && canEdit && (
            <select aria-label="重新绑定" value="" onChange={(event) => { if (event.target.value) onRetarget(event.target.value) }}>
              <option value="">重新绑定到…</option>
              {(step.kind === 'viewport' ? viewports.map((v) => ({ id: v.entity_id, label: v.title ?? v.entity_id })) : frames.map((l) => ({ id: l.entity.entity_id, label: l.entity.title ?? '框' }))).map((o) => <option key={o.id} value={o.id}>{o.label}</option>)}
            </select>
          )}
          {step.problem === 'rotated' && editTarget && <button type="button" className="aiws-link" onClick={() => {
            const frame = store.outline.get(step.targetId)
            if (!frame?.placement) return
            const { rotation: _rotation, ...upright } = frame.placement
            void _rotation
            void store.submit({ editId: `layout:${step.targetId}`, label: '转正 Frame', operations: [{ op: 'tree.place', entity_id: step.targetId, placement: upright }] })
          }}>转正</button>}
        </div>
      )}
      {step.ratioMismatch && !step.problem && (
        <div className="aiws-warning">Frame 比例与舞台（{stage.w} × {stage.h}）不一致，放映时会补边。
          {editTarget && <button type="button" className="aiws-link" data-testid="aiws-step-fit" onClick={() => {
            const ops = fitFrameOps(store, step.targetId, stage)
            if (ops.length) void store.submit({ editId: `layout:${step.targetId}`, label: '调整为舞台比例', operations: ops })
          }}>调整为舞台比例</button>}
        </div>
      )}
      {shared && <div className="aiws-warning">这个 Frame 也在舞台比例不同的路径中，只能适配其中一种比例。</div>}
      <label>步骤标题<input aria-label="步骤标题" data-testid="aiws-step-title" placeholder={step.target?.title ?? '（使用目标标题）'} value={stepTitle} disabled={!canEdit}
        onChange={(event) => setStepTitle(event.target.value)} onBlur={() => { if (stepTitle.trim() !== (step.step.title ?? '')) onUpdate({ title: stepTitle.trim() || undefined }, '步骤标题') }} /></label>
      <label>进入这一步的转场
        <select aria-label="转场" data-testid="aiws-step-transition" value={step.step.transition ?? 'auto'} disabled={!canEdit} onChange={(event) => onUpdate({ transition: event.target.value === 'auto' ? undefined : event.target.value as StepTransition }, '转场')}>
          {TRANSITIONS.map((t) => <option key={t.value} value={t.value}>{t.label}</option>)}
        </select>
      </label>
      {inside.length > 0 && (
        <label>放映时隐藏
          <span className="aiws-path-hide" data-testid="aiws-step-hide">
            {inside.map((l) => (
              <label key={l.entity.entity_id}><input type="checkbox" checked={hidden.has(l.entity.entity_id)} disabled={!canEdit} onChange={(event) => {
                const next = new Set(hidden)
                if (event.target.checked) next.add(l.entity.entity_id)
                else next.delete(l.entity.entity_id)
                onUpdate({ hide: next.size ? [...next] : undefined }, '放映时隐藏')
              }} />{l.entity.title ?? l.entity.view_type ?? l.entity.entity_id}</label>
            ))}
          </span>
        </label>
      )}
      {step.target && (
        <>
          <label>{step.kind === 'frame' ? 'Frame 标题' : 'Viewport 名称'}<input aria-label="目标标题" data-testid="aiws-target-title" value={targetTitle ?? props.title ?? ''} disabled={!editTarget}
            onChange={(event) => setTargetTitle(event.target.value)} onBlur={() => { if (targetTitle !== null && targetTitle !== (props.title ?? '')) text('title', targetTitle); setTargetTitle(null) }} /></label>
          <label>讲解备注（只在提示器中显示）<textarea aria-label="讲解备注" data-testid="aiws-target-notes" value={notes ?? props.notes ?? ''} disabled={!editTarget}
            onChange={(event) => setNotes(event.target.value)} onBlur={() => { if (notes !== null && notes !== (props.notes ?? '')) text('notes', notes); setNotes(null) }} /></label>
          <label>说明（使用引导的气泡；公开内容）<textarea aria-label="说明" data-testid="aiws-target-caption" value={caption ?? props.caption ?? ''} disabled={!editTarget}
            onChange={(event) => setCaption(event.target.value)} onBlur={() => { if (caption !== null && caption !== (props.caption ?? '')) text('caption', caption); setCaption(null) }} /></label>
          {step.kind === 'frame' && (
            <label>这一页的背景
              <span className="aiws-path-row">
                <input type="color" aria-label="页面背景" value={props.background ?? '#ffffff'} disabled={!editTarget} onChange={(event) => text('background', event.target.value)} />
                {props.background && <button type="button" className="aiws-link" disabled={!editTarget} onClick={() => text('background', '')}>使用路径背景</button>}
              </span>
            </label>
          )}
        </>
      )}
    </div>
  )
}

/** Steps of this Surface drawn over the canvas: rectangles, numbers, the order; the selected one can be moved and scaled. */
function PathOverlay({ camera, steps, surfaceId, selected, canEdit, stage, onCommitViewport, onCommitFrame }: {
  camera: Camera; steps: ResolvedStep[]; surfaceId: string; selected: string | null; canEdit: boolean; stage: StageSize | null
  onCommitViewport: (s: ResolvedStep, rect: Rect) => void
  onCommitFrame: (s: ResolvedStep, rect: Rect) => void
}) {
  useCameraVersion(camera)
  const [drag, setDrag] = useState<{ id: string; mode: 'move' | 'scale'; start: { x: number; y: number }; base: Rect; rect: Rect } | null>(null)
  const here = steps.map((s, i) => ({ s, n: i + 1 })).filter(({ s }) => s.surfaceId === surfaceId && s.rect)
  const rectOf = (s: ResolvedStep) => (drag?.id === s.step.id ? drag.rect : s.rect!)
  const screen = (r: Rect) => camera.rectToScreen(r)
  const begin = (event: React.PointerEvent, s: ResolvedStep, mode: 'move' | 'scale') => {
    if (!canEdit || !s.target?.capabilities.includes(s.kind === 'viewport' ? 'update' : 'read')) return
    event.stopPropagation()
    event.preventDefault()
    ;(event.currentTarget as Element).setPointerCapture(event.pointerId)
    setDrag({ id: s.step.id, mode, start: { x: event.clientX, y: event.clientY }, base: s.rect!, rect: s.rect! })
  }
  const moveDrag = (event: React.PointerEvent) => {
    if (!drag || !stage) return
    const dx = (event.clientX - drag.start.x) / camera.zoom
    const dy = (event.clientY - drag.start.y) / camera.zoom
    if (drag.mode === 'move') { setDrag({ ...drag, rect: { ...drag.base, x: drag.base.x + dx, y: drag.base.y + dy } }); return }
    // corner scale about the opposite corner, at the stage's aspect
    const w = Math.max(40, drag.base.w + dx)
    const s = steps.find((x) => x.step.id === drag.id)
    const aspect = s?.kind === 'viewport' ? stage.h / stage.w : drag.base.h / drag.base.w
    let rect = { ...drag.base, w, h: w * aspect }
    if (s?.kind === 'viewport') {
      const z = clampZoom(stage.w / rect.w)
      rect = { ...viewportRect({ x: drag.base.x + (stage.w / z) / 2, y: drag.base.y + (stage.h / z) / 2 }, z, stage) }
    }
    setDrag({ ...drag, rect })
  }
  const endDrag = () => {
    if (!drag) return
    const s = steps.find((x) => x.step.id === drag.id)
    const moved = Math.abs(drag.rect.x - drag.base.x) + Math.abs(drag.rect.y - drag.base.y) + Math.abs(drag.rect.w - drag.base.w) > 0.5
    setDrag(null)
    if (!s || !moved) return
    if (s.kind === 'viewport') onCommitViewport(s, drag.rect)
    else onCommitFrame(s, stage && !sameRatio(drag.rect, stage) && drag.mode === 'scale' ? frameAtRatio(drag.rect, stage) : drag.rect)
  }
  const centers = here.map(({ s }) => { const r = screen(rectOf(s)); return { x: r.x + r.w / 2, y: r.y + r.h / 2 } })
  return (
    <div className="aiws-path-overlay" data-testid="aiws-path-overlay" onPointerMove={moveDrag} onPointerUp={endDrag} onPointerCancel={() => setDrag(null)}>
      <svg>
        <defs><marker id="aiws-path-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0 0L10 5L0 10z" fill="currentColor" /></marker></defs>
        {centers.slice(1).map((c, i) => <line key={`l${i}`} className="aiws-path-link" x1={centers[i].x} y1={centers[i].y} x2={c.x} y2={c.y} />)}
        {here.map(({ s }) => {
          const r = screen(rectOf(s))
          const isSel = s.step.id === selected
          return (
            <g key={s.step.id} data-testid={`aiws-path-box-${s.step.id}`}>
              <rect className={`aiws-path-box${isSel ? ' is-selected' : ''}${s.kind === 'frame' ? ' is-frame' : ''}`} x={r.x} y={r.y} width={r.w} height={r.h}
                onPointerDown={isSel ? (event) => begin(event, s, 'move') : undefined} />
              {isSel && canEdit && <rect className="aiws-path-handle" data-testid="aiws-path-scale" x={r.x + r.w - HANDLE} y={r.y + r.h - HANDLE} width={HANDLE * 2} height={HANDLE * 2} style={{ cursor: 'nwse-resize' }} onPointerDown={(event) => begin(event, s, 'scale')} />}
            </g>
          )
        })}
      </svg>
      {here.map(({ s, n }) => {
        const r = screen(rectOf(s))
        return <div key={s.step.id} className="aiws-path-label" style={{ left: r.x, top: r.y }}>{n} · {s.title}</div>
      })}
    </div>
  )
}

/** "添加当前视角" (§5.2): a frame of the stage's aspect in the middle of the canvas; only what is inside it will be seen. */
function Viewfinder({ camera, rect, onConfirm, onCancel, busy }: { camera: Camera; rect: () => Rect | null; onConfirm: () => void; onCancel: () => void; busy: boolean }) {
  const version = useSyncExternalStore((listener) => camera.onChange(listener), () => `${camera.x},${camera.y},${camera.zoom}`)
  void version
  const box = rect()
  if (!box) return null
  return (
    <div className="aiws-viewfinder" data-testid="aiws-viewfinder" style={{ left: box.x, top: box.y, width: box.w, height: box.h }}>
      <div className="aiws-viewfinder-label">只有框内的内容会被看到：移动或缩放画布来取景</div>
      <div className="aiws-viewfinder-actions">
        <button type="button" className="is-primary" data-testid="aiws-viewfinder-confirm" disabled={busy} onClick={onConfirm}>保存这个视角</button>
        <button type="button" onClick={onCancel}>取消</button>
      </div>
    </div>
  )
}
