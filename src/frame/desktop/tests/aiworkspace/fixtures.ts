import { expect, test as base, type FrameLocator, type Locator, type Page } from '@playwright/test'
import { readdirSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { pickLocal, putZoneFile, selectDialog, selectEmbedFrame, selectRow } from '../e2e/fixtures/selectDialog'
import { queryBySql } from '../../src/app/aiworkspace/api/tableQuery'
import type { QueryPage, QueryParams } from '../../src/app/aiworkspace/api/types'
import * as wasmCore from '../../src/app/aiworkspace/wasm/aiworkspace_wasm.js'

let wasmReady = false
/** The core compiled to WASM, as the Desktop loads it (the view compiler of `queryBySql`). */
export function core(): typeof wasmCore {
  if (!wasmReady) {
    wasmCore.initSync({ module: readFileSync(resolve('src/app/aiworkspace/wasm/aiworkspace_wasm_bg.wasm')) })
    wasmReady = true
  }
  return wasmCore
}

const BACKEND_FIXTURE = resolve('../aiworkspace/fixtures/project-workspace/commits.json')
export const VECTORS_PATH = resolve('../aiworkspace/fixtures/vectors/object-ids.json')
export const BUNDLED_FIXTURE = resolve('src/app/aiworkspace/fixtures/project-workspace.commits.json')
export { BACKEND_FIXTURE }

/** The specs match the Chinese UI, while the Desktop starts in English: every browser context of a test starts in
 * zh-CN (the I18nProvider's stored language), on the dev server and on the production preview alike, and the demos
 * built here are Chinese. i18n.spec.ts covers the English UI. */
export const LOCALE_STORAGE_KEY = 'buckyos.prototype.locale.v1'
export const SPEC_LOCALE = 'zh-CN'

// The backend's JSON results are inspected ad hoc in assertions.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

/** Direct kRPC access to the backend under test — what the assertions "through the API" use. Uploads and downloads go
 * through the standalone nfs-server's `/bucket/v1` data plane, as the Desktop's NDM uploads and bucket URLs do. */
export class Api {
  private seq = 1
  readonly base: string
  readonly nfs: string
  constructor() {
    const backend = process.env.AIWS_E2E_BACKEND
    if (!backend) throw new Error('AIWS_E2E_BACKEND is not set: run with --config=playwright.aiworkspace.config.ts')
    this.base = `${backend}/kapi/aiworkspace`
    this.nfs = `http://127.0.0.1:${process.env.AIWS_E2E_NDM_PORT}`
  }

  /** Bytes bound into the token's user's own bucket `name` (what a browser upload with `bind` leaves). */
  async bind(token: string, name: 'assets' | 'exports', key: string, bytes: Uint8Array | ArrayBuffer): Promise<{ bucket_id: string; key: string }> {
    const bucket_id = `user/aiworkspace/${token.replace(/^tok-/, '')}/${name}`
    const put = await fetch(`${this.nfs}/bucket/v1/${bucket_id}/${key}`, { method: 'PUT', body: bytes as BodyInit })
    expect(put.ok, await put.clone().text()).toBe(true)
    return { bucket_id, key }
  }

  /** An asset upload: bound by the uploader, taken over by the service (`asset.import_object`). */
  async uploadAsset(token: string, workspaceId: string, bytes: Uint8Array): Promise<Json> {
    const bound = await this.bind(token, 'assets', `upload/${Math.random().toString(36).slice(2)}`, bytes)
    return this.rpc(token, 'asset.import_object', { workspace_id: workspaceId, ...bound })
  }

  /** A download a result names (`doc.export` / `replica.bootstrap` → `download`), read with its token. */
  async download(download: Json): Promise<ArrayBuffer> {
    const response = await fetch(`${this.nfs}/bucket/v1/${download.bucket_id}/${download.key}?token=${encodeURIComponent(download.token)}`)
    expect(response.ok).toBe(true)
    return response.arrayBuffer()
  }

  /** `ws.import` of a package the token's user bound into its own `exports` bucket. */
  async importPackage(token: string, pkg: ArrayBuffer, semantics: 'new' | 'restore', replace = false): Promise<Json> {
    const bound = await this.bind(token, 'exports', `import/${Math.random().toString(36).slice(2)}.bcanvas`, pkg)
    return this.rpc(token, 'ws.import', { ...bound, semantics, ...(replace ? { replace: true } : {}) })
  }

  async rpc(token: string, method: string, params: Json): Promise<Json> {
    const response = await fetch(this.base, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ method, params, sys: [this.seq++, token] }) })
    const body = await response.json() as Json
    if (body.error) throw new Error(`${method}: ${body.error}`)
    return body.result
  }

  /** The rows of a table or a table view (`source_id` / `view_id`, filter, sorts, …) as a session without a replica reads
   * them: the view compiled to SQL by the core and run with `doc.query { sql }` (核心 API 调整 §3.5). */
  async table(token: string, workspaceId: string, params: QueryParams): Promise<QueryPage> {
    const fail = (e: { code: string; detail?: string }) => Object.assign(new Error(`${e.code}: ${e.detail ?? ''}`), e)
    const ok = async (method: string, p: Json) => {
      const r = await this.rpc(token, method, { workspace_id: workspaceId, ...p })
      if (r?.ok === false) throw fail(r.error)
      return r
    }
    return queryBySql({
      read: async (ids) => (await ok('doc.read', { targets: ids.map((entity_id) => ({ entity_id })) })).results,
      sql: (p) => ok('doc.query', p),
      sourceQuery: (p) => ok('doc.source_query', p),
      viewSql: (table, fields, view, projection) => core().view_sql(table, fields, view, projection),
      fail,
    }, params)
  }

  /** The readable children of a node (`doc.list`). */
  async children(token: string, workspaceId: string, at: string): Promise<Json[]> {
    return (await this.rpc(token, 'doc.list', { workspace_id: workspaceId, at, limit: 1000 })).items
  }

  async commit(token: string, ws: { workspace_id: string; epoch: string }, operations: Json[], key = `test/${Date.now()}/${Math.random().toString(36).slice(2)}`): Promise<Json> {
    return this.rpc(token, 'doc.commit', { protocol_version: '1.0', workspace_id: ws.workspace_id, epoch: ws.epoch, idempotency_key: key, session_id: 'test-api', operations })
  }

  /** The shared sample of design §3.8, replayed through the public interface (same as the Rust tests). */
  async sample(token: string, title: string): Promise<{ workspace_id: string; epoch: string }> {
    const ws = await this.rpc(token, 'ws.create', { title })
    const fixture = JSON.parse(readFileSync(BACKEND_FIXTURE, 'utf8')) as Json
    let text = JSON.stringify(fixture.commits)
    for (const asset of fixture.assets) {
      const done = await this.uploadAsset(token, ws.workspace_id, Buffer.from(asset.base64, 'base64'))
      text = text.split(asset.placeholder).join(done.object_id)
    }
    const commits = JSON.parse(text) as Json[]
    for (const [index, commit] of commits.entries()) {
      const result = await this.commit(token, ws, commit.operations, `fixture/${index + 1}`)
      expect(result.status, JSON.stringify(result)).toBe('accepted')
    }
    return { workspace_id: ws.workspace_id, epoch: ws.epoch }
  }

  /** One of the phase-two demos (§11), replayed through the public interface like the app does — with its wishes on
   * the Mock executor (its definition added after the data): the test service has no model. */
  async demo(token: string, kind: 'quarterly' | 'film', title: string): Promise<{ workspace_id: string; epoch: string }> {
    const { quarterlyDemoCommits, filmDemoCommits } = await import('../../src/app/aiworkspace/api/demos')
    const { mockWishDefOp } = await import('../../src/app/aiworkspace/ui/wish/mockWishDef')
    const { setAiwsLocale } = await import('../../src/app/aiworkspace/i18n')
    setAiwsLocale(SPEC_LOCALE)
    const shipped = kind === 'quarterly' ? quarterlyDemoCommits() : filmDemoCommits()
    const mocked = shipped.map((c) => ({ ...c, operations: c.operations.map((op) => (op.op === 'entity.create' && op.type_id === 'buckyos.wish' ? { ...op, payload: { ...(op.payload as Record<string, Json>), executor: 'mock' } } : op)) }))
    const commits = [mocked[0], { message: 'Mock wish executor', operations: [mockWishDefOp('zzz') as Json] }, ...mocked.slice(1)]
    const ws = await this.rpc(token, 'ws.create', { title })
    for (const [index, commit] of commits.entries()) {
      const result = await this.commit(token, ws, commit.operations as Json[], `demo/${index + 1}`)
      expect(result.status, JSON.stringify(result)).toBe('accepted')
    }
    return { workspace_id: ws.workspace_id, epoch: ws.epoch }
  }

  async outline(token: string, workspaceId: string): Promise<Json[]> {
    return (await this.rpc(token, 'doc.outline', { workspace_id: workspaceId })).entities
  }

  async read(token: string, workspaceId: string, entityId: string): Promise<Json> {
    return this.rpc(token, 'doc.read', { workspace_id: workspaceId, entity_id: entityId })
  }

  async cell(token: string, workspaceId: string, source: string, record: string, field: string): Promise<Json> {
    const read = await this.rpc(token, 'doc.read', { workspace_id: workspaceId, entity_id: source, selector: { kind: 'table_cell', record_id: record, field_id: field } })
    return read.content
  }

  async ast(token: string, workspaceId: string, entity: string): Promise<Json> {
    return (await this.rpc(token, 'doc.read', { workspace_id: workspaceId, entity_id: entity })).content.content
  }

  async headSeq(token: string, workspaceId: string): Promise<number> {
    return (await this.rpc(token, 'ws.get_info', { workspace_id: workspaceId })).head_seq
  }

  /** `ext.build` of a control from files given as text or JSON (or a folder of `doc/workspace/hello-controls`, or a path under `doc/workspace`). */
  async build(token: string, workspaceId: string, files: Record<string, unknown> | string, extra: Json = {}): Promise<Json> {
    // a name: a folder of hello-controls; a path: a folder under doc/workspace (control-examples/controls/<name>)
    const dir = typeof files === 'string' ? resolve(files.includes('/') ? '../../../doc/workspace' : '../../../doc/workspace/hello-controls/controls', files) : ''
    const list = typeof files === 'string'
      ? readdirSync(dir).map((name) => ({ name, base64: readFileSync(resolve(dir, name)).toString('base64') }))
      : Object.entries(files).map(([name, v]) => ({ name, base64: Buffer.from(typeof v === 'string' ? v : JSON.stringify(v), 'utf8').toString('base64') }))
    const built = await this.rpc(token, 'ext.build', { workspace_id: workspaceId, files: list, ...extra })
    expect(built.status, JSON.stringify(built)).toBe('accepted')
    return built
  }

  /** A workspace with one free canvas `free` (its content folder `free-content`). */
  async freeCanvas(token: string, title: string): Promise<{ workspace_id: string; epoch: string }> {
    const ws = await this.rpc(token, 'ws.create', { title }) as { workspace_id: string; epoch: string }
    const made = await this.commit(token, ws, [
      { op: 'entity.create', entity_id: 'free-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a', payload: { kind: 'folder', system: 'surface_content', surface_id: 'free', title: 'Free' } },
      { op: 'entity.create', entity_id: 'free', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a', payload: { kind: 'surface', layout: { mode: 'free' }, title: 'Free', content_folder_id: 'free-content' } },
    ])
    expect(made.status, JSON.stringify(made)).toBe('accepted')
    return ws
  }

  /** The runs of a wish, newest first (`proc.list`). */
  async runs(token: string, workspaceId: string, wishId: string): Promise<Record<string, Json>[]> {
    return (await this.rpc(token, 'proc.list', { workspace_id: workspaceId, wish_id: wishId, limit: 5 })).runs as Record<string, Json>[]
  }
}

