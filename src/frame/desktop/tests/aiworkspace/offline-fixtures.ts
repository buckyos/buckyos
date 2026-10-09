import { createServer, connect, type Server, type Socket } from 'node:net'
import { chromium, type BrowserContext, type Page } from '@playwright/test'
import { backToList, expect, test as base, type Api, openStatus } from './fixtures'

/**
 * The network between the browser and BuckyOS, for real: a TCP relay in front of the production
 * build served by `vite preview` (which forwards /kapi/aiworkspace to the backend process).
 * `down()` closes the listening socket and every open connection, so the browser — pages, workers
 * and the service worker alike — gets "connection refused"; `up()` listens on the same port again.
 * Each relay has its own port, hence its own origin: service worker, OPFS and localStorage of one
 * test never meet another's.
 */
export class Network {
  private server: Server | null = null
  private readonly sockets = new Set<Socket>()
  private port = 0
  private readonly target: number

  constructor() {
    const target = Number(process.env.AIWS_E2E_PREVIEW_PORT)
    if (!target) throw new Error('AIWS_E2E_PREVIEW_PORT is not set: run with --config=playwright.aiworkspace.config.ts')
    this.target = target
  }

  get origin(): string { return `http://127.0.0.1:${this.port}` }

  private listen(): Promise<void> {
    return new Promise((resolve, reject) => {
      const server = createServer((client) => {
        const upstream = connect(this.target, '127.0.0.1')
        this.sockets.add(client)
        this.sockets.add(upstream)
        client.on('close', () => { this.sockets.delete(client); upstream.destroy() })
        upstream.on('close', () => { this.sockets.delete(upstream); client.destroy() })
        client.on('error', () => undefined)
        upstream.on('error', () => undefined)
        client.pipe(upstream)
        upstream.pipe(client)
      })
      server.once('error', reject)
      server.listen(this.port, '127.0.0.1', () => {
        this.port = (server.address() as { port: number }).port
        this.server = server
        resolve()
      })
    })
  }

  up(): Promise<void> {
    return this.server ? Promise.resolve() : this.listen()
  }

  down(): Promise<void> {
    const server = this.server
    this.server = null
    for (const socket of this.sockets) socket.destroy()
    this.sockets.clear()
    return new Promise((resolve) => { if (server) server.close(() => resolve()); else resolve() })
  }
}

/** Chromium grants `navigator.storage.persist()` to an origin that has the durable-storage permission
 * (set once the origin is loaded; without it a fresh profile is refused — see the "persistence denied" test). */
export async function grantPersistence(context: BrowserContext, page: Page) {
  // the DevTools session stays attached: Chromium drops the permission override when it detaches
  const cdp = await context.newCDPSession(page)
  await cdp.send('Browser.grantPermissions', { permissions: ['durableStorage'] })
}

/** Open the Desktop (production build) through the relay and start the AI Workspace app. */
export async function openDesktop(page: Page, net: Network, token: string) {
  await page.addInitScript((value) => window.localStorage.setItem('aiworkspace.dev', JSON.stringify({ token: value })), token)
  await page.goto(`${net.origin}/?scenario=normal`)
  await page.getByTestId('desktop-app-aiworkspace').click()
  await expect(page.getByTestId('aiws-root')).toBeVisible({ timeout: 30_000 })
  await backToList(page)
}

/** The service worker controls the page and has every application resource in its cache. */
export async function serviceWorkerReady(page: Page) {
  await page.evaluate(async () => { await navigator.serviceWorker.ready })
  await expect.poll(() => page.evaluate(() => navigator.serviceWorker.controller !== null), { timeout: 30_000 }).toBe(true)
}

export async function openCard(page: Page, workspaceId: string) {
  await page.locator(`[data-testid="aiws-workspace-card"][data-workspace-id="${workspaceId}"]`).getByTestId('aiws-open').click()
  await expect(page.getByTestId('aiws-workspace')).toBeVisible({ timeout: 30_000 })
}

/** Open a workspace online, prepare it for offline, and end up as the holder of its replica. */
export async function prepareOffline(page: Page, net: Network, token: string, workspaceId: string) {
  await openDesktop(page, net, token)
  await grantPersistence(page.context(), page)
  await openCard(page, workspaceId)
  await expect(page.getByTestId('aiws-mode')).toHaveAttribute('data-mode', 'direct')
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
  await openStatus(page)
  await page.getByTestId('aiws-prepare-offline').click()
  await expect(page.getByTestId('aiws-mode')).toHaveAttribute('data-mode', 'replica', { timeout: 60_000 })
  await expect(page.getByTestId('aiws-prepare-report')).toBeVisible()
  await expect(page.getByTestId('aiws-conn')).toHaveAttribute('data-status', 'live')
}

export async function editCell(page: Page, record: string, fieldLabel: string, field: string, value: string) {
  const all = page.getByTestId('aiws-table-cell-all-tasks')
  await all.getByTestId(`aiws-cell-${record}-${field}`).getByRole('button').first().click()
  await page.getByLabel(`${fieldLabel} ${record}`, { exact: true }).fill(value)
  await page.getByLabel(`${fieldLabel} ${record}`, { exact: true }).press('Enter')
  return all.getByTestId(`aiws-cell-${record}-${field}`)
}

export async function saveSummary(page: Page) {
  const summary = page.getByTestId('aiws-save-summary')
  return {
    unsaved: Number(await summary.getAttribute('data-unsaved')),
    local: Number(await summary.getAttribute('data-local')),
    attention: Number(await summary.getAttribute('data-attention')),
    pending: Number(await summary.getAttribute('data-pending')),
  }
}

export const test = base.extend<{ net: Network }>({
  // A regular (on-disk) browser profile per test: Chromium never grants persistent storage to the
  // off-the-record contexts Playwright creates by default, and a replica is not offered without it.
  context: async ({ viewport }, use, testInfo) => {
    const context = await chromium.launchPersistentContext(testInfo.outputPath('profile'), { viewport, acceptDownloads: true })
    await use(context)
    await context.close()
  },
  page: async ({ context }, use) => {
    await use(context.pages()[0] ?? await context.newPage())
  },
  net: async ({}, use) => {
    const net = new Network()
    await net.up()
    await use(net)
    await net.down()
  },
})
export { expect }
export type { Api }
