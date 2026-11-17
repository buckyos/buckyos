/* The PDF Frame control (打印 TODO §3.3, T3): a built-in control like the to-do and the calendar, placed on any canvas and
 * edited there. Its box keeps its page shape (the width is the user's, the height follows); its header and footer are
 * its two bands, groups the Blocks of the header and the footer go into (placed there, dragged in, or added by its
 * actions); the content area between them is a placeholder. Its properties are the page shape, the orientation, the
 * band heights and the side margin; a new shape keeps the frame's scale. On a flow page it shows as a preview. */

import { Hash, PanelBottom, PanelTop, PanelTopBottomDashed, RectangleHorizontal, RectangleVertical } from 'lucide-react'
import { DEFAULT_FRAME_CONFIG, frameAreas, frameBox, naturalOrientation, PDF_FRAME_CONTROL, pdfFrameConfig, pdfFrameOperations, solvePdfFrame, type FrameBand, type FramePage, type PdfFrameConfig } from '@controls/pdfframe'
import { num } from '@controls/placement'
import { randomId } from '../../../api/ids'
import type { Json, Operation } from '../../../api/types'
import { aiwsT } from '../../../i18n'
import type { WorkspaceStore } from '../../../state/store'
import type { BlockAction, ConfigField, RenderContext } from '../../blocks/registry'
import { controlCommands } from '../commands'
import type { ContainerDefinition, ControlCommandContext } from '../registry'
import { PdfFrameFlow } from './Art'
import { PdfFrameChrome } from './Chrome'
import { orientationLabel, pageOptions } from './labels'
import { bandsOf, frameConfigOps, frameLayout, PDF_FRAME } from './model'

const LABEL_H = 24
const LABEL_W = 200

/** A label in `band` of the frame (the band created first when it is missing), one commit; selected, and edited unless
 * it already says something. */
function addBandLabel(ctx: ControlCommandContext, band: FrameBand, title: string, align: 'left' | 'center' | 'right') {
  const frame = ctx.store.outline.get(ctx.containerId)
  if (!frame || !ctx.canEdit) return
  const { box, areas, config } = frameLayout(frame)
  const ops: Operation[] = []
  let bandId = bandsOf(ctx.store.outline, frame.entity_id)[band]?.entity_id
  if (!bandId) {
    bandId = randomId('g')
    const r = areas[band]
    ops.push({ op: 'entity.create', entity_id: bandId, type_id: 'buckyos.container', parent_id: frame.entity_id, order_key: band === 'header' ? 'a' : 'b',
      placement: { x: 0, y: Math.round(r.y), w: Math.max(1, Math.round(r.w)), h: Math.max(1, Math.round(r.h)) }, payload: { kind: 'group', slot: band } })
  }
  // the first of the aligned places that nothing in the band covers yet (else the asked one)
  const margin = Math.min(config.margin, box.w / 2)
  const w = Math.max(40, Math.min(LABEL_W, box.w - 2 * margin))
  const y = Math.max(0, (areas[band].h - LABEL_H) / 2)
  const xs = { left: margin, center: (box.w - w) / 2, right: box.w - margin - w }
  const order = align === 'center' ? (['center', 'right', 'left'] as const) : align === 'right' ? (['right', 'center', 'left'] as const) : (['left', 'center', 'right'] as const)
  const taken = ctx.store.outline.childrenOf(bandId).map((e) => e.placement).filter((p) => p !== undefined)
  const free = (x: number) => taken.every((p) => { const px = num(p.x) ?? 0, pw = num(p.w) ?? 0; return x + w <= px || x >= px + pw })
  const at = order.find((a) => free(xs[a])) ?? align
  const x = xs[at]
  const last = ctx.store.outline.childrenOf(bandId).at(-1)?.order_key ?? undefined
  const id = randomId('c')
  ops.push({ op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: bandId, order_key: ctx.store.core.order_key_between(last, undefined),
    placement: { x: Math.round(x), y: Math.round(y), w: Math.round(w), h: LABEL_H },
    payload: { view: { type: 'label' }, ...(title ? { title } : {}), config: { align: at, font_size: 10 } } })
  void ctx.store.submit({ editId: `insert:${id}`, label: aiwsT(band === 'header' ? 'aiworkspace.pdfframe.addHeaderText' : 'aiworkspace.pdfframe.addFooterText'), operations: ops }).then((outcome) => {
    if (outcome.status !== 'accepted' && outcome.status !== 'saved_locally') return
    ctx.select([id])
    if (!title) ctx.edit(id)
  })
}

