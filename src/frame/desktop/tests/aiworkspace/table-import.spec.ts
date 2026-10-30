/* Table data import (notepads/aiworkspace-table-import-todo.md §11, TI-01 – TI-12): pasted spreadsheet HTML and TSV,
 * CSV / xlsx / SQLite files through the drop, the insert catalog and the main menu, the wizard's header switch, types,
 * currencies and invalid values, flow pages and undo, more than 20 000 rows, and aiws's own "copy as table".
 * The fixtures are those of the shared importer (src/frame/aiworkspace/fixtures/table). Screenshots go to
 * test-results/aiworkspace-table/. */

import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import type { Page } from '@playwright/test'
import { mainMenu, openCanvas, test, expect, type Api } from './fixtures'

const ALICE = 'tok-alice'
const SHOTS = 'test-results/aiworkspace-table'
const FIXTURES = resolve('../aiworkspace/fixtures/table')

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any
type Ws = { workspace_id: string; epoch: string }

async function canvasWorkspace(api: Api, title: string, layout: 'free' | 'flow' = 'free'): Promise<Ws> {
  const ws = await api.rpc(ALICE, 'ws.create', { title })
  await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'sf-t-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'i', payload: { kind: 'folder', title: '画布', system: 'surface_content', surface_id: 'sf-t' } },
    { op: 'entity.create', entity_id: 'sf-t', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'i', payload: { kind: 'surface', layout: { mode: layout }, title: '画布', content_folder_id: 'sf-t-content' } },
  ])
  return ws
}

const children = (outline: Json[], parent: string) => outline.filter((e) => e.parent_id === parent && !e.deleted).sort((a, b) => (a.order_key < b.order_key ? -1 : 1))
const tablesIn = async (api: Api, ws: Ws, folder = 'sf-t-content') => children(await api.outline(ALICE, ws.workspace_id), folder).filter((e) => e.type_id === 'buckyos.table-source')
async function rows(api: Api, ws: Ws, sourceId: string, extra: Json = {}): Promise<Json[]> {
  const out: Json[] = []
  let cursor: string | undefined
  for (;;) {
    const page = await api.rpc(ALICE, 'doc.query', { workspace_id: ws.workspace_id, source_id: sourceId, limit: 1000, ...extra, ...(cursor ? { cursor } : {}) })
    out.push(...page.rows)
    if (!page.next_cursor || page.rows.length === 0) return out
    cursor = page.next_cursor
  }
}
const fieldsOf = async (api: Api, ws: Ws, sourceId: string) => (await api.read(ALICE, ws.workspace_id, sourceId)).content.fields as Json[]

/** A paste on the canvas (the clipboard's HTML and plain text, as spreadsheets write them). */
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

async function chooseFile(page: Page, open: () => Promise<void>, name: string, buffer: Buffer, mimeType = 'text/csv') {
  const chooser = page.waitForEvent('filechooser')
  await open()
  await (await chooser).setFiles({ name, mimeType, buffer })
}

const fixture = (path: string) => readFileSync(`${FIXTURES}/${path}`)
const wizard = (page: Page) => page.getByTestId('aiws-table-dialog')

async function closeReport(page: Page) {
  const report = page.getByTestId('aiws-import-report')
  if (await report.count()) await report.getByTestId('aiws-import-report-close').click()
}

