/* Export a printable PDF on a real Zone (打印 TODO §7, T10): the real `print.package`, the real Workflow `media.page`
 * Run in the node's Chromium as the person printing, the PDF committed into their own Zone folder by object id or read
 * from NDM for a download. Run with playwright.dv.config.ts (BUCKYOS_UI_DV_BASE_URL=https://<zone>,
 * BUCKYOS_TEST_ADMIN_PASSWORD); the PDFs are checked with the node's poppler (`/opt/buckyos/tools/bin`, or AIWS_DV_POPPLER). */
import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { devices } from '@playwright/test'
import type { Operation } from '../../src/app/aiworkspace/api/types'
import { mainMenu, openedPicker, zoneGoto } from '../aiworkspace/fixtures'
import { expect, openWorkspace, screenshot, test, workspaceOrigin, type Actor } from './aiworkspace.fixtures'

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any
const FIXTURES = resolve('../aiworkspace/fixtures')
const POPPLER = process.env.AIWS_DV_POPPLER ?? '/opt/buckyos/tools/bin'

const poppler = (tool: string, ...args: string[]) => execFileSync(join(POPPLER, tool), args, { encoding: 'utf8' })
const pdfInfo = (file: string) => {
  const text = poppler('pdfinfo', file)
  const size = /Page size:\s+([\d.]+) x ([\d.]+) pts/.exec(text)!
  return { pages: Number(/Pages:\s+(\d+)/.exec(text)![1]), w: Number(size[1]), h: Number(size[2]) }
}
/** The PDF's text without white space (small print loses its spaces in extraction). */
const pdfText = (file: string) => poppler('pdftotext', file, '-').replace(/\s+/g, '')
const mm = (pts: number) => pts * 25.4 / 72

/** A Workspace titled like the actor's other test Workspaces (cleaned up by the fixture) with these fixture Commits. */
async function workspaceFrom(actor: Actor, fixture: string, keep: (op: Json) => boolean = () => true) {
  const ws = await actor.client.ok<{ workspace_id: string; epoch: string }>('ws.create', { title: actor.client.prefix })
  actor.client.owned.add(ws.workspace_id)
  const commits = (JSON.parse(readFileSync(join(FIXTURES, fixture), 'utf8')) as { commits: { operations: Json[] }[] }).commits
  for (const commit of commits) {
    const operations = commit.operations.filter(keep)
    if (operations.length === 0) continue
    const result = await actor.client.commit(ws, operations as Operation[])
    expect(result, JSON.stringify(result)).toMatchObject({ status: 'accepted' })
  }
  return ws
}

/** A PDF Frame `id` on `parent` whose footer shows `{page} / {pages}` and header `{title}`. */
function pdfFrame(id: string, parent: string, config: Json, box: { w: number; h: number }, band: number): Operation[] {
  return [
    { op: 'entity.create', entity_id: id, type_id: 'buckyos.container', parent_id: parent, order_key: 'zz', placement: { x: 4000, y: 4000, ...box },
      payload: { kind: 'pdf-frame', title: 'DV 页框', config: { ...config, header: band, footer: band, margin: band } } },
    { op: 'entity.create', entity_id: `${id}-h`, type_id: 'buckyos.container', parent_id: id, order_key: 'a', placement: { x: 0, y: 0, w: box.w, h: band }, payload: { kind: 'group', slot: 'header' } },
    { op: 'entity.create', entity_id: `${id}-f`, type_id: 'buckyos.container', parent_id: id, order_key: 'b', placement: { x: 0, y: box.h - band, w: box.w, h: band }, payload: { kind: 'group', slot: 'footer' } },
    { op: 'entity.create', entity_id: `${id}-t`, type_id: 'buckyos.cell', parent_id: `${id}-h`, order_key: 'a', placement: { x: band, y: 16, w: 300, h: 24 },
      payload: { view: { type: 'label' }, title: 'DV {title}', config: { font_size: 10 } } },
    { op: 'entity.create', entity_id: `${id}-n`, type_id: 'buckyos.cell', parent_id: `${id}-f`, order_key: 'a', placement: { x: box.w / 2 - 60, y: 16, w: 120, h: 24 },
      payload: { view: { type: 'label' }, title: 'P {page} / {pages}', config: { align: 'center', font_size: 10 } } },
  ] as Operation[]
}

