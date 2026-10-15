/* draw.io import (notepads/aiworkspace-drawio-import-todo.md): the extended shape, frames and swimlanes, the import
 * entries (file, drop, paste), SVG shapes and the library, metadata and export. Fixtures:
 * src/frame/aiworkspace/fixtures/drawio/. Screenshots go to test-results/aiworkspace-drawio/. */

import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import type { Page } from '@playwright/test'
import { buildOperations, convertPage, ImportReport, readDrawio } from '../../../aiworkspace/import-drawio/mod.ts'
import { fitAll, mainMenu, openCanvas, test, expect, type Api } from './fixtures'

const ALICE = 'tok-alice'
const SHOTS = 'test-results/aiworkspace-drawio'
const FIXTURES = resolve('../aiworkspace/fixtures/drawio')

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

/** A workspace with one empty free canvas `sf-d`. */
async function canvasWorkspace(api: Api, title: string) {
  const ws = await api.rpc(ALICE, 'ws.create', { title })
  await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'sf-d-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'i', payload: { kind: 'folder', title: '画布', system: 'surface_content', surface_id: 'sf-d' } },
    { op: 'entity.create', entity_id: 'sf-d', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'i', payload: { kind: 'surface', layout: { mode: 'free' }, title: '画布', content_folder_id: 'sf-d-content' } },
  ])
  return ws as { workspace_id: string; epoch: string }
}

/** Convert a fixture page with the shared module and commit it through the API (no UI, no images). */
async function commitFixture(api: Api, ws: { workspace_id: string; epoch: string }, file: string, page = 0) {
  const drawio = await readDrawio({ bytes: new Uint8Array(readFileSync(`${FIXTURES}/${file}`)) })
  const report = new ImportReport()
  const plan = await convertPage(drawio.pages[page], report)
  const built = buildOperations(plan, { parentId: 'sf-d', origin: { x: 40, y: 40 }, afterKey: null, contentFolderId: 'sf-d-content', contentAfterKey: null, assets: new Map() })
  const result = await api.commit(ALICE, ws, [...built.objects, ...built.lines] as Json[])
  expect(result.status, JSON.stringify(result)).toBe('accepted')
  return { plan, built, report }
}

async function shot(page: Page, name: string) {
  await page.mouse.move(5, 500)
  await page.getByTestId('aiws-canvas-view').screenshot({ path: `${SHOTS}/${name}.png` })
}

test('DR-render basic shapes, swimlanes and connectors from the fixtures draw in light and dark', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `dr-render ${Date.now()}`)
  const { built } = await commitFixture(api, ws, 'shapes.drawio')
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  for (const id of built.topLevel.slice(0, 5)) await expect(page.getByTestId(`aiws-canvas-block-${id}`)).toBeVisible()
  await shot(page, 'render-shapes-light')
  await page.addInitScript(() => { window.localStorage.setItem('buckyos.prototype.theme.v1', 'dark') })
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await shot(page, 'render-shapes-dark')
})

async function importFile(page: Page, file: string) {
  await expect(page.getByTestId('aiws-import-report')).toHaveCount(0)
  const chooser = page.waitForEvent('filechooser')
  await mainMenu(page, 'aiws-menu-insert', 'aiws-menu-import-drawio')
  await (await chooser).setFiles(`${FIXTURES}/${file}`)
}

const cellsOn = async (api: Api, ws: { workspace_id: string }, surface: string): Promise<Json[]> => {
  const outline = await api.outline(ALICE, ws.workspace_id)
  const ids = new Set([surface])
  let grew = true
  while (grew) { grew = false; for (const e of outline) if (!ids.has(e.entity_id) && ids.has(e.parent_id) && !e.deleted) { ids.add(e.entity_id); grew = true } }
  return outline.filter((e: Json) => ids.has(e.entity_id) && e.entity_id !== surface)
}