const run = (band: FrameBand, title: () => string, align: 'left' | 'center' | 'right') => (context: RenderContext) =>
  controlCommands.send({ kind: 'run', containerId: context.cell.entity_id, run: (ctx) => addBandLabel(ctx, band, title(), align) })

const actions: BlockAction[] = [
  { id: 'pdfframe-header-text', icon: PanelTop, get label() { return aiwsT('aiworkspace.pdfframe.addHeaderText') }, modes: ['edit'], needs: ['structure'], run: run('header', () => '', 'left') },
  { id: 'pdfframe-footer-text', icon: PanelBottom, get label() { return aiwsT('aiworkspace.pdfframe.addFooterText') }, modes: ['edit'], needs: ['structure'], run: run('footer', () => '', 'left') },
  { id: 'pdfframe-page-number', icon: Hash, primary: true, get label() { return aiwsT('aiworkspace.pdfframe.addPageNumber') }, modes: ['edit'], needs: ['structure'],
    run: run('footer', () => aiwsT('aiworkspace.pdfframe.pageNumberText'), 'center') },
]

const config = (context: RenderContext) => pdfFrameConfig(context.cell.config)
const configOps = (key: keyof PdfFrameConfig, parse: (value: Json | null) => Json | null = (v) => v) =>
  (context: RenderContext, store: WorkspaceStore, value: Json | null) => frameConfigOps(store, context.cell.entity_id, { [key]: parse(value) })
const length = (value: Json | null) => (value === null || value === '' || !Number.isFinite(Number(value)) ? null : Math.max(0, Math.round(Number(value))))
const custom = (value: Json | null) => (value === null || !(Number(value) > 0) ? null : Math.round(Number(value) * 10) / 10)
const BAND_PRESETS = [0, 24, 40, 56, 72, 96]

/** Another page shape: a frame keeps its orientation among shapes that come the same way (A and Letter, 16:9 and 4:3),
 * else it takes the new one's (a slide landscape, a sheet portrait). */
function pageOps(context: RenderContext, store: WorkspaceStore, value: Json | null): Promise<Operation[]> {
  const c = config(context)
  const page = (value ?? DEFAULT_FRAME_CONFIG.page) as FramePage
  const natural = naturalOrientation(page, { ...c })
  const orientation = natural === naturalOrientation(c.page, { ...c }) ? c.orientation : natural
  return frameConfigOps(store, context.cell.entity_id, { page, orientation })
}

const fields: ConfigField[] = [
  { key: 'page', id: 'pdfframe-page', section: 'page', get label() { return aiwsT('aiworkspace.pdfframe.pageShape') }, kind: 'select', quick: true, icon: PanelTopBottomDashed,
    get options() { return pageOptions() }, get: (context) => config(context).page, ops: pageOps, editLabel: () => aiwsT('aiworkspace.pdfframe.pageShape') },
  { key: 'orientation', id: 'pdfframe-orientation', section: 'page', get label() { return aiwsT('aiworkspace.pdfframe.orientation') }, kind: 'select', quick: true,
    get options() { return [{ value: 'portrait', label: orientationLabel('portrait'), icon: RectangleVertical }, { value: 'landscape', label: orientationLabel('landscape'), icon: RectangleHorizontal }] },
    get: (context) => config(context).orientation, ops: configOps('orientation'), editLabel: () => aiwsT('aiworkspace.pdfframe.orientation') },
  { key: 'custom_w', id: 'pdfframe-custom-w', section: 'page', get label() { return aiwsT('aiworkspace.pdfframe.customW') }, kind: 'number', min: 1, max: 10000,
    when: (context) => config(context).page === 'custom', get: (context) => config(context).custom_w, ops: configOps('custom_w', custom) },
  { key: 'custom_h', id: 'pdfframe-custom-h', section: 'page', get label() { return aiwsT('aiworkspace.pdfframe.customH') }, kind: 'number', min: 1, max: 10000,
    when: (context) => config(context).page === 'custom', get: (context) => config(context).custom_h, ops: configOps('custom_h', custom) },
  { key: 'header', id: 'pdfframe-header', section: 'bands', get label() { return aiwsT('aiworkspace.pdfframe.headerHeight') }, kind: 'number', quick: true, icon: PanelTop, min: 0, max: 20000, presets: BAND_PRESETS,
    get: (context) => config(context).header, ops: configOps('header', length) },
  { key: 'footer', id: 'pdfframe-footer', section: 'bands', get label() { return aiwsT('aiworkspace.pdfframe.footerHeight') }, kind: 'number', quick: true, icon: PanelBottom, min: 0, max: 20000, presets: BAND_PRESETS,
    get: (context) => config(context).footer, ops: configOps('footer', length) },
  { key: 'margin', id: 'pdfframe-margin', section: 'bands', get label() { return aiwsT('aiworkspace.pdfframe.margin') }, kind: 'number', min: 0, max: 20000,
    get: (context) => config(context).margin, ops: configOps('margin', length) },
]

