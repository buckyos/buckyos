/**
 * Sharing on a real Zone (notepads/share-content-mgr-tech-design-todo.md §6, M5).
 *
 *   BUCKYOS_TEST_ZONE_HOST=test.buckyos.io npx playwright test --config=playwright.dv.config.ts tests/dv/share.spec.ts
 *
 * Signs in as a normal user (default `bob`, the dev-config users bob/alice share the password in
 * `BUCKYOS_TEST_USER_PASSWORD`, default `buckyos2026`), uploads its fixtures through NFSP, and drives
 * Files → Share…, the visitor page on `share.<zone>`, HomeStation and MessageHub hand-offs and My Content.
 * It stops every share it made and deletes its fixture folder; the HomeStation post and the message to the
 * peer stay.
 */

import { expect, test, type APIRequestContext, type Browser, type BrowserContext, type Page } from '@playwright/test'
import { deflateSync } from 'node:zlib'

const zone = process.env.BUCKYOS_TEST_ZONE_HOST || 'test.buckyos.io'
const origin = `https://${zone}`
const user = process.env.BUCKYOS_TEST_SHARE_USER || 'bob'
const peer = process.env.BUCKYOS_TEST_SHARE_PEER || 'alice'
const password = process.env.BUCKYOS_TEST_USER_PASSWORD || 'buckyos2026'
const folder = `share-dv-${Date.now().toString(36)}`

async function login(browser: Browser, userId: string): Promise<{ context: BrowserContext; page: Page; token: string }> {
  const context = await browser.newContext({ ignoreHTTPSErrors: true, viewport: { width: 1440, height: 1000 } })
  const page = await context.newPage()
  await page.goto(`${origin}/`)
  await page.getByLabel('Username', { exact: true }).fill(userId)
  await page.getByLabel('Password', { exact: true }).fill(password)
  await page.getByRole('button', { name: 'Sign In', exact: true }).click()
  await expect(page.getByRole('button', { name: 'BuckyOS' }).first()).toBeVisible({ timeout: 30_000 })
  const token = (await context.cookies()).find((cookie) => cookie.name === 'buckyos_session_token')?.value
  if (!token) throw new Error('real login returned no session cookie')
  return { context, page, token }
}

/** Minimal NFSP client over the gateway (`/nfs/v1/*`) with a Bearer session token. */
class Nfsp {
  private session = ''
  private seq = 0
  constructor(private readonly request: APIRequestContext, private readonly token: string) {}

  private async post(method: string, body: Record<string, unknown>) {
    const response = await this.request.post(`${origin}/nfs/v1/${method}`, { headers: { Authorization: `Bearer ${this.token}` }, data: body })
    const parsed = await response.json() as { ok: boolean; result?: Record<string, unknown>; error?: Record<string, unknown> }
    if (!parsed.ok) throw new Error(`${method}: ${JSON.stringify(parsed.error)}`)
    return parsed.result as Record<string, any>
  }

  async hello() { this.session = (await this.post('hello', { args: {} })).session }
  call(method: string, args: Record<string, unknown>, at?: Record<string, unknown>) { return this.post(method, { session: this.session, args, at }) }
  write(method: string, args: Record<string, unknown>, at?: Record<string, unknown>) { return this.post(method, { session: this.session, seq: ++this.seq, args, at }) }

  async upload(dir: string, name: string, bytes: Buffer) {
    const parent = await this.call('stat', {}, { path: dir })
    const open = await this.write('open_write', { parent_ref: parent.ref, name, size: bytes.length })
    const patch = await this.request.fetch(`${origin}/nfs/v1/uploads/${open.fb_handle}`, {
      method: 'PATCH',
      headers: { Authorization: `Bearer ${this.token}`, 'Upload-Offset': '0', 'Tus-Resumable': '1.0.0', 'Content-Type': 'application/offset+octet-stream' },
      data: bytes,
    })
    expect(patch.status()).toBe(204)
    await this.write('commit_file', { name, fb_handle: open.fb_handle, lease_id: open.lease.lease_id }, { ref: parent.ref })
  }
}

function png(): Buffer {
  const chunk = (type: string, data: Buffer) => {
    const body = Buffer.concat([Buffer.from(type), data])
    let crc = ~0
    for (const byte of body) { crc ^= byte; for (let k = 0; k < 8; k++) crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1)) }
    const out = Buffer.alloc(12 + data.length)
    out.writeUInt32BE(data.length, 0); body.copy(out, 4); out.writeUInt32BE(~crc >>> 0, 8 + data.length)
    return out
  }
  const size = 64
  const rows = Buffer.concat(Array.from({ length: size }, (_, y) => Buffer.from([0, ...Array.from({ length: size }, (_, x) => [40 + x * 3, 80 + y * 2, 200 - x * 2]).flat()])))
  const header = Buffer.alloc(13); header.writeUInt32BE(size, 0); header.writeUInt32BE(size, 4); header.set([8, 2, 0, 0, 0], 8)
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk('IHDR', header), chunk('IDAT', deflateSync(rows)), chunk('IEND', Buffer.alloc(0))])
}