/** A workspace's own origin on the dev server (T4-8): `http://aiws-<label>.localhost:<port>`. */
export function workspaceOrigin(workspaceId: string, port: string | number = process.env.AIWS_E2E_WEB_PORT ?? ''): string {
  return `http://aiws-${workspaceId.replace(/^ws_/, '')}.localhost:${port}`
}

/** A workspace page: the service ignores the first segment and, with `id`, the second (links.ts). */
export function workspacePage(workspaceId: string, surfaceId?: string | null, port?: string | number): string {
  return surfaceId ? `${workspaceOrigin(workspaceId, port)}/w/s?id=${encodeURIComponent(surfaceId)}` : `${workspaceOrigin(workspaceId, port)}/w`
}

/** Open a workspace page on its own origin as `token` (`{ guest: true }` for nobody), in the specs' language, and
 * wait for the workspace. */
export async function openWorkspacePage(page: Page, who: string | { guest: true }, workspaceId: string, surfaceId?: string | null, port?: string | number) {
  await asUser(page, who)
  await page.goto(workspacePage(workspaceId, surfaceId, port))
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', workspaceId, { timeout: 30_000 })
}

/** Open the Desktop shell and the AI Workspace app in it, talking to the standalone backend as `token`, and
 * end on the workspace list (a normal start restores the last workspace: it is closed again here). */
