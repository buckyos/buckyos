/* The PDF Frame control (notepads/aiws-print-pdf-export-todo.md §3.3, T3): inserted from the catalog with its two bands
 * (the canvas title in the header, page numbers in the footer, shown with sample values), text added to the header and a
 * page value inserted into it, a canvas Block dropped into the footer joining it, the page turned (the frame keeps its
 * shape and scale), and the frames as printing lists them (their geometry fits the pages of their shape only); on a
 * flow page, a frame an Agent wrote shows as a preview of its header and footer. */

import type { Page } from '@playwright/test'
import { pdfFrameOperations } from '@controls/pdfframe'
import { frameFits, paperOf } from '@print/page.ts'
import { framesIn } from '../../src/app/aiworkspace/ui/controls/pdfframe/model'
import type { EntityEnvelope } from '../../src/app/aiworkspace/api/types'
import { fitAll, openCanvas, test, expect } from './fixtures'

const ALICE = 'tok-alice'
const SHOTS = 'test-results/aiworkspace-pdfframe'

type Json = Record<string, unknown>

async function shot(page: Page, name: string) {
  await page.getByTestId('aiws-canvas-view').screenshot({ path: `${SHOTS}/${name}.png` })
}

async function box(page: Page, testId: string) {
  const b = await page.getByTestId(testId).boundingBox()
  if (!b) throw new Error(`${testId} is not on screen`)
  return b
}

test('PF01 a PDF Frame: inserted with its bands, page values in its header and footer, a Block dropped into the footer, turned, listed for printing', async ({ page, api }) => {
  const ws = await api.freeCanvas(ALICE, `pdf-frame ${Date.now()}`)
  // a label on the canvas, beside where the frame will be
  expect((await api.commit(ALICE, ws, [{ op: 'entity.create', entity_id: 'loose', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'a',
    placement: { x: -400, y: 0, w: 120, h: 28 }, payload: { view: { type: 'label' }, title: 'Confidential' } }])).status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  const outline = async () => (await api.outline(ALICE, ws.workspace_id)) as unknown as (EntityEnvelope & Json)[]

  // inserted from the catalog: the frame, its two bands, the canvas title in the header, page numbers in the footer
  await page.getByTestId('aiws-tool-catalog').click()
  await page.getByTestId('aiws-catalog-entry-control-pdf-frame').click()
  await page.getByTestId('aiws-catalog-insert').click()
  await expect.poll(async () => (await outline()).some((e) => e.kind === 'pdf-frame' && !e.deleted)).toBe(true)
  let all = await outline()
  const frame = all.find((e) => e.kind === 'pdf-frame')!
  const id = frame.entity_id
  expect(frame.config).toMatchObject({ page: 'A', orientation: 'portrait' })
  expect(frame.placement).toMatchObject({ w: 595, h: 842 })
  const band = (slot: string) => all.find((e) => e.parent_id === id && e.slot === slot)!
  const header = band('header').entity_id
  const footer = band('footer').entity_id
  const inBand = (b: string) => all.filter((e) => e.parent_id === b && !e.deleted)
  expect(inBand(header).map((e) => e.title)).toEqual(['{title}'])
  expect(inBand(footer).map((e) => e.title)).toEqual(['{page} / {pages}'])
  await fitAll(page)
  await expect(page.getByTestId(`aiws-pdfframe-${id}`)).toBeVisible()
  await expect(page.getByTestId(`aiws-pdfframe-content-${id}`)).toContainText('页面内容')
  // sample values while it is edited: page 1 of 12, the canvas's title
  await expect(page.getByTestId(`aiws-label-${inBand(footer)[0].entity_id}`)).toHaveText('1 / 12')
  await expect(page.getByTestId(`aiws-label-${inBand(header)[0].entity_id}`)).toHaveText('Free')
  const portrait = await box(page, `aiws-canvas-block-${id}`)
  expect(portrait.height / portrait.width).toBeCloseTo(842 / 595, 1)
  await shot(page, 'pf01-inserted')

  // text added to the header (the frame's action), then the page number inserted into it (the label's toolbar)
  const content = await box(page, `aiws-pdfframe-content-${id}`)
  await page.mouse.click(content.x + content.width / 2, content.y + content.height / 2, { button: 'right' })
  await page.getByTestId('aiws-context-menu').getByTestId('aiws-menu-pdfframe-header-text').click()
  const editor = page.locator('[data-testid^="aiws-label-edit-"]')
  await expect(editor).toBeVisible()
  await editor.pressSequentially('第')
  await editor.press('Enter')
  await expect.poll(async () => { all = await outline(); return inBand(header).map((e) => e.title) }).toEqual(['{title}', '第'])
  const added = inBand(header)[1].entity_id
  await expect(page.getByTestId(`aiws-canvas-block-${added}`)).toBeVisible()
  await page.getByTestId('aiws-near-label-page-value').click()
  await page.getByTestId('aiws-near-label-page-value-page').click()
  await expect.poll(async () => { all = await outline(); return all.find((e) => e.entity_id === added)?.title }).toBe('第{page}')
  await expect(page.getByTestId(`aiws-label-${added}`)).toHaveText('第1')
  // a label outside a frame keeps its braces: page values belong to frames and printed pages
  await expect(page.getByTestId('aiws-label-loose')).toHaveText('Confidential')

  // a canvas Block dropped on the footer joins it (placed relative to the band)
  const from = await box(page, 'aiws-canvas-block-loose')
  const to = await box(page, `aiws-pdfframe-footer-${id}`)
  await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2)
  await page.mouse.down()
  await page.mouse.move(to.x + to.width * 0.8, to.y + to.height / 2, { steps: 14 })
  await page.mouse.up()
  await expect.poll(async () => { all = await outline(); return all.find((e) => e.entity_id === 'loose')?.parent_id }).toBe(footer)
  const dropped = all.find((e) => e.entity_id === 'loose')!.placement as { x: number; y: number }
  expect(dropped.y).toBeGreaterThan(-28)
  expect(dropped.y).toBeLessThan(56)
  await shot(page, 'pf01-header-footer')

  // turned to landscape: its shape follows (A4 landscape at the same scale), the footer stays at the bottom
  await page.mouse.click(content.x + content.width / 2, content.y + content.height / 2)
  await page.getByTestId('aiws-near-pdfframe-orientation').click()
  await page.getByTestId('aiws-near-pdfframe-orientation-landscape').click()
  await expect.poll(async () => { all = await outline(); return all.find((e) => e.entity_id === id)?.placement }).toMatchObject({ w: 842, h: 595 })
  expect(all.find((e) => e.entity_id === id)?.config).toMatchObject({ orientation: 'landscape' })
  // the header's and footer's Blocks keep their side: the page number stays in the middle, the title on the left
  const placed = (e: Json) => e.placement as { x: number; w: number }
  const pageNumber = placed(inBand(footer).find((e) => e.title === '{page} / {pages}')!)
  expect(Math.abs(pageNumber.x + pageNumber.w / 2 - 421)).toBeLessThanOrEqual(1)
  expect(placed(inBand(header).find((e) => e.title === '{title}')!).x).toBe(56)
  await expect.poll(async () => { const b = await box(page, `aiws-canvas-block-${id}`); return Math.round((b.width / b.height) * 100) / 100 }).toBe(1.42)
  const frameBox = await box(page, `aiws-canvas-block-${id}`)
  const footerBox = await box(page, `aiws-pdfframe-footer-${id}`)
  expect(Math.abs(footerBox.y + footerBox.height - (frameBox.y + frameBox.height))).toBeLessThan(3)

  // what printing lists: the frame on its canvas, its box and content area; it fits A4 and A5 landscape, not Letter
  const frames = framesIn(all)
  expect(frames.map((f) => [f.id, f.surfaceId])).toEqual([[id, 'free']])
  const g = frames[0].geometry
  expect([g.w, Math.round(g.h)]).toEqual([842, 595])
  expect(g.content).toMatchObject({ x: 56, y: 56, w: 730 })
  expect(Math.round(g.content.h)).toBe(483)
  expect(frameFits(g, paperOf('A4', 'landscape'))).toBe(true)
  expect(frameFits(g, paperOf('A5', 'landscape'))).toBe(true)
  expect(frameFits(g, paperOf('A4', 'portrait'))).toBe(false)
  expect(frameFits(g, paperOf('Letter', 'landscape'))).toBe(false)

  // another shape: a slide comes landscape, at the same scale; it then fits a 16:9 page only
  await page.getByTestId('aiws-near-pdfframe-page').click()
  await page.getByTestId('aiws-near-pdfframe-page-16:9').click()
  await expect.poll(async () => { all = await outline(); return all.find((e) => e.entity_id === id)?.placement }).toMatchObject({ w: 842, h: 474 })
  expect(all.find((e) => e.entity_id === id)?.config).toMatchObject({ page: '16:9', orientation: 'landscape' })
  const slide = framesIn(all)[0].geometry
  expect(frameFits(slide, paperOf('16:9', 'landscape'))).toBe(true)
  expect(frameFits(slide, paperOf('A4', 'landscape'))).toBe(false)
  await expect(page.getByTestId(`aiws-pdfframe-content-${id}`)).toContainText('16:9 · 横向')
  await fitAll(page)
  await shot(page, 'pf01-slide')
})

