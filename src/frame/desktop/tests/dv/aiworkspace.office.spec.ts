/* Office import on a real Zone (notepads/done/aiworkspace-office-import-todo.md O7): the Desktop as the Zone serves it —
 * the converter in its Worker, pictures uploaded through NDM, sub-documents and originals turned into PDFs by the
 * node's LibreOffice through nfs_server `repr` (preview-document), commits through the gateway — for a Word document
 * (a flow page; its EMF / WMF pictures as pictures), a PowerPoint deck (pages, a chart as a picture, a path) and the
 * "as it looks" path (LibreOffice → PDF importer, the .docx kept as the original). With `PREVIEW_DV_NO_SOFFICE=1`
 * (soffice moved away on the node) the same imports show placeholders, the hint, and no "as it looks". */

import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import type { Page } from '@playwright/test'
import type { EntityEnvelope, Operation, WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { expect, openWorkspace, screenshot, test, type Actor } from './aiworkspace.fixtures'

type Workspace = Pick<WorkspaceSummary, 'workspace_id' | 'epoch'>
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

test.use({ viewport: { width: 1600, height: 1000 } })

const FIXTURES = resolve('../aiworkspace/fixtures/office')
const NO_SOFFICE = process.env.PREVIEW_DV_NO_SOFFICE === '1'
const DOCX = 'application/vnd.openxmlformats-officedocument.wordprocessingml.document'
const PPTX = 'application/vnd.openxmlformats-officedocument.presentationml.presentation'

async function canvasWorkspace(actor: Actor): Promise<Workspace> {
  const created = await actor.client.ok<WorkspaceSummary>('ws.create', { title: actor.client.prefix })
  actor.client.owned.add(created.workspace_id)
  const ws = { workspace_id: created.workspace_id, epoch: created.epoch }
  const ops: Operation[] = [
    { op: 'entity.create', entity_id: 'sf-o-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'i', payload: { kind: 'folder', title: 'canvas', system: 'surface_content', surface_id: 'sf-o' } },
    { op: 'entity.create', entity_id: 'sf-o', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'i', payload: { kind: 'surface', layout: { mode: 'free' }, title: 'canvas', content_folder_id: 'sf-o-content' } },
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

/** Open the Office import from the insert menu with a fixture; the dialog and whether the node has LibreOffice. */
async function openImport(page: Page, path: string, type: string) {
  const chooser = page.waitForEvent('filechooser')
  await page.getByTestId('aiws-main-menu').click()
  await page.getByTestId('aiws-menu-insert').click()
  await page.getByTestId('aiws-menu-import-office').click()
  await (await chooser).setFiles({ name: path.split('/').pop()!, mimeType: type, buffer: readFileSync(`${FIXTURES}/${path}`) })
  const dialog = page.getByTestId('aiws-office-dialog')
  await expect(dialog.getByTestId('aiws-office-facts')).toBeVisible({ timeout: 60_000 })
  await expect(dialog).not.toHaveAttribute('data-libreoffice', 'unknown', { timeout: 30_000 })
  const libreOffice = (await dialog.getAttribute('data-libreoffice')) === 'true'
  expect(libreOffice, 'the node advertises preview-document unless soffice was moved away').toBe(!NO_SOFFICE)
  return { dialog, libreOffice }
}

/** Start, wait for the new page, read and close the report. */
async function finish(actor: Actor, ws: Workspace, dialog: ReturnType<Page['getByTestId']>, before: string[]) {
  const started = Date.now()
  await dialog.getByTestId('aiws-office-start').click()
  await expect(dialog).toHaveCount(0, { timeout: 300_000 })
  let all: EntityEnvelope[] = []
  await expect.poll(async () => { all = await outline(actor, ws); return all.filter((e) => e.kind === 'surface' && !e.deleted && !before.includes(e.entity_id)).length }, { timeout: 60_000 }).toBe(1)
  const surface = all.find((e) => e.kind === 'surface' && !e.deleted && !before.includes(e.entity_id))!
  const report = actor.page.getByTestId('aiws-import-report')
  const kinds = await report.isVisible() ? await report.locator('[data-kind]').evaluateAll((els) => els.map((e) => e.getAttribute('data-kind'))) : []
  const hint = await report.getByTestId('aiws-import-hint').isVisible().catch(() => false)
  return { surface, all, seconds: (Date.now() - started) / 1000, kinds, hint, report }
}

test('[office] a Word document with EMF / WMF pictures becomes a flow page', async ({ owner }, info) => {
  test.setTimeout(420_000)
  const ws = await canvasWorkspace(owner)
  await openCanvas(owner, ws)
  const { dialog, libreOffice } = await openImport(owner.page, 'docx/real/VariousPictures.docx', DOCX)
  await screenshot(owner.page, info, 'office-docx-dialog')
  const done = await finish(owner, ws, dialog, ['sf-o'])
  expect(done.kinds).toContain(libreOffice ? 'docx_rasterized' : 'docx_unsupported_image')
  expect(done.hint).toBe(!libreOffice)
  await screenshot(owner.page, info, 'office-docx-report')
  if (await done.report.isVisible()) await done.report.getByTestId('aiws-import-report-close').click()
  expect(done.surface.layout?.mode).toBe('flow')
  const blocks = children(done.all, done.surface.entity_id)
  expect(blocks.filter((b) => b.view_type === 'asset')).toHaveLength(libreOffice ? 6 : 3)
  await expect(owner.page.getByTestId('aiws-flow')).toBeVisible()
  await expect.poll(() => owner.page.evaluate(() => [...document.querySelectorAll('[data-testid="aiws-flow"] img')].filter((i) => (i as HTMLImageElement).complete && (i as HTMLImageElement).naturalWidth > 0).length), { timeout: 60_000 }).toBeGreaterThanOrEqual(libreOffice ? 5 : 2)
  await screenshot(owner.page, info, 'office-docx-flow')
  info.annotations.push({ type: 'timing', description: `docx ${done.seconds.toFixed(1)} s` })
})

test('[office] a PowerPoint deck becomes pages with a path; its chart a picture (or a box)', async ({ owner }, info) => {
  test.setTimeout(420_000)
  const ws = await canvasWorkspace(owner)
  await openCanvas(owner, ws)
  const { dialog, libreOffice } = await openImport(owner.page, 'pptx/basic.pptx', PPTX)
  const done = await finish(owner, ws, dialog, ['sf-o'])
  expect(done.kinds).toContain('pptx_notes_dropped')
  expect(done.kinds).toContain(libreOffice ? 'pptx_rasterized' : 'pptx_chart')
  if (await done.report.isVisible()) await done.report.getByTestId('aiws-import-report-close').click()
  const groups = children(done.all, done.surface.entity_id)
  expect(groups).toHaveLength(8)
  const chart = children(done.all, groups[6].entity_id)[2]
  if (libreOffice) expect(chart.view_type).toBe('asset')
  else expect((await read(owner, ws, chart.entity_id)).config).toMatchObject({ dash: 'dashed' })
  const path = done.all.find((e) => e.type_id === 'buckyos.show-path' && !e.deleted)!
  expect((await read(owner, ws, path.entity_id)).steps).toHaveLength(8)
  await owner.page.waitForTimeout(1500)
  await screenshot(owner.page, info, 'office-pptx-canvas')
  info.annotations.push({ type: 'timing', description: `pptx ${done.seconds.toFixed(1)} s` })
})

test('[office] "as it looks": LibreOffice draws the document, the PDF importer takes it apart', async ({ owner }, info) => {
  test.setTimeout(420_000)
  const ws = await canvasWorkspace(owner)
  await openCanvas(owner, ws)
  const { dialog, libreOffice } = await openImport(owner.page, 'docx/basic.docx', DOCX)
  if (!libreOffice) {
    await expect(dialog.getByTestId('aiws-office-mode-faithful')).toBeDisabled()
    await screenshot(owner.page, info, 'office-faithful-unavailable')
    return
  }
  await dialog.getByTestId('aiws-office-mode-faithful').check()
  await dialog.getByTestId('aiws-office-start').click()
  const pdf = owner.page.getByTestId('aiws-pdf-dialog')
  await expect(pdf.getByTestId('aiws-pdf-pages')).toBeVisible({ timeout: 240_000 })
  await screenshot(owner.page, info, 'office-faithful-pdf')
  await pdf.getByTestId('aiws-pdf-start').click()
  await expect(pdf).toHaveCount(0, { timeout: 240_000 })
  let all: EntityEnvelope[] = []
  await expect.poll(async () => { all = await outline(owner, ws); return all.filter((e) => e.kind === 'surface' && !e.deleted).length }, { timeout: 60_000 }).toBe(2)
  const surface = all.find((e) => e.kind === 'surface' && !e.deleted && e.entity_id !== 'sf-o')!
  const files = await Promise.all(children(all, surface.content_folder_id!).filter((e) => e.type_id === 'buckyos.asset-ref').map(async (e) => (await read(owner, ws, e.entity_id)).file_name))
  expect(files).toContain('basic.docx')
  const report = owner.page.getByTestId('aiws-import-report')
  if (await report.isVisible()) await report.getByTestId('aiws-import-report-close').click()
  await owner.page.waitForTimeout(1500)
  await screenshot(owner.page, info, 'office-faithful-canvas')
})