test('TI-01 Excel HTML pasted on the canvas: header and types recognised, a table Block with the values', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `ti-01 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await paste(page, { html: fixture('clipboard/excel.html').toString('utf8'), text: fixture('clipboard/excel.txt').toString('utf8') })
  const dialog = wizard(page)
  await expect(dialog).toBeVisible()
  await expect(dialog.getByTestId('aiws-table-header')).toBeChecked()
  await expect(dialog.getByTestId('aiws-table-size')).toContainText('3 行 × 4 列')
  await expect.poll(() => Promise.all([0, 1, 2, 3].map((i) => dialog.getByTestId(`aiws-table-type-${i}`).inputValue()))).toEqual(['text', 'currency', 'date', 'boolean'])
  await expect(dialog.getByTestId('aiws-table-currency-1')).toHaveValue('USD')
  await expect(dialog.getByTestId('aiws-table-name-0')).toHaveValue('Item')
  await page.screenshot({ path: `${SHOTS}/ti01-wizard.png` })
  await dialog.getByTestId('aiws-table-start').click()
  await expect(dialog).toHaveCount(0)
  await expect.poll(async () => (await tablesIn(api, ws)).length).toBe(1)
  const [source] = await tablesIn(api, ws)
  expect((await fieldsOf(api, ws, source.entity_id)).map((f) => [f.name, f.type, f.currency ?? null])).toEqual([['Item', 'text', null], ['Price', 'currency', 'USD'], ['Date', 'date', null], ['Paid', 'boolean', null]])
  expect((await rows(api, ws, source.entity_id)).map((r) => r.values)).toEqual([
    { c1: 'Apple', c2: { amount: '1234.50', currency: 'USD' }, c3: '2026-07-01', c4: true },
    { c1: 'Pear', c2: { amount: '980.00', currency: 'USD' }, c3: '2026-07-02', c4: false },
    { c1: 'Plum', c2: { amount: '15.25', currency: 'USD' }, c3: '2026-07-15', c4: true },
  ])
  // the table Block on the canvas, selected, showing the money in the reader's language
  const block = children(await api.outline(ALICE, ws.workspace_id), 'sf-t').find((e) => e.view_type === 'table')
  expect(block).toBeTruthy()
  await expect(page.getByTestId(`aiws-canvas-block-${block.entity_id}`)).toBeVisible()
  await expect(page.getByTestId(`aiws-canvas-block-${block.entity_id}`)).toContainText('1,234.50')
  await page.screenshot({ path: `${SHOTS}/ti01-canvas.png` })
})

test('TI-02 plain TSV without a header: columns named 列 A…; the header switch takes the first row', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `ti-02 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await paste(page, { text: fixture('clipboard/tsv-no-header.txt').toString('utf8') })
  const dialog = wizard(page)
  await expect(dialog.getByTestId('aiws-table-header')).not.toBeChecked()
  await expect(dialog.getByTestId('aiws-table-name-0')).toHaveValue('列 A')
  await expect(dialog.getByTestId('aiws-table-name-2')).toHaveValue('列 C')
  await expect(dialog.getByTestId('aiws-table-start')).toHaveText('导入 3 行')
  await dialog.getByTestId('aiws-table-header').check()
  await expect(dialog.getByTestId('aiws-table-name-0')).toHaveValue('1')
  await expect(dialog.getByTestId('aiws-table-name-2')).toHaveValue('red')
  await expect(dialog.getByTestId('aiws-table-start')).toHaveText('导入 2 行')
  await dialog.getByTestId('aiws-table-header').uncheck()
  await dialog.getByTestId('aiws-table-start').click()
  await expect.poll(async () => (await tablesIn(api, ws)).length).toBe(1)
  const [source] = await tablesIn(api, ws)
  // the first text column is the title column: never a select
  expect((await fieldsOf(api, ws, source.entity_id)).map((f) => [f.name, f.type])).toEqual([['列 A', 'number'], ['列 B', 'decimal'], ['列 C', 'text']])
  expect((await rows(api, ws, source.entity_id)).map((r) => r.values.c2)).toEqual(['3.50', '4.00', '4.25'])
})

