/* The stage of a non-public show (第三期规划 §7.2, §8, §10, §11.1): the top-level view a show takes over the workspace
 * window with. It reads either the workspace itself (a read-only show) or the show's temporary clone (a show with
 * operable Blocks), never the editing user state; it runs the show controller, the relay, the presenter's marks, the
 * control bar and the keys of §10.4. Leaving it ends the show (lock released, clone deleted, prompter links dead) and
 * gives the window back to the canvas as it was. */

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import {
  ChevronLeft, ChevronRight, Eraser, Expand, Link2, ListOrdered, LogOut, Monitor, MousePointer2, NotebookText, Pencil, RotateCcw, Square, Trash2, Undo2,
} from 'lucide-react'
import type { AiwsClient } from '../api/client'
import { describeError } from '../api/session'
import type { KeyedContent, PresentationProps, ShowPathPayload, ShowStart, ViewportPayload } from '../api/types'
import { StoreContext, useLoad, useOutlineVersion, useVersion, WorkspaceUiContext, type WorkspaceUi } from '../state/hooks'
import type { WorkspaceStore } from '../state/store'
import { BlockHost } from '../ui/blocks/BlockHost'
import type { Camera } from '../ui/canvas/render/camera'
import { WishPanel } from '../ui/wish/WishPanel'
import { command, ShowController } from './controller'
import { fitStage, playable, resolveSteps, type ResolvedStep } from './model'
import { InkLayer, PointerLayer, type InkStroke, type MarkTool } from './overlays'
import { clearActiveShow, prompterLink, saveActiveShow, ShowRelay } from './relay'
import { StageCanvas } from './StageCanvas'
import './presentation.css'

/** What a show runs on. */
export interface ShowSession {
  /** The workspace being presented (the relay and the lock are on it). */
  workspaceId: string
  pathId: string
  startStepId: string | null
  /** The relay of this show; null for a local show (offline: no lock, no prompter, §8.4). */
  start: ShowStart | null
  /** What the stage reads and writes: the clone's store in a show with operable Blocks, otherwise the workspace's own. */
  store: WorkspaceStore
  resume?: { seq: number; cursor: number }
}

const HIDE_CONTROLS_MS = 2000
const POINTER_PUBLISH_MS = 80
const INK_COLORS = ['#ef4444', '#2563eb', '#16a34a', '#111827', '#f59e0b']

function reducedMotion(): boolean {
  return typeof window.matchMedia === 'function' && window.matchMedia('(prefers-reduced-motion: reduce)').matches
}

/** Notes, caption and page background of a step's target, read from the store the stage shows. */
function useTarget(store: WorkspaceStore, step: ResolvedStep | null) {
  const id = step?.targetId ?? ''
  const version = useVersion(`e:${id}`)
  const load = useCallback(async () => {
    if (!id) return null
    const read = await store.readBatched<KeyedContent<Record<string, unknown>>>(id)
    const payload = read.content.payload
    if (step?.kind === 'viewport') { const v = payload as unknown as ViewportPayload; return { notes: v.notes ?? null, caption: v.caption ?? null, background: null } }
    const p = (payload.presentation as PresentationProps | undefined) ?? {}
    return { notes: p.notes ?? null, caption: p.caption ?? null, background: p.background ?? null }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the kind follows the id
  }, [store, id])
  return useLoad(load, `${id}:${version}`).data ?? null
}