export async function openApp(page: Page, token: string) {
  // keeps what an earlier init script put there (the local executor spec adds its model endpoint)
  await page.addInitScript((value) => { let prev: Record<string, unknown> = {}; try { prev = JSON.parse(window.localStorage.getItem('aiworkspace.dev') ?? '{}') as Record<string, unknown> } catch { prev = {} }; window.localStorage.setItem('aiworkspace.dev', JSON.stringify({ ...prev, token: value })) }, token)
  // the mock Desktop loads its scenario state after the first paint and resets the windows then: an icon clicked
  // before that is lost, so the click waits for the loading to settle
  await page.goto('/?scenario=normal', { waitUntil: 'networkidle' })
  await page.getByTestId('desktop-app-aiworkspace').click()
  // the window opens maximised: the canvas has the room the specs' coordinates assume
  await page.getByTestId('window-aiworkspace').getByRole('button', { name: /^(Maximize|最大化)$/ }).click()
  await backToList(page)
}

/** After the app started: a restored workspace (UI improvement §4) is closed so the list shows; returns its id. */
export async function backToList(page: Page): Promise<string | null> {
  await expect(page.getByTestId('aiws-list').or(page.getByTestId('aiws-workspace'))).toBeVisible({ timeout: 30_000 })
  let restored: string | null = null
  if (await page.getByTestId('aiws-workspace').isVisible()) {
    restored = await page.getByTestId('aiws-workspace').getAttribute('data-workspace-id')
    await closeWorkspace(page)
  }
  await expect(page.getByTestId('aiws-list')).toBeVisible({ timeout: 30_000 })
  await showAllWorkspaces(page)
  return restored
}