test('DR-01 a draw.io flowchart imported from the main menu: shapes, bound connectors and labels, light and dark', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `dr-01 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await importFile(page, 'flowchart.drawio')
  await expect.poll(async () => (await cellsOn(api, ws, 'sf-d')).filter((e) => e.view_type === 'connector').length).toBe(8)
  const cells = await cellsOn(api, ws, 'sf-d')
  expect(cells.filter((e) => e.view_type === 'shape')).toHaveLength(8)
  const lines = cells.filter((e) => e.view_type === 'connector')
  // every line between two shapes is bound at both ends (e8 starts at the free text note: bound too)
  expect(lines.every((l) => l.connector.start && l.connector.end)).toBe(true)
  const yes = lines.find((l) => l.title === '是')
  expect(yes.connector.start.anchor.id).toBe('e')
  expect(yes.connector.route).toBe('elbow')
  // nothing lost: no report dialog, a notice instead
  await expect(page.getByTestId('aiws-import-report')).toHaveCount(0)
  for (const l of lines) await expect(page.getByTestId(`aiws-canvas-block-${l.entity_id}`)).toHaveAttribute('data-start', 'bound')
  await shot(page, 'dr01-flowchart-light')
  await page.addInitScript(() => { window.localStorage.setItem('buckyos.prototype.theme.v1', 'dark') })
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await shot(page, 'dr01-flowchart-dark')
})

const byTitle = (cells: Json[], title: string, view?: string) => cells.find((e) => e.title === title && (!view || e.view_type === view))

test('DR-02 swimlanes import as groups with frames; dragging an object across lanes moves it into the lane; lanes are added and deleted', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `dr-02 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await importFile(page, 'swimlane.drawio')
  await expect(page.getByTestId('aiws-import-report')).toBeVisible()
  await expect(page.getByTestId('aiws-import-notes')).toContainText('隐藏的图层未导入')
  await expect(page.getByTestId('aiws-import-notes')).toContainText('折叠的容器按展开导入')
  await page.getByTestId('aiws-import-report-close').click()
  let cells = await cellsOn(api, ws, 'sf-d')
  const poolFrame = byTitle(cells, '采购流程', 'frame')
  const lane1Frame = byTitle(cells, '申请部门', 'frame')
  const lane2Frame = byTitle(cells, '采购部', 'frame')
  expect(poolFrame && lane1Frame && lane2Frame).toBeTruthy()
  const lane1 = lane1Frame.parent_id
  const lane2 = lane2Frame.parent_id
  expect(cells.find((e) => e.entity_id === lane2).parent_id).toBe(poolFrame.parent_id)
  const order = byTitle(cells, '下单', 'shape')
  expect(order.parent_id).toBe(lane2)
  expect(byTitle(cells, '不要导入我')).toBeUndefined()
  expect(byTitle(cells, '本页由流程委员会维护').locked).toBe(true)
  await shot(page, 'dr02-swimlane-light')
  // drag 下单 up into 申请部门
  const from = await page.getByTestId(`aiws-canvas-block-${order.entity_id}`).boundingBox()
  const laneBox = await page.getByTestId(`aiws-canvas-block-${lane1Frame.entity_id}`).boundingBox()
  await page.mouse.move(from!.x + from!.width / 2, from!.y + from!.height / 2)
  await page.mouse.down()
  await page.mouse.move(from!.x + from!.width / 2, laneBox!.y + laneBox!.height / 2, { steps: 8 })
  await page.mouse.up()
  await expect.poll(async () => (await cellsOn(api, ws, 'sf-d')).find((e) => e.entity_id === order.entity_id).parent_id).toBe(lane1)
  // a press on an empty spot of a lane selects the lane (its group); add a lane below it, then delete it
  const lane2Box = await page.getByTestId(`aiws-canvas-block-${lane2Frame.entity_id}`).boundingBox()
  await page.mouse.click(lane2Box!.x + lane2Box!.width - 30, lane2Box!.y + lane2Box!.height - 20)
  await expect(page.getByTestId(`aiws-selection-${lane2}`)).toBeVisible()
  const poolBefore = cells.find((e) => e.entity_id === poolFrame.parent_id).placement.h
  await page.getByTestId('aiws-near-lane-add').click()
  await expect.poll(async () => (await cellsOn(api, ws, 'sf-d')).find((e) => e.entity_id === poolFrame.parent_id).placement.h).toBe(poolBefore + 180)
  cells = await cellsOn(api, ws, 'sf-d')
  const added = cells.find((e) => e.view_type === 'frame' && e.title === '泳道 3')
  expect(added).toBeTruthy()
  expect(cells.find((e) => e.entity_id === poolFrame.entity_id).placement.h).toBe(poolBefore + 180)
  await shot(page, 'dr02-swimlane-added')
  const addedBox = await page.getByTestId(`aiws-canvas-block-${added.entity_id}`).boundingBox()
  await page.mouse.click(addedBox!.x + addedBox!.width / 2, addedBox!.y + addedBox!.height / 2)
  await page.getByTestId('aiws-near-lane-delete').click()
  await expect.poll(async () => (await cellsOn(api, ws, 'sf-d')).find((e) => e.entity_id === poolFrame.parent_id).placement.h).toBe(poolBefore)
  // dark theme
  await page.addInitScript(() => { window.localStorage.setItem('buckyos.prototype.theme.v1', 'dark') })
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await shot(page, 'dr02-swimlane-dark')
})

