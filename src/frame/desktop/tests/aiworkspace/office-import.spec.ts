/* Office import (notepads/aiworkspace-office-import-todo.md O4, O5, O5b): a .docx from the insert menu becomes a new
 * flow page of rich text and picture Blocks with the original file last; a dropped .pptx asks, then becomes a canvas
 * of page groups with a presentation path (hidden slides left out unless asked for), a chart as a picture when the
 * content server has LibreOffice and as a dashed box when it has not; a file Block expands without a second upload;
 * "as it looks" goes through LibreOffice and the PDF importer. The LibreOffice cases follow what the content server
 * advertises (`hello.repr.profiles`). Screenshots go to test-results/aiworkspace-office/. */

import { readFileSync } from 'node:fs'
import type { Page } from '@playwright/test'
import { mainMenu, openCanvas, test, expect, type Api } from './fixtures'

const ALICE = 'tok-alice'
const SHOTS = 'test-results/aiworkspace-office'
const FIXTURES = new URL('../../../aiworkspace/fixtures/office/', import.meta.url).pathname
const DOCX = 'application/vnd.openxmlformats-officedocument.wordprocessingml.document'
const PPTX = 'application/vnd.openxmlformats-officedocument.presentationml.presentation'

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

async function canvasWorkspace(api: Api, title: string) {
  const ws = await api.rpc(ALICE, 'ws.create', { title })
  await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'sf-o-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'i', payload: { kind: 'folder', title: '画布', system: 'surface_content', surface_id: 'sf-o' } },
    { op: 'entity.create', entity_id: 'sf-o', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'i', payload: { kind: 'surface', layout: { mode: 'free' }, title: '画布', content_folder_id: 'sf-o-content' } },
  ])
  return ws as { workspace_id: string; epoch: string }
}

const children = (outline: Json[], parent: string) => outline.filter((e) => e.parent_id === parent && !e.deleted).sort((a, b) => (a.order_key < b.order_key ? -1 : 1))
const fixture = (path: string) => readFileSync(`${FIXTURES}${path}`)

async function drop(page: Page, name: string, type: string, bytes: Buffer, at: { x: number; y: number }) {
  const dt = await page.evaluateHandle(({ b64, name, type }) => {
    const bin = atob(b64)
    const data = new Uint8Array(bin.length)
    for (let i = 0; i < bin.length; i++) data[i] = bin.charCodeAt(i)
    const transfer = new DataTransfer()
    transfer.items.add(new File([data], name, { type }))
    return transfer
  }, { b64: bytes.toString('base64'), name, type })
  const target = page.locator('.aiws-canvas-main')
  await target.dispatchEvent('dragover', { dataTransfer: dt, clientX: at.x, clientY: at.y })
  await target.dispatchEvent('drop', { dataTransfer: dt, clientX: at.x, clientY: at.y })
}

/** Whether the e2e content server has LibreOffice, as the open dialog found out (`hello.repr.profiles`). */
async function libreOffice(page: Page): Promise<boolean> {
  const dialog = page.getByTestId('aiws-office-dialog')
  await expect(dialog).not.toHaveAttribute('data-libreoffice', 'unknown', { timeout: 20_000 })
  return (await dialog.getAttribute('data-libreoffice')) === 'true'
}

async function newSurface(api: Api, ws: string, known: string[]): Promise<{ outline: Json[]; surface: Json }> {
  let outline: Json[] = []
  await expect.poll(async () => { outline = await api.outline(ALICE, ws); return outline.filter((e) => e.kind === 'surface' && !e.deleted).length }, { timeout: 60_000 }).toBe(known.length + 1)
  return { outline, surface: outline.find((e) => e.kind === 'surface' && !e.deleted && !known.includes(e.entity_id)) }
}

