/* The control mechanism on a real Zone (notepads/aiworkspace-control-mechanism-todo.md §5): the owner's private schedule
 * (created on first use, the Agent commands and both official controls agreeing, a link landing on its record, a member
 * kept out) and the four examples of doc/workspace/control-examples built as they are in the repository — the TV screen
 * with a video over 32 MiB chosen from the Zone, the time-limited control, the photo vote and listen together — with
 * the owner on the desktop and a second account on a phone. */

import { execFileSync } from 'node:child_process'
import { randomBytes, randomUUID } from 'node:crypto'
import { mkdtempSync, readdirSync, readFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { deflateSync } from 'node:zlib'
import { devices, type Browser, type BrowserContext, type Page } from '@playwright/test'
import type { EntityEnvelope, Operation, WorkspaceSummary } from '../../src/app/aiworkspace/api/types'
import { openedPicker, zoneGoto, type PickedFile } from '../aiworkspace/fixtures'
import { selectRow } from '../e2e/fixtures/selectDialog'
import { expect, openWorkspace, putHomeFile, screenshot, test, ZoneClient, type Actor } from './aiworkspace.fixtures'

type Ws = Pick<WorkspaceSummary, 'workspace_id' | 'epoch'>
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type J = any

test.use({ viewport: { width: 1600, height: 1000 } })

const EXAMPLES = resolve('../../../doc/workspace/control-examples/controls')
const MEDIA = process.env.AIWS_DV_MEDIA_DIR ?? ''

function media(name: string): Buffer {
  if (!MEDIA) throw new Error('Set AIWS_DV_MEDIA_DIR to the folder with tv-big.webm, tv-small.webm, podcast-1h.webm, "a song.webm", "a song.lrc", "b song.webm"')
  return readFileSync(join(MEDIA, name))
}

// ---- small helpers

const CRC = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0 })
function crc32(b: Buffer): number { let c = 0xffffffff; for (const x of b) c = CRC[(c ^ x) & 0xff] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0 }
function chunk(type: string, data: Buffer): Buffer {
  const len = Buffer.alloc(4); len.writeUInt32BE(data.length)
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data])
  const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(body))
  return Buffer.concat([len, body, crc])
}
function png(r: number, g: number, b: number, w = 48, h = 32): Buffer {
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2
  const rows = Buffer.alloc((w * 3 + 1) * h)
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) rows.set([r, g, b], y * (w * 3 + 1) + 1 + x * 3)
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(rows)), chunk('IEND', Buffer.alloc(0))])
}

/** The buckyos CLI of the websdk checkout next to this repository, as `actor` (what an Agent runs). */
function cli(actor: Actor, baseURL: string, ...args: string[]): J {
  const launcher = resolve(process.env.BUCKYOS_WEBSDK_DIR ?? '../../../../buckyos-websdk', 'cli/launcher.mjs')
  const cwd = mkdtempSync(join(tmpdir(), 'aiws-ctl-dv-'))
  let out: string
  try {
    out = execFileSync(process.execPath, [launcher, '--non-interactive', '--yes', '--output', 'json', '--timeout', '2m', ...args], {
      cwd, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'],
      env: { ...process.env, BUCKYOS_TOOL_ENDPOINT: baseURL, BUCKYOS_APPCLIENT_SESSION_TOKEN: actor.client.token, NODE_TLS_REJECT_UNAUTHORIZED: '0', HOME: cwd },
    })
  } catch (error) {
    const failed = error as { stdout?: string; stderr?: string }
    throw new Error(`buckyos ${args.slice(0, 3).join(' ')} failed: ${failed.stdout ?? ''} ${failed.stderr?.slice(-2000) ?? ''}`)
  }
  const envelope = JSON.parse(out) as { ok: boolean; data: J }
  expect(envelope.ok, out).toBe(true)
  return envelope.data
}

const serverNow = async (actor: Actor): Promise<number> => (await actor.client.ok<{ server_ms: number }>('ws.time', {})).server_ms
const outline = async (actor: Actor, ws: Ws) => (await actor.client.ok<{ entities: EntityEnvelope[] }>('doc.outline', { workspace_id: ws.workspace_id })).entities
const readEntity = (actor: Actor, ws: Ws, id: string) => actor.client.ok<{ content: J }>('doc.read', { workspace_id: ws.workspace_id, entity_id: id })
const config = async (actor: Actor, ws: Ws, id: string) => (await readEntity(actor, ws, id)).content.payload.config ?? {}
const accepted = async (actor: Actor, ws: Ws, ops: J[]) => {
  const result = await actor.client.commit(ws, ops as Operation[])
  expect(result.status, JSON.stringify(result)).toBe('accepted')
  return result
}

/** A workspace of the owner's (cleaned by the fixture) with one free canvas `free`. */
async function freeCanvas(actor: Actor): Promise<Ws> {
  const created = await actor.client.ok<WorkspaceSummary>('ws.create', { title: actor.client.prefix })
  actor.client.owned.add(created.workspace_id)
  const ws = { workspace_id: created.workspace_id, epoch: created.epoch }
  await accepted(actor, ws, [
    { op: 'entity.create', entity_id: 'free-content', type_id: 'buckyos.container', parent_id: 'canvas-content', order_key: 'a', payload: { kind: 'folder', system: 'surface_content', surface_id: 'free', title: 'Free' } },
    { op: 'entity.create', entity_id: 'free', type_id: 'buckyos.container', parent_id: 'surfaces', order_key: 'a', payload: { kind: 'surface', layout: { mode: 'free' }, title: 'Free', content_folder_id: 'free-content' } },
  ])
  return ws
}

