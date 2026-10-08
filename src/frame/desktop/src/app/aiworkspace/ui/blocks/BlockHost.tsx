/* BlockHost (phase two §5, §8.3, §10.2): the lifecycle of one Block. It reads the Cell, resolves the
 * Renderer in the registry, decides which implementation runs for the current canvas sub-mode and
 * state (static / simplified / view / editor), enforces the mode policy, keeps the mount budget for
 * expensive implementations, and localises every failure: a missing renderer, unreadable data or a
 * throwing component becomes a fallback inside this Block, never a broken workspace. */

import { Component, type ErrorInfo, type ReactNode, memo, useContext, useEffect, useState } from 'react'
import { TriangleAlert } from 'lucide-react'
import { describeError } from '../../api/session'
import type { CellPayload, EntityEnvelope } from '../../api/types'
import { useStore } from '../../state/hooks'
import { shapeOf, sourceLabel } from './affordances'
import { BudgetContext, type MountBudget } from './budget'
import { BlockMetaContext } from './editorToolbar'
import type { CanvasMode, RenderContext } from './registry'
import { useBlockContext } from './useBlockContext'

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

export class BlockBoundary extends Component<BoundaryProps, BoundaryState> {
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

/** The generic read-only fallback (D6, §10.3): raw content stays visible and exportable; the warning is a corner
 * badge whose explanation shows on hover (标准对象的交互改进 §4.1). */
export function GenericFallback({ cell, payload, reason, detail, source }: { cell: EntityEnvelope; payload: CellPayload | null; reason: string; detail: string; source?: EntityEnvelope }) {
  return (
    <div className="aiws-block-fallback" data-testid={`aiws-block-fallback-${cell.entity_id}`} data-reason={reason}>
      <span className="aiws-fallback-badge" title={detail} aria-label={detail}><TriangleAlert size={12} aria-hidden="true" /></span>
      <div className="aiws-block-fallback-title">{cell.title ?? payload?.title ?? cell.entity_id}</div>
      <div className="aiws-fallback-detail">{detail}</div>
      {payload && <div className="aiws-muted">渲染器 {payload.view.type}{payload.view.version ? ` v${payload.view.version}` : ''}{source ? ` · 数据 ${source.name ?? source.entity_id}（${source.type_id}）` : ''}</div>}
      {payload?.config && <details><summary>原始配置</summary><pre>{JSON.stringify(payload.config, null, 2)}</pre></details>}
    </div>
  )
}

/** Memoised: pans and overlay updates re-render the frame list only; a Block re-renders when its own props change. */
export const BlockHost = memo(function BlockHost(props: BlockHostProps) {
  const { context, resolution, error, registryVersion } = useBlockContext(props)
  if (!context && error) return <div className="aiws-block-fallback" data-testid={`aiws-block-fallback-${props.cellId}`} data-reason="read_failed"><div className="aiws-error" role="alert">无法读取 Block：{error}</div></div>
  if (!context) return <div className="aiws-block-loading" data-testid={`aiws-block-loading-${props.cellId}`}>载入中…</div>
  if (!resolution) return <div className="aiws-block-loading">载入定义…</div>
  if (!resolution.ok) return <GenericFallback cell={context.cell} payload={context.payload} reason={resolution.reason} detail={resolution.detail} source={context.source} />
  return <ResolvedBlock context={context} lod={props.lod ?? 'full'} registryVersion={registryVersion} />
})

function ResolvedBlock({ context: base, lod, registryVersion }: { context: RenderContext; lod: Lod; registryVersion: number }) {
  const { cell, mode, definition: def, dataState, depth } = base
  const store = useStore()
  const metaSink = useContext(BlockMetaContext)
  const wantsEditor = base.editorActive && Boolean(def.Editor)
  const budgetKind: keyof MountBudget | null = wantsEditor ? (def.cost.html ? 'html' : def.cost.editor ? 'editors' : null) : null
  const slot = useBudgetSlot(budgetKind)
  const context = { ...base, editorActive: wantsEditor && slot }
  // a hovered or selected Block tells the overlay its outline, resize rule and affordances (§4.2)
  const published = metaSink !== null && depth === 0 && (base.hovered || base.selected)
  useEffect(() => {
    if (!metaSink) return
    if (!published) { metaSink.set(cell.entity_id, null); return }
    let affordances = def.hover ? [] : sourceLabel(context)
    try { if (def.hover) affordances = def.hover(context, store) } catch (error) { console.error('[aiworkspace] hover affordances failed', error) }
    metaSink.set(cell.entity_id, { shape: shapeOf(context), aspect: def.resize?.aspect ?? 'free', affordances, context })
  })
  useEffect(() => () => metaSink?.set(cell.entity_id, null), [metaSink, cell.entity_id])
  let Impl = def.Static
  let role: 'static' | 'simplified' | 'view' | 'editor' = 'static'
  if (lod === 'simplified' && def.Simplified) { Impl = def.Simplified; role = 'simplified' }
  if (wantsEditor && slot && def.Editor) { Impl = def.Editor; role = 'editor' }
  else if (depth === 0 && (mode === 'view' || mode === 'show') && def.View && lod !== 'simplified') { Impl = def.View; role = 'view' }
  const budgetNote = wantsEditor && !slot ? '同时激活的编辑器已达上限，此 Block 保持静态显示' : null
  return (
    <div className="aiws-block-body" data-testid={`aiws-block-${cell.entity_id}`} data-role={role} data-renderer={def.type} data-mode={mode} data-data-state={dataState} data-chrome={def.chrome ?? 'clip'}>
      {budgetNote && <div className="aiws-warning" data-testid="aiws-block-budget">{budgetNote}</div>}
      <BlockBoundary resetKey={`${cell.entity_id}:${role}:${cell.content_rev}:${registryVersion}`} fallback={(error, reset) => (
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
