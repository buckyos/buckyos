/* PDF import on a real Zone (doc/workspace/BuckyOS AI Workspace PDF 导入与放映方案讨论.md §10): the Desktop as the
 * Zone serves it — pdf.js with the CMaps and standard fonts under aiworkspace/pdfjs/, pictures and rasters uploaded
 * through NDM, the pages committed through the gateway — for a made deck (mixed page sizes, a predefined CMap, a
 * blend-mode raster), the Chromium-printed slides fixture (gradient, SVG drawing, embedded CJK fonts) and, when the
 * machine has it, a real LaTeX manual (6 MB, kept as the original). */

import { existsSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import type { Page } from '@playwright/test'
import type { EntityEnvelope, Operation, WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { deck } from '../aiworkspace/pdfFiles'
import { expect, openWorkspace, screenshot, test, type Actor } from './aiworkspace.fixtures'

type Workspace = Pick<WorkspaceSummary, 'workspace_id' | 'epoch'>
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

test.use({ viewport: { width: 1600, height: 1000 } })

const SLIDES = resolve('../aiworkspace/fixtures/pdf/slides.pdf')
const MANUAL = '/usr/share/pve-docs/pve-admin-guide.pdf'

async function canvasWorkspace(actor: Actor): Promise<Workspace> {
  const created = await actor.client.ok<WorkspaceSummary>('ws.create', { title: actor.client.prefix })
  actor.client.owned.add(created.workspace_id)
  const ws = { workspace_id: created.workspace_id, epoch: created.epoch }
  const ops: Operation[] = [
    { op: 'entity.create', entity_id: 'sf-p-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'i', payload: { kind: 'folder', title: 'canvas', system: 'surface_content', surface_id: 'sf-p' } },
    { op: 'entity.create', entity_id: 'sf-p', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'i', payload: { kind: 'surface', layout: { mode: 'free' }, title: 'canvas', content_folder_id: 'sf-p-content' } },
  ]
  const result = await actor.client.commit(ws, ops)
  expect(result.status, JSON.stringify(result)).toBe('accepted')
  return ws
}

const outline = async (actor: Actor, ws: Workspace) => (await actor.client.ok<{ entities: EntityEnvelope[] }>('doc.outline', { workspace_id: ws.workspace_id })).entities
const read = async (actor: Actor, ws: Workspace, id: string) => (await actor.client.ok<{ content: { payload: Json } }>('doc.read', { workspace_id: ws.workspace_id, entity_id: id })).content.payload
const children = (all: EntityEnvelope[], parent: string) => all.filter((e) => e.parent_id === parent && !e.deleted).sort((a, b) => ((a.order_key ?? '') < (b.order_key ?? '') ? -1 : 1))

async function openCanvas(actor: Actor, ws: Workspace) {
  await openWorkspace(actor, ws)
  const top = actor.page.getByTestId('aiws-top-canvas')
  if (await top.isVisible()) await top.click()
  await expect(actor.page.getByTestId('aiws-canvas-view')).toBeVisible()
}

/** Import a PDF from the main menu; `setup` adjusts the dialog before starting. Returns the new canvas. */
async function importPdf(actor: Actor, ws: Workspace, file: { name: string; buffer: Buffer }, setup: (dialog: ReturnType<Page['getByTestId']>) => Promise<void> = async () => {}) {
  const page = actor.page
  const before = (await outline(actor, ws)).filter((e) => e.kind === 'surface' && !e.deleted).map((e) => e.entity_id)
  const chooser = page.waitForEvent('filechooser')
  await page.getByTestId('aiws-main-menu').click()
  await page.getByTestId('aiws-menu-insert').click()
  await page.getByTestId('aiws-menu-import-pdf').click()
  await (await chooser).setFiles({ name: file.name, mimeType: 'application/pdf', buffer: file.buffer })
  const dialog = page.getByTestId('aiws-pdf-dialog')
  await expect(dialog.getByTestId('aiws-pdf-pages')).toBeVisible({ timeout: 60_000 })
  await setup(dialog)
  const started = Date.now()
  await dialog.getByTestId('aiws-pdf-start').click()
  await expect(dialog).toHaveCount(0, { timeout: 240_000 })
  let all: EntityEnvelope[] = []
  await expect.poll(async () => { all = await outline(actor, ws); return all.filter((e) => e.kind === 'surface' && !e.deleted && !before.includes(e.entity_id)).length }, { timeout: 60_000 }).toBe(1)
  const surface = all.find((e) => e.kind === 'surface' && !e.deleted && !before.includes(e.entity_id))!
  // the report (what was simplified) is shown when there is something to say
  const report = page.getByTestId('aiws-import-report')
  const notes = await report.isVisible() ? await report.getByTestId('aiws-import-notes').innerText().catch(() => '') : ''
  if (await report.isVisible()) await report.getByTestId('aiws-import-report-close').click()
  return { surface, all, seconds: (Date.now() - started) / 1000, notes }
}

/** Select a page (a click on its paper, left of everything drawn on it) and zoom onto it. */
async function zoomTo(page: Page, groupId: string) {
  await page.getByTestId('aiws-zoom-menu').click()
  await page.getByTestId('aiws-fit-all').click()
  await page.waitForTimeout(400)
  const b = (await page.getByTestId(`aiws-canvas-block-${groupId}`).boundingBox())!
  await page.mouse.click(b.x + 3, b.y + b.height / 2)
  await expect(page.getByTestId(`aiws-selection-${groupId}`)).toBeVisible()
  await page.getByTestId('aiws-zoom-menu').click()
  await page.getByTestId('aiws-fit-selection').click()
  await page.waitForTimeout(600)
}

/** Every picture of the canvas being shown has loaded (decoded from the Zone's storage). */
async function picturesLoaded(page: Page) {
  await expect.poll(() => page.evaluate(() => {
    const imgs = [...document.querySelectorAll('[data-testid="aiws-canvas"] img')] as HTMLImageElement[]
    return imgs.length > 0 && imgs.every((img) => img.complete && img.naturalWidth > 0)
  }), { timeout: 60_000 }).toBe(true)
}

test('[pdf] a made deck on a real Zone: CMap text, picture, raster, original, path, show', async ({ owner }, info) => {
  test.setTimeout(300_000)
  const page = owner.page
  const ws = await canvasWorkspace(owner)
  await openCanvas(owner, ws)
  const { surface, all, seconds, notes } = await importPdf(owner, ws, { name: 'deck.pdf', buffer: deck() }, async (dialog) => {
    await expect(dialog.getByTestId('aiws-pdf-estimate')).toContainText(/2 (段文字|text objects).*1 (张图片|image).*1 (个图形|shape)/)
    await screenshot(page, info, 'pdf-deck-dialog')
  })
  const groups = children(all, surface.entity_id)
  expect(groups.map((g) => g.kind)).toEqual(['group', 'group', 'group'])
  const texts = await Promise.all(children(all, groups[1].entity_id).map(async (e) => (await read(owner, ws, e.entity_id)).config?.text))
  // the CMap came from the Zone (aiworkspace/pdfjs/cmaps/UniGB-UCS2-H.bcmap)
  expect(texts).toContain('中文用例')
  const page3 = await Promise.all(children(all, groups[2].entity_id).map(async (e) => ({ e, payload: await read(owner, ws, e.entity_id) })))
  const raster = page3.find((r) => r.payload.config?.pdf_source?.kind === 'raster')
  expect(raster?.e.view_type).toBe('asset')
  const original = children(all, surface.content_folder_id!).find((e) => e.media_type === 'application/pdf')
  expect(original, 'the original PDF kept').toBeTruthy()
  const path = all.find((e) => e.type_id === 'buckyos.show-path' && !e.deleted)!
  const steps = (await read(owner, ws, path.entity_id)).steps as Json[]
  expect(steps.map((s) => s.transition)).toEqual(['cut', 'cut', 'cut'])
  expect(notes).toMatch(/p3/)
  await info.attach('timing', { body: Buffer.from(JSON.stringify({ seconds, notes })), contentType: 'application/json' })

  // the new canvas shows the pages; pictures (the picture and the raster) load from the Zone
  await expect(page.getByTestId(`aiws-canvas-block-${groups[2].entity_id}`)).toBeVisible()
  await screenshot(page, info, 'pdf-deck-canvas')
  await zoomTo(page, groups[2].entity_id)
  await picturesLoaded(page)
  await screenshot(page, info, 'pdf-deck-page3')
  await zoomTo(page, groups[0].entity_id)
  await picturesLoaded(page)
  await screenshot(page, info, 'pdf-deck-page1')

  // after a reload the canvas is the same, and plays
  await page.reload()
  await openWorkspace(owner, ws)
  await expect(page.getByTestId(`aiws-canvas-block-${groups[0].entity_id}`)).toBeVisible({ timeout: 30_000 })
  await page.getByTestId('aiws-main-menu').click()
  await page.getByTestId('aiws-top-play').click()
  await page.getByTestId('aiws-start-show-go').click()
  await expect(page.getByTestId('aiws-show')).toBeVisible({ timeout: 30_000 })
  await expect(page.getByTestId('aiws-show')).toHaveAttribute('data-step', steps[0].id)
  await picturesLoaded(page).catch(() => {})
  await page.waitForTimeout(500)
  await screenshot(page, info, 'pdf-deck-show-1')
  await page.keyboard.press('ArrowRight')
  await page.keyboard.press('ArrowRight')
  await expect(page.getByTestId('aiws-show')).toHaveAttribute('data-step', steps[2].id)
  await page.waitForTimeout(500)
  await screenshot(page, info, 'pdf-deck-show-3')
  await page.keyboard.press('Escape')
})

test('[pdf] the Chromium-printed slides: gradient, SVG drawing, list items, embedded CJK', async ({ owner }, info) => {
  test.setTimeout(300_000)
  const page = owner.page
  const ws = await canvasWorkspace(owner)
  await openCanvas(owner, ws)
  const { surface, all } = await importPdf(owner, ws, { name: 'slides.pdf', buffer: readFileSync(SLIDES) })
  const groups = children(all, surface.entity_id)
  expect(groups).toHaveLength(2)
  const page1 = await Promise.all(children(all, groups[0].entity_id).map(async (e) => ({ e, payload: await read(owner, ws, e.entity_id) })))
  expect(page1.find((r) => r.payload.config?.text === '季度回顾 Quarterly Review')?.payload.config.font_color).toBe('#ffffff')
  expect(page1.filter((r) => r.payload.config?.pdf_source?.role === 'LI')).toHaveLength(3)
  expect(page1.some((r) => String(r.payload.config?.svg?.body ?? '').includes('linearGradient'))).toBe(true)
  expect((await read(owner, ws, children(all, groups[0].entity_id)[0].entity_id)).presentation).toEqual({ background: '#1e3a5f' })
  await expect(page.getByTestId(`aiws-canvas-block-${groups[1].entity_id}`)).toBeVisible()
  await screenshot(page, info, 'pdf-slides-canvas')
  await zoomTo(page, groups[0].entity_id)
  await picturesLoaded(page)
  await screenshot(page, info, 'pdf-slides-page1')
  await zoomTo(page, groups[1].entity_id)
  await screenshot(page, info, 'pdf-slides-page2')
})

test('[pdf] a real LaTeX manual (6 MB): eight pages, bookmarks as page names, the original kept', async ({ owner }, info) => {
  test.skip(!existsSync(MANUAL), `${MANUAL} is not on this machine`)
  test.setTimeout(400_000)
  const page = owner.page
  const ws = await canvasWorkspace(owner)
  await openCanvas(owner, ws)
  const { surface, all, seconds, notes } = await importPdf(owner, ws, { name: 'pve-admin-guide.pdf', buffer: readFileSync(MANUAL) }, async (dialog) => {
    await expect(dialog.getByTestId('aiws-pdf-pages')).toContainText(/685/)
    await dialog.getByTestId('aiws-pdf-from').fill('21')
    await dialog.getByTestId('aiws-pdf-to').fill('28')
    await dialog.getByTestId('aiws-pdf-columns').fill('4')
  })
  const groups = children(all, surface.entity_id)
  expect(groups).toHaveLength(8)
  const frames = await Promise.all(groups.map(async (g) => (await read(owner, ws, children(all, g.entity_id)[0].entity_id)).title as string))
  const objects = groups.reduce((n, g) => n + children(all, g.entity_id).length, 0)
  const original = children(all, surface.content_folder_id!).find((e) => e.media_type === 'application/pdf')
  expect(original, 'a 6 MB original is kept').toBeTruthy()
  await info.attach('manual', { body: Buffer.from(JSON.stringify({ seconds, frames, objects, notes })), contentType: 'application/json' })
  expect(objects).toBeGreaterThan(100)
  await expect(page.getByTestId(`aiws-canvas-block-${groups[0].entity_id}`)).toBeVisible()
  await screenshot(page, info, 'pdf-manual-canvas')
  // zoomed onto one page: real text objects
  await zoomTo(page, groups[1].entity_id)
  await screenshot(page, info, 'pdf-manual-page')
})