/** The home starts on "Recent" when it has any (UI framework PRD §9.1): the specs find their workspaces under "All
 * workspaces", which the window then keeps while it lives. Nothing to pick when the list cannot be read (offline). */
export async function showAllWorkspaces(page: Page) {
  const list = page.getByTestId('aiws-list')
  await expect(list).not.toHaveAttribute('data-state', 'loading', { timeout: 30_000 })
  if (await list.getAttribute('data-state') === 'ready') await list.getByTestId('aiws-home-tab-all').click()
}

/** A home card's ⋯ menu → an item. The menu floats over the list and a scroll closes it: it is opened once the card
 * has been brought into view. */
export async function cardMenu(page: Page, workspaceId: string, item: string) {
  const card = page.locator(`[data-testid="aiws-workspace-card"][data-workspace-id="${workspaceId}"]`)
  const more = card.getByTestId('aiws-home-card-more')
  const entry = card.getByTestId(item)
  await more.scrollIntoViewIfNeeded()
  await expect(async () => {
    if (!await entry.isVisible()) await more.click()
    await expect(entry).toBeVisible({ timeout: 2000 })
  }).toPass({ timeout: 20_000 })
  await entry.click()
}

/** Open the main menu and click through items by test id (an item with a submenu opens it on click). */
export async function mainMenu(page: Page, ...testIds: string[]) {
  await page.getByTestId('aiws-main-menu').click()
  for (const id of testIds) {
    const item = page.getByTestId(id)
    if (await item.getAttribute('aria-haspopup') === 'menu') await item.hover()
    else await item.click()
  }
}

/** Main menu → 工作区 → 关闭工作区 (the leave check passes when nothing is unsaved). */
export async function closeWorkspace(page: Page) {
  await mainMenu(page, 'aiws-menu-workspace', 'aiws-back')
}

