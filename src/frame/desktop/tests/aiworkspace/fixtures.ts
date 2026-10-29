import { expect, test as base, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

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

  /** One of the phase-two demos (§11), replayed through the public interface like the app does. */
  async demo(token: string, kind: 'quarterly' | 'film', title: string): Promise<{ workspace_id: string; epoch: string }> {
    const { quarterlyDemoCommits, filmDemoCommits } = await import('../../src/app/aiworkspace/api/demos')
    const { setAiwsLocale } = await import('../../src/app/aiworkspace/i18n')
    setAiwsLocale(SPEC_LOCALE)
    const commits = kind === 'quarterly' ? quarterlyDemoCommits() : filmDemoCommits()
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
  await page.goto('/?scenario=normal')
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
  return restored
}

/** Open the main menu and click through items by test id (an item with a submenu opens it on click). */
export async function mainMenu(page: Page, ...testIds: string[]) {
  await page.getByTestId('aiws-main-menu').click()
  for (const id of testIds) await page.getByTestId(id).click()
}

/** Main menu → 关闭工作区 (the leave check passes when nothing is unsaved). */
export async function closeWorkspace(page: Page) {
  await mainMenu(page, 'aiws-back')
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

/** Main menu → 画布模式 → edit / view / presentation_edit. */
export async function setCanvasMode(page: Page, mode: 'edit' | 'view' | 'presentation_edit') {
  await mainMenu(page, 'aiws-menu-canvas-mode', `aiws-mode-${mode}`)
}

/** Show one tab of the right panel (main menu → 视图 → tab), unless it is already shown. */
export async function openSide(page: Page, tab: 'inspector' | 'relations' | 'annotations' | 'collab' | 'edits') {
  if (await page.locator(`[data-testid="aiws-side-panel"][data-tab="${tab}"]`).isVisible()) return
  await mainMenu(page, 'aiws-menu-view', `aiws-menu-side-${tab}`)
  await expect(page.locator(`[data-testid="aiws-side-panel"][data-tab="${tab}"]`)).toBeVisible()
}

/** The save-state panel: connection, offline actions and the edits that are not committed. */
export async function openStatus(page: Page) {
  if (await page.getByTestId('aiws-status-detail').isVisible()) return
  await page.getByTestId('aiws-status').click()
  await expect(page.getByTestId('aiws-status-detail')).toBeVisible()
}

/** Main menu → 开发工具 → 受控加工（Mock）. */
export async function openMock(page: Page) {
  await mainMenu(page, 'aiws-menu-dev', 'aiws-side-mock')
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

export const test = base.extend<{ api: Api }>({
  api: async ({}, use) => { await use(new Api()) },
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