test('TI-03 a number column changed to date: the values that do not fit are counted, left empty and reported', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `ti-03 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await paste(page, { text: 'id\tday\n1\t2026-01-05\n2\t2026-01-06\n3\t2026-01-07\n' })
  const dialog = wizard(page)
  await expect(dialog.getByTestId('aiws-table-type-0')).toHaveValue('number')
  await dialog.getByTestId('aiws-table-type-0').selectOption('date')
  await expect(dialog.getByTestId('aiws-table-invalid-0')).toContainText('3 个值无法转换')
  await expect(dialog.getByTestId('aiws-table-policy-unset-0')).toBeChecked()
  await page.screenshot({ path: `${SHOTS}/ti03-invalid.png` })
  await dialog.getByTestId('aiws-table-start').click()
  const report = page.getByTestId('aiws-import-report')
  await expect(report.locator('[data-kind="table_invalid"]')).toContainText('× 3')
  await expect(report.locator('[data-kind="table_invalid"]')).toContainText('id: 1')
  await report.getByTestId('aiws-import-report-close').click()
  const [source] = await tablesIn(api, ws)
  expect((await rows(api, ws, source.entity_id)).map((r) => r.values)).toEqual([{ c2: '2026-01-05' }, { c2: '2026-01-06' }, { c2: '2026-01-07' }])
})

test('TI-04 a flow page: the table Block goes after the selected Block; one undo removes it all', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `ti-04 ${Date.now()}`, 'flow')
  const text = (id: string, words: string) => ({ op: 'entity.create', entity_id: id, type_id: 'buckyos.richtext', parent_id: 'sf-t-content', order_key: id === 't1' ? 'a' : 'b', payload: { content: { type: 'doc', content: [{ type: 'paragraph', attrs: { block_id: `b${id}` }, content: [{ type: 'text', text: words }] }] } } })
  await api.commit(ALICE, ws, [text('t1', 'first'), text('t2', 'second'),
    { op: 'entity.create', entity_id: 'c1', type_id: 'buckyos.cell', parent_id: 'sf-t', order_key: 'a', payload: { view: { type: 'richtext' }, source_ref: { entity_id: 't1' } } },
    { op: 'entity.create', entity_id: 'c2', type_id: 'buckyos.cell', parent_id: 'sf-t', order_key: 'b', payload: { view: { type: 'richtext' }, source_ref: { entity_id: 't2' } } }])
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-flow-handle-c1').click()
  await paste(page, { text: 'name\tqty\na\t1\nb\t2\n' })
  await wizard(page).getByTestId('aiws-table-start').click()
  await expect(wizard(page)).toHaveCount(0)
  await expect.poll(async () => children(await api.outline(ALICE, ws.workspace_id), 'sf-t').map((e) => e.view_type)).toEqual(['richtext', 'table', 'richtext'])
  const order = children(await api.outline(ALICE, ws.workspace_id), 'sf-t').map((e) => e.entity_id)
  expect([order[0], order[2]]).toEqual(['c1', 'c2'])
  await page.getByTestId('aiws-undo').click()
  await expect.poll(async () => children(await api.outline(ALICE, ws.workspace_id), 'sf-t').map((e) => e.entity_id)).toEqual(['c1', 'c2'])
  expect(await tablesIn(api, ws)).toHaveLength(0)
})

test('TI-05 a CSV dropped on the canvas asks: a table, or a file', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `ti-05 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await drop(page, 'select.csv', fixture('csv/select.csv'), { x: 600, y: 400 })
  const ask = page.getByTestId('aiws-table-ask')
  await expect(ask).toBeVisible()
  await page.screenshot({ path: `${SHOTS}/ti05-ask.png` })
  await ask.getByTestId('aiws-table-ask-table').click()
  await expect(wizard(page).getByTestId('aiws-table-title')).toHaveValue('select')
  await expect(wizard(page).getByTestId('aiws-table-type-1')).toHaveValue('select')
  await wizard(page).getByTestId('aiws-table-start').click()
  await expect.poll(async () => (await tablesIn(api, ws)).map((t) => t.name)).toEqual(['select'])
  const [source] = await tablesIn(api, ws)
  expect((await fieldsOf(api, ws, source.entity_id))[1].options.map((o: Json) => o.label)).toEqual(['open', 'done', 'blocked'])
  // the same file as a file
  await drop(page, 'select.csv', fixture('csv/select.csv'), { x: 300, y: 300 })
  await page.getByTestId('aiws-table-ask').getByTestId('aiws-table-ask-file').click()
  await expect.poll(async () => children(await api.outline(ALICE, ws.workspace_id), 'sf-t').map((e) => e.view_type).sort()).toEqual(['asset', 'table'])
})

test('TI-06 the insert catalog\'s table entry → from a file: a GB 18030 CSV with semicolons reads right', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `ti-06 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-tool-catalog').click()
  await page.getByTestId('aiws-catalog-entry-block-table').click()
  await chooseFile(page, () => page.getByTestId('aiws-catalog-import-table').click(), '城市.csv', fixture('csv/gb18030-semicolon.csv'))
  const dialog = wizard(page)
  await expect(dialog.getByTestId('aiws-table-name-0')).toHaveValue('城市')
  await dialog.getByTestId('aiws-table-advanced').click()
  await expect(dialog.getByTestId('aiws-table-encoding')).toHaveValue('gb18030')
  await expect(dialog.getByTestId('aiws-table-delimiter')).toHaveValue(';')
  await page.screenshot({ path: `${SHOTS}/ti06-wizard.png` })
  await dialog.getByTestId('aiws-table-start').click()
  await expect.poll(async () => (await tablesIn(api, ws)).length).toBe(1)
  const [source] = await tablesIn(api, ws)
  expect(source.name).toBe('城市')
  expect((await rows(api, ws, source.entity_id)).map((r) => r.values)).toEqual([
    { c1: '北京', c2: 21893095, c3: '16410.54' }, { c1: '上海', c2: 24870895, c3: '6340.50' }, { c1: '广州', c2: 18676605, c3: '7434.40' },
  ])
})

test('TI-07 the main menu\'s "import table data", keeping the original file: the table and the file in the canvas\'s data', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `ti-07 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await chooseFile(page, () => mainMenu(page, 'aiws-menu-insert', 'aiws-menu-import-table'), 'thousands.csv', fixture('csv/thousands.csv'))
  const dialog = wizard(page)
  await dialog.getByTestId('aiws-table-advanced').click()
  await expect(dialog.getByTestId('aiws-table-keep')).not.toBeChecked()
  await dialog.getByTestId('aiws-table-keep').check()
  await dialog.getByTestId('aiws-table-start').click()
  await expect.poll(async () => children(await api.outline(ALICE, ws.workspace_id), 'sf-t-content').map((e) => e.type_id).sort()).toEqual(['buckyos.asset-ref', 'buckyos.table-source'])
  const [source] = await tablesIn(api, ws)
  expect((await rows(api, ws, source.entity_id)).map((r) => r.values.c2)).toEqual([1234567, 89000, 1000])
})