test('OI-01 a .docx from the insert menu: a new flow page of rich text and picture Blocks, the original last', async ({ page, api }) => {
  test.setTimeout(120_000)
  const ws = await canvasWorkspace(api, `oi-01 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  const chooser = page.waitForEvent('filechooser')
  await mainMenu(page, 'aiws-menu-insert', 'aiws-menu-import-office')
  await (await chooser).setFiles({ name: 'basic.docx', mimeType: DOCX, buffer: fixture('docx/basic.docx') })
  const dialog = page.getByTestId('aiws-office-dialog')
  await expect(dialog.getByTestId('aiws-office-facts')).toBeVisible()
  await expect(dialog.getByTestId('aiws-office-mode-editable')).toBeChecked()
  await expect(dialog.getByTestId('aiws-office-keep')).toBeChecked()
  await page.screenshot({ path: `${SHOTS}/oi01-dialog.png` })
  await dialog.getByTestId('aiws-office-start').click()
  await expect(dialog).toHaveCount(0, { timeout: 60_000 })

  const { outline, surface } = await newSurface(api, ws.workspace_id, ['sf-o'])
  expect(surface.title).toBe('季度报告 Quarterly Report')
  expect(surface.layout?.mode).toBe('flow')
  const blocks = children(outline, surface.entity_id)
  expect(blocks.map((b) => b.view_type)).toEqual(['richtext', 'asset', 'richtext', 'asset'])
  // the first rich text: headings 1–3, marks, a link, nested bullets, a code block
  const text = await api.read(ALICE, ws.workspace_id, blocks[0].source_id)
  const content = JSON.stringify(text.content)
  for (const piece of ['"type":"heading"', '"level":3', '"type":"strong"', '"href":"https://example.com/report"', '"type":"bullet_list"', '"type":"code_block"', '"type":"blockquote"']) expect(content).toContain(piece)
  const table = JSON.stringify((await api.read(ALICE, ws.workspace_id, blocks[2].source_id)).content)
  expect(table).toContain('"type":"table"')
  expect(table).toContain('"header":1')
  // the picture: an asset entity in the page's content folder; the original file last
  const files = children(outline, surface.content_folder_id).filter((e) => e.type_id === 'buckyos.asset-ref')
  expect(files).toHaveLength(2)
  expect(files.map((f) => f.name ?? '').concat(files.map((f) => f.media_type ?? '')).join(' ')).toMatch(/png/)
  await expect(page.getByTestId('aiws-flow')).toBeVisible({ timeout: 20_000 })
  await page.screenshot({ path: `${SHOTS}/oi01-flow-page.png`, fullPage: true })
})

test('OI-02 a dropped .pptx asks, then becomes a canvas of page groups with a path; hidden slides left out', async ({ page, api }) => {
  test.setTimeout(180_000)
  const ws = await canvasWorkspace(api, `oi-02 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await drop(page, 'basic.pptx', PPTX, fixture('pptx/basic.pptx'), { x: 600, y: 400 })
  await expect(page.getByTestId('aiws-office-ask')).toBeVisible()
  await page.screenshot({ path: `${SHOTS}/oi02-ask.png` })
  await page.getByTestId('aiws-office-ask-import').click()
  const dialog = page.getByTestId('aiws-office-dialog')
  await expect(dialog.getByTestId('aiws-office-facts')).toContainText('9')
  await expect(dialog.getByTestId('aiws-office-hidden')).not.toBeChecked()
  await expect(dialog.getByTestId('aiws-office-path')).toBeChecked()
  await dialog.getByTestId('aiws-office-start').click()
  await expect(dialog).toHaveCount(0, { timeout: 120_000 })

  const { outline, surface } = await newSurface(api, ws.workspace_id, ['sf-o'])
  expect(surface.title).toBe('Office Import Deck')
  const groups = children(outline, surface.entity_id)
  expect(groups).toHaveLength(8)
  expect(groups.map((g) => [g.placement.w, g.placement.h])).toEqual(Array(8).fill([1920, 1080]))
  const frames = groups.map((g) => children(outline, g.entity_id)[0])
  expect(frames.every((f) => f.view_type === 'frame')).toBe(true)
  const last = await api.read(ALICE, ws.workspace_id, frames[7].entity_id)
  expect(last.content.payload.presentation).toEqual({ background: '#0f172a' })
  expect(last.content.payload.title).toBe('Dark background')
  // slide 3: a white-text rectangle, an SVG preset shape, a line
  const shapes = await Promise.all(children(outline, groups[2].entity_id).slice(1).map(async (e) => (await api.read(ALICE, ws.workspace_id, e.entity_id)).content.payload.config))
  expect(shapes.find((c) => c?.text === 'Blue box')).toMatchObject({ shape: 'rect', fill: '#2563eb', font_color: '#ffffff' })
  expect(shapes.some((c) => c?.shape === 'svg')).toBe(true)
  expect(shapes.some((c) => c?.shape === 'line')).toBe(true)
  // the presentation path: one step per slide
  const path = outline.find((e) => e.type_id === 'buckyos.show-path' && !e.deleted)
  expect((await api.read(ALICE, ws.workspace_id, path.entity_id)).content.payload.steps).toHaveLength(8)
  // the report (notes, crops …) closes before the canvas is used
  await page.getByTestId('aiws-import-report-close').click({ timeout: 20_000 }).catch(() => {})
  await page.waitForTimeout(800)
  await page.screenshot({ path: `${SHOTS}/oi02-canvas.png` })
})

test('OI-03 the report lists what was simplified; hidden slides on request; a chart is a picture with LibreOffice, a box without', async ({ page, api }) => {
  test.setTimeout(240_000)
  const ws = await canvasWorkspace(api, `oi-03 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  const chooser = page.waitForEvent('filechooser')
  await mainMenu(page, 'aiws-menu-insert', 'aiws-menu-import-office')
  await (await chooser).setFiles({ name: 'basic.pptx', mimeType: PPTX, buffer: fixture('pptx/basic.pptx') })
  const dialog = page.getByTestId('aiws-office-dialog')
  const withLibreOffice = await libreOffice(page)
  await expect(dialog.getByTestId('aiws-office-objects-note')).toBeVisible()
  await dialog.getByTestId('aiws-office-hidden').check()
  await dialog.getByTestId('aiws-office-path').uncheck()
  await dialog.getByTestId('aiws-office-start').click()
  await expect(dialog).toHaveCount(0, { timeout: 200_000 })
  const report = page.getByTestId('aiws-import-report')
  await expect(report).toBeVisible({ timeout: 20_000 })
  await expect(report.locator('[data-kind="pptx_notes_dropped"]')).toHaveCount(1)
  await expect(report.locator(withLibreOffice ? '[data-kind="pptx_rasterized"]' : '[data-kind="pptx_chart"]')).toHaveCount(1)
  await page.screenshot({ path: `${SHOTS}/oi03-report.png` })
  await page.getByTestId('aiws-import-report-close').click()
  const { outline, surface } = await newSurface(api, ws.workspace_id, ['sf-o'])
  const groups = children(outline, surface.entity_id)
  expect(groups).toHaveLength(9)
  expect(outline.some((e) => e.type_id === 'buckyos.show-path' && !e.deleted)).toBe(false)
  // slide 7: the chart
  const chart = children(outline, groups[6].entity_id)[2]
  if (withLibreOffice) expect(chart.view_type).toBe('asset')
  else expect((await api.read(ALICE, ws.workspace_id, chart.entity_id)).content.payload.config).toMatchObject({ shape: 'rect', dash: 'dashed' })
})

test('OI-04 a .docx inserted as a file expands into a document without a second upload', async ({ page, api }) => {
  test.setTimeout(120_000)
  const ws = await canvasWorkspace(api, `oi-04 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await drop(page, 'features.docx', DOCX, fixture('docx/features.docx'), { x: 600, y: 400 })
  await page.getByTestId('aiws-office-ask-file').click()
  await expect.poll(async () => children(await api.outline(ALICE, ws.workspace_id), 'sf-o').filter((e) => e.view_type === 'asset').length, { timeout: 20_000 }).toBe(1)
  const file = children(await api.outline(ALICE, ws.workspace_id), 'sf-o').find((e) => e.view_type === 'asset')
  await page.getByTestId(`aiws-canvas-block-${file.entity_id}`).click()
  await page.getByTestId('aiws-near-asset-expand-office').click()
  const dialog = page.getByTestId('aiws-office-dialog')
  await dialog.getByTestId('aiws-office-deleted').check()
  await dialog.getByTestId('aiws-office-start').click()
  await expect(dialog).toHaveCount(0, { timeout: 60_000 })
  const { outline, surface } = await newSurface(api, ws.workspace_id, ['sf-o'])
  const blocks = children(outline, surface.entity_id)
  expect(blocks[0].view_type).toBe('richtext')
  const content = JSON.stringify((await api.read(ALICE, ws.workspace_id, blocks[0].source_id)).content)
  expect(content).toContain('"text":"deleted "')
  expect(content).toContain('[1]')
  // the original is the file Block's object
  const fileRef = (await api.read(ALICE, ws.workspace_id, file.source_id)).content.payload.object_id
  const kept = children(outline, surface.content_folder_id).filter((e) => e.type_id === 'buckyos.asset-ref')
  const refs = await Promise.all(kept.map(async (k) => (await api.read(ALICE, ws.workspace_id, k.entity_id)).content.payload.object_id))
  expect(refs).toContain(fileRef)
})

test('OI-05 "as it looks": LibreOffice makes a PDF, the PDF importer takes it apart, the Office file stays the original', async ({ page, api }) => {
  test.setTimeout(240_000)
  const ws = await canvasWorkspace(api, `oi-05 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  const chooser = page.waitForEvent('filechooser')
  await mainMenu(page, 'aiws-menu-insert', 'aiws-menu-import-office')
  await (await chooser).setFiles({ name: 'basic.docx', mimeType: DOCX, buffer: fixture('docx/basic.docx') })
  const dialog = page.getByTestId('aiws-office-dialog')
  const withLibreOffice = await libreOffice(page)
  await expect(dialog.getByTestId('aiws-office-facts')).toBeVisible()
  if (!withLibreOffice) {
    await expect(dialog.getByTestId('aiws-office-mode-faithful')).toBeDisabled()
    return
  }
  await dialog.getByTestId('aiws-office-mode-faithful').check()
  await dialog.getByTestId('aiws-office-start').click()
  const pdf = page.getByTestId('aiws-pdf-dialog')
  await expect(pdf.getByTestId('aiws-pdf-pages')).toBeVisible({ timeout: 180_000 })
  await page.screenshot({ path: `${SHOTS}/oi05-pdf-dialog.png` })
  await pdf.getByTestId('aiws-pdf-start').click()
  await expect(pdf).toHaveCount(0, { timeout: 120_000 })
  const { outline, surface } = await newSurface(api, ws.workspace_id, ['sf-o'])
  expect(children(outline, surface.entity_id).length).toBeGreaterThanOrEqual(2)
  const files = await Promise.all(children(outline, surface.content_folder_id).filter((e) => e.type_id === 'buckyos.asset-ref').map(async (e) => (await api.read(ALICE, ws.workspace_id, e.entity_id)).content.payload.file_name))
  expect(files).toContain('basic.docx')
  expect(files.some((f) => /\.pdf$/.test(f))).toBe(false)
})

test('OI-06 a Word file with EMF / WMF pictures: pictures through LibreOffice, else left out and reported', async ({ page, api }) => {
  test.setTimeout(240_000)
  const ws = await canvasWorkspace(api, `oi-06 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  const chooser = page.waitForEvent('filechooser')
  await mainMenu(page, 'aiws-menu-insert', 'aiws-menu-import-office')
  await (await chooser).setFiles({ name: 'VariousPictures.docx', mimeType: DOCX, buffer: fixture('docx/real/VariousPictures.docx') })
  const dialog = page.getByTestId('aiws-office-dialog')
  const withLibreOffice = await libreOffice(page)
  await dialog.getByTestId('aiws-office-keep').uncheck()
  await dialog.getByTestId('aiws-office-start').click()
  await expect(dialog).toHaveCount(0, { timeout: 200_000 })
  const report = page.getByTestId('aiws-import-report')
  await expect(report).toBeVisible({ timeout: 20_000 })
  await expect(report.locator(withLibreOffice ? '[data-kind="docx_rasterized"]' : '[data-kind="docx_unsupported_image"]')).toHaveCount(1)
  if (!withLibreOffice) await expect(report.getByTestId('aiws-import-hint')).toBeVisible()
  await page.getByTestId('aiws-import-report-close').click()
  const { outline, surface } = await newSurface(api, ws.workspace_id, ['sf-o'])
  const pictures = children(outline, surface.entity_id).filter((b) => b.view_type === 'asset')
  expect(pictures).toHaveLength(withLibreOffice ? 5 : 2)
  await page.screenshot({ path: `${SHOTS}/oi06-flow.png`, fullPage: true })
})

test('OI-07 an .odt (no editable path) only offers "as it looks", which LibreOffice serves', async ({ page, api }) => {
  test.setTimeout(240_000)
  const ws = await canvasWorkspace(api, `oi-07 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await drop(page, 'basic.odt', 'application/vnd.oasis.opendocument.text', fixture('docx/basic.odt'), { x: 600, y: 400 })
  const ask = page.getByTestId('aiws-office-ask')
  await expect(ask).toBeVisible()
  const button = ask.getByTestId('aiws-office-ask-import')
  // enabled once the content server is known to have LibreOffice
  await expect.poll(async () => (await ask.getByTestId('aiws-office-no-libreoffice').count()) > 0 || await button.isEnabled(), { timeout: 20_000 }).toBe(true)
  if (!(await button.isEnabled())) { await ask.getByTestId('aiws-office-ask-file').click(); return }
  await page.screenshot({ path: `${SHOTS}/oi07-ask.png` })
  await button.click()
  const dialog = page.getByTestId('aiws-office-dialog')
  await expect(dialog.getByTestId('aiws-office-mode-editable')).toBeDisabled()
  await expect(dialog.getByTestId('aiws-office-mode-faithful')).toBeChecked()
  await dialog.getByTestId('aiws-office-start').click()
  const pdf = page.getByTestId('aiws-pdf-dialog')
  await expect(pdf.getByTestId('aiws-pdf-pages')).toBeVisible({ timeout: 180_000 })
  await pdf.getByTestId('aiws-pdf-start').click()
  await expect(pdf).toHaveCount(0, { timeout: 120_000 })
  const { outline, surface } = await newSurface(api, ws.workspace_id, ['sf-o'])
  const files = await Promise.all(children(outline, surface.content_folder_id).filter((e) => e.type_id === 'buckyos.asset-ref').map(async (e) => (await api.read(ALICE, ws.workspace_id, e.entity_id)).content.payload.file_name))
  expect(files).toContain('basic.odt')
})

test('OI-08 the same document imported twice: two pages, the second named apart', async ({ page, api }) => {
  test.setTimeout(120_000)
  const ws = await canvasWorkspace(api, `oi-08 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  for (let k = 0; k < 2; k++) {
    if (k === 1) await page.getByTestId('aiws-import-report-close').click({ timeout: 20_000 })
    const chooser = page.waitForEvent('filechooser')
    await mainMenu(page, 'aiws-menu-insert', 'aiws-menu-import-office')
    await (await chooser).setFiles({ name: 'basic.docx', mimeType: DOCX, buffer: fixture('docx/basic.docx') })
    const dialog = page.getByTestId('aiws-office-dialog')
    await dialog.getByTestId('aiws-office-start').click()
    await expect(dialog).toHaveCount(0, { timeout: 60_000 })
  }
  let outline: Json[] = []
  await expect.poll(async () => { outline = await api.outline(ALICE, ws.workspace_id); return outline.filter((e) => e.kind === 'surface' && !e.deleted).length }, { timeout: 60_000 }).toBe(3)
  const imported = outline.filter((e) => e.kind === 'surface' && !e.deleted && e.entity_id !== 'sf-o')
  expect(imported.map((s) => s.title)).toEqual(['季度报告 Quarterly Report', '季度报告 Quarterly Report'])
  expect(new Set(imported.map((s) => s.name)).size).toBe(2)
})