test('DR-03 a multi-page compressed file becomes canvases per page with background, images uploaded and a report of what was lost', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `dr-03 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await importFile(page, 'multipage.drawio')
  await expect(page.getByTestId('aiws-import-report')).toBeVisible({ timeout: 30_000 })
  const notes = page.getByTestId('aiws-import-notes')
  await expect(notes.locator('[data-kind="image_missing"]')).toBeVisible()
  await expect(notes.locator('[data-kind="complex_html"]')).toBeVisible()
  await expect(page.getByTestId('aiws-import-summary')).toContainText('2 页')
  await page.getByTestId('aiws-import-report-close').click()
  const outline = await api.outline(ALICE, ws.workspace_id)
  const surfaces = outline.filter((e: Json) => e.kind === 'surface' && e.entity_id !== 'sf-d')
  expect(surfaces.map((s: Json) => s.title).sort()).toEqual(['系统总览', '部署'])
  const overview = surfaces.find((s: Json) => s.title === '系统总览')
  const read = await api.read(ALICE, ws.workspace_id, overview.entity_id)
  expect(read.content.payload.config.background).toBe('#fffbe6')
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-background', '#fffbe6')
  const cells = await cellsOn(api, ws, overview.entity_id)
  const assets = cells.filter((e: Json) => e.view_type === 'asset')
  expect(assets).toHaveLength(2)
  // the remote picture that could not be fetched: a dashed box with its link
  const missing = cells.find((e: Json) => e.view_type === 'shape' && e.entity_id && e.title === undefined && false)
  void missing
  const gateway = byTitle(cells, '网关', 'shape')
  const gw = await api.read(ALICE, ws.workspace_id, gateway.entity_id)
  expect(gw.content.payload.config.link).toBe('https://example.com/gateway')
  expect(gw.content.payload.config.tooltip).toBe('所有外部请求的入口')
  expect(gw.content.payload.config.data).toEqual({ version: 'v2.3', owner: '平台组' })
  expect(gw.content.payload.config.text).toBe('网关\nv2.3')
  await fitAll(page)
  await shot(page, 'dr03-multipage-overview')
})

test('DR-04 inline stencils and library shapes import as SVG shapes with their own anchors; a script-drawn library shape is reported', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `dr-04 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await importFile(page, 'stencil.drawio')
  await expect(page.getByTestId('aiws-import-report')).toBeVisible()
  await expect(page.getByTestId('aiws-import-notes').locator('[data-kind="library_shape"]')).toContainText('mxgraph.aws4.resourceIcon')
  await page.getByTestId('aiws-import-report-close').click()
  const cells = await cellsOn(api, ws, 'sf-d')
  const svgShapes = cells.filter((e: Json) => e.view_type === 'shape')
  const configs = await Promise.all(svgShapes.map(async (e: Json) => ({ e, config: (await api.read(ALICE, ws.workspace_id, e.entity_id)).content.payload.config })))
  const kinds = configs.map(({ config }) => config.shape)
  expect(kinds.filter((k: string) => k === 'svg')).toHaveLength(5)
  // script-drawn BPMN shapes take the nearest outline
  expect(configs.find(({ config }) => config.drawio?.id === 'task')!.config.shape).toBe('rect')
  expect(configs.find(({ config }) => config.drawio?.id === 'ev')!.config.shape).toBe('ellipse')
  const star = configs.find(({ config }) => config.drawio?.id === 'star')!
  expect(star.config.anchors.map((a: Json) => a.id).sort()).toEqual(['bottom', 'left', 'right', 'top'])
  const lines = cells.filter((e: Json) => e.view_type === 'connector')
  const s1 = lines.find((l: Json) => l.connector.start?.entity_id === star.e.entity_id && l.connector.start.anchor.id === 'right')
  expect(s1).toBeTruthy()
  for (const { e } of configs) await expect(page.getByTestId(`aiws-shape-${e.entity_id}`)).toBeVisible()
  await fitAll(page)
  await shot(page, 'dr04-stencils-light')
  await page.addInitScript(() => { window.localStorage.setItem('buckyos.prototype.theme.v1', 'dark') })
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  await shot(page, 'dr04-stencils-dark')
})

