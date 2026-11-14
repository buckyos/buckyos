/* PDF import (doc/workspace/BuckyOS AI Workspace PDF 导入与放映方案讨论.md §10 P0): a three-page PDF of mixed
 * proportions (a coloured slide with text, a picture and a box; a slide whose Chinese text uses a predefined CMap and
 * a font the file does not embed; a portrait page) imported from the main menu becomes one new canvas with three page
 * groups and a presentation path; its text is editable, its picture an asset, and the show crops each step to its
 * page. A dropped PDF first asks whether it is a file or pages. Screenshots go to test-results/aiworkspace-pdf/. */

import type { Page } from '@playwright/test'
import { chooseFromDevice, chooseFromZone, mainMenu, openCanvas, openWorkspacePage, test, expect, type Api } from './fixtures'
import { deck, spacedText, overflowText } from './pdfFiles'

const ALICE = 'tok-alice'
const SHOTS = 'test-results/aiworkspace-pdf'

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

async function canvasWorkspace(api: Api, title: string) {
  const ws = await api.rpc(ALICE, 'ws.create', { title })
  await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'sf-p-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'i', payload: { kind: 'folder', title: '画布', system: 'surface_content', surface_id: 'sf-p' } },
    { op: 'entity.create', entity_id: 'sf-p', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'i', payload: { kind: 'surface', layout: { mode: 'free' }, title: '画布', content_folder_id: 'sf-p-content' } },
  ])
  return ws as { workspace_id: string; epoch: string }
}

const children = (outline: Json[], parent: string) => outline.filter((e) => e.parent_id === parent && !e.deleted).sort((a, b) => (a.order_key < b.order_key ? -1 : 1))

async function drop(page: Page, name: string, bytes: Buffer, at: { x: number; y: number }) {
  const dt = await page.evaluateHandle(({ b64, name }) => {
    const bin = atob(b64)
    const data = new Uint8Array(bin.length)
    for (let i = 0; i < bin.length; i++) data[i] = bin.charCodeAt(i)
    const transfer = new DataTransfer()
    transfer.items.add(new File([data], name, { type: 'application/pdf' }))
    return transfer
  }, { b64: bytes.toString('base64'), name })
  const target = page.locator('.aiws-canvas-main')
  await target.dispatchEvent('dragover', { dataTransfer: dt, clientX: at.x, clientY: at.y })
  await target.dispatchEvent('drop', { dataTransfer: dt, clientX: at.x, clientY: at.y })
}

