/* The 控件机制 examples (doc/workspace/control-examples; notepads/aiworkspace-control-mechanism-todo.md §5.3–§5.6), built
 * with `ext.build` as they are in the repository and used in the Desktop: the TV screen plays where its anchor says by
 * the service's clock (also on a device whose clock is 30 s behind) and follows a pause; the time-limited control hides
 * its Blocks for the viewer once their time is up (also after a reload) and shows them marked in design mode; a photo
 * vote prepared from the toolbar takes one choice per person, at most three, the cover showing the most-voted photo;
 * listen together starts from the toolbar without running the control, and a reader is offered none of the actions. */

import { deflateSync } from 'node:zlib'
import { mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { Page } from '@playwright/test'
import { openCanvas, setCanvasMode, test, expect, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type J = any
type Ws = { workspace_id: string; epoch: string }

/** A 16-bit mono WAV of a tone. */
function wav(seconds: number, rate = 8000): Buffer {
  const n = Math.round(seconds * rate)
  const b = Buffer.alloc(44 + n * 2)
  b.write('RIFF', 0); b.writeUInt32LE(36 + n * 2, 4); b.write('WAVEfmt ', 8); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22)
  b.writeUInt32LE(rate, 24); b.writeUInt32LE(rate * 2, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34); b.write('data', 36); b.writeUInt32LE(n * 2, 40)
  for (let i = 0; i < n; i++) b.writeInt16LE(Math.round(Math.sin((i / rate) * 2 * Math.PI * 440) * 6000), 44 + i * 2)
  return b
}

const CRC = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0 })
function crc32(b: Buffer): number { let c = 0xffffffff; for (const x of b) c = CRC[(c ^ x) & 0xff] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0 }
function chunk(type: string, data: Buffer): Buffer {
  const len = Buffer.alloc(4); len.writeUInt32BE(data.length)
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data])
  const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(body))
  return Buffer.concat([len, body, crc])
}
/** A small PNG of one colour. */
function png(r: number, g: number, b: number, w = 24, h = 16): Buffer {
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w, 0); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2
  const rows = Buffer.alloc((w * 3 + 1) * h)
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) rows.set([r, g, b], y * (w * 3 + 1) + 1 + x * 3)
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(rows)), chunk('IEND', Buffer.alloc(0))])
}

const serverNow = async (api: Api): Promise<number> => (await api.rpc(ALICE, 'ws.time', {})).server_ms
const config = async (api: Api, ws: Ws, id: string, token = ALICE) => (await api.read(token, ws.workspace_id, id)).content.payload.config ?? {}

async function exampleWorkspace(api: Api, name: string, title: string): Promise<{ ws: Ws; def: string }> {
  const ws = await api.freeCanvas(ALICE, `${title} ${Date.now()}`)
  const built = await api.build(ALICE, ws.workspace_id, `control-examples/controls/${name}`)
  return { ws, def: built.def_entity_id }
}

const composite = (id: string, def: string, extra: J = {}) => ({ op: 'entity.create', entity_id: id, type_id: 'buckyos.container', parent_id: 'free', order_key: 'a',
  placement: { x: 60, y: 60, w: 'auto', h: 'auto' }, payload: { kind: 'composite', def_ref: { entity_id: def }, ...extra } })

/** A page whose device clock runs `skewMs` off. */
async function skewed(page: Page, skewMs: number) {
  await page.addInitScript((skew) => {
    const now = Date.now.bind(Date)
    Date.now = () => now() + skew
  }, skewMs)
}

