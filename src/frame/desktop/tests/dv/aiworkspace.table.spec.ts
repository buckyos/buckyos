/* Table data import on a real Zone (notepads/aiworkspace-table-import-todo.md §11: TI-01, TI-05, TI-08 on a Zone):
 * the Desktop the Zone serves, the wizard's reader Worker (xlsx / SQLite), the original file uploaded through NDM,
 * commits through the gateway — including one commit of 20 000 rows × 20 columns (the gateway's share of T0).
 * Assertions do not depend on the Desktop's language. */

import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import type { Page } from '@playwright/test'
import type { EntityEnvelope, Operation, WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { expect, openWorkspace, screenshot, test, type Actor } from './aiworkspace.fixtures'

type Workspace = Pick<WorkspaceSummary, 'workspace_id' | 'epoch'>
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

test.use({ viewport: { width: 1600, height: 1000 } })

const FIXTURES = resolve('../aiworkspace/fixtures/table')
const fixture = (path: string) => readFileSync(`${FIXTURES}/${path}`)

async function canvasWorkspace(actor: Actor): Promise<Workspace> {
  const created = await actor.client.ok<WorkspaceSummary>('ws.create', { title: actor.client.prefix })
  actor.client.owned.add(created.workspace_id)
  const ws = { workspace_id: created.workspace_id, epoch: created.epoch }
  const ops: Operation[] = [
    { op: 'entity.create', entity_id: 'sf-t-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'i', payload: { kind: 'folder', title: 'canvas', system: 'surface_content', surface_id: 'sf-t' } },
    { op: 'entity.create', entity_id: 'sf-t', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'i', payload: { kind: 'surface', layout: { mode: 'free' }, title: 'canvas', content_folder_id: 'sf-t-content' } },
  ]
  const result = await actor.client.commit(ws, ops)
  expect(result.status, JSON.stringify(result)).toBe('accepted')
  return ws
}

const outline = async (actor: Actor, ws: Workspace) => (await actor.client.ok<{ entities: EntityEnvelope[] }>('doc.outline', { workspace_id: ws.workspace_id })).entities
const children = (all: EntityEnvelope[], parent: string) => all.filter((e) => e.parent_id === parent && !e.deleted).sort((a, b) => ((a.order_key ?? '') < (b.order_key ?? '') ? -1 : 1))
const tables = async (actor: Actor, ws: Workspace) => children(await outline(actor, ws), 'sf-t-content').filter((e) => e.type_id === 'buckyos.table-source')
const query = (actor: Actor, ws: Workspace, sourceId: string, extra: Json = {}) => actor.client.ok<{ rows: { values: Json }[]; total: number }>('doc.query', { workspace_id: ws.workspace_id, source_id: sourceId, limit: 1000, ...extra })
const fields = async (actor: Actor, ws: Workspace, sourceId: string) => (await actor.client.ok<{ content: { fields: Json[] } }>('doc.read', { workspace_id: ws.workspace_id, entity_id: sourceId })).content.fields

async function openCanvas(actor: Actor, ws: Workspace) {
  await openWorkspace(actor, ws)
  const top = actor.page.getByTestId('aiws-top-canvas')
  if (await top.isVisible()) await top.click()
  await expect(actor.page.getByTestId('aiws-canvas-view')).toBeVisible()
}

async function paste(page: Page, data: { html?: string; text?: string }) {
  await page.evaluate(({ html, text }) => {
    const transfer = new DataTransfer()
    if (html) transfer.setData('text/html', html)
    if (text) transfer.setData('text/plain', text)
    document.body.dispatchEvent(new ClipboardEvent('paste', { clipboardData: transfer, bubbles: true, cancelable: true }))
  }, data)
}

async function drop(page: Page, name: string, bytes: Buffer, at: { x: number; y: number }) {
  const dt = await page.evaluateHandle(({ b64, name }) => {
    const bin = atob(b64)
    const data = new Uint8Array(bin.length)
    for (let i = 0; i < bin.length; i++) data[i] = bin.charCodeAt(i)
    const transfer = new DataTransfer()
    transfer.items.add(new File([data], name, { type: 'text/csv' }))
    return transfer
  }, { b64: bytes.toString('base64'), name })
  const target = page.locator('.aiws-canvas-main')
  await target.dispatchEvent('dragover', { dataTransfer: dt, clientX: at.x, clientY: at.y })
  await target.dispatchEvent('drop', { dataTransfer: dt, clientX: at.x, clientY: at.y })
}

async function importFromMenu(page: Page, name: string, buffer: Buffer, mimeType: string) {
  const chooser = page.waitForEvent('filechooser')
  await page.getByTestId('aiws-main-menu').click()
  await page.getByTestId('aiws-menu-insert').click()
  await page.getByTestId('aiws-menu-import-table').click()
  await (await chooser).setFiles({ name, mimeType, buffer })
}

async function closeReport(page: Page) {
  const report = page.getByTestId('aiws-import-report')
  if (await report.isVisible().catch(() => false)) await report.getByTestId('aiws-import-report-close').click()
}

test('[table] TI-01 Excel HTML pasted on a Zone canvas: types recognised, the table Block with its values', async ({ owner }, info) => {
  const page = owner.page
  const ws = await canvasWorkspace(owner)
  await openCanvas(owner, ws)
  await paste(page, { html: fixture('clipboard/excel.html').toString('utf8'), text: fixture('clipboard/excel.txt').toString('utf8') })
  const dialog = page.getByTestId('aiws-table-dialog')
  await expect.poll(() => Promise.all([0, 1, 2, 3].map((i) => dialog.getByTestId(`aiws-table-type-${i}`).inputValue()))).toEqual(['text', 'currency', 'date', 'boolean'])
  await screenshot(page, info, 'table-ti01-wizard')
  await dialog.getByTestId('aiws-table-start').click()
  await expect(dialog).toHaveCount(0, { timeout: 60_000 })
  await expect.poll(async () => (await tables(owner, ws)).length, { timeout: 30_000 }).toBe(1)
  const [source] = await tables(owner, ws)
  expect((await query(owner, ws, source.entity_id)).rows.map((r) => r.values)).toEqual([
    { c1: 'Apple', c2: { amount: '1234.50', currency: 'USD' }, c3: '2026-07-01', c4: true },
    { c1: 'Pear', c2: { amount: '980.00', currency: 'USD' }, c3: '2026-07-02', c4: false },
    { c1: 'Plum', c2: { amount: '15.25', currency: 'USD' }, c3: '2026-07-15', c4: true },
  ])
  const block = children(await outline(owner, ws), 'sf-t').find((e) => e.view_type === 'table')!
  await expect(page.getByTestId(`aiws-canvas-block-${block.entity_id}`)).toContainText('1,234.50')
  await screenshot(page, info, 'table-ti01-canvas')
})

test('[table] TI-05 a CSV dropped on the Zone canvas: ask, then a table; again, then a file', async ({ owner }, info) => {
  const page = owner.page
  const ws = await canvasWorkspace(owner)
  await openCanvas(owner, ws)
  await drop(page, 'select.csv', fixture('csv/select.csv'), { x: 600, y: 400 })
  await page.getByTestId('aiws-table-ask').getByTestId('aiws-table-ask-table').click()
  await page.getByTestId('aiws-table-dialog').getByTestId('aiws-table-start').click()
  await expect.poll(async () => (await tables(owner, ws)).map((t) => t.name), { timeout: 30_000 }).toEqual(['select'])
  await drop(page, 'select.csv', fixture('csv/select.csv'), { x: 300, y: 300 })
  await page.getByTestId('aiws-table-ask').getByTestId('aiws-table-ask-file').click()
  await expect.poll(async () => children(await outline(owner, ws), 'sf-t').map((e) => e.view_type).sort(), { timeout: 60_000 }).toEqual(['asset', 'table'])
  await screenshot(page, info, 'table-ti05-canvas')
})

test('[table] TI-08 money on a Zone: codes, names, symbols, a chosen default; order and filter by currency', async ({ owner }, info) => {
  const page = owner.page
  const ws = await canvasWorkspace(owner)
  await openCanvas(owner, ws)
  await importFromMenu(page, 'prices.csv', fixture('csv/currency-mixed.csv'), 'text/csv')
  const dialog = page.getByTestId('aiws-table-dialog')
  await expect(dialog.getByTestId('aiws-table-type-1')).toHaveValue('currency')
  await dialog.getByTestId('aiws-table-currency-1').selectOption('USD')
  await dialog.getByTestId('aiws-table-yen-1').selectOption('CNY')
  await screenshot(page, info, 'table-ti08-wizard')
  await dialog.getByTestId('aiws-table-start').click()
  await expect(dialog).toHaveCount(0, { timeout: 60_000 })
  await closeReport(page)
  const [source] = await tables(owner, ws)
  expect((await fields(owner, ws, source.entity_id))[1]).toMatchObject({ type: 'currency', currency: 'USD' })
  expect((await query(owner, ws, source.entity_id)).rows.map((r) => r.values.c2)).toEqual([
    { amount: '200.00', currency: 'USD' }, { amount: '500.00', currency: 'CNY' }, { amount: '300.00', currency: 'CNY' }, { amount: '10.00', currency: 'EUR' },
  ])
  expect((await query(owner, ws, source.entity_id, { sorts: [{ field_id: 'c2', direction: 'asc' }] })).rows.map((r) => r.values.c1)).toEqual(['c', 'b', 'd', 'a'])
  expect((await query(owner, ws, source.entity_id, { filter: { op: 'cmp', field_id: 'c2', operator: 'gt', value: 100 } })).rows.map((r) => r.values.c1)).toEqual(['a'])
})

test('[table] an xlsx workbook through the reader Worker, one table per sheet, the original file uploaded to the Zone', async ({ owner }, info) => {
  test.setTimeout(180_000)
  const page = owner.page
  const ws = await canvasWorkspace(owner)
  await openCanvas(owner, ws)
  await importFromMenu(page, 'workbook.xlsx', fixture('xlsx/workbook.xlsx'), 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet')
  const dialog = page.getByTestId('aiws-table-dialog')
  await expect(dialog.getByTestId('aiws-table-type-1')).toHaveValue('currency', { timeout: 30_000 })
  await dialog.getByTestId('aiws-table-advanced').click()
  await expect(dialog.getByTestId('aiws-table-keep')).toBeChecked()
  await dialog.getByTestId('aiws-table-all-sheets').check()
  await expect(dialog.getByRole('tab')).toHaveCount(2)
  await dialog.getByTestId('aiws-table-start').click()
  await expect(dialog).toHaveCount(0, { timeout: 120_000 })
  await closeReport(page)
  const all = children(await outline(owner, ws), 'sf-t-content')
  expect(all.filter((e) => e.type_id === 'buckyos.table-source').map((e) => e.name)).toEqual(['Sales', 'Events'])
  expect(all.filter((e) => e.type_id === 'buckyos.asset-ref')).toHaveLength(1)
  await screenshot(page, info, 'table-xlsx-canvas')
})

test('[table] 20 000 rows × 20 columns through the gateway: commits of up to 6 MiB accepted (T0)', async ({ owner }, info) => {
  test.setTimeout(600_000)
  const page = owner.page
  const ws = await canvasWorkspace(owner)
  await openCanvas(owner, ws)
  const header = Array.from({ length: 20 }, (_, i) => (i < 10 ? `text${i + 1}` : `num${i + 1}`)).join(',')
  const lines = [header]
  for (let r = 0; r < 20_000; r++) lines.push(Array.from({ length: 20 }, (_, i) => (i < 10 ? `row ${r} col ${i}` : i % 2 ? String(r * 7 + i) : `${r}.${String(i).padStart(2, '0')}`)).join(','))
  const csv = Buffer.from(lines.join('\n'))
  await importFromMenu(page, 'wide.csv', csv, 'text/csv')
  const dialog = page.getByTestId('aiws-table-dialog')
  await expect(dialog.getByTestId('aiws-table-start')).toBeEnabled({ timeout: 60_000 })
  const started = Date.now()
  await dialog.getByTestId('aiws-table-start').click()
  await expect(dialog).toHaveCount(0, { timeout: 540_000 })
  const seconds = (Date.now() - started) / 1000
  await closeReport(page)
  const [source] = await tables(owner, ws)
  expect((await query(owner, ws, source.entity_id, { limit: 1 })).total).toBe(20_000)
  await info.attach('timing', { body: Buffer.from(JSON.stringify({ seconds, bytes: csv.length })), contentType: 'application/json' })
  await screenshot(page, info, 'table-wide-canvas')
})
