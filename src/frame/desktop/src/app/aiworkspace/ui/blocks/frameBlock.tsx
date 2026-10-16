/* eslint-disable react-refresh/only-export-components -- a Block definition and its swimlane helpers */
/* The frame (标准对象的交互改进 §4.4; draw.io 导入 TODO §2.2): a titled area. Without `header` it is the outlined
 * area with a title tab; with `header` it has a title bar of that thickness (0: a border only), on top or — with
 * `orientation: vertical` — down the left side, as draw.io swimlanes and tables have.
 *
 * Swimlanes are built from groups, no new container type (Q3): a lane is a group whose first child is a frame
 * covering it (its "backing", which the canvas selects and moves as the group); a pool is such a group whose other
 * children include lanes. "Add / delete lane" grow or shrink the pool and shift the lanes after it. */

import { useState, type CSSProperties } from 'react'
import { Pencil, Rows3, Trash2 } from 'lucide-react'
import { inkOn } from '@drawio/shapes'
import { keysAfter, randomId } from '@drawio/keys'
import type { EntityEnvelope, Json, Operation, Placement } from '../../api/types'
import { aiwsT } from '../../i18n'
import { useAiwsText } from '../../useAiwsText'
import { useStore } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { cellOp, setCellConfig, setCellKey } from './ops'
import type { BlockDefinition, CreateArgs, RenderContext, ToolbarItem } from './registry'

/** A one-line title typed where it is shown; Enter or leaving saves, Esc keeps the old one. */
export function InPlaceTitle({ context, className, placeholder }: { context: RenderContext; className: string; placeholder: string }) {
  const { t } = useAiwsText()
  const store = useStore()
  const [text, setText] = useState(context.payload.title ?? '')
  const done = (save: boolean) => {
    const next = text.trim()
    if (save && next !== (context.payload.title ?? '')) setCellKey(context, store, 'title', next || null, t('aiworkspace.blocks.edit.change', { label: t('aiworkspace.blocks.frame.titleLabel'), value: next || t('aiworkspace.blocks.edit.cleared') }))
    context.deactivateEditor()
  }
  return (
    <input className={className} aria-label={t('aiworkspace.blocks.frame.titleLabel')} data-testid={`aiws-title-edit-${context.cell.entity_id}`} autoFocus value={text} placeholder={placeholder} onChange={(event) => setText(event.target.value)}
      onBlur={() => done(true)} onKeyDown={(event) => { if (event.key === 'Enter') { event.preventDefault(); done(true) } else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); done(false) } }} />
  )
}

const HEX = /^#[0-9a-f]{6}$/i
const finite = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v)
const paint = (v: string) => (v === 'auto' ? 'var(--aiws-shape-auto-stroke)' : v)

interface FrameLook {
  color: string
  header: number | null
  vertical: boolean
  headerFill: string
  fill: string | null
  fontSize: number | null
  fontColor: string | null
  bold: boolean
  italic: boolean
  strokeWidth: number
  dash: 'solid' | 'dashed' | 'dotted'
  radius: number | null
}

export function frameLook(config: Record<string, unknown> | undefined | null): FrameLook {
  const c = config ?? {}
  const color = typeof c.color === 'string' && (HEX.test(c.color) || c.color === 'auto') ? c.color : 'var(--cp-accent)'
  const fill = typeof c.fill === 'string' && (HEX.test(c.fill) || c.fill === 'auto') ? c.fill : null
  const headerFill = typeof c.header_fill === 'string' && (HEX.test(c.header_fill) || c.header_fill === 'none' || c.header_fill === 'auto') ? c.header_fill : color
  return {
    color, header: finite(c.header) ? Math.max(0, Math.min(400, c.header)) : null, vertical: c.orientation === 'vertical', headerFill, fill,
    fontSize: finite(c.font_size) ? c.font_size : null, fontColor: typeof c.font_color === 'string' && HEX.test(c.font_color) ? c.font_color : null,
    bold: c.bold === true, italic: c.italic === true,
    strokeWidth: finite(c.stroke_width) ? Math.max(0, Math.min(24, c.stroke_width)) : 2,
    dash: c.dash === 'dashed' || c.dash === 'dotted' ? c.dash : 'solid', radius: finite(c.radius) ? Math.max(0, c.radius) : null,
  }
}