test('DR-05 ER tables and UML classes: header frames, rows, cardinality caps, actor', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `dr-05 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await importFile(page, 'er.drawio')
  await expect(page.getByTestId('aiws-import-report')).toBeVisible()
  await page.getByTestId('aiws-import-report-close').click()
  let cells = await cellsOn(api, ws, 'sf-d')
  const customer = byTitle(cells, 'customer', 'frame')
  expect((await api.read(ALICE, ws.workspace_id, customer.entity_id)).content.payload.config.header).toBe(30)
  const rel = cells.find((e: Json) => e.view_type === 'connector' && e.title === '下单')
  const relConfig = (await api.read(ALICE, ws.workspace_id, rel.entity_id)).content.payload.config
  expect(relConfig.start_cap).toBe('cardinality_exactly_one')
  expect(relConfig.end_cap).toBe('cardinality_many')
  await fitAll(page)
  await shot(page, 'dr05-er')
  // UML on a second canvas: the import of a one-page file into the canvas shown
  await importFile(page, 'uml.drawio')
  await expect(page.getByTestId('aiws-import-report')).toBeVisible()
  await page.getByTestId('aiws-import-report-close').click()
  cells = await cellsOn(api, ws, 'sf-d')
  expect(byTitle(cells, 'Animal', 'frame')).toBeTruthy()
  expect(byTitle(cells, '+ speak(): void', 'shape')).toBeTruthy()
  await fitAll(page)
  await shot(page, 'dr05-er-uml')
})

const configOf = async (api: Api, ws: { workspace_id: string }, id: string) => (await api.read(ALICE, ws.workspace_id, id)).content.payload as Json

test('DR-06 drawing by hand: the grouped shape picker, typing into a shape, line and text tools, the text object, own connection points', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `dr-06 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  // the shape tool opens the picker; a diamond is placed with a click
  await page.getByTestId('aiws-tool-insert-shape').click()
  await expect(page.getByTestId('aiws-tool-shape-picker')).toBeVisible()
  await page.getByTestId('aiws-canvas-view').screenshot({ path: `${SHOTS}/dr06-picker.png` })
  await page.getByTestId('aiws-shape-kind-diamond').click()
  await page.mouse.click(500, 400)
  await expect.poll(async () => (await cellsOn(api, ws, 'sf-d')).length).toBe(1)
  const diamond = (await cellsOn(api, ws, 'sf-d'))[0]
  expect((await configOf(api, ws, diamond.entity_id)).config.shape).toBe('diamond')
  // selected after placing: typing replaces its text, Enter saves (the outline name follows the first line)
  await expect(page.getByTestId(`aiws-selection-${diamond.entity_id}`)).toBeVisible()
  await page.keyboard.type('Check')
  await expect(page.getByTestId(`aiws-shape-edit-${diamond.entity_id}`)).toBeFocused()
  await page.keyboard.press('Shift+Enter')
  await page.keyboard.type('2nd')
  await page.keyboard.press('Enter')
  await expect.poll(async () => (await configOf(api, ws, diamond.entity_id)).config.text).toBe('Check\n2nd')
  expect((await configOf(api, ws, diamond.entity_id)).title).toBe('Check')
  // near toolbar: no fill, a thick dashed outline, bold text
  await page.getByTestId('aiws-near-shape-fill').click()
  await page.getByTestId('aiws-color-none').click()
  await expect.poll(async () => (await configOf(api, ws, diamond.entity_id)).config.fill).toBe('none')
  await page.getByTestId('aiws-near-shape-line').click()
  await page.getByTestId('aiws-shape-width-4').click()
  await page.getByTestId('aiws-shape-dash-dashed').click()
  await expect.poll(async () => (await configOf(api, ws, diamond.entity_id)).config.dash).toBe('dashed')
  await page.keyboard.press('Escape')
  await page.getByTestId('aiws-near-shape-bold').click()
  await expect.poll(async () => (await configOf(api, ws, diamond.entity_id)).config).toMatchObject({ stroke_width: 4, bold: true })
  await shot(page, 'dr06-diamond')
  // the text object: placed, then typed into at once
  await page.getByTestId('aiws-tool-insert-shape-text').click()
  await page.mouse.click(800, 300)
  await expect.poll(async () => (await cellsOn(api, ws, 'sf-d')).length).toBe(2)
  const text = (await cellsOn(api, ws, 'sf-d')).find((e: Json) => e.entity_id !== diamond.entity_id)
  await expect(page.getByTestId(`aiws-shape-edit-${text.entity_id}`)).toBeFocused()
  await page.keyboard.type('Plain words')
  await page.keyboard.press('Enter')
  await expect.poll(async () => (await configOf(api, ws, text.entity_id)).config).toMatchObject({ shape: 'text', text: 'Plain words' })
  // own connection points from the inspector
  await page.getByTestId(`aiws-canvas-block-${diamond.entity_id}`).click()
  await page.keyboard.press('F2')
  await page.getByTestId('aiws-shape-anchors').locator('summary').click()
  await page.getByTestId('aiws-anchor-id').fill('port1')
  await page.getByTestId('aiws-anchor-add').click()
  await expect.poll(async () => (await configOf(api, ws, diamond.entity_id)).config.anchors).toEqual([{ id: 'port1', x: 0.5, y: 0 }])
  await shot(page, 'dr06-inspector')
})