async function openFolder(page: Page) {
  await page.getByTestId('desktop-app-files').first().click()
  const win = page.getByTestId('window-files')
  await win.getByRole('cell', { name: new RegExp(`^${user}(\\s|$)`) }).dblclick()
  await win.getByRole('cell', { name: new RegExp(`^${folder}(\\s|$)`) }).dblclick()
  await expect(win.getByRole('cell', { name: /^notes\.txt/ })).toBeVisible({ timeout: 20_000 })
  return win
}

async function openShare(page: Page, name: string) {
  await page.getByTestId('window-files').getByRole('cell', { name: new RegExp(`^${name.replace('.', '\\.')}`) }).click({ button: 'right' })
  await page.getByRole('menuitem', { name: 'Share…', exact: true }).click()
  const dialog = page.getByTestId('share-dialog')
  await expect(dialog.getByTestId('share-dialog-link-form')).toBeVisible({ timeout: 20_000 })
  return dialog
}

test.describe.serial('sharing on a real zone', () => {
  let owner: { context: BrowserContext; page: Page; token: string }
  let nfsp: Nfsp

  test.beforeAll(async ({ browser }) => {
    owner = await login(browser, user)
    nfsp = new Nfsp(owner.context.request, owner.token)
    await nfsp.hello()
    await nfsp.write('mkdir', {}, { path: `/home/${user}/${folder}` })
    await nfsp.upload(`/home/${user}/${folder}`, 'notes.txt', Buffer.from('share dv notes\n'))
    await nfsp.upload(`/home/${user}/${folder}`, 'photo.png', png())
    await nfsp.upload(`/home/${user}/${folder}`, 'report.pdf', Buffer.from('%PDF-1.4 share dv'))
  })

  test.afterAll(async () => {
    if (!nfsp) return
    const mine = await nfsp.call('share.list', { q: folder, include_hidden: true, limit: 200 })
    for (const share of mine.items as { share_id: string; status: string; revision: number }[]) {
      if (share.status === 'active') await nfsp.write('share.stop', { share_id: share.share_id, revision: share.revision, reason: 'dv cleanup' })
    }
    await nfsp.write('delete', { name: folder, recursive: true }, { path: `/home/${user}` })
    await owner.context.close()
  })

  test('password link: share.<zone> page, unlock, snapshot, stats, stop', async ({ browser }) => {
    const page = owner.page
    await openFolder(page)
    const dialog = await openShare(page, 'notes.txt')
    await expect(dialog.getByTestId('share-expiry')).toHaveValue('7d')
    await dialog.getByTestId('share-access-password').check()
    await dialog.getByTestId('share-password').fill('dv-pass')
    await dialog.getByTestId('share-dialog-create').click()
    await expect(dialog.getByTestId('share-dialog-created')).toBeVisible({ timeout: 30_000 })
    const url = await dialog.getByTestId('share-dialog-copy-url').inputValue()
    expect(url.startsWith(`https://share.${zone}/s/`)).toBe(true)
    await dialog.getByTestId('share-dialog-done').click()

    // The share origin serves only /s/*.
    const anon = await browser.newContext({ ignoreHTTPSErrors: true })
    expect((await anon.request.post(`https://share.${zone}/nfs/v1/hello`, { data: { args: {} } })).status()).toBe(404)
    expect((await anon.request.get(`${url}/d`)).status()).toBe(401)
    const visitor = await anon.newPage()
    await visitor.goto(url)
    await visitor.getByPlaceholder('Password').fill('wrong')
    await visitor.getByRole('button', { name: 'Open' }).click()
    await expect(visitor.getByRole('alert')).toHaveText('Wrong password')
    await visitor.getByPlaceholder('Password').fill('dv-pass')
    await visitor.getByRole('button', { name: 'Open' }).click()
    await expect(visitor.locator('pre')).toHaveText('share dv notes\n')
    const [download] = await Promise.all([visitor.waitForEvent('download'), visitor.getByRole('link', { name: 'Download' }).click()])
    expect(download.suggestedFilename()).toBe('notes.txt')

    const share = (await nfsp.call('share.list', { q: folder, kind: 'link' })).items[0]
    await expect.poll(async () => (await nfsp.call('share.stats', { share_id: share.share_id, granularity: 'hour' })).totals, { timeout: 20_000 })
      .toMatchObject({ views: 1, downloads: 1, visitors: 1 })

    const stopped = await nfsp.write('share.stop', { share_id: share.share_id, revision: share.revision })
    expect(stopped.status).toBe('stopped')
    expect((await anon.request.get(url)).status()).toBe(404)
    await anon.close()
  })

  test('HomeStation: the photo snapshot becomes a post attachment served by HomeStation', async ({ browser }) => {
    const page = owner.page
    const dialog = await openShare(page, 'photo.png')
    await dialog.getByTestId('share-dialog-homestation').click()
    await expect(dialog.getByTestId('share-dialog-target-homestation')).toContainText('added to a new post')
    await dialog.getByTestId('share-dialog-target-continue').click()
    const hs = page.getByTestId('window-homestation')
    await expect(hs.getByTestId('hs-publish-attachments')).toContainText('photo.png', { timeout: 30_000 })
    await hs.getByTestId('hs-publish-text').fill(`Share DV ${folder}`)
    await hs.getByTestId('hs-publish-submit').click()
    await expect(hs.getByTestId('hs-publish-attachments')).toHaveCount(0, { timeout: 30_000 })

    const delivery = (await nfsp.call('share.list', { q: folder, kind: 'app_delivery' })).items.find((s: { name: string }) => s.name === 'photo.png')
    expect(delivery.delivery_state).toBe('confirmed')
    expect(delivery.business_ref.entry).toContain(`/home/${user}/`)
    const anon = await browser.newContext({ ignoreHTTPSErrors: true })
    const image = await anon.request.get(`${origin}/home/${user}/objects/${delivery.obj_id}/content`)
    expect(image.status()).toBe(200)
    expect((await image.body()).equals(png())).toBe(true)
    await anon.close()
  })

  test('MessageHub: the PDF snapshot reaches the peer, who reads it through msg-center', async ({ browser }) => {
    const page = owner.page
    await page.getByTestId('window-files').click({ position: { x: 400, y: 14 } })
    const dialog = await openShare(page, 'report.pdf')
    await dialog.getByTestId('share-dialog-messagehub').click()
    await expect(dialog.getByTestId('share-dialog-target-messagehub')).toContainText('sent as an attachment')
    await dialog.getByTestId('share-dialog-target-continue').click()
    const picker = page.getByTestId('forward-dialog')
    await expect(picker).toBeVisible({ timeout: 30_000 })
    await picker.getByTestId('forward-targets').getByRole('button', { name: peer }).first().click()
    await expect(picker).toBeHidden({ timeout: 30_000 })

    const delivery = (await nfsp.call('share.list', { q: folder, kind: 'app_delivery' })).items.find((s: { name: string }) => s.name === 'report.pdf')
    await expect.poll(async () => (await nfsp.call('share.get', { share_id: delivery.share_id })).delivery_state).toBe('confirmed')
    const recipient = await login(browser, peer)
    const content = await recipient.context.request.get(`${origin}/kapi/msg-center/objects/${delivery.obj_id}/content`, { headers: { Authorization: `Bearer ${recipient.token}` } })
    expect(content.status()).toBe(200)
    expect(await content.text()).toBe('%PDF-1.4 share dv')
    await recipient.context.close()
  })

  test('My Content lists the deliveries and the stopped link with its history', async () => {
    const page = owner.page
    const taskbar = page.getByTestId('desktop-taskbar')
    if ((await taskbar.getAttribute('aria-hidden')) === 'true') await page.getByTestId('desktop-taskbar-avatar').click()
    await page.getByTestId('taskbar-all-apps').click()
    await page.getByTestId('all-apps-item-my-content').click()
    const mc = page.getByTestId('window-my-content')
    await mc.getByTestId('mc-search').fill(folder)
    await expect(mc.getByTestId('mc-row')).toHaveCount(3)
    await mc.getByTestId('mc-row').filter({ hasText: 'photo.png' }).click()
    await expect(mc.getByTestId('mc-detail-delivery')).toContainText('Delivered')
    await mc.getByTestId('mc-detail-close').click()
    await mc.getByTestId('mc-row').filter({ hasText: 'notes.txt' }).click()
    await expect(mc.getByTestId('mc-detail').getByTestId('mc-status')).toHaveText('Stopped')
    await expect(mc.getByTestId('mc-detail-audit')).toContainText('Stopped')
    await expect(mc.getByTestId('mc-detail')).toContainText('Views 1 · Downloads 1')
  })
})
