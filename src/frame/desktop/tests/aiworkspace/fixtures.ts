import { expect, test as base, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const BACKEND_FIXTURE = resolve('../aiworkspace/fixtures/project-workspace/commits.json')
export const VECTORS_PATH = resolve('../aiworkspace/fixtures/vectors/object-ids.json')
export const BUNDLED_FIXTURE = resolve('src/app/aiworkspace/fixtures/project-workspace.commits.json')
export { BACKEND_FIXTURE }

// The backend's JSON results are inspected ad hoc in assertions.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

/** Direct kRPC access to the backend under test — what the assertions "through the API" use. */
export class Api {
  private seq = 1
  readonly base: string
  constructor() {
    const backend = process.env.AIWS_E2E_BACKEND
    if (!backend) throw new Error('AIWS_E2E_BACKEND is not set: run with --config=playwright.aiworkspace.config.ts')
    this.base = `${backend}/kapi/aiworkspace`
  }

  async rpc(token: string, method: string, params: Json): Promise<Json> {
    const response = await fetch(this.base, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ method, params, sys: [this.seq++, token] }) })
    const body = await response.json() as Json
    if (body.error) throw new Error(`${method}: ${body.error}`)
    return body.result
  }

  async commit(token: string, ws: { workspace_id: string; epoch: string }, operations: Json[], key = `test/${Date.now()}/${Math.random().toString(36).slice(2)}`): Promise<Json> {
    return this.rpc(token, 'doc.commit', { protocol_version: '0.1', workspace_id: ws.workspace_id, epoch: ws.epoch, idempotency_key: key, session_id: 'test-api', operations })
  }

  /** The shared sample of design §3.8, replayed through the public interface (same as the Rust tests). */
  async sample(token: string, title: string): Promise<{ workspace_id: string; epoch: string }> {
    const ws = await this.rpc(token, 'ws.create', { title })
    const fixture = JSON.parse(readFileSync(BACKEND_FIXTURE, 'utf8')) as Json
    let text = JSON.stringify(fixture.commits)
    for (const asset of fixture.assets) {
      const bytes = Buffer.from(asset.base64, 'base64')
      const begin = await this.rpc(token, 'asset.begin_upload', { workspace_id: ws.workspace_id, size: bytes.length })
      const put = await fetch(`${this.base}/upload/${begin.upload_id}`, { method: 'PUT', headers: { authorization: `Bearer ${token}` }, body: bytes })
      expect(put.ok).toBe(true)
      const done = await this.rpc(token, 'asset.finish_upload', { workspace_id: ws.workspace_id, upload_id: begin.upload_id })
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
}

/** Open the Desktop shell and the AI Workspace app in it, talking to the standalone backend as `token`. */
export async function openApp(page: Page, token: string) {
  await page.addInitScript((value) => window.localStorage.setItem('aiworkspace.dev', JSON.stringify({ token: value })), token)
  await page.goto('/?scenario=normal')
  await page.getByTestId('desktop-app-aiworkspace').click()
  await expect(page.getByTestId('aiws-list')).toBeVisible({ timeout: 30_000 })
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

/** Open a workspace and switch to the canvas top-level mode. */
export async function openCanvas(page: Page, token: string, workspaceId: string) {
  await openWorkspace(page, token, workspaceId)
  await page.getByTestId('aiws-top-canvas').click()
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

export const test = base.extend<{ api: Api }>({
  api: async ({}, use) => { await use(new Api()) },
})
export { expect }