function FrameView({ context, editing }: { context: RenderContext; editing: boolean }) {
  const { t } = useAiwsText()
  const look = frameLook(context.payload.config)
  const id = context.cell.entity_id
  const title = editing ? <InPlaceTitle context={context} className="aiws-title-input" placeholder={t('aiworkspace.blocks.frame.title')} /> : <span>{context.payload.title ?? (look.header === null ? t('aiworkspace.blocks.frame.title') : '')}</span>
  if (look.header === null) {
    return (
      <div className="aiws-frame" style={{ borderColor: look.color }} data-testid={`aiws-frame-${id}`}>
        <div className={`aiws-frame-title${editing ? ' is-editing' : ''}`} style={{ background: look.color }}>{title}</div>
      </div>
    )
  }
  const headerBg = look.headerFill === 'none' ? 'transparent' : look.headerFill === 'auto' ? 'var(--aiws-shape-auto-fill)' : paint(look.headerFill)
  const ink = look.fontColor ?? (look.headerFill.startsWith('#') ? inkOn(look.headerFill) : null) ?? 'var(--cp-text)'
  const border = paint(look.color)
  const style: CSSProperties = {
    borderColor: border, borderWidth: look.strokeWidth, borderStyle: look.dash, borderRadius: look.radius ?? 0,
    background: look.fill ? (look.fill === 'auto' ? 'var(--aiws-shape-auto-fill)' : look.fill) : undefined,
  }
  const textStyle: CSSProperties = { color: ink, fontSize: look.fontSize ?? 12, fontWeight: look.bold ? 700 : undefined, fontStyle: look.italic ? 'italic' : undefined }
  return (
    <div className={`aiws-frame has-header${look.vertical ? ' is-vertical' : ''}`} style={style} data-testid={`aiws-frame-${id}`}>
      {look.header > 0 && (
        <div className={`aiws-frame-header${editing ? ' is-editing' : ''}`} style={{ ...(look.vertical ? { width: look.header } : { height: look.header }), background: headerBg, borderColor: border, ...textStyle }}>
          {title}
        </div>
      )}
    </div>
  )
}

const FrameStatic = (context: RenderContext) => <FrameView context={context} editing={false} />
const FrameEditor = (context: RenderContext) => <FrameView context={context} editing />

const canChange = (context: RenderContext): string | false => (context.capabilities.includes('update') && context.mode === 'edit' && context.readOnlyReason === null ? false : aiwsT('aiworkspace.blocks.noPermission'))

// ---- swimlanes

/** The frame covering a group as its first child (the group's look), if it has one. */
export function backingOf(store: WorkspaceStore, groupId: string): EntityEnvelope | undefined {
  const group = store.outline.get(groupId)
  if (!group || group.kind !== 'group') return undefined
  const first = store.outline.childrenOf(groupId).find((c) => c.type_id === 'buckyos.cell' || c.kind === 'group')
  if (!first || first.type_id !== 'buckyos.cell' || (first.view_type !== 'frame' && first.view_type !== 'shape')) return undefined
  const p = first.placement
  const g = group.placement
  if (!p || !g || Math.abs(p.x) > 1 || Math.abs(p.y) > 1 || Math.abs(p.w - g.w) > 1 || Math.abs(p.h - g.h) > 1) return undefined
  return first
}

/** The lanes of a pool: its child groups backed by a frame, in reading order. */
function lanesOf(store: WorkspaceStore, poolId: string, rows: boolean): EntityEnvelope[] {
  return store.outline.childrenOf(poolId).filter((c) => c.kind === 'group' && backingOf(store, c.entity_id)?.view_type === 'frame')
    .sort((a, b) => (rows ? (a.placement?.y ?? 0) - (b.placement?.y ?? 0) : (a.placement?.x ?? 0) - (b.placement?.x ?? 0)))
}

