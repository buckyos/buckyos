/* The two phase-two demos (phase two §11), migrated from the ai-canvas prototype as commit
 * sequences through the public interface: data in the data tree, Blocks on free Surfaces, the Mock
 * wish executor as a definition entity, and a hand-written declarative definition (extension
 * sample 2). Nothing is pre-executed: the wishes run explicitly from the UI. */

import { aiwsT, aiwsText, type AiwsText } from '../i18n'
import { mockWishDefOp } from '../ui/wish/mockWishDef'
import { unwrap, type AiwsClient } from './client'
import { PROTOCOL_VERSION, type Json, type Operation, type Placement, type WorkspaceSummary } from './types'

interface DemoCommit { message: string; operations: Operation[] }

function keys(n: number): string[] {
  // base-36 keys with gaps, never ending in 0
  return Array.from({ length: n }, (_, i) => `${(i + 1).toString(36)}z`)
}

function text(blockId: string, t: string, type: 'paragraph' | 'heading' = 'paragraph', level = 2): Json {
  return type === 'heading' ? { type: 'heading', attrs: { block_id: blockId, level }, content: [{ type: 'text', text: t }] } : { type: 'paragraph', attrs: { block_id: blockId }, content: [{ type: 'text', text: t }] }
}

function doc(lines: string[], prefix: string): Json {
  const content: Json[] = []
  lines.forEach((line, i) => {
    const h = /^(#{1,3})\s+(.*)$/.exec(line)
    if (h) content.push(text(`${prefix}-${i}`, h[2], 'heading', h[1].length))
    else if (line.trim()) content.push(text(`${prefix}-${i}`, line.replace(/^- /, '· ')))
  })
  return { type: 'doc', content }
}

function surface(id: string, title: string, key: string, mode: 'free' | 'flow' = 'free'): Operation[] {
  return [
    { op: 'entity.create', entity_id: `${id}-content`, type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: key, name: title, payload: { kind: 'folder', title, system: 'surface_content', surface_id: id } },
    { op: 'entity.create', entity_id: id, type_id: 'buckyos.container', parent_id: 'surfaces', order_key: key, name: title, payload: { kind: 'surface', layout: { mode }, title, content_folder_id: `${id}-content` } },
  ]
}

function cell(id: string, surfaceId: string, key: string, view: string, sourceId: string | undefined, placement: Placement, extra: Record<string, Json> = {}): Operation {
  return { op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: surfaceId, order_key: key, placement, payload: { view: { type: view }, ...(sourceId ? { source_ref: { entity_id: sourceId } } : {}), ...extra } }
}

function wish(id: string, parentId: string, key: string, title: string, prompt: string, inputs: string[], surfaceId: string, outputName: string): Operation {
  return { op: 'entity.create', entity_id: id, type_id: 'buckyos.wish', parent_id: parentId, order_key: key, name: title, payload: {
    title, prompt, executor: 'mock', output_mode: 'overwrite', inputs: inputs.map((entity_id) => ({ entity_id, version: { mode: 'follow' } })),
    output: { container_id: parentId, surface_id: surfaceId, name: outputName },
  } }
}

// ---- demo 1: quarterly business analysis (§11.1)
// Demo text is product content in the language at creation time: everything is read when the commits are built.

function salesRows(t: AiwsText['t']): [string, string, number, number, number, number][] {
  const east = t('aiworkspace.demo.quarterly.region.east'), south = t('aiworkspace.demo.quarterly.region.south')
  const north = t('aiworkspace.demo.quarterly.region.north'), southwest = t('aiworkspace.demo.quarterly.region.southwest')
  const watch = t('aiworkspace.demo.quarterly.product.watch'), earbuds = t('aiworkspace.demo.quarterly.product.earbuds'), speaker = t('aiworkspace.demo.quarterly.product.speaker')
  return [
    [east, watch, 128000, 76000, 120000, 112000], [east, earbuds, 96000, 58000, 100000, 101000], [east, speaker, 42000, 30000, 50000, 47000],
    [south, watch, 101000, 61000, 95000, 88000], [south, earbuds, 74000, 44000, 90000, 86000], [south, speaker, 23000, 25000, 40000, 31000],
    [north, watch, 88000, 55000, 100000, 92000], [north, earbuds, 66000, 40000, 70000, 61000], [north, speaker, 38000, 26000, 36000, 29000],
    [southwest, watch, 54000, 36000, 70000, 60000], [southwest, earbuds, 41000, 27000, 45000, 39000], [southwest, speaker, 19000, 14000, 20000, 18000],
  ]
}

export function quarterlyDemoCommits(): DemoCommit[] {
  const { t } = aiwsText()
  const k = keys(40)
  const salesName = t('aiworkspace.demo.quarterly.salesName')
  const introName = t('aiworkspace.demo.quarterly.introName')
  // the declarative definition binds by field name: the bindings use the same names as the fields
  const field = {
    region: t('aiworkspace.demo.quarterly.field.region'), product: t('aiworkspace.demo.quarterly.field.product'), revenue: t('aiworkspace.demo.quarterly.field.revenue'),
    cost: t('aiworkspace.demo.quarterly.field.cost'), target: t('aiworkspace.demo.quarterly.field.target'), prev: t('aiworkspace.demo.quarterly.field.prev'),
  }
  const revenueTotal = t('aiworkspace.demo.quarterly.kpi.revenueTotal')
  const sales: Operation = { op: 'entity.create', entity_id: 'sales', type_id: 'buckyos.table-source', parent_id: 'data', order_key: k[0], name: salesName, payload: {
    title_field_id: 'product', description: t('aiworkspace.demo.quarterly.salesDescription'),
    fields: [
      { field_id: 'region', name: field.region, type: 'text', required: true }, { field_id: 'product', name: field.product, type: 'text', required: true },
      { field_id: 'revenue', name: field.revenue, type: 'number' }, { field_id: 'cost', name: field.cost, type: 'number' }, { field_id: 'target', name: field.target, type: 'number' }, { field_id: 'prev', name: field.prev, type: 'number' },
    ],
  } }
  const rows: Operation = { op: 'table.insert_records', source_id: 'sales', records: salesRows(t).map(([region, product, revenue, cost, target, prev], i) => ({ record_id: `s-${i + 1}`, values: { region, product, revenue, cost, target, prev } })) }
  const intro: Operation = { op: 'entity.create', entity_id: 'intro', type_id: 'buckyos.richtext', parent_id: 'data', order_key: k[1], name: introName, payload: { content: doc([`# ${t('aiworkspace.demo.quarterly.introTitle')}`, t('aiworkspace.demo.quarterly.introText')], 'intro') } }
  const kpi: Operation = { op: 'entity.create', entity_id: 'def-kpi', type_id: 'buckyos.block-def', parent_id: 'data', order_key: k[2], name: 'kpi-card', payload: {
    def_id: 'sample.kpi-card', version: 1, kind: 'declarative', title: t('aiworkspace.demo.quarterly.kpi.title'), description: t('aiworkspace.demo.quarterly.kpi.description'),
    accepts: ['buckyos.table-source'], default_size: { w: 320, h: 240 },
    declarative: {
      layout: 'stack', accent: '#4f8df7',
      items: [{ kind: 'metric', label: revenueTotal, bind: `sum:${field.revenue}`, format: 'currency' }, { kind: 'metric', label: t('aiworkspace.demo.quarterly.kpi.costTotal'), bind: `sum:${field.cost}`, format: 'currency' }, { kind: 'bar', label: t('aiworkspace.demo.quarterly.kpi.byRegion'), bind: field.revenue, by: field.region, format: 'currency' }],
      view: { items: [{ kind: 'badge', text: t('aiworkspace.demo.quarterly.kpi.viewBadge') }, { kind: 'metric', label: revenueTotal, bind: `sum:${field.revenue}`, format: 'currency' }, { kind: 'list', label: t('aiworkspace.demo.quarterly.kpi.products'), bind: field.product, limit: 4 }] },
      actions: [{ id: 'open-source', label: t('aiworkspace.demo.quarterly.kpi.openSource'), command: 'open_source', modes: ['edit', 'view'] }, { id: 'open-def', label: t('aiworkspace.demo.quarterly.kpi.openDefinition'), command: 'open_definition', modes: ['view'] }],
      inspector: [{ key: 'accent', label: t('aiworkspace.demo.quarterly.kpi.accent'), kind: 'color', modes: ['edit'] }, { key: 'note', label: t('aiworkspace.demo.quarterly.kpi.note'), kind: 'text', modes: ['edit', 'view'] }],
    },
  } }
  const main = surface('sf-analysis', t('aiworkspace.demo.quarterly.surface.analysis'), k[0])
  const detail = surface('sf-detail', t('aiworkspace.demo.quarterly.surface.detail'), k[1])
  const wishOp = wish('wish-analysis', 'sf-analysis-content', k[0], t('aiworkspace.demo.quarterly.wish.title'), t('aiworkspace.demo.quarterly.wish.prompt', { sales: salesName }), ['sales'], 'sf-analysis', t('aiworkspace.demo.quarterly.wish.output'))
  return [
    { message: t('aiworkspace.demo.quarterly.commit.data'), operations: [sales, rows, intro] },
    { message: t('aiworkspace.demo.quarterly.commit.definitions'), operations: [mockWishDefOp(k[3]) as Operation, kpi] },
    { message: t('aiworkspace.demo.quarterly.commit.surfaces'), operations: [...main, ...detail] },
    { message: t('aiworkspace.demo.quarterly.commit.analysis'), operations: [
      wishOp,
      cell('blk-intro', 'sf-analysis', k[0], 'richtext', 'intro', { x: 40, y: 40, w: 900, h: 120 }, { title: introName }),
      cell('blk-sales', 'sf-analysis', k[1], 'table', 'sales', { x: 40, y: 190, w: 900, h: 420 }, { title: salesName, fields: [{ field_id: 'region', width: 90 }, { field_id: 'product', width: 120 }, { field_id: 'revenue', width: 110 }, { field_id: 'cost', width: 100 }, { field_id: 'target', width: 100 }, { field_id: 'prev', width: 130 }] }),
      cell('blk-wish', 'sf-analysis', k[2], 'wish', 'wish-analysis', { x: 980, y: 190, w: 420, h: 300 }, { title: t('aiworkspace.demo.quarterly.block.wish') }),
      cell('blk-kpi', 'sf-analysis', k[3], 'declarative', 'sales', { x: 980, y: 40, w: 420, h: 130 }, { title: t('aiworkspace.demo.quarterly.block.kpi'), def_ref: { entity_id: 'def-kpi' } }),
      cell('blk-frame', 'sf-analysis', k[4], 'frame', undefined, { x: 960, y: 520, w: 1100, h: 820 }, { title: t('aiworkspace.demo.quarterly.block.frame'), config: { color: '#4f8df7' } }),
    ] },
    { message: t('aiworkspace.demo.quarterly.commit.detail'), operations: [
      cell('blk-sales-2', 'sf-detail', k[0], 'table', 'sales', { x: 40, y: 40, w: 760, h: 360 }, { title: t('aiworkspace.demo.quarterly.block.eastDetail'), filter: { op: 'cmp', field_id: 'region', operator: 'eq', value: t('aiworkspace.demo.quarterly.region.east') } }),
      cell('blk-chart-2', 'sf-detail', k[1], 'sample.bar-chart', 'sales', { x: 840, y: 40, w: 420, h: 300 }, { title: t('aiworkspace.demo.quarterly.block.revenueByRegion'), config: { value: 'revenue', by: 'region' } }),
      cell('blk-metric-2', 'sf-detail', k[2], 'sample.metric', 'sales', { x: 840, y: 360, w: 220, h: 120 }, { title: revenueTotal, config: { field: 'revenue', aggregation: 'sum' } }),
    ] },
  ]
}

// ---- demo 2: AI short-film workflow (§11.2)

function scriptLines(t: AiwsText['t']): string[] {
  return [`# ${t('aiworkspace.demo.film.script.title')}`, t('aiworkspace.demo.film.script.logline'), `## ${t('aiworkspace.demo.film.script.scenes')}`,
    t('aiworkspace.demo.film.script.scene1'), t('aiworkspace.demo.film.script.scene2'), t('aiworkspace.demo.film.script.scene3'),
    t('aiworkspace.demo.film.script.scene4'), t('aiworkspace.demo.film.script.scene5'), t('aiworkspace.demo.film.script.scene6')]
}

function styleLines(t: AiwsText['t']): string[] {
  return [`# ${t('aiworkspace.demo.film.styleName')}`, t('aiworkspace.demo.film.style.summary'), `- ${t('aiworkspace.demo.film.style.night')}`, `- ${t('aiworkspace.demo.film.style.characters')}`]
}

function characterRows(t: AiwsText['t']): [string, string, string, string, string, string][] {
  return [
    [t('aiworkspace.demo.film.che.name'), t('aiworkspace.demo.film.che.role'), t('aiworkspace.demo.film.che.look'), t('aiworkspace.demo.film.che.personality'), t('aiworkspace.demo.film.che.color'), t('aiworkspace.demo.film.che.outfit')],
    [t('aiworkspace.demo.film.xiaoman.name'), t('aiworkspace.demo.film.xiaoman.role'), t('aiworkspace.demo.film.xiaoman.look'), t('aiworkspace.demo.film.xiaoman.personality'), t('aiworkspace.demo.film.xiaoman.color'), t('aiworkspace.demo.film.xiaoman.outfit')],
    [t('aiworkspace.demo.film.zhou.name'), t('aiworkspace.demo.film.zhou.role'), t('aiworkspace.demo.film.zhou.look'), t('aiworkspace.demo.film.zhou.personality'), t('aiworkspace.demo.film.zhou.color'), t('aiworkspace.demo.film.zhou.outfit')],
  ]
}

export function filmDemoCommits(): DemoCommit[] {
  const { t } = aiwsText()
  const k = keys(40)
  const scriptName = t('aiworkspace.demo.film.scriptName')
  const charactersName = t('aiworkspace.demo.film.charactersName')
  const styleName = t('aiworkspace.demo.film.styleName')
  const howtoName = t('aiworkspace.demo.film.howtoName')
  const sheetsName = t('aiworkspace.demo.film.wish1.output')
  const storyboardName = t('aiworkspace.demo.film.wish2.output')
  const wish1Title = t('aiworkspace.demo.film.wish1.title')
  const wish2Title = t('aiworkspace.demo.film.wish2.title')
  const wish3Title = t('aiworkspace.demo.film.wish3.title')
  const script: Operation = { op: 'entity.create', entity_id: 'script', type_id: 'buckyos.richtext', parent_id: 'data', order_key: k[0], name: scriptName, payload: { content: doc(scriptLines(t), 'script') } }
  const characters: Operation = { op: 'entity.create', entity_id: 'characters', type_id: 'buckyos.table-source', parent_id: 'data', order_key: k[1], name: charactersName, payload: {
    title_field_id: 'name', fields: [{ field_id: 'name', name: t('aiworkspace.demo.film.field.name'), type: 'text', required: true }, { field_id: 'role', name: t('aiworkspace.demo.film.field.role'), type: 'text' }, { field_id: 'look', name: t('aiworkspace.demo.film.field.look'), type: 'text' }, { field_id: 'personality', name: t('aiworkspace.demo.film.field.personality'), type: 'text' }, { field_id: 'color', name: t('aiworkspace.demo.film.field.color'), type: 'text' }, { field_id: 'outfit', name: t('aiworkspace.demo.film.field.outfit'), type: 'text' }],
  } }
  const rows: Operation = { op: 'table.insert_records', source_id: 'characters', records: characterRows(t).map(([name, role, look, personality, color, outfit], i) => ({ record_id: `ch-${i + 1}`, values: { name, role, look, personality, color, outfit } })) }
  const style: Operation = { op: 'entity.create', entity_id: 'style', type_id: 'buckyos.richtext', parent_id: 'data', order_key: k[2], name: styleName, payload: { content: doc(styleLines(t), 'style') } }
  const howto: Operation = { op: 'entity.create', entity_id: 'howto', type_id: 'buckyos.richtext', parent_id: 'data', order_key: k[3], name: howtoName, payload: { content: doc([`# ${t('aiworkspace.demo.film.howto.title')}`, t('aiworkspace.demo.film.howto.text')], 'howto') } }
  const sf = surface('sf-film', t('aiworkspace.demo.film.surface'), k[0])
  const w1 = wish('wish-characters', 'sf-film-content', k[0], wish1Title, t('aiworkspace.demo.film.wish1.prompt', { characters: charactersName, style: styleName }), ['characters', 'style'], 'sf-film', sheetsName)
  const w2 = wish('wish-storyboard', 'sf-film-content', k[1], wish2Title, t('aiworkspace.demo.film.wish2.prompt', { script: scriptName, sheets: sheetsName, style: styleName }), ['script', 'style', 'wish-characters-out-data'], 'sf-film', storyboardName)
  const w3 = wish('wish-video', 'sf-film-content', k[2], wish3Title, t('aiworkspace.demo.film.wish3.prompt', { storyboard: storyboardName }), ['wish-storyboard-out-data'], 'sf-film', t('aiworkspace.demo.film.wish3.output'))
  // the second and third wishes reference result folders that do not exist until the previous stage ran: the
  // references are declared after the folders exist (see below), so the fixture declares them without inputs first
  const stripInputs = (op: Operation) => ({ ...op, payload: { ...(op.payload as Record<string, Json>), inputs: ((op.payload as { inputs: { entity_id: string }[] }).inputs).filter((i) => !i.entity_id.startsWith('wish-')) } })
  return [
    { message: t('aiworkspace.demo.film.commit.data'), operations: [script, characters, rows, style, howto] },
    { message: t('aiworkspace.demo.film.commit.definition'), operations: [mockWishDefOp(k[4]) as Operation] },
    { message: t('aiworkspace.demo.film.commit.canvas'), operations: [
      ...sf, w1, stripInputs(w2), stripInputs(w3),
      cell('blk-howto', 'sf-film', k[0], 'richtext', 'howto', { x: 60, y: 40, w: 1280, h: 130 }, { title: howtoName }),
      cell('blk-frame-in', 'sf-film', k[1], 'frame', undefined, { x: 40, y: 200, w: 600, h: 900 }, { title: t('aiworkspace.demo.film.frame.input'), config: { color: '#4f8df7' } }),
      cell('blk-script', 'sf-film', k[2], 'richtext', 'script', { x: 60, y: 240, w: 560, h: 380 }, { title: scriptName }),
      cell('blk-characters', 'sf-film', k[3], 'table', 'characters', { x: 60, y: 640, w: 560, h: 220 }, { title: charactersName }),
      cell('blk-style', 'sf-film', k[4], 'richtext', 'style', { x: 60, y: 880, w: 560, h: 180 }, { title: styleName }),
      cell('blk-frame-a', 'sf-film', k[5], 'frame', undefined, { x: 680, y: 200, w: 1300, h: 900 }, { title: t('aiworkspace.demo.film.frame.sheets'), config: { color: '#22c55e' } }),
      cell('blk-wish-1', 'sf-film', k[6], 'wish', 'wish-characters', { x: 700, y: 240, w: 420, h: 300 }, { title: wish1Title }),
      cell('blk-frame-b', 'sf-film', k[7], 'frame', undefined, { x: 2020, y: 200, w: 1500, h: 1100 }, { title: t('aiworkspace.demo.film.frame.storyboard'), config: { color: '#f59e0b' } }),
      cell('blk-wish-2', 'sf-film', k[8], 'wish', 'wish-storyboard', { x: 2040, y: 240, w: 420, h: 300 }, { title: wish2Title }),
      cell('blk-frame-c', 'sf-film', k[9], 'frame', undefined, { x: 3560, y: 200, w: 1200, h: 900 }, { title: t('aiworkspace.demo.film.frame.video'), config: { color: '#ef4444' } }),
      cell('blk-wish-3', 'sf-film', k[10], 'wish', 'wish-video', { x: 3580, y: 240, w: 420, h: 300 }, { title: wish3Title }),
    ] },
  ]
}

/** A template stopped after its workspace was created: the partial workspace exists and is named here. */
export class TemplateFailure extends Error {
  readonly workspace: WorkspaceSummary
  readonly stage: string
  constructor(workspace: WorkspaceSummary, stage: string, cause: unknown) {
    super(aiwsT('aiworkspace.core.template.failure', { stage, error: cause instanceof Error ? cause.message : String(cause) }), { cause })
    this.name = 'TemplateFailure'
    this.workspace = workspace
    this.stage = stage
  }
}

export type DemoKind = 'quarterly' | 'film'

export async function createDemoWorkspace(client: AiwsClient, kind: DemoKind, title: string): Promise<WorkspaceSummary> {
  const commits = kind === 'quarterly' ? quarterlyDemoCommits() : filmDemoCommits()
  const workspace = unwrap(await client.wsCreate(title))
  for (const [index, commit] of commits.entries()) {
    try {
      const result = await client.commit({
        protocol_version: PROTOCOL_VERSION, workspace_id: workspace.workspace_id, epoch: workspace.epoch,
        idempotency_key: `demo/${kind}/${index + 1}`, session_id: 'demo', origin: 'human', message: commit.message, operations: commit.operations,
      })
      if (result.status !== 'accepted') throw new Error(aiwsT('aiworkspace.core.template.commitRejected', { code: result.code, result: JSON.stringify(result) }))
    } catch (error) {
      throw new TemplateFailure(workspace, aiwsT('aiworkspace.core.template.stepFailed', { step: index + 1, total: commits.length }), error)
    }
  }
  return workspace
}