/** A folder of its own below the actor's home folder (the picker's “My files”). */
async function homeFolder(actor: Actor, name: string) {
  let seq = 0
  const call = async (method: string, body: Json) => {
    const parsed = await (await actor.page.request.post(`/nfs/v1/${method}`, { headers: { Authorization: `Bearer ${await actor.client.session()}` }, data: body })).json() as Json
    if (!parsed.ok) throw new Error(`${method}: ${JSON.stringify(parsed.error)}`)
    return parsed.result
  }
  const session = (await call('hello', { args: {} })).session
  const path = `/home/${actor.userId}/${name}`
  await call('mkdir', { session, seq: ++seq, args: {}, at: { realm: 'dfs', path } })
  return { path, stat: async (file: string) => await call('stat', { session, args: {}, at: { realm: 'dfs', path: `${path}/${file}` } }) as Json }
}

async function openPrint(actor: Actor, phone = false) {
  if (phone) {
    // the phone's main menu is a sheet: groups open on a tap
    await actor.page.getByTestId('aiws-main-menu').tap()
    await actor.page.getByTestId('aiws-menu-surface').tap()
    await actor.page.getByTestId('aiws-menu-export-pdf').tap()
  } else await mainMenu(actor.page, 'aiws-menu-surface', 'aiws-menu-export-pdf')
  const dialog = actor.page.getByTestId('aiws-print-dialog')
  await expect(dialog).toBeVisible()
  return dialog
}

async function download(actor: Actor, dialog: ReturnType<Actor['page']['getByTestId']>, name: string) {
  const event = actor.page.waitForEvent('download', { timeout: 180_000 })
  await dialog.getByTestId('aiws-print-device').click()
  const file = await event
  expect(file.suggestedFilename()).toBe(name)
  const path = test.info().outputPath(`${Date.now()}-${name}`)
  await file.saveAs(path)
  await expect(dialog.getByTestId('aiws-print-done')).toBeVisible({ timeout: 30_000 })
  return path
}