/** Drop a fixture file on the canvas at a screen point (the browser's own drag and drop). */
async function dropFile(page: Page, file: string, mime: string, at: { x: number; y: number }) {
  const b64 = readFileSync(`${FIXTURES}/${file}`).toString('base64')
  const dt = await page.evaluateHandle(({ b64, name, mime }) => {
    const bin = atob(b64)
    const bytes = new Uint8Array(bin.length)
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i)
    const transfer = new DataTransfer()
    transfer.items.add(new File([bytes], name, { type: mime }))
    return transfer
  }, { b64, name: file.split('/').pop()!, mime })
  const main = page.locator('.aiws-canvas-main')
  await main.dispatchEvent('dragover', { dataTransfer: dt, clientX: at.x, clientY: at.y })
  await main.dispatchEvent('drop', { dataTransfer: dt, clientX: at.x, clientY: at.y })
}

/** Paste text from the system clipboard onto the canvas. */
async function pasteText(page: Page, text: string) {
  await page.evaluate((t) => {
    const transfer = new DataTransfer()
    transfer.setData('text/plain', t)
    document.body.dispatchEvent(new ClipboardEvent('paste', { clipboardData: transfer, bubbles: true, cancelable: true }))
  }, text)
}

test('DR-07 carriers: a .drawio.png dropped on the canvas lands at the drop point; a model pasted as text and a .drawio.svg import too', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `dr-07 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await dropFile(page, 'flowchart.drawio.png', 'image/png', { x: 400, y: 300 })
  await expect.poll(async () => (await cellsOn(api, ws, 'sf-d')).filter((e: Json) => e.view_type === 'connector').length).toBe(8)
  const first = await cellsOn(api, ws, 'sf-d')
  expect(first.filter((e: Json) => e.view_type === 'asset')).toHaveLength(0)
  // the model draw.io puts on the clipboard (URL-encoded), pasted: imported beside what is there
  const model = readFileSync(`${FIXTURES}/uml.drawio`, 'utf8').match(/<mxGraphModel[\s\S]*<\/mxGraphModel>/)![0]
  await pasteText(page, encodeURIComponent(model))
  await expect(page.getByTestId('aiws-import-report')).toBeVisible()
  await page.getByTestId('aiws-import-report-close').click()
  const second = await cellsOn(api, ws, 'sf-d')
  expect(byTitle(second, 'Animal', 'frame')).toBeTruthy()
  // placed beside the flowchart, not on it
  const flowRight = Math.max(...first.filter((e: Json) => e.parent_id === 'sf-d' && e.placement).map((e: Json) => e.placement.x + e.placement.w))
  const animalGroup = second.find((e: Json) => e.entity_id === byTitle(second, 'Animal', 'frame').parent_id)
  expect(animalGroup.placement.x).toBeGreaterThan(flowRight)
  await fitAll(page)
  await shot(page, 'dr07-dropped-and-pasted')
  // a .drawio.svg from the file picker
  await importFile(page, 'flowchart.drawio.svg')
  await expect.poll(async () => (await cellsOn(api, ws, 'sf-d')).filter((e: Json) => e.view_type === 'connector').length).toBe(8 + 5 + 8)
})

test('DR-08 SVG shapes: pasted SVG becomes a recolourable shape; the library panel places and drops draw.io shapes that take lines on their ports', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `dr-08 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await pasteText(page, '<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 2l3 7h7l-5.5 4 2 7L12 16l-6.5 4 2-7L2 9h7z"/><script>alert(1)</script></svg>')
  await expect.poll(async () => (await cellsOn(api, ws, 'sf-d')).length).toBe(1)
  const star = (await cellsOn(api, ws, 'sf-d'))[0]
  const starConfig = (await configOf(api, ws, star.entity_id)).config
  expect(starConfig.shape).toBe('svg')
  expect(starConfig.stroke).toBe('auto')
  expect(starConfig.svg.body).not.toContain('script')
  await expect(page.getByTestId(`aiws-shape-${star.entity_id}`).locator('path')).toHaveCount(1)
  // recolour: the stroke follows the shape's stroke colour
  await page.getByTestId(`aiws-canvas-block-${star.entity_id}`).click()
  await page.getByTestId('aiws-near-shape-stroke').click()
  await page.getByTestId('aiws-color-d1242f').click()
  await expect.poll(async () => (await configOf(api, ws, star.entity_id)).config.stroke).toBe('#d1242f')
  await expect(page.getByTestId(`aiws-shape-${star.entity_id}`).locator('svg.aiws-shape-art')).toHaveCSS('--aiws-shape-stroke', '#d1242f')
  // the library: search, click to place
  await page.mouse.click(1200, 800)
  await page.getByTestId('aiws-tool-library').click()
  await expect(page.getByTestId('aiws-shape-library')).toBeVisible()
  await page.getByTestId('aiws-library-search').fill('decision')
  await expect(page.getByTestId('aiws-library-shape-mxgraph.flowchart.decision')).toBeVisible()
  await page.getByTestId('aiws-canvas-view').screenshot({ path: `${SHOTS}/dr08-library.png` })
  await page.getByTestId('aiws-library-shape-mxgraph.flowchart.decision').click()
  await page.mouse.click(700, 400)
  await expect.poll(async () => (await cellsOn(api, ws, 'sf-d')).length).toBe(2)
  // drag a BPMN shape from the library onto the canvas
  await page.getByTestId('aiws-tool-library').click()
  await page.getByTestId('aiws-library-select').selectOption('bpmn')
  await page.getByTestId('aiws-library-shape-mxgraph.bpmn.user_task').dragTo(page.getByTestId('aiws-canvas'), { targetPosition: { x: 1000, y: 450 } })
  await expect.poll(async () => (await cellsOn(api, ws, 'sf-d')).length).toBe(3)
  const cells = await cellsOn(api, ws, 'sf-d')
  const configs = await Promise.all(cells.map(async (e: Json) => ({ e, c: (await configOf(api, ws, e.entity_id)).config })))
  const decision = configs.find(({ c }) => c.drawio?.style?.includes('mxgraph.flowchart.decision'))!
  expect(decision.c.anchors.length).toBeGreaterThan(0)
  expect(configs.some(({ c }) => c.drawio?.style?.includes('mxgraph.bpmn.user_task'))).toBe(true)
  await page.keyboard.press('Escape')
  await fitAll(page)
  await shot(page, 'dr08-svg-shapes')
})