/** The workspace view context the Blocks on stage need (no annotations, no navigation away from the stage). */
function useStageUi(store: WorkspaceStore): WorkspaceUi {
  const outlineVersion = useOutlineVersion()
  return useMemo<WorkspaceUi>(() => {
    const entities = store.outline.all()
    return {
      entities, byId: new Map(entities.map((e) => [e.entity_id, e])), annotations: [], openEntity: () => undefined, annotate: null,
      activeAnnotation: null, setActiveAnnotation: () => undefined, annotationFilter: null, showAnnotations: () => undefined,
      renderCell: (cellId, depth) => <div className="aiws-embedded"><BlockHost cellId={cellId} mode="view" view="source" depth={depth} /></div>,
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- outlineVersion is the invalidation signal
  }, [store, outlineVersion])
}

export function StageView({ session, client, onExit }: { session: ShowSession; client: AiwsClient; onExit: (reason?: string) => void }) {
  const ui = useStageUi(session.store)
  return (
    <StoreContext.Provider value={session.store}>
      <WorkspaceUiContext.Provider value={ui}>
        <Stage session={session} client={client} onExit={onExit} />
      </WorkspaceUiContext.Provider>
    </StoreContext.Provider>
  )
}

function Stage({ session, client, onExit }: { session: ShowSession; client: AiwsClient; onExit: (reason?: string) => void }) {
  const { store } = session
  const outlineVersion = useOutlineVersion()
  const pathVersion = useVersion(`e:${session.pathId}`)
  const loadPath = useCallback(() => store.session.read<KeyedContent<ShowPathPayload>>(session.pathId).then((r) => r.content.payload), [store, session.pathId])
  const path = useLoad(loadPath, pathVersion).data ?? null
  const steps = useMemo(() => (path ? playable(resolveSteps(store.outline, path)) : []),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- outlineVersion is the invalidation signal
    [path, store, outlineVersion])
  const [controller, setController] = useState<ShowController | null>(null)
  if (!controller && path && steps.length > 0) {
    const start = Math.max(0, steps.findIndex((s) => s.step.id === session.startStepId))
    setController(new ShowController(steps, start, reducedMotion))
  }
  // a read-only show follows the live document: steps that change or go away are followed, not crashed into
  useEffect(() => { controller?.updateSteps(steps) }, [controller, steps])
  if (!path) return <div className="aiws-show aiws-show-loading" data-testid="aiws-show">正在准备放映…</div>
  if (!controller) {
    return (
      <div className="aiws-show aiws-show-loading" data-testid="aiws-show">
        <div className="aiws-panel aiws-show-empty">
          <b>这条路径没有可以放映的步骤</b>
          <div className="aiws-muted">步骤都已停用，或它们的目标已删除。请在“路径编辑”中检查失效项。</div>
          <button type="button" data-testid="aiws-show-exit" onClick={() => onExit()}>退出放映</button>
        </div>
      </div>
    )
  }
  return <StageRunning session={session} client={client} onExit={onExit} controller={controller} path={path} />
}

function StageRunning({ session, client, onExit, controller, path }: { session: ShowSession; client: AiwsClient; onExit: (reason?: string) => void; controller: ShowController; path: ShowPathPayload }) {
  const { store } = session
  const state = useSyncExternalStore(controller.subscribe, controller.snapshot)
  const current = controller.current()
  const next = controller.next()
  const target = useTarget(store, current)
  const nextTarget = useTarget(store, next)
  const rootRef = useRef<HTMLDivElement>(null)
  const [editing, setEditing] = useState<string | null>(null)
  const [wish, setWish] = useState<{ wishId: string; cellId: string } | null>(null)
  const [tool, setTool] = useState<MarkTool>('none')
  const [ink, setInk] = useState<InkStroke[]>([])
  const [inkColor, setInkColor] = useState(INK_COLORS[0])
  const [onStage, setOnStage] = useState<{ camera: Camera; surfaceId: string } | null>(null)
  const [panel, setPanel] = useState<'none' | 'steps' | 'notes' | 'link'>('none')
  const [controlsShown, setControlsShown] = useState(true)
  const [problem, setProblem] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)
  const hideTimer = useRef(0)
  const relayRef = useRef<ShowRelay | null>(null)
  const startedAt = session.start?.started_at ?? ''
  const exitRef = useRef(onExit)
  useLayoutEffect(() => { exitRef.current = onExit })

  // ---- the relay: commands in, state out
  useEffect(() => {
    if (!session.start) return
    const relay = new ShowRelay(client, session.workspaceId, session.start, session.resume)
    relayRef.current = relay
    relay.onCommand = (cmd) => controller.apply(cmd)
    relay.onEnded = (reason) => { clearActiveShow(session.workspaceId); exitRef.current(reason) }
    relay.onTrouble = (detail) => setProblem(detail ? `提示器连接不稳定：${detail}` : null)
    relay.run()
    const publish = () => {
      relay.publish(controller.published(relay.showId, session.pathId, startedAt))
      saveActiveShow({ workspaceId: session.workspaceId, pathId: session.pathId, start: relay.start, ...relay.position(), stepId: controller.current()?.step.id ?? null })
    }
    publish()
    const off = controller.subscribe(publish)
    return () => { off(); relay.detach(); relayRef.current = null }
  }, [session, client, controller, startedAt])

  const exit = useCallback(() => {
    const relay = relayRef.current
    clearActiveShow(session.workspaceId)
    void relay?.close()
    exitRef.current()
  }, [session.workspaceId])

  // ---- controls hide when the mouse rests (§10.1)
  const wake = useCallback(() => {
    setControlsShown(true)
    window.clearTimeout(hideTimer.current)
    hideTimer.current = window.setTimeout(() => setControlsShown(false), HIDE_CONTROLS_MS)
  }, [])
  useEffect(() => {
    hideTimer.current = window.setTimeout(() => setControlsShown(false), HIDE_CONTROLS_MS)
    return () => window.clearTimeout(hideTimer.current)
  }, [])

  // ---- keys (§10.4): without an activated Block the keys drive the show; with one, they are the Block's (Esc gives them back)
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const el = event.target instanceof Element ? event.target : null
      if (editing || wish) {
        if (event.key === 'Escape' && !el?.closest('.aiws-show-drawer input, .aiws-show-drawer textarea')) { setEditing(null); if (wish) setWish(null) }
        return
      }
      if (event.isComposing || el?.closest('input, textarea, select, [contenteditable="true"], [data-role="editor"]')) return
      if (event.ctrlKey || event.metaKey || event.altKey) return
      const run = (kind: Parameters<typeof command>[0], args?: Parameters<typeof command>[1]) => { event.preventDefault(); controller.apply(command(kind, args)) }
      switch (event.key) {
        case 'ArrowRight': case 'ArrowDown': case 'PageDown': case ' ': case 'Enter': run('next'); break
        case 'ArrowLeft': case 'ArrowUp': case 'PageUp': case 'Backspace': run('prev'); break
        case 'Home': run('first'); break
        case 'End': run('last'); break
        case 'b': case 'B': case '.': run('black'); break
        case 'Escape':
          event.preventDefault()
          if (panel !== 'none') setPanel('none')
          else if (tool !== 'none') setTool('none')
          else if (controller.snapshot().free) controller.apply(command('back'))
          else exit()
          break
        default: return
      }
      wake()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [controller, editing, wish, panel, tool, exit, wake])

  // ---- operable Blocks (§10.2): only `presentation.live`; a wish opens its panel beside the stage
  const onEditingChange = (id: string | null) => {
    if (!id) { setEditing(null); return }
    const entity = store.outline.get(id)
    if (!entity?.live) return
    if (entity.view_type === 'wish' && entity.source_id) { setWish({ wishId: entity.source_id, cellId: id }); return }
    setEditing(id)
  }

  // ---- the laser pointer, in stage coordinates for the prompters
  const lastPointer = useRef(0)
  const onPointer = (p: { x: number; y: number } | null) => {
    const now = performance.now()
    if (p && now - lastPointer.current < POINTER_PUBLISH_MS) return
    lastPointer.current = now
    const el = rootRef.current
    if (!p || !el) { controller.setPointer(null); return }
    const s = fitStage({ x: 0, y: 0, w: el.clientWidth, h: el.clientHeight }, path.stage)
    controller.setPointer({ x: Math.max(0, Math.min(1, (p.x - s.x) / s.w)), y: Math.max(0, Math.min(1, (p.y - s.y) / s.h)) })
  }
  useEffect(() => { if (tool !== 'pointer') controller.setPointer(null) }, [tool, controller])

  const link = session.start ? prompterLink(session.workspaceId, session.start) : null
  const copyLink = () => {
    if (!link) return
    void navigator.clipboard?.writeText(link).then(() => { setCopied(true); window.setTimeout(() => setCopied(false), 1500) }, () => setPanel('link'))
  }
  const fullscreen = () => { void rootRef.current?.requestFullscreen?.().catch(() => undefined) }
  const screens = typeof (window as { getScreenDetails?: unknown }).getScreenDetails === 'function'
  const toOtherScreen = async () => {
    try {
      const details = await (window as unknown as { getScreenDetails: () => Promise<{ screens: { isPrimary: boolean }[]; currentScreen: unknown }> }).getScreenDetails()
      const other = details.screens.find((s) => !s.isPrimary) ?? details.currentScreen
      await rootRef.current?.requestFullscreen({ screen: other } as FullscreenOptions)
    } catch (error) {
      store.notify('error', `无法放到其他屏幕：${describeError(error)}`)
    }
  }

  const steps = controller.playable()
  const live = session.start?.live ?? false
  const statusText = live ? '非公开放映：可操作的 Block 写入临时副本，本场修改不会保存' : '非公开放映：本场修改不会保存'
  const visibleControls = controlsShown || panel !== 'none' || tool === 'pen' || tool === 'eraser'
  return (
    <div ref={rootRef} className={`aiws-show${visibleControls ? '' : ' is-idle'}${tool !== 'none' ? ` has-tool-${tool}` : ''}`} data-testid="aiws-show" data-show-id={session.start?.show_id}
      data-step={current?.step.id} data-index={state.index} data-count={steps.length} data-black={state.black ? 'true' : undefined} data-free={state.free ? 'true' : undefined}
      data-live={live ? 'true' : undefined} onPointerMove={wake}>
      <StageCanvas stage={path.stage} background={path.background} current={current} next={next} pageBackground={target?.background}
        transition={state.transition} nonce={state.nonce} free={state.free} black={state.black} onFree={() => controller.setFree()}
        editing={editing} onEditingChange={onEditingChange} onCamera={(camera, surfaceId) => setOnStage((prev) => (prev?.camera === camera ? prev : { camera, surfaceId }))}
        underMask={<InkLayer camera={onStage?.camera ?? null} surfaceId={onStage?.surfaceId ?? null} tool={tool} color={inkColor} width={4} strokes={ink} onStrokes={setInk} />}
        overMask={<PointerLayer active={tool === 'pointer'} onMove={onPointer} />} />

      {state.free && (
        <button type="button" className="aiws-show-back aiws-stage-ui" data-testid="aiws-show-back" onClick={() => controller.apply(command('back'))}>
          <RotateCcw size={16} aria-hidden="true" />回到本步骤
        </button>
      )}
      {editing && <div className="aiws-show-hint aiws-stage-ui" role="status">正在操作这个 Block：按键交给它；Esc 交还给放映</div>}

      <div className="aiws-show-bar aiws-panel aiws-stage-ui" role="toolbar" aria-label="放映控制" data-testid="aiws-show-bar">
        <button type="button" className="aiws-tool" aria-label="上一项" title="上一项（← PageUp）" data-testid="aiws-show-prev" disabled={state.index === 0 && !state.free} onClick={() => controller.apply(command('prev'))}><ChevronLeft size={18} /></button>
        <button type="button" className="aiws-tool aiws-show-counter" aria-label="步骤目录" title="步骤目录" aria-pressed={panel === 'steps'} data-testid="aiws-show-counter" onClick={() => setPanel(panel === 'steps' ? 'none' : 'steps')}>
          <ListOrdered size={16} aria-hidden="true" /><span>{state.index + 1} / {steps.length}</span>
        </button>
        <button type="button" className="aiws-tool" aria-label="下一项" title="下一项（→ PageDown 空格）" data-testid="aiws-show-next" disabled={state.index >= steps.length - 1 && !state.free} onClick={() => controller.apply(command('next'))}><ChevronRight size={18} /></button>
        <span className="aiws-show-sep" />
        <button type="button" className="aiws-tool" aria-label="黑屏" title="黑屏（B）" aria-pressed={state.black} data-testid="aiws-show-black" onClick={() => controller.apply(command('black'))}><Square size={16} fill={state.black ? 'currentColor' : 'none'} /></button>
        <button type="button" className="aiws-tool" aria-label="激光笔" title="激光笔" aria-pressed={tool === 'pointer'} data-testid="aiws-show-pointer" onClick={() => setTool(tool === 'pointer' ? 'none' : 'pointer')}><MousePointer2 size={16} /></button>
        <button type="button" className="aiws-tool" aria-label="板书" title="板书（触控笔或鼠标）" aria-pressed={tool === 'pen'} data-testid="aiws-show-pen" onClick={() => setTool(tool === 'pen' ? 'none' : 'pen')}><Pencil size={16} /></button>
        {(tool === 'pen' || tool === 'eraser') && (
          <span className="aiws-show-ink-tools" data-testid="aiws-show-ink-tools">
            {INK_COLORS.map((color) => <button key={color} type="button" className={`aiws-show-swatch${inkColor === color && tool === 'pen' ? ' is-on' : ''}`} style={{ background: color }} aria-label={`笔色 ${color}`} onClick={() => { setInkColor(color); setTool('pen') }} />)}
            <button type="button" className="aiws-tool" aria-label="橡皮" title="橡皮" aria-pressed={tool === 'eraser'} data-testid="aiws-show-eraser" onClick={() => setTool(tool === 'eraser' ? 'pen' : 'eraser')}><Eraser size={16} /></button>
            <button type="button" className="aiws-tool" aria-label="撤销一笔" title="撤销一笔" disabled={ink.length === 0} data-testid="aiws-show-ink-undo" onClick={() => setInk(ink.slice(0, -1))}><Undo2 size={16} /></button>
            <button type="button" className="aiws-tool" aria-label="清除板书" title="清除板书" disabled={ink.length === 0} data-testid="aiws-show-ink-clear" onClick={() => setInk([])}><Trash2 size={16} /></button>
          </span>
        )}
        <span className="aiws-show-sep" />
        <button type="button" className="aiws-tool" aria-label="讲解备注" title="展开当前步骤的备注（单屏排练）" aria-pressed={panel === 'notes'} data-testid="aiws-show-notes" onClick={() => setPanel(panel === 'notes' ? 'none' : 'notes')}><NotebookText size={16} /></button>
        <button type="button" className="aiws-tool" aria-label="复制提示器链接" title={link ? (copied ? '已复制' : '复制提示器链接：任何浏览器打开即可翻页并看备注') : '离线放映没有提示器链接'} disabled={!link} data-testid="aiws-show-link"
          onClick={() => { copyLink(); setPanel(panel === 'link' ? 'none' : 'link') }}><Link2 size={16} /></button>
        <button type="button" className="aiws-tool" aria-label="全屏" title="全屏" data-testid="aiws-show-fullscreen" onClick={fullscreen}><Expand size={16} /></button>
        {screens && <button type="button" className="aiws-tool" aria-label="放到其他屏幕" title="全屏显示到另一块屏幕" onClick={() => { void toOtherScreen() }}><Monitor size={16} /></button>}
        <span className="aiws-show-sep" />
        <span className="aiws-show-status" data-testid="aiws-show-status" title={statusText}>{live ? '临时副本' : session.start?.locked ? '只读放映 · 已关闭写入' : session.start ? '只读放映' : '本地放映'}</span>
        <button type="button" className="aiws-tool aiws-tool-text" data-testid="aiws-show-exit" title="退出放映（Esc）" onClick={exit}><LogOut size={16} /><span>退出</span></button>
      </div>
      {panel === 'none' && <div className="aiws-show-note aiws-stage-ui" role="note" data-testid="aiws-show-note">{statusText}{problem ? ` · ${problem}` : ''}</div>}

      {panel === 'steps' && (
        <div className="aiws-show-panel aiws-panel aiws-stage-ui" data-testid="aiws-show-steps" role="listbox" aria-label="步骤目录">
          {steps.map((s, i) => (
            <button key={s.step.id} type="button" role="option" aria-selected={i === state.index} className={`aiws-show-step${i === state.index ? ' is-current' : ''}`} data-testid={`aiws-show-step-${s.step.id}`}
              onClick={() => { controller.apply(command('goto', { step_id: s.step.id })); setPanel('none') }}>
              <span className="aiws-show-step-n">{i + 1}</span><span className="aiws-show-step-title">{s.title}</span><span className="aiws-muted">{s.kind === 'frame' ? 'Frame' : 'Viewport'}</span>
            </button>
          ))}
        </div>
      )}
      {panel === 'notes' && (
        <div className="aiws-show-panel aiws-show-notes aiws-panel aiws-stage-ui" data-testid="aiws-show-notes-panel">
          <div className="aiws-show-notes-title">{current?.title}</div>
          <div className="aiws-show-notes-body">{target?.notes || <span className="aiws-muted">这一步没有讲解备注。</span>}</div>
          {next && <div className="aiws-muted">下一步：{next.title}{nextTarget?.notes ? '（有备注）' : ''}</div>}
        </div>
      )}
      {panel === 'link' && link && (
        <div className="aiws-show-panel aiws-panel aiws-stage-ui" data-testid="aiws-show-link-panel">
          <b>提示器链接</b>
          <div className="aiws-muted">在任何浏览器（手机、平板、另一台电脑）打开它，即进入本场的演讲者控制面板。持有链接的人可以翻页并看到备注；放映结束后链接立即失效。</div>
          <input readOnly value={link} aria-label="提示器链接" data-testid="aiws-show-link-input" onFocus={(event) => event.currentTarget.select()} />
          <div className="aiws-dialog-actions">
            <button type="button" className="is-primary" onClick={copyLink}>{copied ? '已复制' : '复制'}</button>
            <button type="button" onClick={() => window.open(link, '_blank', 'noopener')}>在新窗口打开</button>
          </div>
        </div>
      )}
      {wish && (
        <div className="aiws-show-drawer aiws-panel aiws-stage-ui" data-testid="aiws-show-wish">
          <div className="aiws-dialog-head"><b>许愿格</b><span className="aiws-grow" /><button type="button" onClick={() => setWish(null)}>关闭</button></div>
          <WishPanel wishId={wish.wishId} cellId={wish.cellId} readOnly={!live} />
        </div>
      )}
    </div>
  )
}