/** The zoom control of the presenter toolbar → 适应全部. */
export async function fitAll(page: Page) {
  await page.getByTestId('aiws-zoom-menu').click()
  await page.getByTestId('aiws-fit-all').click()
}

export async function zoomIn(page: Page) {
  await page.getByTestId('aiws-zoom-menu').click()
  await page.getByTestId('aiws-zoom-in').click()
  await page.keyboard.press('Escape')
}

/** Main menu → 画布 → 画布模式 → edit / view / presentation_edit. */
export async function setCanvasMode(page: Page, mode: 'edit' | 'view' | 'presentation_edit') {
  await mainMenu(page, 'aiws-menu-surface', 'aiws-menu-canvas-mode', `aiws-mode-${mode}`)
}

/** Show one tab of the side drawer (main menu → 视图 → 面板 → tab), unless it is already shown. */
export async function openSide(page: Page, tab: 'inspector' | 'relations' | 'annotations') {
  // properties are the floating properties panel (UI 框架 PRD §10.2), not a tab of the side drawer
  if (tab === 'inspector') return openProperties(page)
  if (await page.locator(`[data-testid="aiws-side-panel"][data-tab="${tab}"]`).isVisible()) return
  await mainMenu(page, 'aiws-menu-view', 'aiws-menu-panels', `aiws-menu-side-${tab}`)
  await expect(page.locator(`[data-testid="aiws-side-panel"][data-tab="${tab}"]`)).toBeVisible()
}

/** The properties panel of the selection (F2; the page's when nothing is selected), UI 框架 PRD §10.2. */
export async function openProperties(page: Page) {
  if (await page.getByTestId('aiws-props-panel').isVisible()) return
  await page.keyboard.press('F2')
  await expect(page.getByTestId('aiws-props-panel')).toBeVisible()
}

/** The status panel (UI framework PRD §4.5), opened from the status bar, with its "connection and offline" part
 * unfolded: the edits that are not committed, connection details and the offline actions. */
export async function openStatus(page: Page) {
  if (!await page.getByTestId('aiws-status-detail').isVisible()) await page.getByTestId('aiws-status').click()
  await expect(page.getByTestId('aiws-status-detail')).toBeVisible()
  const connection = page.getByTestId('aiws-status-connection')
  if (!await connection.evaluate((el) => (el as HTMLDetailsElement).open)) await page.getByTestId('aiws-status-connection-toggle').click()
}

/** Main menu → 协作 → 成员与权限… (UI 框架 PRD §8.1): the data mode with the Workspace's permission editor as its tab. */
export async function openPermissions(page: Page) {
  await mainMenu(page, 'aiws-menu-collab', 'aiws-menu-members')
  await expect(page.getByTestId('aiws-detail-permissions')).toBeVisible()
}

/** Back from the data mode (the permission editor's tab included) to the canvas. */
export async function closePermissions(page: Page) {
  await page.getByTestId('aiws-top-canvas').click()
  await expect(page.getByTestId('aiws-canvas')).toBeVisible()
}

/** Unfold rows of the data mode's object tree, outermost first (a row already unfolded stays so). */
export async function expandTree(page: Page, ...entityIds: string[]) {
  for (const id of entityIds) {
    const item = page.locator(`[data-testid="aiws-tree-item"][data-entity-id="${id}"]`)
    if (await item.getAttribute('aria-expanded') === 'false') await item.locator('.aiws-tree-toggle').first().click()
    await expect(item).toHaveAttribute('aria-expanded', 'true')
  }
}

/** The data mode's object tree: the context menu of a row (a right click), then entries by test id (a submenu opens on
 * hover). `entityId` null: the tree's ＋ (where the current object is). */
export async function treeMenu(page: Page, entityId: string | null, ...testIds: string[]) {
  if (entityId) await page.getByTestId(`aiws-tree-${entityId}`).click({ button: 'right' })
  else await page.getByTestId('aiws-dm-tree-add').click()
  await expect(page.getByTestId('aiws-dm-menu')).toBeVisible()
  for (const id of testIds) {
    const item = page.getByTestId(id)
    if (await item.getAttribute('aria-haspopup') === 'menu') await item.hover()
    else await item.click()
  }
}

