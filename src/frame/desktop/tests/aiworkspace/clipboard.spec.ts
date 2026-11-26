/* The canvas and the system clipboard (doc/desktop/BuckyOS 剪贴板管理与拖拽支持规范.md §3, §7, §8): a Block copy
 * leaves readable text on the system clipboard and pastes back only through its handle; text copied elsewhere after it
 * is pasted as that text (a text Block), never as the earlier Block; Zone references pasted or dropped take the path of
 * a dropped file (a PDF asks "as a file / its pages"); a Zone address asks "file or text"; a file dropped on a
 * directory editor on the canvas becomes one directory item and nothing on the canvas. Screenshots go to
 * test-results/aiworkspace-clipboard/. */

import type { Page } from '@playwright/test'
import { expect, openCanvas, test, type Api } from './fixtures'

const ALICE = 'tok-alice'
const SHOTS = 'test-results/aiworkspace-clipboard'
const CONTENT_REFS = 'application/vnd.buckyos.content-refs+json'
const PDF_PATH = 'cyfs:///home/Documents/Friend Contract Draft.pdf'
const PDF_NAME = 'Friend Contract Draft.pdf'

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- API results, read field by field
type Json = Record<string, any>

/** A free canvas "Free" with a shape "Quarterly goals" and, when asked, a directory Block. */
async function canvasWith(api: Api, title: string, directory = false) {
  const ws = await api.freeCanvas(ALICE, title)
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'goal', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'a', placement: { x: 120, y: 120, w: 200, h: 110 },
      payload: { view: { type: 'shape' }, title: 'Quarterly goals', config: { shape: 'rect', fill: '#e8f0fe', stroke: '#4f8df7' } } },
    ...(directory ? [
      { op: 'entity.create', entity_id: 'box', type_id: 'buckyos.dir', parent_id: 'free-content', order_key: 'a', name: 'Box', payload: { members: {} } },
      { op: 'entity.create', entity_id: 'box-cell', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'b', placement: { x: 420, y: 120, w: 480, h: 320 }, payload: { view: { type: 'directory' }, source_ref: { entity_id: 'box' } } },
    ] : []),
  ]) as { status: string }
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  return ws
}

const cellsOf = async (api: Api, workspaceId: string) => (await api.outline(ALICE, workspaceId)).filter((e: Json) => e.type_id === 'buckyos.cell' && e.parent_id === 'free' && !e.deleted)

/** A paste the canvas receives with these representations (a synthetic event: parsing and dispatch, §8). */
async function paste(page: Page, data: Record<string, string>) {
  await page.evaluate((reps) => {
    const transfer = new DataTransfer()
    for (const [format, value] of Object.entries(reps)) transfer.setData(format, value)
    document.body.dispatchEvent(new ClipboardEvent('paste', { clipboardData: transfer, bubbles: true, cancelable: true }))
  }, data)
}

const zoneRefs = (path: string, name: string) => JSON.stringify({ v: 1, items: [{ origin: 'zone', kind: 'file', source: { kind: 'cyfs-path', path }, hints: { name, mime: 'application/pdf' } }] })