test('TI-08 money: codes, names and symbols; the default currency for bare amounts; order and filter within a currency', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `ti-08 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await chooseFile(page, () => mainMenu(page, 'aiws-menu-insert', 'aiws-menu-import-table'), 'prices.csv', fixture('csv/currency-mixed.csv'))
  const dialog = wizard(page)
  await expect(dialog.getByTestId('aiws-table-type-1')).toHaveValue('currency')
  await expect(dialog.getByTestId('aiws-table-yen-1')).toHaveValue('CNY')
  await dialog.getByTestId('aiws-table-currency-1').selectOption('USD')
  await page.screenshot({ path: `${SHOTS}/ti08-currency.png` })
  await dialog.getByTestId('aiws-table-start').click()
  await closeReport(page)
  await expect.poll(async () => (await tablesIn(api, ws)).length).toBe(1)
  const [source] = await tablesIn(api, ws)
  expect((await fieldsOf(api, ws, source.entity_id))[1]).toMatchObject({ type: 'currency', currency: 'USD' })
  expect((await rows(api, ws, source.entity_id)).map((r) => r.values.c2)).toEqual([
    { amount: '200.00', currency: 'USD' }, { amount: '500.00', currency: 'CNY' }, { amount: '300.00', currency: 'CNY' }, { amount: '10.00', currency: 'EUR' },
  ])
  const sorted = await rows(api, ws, source.entity_id, { sorts: [{ field_id: 'c2', direction: 'asc' }] })
  expect(sorted.map((r) => r.values.c1)).toEqual(['c', 'b', 'd', 'a'])
  const over = await rows(api, ws, source.entity_id, { filter: { op: 'cmp', field_id: 'c2', operator: 'gt', value: 100 } })
  expect(over.map((r) => r.values.c1)).toEqual(['a'])
  const block = children(await api.outline(ALICE, ws.workspace_id), 'sf-t').find((e) => e.view_type === 'table')
  await expect(page.getByTestId(`aiws-canvas-block-${block.entity_id}`)).toContainText('€10.00')
})

test('TI-09 an xlsx workbook: the first sheet by default, one table per sheet on request; dates, money, ratios, formulas, a merged header', async ({ page, api }) => {
  test.setTimeout(120_000)
  const ws = await canvasWorkspace(api, `ti-09 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await chooseFile(page, () => mainMenu(page, 'aiws-menu-insert', 'aiws-menu-import-table'), 'workbook.xlsx', fixture('xlsx/workbook.xlsx'), 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet')
  const dialog = wizard(page)
  await expect(dialog.getByTestId('aiws-table-title')).toHaveValue('Sales')
  await expect.poll(() => Promise.all([0, 1, 2, 3, 4].map((i) => dialog.getByTestId(`aiws-table-type-${i}`).inputValue()))).toEqual(['date', 'currency', 'number', 'decimal', 'decimal'])
  await expect(dialog.getByTestId('aiws-table-currency-1')).toHaveValue('CNY')
  await expect(dialog.getByTestId('aiws-table-name-2')).toHaveValue('列 C')
  await dialog.getByTestId('aiws-table-advanced').click()
  await expect(dialog.getByTestId('aiws-table-keep')).toBeChecked()
  await dialog.getByTestId('aiws-table-all-sheets').check()
  await expect(dialog.getByRole('tab')).toHaveCount(2)
  await page.screenshot({ path: `${SHOTS}/ti09-wizard.png` })
  await dialog.getByTestId('aiws-table-start').click()
  const report = page.getByTestId('aiws-import-report')
  await expect(report.locator('[data-kind="table_formula"]')).toBeVisible()
  await expect(report.locator('[data-kind="table_merged_header"]')).toBeVisible()
  await page.screenshot({ path: `${SHOTS}/ti09-report.png` })
  await report.getByTestId('aiws-import-report-close').click()
  const tables = await tablesIn(api, ws)
  expect(tables.map((t) => t.name)).toEqual(['Sales', 'Events'])
  const sales = await rows(api, ws, tables[0].entity_id)
  expect(sales[0].values).toMatchObject({ c1: '2026-01-01', c2: { amount: '1200.50', currency: 'CNY' }, c4: '0.125', c5: '150.0625' })
  expect(children(await api.outline(ALICE, ws.workspace_id), 'sf-t-content').filter((e) => e.type_id === 'buckyos.asset-ref')).toHaveLength(1)
  // two Blocks side by side
  const blocks = children(await api.outline(ALICE, ws.workspace_id), 'sf-t').filter((e) => e.view_type === 'table')
  expect(blocks).toHaveLength(2)
  expect(blocks[1].placement.x).toBe(blocks[0].placement.x + blocks[0].placement.w + 40)
})

