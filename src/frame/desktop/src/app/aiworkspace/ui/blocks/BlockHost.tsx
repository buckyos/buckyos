/* BlockHost (phase two §5, §8.3, §10.2): the lifecycle of one Block. It reads the Cell, resolves the
 * Renderer in the registry, decides which implementation runs for the current canvas sub-mode and
 * state (static / simplified / view / editor), enforces the mode policy, keeps the mount budget for
 * expensive implementations, and localises every failure: a missing renderer, unreadable data or a
 * throwing component becomes a fallback inside this Block, never a broken workspace. */

import { Component, type ErrorInfo, type ReactNode, memo, useCallback, useContext, useEffect, useMemo, useState, useSyncExternalStore } from 'react'
import { describeError, type ReadOk } from '../../api/session'
import type { Capability, CellPayload, EntityEnvelope, KeyedContent } from '../../api/types'
import { useDirectReadOnly, useEntity, useLoad, useStore, useVersion, useWorkspaceUi } from '../../state/hooks'
import { BudgetContext, type MountBudget } from './budget'
import { blockRegistry, modePolicy, type BlockDefinition, type CanvasMode, type DataState, type RenderContext, type Resolution } from './registry'

export type Lod = 'full' | 'simplified' | 'placeholder'

export interface BlockHostProps {
  cellId: string
  mode: CanvasMode
  view: 'canvas' | 'source'
  selected?: boolean
  hovered?: boolean
  /** The host that owns selection decides activation (double-click / "edit content"); the source view activates at once. */
  editorActive?: boolean
  onActivate?: () => void
  onDeactivate?: () => void
  size?: { w: number; h: number }
  zoom?: number
  lod?: Lod
  /** Embedded inside rich text: always static, never an editor. */
  depth?: number
}

/** Reserve one slot of the budget while mounted; `false` when the limit is reached. */
function useBudgetSlot(kind: keyof MountBudget | null): boolean {
  const budget = useContext(BudgetContext)
  const [granted, setGranted] = useState<boolean>(false)
  useEffect(() => {
    if (!kind) return
    let held = false
    const attempt = () => { if (!held && budget.take(kind)) { held = true; setGranted(true) } }
    attempt()
    const off = budget.subscribe(attempt)
    return () => { off(); if (held) budget.give(kind); setGranted(false) }
  }, [budget, kind])
  return kind ? granted : true
}

interface BoundaryProps { children: ReactNode; fallback: (error: Error, reset: () => void) => ReactNode; resetKey: string }
interface BoundaryState { error: Error | null; key: string }

class BlockBoundary extends Component<BoundaryProps, BoundaryState> {
  state: BoundaryState = { error: null, key: this.props.resetKey }
  static getDerivedStateFromProps(props: BoundaryProps, state: BoundaryState): Partial<BoundaryState> | null {
    return props.resetKey !== state.key ? { error: null, key: props.resetKey } : null
  }
  static getDerivedStateFromError(error: Error): Partial<BoundaryState> { return { error } }
  componentDidCatch(error: Error, info: ErrorInfo) { console.error('[aiworkspace] block renderer failed', error, info.componentStack) }
  render() {
    if (this.state.error) return this.props.fallback(this.state.error, () => this.setState({ error: null }))
    return this.props.children
  }
}

function dataStateOf(payload: CellPayload, source: EntityEnvelope | undefined, loaded: boolean): DataState {
  if (!payload.source_ref) return 'none'
  if (!loaded) return 'ready'
  if (!source || source.deleted) return 'missing'
  if (!source.capabilities.includes('read')) return 'unreadable'
  if (source.degraded) return 'degraded'
  return 'ready'
}

/** The generic read-only fallback (D6, §10.3): raw content stays visible and exportable. */
export function GenericFallback({ cell, payload, reason, detail, source }: { cell: EntityEnvelope; payload: CellPayload | null; reason: string; detail: string; source?: EntityEnvelope }) {
  return (
    <div className="aiws-block-fallback" data-testid={`aiws-block-fallback-${cell.entity_id}`} data-reason={reason}>
      <div className="aiws-block-fallback-title">{cell.title ?? payload?.title ?? cell.entity_id}</div>
      <div className="aiws-warning">{detail}</div>
      {payload && <div className="aiws-muted">渲染器 {payload.view.type}{payload.view.version ? ` v${payload.view.version}` : ''}{source ? ` · 数据 ${source.name ?? source.entity_id}（${source.type_id}）` : ''}</div>}
      {payload?.config && <details><summary>原始配置</summary><pre>{JSON.stringify(payload.config, null, 2)}</pre></details>}
    </div>
  )
}

/** Memoised: pans and overlay updates re-render the frame list only; a Block re-renders when its own props change. */
export const BlockHost = memo(function BlockHost(props: BlockHostProps) {
  const { cellId, depth = 0 } = props
  const store = useStore()
  const ui = useWorkspaceUi()
  const cellEntity = useEntity(cellId)
  const version = useVersion(`e:${cellId}`)
  const load = useCallback(() => store.readBatched<KeyedContent<CellPayload>>(cellId), [store, cellId])
  const read = useLoad<ReadOk<KeyedContent<CellPayload>>>(load, version)
  useSyncExternalStore(blockRegistry.subscribe, blockRegistry.snapshot)
  const payload = read.data?.content.payload
  const sourceId = payload?.source_ref?.entity_id
  const source = useEntity(sourceId ?? '')
  const readOnlyNow = useDirectReadOnly()
  const cell = cellEntity ?? read.data
  if (read.error && !read.data) {
    const failure = read.error
    return <div className="aiws-block-fallback" data-testid={`aiws-block-fallback-${cellId}`} data-reason="read_failed"><div className="aiws-error" role="alert">无法读取 Block：{failure}</div></div>
  }
  if (!payload || !cell) return <div className="aiws-block-loading" data-testid={`aiws-block-loading-${cellId}`}>载入中…</div>
  return <ResolvedBlock {...props} cell={cell} payload={payload} keyRevs={read.data?.content.key_revs ?? {}} source={source} readOnlyNow={readOnlyNow} openEntity={ui.openEntity} depth={depth} />
})