test('CB-01 a Block copy leaves readable text; text copied elsewhere afterwards is pasted as text, not as the Block', async ({ page, context, api }) => {
  const ws = await canvasWith(api, `cb-01 ${Date.now()}`)
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-canvas-block-goal').click()
  await page.keyboard.press('Control+c')
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '已复制 1 个对象' })).toBeVisible()
  // the system clipboard: the Block's text for people, no marker, id or JSON
  const copied = await page.evaluate(async () => {
    const [item] = await navigator.clipboard.read()
    return { types: item.types, text: await (await item.getType('text/plain')).text(), html: item.types.includes('text/html') ? await (await item.getType('text/html')).text() : '' }
  })
  expect(copied.text).toBe('Quarterly goals')
  expect(copied.html).toContain('Quarterly goals')
  expect(copied.html).toContain('data-buckyos-transfer')
  // pasted back with its handle: a new Block
  await page.keyboard.press('Control+v')
  await expect.poll(async () => (await cellsOf(api, ws.workspace_id)).filter((e: Json) => e.title === 'Quarterly goals').length).toBe(2)
  // another App copies text: the next paste is that text, the Block does not come back (§3.5)
  await page.evaluate(() => navigator.clipboard.writeText('周五发版 https://example.com/release'))
  await page.getByTestId('aiws-canvas').click({ position: { x: 900, y: 700 } })
  await page.keyboard.press('Control+v')
  await expect.poll(async () => (await cellsOf(api, ws.workspace_id)).filter((e: Json) => e.view_type === 'richtext').length).toBe(1)
  expect((await cellsOf(api, ws.workspace_id)).filter((e: Json) => e.title === 'Quarterly goals')).toHaveLength(2)
  const text = (await cellsOf(api, ws.workspace_id)).find((e: Json) => e.view_type === 'richtext')!
  const doc = JSON.stringify((await api.read(ALICE, ws.workspace_id, text.source_id)).content)
  expect(doc).toContain('周五发版')
  expect(doc).toContain('"href":"https://example.com/release"')
  await page.screenshot({ path: `${SHOTS}/cb01-text-pasted.png` })
  // an outside paste of nothing the canvas takes says what it takes
  await paste(page, { 'text/plain': '   ' })
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '画布接受对象' })).toBeVisible()
})