/** Main menu → 帮助 → 开发工具 → 受控加工（Mock）. */
export async function openMock(page: Page) {
  await mainMenu(page, 'aiws-menu-help', 'aiws-menu-dev', 'aiws-side-mock')
}

export async function openWorkspace(page: Page, token: string, workspaceId: string) {
  await openApp(page, token)
  await page.locator(`[data-testid="aiws-workspace-card"][data-workspace-id="${workspaceId}"]`).getByTestId('aiws-open').click()
  await expect(page.getByTestId('aiws-workspace')).toBeVisible({ timeout: 30_000 })
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
}

/** No edit is unsaved or waiting for a decision. */
export async function expectAllCommitted(page: Page) {
  await expect(page.getByTestId('aiws-save-summary')).toHaveAttribute('data-unsaved', '0')
  await expect(page.getByTestId('aiws-save-summary')).toHaveAttribute('data-attention', '0')
}

/** Open a workspace and make sure the canvas (the default top-level view) is shown. */
export async function openCanvas(page: Page, token: string, workspaceId: string) {
  await openWorkspace(page, token, workspaceId)
  if (await page.getByTestId('aiws-top-canvas').isVisible()) await page.getByTestId('aiws-top-canvas').click()
  await expect(page.getByTestId('aiws-canvas-view')).toBeVisible()
}

/** Screen centre of a canvas Block frame. */
export async function blockCenter(page: Page, blockId: string): Promise<{ x: number; y: number }> {
  const box = await page.getByTestId(`aiws-canvas-block-${blockId}`).boundingBox()
  if (!box) throw new Error(`block ${blockId} is not on screen`)
  return { x: box.x + box.width / 2, y: box.y + box.height / 2 }
}

/** The window-level counters the app exposes under the dev override. */
export async function hooks(page: Page): Promise<{ commits: number; canvas: { blocks: number; mounted: number; hidden: number; placeholders: number; editors: number; html: number; zoom: number; mode: string } | undefined }> {
  return page.evaluate(() => {
    const h = (window as unknown as { __aiwsTestHooks?: { commits?: number; canvas?: unknown } }).__aiwsTestHooks
    return { commits: h?.commits ?? 0, canvas: h?.canvas as never }
  })
}

/** Talk to the backend as `who` (`{ guest: true }` for nobody) in the specs' language, on every origin this page visits
 * (a workspace's own origin starts with an empty storage). */
export async function asUser(page: Page, who: string | { guest: true }) {
  await page.addInitScript(({ who, localeKey, locale }) => {
    window.localStorage.setItem('aiworkspace.dev', JSON.stringify(typeof who === 'string' ? { token: who } : who))
    window.localStorage.setItem(localeKey, locale)
  }, { who, localeKey: LOCALE_STORAGE_KEY, locale: SPEC_LOCALE })
}

/** The test accounts' preferences across Workspaces (UI 框架 PRD §4.6, §12: the tool zones' switches, the status bar,
 * the home's category and filters) follow a person from test to test on the shared backend: every test starts without. */
async function resetPrefs(api: Api) {
  for (const token of ['tok-alice', 'tok-bob']) await api.rpc(token, 'user.set_prefs', { prefs: {} }).catch(() => undefined)
}

export const test = base.extend<{ api: Api; freshPrefs: void }>({
  api: async ({}, use) => { await use(new Api()) },
  freshPrefs: [async ({}, use) => {
    try { await resetPrefs(new Api()) } catch { /* no backend in this run */ }
    await use()
  }, { auto: true }],
  // the specs' language on every origin, the workspace origins `aiws-<label>.localhost` included
  context: async ({ context }, use) => {
    await context.addInitScript(({ localeKey, locale }) => { if (!window.localStorage.getItem(localeKey)) window.localStorage.setItem(localeKey, locale) }, { localeKey: LOCALE_STORAGE_KEY, locale: SPEC_LOCALE })
    await use(context)
  },
  storageState: async ({}, use) => {
    const ports = [process.env.AIWS_E2E_WEB_PORT, process.env.AIWS_E2E_PREVIEW_PORT].filter(Boolean)
    await use({ cookies: [], origins: ports.map((port) => ({ origin: `http://localhost:${port}`, localStorage: [{ name: LOCALE_STORAGE_KEY, value: SPEC_LOCALE }] })) })
  },
})
export { expect }