/** The pool a frame heads (its group holds lanes) or the pool its lane belongs to. */
function laneContext(store: WorkspaceStore, frameId: string): { pool: EntityEnvelope; lane: EntityEnvelope | null; rows: boolean } | null {
  const frame = store.outline.get(frameId)
  const group = frame?.parent_id ? store.outline.get(frame.parent_id) : undefined
  if (!frame || !group || backingOf(store, group.entity_id)?.entity_id !== frameId) return null
  if (lanesOf(store, group.entity_id, true).length > 0) return { pool: group, lane: null, rows: poolRows(store, group.entity_id) }
  const parent = group.parent_id ? store.outline.get(group.parent_id) : undefined
  if (parent && lanesOf(store, parent.entity_id, true).some((l) => l.entity_id === group.entity_id)) return { pool: parent, lane: group, rows: poolRows(store, parent.entity_id) }
  return null
}

/** Lanes stacked top to bottom (a pool titled down its left side) or side by side. */
function poolRows(store: WorkspaceStore, poolId: string): boolean {
  const lanes = lanesOf(store, poolId, true)
  const a = lanes[0]?.placement
  const b = lanes[1]?.placement
  if (a && b) return Math.abs(b.y - a.y) >= Math.abs(b.x - a.x)
  return a ? a.w >= a.h : true
}

const grow = (p: Placement, rows: boolean, d: number): Placement => (rows ? { ...p, h: Math.max(1, p.h + d) } : { ...p, w: Math.max(1, p.w + d) })
const shift = (p: Placement, rows: boolean, d: number): Placement => (rows ? { ...p, y: p.y + d } : { ...p, x: p.x + d })

async function addLane(store: WorkspaceStore, frameId: string) {
  const ctx = laneContext(store, frameId)
  if (!ctx) return
  const { pool, rows } = ctx
  const lanes = lanesOf(store, pool.entity_id, rows)
  const after = ctx.lane ?? lanes.at(-1)
  if (!after?.placement || !pool.placement) return
  const size = rows ? after.placement.h : after.placement.w
  const at = shift(after.placement, rows, size)
  const template = backingOf(store, after.entity_id)
  const read = template ? await store.session.read<{ payload: { config?: Record<string, Json> } }>(template.entity_id) : null
  const laneId = randomId('g')
  const siblings = store.outline.childrenOf(pool.entity_id)
  const index = siblings.findIndex((s) => s.entity_id === after.entity_id)
  const orderKey = store.core.order_key_between(siblings[index]?.order_key ?? undefined, siblings[index + 1]?.order_key ?? undefined)
  const ops: Operation[] = [
    { op: 'entity.create', entity_id: laneId, type_id: 'buckyos.container', parent_id: pool.entity_id, order_key: orderKey, placement: at, payload: { kind: 'group', layout: { mode: 'free' } } },
    { op: 'entity.create', entity_id: randomId('c'), type_id: 'buckyos.cell', parent_id: laneId, order_key: keysAfter(null, 1)[0], placement: { x: 0, y: 0, w: at.w, h: at.h }, payload: { view: { type: 'frame' }, title: aiwsT('aiworkspace.blocks.frame.lane', { n: lanes.length + 1 }), config: read?.content.payload.config ?? { color: '#4f8df7', header: 30, orientation: 'vertical' } } },
  ]
  // lanes after the new one move by its size; the pool and its frame grow by it
  for (const lane of lanes) if (lane.placement && (rows ? lane.placement.y : lane.placement.x) > (rows ? after.placement.y : after.placement.x)) ops.push({ op: 'tree.place', entity_id: lane.entity_id, placement: shift(lane.placement, rows, size) })
  ops.push({ op: 'tree.place', entity_id: pool.entity_id, placement: grow(pool.placement, rows, size) })
  const head = backingOf(store, pool.entity_id)
  if (head?.placement) ops.push({ op: 'tree.place', entity_id: head.entity_id, placement: grow(head.placement, rows, size) })
  void store.submit({ editId: `lane:${laneId}`, label: aiwsT('aiworkspace.blocks.frame.addLane'), operations: ops })
}

