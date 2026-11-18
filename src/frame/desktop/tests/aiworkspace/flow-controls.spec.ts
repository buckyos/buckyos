/* UI 框架 PRD §6.2, §10.4 (third pass, §18.3): a control on a flow page has a titled Block's title row — grip, the type's
 * icon and its title, its primary actions (two at most), its quick properties, ⋯ (the Block's whole menu) — for the
 * built-in controls (a mind map, a to-do) and the extensions' (an HTML control, a composite). On a computer the tools
 * show on hover or focus, on an emulated Pixel 7 always. The to-do's own ⋯ (Q39) is this row's ⋯ on a page: one, not
 * two. Real backend; §15 scenario 17 (flow half). */

import { deflateSync } from 'node:zlib'
import { devices, type Locator, type Page } from '@playwright/test'
import { expect, openWorkspacePage, test, type Api } from './fixtures'

const ALICE = 'tok-alice'
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any
type Ws = { workspace_id: string; epoch: string }

const CRC = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0 })
function crc32(b: Buffer): number { let c = 0xffffffff; for (const x of b) c = CRC[(c ^ x) & 0xff] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0 }
function chunk(type: string, data: Buffer): Buffer {
  const len = Buffer.alloc(4); len.writeUInt32BE(data.length)
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data])
  const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(body))
  return Buffer.concat([len, body, crc])
}
/** A small PNG of one colour. */
function png(r: number, g: number, b: number, w = 24, h = 16): Buffer {
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2
  const rows = Buffer.alloc((w * 3 + 1) * h)
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) rows.set([r, g, b], y * (w * 3 + 1) + 1 + x * 3)
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(rows)), chunk('IEND', Buffer.alloc(0))])
}

const TODO_FIELDS = [
  { field_id: 'title', name: 'Title', type: 'text', required: true },
  { field_id: 'status', name: 'Status', type: 'select', required: true, options: [{ option_id: 'todo', label: 'To do' }, { option_id: 'done', label: 'Done' }] },
  { field_id: 'due', name: 'Due', type: 'date', nullable: true },
  { field_id: 'order', name: 'Order', type: 'text', nullable: true },
  { field_id: 'done_at', name: 'Completed', type: 'datetime', nullable: true },
  { field_id: 'ext', name: 'Extension', type: 'json', nullable: true },
]
const AUTO = { x: 'auto', y: 'auto', w: 'auto', h: 'auto' }
const node = (id: string, parent: string, order: string, title: string) => ({ op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: parent, order_key: order, placement: AUTO, payload: { view: { type: 'mindnode' }, title } })

/** A flow page `page` holding a mind map `mm`, a to-do `tl` (two items, one done), the listen-together HTML control
 * `listen` and the photo-vote composite `vote` (two photos, an empty votes table). */