test.describe('[print] desktop', () => {
  test('[print] a flow canvas: paginated A4 saved into the Zone by object id; A5 with a PDF Frame downloaded', async ({ owner }, info) => {
    test.setTimeout(600_000)
    // the project fixture without its uploaded picture, and enough tasks to run over several pages
    const ws = await workspaceFrom(owner, 'project-workspace/commits.json', (op) => !['diagram', 'cell-diagram'].includes(op.entity_id))
    const records = Array.from({ length: 90 }, (_, i) => ({ record_id: `dv-${i}`, values: { title: `打印行 ${String(i).padStart(3, '0')}`, status: 'option-done', budget: `${100 + i}.00`, due: '2026-12-01', owner: '林' } }))
    expect(await owner.client.commit(ws, [{ op: 'table.insert_records', source_id: 'tasks', records } as Operation])).toMatchObject({ status: 'accepted' })
    // an A-series PDF Frame on the flow page itself: it decorates the pages and is never printed as content
    const band = 56
    expect(await owner.client.commit(ws, pdfFrame('dvpf', 'surface-main', { page: 'A', orientation: 'portrait' }, { w: 595, h: 842 }, band))).toMatchObject({ status: 'accepted' })
    const folder = await homeFolder(owner, `${owner.client.prefix}-print`)
    await openWorkspace(owner, ws)

    // A4 into the Zone: the place first, then the package, the Run and the commit of the PDF's object
    const dialog = await openPrint(owner)
    await expect(dialog.getByTestId('aiws-print-paper')).toHaveValue('A4')
    await expect(dialog.getByTestId('aiws-print-frame')).toHaveValue('')
    await dialog.getByTestId('aiws-print-zone').click()
    const picker = (await openedPicker(owner.page)).dialog
    await zoneGoto(picker, [folder.path.split('/').pop()!])
    const name = await picker.getByTestId('select-name').inputValue()
    expect(name).toMatch(/\.pdf$/)
    await picker.getByTestId('select-confirm').click()
    await expect(dialog.getByTestId('aiws-print-done')).toContainText(`${folder.path}/${name}`, { timeout: 300_000 })
    await screenshot(owner.page, info, 'flow-saved')
    const saved = await folder.stat(name)
    expect(Number(saved.size ?? saved.attrs?.size ?? 0), JSON.stringify(saved)).toBeGreaterThan(5_000)
    await expect(dialog.getByTestId('aiws-print-pages')).toHaveText(/\d+/)

    // A5 with the frame, downloaded: read from NDM with the person's own session
    await dialog.getByTestId('aiws-print-paper').selectOption('A5')
    await dialog.getByTestId('aiws-print-frame').selectOption('dvpf')
    const a5 = await download(owner, dialog, name)
    const info5 = pdfInfo(a5)
    expect(Math.round(mm(info5.w))).toBe(148)
    expect(Math.round(mm(info5.h))).toBe(210)
    expect(info5.pages).toBeGreaterThan(2)
    const text = pdfText(a5)
    // every page is decorated, the page numbers count the whole document, the rows are all there and once only
    for (let p = 1; p <= info5.pages; p++) expect(text).toContain(`P${p}/${info5.pages}`)
    for (const i of [0, 45, 89]) expect(text.split(`打印行${String(i).padStart(3, '0')}`).length - 1).toBe(1)
    // the frame instance is not content: its labels appear only as the decoration (once per page)
    expect(text.split('DV项目工作区').length - 1).toBe(info5.pages)
  })

  test('[print] a free canvas: one page per step of its path, across canvases, at the stage size', async ({ owner }) => {
    test.setTimeout(600_000)
    const ws = await workspaceFrom(owner, 'presentation/commits.json')
    expect(await owner.client.commit(ws, pdfFrame('dvwide', 's-work', { page: '16:9', orientation: 'landscape' }, { w: 842, h: 474 }, 40))).toMatchObject({ status: 'accepted' })
    await openWorkspace(owner, ws)
    await owner.page.goto(`${new URL(owner.page.url()).origin}/w/launch?id=s-launch`)
    await expect(owner.page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
    const dialog = await openPrint(owner)
    await expect(dialog.getByTestId('aiws-print-paper')).toHaveValue('16:9')
    await expect(dialog.getByTestId('aiws-print-pages')).toContainText('5')
    await dialog.getByTestId('aiws-print-frame').selectOption('dvwide')
    const pdf = await download(owner, dialog, '发布会.pdf')
    const info = pdfInfo(pdf)
    expect(info.pages).toBe(5)
    expect(Math.round(mm(info.w))).toBe(297)
    expect(Math.round(mm(info.h))).toBe(167)
    const text = pdfText(pdf)
    for (let p = 1; p <= 5; p++) expect(text).toContain(`P${p}/5`)
    expect(text.split('DV发布会').length - 1).toBe(5)
    expect(text).toContain('谢谢')
  })
})

test.describe('[print] phone', () => {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars -- a describe group cannot switch the browser
  const { defaultBrowserType, ...pixel7 } = devices['Pixel 7']
  test.use(pixel7)
  test('[print] on a phone: the dialog takes the screen and the PDF goes into the Zone', async ({ owner }, info) => {
    test.setTimeout(240_000)
    const ws = await workspaceFrom(owner, 'project-workspace/commits.json', (op) => !['diagram', 'cell-diagram'].includes(op.entity_id))
    const folder = await homeFolder(owner, `${owner.client.prefix}-phone`)
    // the phone's shell has no identity badge: the workspace and its live connection
    await owner.page.goto(`${workspaceOrigin(owner, ws)}/w`)
    await expect(owner.page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', ws.workspace_id)
    await expect(owner.page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
    const dialog = await openPrint(owner, true)
    const box = (await dialog.boundingBox())!
    const viewport = owner.page.viewportSize()!
    expect(box.width).toBeGreaterThanOrEqual(viewport.width - 2)
    await dialog.getByTestId('aiws-print-zone').click()
    const picker = (await openedPicker(owner.page)).dialog
    await zoneGoto(picker, [folder.path.split('/').pop()!])
    const name = await picker.getByTestId('select-name').inputValue()
    await picker.getByTestId('select-confirm').click()
    await expect(dialog.getByTestId('aiws-print-done')).toContainText(`${folder.path}/${name}`, { timeout: 300_000 })
    await screenshot(owner.page, info, 'phone-saved')
    expect(Number((await folder.stat(name)).size ?? 0)).toBeGreaterThan(5_000)
  })
})