export const pdfFrameControl: ContainerDefinition = {
  kind: PDF_FRAME, version: 1, get title() { return aiwsT('aiworkspace.pdfframe.title') }, icon: PanelTopBottomDashed,
  configSchema: {
    type: 'object', additionalProperties: false,
    properties: {
      page: { enum: ['A', 'Letter', '16:9', '4:3', 'custom'] }, orientation: { enum: ['portrait', 'landscape'] },
      custom_w: { type: 'number', exclusiveMinimum: 0, maximum: 10000 }, custom_h: { type: 'number', exclusiveMinimum: 0, maximum: 10000 },
      header: { type: 'number', minimum: 0, maximum: 20000 }, footer: { type: 'number', minimum: 0, maximum: 20000 }, margin: { type: 'number', minimum: 0, maximum: 20000 },
    },
  },
  children: { accepts: [], containers: true, slots: [{ id: 'header', accepts: ['container'] }, { id: 'footer', accepts: ['container'] }] },
  sizing: 'box',
  resize: { aspect: 'locked' },
  catalogGroup: 'layout',
  defaultSize: { w: 595, h: 842 },
  layout: { direction: 'bottom_up', algorithm: 'fixed', solve: solvePdfFrame },
  Chrome: PdfFrameChrome,
  Flow: PdfFrameFlow,
  // the frame, its two bands, the canvas's title in the header and the page number in the footer: the frame is selected
  create: (args) => {
    const headerId = randomId('g')
    const footerId = randomId('g')
    const c = DEFAULT_FRAME_CONFIG
    const areas = frameAreas(frameBox(undefined, c), c)
    const w = Math.min(LABEL_W, areas.content.w)
    const label = (band: FrameBand, parent: string, title: string, align: 'left' | 'center'): Operation => ({
      op: 'entity.create', entity_id: randomId('c'), type_id: 'buckyos.cell', parent_id: parent, order_key: 'a',
      placement: { x: Math.round(align === 'left' ? areas.content.x : areas.content.x + (areas.content.w - w) / 2), y: Math.round(Math.max(0, (areas[band].h - LABEL_H) / 2)), w: Math.round(w), h: LABEL_H },
      payload: { view: { type: 'label' }, title, config: { align, font_size: 10, muted: true } } })
    return [
      ...pdfFrameOperations({ id: args.cellId, parentId: args.parentId, orderKey: args.orderKey, x: args.placement.x, y: args.placement.y,
        title: args.title || aiwsT('aiworkspace.pdfframe.title'), headerId, footerId }) as Operation[],
      label('header', headerId, '{title}', 'left'),
      label('footer', footerId, aiwsT('aiworkspace.pdfframe.pageNumberText'), 'center'),
    ]
  },
  actions,
  configFields: fields,
  get sections() { return [{ id: 'page', label: aiwsT('aiworkspace.pdfframe.section.page') }, { id: 'bands', label: aiwsT('aiworkspace.pdfframe.section.bands') }] },
  agent: PDF_FRAME_CONTROL,
}

