/* The properties panel (UI 框架 PRD §10.2): one floating panel for the whole window, opened only by the "properties"
 * action (Block toolbar, context menu, F2 — selecting a Block never opens it). Unpinned it stands beside the Block —
 * right, else left, below, above — clear of the tool areas and the window edges, and moves there again after a pan or a
 * zoom has ended; pinned it stays where it was put (by default against the right edge) and follows the selection, showing
 * the page's properties when nothing is selected. Its height is at most the room between the tool areas and 560 px; the
 * body scrolls under a fixed title. Sections fold, and how each was left is remembered per Block type (user state
 * `ui:props-sections:<type>`). Every change is one undoable commit; without the right or in view mode it is read-only
 * and says why. On a phone it is a sheet from the bottom. */

import { useCallback, useLayoutEffect, useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react'
import { ChevronDown, ChevronRight, ChevronsUp, FileText, Pin, PinOff, SlidersHorizontal, X } from 'lucide-react'
import { num } from '@controls/placement'
import { describeError } from '../../api/session'
import type { EntityEnvelope, Json, StoredPlacement } from '../../api/types'
import { setLive } from '../../presentation/pathOps'
import { useStore, useUserState, useWorkspaceUi } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { BlockBoundary } from '../blocks/BlockHost'
import { setCellConfig } from '../blocks/ops'
import type { CanvasMode, RenderContext } from '../blocks/registry'
import { InstanceConfig, InstanceData, InstanceEvents, isInstance } from '../controls/composite/Inspector'
import { controlRegistry } from '../controls/registry'
import { MetadataForm } from '../canvas/MetadataForm'
import type { Laid } from '../canvas/layout'
import type { Rect } from '../canvas/render/camera'
import { renameBase, renameSurface, setSurfaceBackground, surfaceWriteReason, useSurfaceConfig } from '../canvas/surfaceManage'
import { PaletteGrid } from '../canvas/tools'
import { fieldsOf, PropertyControl } from './properties'
import './blockui.css'

const WIDTH = 320
const MAX_HEIGHT = 560
const GAP = 12

/** Pinned or not, and where (`{ pinned, x, y }`): a cross-workspace preference of the person (§10.2, K4, state/prefs.ts). */
export { PROPS_PANEL_PREF as PROPS_PANEL_KEY } from '../framework/personal'

export interface PanelLayout {
  laid: Laid | undefined
  /** May this person move and resize it now (and is it unlocked)? */
  reason: string | null
  /** Write a new placement (one undoable commit). */
  place: (placement: StoredPlacement) => void
  /** Lock or unlock it; null when this person may not. */
  setLocked: ((locked: boolean) => void) | null
}

export interface PropertyPanelProps {
  /** The selected Block (its backing for a swimlane), or null for the page's properties. */
  subject: EntityEnvelope | null
  /** How many objects are selected (several: only what they share). */
  count: number
  context: RenderContext | null
  resolved: boolean
  registryVersion: number
  mode: CanvasMode
  surface: EntityEnvelope
  isFree: boolean
  layout: PanelLayout | null
  /** The Block on screen (canvas area coordinates), for the unpinned placement. */
  anchor: Rect | null
  area: { w: number; h: number }
  insets: { top: number; right: number; bottom: number; left: number }
  pinned: boolean
  pinAt: { x: number; y: number } | null
  phone: boolean
  onPin: (pinned: boolean) => void
  onMove: (at: { x: number; y: number }) => void
  onClose: () => void
}

type SectionState = Record<string, boolean>

/** The section frame: a heading that folds it. */
function Section({ id, title, collapsed, onToggle, children }: { id: string; title: string; collapsed: boolean; onToggle: () => void; children: ReactNode }) {
  return (
    <section className={`aiws-props-section${collapsed ? ' is-collapsed' : ''}`} data-testid={`aiws-props-section-${id}`} data-collapsed={collapsed ? 'true' : 'false'}>
      <button type="button" className="aiws-props-section-head" aria-expanded={!collapsed} data-testid={`aiws-props-toggle-${id}`} onClick={onToggle}>
        {collapsed ? <ChevronRight size={14} aria-hidden="true" /> : <ChevronDown size={14} aria-hidden="true" />}<span>{title}</span>
      </button>
      {!collapsed && <div className="aiws-props-section-body">{children}</div>}
    </section>
  )
}

/** A value written when the field is left (Enter, blur). */
function Field({ label, value, disabled, testId, kind = 'text', onCommit }: { label: string; value: string; disabled?: boolean; testId: string; kind?: 'text' | 'number'; onCommit: (value: string) => void }) {
  const [draft, setDraft] = useState<string | null>(null)
  const done = () => { if (draft !== null && draft !== value) onCommit(draft); setDraft(null) }
  return (
    <label className="aiws-props-field">
      <span className="aiws-props-label">{label}</span>
      <input type={kind} step="any" aria-label={label} value={draft ?? value} disabled={disabled} data-testid={testId} onChange={(e) => setDraft(e.target.value)} onBlur={done}
        onKeyDown={(e) => { if (e.key === 'Enter') e.currentTarget.blur(); else if (e.key === 'Escape') { e.stopPropagation(); setDraft(null) } }} />
    </label>
  )
}

/** Where the unpinned panel goes beside `a`: right, left, below, above — the first that fits the clear area. */
function besideRect(a: Rect, size: { w: number; h: number }, area: { w: number; h: number }, insets: PropertyPanelProps['insets']): { x: number; y: number } {
  const minX = insets.left + 8, maxX = area.w - insets.right - 8, minY = insets.top + 8, maxY = area.h - insets.bottom - 8
  const clampY = (y: number) => Math.max(minY, Math.min(y, maxY - size.h))
  const clampX = (x: number) => Math.max(minX, Math.min(x, maxX - size.w))
  if (a.x + a.w + GAP + size.w <= maxX) return { x: a.x + a.w + GAP, y: clampY(a.y) }
  if (a.x - GAP - size.w >= minX) return { x: a.x - GAP - size.w, y: clampY(a.y) }
  if (a.y + a.h + GAP + size.h <= maxY) return { x: clampX(a.x), y: a.y + a.h + GAP }
  if (a.y - GAP - size.h >= minY) return { x: clampX(a.x), y: a.y - GAP - size.h }
  return { x: clampX(maxX - size.w), y: clampY(minY) }
}

export function PropertyPanel(props: PropertyPanelProps) {
  const { subject, context, mode, area, insets, pinned, phone } = props
  const store = useStore()
  const { t } = useAiwsText()
  const ref = useRef<HTMLDivElement>(null)
  const [height, setHeight] = useState(240)
  const [drag, setDrag] = useState<{ x: number; y: number } | null>(null)
  const [full, setFull] = useState(false)
  const definition = props.resolved ? context?.definition : undefined
  const typeKey = subject ? subject.view_type ?? subject.kind ?? subject.type_id : 'page'
  const sectionsKey = `ui:props-sections:${typeKey}`
  const remembered = useUserState<SectionState>(sectionsKey) ?? {}
  const collapsed = (id: string) => remembered[id] ?? id === 'diagnostics'
  const toggle = (id: string) => store.userState.set(sectionsKey, { ...remembered, [id]: !collapsed(id) })
  const maxHeight = Math.max(160, Math.min(area.h - insets.top - insets.bottom - 16, MAX_HEIGHT))
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const measure = () => setHeight((h) => (Math.abs(el.offsetHeight - h) > 1 ? el.offsetHeight : h))
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(el)
    return () => observer.disconnect()
  }, [])
  // ---- where it stands
  const size = { w: Math.min(WIDTH, area.w - 16), h: Math.min(height, maxHeight) }
  const clampPin = (at: { x: number; y: number }) => ({ x: Math.max(8, Math.min(at.x, area.w - size.w - 8)), y: Math.max(insets.top + 4, Math.min(at.y, area.h - size.h - 8)) })
  const pinnedAt = clampPin(drag ?? props.pinAt ?? { x: area.w - insets.right - size.w - 8, y: insets.top + 8 })
  const at = pinned ? pinnedAt : props.anchor ? besideRect(props.anchor, size, area, insets) : pinnedAt
  const style: CSSProperties | undefined = phone ? undefined : { left: Math.round(at.x), top: Math.round(at.y), width: size.w, maxHeight }
  // a pinned panel is moved by its title bar
  const startDrag = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!pinned || phone || event.button !== 0 || (event.target as HTMLElement).closest('button')) return
    event.preventDefault()
    const head = event.currentTarget
    const start = { x: event.clientX, y: event.clientY, at: pinnedAt }
    head.setPointerCapture(event.pointerId)
    let last = start.at
    const move = (e: PointerEvent) => { last = clampPin({ x: start.at.x + e.clientX - start.x, y: start.at.y + e.clientY - start.y }); setDrag(last) }
    const end = () => {
      head.removeEventListener('pointermove', move)
      head.removeEventListener('pointerup', end)
      head.removeEventListener('pointercancel', end)
      setDrag(null)
      if (last !== start.at) props.onMove({ x: Math.round(last.x), y: Math.round(last.y) })
    }
    head.addEventListener('pointermove', move)
    head.addEventListener('pointerup', end)
    head.addEventListener('pointercancel', end)
  }
  const Icon = definition?.icon ?? (subject ? controlRegistry.get(subject.kind)?.icon : FileText) ?? SlidersHorizontal
  const typeTitle = subject ? definition?.title ?? controlRegistry.get(subject.kind)?.title ?? subject.kind ?? subject.view_type ?? '' : t('aiworkspace.blockui.page')
  const name = subject ? subject.title ?? context?.payload.title ?? '' : props.surface.title ?? props.surface.name ?? ''
  const reason = subject ? context?.readOnlyReason ?? null : surfaceWriteReason(store, props.surface, mode !== 'edit', null)
  return (
    <div ref={ref} className={`aiws-panel aiws-props-panel${phone ? ' is-sheet' : ''}${phone && full ? ' is-full' : ''}`} role="dialog" aria-label={t('aiworkspace.canvas.inspector.title')}
      data-testid="aiws-props-panel" data-pinned={pinned ? 'true' : 'false'} data-subject={subject?.entity_id ?? 'page'} data-mode={mode} style={style}
      onPointerDown={(event) => event.stopPropagation()} onKeyDown={(event) => { if (event.key === 'Escape' && !event.defaultPrevented) { event.stopPropagation(); props.onClose() } }}>
      <div className={`aiws-props-head${pinned && !phone ? ' is-draggable' : ''}`} data-testid="aiws-props-head" onPointerDown={startDrag}>
        <Icon size={16} aria-hidden="true" />
        <div className="aiws-props-heading"><b>{t('aiworkspace.canvas.inspector.title')}</b><span className="aiws-muted">{name ? `${typeTitle} · ${name}` : typeTitle}</span></div>
        {phone && <button type="button" className="aiws-tool aiws-tool-small" aria-label={t('aiworkspace.blockui.fullHeight')} title={t('aiworkspace.blockui.fullHeight')} aria-pressed={full} onClick={() => setFull(!full)}><ChevronsUp size={16} /></button>}
        {!phone && (
          <button type="button" className="aiws-tool aiws-tool-small" aria-pressed={pinned} aria-label={t(pinned ? 'aiworkspace.blockui.unpin' : 'aiworkspace.blockui.pin')} title={t(pinned ? 'aiworkspace.blockui.unpin' : 'aiworkspace.blockui.pin')}
            data-testid="aiws-props-pin" onClick={() => props.onPin(!pinned)}>{pinned ? <PinOff size={16} /> : <Pin size={16} />}</button>
        )}
        <button type="button" className="aiws-tool aiws-tool-small" aria-label={t('aiworkspace.blockui.close')} title={t('aiworkspace.blockui.close')} data-testid="aiws-props-close" onClick={props.onClose}><X size={16} /></button>
      </div>
      <div className="aiws-props-body" data-testid="aiws-inspector" data-mode={mode}>
        {reason && <div className="aiws-muted aiws-props-reason" data-testid="aiws-props-readonly">{reason}</div>}
        {subject
          ? <BlockSections {...props} subject={subject} collapsed={collapsed} toggle={toggle} />
          : <PageSections surface={props.surface} mode={mode} collapsed={collapsed} toggle={toggle} />}
      </div>
    </div>
  )
}