test('CE01 TV screen: an empty screen takes a sound from the picker; viewers play where the broadcast is, by the service clock, and pause with it', async ({ page, browser, api }) => {
  const { ws, def } = await exampleWorkspace(api, 'tv-screen', 'tv-e2e')
  expect((await api.commit(ALICE, ws, [composite('tv', def)])).status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  // the empty screen: who may change it chooses the video or sound (the system picker; this device is a choice in it)
  await page.getByTestId('aiws-broadcast-choose-tv').click()
  const dir = mkdtempSync(join(tmpdir(), 'aiws-tv-'))
  writeFileSync(join(dir, 'tone.wav'), wav(12))
  await page.getByTestId('select-local-input').setInputFiles(join(dir, 'tone.wav'))
  let assetCell = ''
  await expect.poll(async () => {
    const cell = (await api.outline(ALICE, ws.workspace_id)).find((e: J) => e.parent_id === 'tv' && e.slot === 'screen' && !e.deleted)
    assetCell = cell?.entity_id ?? ''
    return Boolean(cell)
  }, { timeout: 30_000 }).toBe(true)
  // the asset, its Block and a fresh anchor came in one Commit: from 0, paused
  expect((await config(api, ws, 'tv')).broadcast).toMatchObject({ pos_ms: 0, playing: false })
  const player = page.locator(`[data-testid="aiws-broadcast-${(await api.outline(ALICE, ws.workspace_id)).find((e: J) => e.entity_id === assetCell).source_id}"]`)
  await expect(player).toBeAttached({ timeout: 15_000 })

  // started 3 s ago at 2 s: a viewer whose device clock is 30 s behind plays at about 5 s
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read'] })
  const at = await serverNow(api)
  const read = await api.read(ALICE, ws.workspace_id, 'tv')
  expect((await api.commit(ALICE, ws, [{ op: 'entity.set_props', entity_id: 'tv', key: 'config', props: { broadcast: { pos_ms: 2000, at_ms: at - 3000, playing: true } }, expect: { broadcast: read.content.key_revs['p:config.broadcast'] ?? 0 } }])).status).toBe('accepted')
  const bobContext = await browser.newContext({ viewport: { width: 1400, height: 900 } })
  const bob = await bobContext.newPage()
  await skewed(bob, -30_000)
  await openCanvas(bob, BOB, ws.workspace_id)
  const bobPlayer = bob.locator('.aiws-broadcast audio')
  await expect(bobPlayer).toBeAttached({ timeout: 15_000 })
  // a reader is not offered the broadcast controls
  await expect(bob.getByTestId('aiws-broadcast-start')).toHaveCount(0)
  await expect(bob.getByTestId('aiws-broadcast-pause')).toHaveCount(0)
  const expected = async () => (2000 + (await serverNow(api)) - (at - 3000)) / 1000
  await expect.poll(async () => Math.abs(await bobPlayer.evaluate((a: HTMLAudioElement) => a.currentTime) - await expected()), { timeout: 15_000 }).toBeLessThan(1)

  // the owner pauses at 7 s: the viewer stops there
  const paused = await api.read(ALICE, ws.workspace_id, 'tv')
  expect((await api.commit(ALICE, ws, [{ op: 'entity.set_props', entity_id: 'tv', key: 'config', props: { broadcast: { pos_ms: 7000, at_ms: await serverNow(api), playing: false } }, expect: { broadcast: paused.content.key_revs['p:config.broadcast'] } }])).status).toBe('accepted')
  await expect.poll(() => bobPlayer.evaluate((a: HTMLAudioElement) => a.paused && Math.abs(a.currentTime - 7) < 0.3), { timeout: 15_000 }).toBe(true)
  // a reader cannot change the anchor through the API either
  const refused = await api.commit(BOB, ws, [{ op: 'entity.set_props', entity_id: 'tv', key: 'config', props: { broadcast: { pos_ms: 0, at_ms: 0, playing: true } }, expect: { broadcast: paused.content.key_revs['p:config.broadcast'] + 1 } }])
  expect(refused.status).not.toBe('accepted')
  await bobContext.close()
})

test('CE02 time-limited: the viewer sees the time left, then the Blocks are hidden for them (also after a reload); design mode shows them marked', async ({ page, api }) => {
  const { ws, def } = await exampleWorkspace(api, 'time-limited', 'limited-e2e')
  const label = (id: string, text: string, key: string) => ({ op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: 'tl', order_key: key,
    placement: { x: 'auto', y: 'auto', w: 160, h: 32 }, payload: { view: { type: 'label' }, title: text, slot: 'content' } })
  expect((await api.commit(ALICE, ws, [composite('tl', def, { config: { limit_ms: 6000 } }), label('l1', 'Secret one', 'a'), label('l2', 'Secret two', 'b')])).status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  // design mode: everything shows, marked
  await expect(page.getByTestId('aiws-composite-limited-tl')).toBeVisible()
  await expect(page.getByTestId('aiws-canvas-block-l1')).toBeVisible()
  await setCanvasMode(page, 'view')
  await expect(page.getByTestId('aiws-canvas-block-l1')).toBeVisible()
  // the time left counts down (m:ss), then the Blocks go for this viewer
  await expect(page.locator('[data-proj^="tl.clock~left~"]')).toContainText(/0:0[1-6]/, { timeout: 10_000 })
  await expect(page.getByTestId('aiws-canvas-block-l1')).toBeHidden({ timeout: 15_000 })
  await expect(page.getByTestId('aiws-canvas-block-l2')).toBeHidden()
  await expect(page.locator('[data-proj^="tl.clock~gone~"]')).toBeVisible()
  // their first opening is theirs (user state): a reload keeps them hidden
  await openCanvas(page, ALICE, ws.workspace_id)
  await setCanvasMode(page, 'view')
  await expect(page.locator('[data-proj^="tl.clock~gone~"]')).toBeVisible({ timeout: 15_000 })
  await expect(page.getByTestId('aiws-canvas-block-l1')).toBeHidden()
  // the shared document did not change
  expect((await api.read(ALICE, ws.workspace_id, 'l1')).content.payload.title).toBe('Secret one')
  await setCanvasMode(page, 'edit')
  await expect(page.getByTestId('aiws-canvas-block-l1')).toBeVisible()
})

test('CE03 photo vote: the ballot prepared from the toolbar; one choice per person, at most three; the cover shows the favourite; another\'s choice is refused', async ({ page, browser, api }) => {
  const { ws, def } = await exampleWorkspace(api, 'photo-vote', 'vote-e2e')
  const colours: [number, number, number][] = [[220, 60, 60], [60, 160, 60], [60, 90, 220], [230, 180, 40], [150, 60, 200], [40, 190, 190], [240, 120, 40], [120, 120, 120]]
  const members: Record<string, J> = {}
  for (const [i, c] of colours.entries()) members[`Photo ${i + 1}.png`] = { kind: 'file', object_id: (await api.uploadAsset(ALICE, ws.workspace_id, png(...c))).object_id }
  expect((await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'photos', type_id: 'buckyos.dir', parent_id: 'data', order_key: 'p', name: 'photos', payload: { members } },
    { op: 'entity.create', entity_id: 'votes', type_id: 'buckyos.table-source', parent_id: 'data', order_key: 'v', name: 'votes', payload: { fields: [
      { field_id: 'voter', name: 'voter', type: 'text', submitter: true },
      { field_id: 'picks', name: 'picks', type: 'multi_select', options: [], max_items: 3 },
    ] } },
    composite('vote', def, { bindings: { photos: { entity_id: 'photos' }, votes: { entity_id: 'votes' } } }),
  ])).status).toBe('accepted')
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read'] })
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', scope_entity_id: 'votes', capabilities: ['read', 'append'] })

  await openCanvas(page, ALICE, ws.workspace_id)
  await expect(page.locator('[data-proj^="vote.photos~img~"]')).toHaveCount(8, { timeout: 15_000 })
  // the declared action, on the toolbar of the selected control: an option per photo
  const title = await page.getByTestId('aiws-composite-title-vote').boundingBox()
  await page.mouse.click(title!.x + 20, title!.y + title!.height / 2)
  await page.getByTestId('aiws-near-ext-action-prepare').click()
  await expect.poll(async () => (await api.read(ALICE, ws.workspace_id, 'votes')).content.fields.find((f: J) => f.field_id === 'picks').options.length, { timeout: 15_000 }).toBe(8)

  // view mode: a tap picks a photo (the composite's default handler toggles the caller's own record)
  await setCanvasMode(page, 'view')
  const pick = (p: Page, n: number) => p.locator(`[data-proj="vote.photos~img~Photo ${n}.png"]`).click()
  for (const n of [1, 2, 3]) await pick(page, n)
  const rows = async (token = ALICE) => (await api.rpc(token, 'doc.query', { workspace_id: ws.workspace_id, source_id: 'votes', limit: 50 })).rows as J[]
  await expect.poll(async () => (await rows())[0]?.values.picks?.length ?? 0, { timeout: 15_000 }).toBe(3)
  expect((await rows())[0].values.voter).toBe('alice')
  // a fourth is refused (the client says so; the kernel enforces max_items)
  await pick(page, 4)
  await expect(page.getByTestId('aiws-notice').filter({ hasText: '3' }).last()).toBeVisible()
  expect((await rows())[0].values.picks).toHaveLength(3)
  await expect(page.locator('[data-proj="vote.photos~ring~Photo 1.png"]')).toBeVisible()

  // bob picks on his own record
  const bobContext = await browser.newContext({ viewport: { width: 1400, height: 900 } })
  const bob = await bobContext.newPage()
  await openCanvas(bob, BOB, ws.workspace_id)
  await setCanvasMode(bob, 'view')
  await expect(bob.locator('[data-proj^="vote.photos~img~"]')).toHaveCount(8, { timeout: 15_000 })
  for (const n of [1, 5]) await pick(bob, n)
  await expect.poll(async () => (await rows()).length, { timeout: 15_000 }).toBe(2)
  // the cover: Photo 1 has two votes, first on every screen
  for (const p of [page, bob]) await expect(p.locator('[data-proj^="vote.cover~img~"]').first()).toHaveAttribute('data-item', 'Photo 1.png', { timeout: 15_000 })
  await expect(page.locator('[data-proj="vote.cover~count~Photo 1.png"]')).toContainText('2')

  // through the API: bob changes alice's record, writes the submitter, picks four — all refused
  const aliceRow = (await rows()).find((r) => r.values.voter === 'alice')
  const opt = (await api.read(ALICE, ws.workspace_id, 'votes')).content.fields.find((f: J) => f.field_id === 'picks').options.map((o: J) => o.option_id)
  const tries = [
    { op: 'table.set_values', source_id: 'votes', values: [{ record_id: aliceRow.record_id, field_id: 'picks', value: [opt[7]], expect: { rev: aliceRow.revs.picks } }] },
    { op: 'table.insert_records', source_id: 'votes', records: [{ record_id: 'forged', values: { voter: 'carol', picks: [opt[6]] } }] },
    { op: 'table.insert_records', source_id: 'votes', records: [{ record_id: 'four', values: { picks: opt.slice(0, 4) } }] },
  ]
  for (const op of tries) expect((await api.commit(BOB, ws, [op])).status, JSON.stringify(op)).not.toBe('accepted')
  await bobContext.close()
})

