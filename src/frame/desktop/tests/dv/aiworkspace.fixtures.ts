import { createHash, randomBytes, randomUUID } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { expect, test as base, type BrowserContext, type Page, type TestInfo } from '@playwright/test'
import { unwrap } from '../../src/app/aiworkspace/api/client'
import { PROTOCOL_VERSION, type CommitResult, type Operation, type Result, type RichTextContent, type WorkspaceInfo, type WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { expectAllCommitted } from '../aiworkspace/fixtures'

type Params = Record<string, unknown>
type Workspace = Pick<WorkspaceInfo, 'workspace_id' | 'epoch'>
type Cell = { value: unknown; rev: number; is_set: boolean }
export type Actor = { page: Page; client: ZoneClient; userId: string }

export class ZoneClient {
  readonly owned = new Set<string>()
  readonly prefix = `dv-aiws-${randomUUID()}`
  private seq = 0

  constructor(readonly page: Page, private readonly token: string) {}

  async service<T>(service: string, method: string, params: Params, token: string | null = this.token): Promise<T> {
    const reply = await this.page.evaluate(async (input) => {
      const response = await fetch(`/kapi/${input.service}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ method: input.method, params: input.params, sys: input.token === null ? [input.seq] : [input.seq, input.token] }),
      })
      return { status: response.status, body: await response.json() as { result?: unknown; error?: string } }
    }, { service, method, params, token, seq: ++this.seq })
    if (reply.status !== 200) throw new Error(`${service}.${method}: HTTP ${reply.status}`)
    if (reply.body.error) throw new Error(`${service}.${method}: ${reply.body.error}`)
    if (!('result' in reply.body)) throw new Error(`${service}.${method}: missing result`)
    return reply.body.result as T
  }

  call<T>(method: string, params: Params, token?: string | null) {
    return this.service<T>('aiworkspace', method, params, token)
  }

  async ok<T>(method: string, params: Params): Promise<T> {
    return unwrap(await this.call<Result<T>>(method, params))
  }

  info(ws: Workspace) { return this.ok<WorkspaceInfo>('ws.get_info', { workspace_id: ws.workspace_id }) }
  list() { return this.ok<{ workspaces: WorkspaceSummary[] }>('ws.list', {}) }

  async cell(ws: Workspace, record: string, field: string): Promise<Cell> {
    return (await this.ok<{ content: Cell }>('doc.read', {
      workspace_id: ws.workspace_id, entity_id: 'tasks',
      selector: { kind: 'table_cell', record_id: record, field_id: field },
    })).content
  }

  async ast(ws: Workspace) {
    return (await this.ok<{ content: RichTextContent }>('doc.read', { workspace_id: ws.workspace_id, entity_id: 'notes' })).content.content
  }

  commit(ws: Workspace, operations: Operation[], key = randomUUID()) {
    return this.call<CommitResult>('doc.commit', {
      workspace_id: ws.workspace_id, epoch: ws.epoch, protocol_version: PROTOCOL_VERSION, session_id: 'dv-aiws',
      idempotency_key: key, operations,
    })
  }
}

function password() {
  const path = process.env.BUCKYOS_TEST_ADMIN_PASSWORD_FILE
  const value = path ? readFileSync(path, 'utf8').trimEnd() : process.env.BUCKYOS_TEST_ADMIN_PASSWORD
  if (!value) throw new Error('Set BUCKYOS_TEST_ADMIN_PASSWORD_FILE or BUCKYOS_TEST_ADMIN_PASSWORD')
  return value
}

function hashPassword(user: string, value: string, nonce?: number) {
  const stored = createHash('sha256').update(`${value}${user}.buckyos`).digest('base64')
  return nonce === undefined ? stored : createHash('sha256').update(`${stored}${nonce}`).digest('base64')
}

async function login(page: Page, userId: string, value: string): Promise<Actor> {
  await page.goto('/')
  await page.getByLabel('Username', { exact: true }).fill(userId)
  await page.getByLabel('Password', { exact: true }).fill(value)
  await page.getByRole('button', { name: 'Sign In', exact: true }).click()
  await expect(page.getByTestId('desktop-app-aiworkspace')).toBeVisible({ timeout: 30_000 })
  const cookie = (await page.context().cookies()).find((entry) => entry.name === 'buckyos_session_token')
  if (!cookie) throw new Error('Real login returned no BuckyOS session cookie')
  const claims = JSON.parse(Buffer.from(cookie.value.split('.')[1], 'base64url').toString('utf8')) as { sub: string }
  expect(claims.sub).toBe(userId)
  expect(await page.evaluate(() => localStorage.getItem('aiworkspace.dev'))).toBeNull()
  const client = new ZoneClient(page, cookie.value)
  await client.list()
  return { page, client, userId }
}

function observe(page: Page, errors: string[]) {
  page.on('pageerror', (error) => errors.push(error.message))
  page.on('console', (message) => {
    if (message.type() !== 'error') return
    if (message.location().url.endsWith('/sso_refresh') && message.text().includes('401')) return
    errors.push(message.text())
  })
}

async function screenshot(page: Page, info: TestInfo, name: string) {
  const path = info.outputPath(`${name}.png`)
  await page.screenshot({ path, fullPage: true })
  await info.attach(name, { path, contentType: 'image/png' })
}

export async function openWorkspace(actor: Actor, ws: Workspace) {
  await actor.page.goto(`/workspace/${ws.workspace_id}`)
  await expect(actor.page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', ws.workspace_id)
  await expect(actor.page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
  await expect(actor.page.getByTestId('aiws-principal')).toHaveAttribute('data-principal', actor.userId)
}

export async function createTemplate(actor: Actor, template: 'sample' | 'quarterly', desktop = false): Promise<Workspace> {
  const page = actor.page
  if (desktop) {
    await page.getByTestId('desktop-app-aiworkspace').click()
    await page.getByTestId('window-aiworkspace').getByRole('button', { name: 'Maximize', exact: true }).click()
  } else await page.goto('/workspace')
  await expect(page.getByTestId('aiws-transport')).toHaveText('Zone 服务', { timeout: 30_000 })
  await page.getByTestId('aiws-new-template').click()
  await page.getByTestId(`aiws-template-${template}`).click()
  await page.getByLabel('工作区标题', { exact: true }).fill(actor.client.prefix)
  const [created] = await Promise.all([page.waitForResponse((response) => {
    if (new URL(response.url()).pathname.replace(/\/+$/, '') !== '/kapi/aiworkspace' || response.request().method() !== 'POST') return false
    return response.request().postDataJSON()?.method === 'ws.create'
  }, { timeout: 30_000 }), page.getByTestId('aiws-create-template').click()])
  const body = await created.json() as { result: Result<WorkspaceSummary> }
  const ws = unwrap(body.result)
  actor.client.owned.add(ws.workspace_id)
  await expect(page.getByTestId('aiws-workspace')).toHaveAttribute('data-workspace-id', ws.workspace_id, { timeout: 30_000 })
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
  await expectAllCommitted(page)
  expect((await actor.client.info(ws)).title).toBe(actor.client.prefix)
  return { workspace_id: ws.workspace_id, epoch: ws.epoch }
}

export function createSample(actor: Actor, desktop = false) {
  return createTemplate(actor, 'sample', desktop)
}

export const test = base.extend<{ owner: Actor; peer: Actor }>({
  owner: async ({ page }, use, info) => {
    const errors: string[] = []
    observe(page, errors)
    const actor = await login(page, process.env.BUCKYOS_TEST_ADMIN_USER || 'devtest', password())
    try {
      await use(actor)
    } finally {
      if (!page.isClosed()) await screenshot(page, info, 'owner-final').catch((error: unknown) => errors.push(String(error)))
      const active = (await actor.client.list()).workspaces
      const ids = [...new Set([...actor.client.owned, ...active.filter((ws) => ws.title === actor.client.prefix).map((ws) => ws.workspace_id)])]
      const failures: string[] = []
      for (const workspace_id of ids) {
        try {
          const current = await actor.client.ok<WorkspaceInfo>('ws.get_info', { workspace_id })
          expect(current.title).toBe(actor.client.prefix)
          await actor.client.ok('ws.delete', { workspace_id })
        } catch (error) {
          failures.push(String(error))
        }
      }
      const remaining = (await actor.client.list()).workspaces.filter((ws) => ids.includes(ws.workspace_id))
      await info.attach('workspace-cleanup', {
        body: Buffer.from(JSON.stringify({ created: ids, remaining: remaining.map((ws) => ws.workspace_id), failures })),
        contentType: 'application/json',
      })
      await info.attach('browser-errors', { body: Buffer.from(JSON.stringify(errors)), contentType: 'application/json' })
      expect(failures, 'workspace cleanup failures').toEqual([])
      expect(remaining, 'test workspaces left in the active list').toEqual([])
      expect(errors, 'unexpected owner browser errors').toEqual([])
    }
  },
  peer: async ({ owner, browser, baseURL }, use, info) => {
    const userId = `dvaiws${randomBytes(8).toString('hex')}`
    const value = `Dv-${randomBytes(18).toString('hex')}!`
    const sudo = async () => {
      const nonce = Date.now()
      const result = await owner.client.service<{ session_token: string }>('verify-hub', 'sudo_by_password', {
        username: owner.userId, password: hashPassword(owner.userId, password(), nonce),
        target: { kind: 'system', service_id: 'control-panel' }, aud: 'system-config', login_nonce: nonce,
      })
      if (!result.session_token) throw new Error('sudo_by_password returned no session')
      return result.session_token
    }
    let created = false
    let context: BrowserContext | null = null
    const errors: string[] = []
    try {
      const result = await owner.client.service<{ ok: boolean; created: boolean }>('control-panel', 'user.create', {
        user_id: userId, show_name: `AI Workspace DV ${userId}`, password_hash: hashPassword(userId, value),
      }, await sudo())
      created = result.created
      expect(result).toMatchObject({ ok: true, created: true })
      context = await browser.newContext({ baseURL, ignoreHTTPSErrors: true, viewport: { width: 1600, height: 1000 } })
      const page = await context.newPage()
      observe(page, errors)
      const actor = await login(page, userId, value)
      try { await use(actor) } finally { await screenshot(page, info, 'peer-final').catch((error: unknown) => errors.push(String(error))) }
    } finally {
      await context?.close().catch((error: unknown) => errors.push(String(error)))
      if (created) {
        const deleted = await owner.client.service('control-panel', 'user.delete', { user_id: userId }, await sudo())
        expect(deleted).toMatchObject({ ok: true, user_id: userId })
        const detail = await owner.client.service('control-panel', 'user.get', { user_id: userId }, await sudo())
        expect(detail).toMatchObject({ user_id: userId, state: 'deleted' })
      }
      await info.attach('peer-cleanup', { body: Buffer.from(JSON.stringify({ user_id: userId, deleted: created, errors })), contentType: 'application/json' })
      expect(errors, 'unexpected peer browser errors').toEqual([])
    }
  },
})

export { expect, expectAllCommitted, screenshot }