async function flowWorkspace(api: Api, title: string): Promise<Ws> {
  const ws = await api.rpc(ALICE, 'ws.create', { title }) as Ws
  const listen = (await api.build(ALICE, ws.workspace_id, 'control-examples/controls/listen-together')).def_entity_id
  const vote = (await api.build(ALICE, ws.workspace_id, 'control-examples/controls/photo-vote')).def_entity_id
  const red = (await api.uploadAsset(ALICE, ws.workspace_id, png(220, 60, 60))).object_id
  const blue = (await api.uploadAsset(ALICE, ws.workspace_id, png(60, 90, 220))).object_id
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'page-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a', payload: { kind: 'folder', system: 'surface_content', surface_id: 'page', title: '页' } },
    { op: 'entity.create', entity_id: 'page', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a', payload: { kind: 'surface', layout: { mode: 'flow' }, title: '页', content_folder_id: 'page-content' } },
    { op: 'entity.create', entity_id: 'mm', type_id: 'buckyos.container', parent_id: 'page', order_key: 'a', payload: { kind: 'mindmap', layout: { mode: 'tree' }, title: '计划', config: { direction: 'right', edge: 'curve' } } },
    node('mroot', 'mm', 'a', '计划'), node('n1', 'mroot', 'a', '市场'), node('n2', 'mroot', 'b', '产品'),
    { op: 'entity.create', entity_id: 'tasks', type_id: 'buckyos.table-source', schema_id: 'buckyos.todo@1', parent_id: 'page-content', order_key: 'b', name: 'Tasks', payload: { title_field_id: 'title', fields: TODO_FIELDS } },
    { op: 'entity.create', entity_id: 'tl', type_id: 'buckyos.container', parent_id: 'page', order_key: 'b', payload: { kind: 'todo', layout: { mode: 'data' }, title: 'Week', bindings: { items: { entity_id: 'tasks' } } } },
    { op: 'table.insert_records', source_id: 'tasks', records: [
      { record_id: 'a', values: { title: 'Write the report', status: 'todo', order: 'b' } },
      { record_id: 'd', values: { title: 'Pay rent', status: 'done', order: 'h', done_at: '2026-01-01T10:00:00Z' } },
    ] },
    { op: 'entity.create', entity_id: 'playlist', type_id: 'buckyos.dir', parent_id: 'data', order_key: 'p', name: 'playlist', payload: { members: {} } },
    { op: 'entity.create', entity_id: 'listen', type_id: 'buckyos.cell', parent_id: 'page', order_key: 'c', payload: { view: { type: 'html' }, def_ref: { entity_id: listen }, title: '一起听', bindings: { playlist: { entity_id: 'playlist' } } } },
    { op: 'entity.create', entity_id: 'photos', type_id: 'buckyos.dir', parent_id: 'data', order_key: 'q', name: 'photos', payload: { members: { 'Red.png': { kind: 'file', object_id: red }, 'Blue.png': { kind: 'file', object_id: blue } } } },
    { op: 'entity.create', entity_id: 'votes', type_id: 'buckyos.table-source', parent_id: 'data', order_key: 'v', name: 'votes', payload: { fields: [
      { field_id: 'voter', name: 'voter', type: 'text', submitter: true },
      { field_id: 'picks', name: 'picks', type: 'multi_select', options: [], max_items: 3 },
    ] } },
    { op: 'entity.create', entity_id: 'vote', type_id: 'buckyos.container', parent_id: 'page', order_key: 'd', payload: { kind: 'composite', def_ref: { entity_id: vote }, title: '投票', bindings: { photos: { entity_id: 'photos' }, votes: { entity_id: 'votes' } } } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  return ws
}

const configOf = async (api: Api, ws: Ws, id: string): Promise<Json> => (await api.read(ALICE, ws.workspace_id, id)).content.payload.config ?? {}
/** How visible the tools of a title row are (they fade in: opacity, not display). */
const toolsOpacity = (row: Locator) => row.locator('.aiws-tool-row').evaluate((el) => getComputedStyle(el).opacity)
const ROWS = ['mm', 'tl', 'listen', 'vote']

async function openPage(page: Page, ws: Ws) {
  await openWorkspacePage(page, ALICE, ws.workspace_id, 'page')
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
  await expect(page.getByTestId('aiws-flow')).toHaveAttribute('data-mode', 'edit')
  for (const id of ROWS) await expect(page.getByTestId(`aiws-flow-row-${id}`), id).toBeVisible()
}

test('FC-01 §6.2 §10.4 (computer) a flow page\'s controls carry their tools in their title row: primary actions, quick properties and ⋯ on hover; one ⋯ for a to-do', async ({ page, api }) => {
  const ws = await flowWorkspace(api, `flow-controls ${Date.now()}`)
  await openPage(page, ws)
  const row = (id: string) => page.getByTestId(`aiws-flow-row-${id}`)
  // not hovered: the titles show, the tools do not; hovered, the tools fade in
  await page.mouse.move(2, 400)
  for (const id of ROWS) await expect.poll(() => toolsOpacity(row(id)), { message: id }).toBe('0')
  await expect(row('tl')).toContainText('Week')
  await expect(row('mm')).toContainText('计划')
  await expect(row('vote')).toContainText('投票')
  await page.getByTestId('aiws-flow-control-mm').hover({ position: { x: 300, y: 20 } })
  await expect.poll(() => toolsOpacity(row('mm'))).toBe('1')
  await expect.poll(() => toolsOpacity(row('tl'))).toBe('0')

  // the mind map: its primary action, its three quick properties and ⋯
  for (const id of ['aiws-near-mindmap-outline', 'aiws-near-mindmap-direction', 'aiws-near-mindmap-edge', 'aiws-near-mindmap-spacing', 'aiws-flow-more-mm']) await expect(row('mm').getByTestId(id), id).toBeVisible()
  await page.screenshot({ path: 'test-results/aiworkspace-flow-controls/fc01-mindmap-row.png' })
  // a quick property changed from the row: one commit, the same value as the properties panel's
  await row('mm').getByTestId('aiws-near-mindmap-direction').click()
  await page.getByTestId('aiws-near-mindmap-direction-left').click()
  await expect.poll(async () => (await configOf(api, ws, 'mm')).direction).toBe('left')
  // ⋯ is the Block's whole menu: the type's other actions are there, so is "properties"
  await row('mm').getByTestId('aiws-flow-more-mm').click()
  const menu = page.getByTestId('aiws-flow-block-menu')
  await expect(menu).toBeVisible()
  for (const id of ['aiws-menu-mindmap-outline', 'aiws-menu-mindmap-copy-markdown', 'aiws-inspect-mm']) await expect(menu.getByTestId(id), id).toBeVisible()
  await page.keyboard.press('Escape')
  await expect(menu).toHaveCount(0)
  // the primary action runs from the row: the map as an outline, right after it on the page
  await page.getByTestId('aiws-flow-control-mm').hover({ position: { x: 300, y: 20 } })
  await row('mm').getByTestId('aiws-near-mindmap-outline').click()
  await expect.poll(async () => (await api.outline(ALICE, ws.workspace_id)).filter((e: Json) => e.parent_id === 'page' && e.view_type === 'richtext' && !e.deleted).length).toBe(1)

  // the to-do: its title and ⋯ are the row's; its own header keeps the count and the view switch, not a second title or ⋯
  const todo = page.getByTestId('aiws-flow-control-tl')
  await todo.scrollIntoViewIfNeeded()
  await expect(page.getByTestId('aiws-todo-menu-tl')).toHaveCount(0)
  await expect(page.getByTestId('aiws-todo-head-tl').locator('.aiws-dc-title')).toHaveCount(0)
  await expect(page.getByTestId('aiws-todo-count-tl')).toBeVisible()
  await expect(todo.locator('[data-testid^="aiws-flow-more-"]')).toHaveCount(1)
  await todo.hover({ position: { x: 300, y: 12 } })
  await expect.poll(() => toolsOpacity(row('tl'))).toBe('1')
  await row('tl').getByTestId('aiws-flow-more-tl').click()
  await expect(menu).toBeVisible()
  await expect(menu.getByTestId('aiws-inspect-tl')).toBeVisible()
  // the items its body publishes (its view menu) are in the Block's menu
  await menu.getByTestId('aiws-menu-control-hide-done').click()
  await expect(page.locator('[data-proj="tl~title~d"]')).toHaveCount(0)
  await expect(page.locator('[data-proj="tl~title~a"]')).toBeVisible()
  // a right click on an item is the item's menu alone, not the Block's on top of it; beside the items it is the Block's
  await page.locator('[data-proj="tl~title~a"]').click({ button: 'right' })
  await expect(page.getByTestId('aiws-todo-itemmenu-tl')).toBeVisible()
  await expect(menu).toHaveCount(0)
  await page.keyboard.press('Escape')
  await expect(page.getByTestId('aiws-todo-itemmenu-tl')).toHaveCount(0)
  await page.getByTestId('aiws-todo-count-tl').click({ button: 'right' })
  await expect(menu).toBeVisible()
  await expect(page.getByTestId('aiws-todo-itemmenu-tl')).toHaveCount(0)
  await page.keyboard.press('Escape')

  // the HTML control: its manifest's actions, two of them on the row, the rest in ⋯; one runs without running the control
  const listen = page.getByTestId('aiws-cell-frame-listen')
  await listen.scrollIntoViewIfNeeded()
  await listen.hover({ position: { x: 300, y: 12 } })
  await expect.poll(() => toolsOpacity(row('listen'))).toBe('1')
  await expect(row('listen').getByTestId('aiws-near-ext-action-start')).toBeVisible()
  await expect(row('listen').getByTestId('aiws-near-ext-action-pause')).toBeVisible()
  await expect(row('listen').getByTestId('aiws-near-ext-action-resume')).toHaveCount(0)
  await row('listen').getByTestId('aiws-flow-more-listen').click()
  await expect(menu.getByTestId('aiws-menu-ext-action-resume')).toBeVisible()
  await page.keyboard.press('Escape')
  await listen.hover({ position: { x: 300, y: 12 } })
  await row('listen').getByTestId('aiws-near-ext-action-start').click()
  await expect.poll(async () => (await configOf(api, ws, 'listen')).anchor?.playing ?? false, { timeout: 15_000 }).toBe(true)

  // the composite: its declared action on its title row, ⋯ its Block menu
  const vote = page.getByTestId('aiws-group-vote')
  await vote.scrollIntoViewIfNeeded()
  await vote.hover({ position: { x: 300, y: 8 } })
  await expect.poll(() => toolsOpacity(row('vote'))).toBe('1')
  await row('vote').getByTestId('aiws-near-ext-action-prepare').click()
  await expect.poll(async () => (await api.read(ALICE, ws.workspace_id, 'votes')).content.fields.find((f: Json) => f.field_id === 'picks').options.length, { timeout: 15_000 }).toBe(2)
  await vote.hover({ position: { x: 300, y: 8 } })
  await row('vote').getByTestId('aiws-flow-more-vote').click()
  await expect(menu).toBeVisible()
  await expect(menu.getByTestId('aiws-menu-ext-action-prepare')).toBeVisible()
  await expect(vote).toHaveClass(/is-selected/)
  await page.keyboard.press('Escape')

  // a locked control offers reading only: no tools of its type on the row, ⋯ stays
  const rev = (await api.read(ALICE, ws.workspace_id, 'mm')).content.key_revs.locked ?? 0
  expect((await api.commit(ALICE, ws, [{ op: 'entity.set_keys', entity_id: 'mm', keys: [{ key: 'locked', value: true, expect: { rev } }] }])).status).toBe('accepted')
  await page.getByTestId('aiws-flow-control-mm').scrollIntoViewIfNeeded()
  await page.getByTestId('aiws-flow-control-mm').hover({ position: { x: 300, y: 20 } })
  await expect(row('mm').getByTestId('aiws-near-mindmap-direction')).toHaveCount(0)
  await expect(row('mm').getByTestId('aiws-flow-more-mm')).toBeVisible()
})

test.describe('on a phone', () => {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the browser comes from the project, the rest is the phone
  const { defaultBrowserType, ...pixel } = devices['Pixel 7']
  test.use(pixel)

  test('FC-02 §6.2 (Pixel 7) a flow page\'s controls show their title row\'s tools all the time, a finger\'s size; ⋯ is the Block menu\'s sheet', async ({ page, api }) => {
    const ws = await flowWorkspace(api, `flow-controls-phone ${Date.now()}`)
    await openPage(page, ws)
    await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-phone', 'true')
    const row = (id: string) => page.getByTestId(`aiws-flow-row-${id}`)
    // nothing touched: every row's tools are shown
    for (const id of ROWS) await expect.poll(() => toolsOpacity(row(id)), { message: id }).toBe('1')
    const big = async (locator: Locator, what: string) => {
      const b = (await locator.boundingBox())!
      expect(b.width, what).toBeGreaterThanOrEqual(44)
      expect(b.height, what).toBeGreaterThanOrEqual(44)
    }
    await big(row('mm').getByTestId('aiws-near-mindmap-outline'), 'mind map: primary')
    await big(row('mm').getByTestId('aiws-flow-more-mm'), 'mind map: ⋯')
    await expect(row('mm').getByTestId('aiws-near-mindmap-direction')).toBeVisible()
    // a quick property's choices: a sheet from the bottom
    await row('mm').getByTestId('aiws-near-mindmap-direction').tap()
    await expect(page.locator('.aiws-sheet-layer .aiws-sheet-menu')).toBeVisible()
    await page.getByTestId('aiws-menu-close').tap()
    await expect(page.locator('.aiws-sheet-layer')).toHaveCount(0)

    // the to-do: one ⋯, the row's; it opens the Block's menu as a sheet, with what the to-do publishes
    await page.getByTestId('aiws-flow-control-tl').scrollIntoViewIfNeeded()
    await expect(page.getByTestId('aiws-todo-menu-tl')).toHaveCount(0)
    await big(row('tl').getByTestId('aiws-flow-more-tl'), 'to-do: ⋯')
    await row('tl').getByTestId('aiws-flow-more-tl').tap()
    const menu = page.getByTestId('aiws-flow-block-menu')
    await expect(menu).toBeVisible()
    await expect(menu).toHaveClass(/is-sheet/)
    await menu.getByTestId('aiws-menu-control-hide-done').tap()
    await expect(page.locator('[data-proj="tl~title~d"]')).toHaveCount(0)

    // the extensions: their declared actions on the row, a tap away
    await page.getByTestId('aiws-cell-frame-listen').scrollIntoViewIfNeeded()
    await big(row('listen').getByTestId('aiws-near-ext-action-start'), 'HTML control: primary')
    await page.getByTestId('aiws-group-vote').scrollIntoViewIfNeeded()
    await big(row('vote').getByTestId('aiws-near-ext-action-prepare'), 'composite: primary')
    await row('vote').getByTestId('aiws-near-ext-action-prepare').tap()
    await expect.poll(async () => (await api.read(ALICE, ws.workspace_id, 'votes')).content.fields.find((f: Json) => f.field_id === 'picks').options.length, { timeout: 15_000 }).toBe(2)
    await page.screenshot({ path: 'test-results/aiworkspace-flow-controls/fc02-phone.png' })
  })
})