/** `ext.build` of an example as it is in the repository. */
async function build(actor: Actor, ws: Ws, name: string): Promise<string> {
  const dir = join(EXAMPLES, name)
  const files = readdirSync(dir).map((file) => ({ name: file, base64: readFileSync(join(dir, file)).toString('base64') }))
  const built = await actor.client.ok<{ status: string; def_entity_id: string }>('ext.build', { workspace_id: ws.workspace_id, files })
  expect(built.status).toBe('accepted')
  return built.def_entity_id
}

/** Bytes into the workspace: put into the caller's own `assets` bucket (the data plane), taken over by the service. */
async function upload(actor: Actor, ws: Ws, bytes: Buffer): Promise<string> {
  const bucket = `user/aiworkspace/${actor.userId}/assets`
  const key = `${ws.workspace_id}/dv-${randomUUID()}`
  const put = await actor.page.request.put(`/bucket/v1/${bucket}/${key}`, { headers: { Authorization: `Bearer ${actor.client.token}` }, data: bytes })
  expect(put.ok(), await put.text()).toBe(true)
  const staged = await actor.client.ok<{ object_id: string }>('asset.import_object', { workspace_id: ws.workspace_id, bucket_id: bucket, key })
  return staged.object_id
}

/** A file put into the actor's home and chosen there in the system picker `open` brings up. "My files" of an admin
 * is `/home` (every member's folder): then the actor's own folder first. */
async function chooseFromHome(actor: Actor, open: () => Promise<unknown>, dir: string, file: PickedFile) {
  await putHomeFile(actor, dir, file)
  await open()
  const { dialog } = await openedPicker(actor.page)
  await zoneGoto(dialog, [])
  const own = selectRow(dialog, actor.userId)
  const first = selectRow(dialog, dir.split('/').filter(Boolean)[0])
  await expect(own.or(first).first()).toBeVisible({ timeout: 15_000 })
  if (await own.isVisible()) await own.click()
  for (const segment of dir.split('/').filter(Boolean)) await selectRow(dialog, segment).click()
  await selectRow(dialog, file.name).click()
  await dialog.getByTestId('select-confirm').click()
}

const composite = (id: string, def: string, extra: J = {}) => ({ op: 'entity.create', entity_id: id, type_id: 'buckyos.container', parent_id: 'free', order_key: 'a',
  placement: { x: 60, y: 60, w: 'auto', h: 'auto' }, payload: { kind: 'composite', def_ref: { entity_id: def }, ...extra } })

async function showCanvas(page: Page) {
  if (await page.getByTestId('aiws-top-canvas').isVisible().catch(() => false)) await page.getByTestId('aiws-top-canvas').click()
  await expect(page.getByTestId('aiws-canvas-view')).toBeVisible({ timeout: 30_000 })
}

async function open(actor: Actor, ws: Ws) {
  await openWorkspace(actor, ws)
  await showCanvas(actor.page)
}

async function canvasMode(page: Page, mode: 'edit' | 'view') {
  await page.getByTestId('aiws-main-menu').click()
  await page.getByTestId('aiws-menu-canvas-mode').click()
  await page.getByTestId(`aiws-mode-${mode}`).click()
  await expect(page.getByTestId('aiws-canvas')).toHaveAttribute('data-mode', mode)
}

/** A device clock `skewMs` off (every page this context opens from now on). */
async function skew(target: Page | BrowserContext, skewMs: number) {
  await target.addInitScript((ms) => { const now = Date.now.bind(Date); Date.now = () => now() + ms }, skewMs)
}

/** The same account on a phone: a Pixel 7 context signed in with the account's Zone session. */
async function onPhone(actor: Actor, browser: Browser, baseURL: string, errors: string[]): Promise<{ phone: Actor; context: BrowserContext }> {
  const context = await browser.newContext({ ...devices['Pixel 7'], baseURL, ignoreHTTPSErrors: true, storageState: await actor.page.context().storageState() })
  const page = await context.newPage()
  page.on('pageerror', (error) => errors.push(`${actor.userId} phone: ${error.message}`))
  page.on('console', (message) => { if (message.type() === 'error' && !message.location().url.endsWith('/sso_refresh')) errors.push(`${actor.userId} phone: ${message.text()}`) })
  await page.goto('/')
  return { phone: { page, client: new ZoneClient(page, actor.client.token), userId: actor.userId }, context }
}