test('PI-01 a three-page PDF from the main menu: one canvas, page groups with editable text, a picture, a path that plays page by page', async ({ page, api }) => {
  test.setTimeout(120_000)
  const ws = await canvasWorkspace(api, `pi-01 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await chooseFromZone(page, () => mainMenu(page, 'aiws-menu-insert', 'aiws-menu-import-file', 'aiws-menu-import-pdf'), { name: 'deck.pdf', mimeType: 'application/pdf', buffer: deck() })
  const dialog = page.getByTestId('aiws-pdf-dialog')
  await expect(dialog.getByTestId('aiws-pdf-pages')).toContainText('共 3 页')
  await expect(dialog.getByTestId('aiws-pdf-preview')).toBeVisible()
  await expect(dialog.getByTestId('aiws-pdf-estimate')).toContainText('2 段文字、1 张图片、1 个图形')
  await expect(dialog.getByTestId('aiws-pdf-keep')).toBeChecked()
  await page.screenshot({ path: `${SHOTS}/pi01-dialog.png` })
  await dialog.getByTestId('aiws-pdf-start').click()
  await expect(dialog).toHaveCount(0, { timeout: 60_000 })

  // one new canvas named after the file, three page groups, each a Frame first
  let outline: Json[] = []
  await expect.poll(async () => { outline = await api.outline(ALICE, ws.workspace_id); return outline.filter((e) => e.kind === 'surface' && !e.deleted).length }).toBe(2)
  const surface = outline.find((e) => e.kind === 'surface' && e.entity_id !== 'sf-p')
  expect(surface.title).toBe('deck')
  const groups = children(outline, surface.entity_id)
  expect(groups.map((g) => g.kind)).toEqual(['group', 'group', 'group'])
  const frames = groups.map((g) => children(outline, g.entity_id)[0])
  expect(frames.map((f) => f.view_type)).toEqual(['frame', 'frame', 'frame'])
  // pages keep their proportions: two 16:9 pages 1920 wide, a portrait page 1920 high
  expect(groups.map((g) => [g.placement.w, g.placement.h])).toEqual([[1920, 1080], [1920, 1080], [1357, 1920]])
  const frame1 = await api.read(ALICE, ws.workspace_id, frames[0].entity_id)
  expect(frame1.content.payload.presentation).toEqual({ background: '#1f3b5e' })
  expect(frame1.content.payload.title).toBe('第 1 页')

  // page 1: text objects with their colour and weight, the picture as an asset, the box as a rectangle
  const page1 = children(outline, groups[0].entity_id)
  const configs = await Promise.all(page1.slice(1).map(async (e) => ({ e, payload: (await api.read(ALICE, ws.workspace_id, e.entity_id)).content.payload })))
  const title = configs.find((c) => c.payload.config?.text === 'Quarterly Review')
  expect(title?.payload.config).toMatchObject({ shape: 'text', font_color: '#ffffff', bold: true, wrap: false })
  const body = configs.find((c) => String(c.payload.config?.text ?? '').startsWith('Revenue grew'))
  expect(body?.payload.config.text).toBe('Revenue grew 23 percent\nNew customers: 1,204')
  expect(configs.filter((c) => c.e.view_type === 'asset')).toHaveLength(1)
  expect(configs.find((c) => c.payload.config?.shape === 'rect')?.payload.config.fill).toBe('#f59e0a')
  // page 2: Chinese text read through the CMap served with the app
  const page2 = children(outline, groups[1].entity_id)
  const texts2 = await Promise.all(page2.map(async (e) => (await api.read(ALICE, ws.workspace_id, e.entity_id)).content.payload.config?.text))
  expect(texts2).toContain('中文用例')
  // page 3: the box drawn with a blend mode stays in a raster under the text, cropped to it
  const page3 = children(outline, groups[2].entity_id)
  const reads3 = await Promise.all(page3.map(async (e) => ({ e, payload: (await api.read(ALICE, ws.workspace_id, e.entity_id)).content.payload })))
  const raster = reads3.find((r) => r.payload.config?.pdf_source?.kind === 'raster')
  expect(raster?.e.view_type).toBe('asset')
  expect(page3.indexOf(raster!.e)).toBe(1)
  expect(raster!.e.placement.w).toBeLessThan(groups[2].placement.w / 2)
  expect(reads3.some((r) => r.payload.config?.text === 'A portrait page in the same deck.')).toBe(true)
  // the original file is kept in the canvas's data
  const original = children(outline, surface.content_folder_id).find((e) => e.type_id === 'buckyos.asset-ref' && e.media_type === 'application/pdf')
  expect(original).toBeTruthy()

  // a presentation path: one cut step per page, black, the 16:9 stage
  const path = outline.find((e) => e.type_id === 'buckyos.show-path' && !e.deleted)
  const pathPayload = (await api.read(ALICE, ws.workspace_id, path.entity_id)).content.payload
  expect(pathPayload).toMatchObject({ title: 'deck', purpose: 'presentation', stage: { w: 1920, h: 1080 }, background: '#000000' })
  expect(pathPayload.steps.map((s: Json) => [s.target.entity_id, s.transition])).toEqual(frames.map((f) => [f.entity_id, 'cut']))

  // the canvas shows the new pages; text is edited in place
  // the blend-mode box is reported (kept as a picture); the report also offers the show
  const report = page.getByTestId('aiws-import-report')
  await expect(report.getByTestId('aiws-import-summary')).toContainText('3 页')
  await expect(report.locator('[data-kind="pdf_effects"]')).toBeVisible()
  await expect(report.locator('[data-kind="pdf_raster_parts"]')).toContainText('p3')
  await expect(report.getByTestId('aiws-import-report-show')).toBeVisible()
  await page.screenshot({ path: `${SHOTS}/pi01-report.png` })
  await report.getByTestId('aiws-import-report-close').click()
  await expect(report).toHaveCount(0)
  await expect(page.getByTestId(`aiws-canvas-block-${groups[0].entity_id}`)).toBeVisible()
  await page.screenshot({ path: `${SHOTS}/pi01-canvas.png` })
  const titleBlock = page.getByTestId(`aiws-canvas-block-${title!.e.entity_id}`)
  await expect(titleBlock.locator('svg.aiws-pdf-text text')).toContainText(['Quarterly Review'])
  await titleBlock.dblclick()
  const editor = page.getByTestId(`aiws-shape-edit-${title!.e.entity_id}`)
  if (!(await editor.isVisible())) await titleBlock.dblclick()
  await expect(editor).toBeFocused()
  await page.keyboard.press('ControlOrMeta+a')
  await page.keyboard.type('Annual Review')
  await page.keyboard.press('Enter')
  await expect.poll(async () => (await api.read(ALICE, ws.workspace_id, title!.e.entity_id)).content.payload.config.text).toBe('Annual Review')
  await expect(titleBlock.locator('svg.aiws-pdf-text')).toHaveCount(0)
  await expect.poll(async () => (await api.read(ALICE, ws.workspace_id, title!.e.entity_id)).content.payload.config.pdf_source.text_layout).toBeUndefined()
  await page.keyboard.press('Escape')

  // the show: step 2 crops the stage to page 2 (a 16:9 page on a 16:9 stage fills it)
  await mainMenu(page, 'aiws-menu-collab', 'aiws-top-play')
  await expect(page.getByTestId('aiws-start-show')).toBeVisible()
  await page.getByTestId('aiws-start-show-go').click()
  await expect(page.getByTestId('aiws-show')).toBeVisible({ timeout: 30_000 })
  await expect(page.getByTestId('aiws-show')).toHaveAttribute('data-step', pathPayload.steps[0].id)
  await page.waitForTimeout(300)
  await page.screenshot({ path: `${SHOTS}/pi01-show-step1.png` })
  await page.keyboard.press('ArrowRight')
  await expect(page.getByTestId('aiws-show')).toHaveAttribute('data-step', pathPayload.steps[1].id)
  const stage = (await page.getByTestId('aiws-stage-canvas').boundingBox())!
  const [hx, hy, hw, hh] = (await page.getByTestId('aiws-stage-mask').getAttribute('data-hole'))!.split(',').map(Number)
  const k = Math.min(stage.width / 1920, stage.height / 1080)
  expect(Math.abs(hw - 1920 * k)).toBeLessThan(2)
  expect(Math.abs(hh - 1080 * k)).toBeLessThan(2)
  expect(Math.abs(hx - (stage.width - hw) / 2)).toBeLessThan(2)
  expect(Math.abs(hy - (stage.height - hh) / 2)).toBeLessThan(2)
  // the page colour fades in over 150 ms
  await page.waitForTimeout(300)
  await page.screenshot({ path: `${SHOTS}/pi01-show-step2.png` })
  // the portrait page: letterboxed in the 16:9 stage
  await page.keyboard.press('ArrowRight')
  await expect(page.getByTestId('aiws-show')).toHaveAttribute('data-step', pathPayload.steps[2].id)
  await expect.poll(async () => Number((await page.getByTestId('aiws-stage-mask').getAttribute('data-hole'))!.split(',')[2])).toBeLessThan(hw * 0.6)
  await page.waitForTimeout(300)
  await page.screenshot({ path: `${SHOTS}/pi01-show-step3.png` })
})

test('PI-02 a dropped PDF asks: inserted as a file, or cancelled without writing anything', async ({ page, api }) => {
  const ws = await canvasWorkspace(api, `pi-02 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await drop(page, 'deck.pdf', deck(), { x: 600, y: 400 })
  await expect(page.getByTestId('aiws-pdf-ask')).toBeVisible()
  await page.screenshot({ path: `${SHOTS}/pi02-ask.png` })
  await page.getByTestId('aiws-pdf-ask-file').click()
  await expect.poll(async () => children(await api.outline(ALICE, ws.workspace_id), 'sf-p').filter((e) => e.view_type === 'asset').length, { timeout: 20_000 }).toBe(1)
  const file = children(await api.outline(ALICE, ws.workspace_id), 'sf-p').find((e) => e.view_type === 'asset')
  // again, as pages, then cancelled at the options: nothing new
  await drop(page, 'deck.pdf', deck(), { x: 900, y: 400 })
  await page.getByTestId('aiws-pdf-ask-pages').click()
  await expect(page.getByTestId('aiws-pdf-pages')).toContainText('共 3 页')
  await page.getByTestId('aiws-pdf-dialog').getByRole('button', { name: '取消' }).click()
  await expect(page.getByTestId('aiws-pdf-dialog')).toHaveCount(0)
  const outline = await api.outline(ALICE, ws.workspace_id)
  expect(outline.filter((e: Json) => e.kind === 'surface' && !e.deleted)).toHaveLength(1)

  // the file Block expands into pages: its stored file is the original, no second upload
  await page.getByTestId(`aiws-canvas-block-${file.entity_id}`).click()
  await page.getByTestId('aiws-near-asset-expand-pdf').click()
  const dialog = page.getByTestId('aiws-pdf-dialog')
  await expect(dialog.getByTestId('aiws-pdf-pages')).toContainText('共 3 页')
  await dialog.getByTestId('aiws-pdf-to').fill('1')
  await dialog.getByTestId('aiws-pdf-path').uncheck()
  await dialog.getByTestId('aiws-pdf-start').click()
  await expect(dialog).toHaveCount(0, { timeout: 60_000 })
  let after: Json[] = []
  await expect.poll(async () => { after = await api.outline(ALICE, ws.workspace_id); return after.filter((e) => e.kind === 'surface' && !e.deleted).length }).toBe(2)
  const surface = after.find((e) => e.kind === 'surface' && e.entity_id !== 'sf-p')
  expect(children(after, surface.entity_id)).toHaveLength(1)
  expect(after.some((e) => e.type_id === 'buckyos.show-path' && !e.deleted)).toBe(false)
  const fileRef = (await api.read(ALICE, ws.workspace_id, file.source_id)).content.payload.object_id
  const kept = children(after, surface.content_folder_id).find((e) => e.media_type === 'application/pdf')
  expect((await api.read(ALICE, ws.workspace_id, kept.entity_id)).content.payload.object_id).toBe(fileRef)
})

test('PI-03 editable PDF keeps fragment gaps and compressed advances after import and refresh', async ({ page, api }) => {
  test.setTimeout(120_000)
  const ws = await canvasWorkspace(api, `pi-03 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await chooseFromDevice(page, () => mainMenu(page, 'aiws-menu-insert', 'aiws-menu-import-file', 'aiws-menu-import-pdf'), { name: 'spaced.pdf', mimeType: 'application/pdf', buffer: spacedText() })
  const dialog = page.getByTestId('aiws-pdf-dialog')
  await expect(dialog.getByTestId('aiws-pdf-pages')).toContainText('共 1 页')
  await dialog.getByTestId('aiws-pdf-start').click()
  await expect(dialog).toHaveCount(0, { timeout: 60_000 })
  const report = page.getByTestId('aiws-import-report')
  await expect(report).toHaveCount(0)
  const outline = await api.outline(ALICE, ws.workspace_id)
  const configs = await Promise.all(outline.filter((e: Json) => e.view_type === 'shape' && !e.deleted).map(async (e: Json) => ({ id: e.entity_id, config: (await api.read(ALICE, ws.workspace_id, e.entity_id)).content.payload.config })))
  const fragments = configs.flatMap(({ id, config }) => (config.pdf_source?.text_layout?.spans ?? []).map((span: Json) => ({ id, span })))
  const hello = fragments.find(({ span }) => span.text === 'hello')!
  const world = fragments.find(({ span }) => span.text === 'world')!
  const compressed = fragments.find(({ span }) => span.text === 'MMMMMMMM')!
  expect(hello).toBeTruthy()
  expect(world).toBeTruthy()
  expect(compressed).toBeTruthy()
  expect(world.id).toBe(hello.id)
  expect(world.span.x - hello.span.x - hello.span.width).toBeCloseTo(1.7 * 6.4, 1)
  expect(compressed.span.width).toBeCloseTo(8 * 0.833 * 20 * 0.5 * 6.4, 1)
  expect(compressed.span.size).toBe(128)
  const check = async () => {
    for (const { id, span } of [hello, world, compressed]) {
      const rendered = page.getByTestId(`aiws-shape-${id}`).locator('svg.aiws-pdf-text text').filter({ hasText: new RegExp(`^${span.text}$`) })
      await expect(rendered).toHaveCount(1)
      const geometry = await rendered.evaluate((node) => {
        const el = node as SVGTextElement
        const first = el.getStartPositionOfChar(0), last = el.getEndPositionOfChar(el.getNumberOfChars() - 1)
        return { x: first.x, y: first.y, advance: last.x - first.x }
      })
      expect(geometry.x).toBeCloseTo(span.x, 1)
      expect(geometry.y).toBeCloseTo(span.y, 1)
      expect(geometry.advance).toBeCloseTo(span.width, 1)
    }
  }
  await check()
  await page.screenshot({ path: `${SHOTS}/pi03-fragment-layout.png` })
  const imported = outline.find((e: Json) => e.kind === 'surface' && e.entity_id !== 'sf-p' && !e.deleted)
  await openWorkspacePage(page, ALICE, ws.workspace_id, imported.entity_id)
  await page.reload()
  await check()
})

test('PI-04 layout overflow preserves the opaque background and all text in one page raster', async ({ page, api }) => {
  test.setTimeout(120_000)
  const ws = await canvasWorkspace(api, `pi-04 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await chooseFromDevice(page, () => mainMenu(page, 'aiws-menu-insert', 'aiws-menu-import-file', 'aiws-menu-import-pdf'), { name: 'overflow.pdf', mimeType: 'application/pdf', buffer: overflowText() })
  const dialog = page.getByTestId('aiws-pdf-dialog')
  await expect(dialog.getByTestId('aiws-pdf-pages')).toContainText('共 1 页')
  await dialog.getByTestId('aiws-pdf-start').click()
  await expect(dialog).toHaveCount(0, { timeout: 60_000 })
  const report = page.getByTestId('aiws-import-report')
  await expect(report.locator('[data-kind="pdf_text_layout_limit"]')).toBeVisible()
  await report.getByTestId('aiws-import-report-close').click()
  const outline = await api.outline(ALICE, ws.workspace_id)
  const surface = outline.find((e: Json) => e.kind === 'surface' && e.entity_id !== 'sf-p' && !e.deleted)
  const group = children(outline, surface.entity_id)[0]
  const objects = children(outline, group.entity_id)
  expect(objects.map((e: Json) => e.view_type)).toEqual(['frame', 'asset'])
  const raster = objects[1]
  const config = (await api.read(ALICE, ws.workspace_id, raster.entity_id)).content.payload.config
  expect(config.pdf_source.kind).toBe('raster')
  const block = page.getByTestId(`aiws-canvas-block-${raster.entity_id}`)
  const image = block.locator('img')
  await expect(image).toBeVisible()
  const pixels = await image.evaluate(async (node) => {
    const image = node as HTMLImageElement
    await image.decode()
    const canvas = document.createElement('canvas')
    canvas.width = image.naturalWidth
    canvas.height = image.naturalHeight
    const ctx = canvas.getContext('2d')!
    ctx.drawImage(image, 0, 0)
    const { data } = ctx.getImageData(0, 0, canvas.width, canvas.height)
    let dark = 0, gray = 0
    for (let i = 0; i < data.length; i += 4) {
      if (data[i + 3] < 240) continue
      if (data[i] < 80 && data[i + 1] < 80 && data[i + 2] < 80) dark++
      if (data[i] > 120 && data[i] < 190 && Math.abs(data[i] - data[i + 1]) < 3 && Math.abs(data[i] - data[i + 2]) < 3) gray++
    }
    return { dark, gray }
  })
  expect(pixels.dark).toBeGreaterThan(1000)
  expect(pixels.gray).toBeGreaterThan(10000)
  await page.screenshot({ path: `${SHOTS}/pi04-overflow-background.png` })
})

test('PI-05 an import is one Commit, sent in parts: a refused one leaves nothing; one undo takes it back, one redo restores it', async ({ page, api }) => {
  test.setTimeout(120_000)
  const ws = await canvasWorkspace(api, `pi-05 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  // parts of 4 KB: this three-page import goes through `doc.stage` the way a 30 MB one does
  await page.evaluate(() => {
    const w = window as unknown as { __aiwsTestHooks?: Record<string, unknown> }
    w.__aiwsTestHooks ??= { editors: {} }
    w.__aiwsTestHooks.commitPartBytes = 4096
  })
  const sent: string[] = []
  let refuse = true
  await page.route('**/kapi/aiworkspace', async (route) => {
    let body: Json = null
    try { body = route.request().postDataJSON() } catch { /* not JSON */ }
    if (body?.method === 'doc.stage' || body?.method === 'doc.commit') sent.push(body.method)
    if (refuse && body?.method === 'doc.commit' && Array.isArray(body.params?.staged)) {
      await route.fulfill({ json: { result: { status: 'rejected', code: 'QUOTA_LIMIT', retryable: false, detail: 'the workspace is full', errors: [{ code: 'QUOTA_LIMIT', retryable: false, detail: 'the workspace is full' }] }, sys: [body.sys?.[0]] } })
      return
    }
    await route.continue()
  })
  const start = async () => {
    await chooseFromZone(page, () => mainMenu(page, 'aiws-menu-insert', 'aiws-menu-import-file', 'aiws-menu-import-pdf'), { name: 'deck.pdf', mimeType: 'application/pdf', buffer: deck() })
    const dialog = page.getByTestId('aiws-pdf-dialog')
    await expect(dialog.getByTestId('aiws-pdf-pages')).toContainText('共 3 页')
    await dialog.getByTestId('aiws-pdf-start').click()
    await expect(dialog).toHaveCount(0, { timeout: 60_000 })
  }
  const live = async () => (await api.outline(ALICE, ws.workspace_id)).filter((e: Json) => !e.deleted)
  const before = await live()
  const head = await api.headSeq(ALICE, ws.workspace_id)

  // the final commit is refused: no canvas, no pages, no path — and nothing to undo
  await start()
  const report = page.getByTestId('aiws-import-report')
  await expect(report.getByTestId('aiws-import-error')).toContainText('导入没有写入')
  await expect(report.getByTestId('aiws-import-error')).toContainText('QUOTA_LIMIT')
  await report.getByTestId('aiws-import-report-close').click()
  expect(sent.filter((m) => m === 'doc.stage').length).toBeGreaterThan(1)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head)
  expect((await live()).map((e: Json) => e.entity_id).sort()).toEqual(before.map((e: Json) => e.entity_id).sort())

  // accepted: every part, one Commit
  refuse = false
  sent.length = 0
  await start()
  await expect.poll(async () => (await live()).filter((e: Json) => e.kind === 'surface').length).toBe(2)
  expect(sent.filter((m) => m === 'doc.commit')).toHaveLength(1)
  expect(sent.filter((m) => m === 'doc.stage').length).toBeGreaterThan(1)
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 1)
  const imported = await live()
  expect(imported.some((e: Json) => e.type_id === 'buckyos.show-path')).toBe(true)
  await report.getByTestId('aiws-import-report-close').click()
  await expect(page.getByTestId('aiws-undo')).toHaveText(/撤销 1/)

  // one undo: the canvas, its pages, the kept file and the path are gone together
  await page.getByTestId('aiws-undo').click()
  await expect.poll(async () => (await live()).map((e: Json) => e.entity_id).sort()).toEqual(before.map((e: Json) => e.entity_id).sort())
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 2)
  // one redo: all of it back
  await page.getByTestId('aiws-redo').click()
  await expect.poll(async () => (await live()).map((e: Json) => e.entity_id).sort()).toEqual(imported.map((e: Json) => e.entity_id).sort())
  expect(await api.headSeq(ALICE, ws.workspace_id)).toBe(head + 3)
})