test('CB-02 Zone references pasted and dropped take the drop path of a file: a PDF asks "as a file / its pages"', async ({ page, api }) => {
  const ws = await canvasWith(api, `cb-02 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  const before = (await cellsOf(api, ws.workspace_id)).length
  await paste(page, { [CONTENT_REFS]: zoneRefs(PDF_PATH, PDF_NAME), 'text/plain': PDF_NAME })
  const ask = page.getByTestId('aiws-pdf-ask')
  await expect(ask).toBeVisible({ timeout: 20_000 })
  await page.screenshot({ path: `${SHOTS}/cb02-paste-ask.png` })
  await ask.getByTestId('aiws-pdf-ask-file').click()
  await expect.poll(async () => (await cellsOf(api, ws.workspace_id)).filter((e: Json) => e.view_type === 'asset').length, { timeout: 30_000 }).toBe(1)
  expect(JSON.stringify(await api.outline(ALICE, ws.workspace_id))).toContain(PDF_NAME)
  // dropped at a point: the same ask; cancelling leaves the canvas as it was
  const main = page.locator('.aiws-canvas-main')
  const dt = await page.evaluateHandle(([format, value]) => { const t = new DataTransfer(); t.setData(format, value); t.setData('text/plain', 'x'); return t }, [CONTENT_REFS, zoneRefs(PDF_PATH, PDF_NAME)])
  await main.dispatchEvent('dragover', { dataTransfer: dt, clientX: 700, clientY: 500 })
  await main.dispatchEvent('drop', { dataTransfer: dt, clientX: 700, clientY: 500 })
  await expect(ask).toBeVisible({ timeout: 20_000 })
  await ask.press('Escape')
  await expect(ask).toHaveCount(0)
  await page.waitForTimeout(400)
  expect((await cellsOf(api, ws.workspace_id)).length).toBe(before + 1)
  // references from a newer major version are not read: the text below them is pasted instead
  await paste(page, { [CONTENT_REFS]: JSON.stringify({ v: 2, items: [] }), 'text/plain': 'just words' })
  await expect.poll(async () => (await cellsOf(api, ws.workspace_id)).filter((e: Json) => e.view_type === 'richtext').length).toBe(1)
})

test('CB-03 a Zone address pasted on the canvas asks: the file it names, or the text', async ({ page, api }) => {
  const ws = await canvasWith(api, `cb-03 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await paste(page, { 'text/plain': PDF_PATH })
  const ask = page.getByTestId('aiws-paste-ask')
  await expect(ask).toBeVisible()
  await page.screenshot({ path: `${SHOTS}/cb03-address-ask.png` })
  // nothing is read before the choice
  await ask.getByTestId('aiws-paste-ask-text').click()
  await expect.poll(async () => (await cellsOf(api, ws.workspace_id)).filter((e: Json) => e.view_type === 'richtext').length).toBe(1)
  await paste(page, { 'text/plain': PDF_PATH })
  await ask.getByTestId('aiws-paste-ask-file').click()
  await expect(page.getByTestId('aiws-pdf-ask')).toBeVisible({ timeout: 20_000 })
  await page.getByTestId('aiws-pdf-ask').press('Escape')
  // cancelled: nothing at all
  await paste(page, { 'text/plain': PDF_PATH })
  await ask.getByTestId('aiws-paste-ask-cancel').click()
  await page.waitForTimeout(400)
  expect((await cellsOf(api, ws.workspace_id)).filter((e: Json) => e.view_type === 'richtext' || e.view_type === 'asset')).toHaveLength(1)
})

test('CB-04 a file dropped on a directory editor on the canvas adds one directory item and nothing on the canvas', async ({ page, api }) => {
  const ws = await canvasWith(api, `cb-04 ${Date.now()}`, true)
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-canvas-block-box-cell').click({ position: { x: 20, y: 12 } })
  await page.keyboard.press('Enter')
  const editor = page.locator('[data-testid="aiws-directory"][data-variant="edit"]')
  await expect(editor).toBeVisible()
  const before = (await cellsOf(api, ws.workspace_id)).length
  const dt = await page.evaluateHandle(() => { const t = new DataTransfer(); t.items.add(new File(['dropped bytes'], 'dropped.txt', { type: 'text/plain' })); return t })
  await editor.dispatchEvent('dragover', { dataTransfer: dt })
  await editor.dispatchEvent('drop', { dataTransfer: dt })
  await expect.poll(async () => Object.keys((await api.read(ALICE, ws.workspace_id, 'box')).content.payload.members ?? {}), { timeout: 20_000 }).toEqual(['dropped.txt'])
  await page.waitForTimeout(600)
  expect((await cellsOf(api, ws.workspace_id)).length).toBe(before)
})

test('CB-05 a resource Block copied also offers its file: its name as text, an obj:// link and the Zone reference list', async ({ page, context, api }) => {
  const ws = await api.freeCanvas(ALICE, `cb-05 ${Date.now()}`)
  const note = await api.uploadAsset(ALICE, ws.workspace_id, Buffer.from('meeting notes'))
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'doc', type_id: 'buckyos.asset-ref', parent_id: 'free-content', order_key: 'a', name: 'notes.txt', payload: { object_id: note.object_id, file_name: 'notes.txt' } },
    { op: 'entity.create', entity_id: 'doc-cell', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'a', placement: { x: 120, y: 120, w: 240, h: 160 }, payload: { view: { type: 'asset' }, source_ref: { entity_id: 'doc' } } },
  ]) as { status: string }
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-canvas-block-doc-cell').click()
  // the file's facts are read once it is selected (a copy event cannot wait for them)
  await page.waitForTimeout(800)
  // the copy event itself: every format it puts on the clipboard (recorded as they are written)
  await page.evaluate(() => {
    const written: Record<string, string> = {}
    const setData = DataTransfer.prototype.setData
    DataTransfer.prototype.setData = function (format: string, value: string) { written[format.toLowerCase()] = value; return setData.call(this, format, value) }
    ;(window as { __copied?: Record<string, string> }).__copied = written
  })
  await page.keyboard.press('Control+c')
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '已复制 1 个对象' })).toBeVisible()
  const data = await page.evaluate(() => (window as { __copied?: Record<string, string> }).__copied ?? {})
  expect(data['text/plain']).toBe('notes.txt')
  expect(data['text/uri-list']).toBe(`obj://${note.object_id}`)
  const refs = JSON.parse(data['application/vnd.buckyos.content-refs+json'])
  expect(refs).toMatchObject({ v: 1, items: [{ origin: 'zone', kind: 'file', source: { kind: 'object-id', objectId: note.object_id }, hints: { name: 'notes.txt' } }] })
  expect(JSON.parse(data['application/vnd.buckyos.transfer+json'])).toMatchObject({ v: 1 })
  expect(data['text/html']).toContain(`href="obj://${note.object_id}"`)
  expect(Object.values(data).join('\n')).not.toContain('aiws-clipboard')
})