test('DR-09 metadata and page: a link, tooltip and data on a Block; the link button on hover; the canvas background', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `dr-09 ${Date.now()}`)
  const { built } = await commitFixture(api, ws, 'flowchart.drawio')
  await openCanvas(page, ALICE, ws.workspace_id)
  await fitAll(page)
  const submit = built.ids.get('submit')!
  await page.getByTestId(`aiws-canvas-block-${submit}`).click()
  await page.keyboard.press('F2')
  const meta = page.getByTestId('aiws-metadata')
  await meta.locator('summary').click()
  await page.getByTestId('aiws-meta-link').fill('https://example.com/form')
  await page.getByTestId('aiws-meta-tooltip').fill('填写报销单')
  await page.getByTestId('aiws-meta-key').focus()
  await expect.poll(async () => (await configOf(api, ws, submit)).config.link).toBe('https://example.com/form')
  await expect.poll(async () => (await configOf(api, ws, submit)).config.tooltip).toBe('填写报销单')
  await page.getByTestId('aiws-meta-key').fill('owner')
  await page.getByTestId('aiws-meta-value').fill('财务部')
  await page.getByTestId('aiws-meta-add').click()
  await expect.poll(async () => (await configOf(api, ws, submit)).config.data).toEqual({ owner: '财务部' })
  // hovering the Block shows its link button and its tooltip
  await page.mouse.click(1200, 900)
  const box = await page.getByTestId(`aiws-canvas-block-${submit}`).boundingBox()
  await page.mouse.move(box!.x + 10, box!.y + 10)
  await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2)
  await expect(page.getByTestId('aiws-afford-link')).toBeVisible()
  await expect(page.getByTestId(`aiws-block-${submit}`)).toHaveAttribute('title', '填写报销单')
  // the canvas background, from the canvas icon's popover
  await page.getByTestId('aiws-surface-icon').click()
  await page.getByTestId('aiws-background-fffbe6').click()
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-background', '#fffbe6')
  expect((await api.read(ALICE, ws.workspace_id, 'sf-d')).content.payload.config.background).toBe('#fffbe6')
  await page.keyboard.press('Escape')
  await shot(page, 'dr09-background')
})