test('CE04 listen together: started from the toolbar without running it, a listener follows; a reader is offered no action', async ({ page, browser, api }) => {
  const { ws, def } = await exampleWorkspace(api, 'listen-together', 'listen-e2e')
  const song = (await api.uploadAsset(ALICE, ws.workspace_id, wav(90))).object_id
  const lrc = (await api.uploadAsset(ALICE, ws.workspace_id, new TextEncoder().encode('[00:00.00]First line\n[00:03.00]Second line\n[00:06.00]Third line\n'))).object_id
  const second = (await api.uploadAsset(ALICE, ws.workspace_id, wav(8))).object_id
  expect((await api.commit(ALICE, ws, [
    { op: 'entity.create', entity_id: 'playlist', type_id: 'buckyos.dir', parent_id: 'data', order_key: 'p', name: 'playlist', payload: { members: {
      'a song.wav': { kind: 'file', object_id: song }, 'a song.lrc': { kind: 'file', object_id: lrc }, 'b song.wav': { kind: 'file', object_id: second } } } },
    { op: 'entity.create', entity_id: 'listen', type_id: 'buckyos.cell', parent_id: 'free', order_key: 'a', placement: { x: 60, y: 60, w: 380, h: 320 },
      payload: { view: { type: 'html' }, def_ref: { entity_id: def }, bindings: { playlist: { entity_id: 'playlist' } } } },
  ])).status).toBe('accepted')
  await api.rpc(ALICE, 'ws.grant', { workspace_id: ws.workspace_id, subject: 'bob', capabilities: ['read'] })

  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-canvas-block-listen').click()
  // the owner starts without running the control: the host writes the anchor (one Commit)
  await expect(page.locator('.aiws-html-frame')).toHaveCount(0)
  await page.getByTestId('aiws-near-ext-action-start').click()
  await expect.poll(async () => (await config(api, ws, 'listen')).anchor?.playing ?? false, { timeout: 15_000 }).toBe(true)
  await expect(page.locator('.aiws-html-frame')).toHaveCount(0)

  // a reader: no owner action on the toolbar; started, the player follows the broadcast and shows the lyrics
  const bobContext = await browser.newContext({ viewport: { width: 1400, height: 900 } })
  const bob = await bobContext.newPage()
  await openCanvas(bob, BOB, ws.workspace_id)
  await setCanvasMode(bob, 'view')
  await bob.getByTestId('aiws-canvas-block-listen').click()
  await expect(bob.getByTestId('aiws-near-ext-action-start')).toHaveCount(0)
  await expect(bob.getByTestId('aiws-near-ext-action-next')).toHaveCount(0)
  // in view mode the tap that selected it started it
  const frame = bob.frameLocator('.aiws-html-frame')
  await expect(frame.locator('#track')).toHaveText('a song.wav', { timeout: 15_000 })
  await expect(frame.locator('#lyrics li')).toHaveCount(3)
  const anchor = (await config(api, ws, 'listen')).anchor
  const position = () => frame.locator('#audio').evaluate((a: HTMLAudioElement) => a.currentTime)
  await expect.poll(async () => Math.abs(await position() - (anchor.pos_ms + (await serverNow(api)) - anchor.at_ms) / 1000), { timeout: 15_000 }).toBeLessThan(1)

  // the owner skips to the next song: the running player follows at once (H1), from 0
  await page.getByTestId('aiws-near-ext-action-next').click()
  await expect.poll(async () => (await config(api, ws, 'listen')).track, { timeout: 20_000 }).toBe('b song.wav')
  await expect(frame.locator('#track')).toHaveText('b song.wav', { timeout: 15_000 })
  // bob's volume is his own (user state), not the document's
  await frame.locator('#volume').fill('0.3')
  await expect.poll(async () => JSON.stringify((await api.rpc(BOB, 'ws.get_user_state', { workspace_id: ws.workspace_id })).entries ?? {}), { timeout: 15_000 }).toContain('volume')
  expect(JSON.stringify(await config(api, ws, 'listen'))).not.toContain('0.3')
  await bobContext.close()
})