// ---- the system file picker (sysdlg/select): every file the Workspace takes in is chosen there

/** A file the specs choose, from the Zone or from this device. */
export type PickedFile = { name: string; mimeType: string; buffer: Buffer }
/** Where the specs put the files they choose from the Zone (the mock Desktop runtime's file data). */
export const ZONE_DIR = '/home/Documents/aiws-e2e'

const asPayload = (file: PickedFile | string): PickedFile => typeof file === 'string'
  ? { name: file.slice(file.lastIndexOf('/') + 1), mimeType: 'application/octet-stream', buffer: readFileSync(file) } : file

/** The picker over this page: in the Desktop window (path A), or framed over a Workspace's own page (path C). */
export async function openedPicker(page: Page): Promise<{ dialog: Locator; frame: FrameLocator | null }> {
  const inWindow = selectDialog(page)
  const framed = selectDialog(selectEmbedFrame(page))
  // (a frame locator cannot be part of `or`)
  await expect.poll(async () => await inWindow.isVisible() || await framed.isVisible(), { timeout: 30_000 }).toBe(true)
  return await inWindow.isVisible() ? { dialog: inWindow, frame: null } : { dialog: framed, frame: selectEmbedFrame(page) }
}

const HOME_LABEL = /^(我的文件|My files)$/

/** From the picker's home root ("我的文件") down through `segments`, the folders below the home folder. */
export async function zoneGoto(dialog: Locator, segments: string[]) {
  const locations = dialog.getByTestId('select-locations')
  if (await locations.evaluate((el) => el.tagName === 'SELECT')) {
    const label = (await locations.locator('option').allTextContents()).find((text) => HOME_LABEL.test(text.trim()))
    await locations.selectOption({ label: label?.trim() ?? '我的文件' })
  } else await locations.getByRole('button', { name: HOME_LABEL }).click()
  for (const segment of segments) await selectRow(dialog, segment).click()
}
const belowHome = (dir: string) => dir.split('/').filter(Boolean).slice(1)

/** Choose files from the Zone in the picker `open` brings up. They are put into the mock Zone first — of this page,
 * which reads them, and of the framed system page, which lists them (path C) — then picked in their folder. */
export async function chooseFromZone(page: Page, open: () => Promise<unknown>, files: PickedFile | string | Array<PickedFile | string>, dir = ZONE_DIR) {
  const list = (Array.isArray(files) ? files : [files]).map(asPayload)
  for (const file of list) await putZoneFile(page, `${dir}/${file.name}`, file.buffer, file.mimeType)
  await open()
  const { dialog } = await openedPicker(page)
  const framed = page.frames().find((frame) => /\/embed\/sysdlg\/select/.test(frame.url()))
  if (framed) for (const file of list) await putZoneFile(framed, `${dir}/${file.name}`, file.buffer, file.mimeType)
  await zoneGoto(dialog, belowHome(dir))
  for (const file of list) await selectRow(dialog, file.name).click()
  await dialog.getByTestId('select-confirm').click()
}

/** Choose a whole Zone folder (its files put into the mock Zone first) in the folder picker `open` brings up. */
export async function chooseFolderFromZone(page: Page, open: () => Promise<unknown>, dir: string, files: Array<{ path: string; buffer: Buffer }>) {
  for (const file of files) await putZoneFile(page, `${dir}/${file.path}`, file.buffer)
  await open()
  const { dialog } = await openedPicker(page)
  const framed = page.frames().find((frame) => /\/embed\/sysdlg\/select/.test(frame.url()))
  if (framed) for (const file of files) await putZoneFile(framed, `${dir}/${file.path}`, file.buffer)
  await zoneGoto(dialog, belowHome(dir))
  await dialog.getByTestId('select-confirm').click()
}

/** Choose files (or a folder) from this device: the picker's "This device" entry, then the browser's own picker. */
export async function chooseFromDevice(page: Page, open: () => Promise<unknown>, files: string | string[] | PickedFile | PickedFile[]) {
  await open()
  const { dialog } = await openedPicker(page)
  await pickLocal(page, files, dialog)
}