test('PF02 a PDF Frame on a flow page (written by an Agent): a preview of its header and footer with sample values', async ({ page, api }) => {
  const ws = await api.rpc(ALICE, 'ws.create', { title: `pdf-frame flow ${Date.now()}` }) as { workspace_id: string; epoch: string }
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'page-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a', payload: { kind: 'folder', system: 'surface_content', surface_id: 'page', title: 'Report' } },
    { op: 'entity.create', entity_id: 'page', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a', payload: { kind: 'surface', layout: { mode: 'flow' }, title: 'Report', content_folder_id: 'page-content' } },
    ...pdfFrameOperations({ id: 'pf', parentId: 'page', orderKey: 'm', x: 0, y: 0, config: { page: '16:9' }, title: 'Slides', headerId: 'pf-h', footerId: 'pf-f' }),
    { op: 'entity.create', entity_id: 'pn', type_id: 'buckyos.cell', parent_id: 'pf-f', order_key: 'a', placement: { x: 600, y: 16, w: 180, h: 24 },
      payload: { view: { type: 'label' }, title: '{title} · {page}', config: { align: 'right', font_size: 10 } } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  const preview = page.getByTestId('aiws-pdfframe-flow-pf')
  await expect(preview).toBeVisible()
  await expect(preview.getByTestId('aiws-label-pn')).toHaveText('Report · 1')
  const art = await preview.locator('.aiws-pdfframe-art').boundingBox()
  expect(Math.round((art!.width / art!.height) * 100) / 100).toBeCloseTo(16 / 9, 1)
  const frames = framesIn((await api.outline(ALICE, ws.workspace_id)) as unknown as EntityEnvelope[])
  expect(frames.map((f) => [f.id, f.title, f.surfaceId, Math.round(f.geometry.w), Math.round(f.geometry.h)])).toEqual([['pf', 'Slides', 'page', 842, 474]])
})