function deleteLane(store: WorkspaceStore, frameId: string) {
  const ctx = laneContext(store, frameId)
  const lane = ctx?.lane
  const at = lane?.placement
  const poolAt = ctx?.pool.placement
  if (!ctx || !lane || !at || !poolAt) return
  const { pool, rows } = ctx
  const lanes = lanesOf(store, pool.entity_id, rows)
  if (lanes.length <= 1) { store.notify('info', aiwsT('aiworkspace.blocks.frame.keepOneLane')); return }
  const size = rows ? at.h : at.w
  const descendants = store.outline.descendants(lane.entity_id)
  const ops: Operation[] = [{ op: 'entity.delete', entity_id: lane.entity_id, ...(descendants.length ? { subtree: { delete: descendants.map((d) => d.entity_id) } } : {}), expect: { rev: lane.life_rev } }]
  for (const other of lanes) if (other.placement && (rows ? other.placement.y : other.placement.x) > (rows ? at.y : at.x)) ops.push({ op: 'tree.place', entity_id: other.entity_id, placement: shift(other.placement, rows, -size) })
  ops.push({ op: 'tree.place', entity_id: pool.entity_id, placement: grow(poolAt, rows, -size) })
  const head = backingOf(store, pool.entity_id)
  if (head?.placement) ops.push({ op: 'tree.place', entity_id: head.entity_id, placement: grow(head.placement, rows, -size) })
  void store.submit({ editId: `lane:${lane.entity_id}:delete`, label: aiwsT('aiworkspace.blocks.frame.deleteLane'), operations: ops })
}

/** A pool with `n` lanes in one commit: rows (titles down the left, the default) or columns (titles on top). */
function swimlaneOps(args: CreateArgs, rows: boolean, n: number): Operation[] {
  const { w, h } = args.placement
  const header = 30
  const lanes = Math.min(8, Math.max(1, n))
  const color = '#4f8df7'
  const ops: Operation[] = [
    { op: 'entity.create', entity_id: args.cellId, type_id: 'buckyos.container', parent_id: args.parentId, order_key: args.orderKey, placement: args.placement, payload: { kind: 'group', layout: { mode: 'free' } } },
  ]
  const keys = keysAfter(null, lanes + 1)
  ops.push({ op: 'entity.create', entity_id: randomId('c'), type_id: 'buckyos.cell', parent_id: args.cellId, order_key: keys[0], placement: { x: 0, y: 0, w, h }, payload: { view: { type: 'frame' }, title: args.title || aiwsT('aiworkspace.blocks.frame.pool'), config: { color, header, ...(rows ? { orientation: 'vertical' } : {}), bold: true } } })
  for (let i = 0; i < lanes; i++) {
    const laneId = randomId('g')
    const placement = rows ? { x: header, y: i * Math.floor(h / lanes), w: w - header, h: Math.floor(h / lanes) } : { x: i * Math.floor(w / lanes), y: header, w: Math.floor(w / lanes), h: h - header }
    ops.push({ op: 'entity.create', entity_id: laneId, type_id: 'buckyos.container', parent_id: args.cellId, order_key: keys[i + 1], placement, payload: { kind: 'group', layout: { mode: 'free' } } })
    ops.push({ op: 'entity.create', entity_id: randomId('c'), type_id: 'buckyos.cell', parent_id: laneId, order_key: keysAfter(null, 1)[0], placement: { x: 0, y: 0, w: placement.w, h: placement.h }, payload: { view: { type: 'frame' }, title: aiwsT('aiworkspace.blocks.frame.lane', { n: i + 1 }), config: { color, header, ...(rows ? { orientation: 'vertical' } : {}), header_fill: '#dbe8ff' } } })
  }
  return ops
}

