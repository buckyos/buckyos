/* The dialog tests cover PDF export from flow pages and presentation paths, fonts, errors, cancellation, permissions
 * and phones. They intercept print.package, media.page and the PDF download so the workflow can be driven through each
 * outcome. The table test uses the production renderer from a real print.package and prints its pages in Chromium. */

import { devices, type Page } from '@playwright/test'
import { execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
import { PACK_FONTS } from '@print/fonts'
import { expect, hooks, mainMenu, openCanvas, openWorkspacePage, openedPicker, test, zoneGoto, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

const PRESENTATION = resolve('../aiworkspace/fixtures/presentation/commits.json')
const PKG_ID = `cyfile:${'0a'.repeat(32)}`
const PDF_ID = `cyfile:${'0b'.repeat(32)}`
const PDF = Buffer.from('%PDF-1.4\n% aiws print e2e\n1 0 obj << /Type /Catalog >> endobj\n%%EOF\n')
const READY = 'window.__AIWS_PRINT_READY__ === true'
/** A sans-serif family of the font package (`ext-fonts`): not on a node until an administrator downloads it. */
const PACK_SANS = PACK_FONTS.find((f) => f.generic === 'sans')!.family

interface Fake {
  /** `print.package` params, in order. */
  packages: Json[]
  /** `create_run` requests. */
  runs: Json[]
  cancels: string[]
  /** How the next Runs end. */
  outcome: 'completed' | 'browser_missing' | 'hang'
  issues: Json[]
  pages: number
}

/** The service's `print.package`, the Workflow and NDM's read of the PDF, answered here. */
async function fakePrint(page: Page, init: Partial<Fake> = {}): Promise<Fake> {
  const fake: Fake = { packages: [], runs: [], cancels: [], outcome: 'completed', issues: [], pages: 3, ...init }
  await page.route('**/kapi/aiworkspace', async (route) => {
    const body = route.request().postDataJSON() as Json
    if (body?.method !== 'print.package') return route.fallback()
    fake.packages.push(body.params)
    const job = { surface_id: body.params.surface_id, mode: body.params.path_id ? 'free' : 'flow', path_id: body.params.path_id ?? null, paper: body.params.paper, frame_id: null, fonts: {}, title: 'x', date: body.params.date, locale: body.params.locale }
    await route.fulfill({ json: { result: { ok: true, obj_id: PKG_ID, size: 4096, job, issues: fake.issues }, sys: [body.sys?.[0] ?? 0] } })
  })
  const polls = new Map<string, number>()
  await page.route('**/kapi/workflow', async (route) => {
    const body = route.request().postDataJSON() as Json
    const ok = (result: Json) => route.fulfill({ json: { result: { ok: true, ...result }, sys: [body.sys?.[0] ?? 0] } })
    const runId = body.params?.run_id as string
    switch (body.method) {
      case 'create_run':
        fake.runs.push(body.params)
        return ok({ run_id: `run-${fake.runs.length}`, status: 'created', events: [], workflow_id: 'media.page', revision: 'rev', reused: false, output: {}, seq: 1 })
      case 'start_run':
        return ok({ run_id: runId, status: 'running', events: [] })
      case 'cancel_run':
        fake.cancels.push(runId)
        return ok({ run_id: runId, status: 'aborted', events: [] })
      case 'get_run_output': {
        const n = (polls.get(runId) ?? 0) + 1
        polls.set(runId, n)
        const base = { run_id: runId, revision: 'rev', node_errors: {}, run_error: null, delivery: { artifacts: [], held: 0 } }
        if (fake.cancels.includes(runId)) return ok({ ...base, status: 'aborted', terminal: true, output: {}, run_error: { code: 'cancelled', message: 'cancelled', details: {}, retryable: false } })
        if (fake.outcome === 'hang' || n < 2) return ok({ ...base, status: 'running', terminal: false, output: {} })
        if (fake.outcome === 'browser_missing') {
          const error = { code: 'tools_missing', message: 'this node has not downloaded the browser yet', details: { tools_missing: ['chromium'], online_item: 'chromium', tool: 'page-render' }, retryable: false }
          return ok({ ...base, status: 'failed', terminal: true, output: {}, node_errors: { page_to_pdf: error }, run_error: error })
        }
        return ok({ ...base, status: 'completed', terminal: true, output: { page_to_pdf: { pdf_obj_id: PDF_ID, pdf_content_id: `sha256:${'0c'.repeat(32)}`, pages: fake.pages } }, delivery: { artifacts: [PDF_ID], held: 1 } })
      }
      default:
        return route.fulfill({ json: { result: { ok: false, error: 'not_found', message: body.method }, sys: [0] } })
    }
  })
  await page.route('**/ndm/v1/store/get_object', (route) => route.fulfill({ json: { obj_id: PDF_ID, obj_data: JSON.stringify({ name: 'print.pdf', size: PDF.length, content: `sha256:${'0c'.repeat(32)}` }) } }))
  await page.route('**/ndm/proxy/v1/read/object/open', (route) => route.fulfill({ status: 200, contentType: 'application/pdf', body: PDF }))
  return fake
}

async function openPrint(page: Page) {
  await mainMenu(page, 'aiws-menu-surface', 'aiws-menu-export-pdf')
}

async function presentationWorkspace(api: Api, title: string): Promise<{ workspace_id: string; epoch: string }> {
  const ws = await api.rpc(ALICE, 'ws.create', { title }) as { workspace_id: string; epoch: string }
  const fixture = JSON.parse(readFileSync(PRESENTATION, 'utf8')) as { commits: { operations: Json[] }[] }
  for (const [index, commit] of fixture.commits.entries()) {
    const result = await api.commit(ALICE, ws, commit.operations, `presentation/${index + 1}`)
    expect(result.status, JSON.stringify(result)).toBe('accepted')
  }
  return ws
}

async function saveIntoDocuments(page: Page, fileName: string) {
  const save = (await openedPicker(page)).dialog
  await zoneGoto(save, ['Documents'])
  await expect(save.getByTestId('select-name')).toHaveValue(fileName)
  await save.getByTestId('select-confirm').click()
}

test('PR-01 a flow page: straight to the dialog — A4 portrait, no PDF Frame —, the place in the Zone first, then the PDF; another size downloaded', async ({ page, api }) => {
  test.setTimeout(150_000)
  const fake = await fakePrint(page, { issues: [{ code: 'asset_missing', entity_id: 'a' }, { code: 'asset_missing', entity_id: 'b' }, { code: 'content_unreadable' }] })
  const ws = await api.sample(ALICE, `打印 流式 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await openPrint(page)
  const dialog = page.getByTestId('aiws-print-dialog')
  await expect(dialog).toBeVisible()
  await expect(dialog).toContainText('项目工作区')
  // a flow page has no path to choose; the page size defaults to A4 portrait (Q5); no PDF Frame by default
  await expect(dialog.getByTestId('aiws-print-path')).toHaveCount(0)
  await expect(dialog.getByTestId('aiws-print-paper')).toHaveValue('A4')
  await expect(dialog.getByTestId('aiws-print-portrait')).toHaveAttribute('aria-pressed', 'true')
  await expect(dialog.getByTestId('aiws-print-frame')).toHaveValue('')
  await expect(dialog.getByTestId('aiws-print-pages')).toContainText('自动分页')
  await expect(dialog.getByTestId('aiws-print-fonts-missing')).toHaveCount(0)
  await expect(dialog.getByTestId('aiws-print-zone')).toHaveClass(/is-primary/)

  // the Zone first: the place, then the print package, the Run, the commit
  await dialog.getByTestId('aiws-print-zone').click()
  await saveIntoDocuments(page, '项目工作区.pdf')
  await expect(dialog.getByTestId('aiws-print-done')).toContainText('/home/Documents/项目工作区.pdf', { timeout: 30_000 })
  expect(fake.packages).toHaveLength(1)
  expect(fake.packages[0]).toMatchObject({ workspace_id: ws.workspace_id, surface_id: 'surface-main', paper: { name: 'A4', width_mm: 210, height_mm: 297 }, locale: 'zh-CN' })
  // the page context's date as a PDF Frame shows it while edited: the person's day in their language
  expect(fake.packages[0].date).toMatch(/^\d{4}年\d{1,2}月\d{1,2}日$/)
  expect(fake.packages[0]).not.toHaveProperty('path_id')
  expect(fake.packages[0]).not.toHaveProperty('frame_id')
  expect(fake.runs[0]).toEqual({ workflow_id: 'media.page', input: { obj_id: PKG_ID, output: 'pdf', paper: '210x297mm', landscape: false, margin_mm: 0, background: true, wait_ready: READY } })
  // afterwards: the pages of the PDF and what the service left out, by kind (no names)
  await expect(dialog.getByTestId('aiws-print-pages')).toHaveText('PDF 共 3 页')
  const issues = dialog.getByTestId('aiws-print-issues')
  await expect(issues.locator('li[data-code="asset_missing"]')).toContainText('2 个文件缺失')
  await expect(issues.locator('li[data-code="content_unreadable"]')).toContainText('1 处')
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '已保存到 /home/Documents/项目工作区.pdf' })).toBeVisible()

  // A5 landscape, downloaded to this device: the bytes NDM holds for the caller
  await dialog.getByTestId('aiws-print-paper').selectOption('A5')
  await dialog.getByTestId('aiws-print-landscape').click()
  const download = page.waitForEvent('download')
  await dialog.getByTestId('aiws-print-device').click()
  const file = await download
  expect(file.suggestedFilename()).toBe('项目工作区.pdf')
  expect(readFileSync(await file.path())).toEqual(PDF)
  await expect(dialog.getByTestId('aiws-print-done')).toContainText('已下载')
  expect(fake.packages[1].paper).toEqual({ name: 'A5', width_mm: 210, height_mm: 148 })
  expect(fake.runs[1].input.paper).toBe('210x148mm')
  await dialog.getByTestId('aiws-print-close').click()
  await expect(dialog).toHaveCount(0)

  // the data mode has no print entry
  await mainMenu(page, 'aiws-menu-data-mode')
  await mainMenu(page, 'aiws-menu-surface')
  await expect(page.getByTestId('aiws-menu-export-pdf')).toBeDisabled()
})

test('PR-02 a free canvas without a presentation path: no dialog, the reason, and path editing; without `export` the entry says why', async ({ page, api, browser }) => {
  test.setTimeout(120_000)
  await fakePrint(page)
  const ws = await api.freeCanvas(ALICE, `打印 无路径 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await openPrint(page)
  const notice = page.getByTestId('aiws-notice').filter({ hasText: '这个画布没有演示路径，无法打印' })
  await expect(notice).toBeVisible()
  await expect(page.getByTestId('aiws-print-dialog')).toHaveCount(0)
  await notice.getByTestId('aiws-notice-action').click()
  await expect.poll(async () => (await hooks(page)).canvas?.mode).toBe('presentation_edit')

  // bob reads the workspace but may not export it
  expect((await api.rpc(ALICE, 'acl.set', { workspace_id: ws.workspace_id, changes: [{ subject: 'bob', allow: ['read'] }] })).ok).not.toBe(false)
  const context = await browser.newContext()
  const bobPage = await context.newPage()
  await openWorkspacePage(bobPage, BOB, ws.workspace_id)
  await mainMenu(bobPage, 'aiws-menu-surface')
  const entry = bobPage.getByTestId('aiws-menu-export-pdf')
  await expect(entry).toBeDisabled()
  await expect(entry).toHaveAttribute('title', '没有导出权限')
  await context.close()
})

test('PR-03 a free canvas prints along its presentation paths: guides left out, the pages and the ones on another canvas, what is skipped, the stage\'s page size', async ({ page, api }) => {
  test.setTimeout(150_000)
  const fake = await fakePrint(page)
  const ws = await presentationWorkspace(api, `打印 路径 ${Date.now()}`)
  // a second path through this canvas: a step turned off is not printed
  const made = await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'path-second', type_id: 'buckyos.show-path', parent_id: 'shows', order_key: 'q', name: '精简版', payload: { title: '精简版', purpose: 'presentation', stage: { w: 1440, h: 1080 }, steps: [
      { id: 'a', target: { kind: 'frame', entity_id: 'frame1' } }, { id: 'b', target: { kind: 'frame', entity_id: 'frame2' }, enabled: false }, { id: 'c', target: { kind: 'frame', entity_id: 'frame3' } }] } },
  ])
  expect(made.status, JSON.stringify(made)).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  await openPrint(page)
  const dialog = page.getByTestId('aiws-print-dialog')
  await expect(dialog).toContainText('发布会')
  // both presentation paths, not the guide
  const path = dialog.getByTestId('aiws-print-path')
  await expect(path.locator('option')).toHaveText(['五分钟介绍', '精简版'])
  // five steps, the last on the other canvas; a 16:9 stage prints on 16:9 landscape
  await expect(dialog.getByTestId('aiws-print-pages')).toHaveText('共 5 页（其中 1 页在其他画布上）')
  await expect(dialog.getByTestId('aiws-print-paper')).toHaveValue('16:9')
  await expect(dialog.getByTestId('aiws-print-landscape')).toHaveAttribute('aria-pressed', 'true')
  await expect(dialog.getByTestId('aiws-print-skipped')).toHaveCount(0)

  await path.selectOption({ label: '精简版' })
  await expect(dialog.getByTestId('aiws-print-pages')).toHaveText('共 2 页（其中 1 页在其他画布上）')
  await expect(dialog.getByTestId('aiws-print-skipped')).toContainText('第 2 步「问题」不会打印：已停用')
  await expect(dialog.getByTestId('aiws-print-paper')).toHaveValue('4:3')

  await dialog.getByTestId('aiws-print-zone').click()
  await saveIntoDocuments(page, '发布会.pdf')
  await expect(dialog.getByTestId('aiws-print-done')).toContainText('/home/Documents/发布会.pdf', { timeout: 30_000 })
  expect(fake.packages[0]).toMatchObject({ surface_id: 's-launch', path_id: 'path-second', paper: { name: '4:3', width_mm: 297, height_mm: 222.75 } })
  expect(fake.runs[0].input.paper).toBe('297x222.75mm')
})