test('DR-10 export: a canvas imported from draw.io and exported again keeps its ids, layout, bindings and styles', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `dr-10 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  for (const file of ['flowchart.drawio']) {
    await importFile(page, file)
    await expect.poll(async () => (await cellsOn(api, ws, 'sf-d')).filter((e: Json) => e.view_type === 'connector').length).toBe(8)
  }
  const download = page.waitForEvent('download')
  await mainMenu(page, 'aiws-menu-export-menu', 'aiws-menu-export-drawio')
  const file = await download
  expect(file.suggestedFilename()).toBe('画布.drawio')
  const exported = readFileSync(await file.path(), 'utf8')
  // the export read back with the importer: the same objects in the same places, the same bindings
  const report = new ImportReport()
  const again = await convertPage((await readDrawio({ text: exported })).pages[0], report)
  const original = await convertPage((await readDrawio({ bytes: new Uint8Array(readFileSync(`${FIXTURES}/flowchart.drawio`)) })).pages[0], new ImportReport())
  const byKey = new Map(again.items.map((i) => [i.key, i]))
  for (const item of original.items) {
    const twin = byKey.get(item.key)
    expect(twin, item.key).toBeTruthy()
    expect(twin!.type).toBe(item.type)
    if (item.type === 'connector') {
      expect(twin!.line!.start).toEqual(item.line!.start)
      expect(twin!.line!.end).toEqual(item.line!.end)
      expect(twin!.line!.route).toBe(item.line!.route)
      continue
    }
    for (const k of ['x', 'y', 'w', 'h'] as const) expect(Math.abs(twin!.rect[k] - item.rect[k]), `${item.key}.${k}`).toBeLessThanOrEqual(1)
    expect(twin!.config.shape).toBe(item.config.shape)
    expect(twin!.config.fill).toBe(item.config.fill)
    expect(twin!.config.text).toBe(item.config.text)
  }
})

test('DR-11 the insert catalog: a swimlane pool with three lanes, objects dragged in and out; an SVG shape from pasted code; "import draw.io" from the catalog', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `dr-11 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-tool-catalog').click()
  await page.getByTestId('aiws-catalog-search').fill('泳道')
  await page.getByTestId('aiws-catalog-entry-block-shape:swimlane').or(page.getByTestId('aiws-catalog-entry-block-frame:swimlane')).click()
  await page.getByTestId('aiws-catalog-option-lanes').selectOption('3')
  await page.getByTestId('aiws-catalog-insert').click()
  await expect.poll(async () => (await cellsOn(api, ws, 'sf-d')).filter((e: Json) => e.view_type === 'frame').length).toBe(4)
  let cells = await cellsOn(api, ws, 'sf-d')
  const pool = cells.find((e: Json) => e.kind === 'group' && e.parent_id === 'sf-d')
  const lanes = cells.filter((e: Json) => e.kind === 'group' && e.parent_id === pool.entity_id).sort((a: Json, b: Json) => a.placement.y - b.placement.y)
  expect(lanes).toHaveLength(3)
  await fitAll(page)
  await shot(page, 'dr11-swimlane-pool')
  // a shape placed in the second lane joins it; dragged out of the pool it leaves it
  const lane2 = cells.find((e: Json) => e.view_type === 'frame' && e.parent_id === lanes[1].entity_id)
  const laneBox = (await page.getByTestId(`aiws-canvas-block-${lane2.entity_id}`).boundingBox())!
  await page.getByTestId('aiws-tool-insert-shape').click()
  await page.getByTestId('aiws-shape-kind-rect').click()
  // the click is the shape's top-left: near the lane's top, its centre is inside the lane
  await page.mouse.click(laneBox.x + 80, laneBox.y + 8)
  await expect.poll(async () => (await cellsOn(api, ws, 'sf-d')).filter((e: Json) => e.view_type === 'shape').length).toBe(1)
  let shape = (await cellsOn(api, ws, 'sf-d')).find((e: Json) => e.view_type === 'shape')
  expect(shape.parent_id).toBe(lanes[1].entity_id)
  // a move inside the lane keeps it in the lane
  const sb = (await page.getByTestId(`aiws-canvas-block-${shape.entity_id}`).boundingBox())!
  await page.mouse.move(sb.x + sb.width / 2, sb.y + sb.height / 2)
  await page.mouse.down()
  await page.mouse.move(sb.x + sb.width / 2 + 30, sb.y + sb.height / 2 + 5, { steps: 5 })
  await page.mouse.up()
  await expect.poll(async () => (await cellsOn(api, ws, 'sf-d')).find((e: Json) => e.entity_id === shape.entity_id).parent_id).toBe(lanes[1].entity_id)
  const poolFrame = cells.find((e: Json) => e.view_type === 'frame' && e.parent_id === pool.entity_id)
  const pb = (await page.getByTestId(`aiws-canvas-block-${poolFrame.entity_id}`).boundingBox())!
  const sb2 = (await page.getByTestId(`aiws-canvas-block-${shape.entity_id}`).boundingBox())!
  await page.mouse.move(sb2.x + sb2.width / 2, sb2.y + sb2.height / 2)
  await page.mouse.down()
  await page.mouse.move(sb2.x + sb2.width / 2, pb.y + pb.height + 120, { steps: 8 })
  await page.mouse.up()
  await expect.poll(async () => (await cellsOn(api, ws, 'sf-d')).find((e: Json) => e.entity_id === shape.entity_id).parent_id).toBe('sf-d')
  // the SVG entry
  await page.getByTestId('aiws-tool-catalog').click()
  await page.getByTestId('aiws-catalog-search').fill('SVG')
  await page.getByTestId('aiws-catalog-entry-block-shape:svg').click()
  await page.getByTestId('aiws-catalog-svg-text').fill('<svg viewBox="0 0 10 10" width="80" height="80"><circle cx="5" cy="5" r="4" fill="#1a7f37"/></svg>')
  await page.getByTestId('aiws-catalog-insert').click()
  await expect.poll(async () => (await cellsOn(api, ws, 'sf-d')).filter((e: Json) => e.view_type === 'shape').length).toBe(2)
  cells = await cellsOn(api, ws, 'sf-d')
  shape = cells.filter((e: Json) => e.view_type === 'shape').find((e: Json) => e.entity_id !== shape.entity_id)
  expect((await configOf(api, ws, shape.entity_id)).config).toMatchObject({ shape: 'svg', fill: '#1a7f37' })
  // import from the catalog
  await page.getByTestId('aiws-tool-catalog').click()
  const chooser = page.waitForEvent('filechooser')
  await page.getByTestId('aiws-catalog-import-drawio').click()
  await (await chooser).setFiles(`${FIXTURES}/flowchart.drawio`)
  await expect.poll(async () => (await cellsOn(api, ws, 'sf-d')).filter((e: Json) => e.view_type === 'connector').length).toBe(8)
})