function ResolvedBlock(props: BlockHostProps & { cell: EntityEnvelope; payload: CellPayload; keyRevs: Record<string, number>; source: EntityEnvelope | undefined; readOnlyNow: boolean; openEntity: (id: string) => void }) {
  const { cell, payload, source, mode, view, readOnlyNow, depth = 0, lod = 'full' } = props
  const store = useStore()
  const resolution: Resolution = useMemo(() => blockRegistry.resolve(payload, source?.type_id), [payload, source?.type_id])
  const definition = resolution.ok ? resolution.definition : resolution.definition
  const policy = modePolicy(mode)
  const embedded = depth > 0
  const loaded = store.outline.isLoaded()
  const dataState = dataStateOf(payload, source, loaded)
  // explicit activation only (§8.3): the owner of the selection says so; the source view activates directly
  const wantsEditor = Boolean(props.editorActive) && !embedded && policy.editContent && Boolean(definition?.Editor)
  const html = definition?.cost.html ?? false
  const budgetKind: keyof MountBudget | null = wantsEditor ? (html ? 'html' : definition?.cost.editor ? 'editors' : null) : null
  const slot = useBudgetSlot(budgetKind)
  const activateRef = props.onActivate
  const deactivateRef = props.onDeactivate
  const capabilities: Capability[] = cell.capabilities
  const readOnlyReason = readOnlyNow ? '后台不可达，此窗口未启用离线：只读'
    : mode === 'presentation_edit' ? '播放编辑尚未实现：只读占位'
      : mode === 'view' ? '查看模式：除批注外不修改文档'
        : !capabilities.includes('update') ? '没有修改此 Block 的权限'
          : source && !source.capabilities.some((c) => c === 'update' || c === 'append') && payload.source_ref ? '没有修改其数据的权限' : null
  const context: RenderContext = {
    cell, payload, keyRevs: props.keyRevs, source, definition: definition ?? MISSING_DEF, mode, view,
    selected: Boolean(props.selected), hovered: Boolean(props.hovered), editorActive: wantsEditor && slot,
    capabilities, readOnlyReason, dataState, size: props.size ?? { w: payload.config?.w as number ?? 320, h: 200 }, zoom: props.zoom ?? 1,
    activateEditor: () => { if (policy.editContent && !embedded) activateRef?.() },
    deactivateEditor: () => deactivateRef?.(),
    openEntity: props.openEntity,
  }
  if (!resolution.ok) {
    if (resolution.reason === 'source_required' || resolution.reason === 'type_not_accepted') {
      // the definition exists but cannot show this binding: generic read-only view (D6)
      return <GenericFallback cell={cell} payload={payload} reason={resolution.reason} detail={resolution.detail} source={source} />
    }
    return <GenericFallback cell={cell} payload={payload} reason={resolution.reason} detail={resolution.detail} source={source} />
  }
  if (dataState === 'missing') return <GenericFallback cell={cell} payload={payload} reason="data_missing" detail="绑定的数据不存在或已删除。" />
  if (dataState === 'unreadable') return <GenericFallback cell={cell} payload={payload} reason="data_unreadable" detail="没有读取其数据的权限。" />
  const def = resolution.definition
  let Impl = def.Static
  let role: 'static' | 'simplified' | 'view' | 'editor' = 'static'
  if (lod === 'simplified' && def.Simplified) { Impl = def.Simplified; role = 'simplified' }
  if (wantsEditor && slot && def.Editor) { Impl = def.Editor; role = 'editor' }
  else if (mode === 'view' && def.View && lod !== 'simplified') { Impl = def.View; role = 'view' }
  const budgetNote = wantsEditor && !slot ? '同时激活的编辑器已达上限，此 Block 保持静态显示' : null
  return (
    <div className="aiws-block-body" data-testid={`aiws-block-${cell.entity_id}`} data-role={role} data-renderer={def.type} data-mode={mode} data-data-state={dataState}>
      {budgetNote && <div className="aiws-warning" data-testid="aiws-block-budget">{budgetNote}</div>}
      <BlockBoundary resetKey={`${cell.entity_id}:${role}:${cell.content_rev}`} fallback={(error, reset) => (
        <div className="aiws-block-fallback" data-testid={`aiws-block-fallback-${cell.entity_id}`} data-reason="renderer_error">
          <div className="aiws-error" role="alert">渲染器 {def.type} 出错：{describeError(error)}</div>
          <button type="button" className="aiws-link" onClick={reset}>重试</button>
        </div>
      )}>
        <Impl {...context} />
      </BlockBoundary>
    </div>
  )
}

const MISSING_DEF: BlockDefinition = {
  type: 'missing', version: 0, title: '（缺失）', accepts: [], allowNoSource: true, defaultSize: { w: 240, h: 120 }, cost: { editor: false, html: false },
  Static: () => null,
}