test('PR-04 print fonts: set in the workspace (工作区 ▸ 打印字体…); a font of the font package not on the node is named, with the way to install it; printing still goes on', async ({ page, api }) => {
  test.setTimeout(150_000)
  await fakePrint(page)
  const ws = await api.sample(ALICE, `打印 字体 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)

  await mainMenu(page, 'aiws-menu-workspace', 'aiws-menu-print-fonts')
  const fonts = page.getByTestId('aiws-print-fonts-dialog')
  await expect(fonts.getByTestId('aiws-print-font-sans')).toHaveValue('')
  await fonts.getByTestId('aiws-print-font-sans').selectOption(PACK_SANS)
  await fonts.getByTestId('aiws-print-font-serif').selectOption('Noto Serif CJK JP')
  await fonts.getByTestId('aiws-print-fonts-save').click()
  await expect(fonts).toHaveCount(0)
  await expect.poll(async () => (await api.read(ALICE, ws.workspace_id, 'root')).content.payload.config?.fonts).toEqual({ sans: PACK_SANS, serif: 'Noto Serif CJK JP' })

  // the font package is not on the node: the dialog names the missing family (a default font is never missing)
  await openPrint(page)
  const dialog = page.getByTestId('aiws-print-dialog')
  const missing = dialog.getByTestId('aiws-print-fonts-missing')
  await expect(missing).toContainText(PACK_SANS)
  await expect(missing).not.toContainText('Noto Serif CJK JP')
  await expect(missing).toContainText('打印可能失真')
  // an administrator goes to Settings › Extension tools
  await missing.getByTestId('aiws-print-fonts-install').click()
  const settings = page.getByTestId('window-settings')
  await expect(settings).toBeVisible()
  await settings.getByRole('button', { name: /^(Close|关闭)$/ }).first().click()
  await expect(settings).toHaveCount(0)
  // printing is not stopped
  await expect(dialog.getByTestId('aiws-print-zone')).toBeEnabled()

  // from the dialog to the print fonts and back: a default font instead, no notice
  await dialog.getByTestId('aiws-print-fonts-settings').click()
  await expect(dialog).toHaveCount(0)
  await expect(fonts.getByTestId('aiws-print-font-sans')).toHaveValue(PACK_SANS)
  await fonts.getByTestId('aiws-print-font-sans').selectOption('DejaVu Sans')
  await fonts.getByTestId('aiws-print-fonts-save').click()
  await expect.poll(async () => (await api.read(ALICE, ws.workspace_id, 'root')).content.payload.config?.fonts).toEqual({ sans: 'DejaVu Sans', serif: 'Noto Serif CJK JP' })
  await expect(fonts).toHaveCount(0)
  await expect(dialog).toBeVisible()
  await expect(dialog.getByTestId('aiws-print-fonts-missing')).toHaveCount(0)
})

test('PR-05 the browser is not downloaded yet: the dialog says so, the administrator downloads it there, then the print goes through; nothing was saved meanwhile', async ({ page, api }) => {
  test.setTimeout(150_000)
  const fake = await fakePrint(page, { outcome: 'browser_missing' })
  const ws = await api.sample(ALICE, `打印 浏览器 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  // the mock control panel (api/settings_mock.ts, loaded on its first call): ext-core's browser not downloaded on ood1
  await page.evaluate(async () => {
    const mock = '/src/api/settings_mock.ts'
    await import(/* @vite-ignore */ mock)
    ;(window as unknown as { __extToolsMock: { setOnline(node: string, item: string, state: string): void } }).__extToolsMock.setOnline('ood1', 'chromium', 'missing')
  })
  let downloads = 0
  page.on('download', () => { downloads += 1 })
  await openPrint(page)
  const dialog = page.getByTestId('aiws-print-dialog')
  await dialog.getByTestId('aiws-print-device').click()
  await expect(dialog.getByTestId('aiws-print-failure')).toHaveAttribute('data-kind', 'browser_missing', { timeout: 30_000 })
  await expect(dialog.getByTestId('aiws-print-failure')).toContainText('还没有下载用来排版的浏览器')
  expect(downloads).toBe(0)
  const browserBox = dialog.getByTestId('aiws-print-browser')
  await browserBox.getByTestId('aiws-print-browser-download').click()
  await expect(browserBox).toHaveAttribute('data-state', 'downloading', { timeout: 10_000 })
  await expect(browserBox.getByTestId('aiws-print-browser-ready')).toBeVisible({ timeout: 30_000 })

  // the node has it now: the same dialog prints, nothing restarted
  fake.outcome = 'completed'
  const download = page.waitForEvent('download')
  await dialog.getByTestId('aiws-print-device').click()
  expect((await download).suggestedFilename()).toBe('项目工作区.pdf')
  await expect(dialog.getByTestId('aiws-print-done')).toBeVisible()
  await expect(dialog.getByTestId('aiws-print-failure')).toHaveCount(0)
  expect(fake.runs).toHaveLength(2)
})

test('PR-06 cancelling while the PDF is laid out cancels the Run and leaves no file', async ({ page, api }) => {
  test.setTimeout(120_000)
  const fake = await fakePrint(page, { outcome: 'hang' })
  const ws = await api.sample(ALICE, `打印 取消 ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  let downloads = 0
  page.on('download', () => { downloads += 1 })
  await openPrint(page)
  const dialog = page.getByTestId('aiws-print-dialog')
  await dialog.getByTestId('aiws-print-device').click()
  const progress = dialog.getByTestId('aiws-print-progress')
  await expect(progress).toHaveAttribute('data-phase', 'render')
  await expect(progress).toContainText('排版中')
  // closing is not offered while it runs; cancelling is
  await expect(dialog.getByTestId('aiws-print-close')).toBeDisabled()
  await progress.getByTestId('aiws-print-cancel').click()
  await expect(dialog.getByTestId('aiws-print-failure')).toHaveAttribute('data-kind', 'cancelled')
  await expect(dialog.getByTestId('aiws-print-failure')).toContainText('已取消，没有生成文件')
  expect(fake.cancels).toEqual(['run-1'])
  await expect(progress).toHaveCount(0)
  expect(downloads).toBe(0)
  await dialog.getByTestId('aiws-print-close').click()
  await expect(dialog).toHaveCount(0)
})

test.describe('phone', () => {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the browser comes from the project, the rest is the phone
  const { defaultBrowserType, ...pixel } = devices['Pixel 7']
  test.use(pixel)

  test('PR-07 on a phone: 画布 ▸ 导出可打印 PDF… in the phone\'s main menu, the dialog full screen, saved into the Zone', async ({ page, api }) => {
    test.setTimeout(150_000)
    const fake = await fakePrint(page)
    const ws = await api.sample(ALICE, `打印 手机 ${Date.now()}`)
    await openWorkspacePage(page, ALICE, ws.workspace_id)
    await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
    await page.getByTestId('aiws-main-menu').tap()
    const menu = page.getByTestId('aiws-main-menu-list')
    await menu.getByTestId('aiws-menu-surface').tap()
    await menu.getByTestId('aiws-menu-export-pdf').tap()
    const dialog = page.getByTestId('aiws-print-dialog')
    await expect(dialog).toBeVisible()
    const { width, height } = page.viewportSize()!
    const box = (await dialog.boundingBox())!
    expect(box.x).toBeGreaterThanOrEqual(0)
    expect(Math.round(box.width)).toBe(width)
    expect(Math.round(box.height)).toBe(height)
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
    await dialog.getByTestId('aiws-print-zone').tap()
    await saveIntoDocuments(page, '项目工作区.pdf')
    await expect(dialog.getByTestId('aiws-print-done')).toContainText('/home/Documents/项目工作区.pdf', { timeout: 30_000 })
    expect(fake.runs).toHaveLength(1)
  })
})

test('two views of one source print their own titles, columns, row heights, headers and styles while the screen keeps its card preference', async ({ page, api }, testInfo) => {
  const ws = await api.sample(ALICE, `打印表格 ${Date.now()}`)
  const allId = 'cell-all-tasks'
  const openId = 'cell-open-tasks'
  const setView = async (id: string, title: string, titleWidth: number, config: Json) => {
    const { content } = await api.read(ALICE, ws.workspace_id, id)
    const settings = { title, fields: [{ field_id: 'title', width: titleWidth }, { field_id: 'owner', width: 90 }], config }
    return { op: 'entity.set_keys', entity_id: id, keys: Object.entries(settings).map(([key, value]) => ({ key, value, expect: { rev: content.key_revs[key] ?? 0 } })) }
  }
  const saved = await api.commit(ALICE, ws, [
    await setView(allId, '全表交付清单', 200, { row_heights: { 'task-41': 80 }, grid_style: 'striped', show_header: false }),
    await setView(openId, '待办交付清单', 300, { row_heights: { 'task-41': 45 }, grid_style: 'lines', show_header: true }),
  ])
  expect(saved.status, JSON.stringify(saved)).toBe('accepted')
  await openWorkspacePage(page, ALICE, ws.workspace_id, 'surface-main')
  const allOnScreen = page.getByTestId(`aiws-table-${allId}`)
  const openOnScreen = page.getByTestId(`aiws-cell-frame-${openId}`).getByTestId(`aiws-table-${openId}`)
  await allOnScreen.getByTestId(`aiws-table-menu-${allId}`).click()
  await page.getByTestId('aiws-ctl-menu-table-layout-cards').click()
  await expect(allOnScreen).toHaveAttribute('data-layout', 'cards')
  await expect(openOnScreen).toHaveAttribute('data-layout', 'grid')

  const pkg = await api.rpc(ALICE, 'print.package', { workspace_id: ws.workspace_id, surface_id: 'surface-main',
    paper: { name: 'A4', width_mm: 210, height_mm: 297 }, locale: 'zh-CN', date: '2026年10月8日' })
  expect(pkg.ok, JSON.stringify(pkg)).toBe(true)
  // The standalone NFS serves the workspace owner's exports bucket, including the immutable print package.
  const bucket = 'user/aiworkspace/alice/exports'
  const listed = await fetch(`${api.nfs}/kapi/nfs-server`, { method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ method: 'bucket.list_objects', params: { bucket_id: bucket, prefix: `${ws.workspace_id}/print/` }, sys: [1, ''] }) })
  const listing = await listed.json()
  expect(listing.result?.items, JSON.stringify(listing)).toHaveLength(1)
  const item = listing.result.items[0]
  expect(item.obj_id).toBe(pkg.obj_id)
  const zip = await fetch(`${api.nfs}/bucket/v1/${bucket}/${item.key}`)
  expect(zip.ok).toBe(true)
  const archive = testInfo.outputPath('package.zip')
  writeFileSync(archive, Buffer.from(await zip.arrayBuffer()))
  const folder = testInfo.outputPath('print')
  execFileSync('unzip', ['-q', archive, '-d', folder])
  const printed = await page.context().newPage()
  await printed.emulateMedia({ media: 'print' })
  await printed.goto(pathToFileURL(resolve(folder, 'index.html')).href)
  await printed.waitForFunction(READY)
  const allBlock = printed.locator(`[data-cell-id="${allId}"]`)
  const openBlock = printed.locator(`[data-cell-id="${openId}"]`)
  await expect(allBlock.locator('.aiws-fblock-title')).toHaveText('全表交付清单')
  await expect(openBlock.locator('.aiws-fblock-title')).toHaveText('待办交付清单')
  const all = allBlock.getByTestId(`aiws-print-table-${allId}`)
  const open = openBlock.getByTestId(`aiws-print-table-${openId}`)
  await expect(all.locator('thead')).toHaveCount(0)
  await expect(open.first().locator('thead th')).toHaveText(['任务', '负责人'])
  const allRow = all.locator('tbody tr').filter({ hasText: '定义对象格式' })
  const openRow = open.locator('tbody tr').filter({ hasText: '定义对象格式' })
  await expect(allRow).toHaveCSS('height', '80px')
  await expect(openRow).toHaveCSS('height', '45px')
  await expect(all.locator('tbody tr').filter({ hasText: '完成第一期架构验证' })).toHaveCSS('height', '34px')
  await expect(all.first().locator('col').first()).toHaveCSS('width', '200px')
  await expect(open.first().locator('col').first()).toHaveCSS('width', '300px')
  await expect(all.locator('tr[data-stripe="1"]').first()).not.toHaveCSS('background-color', 'rgba(0, 0, 0, 0)')
  await expect(allRow.locator('td').first()).toHaveCSS('border-left-width', '1px')
  await expect(openRow.locator('td').first()).toHaveCSS('border-left-width', '0px')
  await expect(printed.getByTestId('aiws-table-card')).toHaveCount(0)
  const pdf = await printed.pdf({ preferCSSPageSize: true, printBackground: true })
  expect(pdf.subarray(0, 5).toString()).toBe('%PDF-')
  await expect(allOnScreen).toHaveAttribute('data-layout', 'cards')
  await expect(openOnScreen).toHaveAttribute('data-layout', 'grid')
})