interface SectionsProps { collapsed: (id: string) => boolean; toggle: (id: string) => void }

function BlockSections({ subject: cell, count, context, resolved, registryVersion, mode, isFree, layout, collapsed, toggle }: PropertyPanelProps & SectionsProps & { subject: EntityEnvelope }) {
  const store = useStore()
  const ui = useWorkspaceUi()
  const { t, tNode } = useAiwsText()
  const live = context?.cell ?? cell
  const definition = resolved ? context?.definition : undefined
  const source = context?.source
  const payload = context?.payload
  const keyRevs = context?.keyRevs ?? {}
  const readOnly = !context || context.readOnlyReason !== null
  const section = (id: string, title: string, body: ReactNode) => (body ? <Section key={id} id={id} title={title} collapsed={collapsed(id)} onToggle={() => toggle(id)}>{body}</Section> : null)
  const setTitle = (title: string) => {
    if (!payload || readOnly || title === (payload.title ?? '')) return
    void store.submit({ editId: `key:${cell.entity_id}:title`, label: title ? t('aiworkspace.canvas.blockTitleLabel', { title }) : t('aiworkspace.canvas.blockTitleClearedLabel'), operations: [title ? { op: 'entity.set_keys', entity_id: cell.entity_id, keys: [{ key: 'title', value: title, expect: { rev: keyRevs.title ?? 0 } }] } : { op: 'entity.unset_keys', entity_id: cell.entity_id, keys: [{ key: 'title', expect: { rev: keyRevs.title ?? 0 } }] }] })
  }
  if (count > 1) {
    return (
      <>
        <div className="aiws-muted" data-testid="aiws-props-multi">{t('aiworkspace.blockui.multiple', { count })}</div>
        {layout?.setLocked && section('layout', t('aiworkspace.blockui.section.layout'), (
          <label className="aiws-props-field is-boolean"><span className="aiws-props-label">{t('aiworkspace.canvas.lock.lock')}</span>
            <input type="checkbox" checked={Boolean(layout.laid?.locked)} data-testid="aiws-props-locked" onChange={(e) => layout.setLocked?.(e.target.checked)} /></label>
        ))}
      </>
    )
  }
  const fields = fieldsOf(definition, context)
  const typeSections = definition?.sections ?? (definition ? [{ id: 'type', label: definition.title }] : [])
  const firstSection = typeSections[0]?.id ?? 'type'
  const instance = isInstance(cell, store)
  const instanceReason = mode === 'edit' && cell.capabilities.includes('update') ? null : t('aiworkspace.blocks.noPermission')
  const control = controlRegistry.get(cell.kind)
  const Inspector = definition?.Inspector
  const operable = cell.view_type !== 'frame' && cell.view_type !== 'connector' && cell.type_id === 'buckyos.cell'
  // a group with a backing is placed as the group (its frame or shape covers it)
  const placement = layout?.laid?.entity.placement ?? live.placement
  const numeric = placement && ['x', 'y', 'w', 'h'].every((k) => num(placement[k as 'x']) !== undefined)
  const layoutReason = layout?.reason ?? null
  const place = (key: 'x' | 'y' | 'w' | 'h' | 'rotation', value: string) => {
    const n = Number(value)
    if (!layout || !placement || layoutReason || value.trim() === '' || !Number.isFinite(n)) return
    const next: StoredPlacement = { ...placement, [key]: key === 'w' || key === 'h' ? Math.max(1, Math.round(n)) : key === 'rotation' ? ((Math.round(n) % 360) + 360) % 360 : Math.round(n) }
    if (key === 'rotation' && next.rotation === 0) delete next.rotation
    layout.place(next)
  }
  return (
    <>
      {section('common', t('aiworkspace.blockui.section.common'), payload && (
        <>
          <Field label={t('aiworkspace.canvas.inspector.titleField')} value={payload.title ?? ''} disabled={readOnly} testId="aiws-props-title" onCommit={(v) => setTitle(v.trim())} />
          {definition && fields.filter((f) => f.section === 'common').map((field) => <PropertyControl key={field.key} definition={definition} field={field} context={context!} store={store} />)}
        </>
      ))}
      {definition && context && typeSections.map((s) => {
        const own = fields.filter((f) => f.section !== 'common' && (f.section ?? firstSection) === s.id)
        const body = (own.length > 0 || (instance && s.id === firstSection)) && (
          <>
            {own.map((field) => <PropertyControl key={field.key} definition={definition} field={field} context={context} store={store} />)}
            {instance && s.id === firstSection && <InstanceConfig entity={cell} readOnly={instanceReason} />}
          </>
        )
        return section(`type-${s.id}`, s.label, body)
      })}
      {!definition && instance && section('type-type', t('aiworkspace.blockui.section.type'), <InstanceConfig entity={cell} readOnly={instanceReason} />)}
      {Inspector && context && section('drawn', definition?.inspectorTitle ?? definition?.title ?? '', (
        <BlockBoundary resetKey={`${cell.entity_id}:${mode}:${live.content_rev}:${registryVersion}`} fallback={(error, reset) => (
          <div className="aiws-block-fallback" data-testid="aiws-inspector-error"><div className="aiws-error" role="alert">{t('aiworkspace.canvas.inspector.error', { reason: describeError(error) })}</div><button type="button" onClick={reset}>{t('aiworkspace.common.retry')}</button></div>
        )}>
          <Inspector {...context} />
        </BlockBoundary>
      ))}
      {control?.Inspector && section('control', control.title, <control.Inspector containerId={cell.entity_id} readOnly={instanceReason} />)}
      {section('data', t('aiworkspace.blockui.section.data'), (source || instance || cell.def_id) && (
        <>
          {source && <div className="aiws-muted">{tNode('aiworkspace.canvas.inspector.source', { name: <button type="button" className="aiws-link" data-testid="aiws-props-open-source" onClick={() => ui.openEntity(source.entity_id)}>{source.title ?? source.name ?? source.entity_id}</button>, type: source.type_id.replace('buckyos.', '') })}</div>}
          {instance ? <InstanceData entity={cell} readOnly={instanceReason} />
            : cell.def_id && <button type="button" className="aiws-link" onClick={() => ui.openEntity(cell.def_id!)}>{t('aiworkspace.ext.composite.openDefinition')}</button>}
        </>
      ))}
      {isFree && layout && section('layout', t('aiworkspace.blockui.section.layout'), (
        <>
          {numeric ? (
            <div className="aiws-props-grid">
              {([['x', t('aiworkspace.blockui.layout.x')], ['y', t('aiworkspace.blockui.layout.y')], ['w', t('aiworkspace.blockui.layout.w')], ['h', t('aiworkspace.blockui.layout.h')]] as const).map(([k, label]) => (
                <Field key={k} kind="number" label={label} value={String(num(placement?.[k]) ?? '')} disabled={layoutReason !== null} testId={`aiws-props-layout-${k}`} onCommit={(v) => place(k, v)} />
              ))}
              {!layout.laid?.isGroup && <Field kind="number" label={t('aiworkspace.blockui.layout.rotation')} value={String(placement?.rotation ?? 0)} disabled={layoutReason !== null} testId="aiws-props-layout-rotation" onCommit={(v) => place('rotation', v)} />}
            </div>
          ) : <div className="aiws-muted">{t('aiworkspace.blockui.layout.auto')}</div>}
          {layoutReason && <div className="aiws-muted">{layoutReason}</div>}
          {layout.setLocked && (
            <label className="aiws-props-field is-boolean"><span className="aiws-props-label">{t('aiworkspace.canvas.lock.lock')}</span>
              <input type="checkbox" checked={Boolean(layout.laid?.locked ?? cell.locked)} data-testid="aiws-props-locked" onChange={(e) => layout.setLocked?.(e.target.checked)} /></label>
          )}
        </>
      ))}
      {section('interaction', t('aiworkspace.blockui.section.interaction'), (payload && cell.type_id === 'buckyos.cell') || instance ? (
        <>
          {payload && cell.type_id === 'buckyos.cell' && <MetadataForm config={payload.config ?? {}} readOnly={readOnly} onChange={(patch, label) => { if (!readOnly && context) setCellConfig(context, store, patch, label) }} />}
          {instance && <InstanceEvents entity={cell} readOnly={instanceReason} />}
        </>
      ) : null)}
      {section('presentation', t('aiworkspace.blockui.section.presentation'), operable && payload && (
        <label className="aiws-props-field is-boolean" title={t('aiworkspace.canvas.inspector.liveTip')}>
          <span className="aiws-props-label">{t('aiworkspace.canvas.inspector.live')}</span>
          <input type="checkbox" data-testid="aiws-inspector-live" checked={payload.presentation?.live === true} disabled={readOnly}
            onChange={(event) => { void setLive(store, cell.entity_id, event.target.checked) }} />
        </label>
      ))}
      {section('diagnostics', t('aiworkspace.blockui.section.diagnostics'), (
        <div className="aiws-props-diag" data-testid="aiws-props-diagnostics">
          <div>{t('aiworkspace.blockui.diag.entity')} <code>{cell.entity_id}</code></div>
          {cell.source_id && <div>{t('aiworkspace.blockui.diag.source')} <code>{cell.source_id}</code></div>}
          <div>{t('aiworkspace.blockui.diag.renderer')} <code>{live.view_type ?? live.kind ?? live.type_id}{live.view_version ? ` v${live.view_version}` : ''}</code></div>
          {live.placement && <div className="aiws-muted">{t('aiworkspace.canvas.inspector.placement', { x: String(live.placement.x), y: String(live.placement.y), w: String(live.placement.w), h: String(live.placement.h), order: live.order_key ?? '' })}</div>}
        </div>
      ))}
    </>
  )
}