test('TI-10 a SQLite file: its tables listed and chosen, BLOB columns left out, NUMERIC(10,2) a decimal of scale 2', async ({ page, api }) => {
  test.setTimeout(120_000)
  const ws = await canvasWorkspace(api, `ti-10 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await chooseFile(page, () => mainMenu(page, 'aiws-menu-insert', 'aiws-menu-import-table'), 'shop.sqlite', fixture('sqlite/shop.sqlite'), 'application/vnd.sqlite3')
  const dialog = wizard(page)
  await expect(dialog.getByRole('tab')).toHaveCount(3)
  await dialog.getByTestId('aiws-table-advanced').click()
  await expect(dialog.getByTestId('aiws-table-tables').locator('input[type=checkbox]:checked')).toHaveCount(3)
  await expect(dialog.getByTestId('aiws-table-table-active_products')).toHaveCount(0)
  await page.screenshot({ path: `${SHOTS}/ti10-wizard.png` })
  await dialog.getByTestId('aiws-table-start').click()
  const report = page.getByTestId('aiws-import-report')
  await expect(report.locator('[data-kind="table_blob_column"]')).toContainText('products.photo')
  await report.getByTestId('aiws-import-report-close').click()
  const tables = await tablesIn(api, ws)
  expect(tables.map((t) => t.name)).toEqual(['products', 'orders', 'odd "name"'])
  const fields = await fieldsOf(api, ws, tables[0].entity_id)
  expect(fields.map((f) => [f.name, f.type, f.scale ?? null])).toEqual([['id', 'number', null], ['name', 'text', null], ['price', 'decimal', 2], ['weight', 'decimal', 2], ['active', 'boolean', null], ['added', 'date', null]])
  expect((await rows(api, ws, tables[1].entity_id))[0].values.c6).toBe('9007199254740993')
})

test('TI-11 more than 20 000 rows: two commits with progress; cancelled during the first, the import says how far it got', async ({ page, api }) => {
  test.setTimeout(240_000)
  const ws = await canvasWorkspace(api, `ti-11 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  const csv = Buffer.from(['n,label', ...Array.from({ length: 20_001 }, (_, i) => `${i},row ${i}`)].join('\n'))
  await chooseFile(page, () => mainMenu(page, 'aiws-menu-insert', 'aiws-menu-import-table'), 'big.csv', csv)
  const dialog = wizard(page)
  await expect(dialog.getByTestId('aiws-table-start')).toHaveText('导入 20001 行')
  await dialog.getByTestId('aiws-table-start').click()
  await expect(dialog.getByTestId('aiws-table-progress')).toContainText('正在导入', { timeout: 60_000 })
  await page.screenshot({ path: `${SHOTS}/ti11-progress.png` })
  await dialog.getByTestId('aiws-table-cancel').click()
  const report = page.getByTestId('aiws-import-report')
  await expect(report.getByTestId('aiws-import-error')).toContainText('已取消，导入了 20001 行中的 20000 行', { timeout: 120_000 })
  await report.getByTestId('aiws-import-report-close').click()
  const [source] = await tablesIn(api, ws)
  expect((await api.rpc(ALICE, 'doc.query', { workspace_id: ws.workspace_id, source_id: source.entity_id, limit: 1 })).total).toBe(20_000)
  // rows keep the file's order (record ids sort in row order)
  const first = await api.rpc(ALICE, 'doc.query', { workspace_id: ws.workspace_id, source_id: source.entity_id, limit: 3 })
  expect(first.rows.map((r: Json) => r.values.c1)).toEqual([0, 1, 2])
})

test('TI-12 "copy as table" in a table view, pasted on another canvas: the wizard prefilled with the types and options', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `ti-12 ${Date.now()}`)
  await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'tasks', type_id: 'buckyos.table-source', parent_id: 'sf-t-content', order_key: 'a', name: 'tasks', payload: { title_field_id: 'f1', fields: [
      { field_id: 'f1', name: 'Task', type: 'text' },
      { field_id: 'f2', name: 'Status', type: 'select', options: [{ option_id: 'x1', label: 'Open' }, { option_id: 'x2', label: 'Done' }] },
      { field_id: 'f3', name: 'Cost', type: 'currency', currency: 'EUR' },
      { field_id: 'f4', name: 'Due', type: 'date' }] } },
    { op: 'table.insert_records', source_id: 'tasks', records: [
      { record_id: 'r1', values: { f1: 'Plan', f2: 'x1', f3: '12.5', f4: '2026-11-01' } },
      { record_id: 'r2', values: { f1: 'Ship', f2: 'x2', f3: { amount: '99', currency: 'USD' } } }] },
    { op: 'entity.create', entity_id: 'tv', type_id: 'buckyos.cell', parent_id: 'sf-t', order_key: 'a', placement: { x: 100, y: 100, w: 640, h: 240 }, payload: { view: { type: 'table' }, source_ref: { entity_id: 'tasks' } } },
  ])
  await page.context().grantPermissions(['clipboard-read', 'clipboard-write'])
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-canvas-block-tv').click()
  await page.getByTestId('aiws-near-table-copy').click()
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '已复制 2 行为表格' })).toBeVisible()
  const clip = await page.evaluate(async () => {
    const [item] = await navigator.clipboard.read()
    return { html: await (await item.getType('text/html')).text(), text: await (await item.getType('text/plain')).text() }
  })
  expect(clip.text.split(/\r?\n/)[0]).toBe('Task\tStatus\tCost\tDue')
  // another canvas of the same workspace
  await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'sf-u-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'j', payload: { kind: 'folder', title: '二', system: 'surface_content', surface_id: 'sf-u' } },
    { op: 'entity.create', entity_id: 'sf-u', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'j', payload: { kind: 'surface', layout: { mode: 'free' }, title: '二', content_folder_id: 'sf-u-content' } },
  ])
  await page.getByTestId('aiws-surface-switch').click()
  await page.getByTestId('aiws-surface-item-sf-u').getByRole('menuitem').click()
  await expect(page.getByTestId('aiws-canvas-block-tv')).toHaveCount(0)
  await paste(page, clip)
  const dialog = wizard(page)
  await expect(dialog.getByTestId('aiws-table-header')).toBeChecked()
  await expect.poll(() => Promise.all([0, 1, 2, 3].map((i) => dialog.getByTestId(`aiws-table-type-${i}`).inputValue()))).toEqual(['text', 'select', 'currency', 'date'])
  await expect(dialog.getByTestId('aiws-table-currency-2')).toHaveValue('EUR')
  await page.screenshot({ path: `${SHOTS}/ti12-wizard.png` })
  await dialog.getByTestId('aiws-table-start').click()
  await expect(dialog).toHaveCount(0)
  const all = (await api.outline(ALICE, ws.workspace_id)).filter((e: Json) => e.type_id === 'buckyos.table-source' && !e.deleted && e.entity_id !== 'tasks')
  expect(all).toHaveLength(1)
  const fields = await fieldsOf(api, ws, all[0].entity_id)
  expect(fields.map((f) => [f.name, f.type])).toEqual([['Task', 'text'], ['Status', 'select'], ['Cost', 'currency'], ['Due', 'date']])
  expect(fields[1].options.map((o: Json) => o.label)).toEqual(['Open', 'Done'])
  const values = (await rows(api, ws, all[0].entity_id)).map((r) => r.values)
  expect(values).toEqual([{ c1: 'Plan', c2: 'o1', c3: { amount: '12.50', currency: 'EUR' }, c4: '2026-11-01' }, { c1: 'Ship', c2: 'o2', c3: { amount: '99.00', currency: 'USD' } }])
})