test('CE05 photo vote placed from the catalog: its on_bind asks which photo folder, creates its own votes table; its on_init_control prepares the ballot', async ({ page, api }) => {
  const { ws, def } = await exampleWorkspace(api, 'photo-vote', 'vote-bind-e2e')
  const folder = async (id: string, names: string[]) => {
    const members: Record<string, J> = {}
    for (const [i, n] of names.entries()) members[n] = { kind: 'file', object_id: (await api.uploadAsset(ALICE, ws.workspace_id, png(40 * i, 120, 200))).object_id }
    return { op: 'entity.create', entity_id: id, type_id: 'buckyos.dir', parent_id: 'data', order_key: id, name: id, payload: { members } }
  }
  expect((await api.commit(ALICE, ws, [await folder('trip', ['Beach.png', 'Hill.png', 'Lake.png']), await folder('party', ['Cake.png'])])).status).toBe('accepted')
  await openCanvas(page, ALICE, ws.workspace_id)
  const insert = async () => {
    await page.getByTestId('aiws-tool-catalog').click()
    await page.getByTestId(`aiws-catalog-entry-def-${def}`).click()
    await page.getByTestId('aiws-catalog-insert').click()
  }
  await insert()
  // two photo folders: asked which (no "new": a folder of photos is not created here)
  const dialog = page.getByTestId('aiws-source-chooser')
  await expect(dialog.getByTestId('aiws-source-party')).toBeVisible()
  await expect(dialog.getByTestId('aiws-source-new')).toHaveCount(0)
  await dialog.getByTestId('aiws-source-trip').click()
  const instance = async () => (await api.outline(ALICE, ws.workspace_id)).filter((e: J) => e.kind === 'composite' && e.def_id === def && !e.deleted)
  await expect.poll(async () => (await instance()).length).toBe(1)
  const [vote] = await instance()
  expect(vote.bindings.photos).toBe('trip')
  // its own votes table: a submitter field and three picks at most, owned by the control
  const votes = (await api.read(ALICE, ws.workspace_id, vote.bindings.votes)).content
  expect(votes.fields.find((f: J) => f.field_id === 'voter').submitter).toBe(true)
  expect(votes.fields.find((f: J) => f.field_id === 'picks').max_items).toBe(3)
  // …and the ballot prepared right after (the declared action): one option per photo of the folder
  await expect.poll(async () => (await api.read(ALICE, ws.workspace_id, vote.bindings.votes)).content.fields.find((f: J) => f.field_id === 'picks').options.map((o: J) => o.label).sort(), { timeout: 15_000 })
    .toEqual(['Beach.png', 'Hill.png', 'Lake.png'])
  await expect(page.locator(`[data-proj^="${vote.entity_id}.photos~img~"]`)).toHaveCount(3)

  // a second one: another votes table (when: always); cancelled: nothing placed
  await insert()
  await dialog.getByTestId('aiws-source-party').click()
  await expect.poll(async () => (await instance()).length).toBe(2)
  const tables = new Set((await instance()).map((e: J) => e.bindings.votes))
  expect(tables.size).toBe(2)
  await insert()
  await dialog.getByTestId('aiws-bind-cancel').click()
  await expect(dialog).toHaveCount(0)
  expect(await instance()).toHaveLength(2)
})