/** The page's own properties (no Block selected): its name and background (draw.io 导入 TODO §2.5). */
function PageSections({ surface, mode, collapsed, toggle }: { surface: EntityEnvelope; mode: CanvasMode } & SectionsProps) {
  const store = useStore()
  const { t } = useAiwsText()
  const look = useSurfaceConfig(surface.entity_id)
  const reason = surfaceWriteReason(store, surface, mode !== 'edit', null)
  const title = surface.title ?? surface.name ?? ''
  const rename = useCallback((next: string) => {
    const value = next.trim()
    if (!value || value === title || reason) return
    void renameBase(store, surface).then((base) => renameSurface(store, surface, value, base)).catch((error: unknown) => store.notify('error', describeError(error)))
  }, [store, surface, title, reason])
  const setBackground = (value: string | null) => { if (!reason) void setSurfaceBackground(store, surface, value).catch((error: unknown) => store.notify('error', describeError(error))) }
  const section = (id: string, label: string, body: ReactNode) => <Section key={id} id={id} title={label} collapsed={collapsed(id)} onToggle={() => toggle(id)}>{body}</Section>
  return (
    <>
      {section('common', t('aiworkspace.blockui.section.common'), <Field label={t('aiworkspace.blockui.pageName')} value={title} disabled={reason !== null} testId="aiws-props-page-title" onCommit={rename} />)}
      {section('look', t('aiworkspace.blockui.section.look'), (
        <div className="aiws-props-field is-color">
          <span className="aiws-props-label">{t('aiworkspace.blockui.background')}</span>
          <PaletteGrid label={t('aiworkspace.blockui.background')} palette="fill" extras={[{ value: '', label: t('aiworkspace.canvas.color.default') }]} value={look.background ?? ''} disabled={reason !== null} testId="aiws-props-page-background" onPick={(hex) => setBackground(hex || null)} />
        </div>
      ))}
      {section('diagnostics', t('aiworkspace.blockui.section.diagnostics'), (
        <div className="aiws-props-diag">
          <div>{t('aiworkspace.blockui.diag.entity')} <code>{surface.entity_id}</code></div>
          <div>{t(surface.layout?.mode === 'free' ? 'aiworkspace.shell.freeCanvas' : 'aiworkspace.shell.flowPage')}</div>
          {look.pageSize && <div>{t('aiworkspace.blockui.pageSize', { w: look.pageSize.w, h: look.pageSize.h })}</div>}
          {Object.keys(look.config).length > 0 && <pre className="aiws-props-raw">{JSON.stringify(look.config as Json, null, 2)}</pre>}
        </div>
      ))}
    </>
  )
}