function localDate(ms: number): string {
  const d = new Date(ms)
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`
}

function localIso(date: string, hh: number): string {
  const d = new Date(`${date}T${String(hh).padStart(2, '0')}:00:00`)
  const off = -d.getTimezoneOffset()
  const sign = off >= 0 ? '+' : '-'
  return `${date}T${String(hh).padStart(2, '0')}:00:00${sign}${String(Math.floor(Math.abs(off) / 60)).padStart(2, '0')}:${String(Math.abs(off) % 60).padStart(2, '0')}`
}

// ---- §5.1 / §5.2 on the Zone: the private schedule

test('[ctl] the private schedule: made on first use, the Agent commands and both controls agree, a link lands on its record, a member is kept out', async ({ owner, peer, baseURL }, info) => {
  test.setTimeout(300_000)
  const tag = randomBytes(3).toString('hex')
  const page = owner.page
  const before = (await owner.client.list()).workspaces.find((w) => (w as J).name === 'schedule')
  let wsId = ''
  const todos: string[] = []
  const events: string[] = []
  let peerSchedule = ''
  try {
    // the Agent's first command makes the schedule (G9, G11)
    const today = localDate(Date.now())
    const first = cli(owner, baseURL!, 'aiws', 'todo', 'add', `DV ${tag} 续签护照`, '--due', today).item
    todos.push(first.id)
    const listed = cli(owner, baseURL!, 'aiws', 'todo', 'list')
    wsId = listed.workspace_id
    expect(listed.table).toBe('todo')
    if (before) expect(wsId).toBe(before.workspace_id)
    const ensured = await owner.client.ok<{ workspace_id: string; epoch: string; created: boolean }>('ws.ensure', { name: 'schedule' })
    expect(ensured).toMatchObject({ workspace_id: wsId, created: false })
    const ws = { workspace_id: wsId, epoch: ensured.epoch }
    await info.attach('schedule', { body: Buffer.from(JSON.stringify({ workspace_id: wsId, existed_before: Boolean(before) })), contentType: 'application/json' })

    // the schedule page: both controls, the Agent's to-do in the list
    await open(owner, ws)
    const item = (id: string, node = 'title') => page.locator(`[data-proj="schedule-board-todo~${node}~${id}"]`)
    await expect(page.getByTestId('aiws-todo-schedule-board-todo')).toBeVisible()
    await expect(page.getByTestId('aiws-cal-schedule-board-calendar')).toBeVisible()
    await expect(item(first.id)).toContainText(`DV ${tag} 续签护照`)
    // the to-do's due date shows on the calendar
    await expect(page.locator(`[data-proj^="schedule-board-calendar~title~todo:${first.id}"]`).first()).toContainText(`DV ${tag}`)

    // quick add in the control (Enter keeps adding), tick to complete: the Agent's list agrees
    const head = await page.getByTestId('aiws-todo-head-schedule-board-todo').boundingBox()
    await page.mouse.click(head!.x + 200, head!.y + 12)
    await expect(page.getByTestId('aiws-data-body-schedule-board-todo')).toHaveAttribute('data-active', 'true')
    const add = page.getByTestId('aiws-todo-add-schedule-board-todo')
    await add.click()
    await add.fill(`DV ${tag} 买牛奶`)
    await add.press('Enter')
    await add.fill(`DV ${tag} 浇花`)
    await add.press('Enter')
    await expect(add).toHaveValue('')
    await expect.poll(() => cli(owner, baseURL!, 'aiws', 'todo', 'list').items.filter((i: J) => i.title.startsWith(`DV ${tag}`)).length, { timeout: 15_000 }).toBe(3)
    const mine = cli(owner, baseURL!, 'aiws', 'todo', 'list').items.filter((i: J) => i.title.startsWith(`DV ${tag}`))
    todos.push(...mine.map((i: J) => i.id).filter((id: string) => !todos.includes(id)))
    const milk = mine.find((i: J) => i.title.endsWith('买牛奶'))
    await item(milk.id, 'check').click()
    await expect.poll(() => cli(owner, baseURL!, 'aiws', 'todo', 'list', '--all').items.find((i: J) => i.id === milk.id)?.done, { timeout: 15_000 }).toBe(true)

    // the Agent writes while the control is open: seen at once (§5.1 7)
    const agent = cli(owner, baseURL!, 'aiws', 'todo', 'add', `DV ${tag} Agent 加的`).item
    todos.push(agent.id)
    await expect(item(agent.id)).toContainText('Agent 加的', { timeout: 10_000 })
    cli(owner, baseURL!, 'aiws', 'todo', 'update', '--id', agent.id, '--title', `DV ${tag} Agent 改过`)
    await expect(item(agent.id)).toContainText('Agent 改过', { timeout: 10_000 })

    // an event from the Agent, today 10:00–11:00 on this machine: on the week view; the kernel refuses one ending before it starts
    const ev = cli(owner, baseURL!, 'aiws', 'cal', 'add', `DV ${tag} 评审`, '--start', localIso(today, 10), '--end', localIso(today, 11))
    events.push(ev.id)
    await expect(page.locator(`[data-proj^="schedule-board-calendar~title~${ev.id}"]`).first()).toContainText(`DV ${tag} 评审`, { timeout: 10_000 })
    const backwards = await owner.client.commit(ws, [{ op: 'table.insert_records', source_id: 'calendar', records: [{ record_id: `dvbad${tag}`, values: { title: 'backwards', start: localIso(today, 11), end: localIso(today, 10) } }] }] as unknown as Operation[])
    expect(backwards.status, JSON.stringify(backwards)).not.toBe('accepted')
    await screenshot(page, info, 'schedule-board')

    // a link to one record selects it; page=control shows the control alone (G12, G14)
    const origin = new URL(page.url()).origin
    await page.goto(`${origin}/w/s?id=schedule-board&block=schedule-board-todo&record=${encodeURIComponent(agent.id)}`)
    await expect(item(agent.id)).toBeVisible({ timeout: 30_000 })
    await expect(page.locator(`[data-proj="schedule-board-todo~bg~${agent.id}"] path`).first()).toBeVisible()
    await page.goto(`${origin}/w/s?id=schedule-board&block=schedule-board-todo&page=control`)
    await expect(page.getByTestId('aiws-control-page-schedule-board-todo')).toBeVisible({ timeout: 30_000 })
    await expect(item(agent.id)).toContainText('Agent 改过')

    // a member: not listed, not readable; their own schedule is a workspace of their own (§5.2 11)
    expect((await peer.client.list()).workspaces.map((w) => w.workspace_id)).not.toContain(wsId)
    await expect(peer.client.ok('ws.get_info', { workspace_id: wsId })).rejects.toThrow()
    const theirs = await peer.client.ok<{ workspace_id: string; created: boolean }>('ws.ensure', { name: 'schedule' })
    peerSchedule = theirs.workspace_id
    expect(theirs.workspace_id).not.toBe(wsId)
    await expect(peer.client.ok('doc.read', { workspace_id: wsId, entity_id: 'todo' })).rejects.toThrow()
  } finally {
    if (peerSchedule) await peer.client.ok('ws.delete', { workspace_id: peerSchedule }).catch(() => undefined)
    if (wsId && !before) await owner.client.ok('ws.delete', { workspace_id: wsId })
    else if (wsId) {
      if (todos.length) cli(owner, baseURL!, 'aiws', 'todo', 'rm', '--id', todos.join(','))
      for (const id of events) cli(owner, baseURL!, 'aiws', 'cal', 'rm', '--id', id)
    }
  }
})

// ---- §5.3 TV screen

test('[ctl] TV screen: a video over 32 MiB from the Zone; a phone joins two minutes in at the broadcast position; pause, resume, own listening, a new video; a reader is refused', async ({ owner, peer, browser, baseURL }, info) => {
  test.setTimeout(600_000)
  const errors: string[] = []
  const ws = await freeCanvas(owner)
  const def = await build(owner, ws, 'tv-screen')
  await accepted(owner, ws, [composite('tv', def)])
  await owner.client.ok('ws.grant', { workspace_id: ws.workspace_id, subject: peer.userId, capabilities: ['read'] })
  const page = owner.page
  // the owner's device clock is 30 s behind (§5.3 5): the anchor goes by the service's clock
  await skew(page, -30_000)
  await open(owner, ws)

  // an empty screen: the owner chooses a video in the Zone (the system picker)
  const big = media('tv-big.webm')
  expect(big.length).toBeGreaterThan(32 * 1024 * 1024)
  await chooseFromHome(owner, () => page.getByTestId('aiws-broadcast-choose-tv').click(), 'AIWS DV/tv', { name: 'tv-big.webm', mimeType: 'video/webm', buffer: big })
  let screenCell: EntityEnvelope | undefined
  await expect.poll(async () => {
    screenCell = (await outline(owner, ws)).find((e) => e.parent_id === 'tv' && (e as J).slot === 'screen' && !e.deleted)
    return Boolean(screenCell)
  }, { timeout: 180_000, intervals: [2000] }).toBe(true)
  const assetRef = (await readEntity(owner, ws, (screenCell as J).source_id)).content.payload
  expect(assetRef.size).toBe(big.length)
  await page.getByTestId('aiws-broadcast-start').click({ timeout: 30_000 })
  await expect.poll(async () => (await config(owner, ws, 'tv')).broadcast?.playing ?? false, { timeout: 15_000 }).toBe(true)
  const started = (await config(owner, ws, 'tv')).broadcast
  // written by the service clock, not the owner's slow one
  expect(Math.abs(started.at_ms - await serverNow(owner))).toBeLessThan(10_000)

  // two minutes later a member opens it on a phone and taps for sound
  await page.waitForTimeout(120_000)
  const { phone, context } = await onPhone(peer, browser, baseURL!, errors)
  try {
    await open(phone, ws)
    const video = phone.page.locator('.aiws-broadcast video')
    await expect(video).toBeAttached({ timeout: 30_000 })
    const mute = phone.page.getByTestId('aiws-broadcast-mute')
    if (await phone.page.getByTestId('aiws-broadcast-tap').isVisible().catch(() => false)) await phone.page.getByTestId('aiws-broadcast-tap').tap()
    await mute.tap()
    const live = async () => {
      const a = (await config(owner, ws, 'tv')).broadcast
      return (a.pos_ms + (a.playing ? (await serverNow(owner)) - a.at_ms : 0)) / 1000
    }
    const drift = async () => Math.abs(await video.evaluate((v: HTMLVideoElement) => v.currentTime) - await live())
    await expect.poll(drift, { timeout: 30_000 }).toBeLessThan(1)
    const at = await video.evaluate((v: HTMLVideoElement) => ({ t: v.currentTime, muted: v.muted, paused: v.paused }))
    expect(at.t).toBeGreaterThan(115)
    expect(at.paused).toBe(false)
    expect(at.muted).toBe(false)
    await info.attach('tv-join', { body: Buffer.from(JSON.stringify({ ...at, drift: await drift() })), contentType: 'application/json' })
    // a reader is offered none of the owner's controls
    for (const id of ['aiws-broadcast-start', 'aiws-broadcast-pause', 'aiws-broadcast-replace']) await expect(phone.page.getByTestId(id)).toHaveCount(0)

    // the owner pauses: the phone stops at the same place; resumes: it follows
    await page.getByTestId('aiws-broadcast-pause').click()
    await expect.poll(async () => {
      const a = (await config(owner, ws, 'tv')).broadcast
      return video.evaluate((v: HTMLVideoElement, pos: number) => v.paused && Math.abs(v.currentTime - pos) < 0.5, a.pos_ms / 1000)
    }, { timeout: 15_000 }).toBe(true)
    await page.getByTestId('aiws-broadcast-start').click()
    await expect.poll(drift, { timeout: 15_000 }).toBeLessThan(1)

    // the phone's own pause is its own; back to live returns to the broadcast position
    await phone.page.getByTestId('aiws-broadcast-own').tap()
    await expect.poll(() => video.evaluate((v: HTMLVideoElement) => v.paused)).toBe(true)
    expect((await config(owner, ws, 'tv')).broadcast.playing).toBe(true)
    await phone.page.waitForTimeout(3000)
    await phone.page.getByTestId('aiws-broadcast-back').tap()
    await expect.poll(drift, { timeout: 15_000 }).toBeLessThan(1)

    // a reader cannot move the anchor through the API either
    const read = await readEntity(peer, ws, 'tv')
    const refused = await peer.client.commit(ws, [{ op: 'entity.set_props', entity_id: 'tv', key: 'config', props: { broadcast: { pos_ms: 0, at_ms: 0, playing: false } }, expect: { broadcast: read.content.key_revs['p:config.broadcast'] } }] as unknown as Operation[])
    expect(refused.status).not.toBe('accepted')

    // a new video: the phone switches and starts it from 0 (no new video at the old position)
    const oldSrc = await video.getAttribute('src')
    await chooseFromHome(owner, () => page.getByTestId('aiws-broadcast-replace').click(), 'AIWS DV/tv', { name: 'tv-small.webm', mimeType: 'video/webm', buffer: media('tv-small.webm') })
    await expect.poll(async () => (await video.getAttribute('src')) !== oldSrc, { timeout: 60_000 }).toBe(true)
    const samples: number[] = []
    for (let i = 0; i < 6; i++) { samples.push(await video.evaluate((v: HTMLVideoElement) => v.currentTime)); await phone.page.waitForTimeout(500) }
    expect(Math.max(...samples)).toBeLessThan(15)
    await expect.poll(drift, { timeout: 20_000 }).toBeLessThan(1)
    await screenshot(phone.page, info, 'tv-phone')
    await screenshot(page, info, 'tv-owner')
  } finally {
    await context.close()
    await info.attach('phone-errors', { body: Buffer.from(JSON.stringify(errors)), contentType: 'application/json' })
  }
})

// ---- §5.4 time-limited

test('[ctl] time-limited: each viewer gets their own minute from their first opening (also after a reload); the owner designing sees everything marked', async ({ owner, peer }, info) => {
  test.setTimeout(300_000)
  const ws = await freeCanvas(owner)
  const def = await build(owner, ws, 'time-limited')
  const label = (id: string, text: string, key: string) => ({ op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: 'tl', order_key: key,
    placement: { x: 'auto', y: 'auto', w: 180, h: 36 }, payload: { view: { type: 'label' }, title: text, slot: 'content' } })
  await accepted(owner, ws, [composite('tl', def, { config: { limit_ms: 60_000 } }), label('l1', 'Secret one', 'a'), label('l2', 'Secret two', 'b')])
  await owner.client.ok('ws.grant', { workspace_id: ws.workspace_id, subject: peer.userId, capabilities: ['read'] })
  const block = (p: Page, id: string) => p.getByTestId(`aiws-canvas-block-${id}`)

  // A (the member) opens first and looks
  await open(peer, ws)
  await canvasMode(peer.page, 'view')
  const openedA = Date.now()
  await expect(block(peer.page, 'l1')).toBeVisible()
  await expect(peer.page.locator('[data-proj^="tl.clock~left~"]')).toContainText(/[01]:\d\d/, { timeout: 15_000 })
  // B (the owner, looking) 30 s later
  await owner.page.waitForTimeout(Math.max(0, 30_000 - (Date.now() - openedA)))
  await open(owner, ws)
  await canvasMode(owner.page, 'view')
  await expect(block(owner.page, 'l1')).toBeVisible()

  // A's minute is up: hidden for A, still shown for B
  await expect(block(peer.page, 'l1')).toBeHidden({ timeout: 45_000 })
  const hiddenA = Date.now() - openedA
  await expect(block(peer.page, 'l2')).toBeHidden()
  await expect(peer.page.locator('[data-proj^="tl.clock~gone~"]')).toBeVisible()
  await expect(block(owner.page, 'l1')).toBeVisible()
  // then B's
  await expect(block(owner.page, 'l1')).toBeHidden({ timeout: 45_000 })
  const hiddenB = Date.now() - openedA
  expect(hiddenB - hiddenA).toBeGreaterThan(20_000)
  await info.attach('limited', { body: Buffer.from(JSON.stringify({ hiddenA, hiddenB })), contentType: 'application/json' })

  // A reloads: still hidden; the shared document is unchanged
  await peer.page.reload()
  await showCanvas(peer.page)
  await canvasMode(peer.page, 'view')
  await expect(peer.page.locator('[data-proj^="tl.clock~gone~"]')).toBeVisible({ timeout: 30_000 })
  await expect(block(peer.page, 'l1')).toBeHidden()
  expect((await readEntity(owner, ws, 'l1')).content.payload.title).toBe('Secret one')
  // the owner designing sees all of it, marked
  await canvasMode(owner.page, 'edit')
  await expect(block(owner.page, 'l1')).toBeVisible()
  await expect(owner.page.getByTestId('aiws-composite-limited-tl')).toBeVisible()
  await screenshot(owner.page, info, 'limited-owner-edit')
})

// ---- §5.5 photo vote

test('[ctl] photo vote: eight photos; both vote three on a phone, one changes a pick; every screen shows the same top three; a fourth, another\'s record and a forged submitter are refused', async ({ owner, peer, browser, baseURL }, info) => {
  test.setTimeout(420_000)
  const errors: string[] = []
  const ws = await freeCanvas(owner)
  const def = await build(owner, ws, 'photo-vote')
  const colours: [number, number, number][] = [[220, 60, 60], [60, 160, 60], [60, 90, 220], [230, 180, 40], [150, 60, 200], [40, 190, 190], [240, 120, 40], [120, 120, 120]]
  const members: Record<string, J> = {}
  for (const [i, c] of colours.entries()) members[`Photo ${i + 1}.png`] = { kind: 'file', object_id: await upload(owner, ws, png(...c)) }
  await accepted(owner, ws, [
    { op: 'entity.create', entity_id: 'photos', type_id: 'buckyos.dir', parent_id: 'data', order_key: 'p', name: 'photos', payload: { members } },
    { op: 'entity.create', entity_id: 'votes', type_id: 'buckyos.table-source', parent_id: 'data', order_key: 'v', name: 'votes', payload: { fields: [
      { field_id: 'voter', name: 'voter', type: 'text', submitter: true },
      { field_id: 'picks', name: 'picks', type: 'multi_select', options: [], max_items: 3 },
    ] } },
    composite('vote', def, { bindings: { photos: { entity_id: 'photos' }, votes: { entity_id: 'votes' } } }),
  ])
  await owner.client.ok('ws.grant', { workspace_id: ws.workspace_id, subject: peer.userId, capabilities: ['read'] })
  await owner.client.ok('ws.grant', { workspace_id: ws.workspace_id, subject: peer.userId, scope_entity_id: 'votes', capabilities: ['read', 'append'] })

  // the owner prepares the ballot from the toolbar (declared action)
  const page = owner.page
  await open(owner, ws)
  await expect(page.locator('[data-proj^="vote.photos~img~"]')).toHaveCount(8, { timeout: 30_000 })
  const title = await page.getByTestId('aiws-composite-title-vote').boundingBox()
  await page.mouse.click(title!.x + 20, title!.y + title!.height / 2)
  await page.getByTestId('aiws-near-ext-action-prepare').click()
  await expect.poll(async () => (await readEntity(owner, ws, 'votes')).content.fields.find((f: J) => f.field_id === 'picks').options.length, { timeout: 15_000 }).toBe(8)

  const rows = async () => (await owner.client.ok<{ rows: J[] }>('doc.query', { workspace_id: ws.workspace_id, source_id: 'votes', limit: 50 })).rows
  const ownerPhone = await onPhone(owner, browser, baseURL!, errors)
  const peerPhone = await onPhone(peer, browser, baseURL!, errors)
  try {
    const phones = [ownerPhone.phone, peerPhone.phone]
    for (const p of phones) {
      await open(p, ws)
      await expect(p.page.getByTestId('aiws-canvas')).toHaveAttribute('data-mode', 'view')
      await expect(p.page.locator('[data-proj^="vote.photos~img~"]')).toHaveCount(8, { timeout: 30_000 })
    }
    // a finger on the photo: on a picked one the ring (the same `pick` role) is on top, which takes the tap as well
    const pick = async (p: Actor, n: number) => {
      const box = await p.page.locator(`[data-proj="vote.photos~img~Photo ${n}.png"]`).boundingBox()
      if (!box) throw new Error(`Photo ${n} is not on screen`)
      await p.page.touchscreen.tap(box.x + box.width / 2, box.y + box.height / 2)
      await p.page.waitForTimeout(400)
    }
    // the first picks of both at the same moment (§5.5 5): both recorded
    await Promise.all([pick(ownerPhone.phone, 1), pick(peerPhone.phone, 1)])
    await expect.poll(async () => (await rows()).length, { timeout: 15_000 }).toBe(2)
    for (const n of [2, 3]) await pick(ownerPhone.phone, n)
    for (const n of [4, 2]) await pick(peerPhone.phone, n)
    // the member changes a pick: 4 off, 5 on
    await pick(peerPhone.phone, 4)
    await pick(peerPhone.phone, 5)
    const picksOf = async (user: string) => {
      const labels = (await readEntity(owner, ws, 'votes')).content.fields.find((f: J) => f.field_id === 'picks').options as J[]
      const row = (await rows()).find((r) => r.values.voter === user)
      return (row?.values.picks ?? []).map((id: string) => labels.find((o) => o.option_id === id)?.label).sort()
    }
    await expect.poll(() => picksOf(owner.userId), { timeout: 15_000 }).toEqual(['Photo 1.png', 'Photo 2.png', 'Photo 3.png'])
    await expect.poll(() => picksOf(peer.userId), { timeout: 15_000 }).toEqual(['Photo 1.png', 'Photo 2.png', 'Photo 5.png'])
    // a fourth on the phone: refused, told
    await pick(ownerPhone.phone, 6)
    await expect(ownerPhone.phone.page.getByTestId('aiws-notice').last()).toBeVisible()
    expect(await picksOf(owner.userId)).toHaveLength(3)

    // the cover: Photo 1 and Photo 2 (two votes each, folder order), then Photo 3 (one vote, first in the folder) — the same on all three screens
    const top = (p: Page) => p.locator('[data-proj^="vote.cover~img~"]').evaluateAll((els) => els.map((e) => e.getAttribute('data-item')))
    for (const p of [page, ownerPhone.phone.page, peerPhone.phone.page]) await expect.poll(() => top(p), { timeout: 20_000 }).toEqual(['Photo 1.png', 'Photo 2.png', 'Photo 3.png'])
    await screenshot(peerPhone.phone.page, info, 'vote-peer-phone')
    await screenshot(page, info, 'vote-owner')

    // through the API: the member changes the owner's record, forges the submitter, picks four — all refused
    const ownerRow = (await rows()).find((r) => r.values.voter === owner.userId)
    const opt = (await readEntity(owner, ws, 'votes')).content.fields.find((f: J) => f.field_id === 'picks').options.map((o: J) => o.option_id)
    const tries = [
      { op: 'table.set_values', source_id: 'votes', values: [{ record_id: ownerRow.record_id, field_id: 'picks', value: [opt[7]], expect: { rev: ownerRow.revs.picks } }] },
      { op: 'table.insert_records', source_id: 'votes', records: [{ record_id: `forged${randomBytes(3).toString('hex')}`, values: { voter: owner.userId, picks: [opt[6]] } }] },
      { op: 'table.insert_records', source_id: 'votes', records: [{ record_id: `four${randomBytes(3).toString('hex')}`, values: { picks: opt.slice(0, 4) } }] },
    ]
    for (const op of tries) expect((await peer.client.commit(ws, [op] as unknown as Operation[])).status, JSON.stringify(op)).not.toBe('accepted')
  } finally {
    await ownerPhone.context.close()
    await peerPhone.context.close()
    await info.attach('phone-errors', { body: Buffer.from(JSON.stringify(errors)), contentType: 'application/json' })
  }
})

// ---- §5.6 listen together

test('[ctl] listen together: started from the toolbar without running it; a phone follows (lyrics, next song); a podcast from minute 40 by range reads; the listener\'s volume is theirs; a reader is offered no action', async ({ owner, peer, browser, baseURL }, info) => {
  test.setTimeout(420_000)
  const errors: string[] = []
  const ws = await freeCanvas(owner)
  const def = await build(owner, ws, 'listen-together')
  const podcast = media('podcast-1h.webm')
  const files: Record<string, Buffer> = { 'a song.webm': media('a song.webm'), 'a song.lrc': media('a song.lrc'), 'b song.webm': media('b song.webm'), 'podcast.webm': podcast }
  const members: Record<string, J> = {}
  const objects: Record<string, string> = {}
  for (const [name, bytes] of Object.entries(files)) { objects[name] = await upload(owner, ws, bytes); members[name] = { kind: 'file', object_id: objects[name] } }
  await accepted(owner, ws, [
    { op: 'entity.create', entity_id: 'playlist', type_id: 'buckyos.dir', parent_id: 'data', order_key: 'p', name: 'playlist', payload: { members } },
    { op: 'entity.create', entity_id: 'listen', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'a', placement: { x: 60, y: 60, w: 380, h: 320 },
      payload: { view: { type: 'html' }, def_ref: { entity_id: def }, bindings: { playlist: { entity_id: 'playlist' } } } },
  ])
  await owner.client.ok('ws.grant', { workspace_id: ws.workspace_id, subject: peer.userId, capabilities: ['read'] })

  // the owner starts it from the toolbar without running the control: one Commit writes the anchor
  const page = owner.page
  await open(owner, ws)
  await page.getByTestId('aiws-canvas-block-listen').click()
  await expect(page.locator('.aiws-html-frame')).toHaveCount(0)
  const seq = (await owner.client.info(ws)).head_seq
  await page.getByTestId('aiws-near-ext-action-start').click()
  await expect.poll(async () => (await config(owner, ws, 'listen')).anchor?.playing ?? false, { timeout: 15_000 }).toBe(true)
  expect((await owner.client.info(ws)).head_seq).toBe(seq + 1)
  await expect(page.locator('.aiws-html-frame')).toHaveCount(0)

  // a reader on the desktop is offered none of the owner's actions
  await open(peer, ws)
  await canvasMode(peer.page, 'view')
  await peer.page.getByTestId('aiws-canvas-block-listen').click()
  for (const id of ['start', 'pause', 'resume', 'next', 'shuffle']) await expect(peer.page.getByTestId(`aiws-near-ext-action-${id}`)).toHaveCount(0)

  const { phone, context } = await onPhone(peer, browser, baseURL!, errors)
  // the stream addresses (`/kapi/aiworkspace/stream/<token>`): the podcast's are the ones whose Content-Range names its size
  const ranges: { url: string; range: string | null; status: number; contentRange: string | null; length: string | null }[] = []
  context.on('response', (response) => {
    if (!response.url().includes('/kapi/aiworkspace/stream/')) return
    const headers = response.headers()
    ranges.push({ url: response.url().replace(/stream\/.*/, 'stream/…'), range: response.request().headers().range ?? null, status: response.status(), contentRange: headers['content-range'] ?? null, length: headers['content-length'] ?? null })
  })
  try {
    // the member taps it on the phone: it plays where the broadcast is, with the lyrics
    await open(phone, ws)
    await expect(phone.page.getByTestId('aiws-html-start-listen')).toBeAttached({ timeout: 30_000 })
    await phone.page.getByTestId('aiws-canvas-block-listen').tap()
    const frame = phone.page.frameLocator('.aiws-html-frame')
    await expect(frame.locator('#track')).toHaveText('a song.webm', { timeout: 30_000 })
    await expect(frame.locator('#lyrics li')).toHaveCount(3)
    if (await frame.locator('#tap').isVisible().catch(() => false)) await frame.locator('#tap').tap()
    const position = () => frame.locator('#audio').evaluate((a: HTMLAudioElement) => a.currentTime)
    const live = async () => {
      const a = (await config(owner, ws, 'listen')).anchor
      return (a.pos_ms + (a.playing ? (await serverNow(owner)) - a.at_ms : 0)) / 1000
    }
    await expect.poll(async () => Math.abs(await position() - await live()), { timeout: 20_000 }).toBeLessThan(1)

    // the owner skips to the next song and pauses: the running player follows (H1)
    await page.getByTestId('aiws-near-ext-action-next').click()
    await expect(frame.locator('#track')).toHaveText('b song.webm', { timeout: 20_000 })
    await expect.poll(async () => Math.abs(await position() - await live()), { timeout: 20_000 }).toBeLessThan(1)
    await page.getByTestId('aiws-near-ext-action-pause').click()
    await expect.poll(() => frame.locator('#audio').evaluate((a: HTMLAudioElement) => a.paused), { timeout: 15_000 }).toBe(true)
    await page.getByTestId('aiws-near-ext-action-resume').click()
    await expect.poll(() => frame.locator('#audio').evaluate((a: HTMLAudioElement) => !a.paused), { timeout: 15_000 }).toBe(true)

    // the podcast from minute 40 (H6): the phone plays there without reading the hour from its start
    const read = await readEntity(owner, ws, 'listen')
    const revs = read.content.key_revs
    await accepted(owner, ws, [{ op: 'entity.set_props', entity_id: 'listen', key: 'config', props: { track: 'podcast.webm', anchor: { pos_ms: 40 * 60_000, at_ms: await serverNow(owner), playing: true } },
      expect: { track: revs['p:config.track'] ?? 0, anchor: revs['p:config.anchor'] ?? 0 } }])
    const asked = Date.now()
    await expect(frame.locator('#track')).toHaveText('podcast.webm', { timeout: 20_000 })
    await expect.poll(async () => Math.abs(await position() - await live()), { timeout: 30_000 }).toBeLessThan(1)
    const seekMs = Date.now() - asked
    const served = ranges.filter((r) => r.contentRange?.endsWith(`/${podcast.length}`) || (r.status === 200 && Number(r.length) === podcast.length))
    const bytes = served.reduce((sum, r) => sum + Number(r.length ?? 0), 0)
    await info.attach('podcast-ranges', { body: Buffer.from(JSON.stringify({ seekMs, size: podcast.length, bytes, served, all: ranges.length })), contentType: 'application/json' })
    expect(served.length).toBeGreaterThan(0)
    expect(served.every((r) => r.status === 206)).toBe(true)
    // the reads start near minute 40, not at the first byte
    expect(served.some((r) => Number(/bytes (\d+)-/.exec(r.contentRange ?? '')?.[1] ?? 0) > podcast.length / 2)).toBe(true)

    // the member's volume is theirs (user state), kept over a reload, not the document's
    await frame.locator('#volume').fill('0.3')
    await expect.poll(async () => JSON.stringify((await phone.client.ok<J>('ws.get_user_state', { workspace_id: ws.workspace_id })).entries ?? {}), { timeout: 15_000 }).toContain('volume')
    expect(JSON.stringify(await config(owner, ws, 'listen'))).not.toContain('0.3')
    await phone.page.reload()
    await showCanvas(phone.page)
    await expect(phone.page.getByTestId('aiws-html-start-listen')).toBeAttached({ timeout: 30_000 })
    await phone.page.getByTestId('aiws-canvas-block-listen').tap()
    await expect(phone.page.frameLocator('.aiws-html-frame').locator('#volume')).toHaveValue('0.3', { timeout: 30_000 })
    await screenshot(phone.page, info, 'listen-phone')
  } finally {
    await context.close()
    await info.attach('phone-errors', { body: Buffer.from(JSON.stringify(errors)), contentType: 'application/json' })
  }
})