function frameToolbar(context: RenderContext, store: WorkspaceStore): ToolbarItem[] {
  const reason = canChange(context)
  const look = frameLook(context.payload.config)
  const lanes = laneContext(store, context.cell.entity_id)
  const structureReason: string | false = context.capabilities.includes('structure') || (lanes && store.outline.get(lanes.pool.entity_id)?.capabilities.includes('structure')) ? false : aiwsT('aiworkspace.blocks.frame.noStructure')
  const set = (patch: Record<string, Json | null>, label: string) => setCellConfig(context, store, patch, label)
  return [
    { kind: 'color', id: 'frame-color', label: aiwsT('aiworkspace.blocks.color'), palette: 'ink', value: typeof context.payload.config?.color === 'string' ? context.payload.config.color : undefined, disabled: reason, onPick: (value) => set({ color: value || null }, aiwsT('aiworkspace.blocks.frame.color')) },
    ...(look.header !== null ? [{ kind: 'color' as const, id: 'frame-header-fill', label: aiwsT('aiworkspace.blocks.frame.headerColor'), palette: 'fill' as const, extras: [{ value: 'none', label: aiwsT('aiworkspace.common.none') }], value: look.headerFill, disabled: reason, onPick: (value: string) => set({ header_fill: value || null }, aiwsT('aiworkspace.blocks.frame.headerColor')) }] : []),
    { kind: 'button', id: 'frame-rename', icon: Pencil, label: aiwsT('aiworkspace.common.rename'), disabled: reason, run: () => context.activateEditor() },
    ...(lanes ? [{ kind: 'button' as const, id: 'lane-add', icon: Rows3, label: aiwsT(lanes.lane ? (lanes.rows ? 'aiworkspace.blocks.frame.addLaneBelow' : 'aiworkspace.blocks.frame.addLaneRight') : 'aiworkspace.blocks.frame.addLane'), disabled: reason || structureReason, run: () => { void addLane(store, context.cell.entity_id) } }] : []),
    ...(lanes?.lane ? [{ kind: 'button' as const, id: 'lane-delete', icon: Trash2, label: aiwsT('aiworkspace.blocks.frame.deleteLane'), disabled: reason || structureReason, run: () => deleteLane(store, context.cell.entity_id) }] : []),
  ]
}

export const frameDefinition: BlockDefinition = {
  type: 'frame', version: 1, get title() { return aiwsT('aiworkspace.blocks.frame.title') }, accepts: [], allowNoSource: true, pureUi: true,
  defaultSize: { w: 600, h: 400 }, cost: { editor: false, html: false },
  Static: FrameStatic, Editor: FrameEditor, chrome: 'none',
  catalog: {
    group: 'layout', get description() { return aiwsT('aiworkspace.blocks.frame.description') }, needs: 'none', standard: true,
    variants: [{
      id: 'swimlane', get title() { return aiwsT('aiworkspace.blocks.frame.swimlane') }, group: 'layout', get description() { return aiwsT('aiworkspace.blocks.frame.swimlaneDescription') },
      config: { swimlane: 'rows', lanes: 2 }, size: { w: 760, h: 360 },
      options: [
        { key: 'swimlane', get label() { return aiwsT('aiworkspace.blocks.direction') }, choices: [
          { value: 'rows', get label() { return aiwsT('aiworkspace.blocks.frame.rows') } },
          { value: 'columns', get label() { return aiwsT('aiworkspace.blocks.frame.columns') } },
        ] },
        { key: 'lanes', get label() { return aiwsT('aiworkspace.blocks.frame.laneCount') }, choices: [1, 2, 3, 4, 5, 6].map((n) => ({ value: String(n), label: String(n) })) },
      ],
    }],
  },
  actions: [{ id: 'rename', get label() { return aiwsT('aiworkspace.common.rename') }, modes: ['edit'], needs: ['update'], run: (context) => context.activateEditor(), key: 'Enter' }],
  toolbar: frameToolbar,
  configFields: [
    { key: 'color', get label() { return aiwsT('aiworkspace.blocks.color') }, kind: 'color' },
    { key: 'header', get label() { return aiwsT('aiworkspace.blocks.frame.header') }, kind: 'number' },
    { key: 'orientation', get label() { return aiwsT('aiworkspace.blocks.frame.titlePosition') }, kind: 'select', options: [
      { value: 'horizontal', get label() { return aiwsT('aiworkspace.blocks.frame.titleTop') } },
      { value: 'vertical', get label() { return aiwsT('aiworkspace.blocks.frame.titleLeft') } },
    ] },
  ],
  create: (args) => {
    const c = args.config ?? {}
    if (c.swimlane === 'rows' || c.swimlane === 'columns') return swimlaneOps(args, c.swimlane === 'rows', Number(c.lanes) || 2)
    return [cellOp({ ...args, config: undefined }, 'frame', undefined, { config: { color: typeof c.color === 'string' ? c.color : '#4f8df7' } })]
  },
}
