/* Many speakers in a public class (多人发言 TODO T3) against the real backend: two Chromium instances, each with a fake
 * microphone playing a speech-like recording with pauses. A presenter and an audience member: the member asks to
 * speak, the presenter grants it from the people panel, the member starts in the room's mode (manual), holds "talk",
 * lets the auto gate open on speech and close in the pauses, mutes, is muted with everyone, loses the right again.
 * A small class ticked in the start panel lets whoever joins speak at once, in auto mode — and a manager can still stop
 * one of them. */

import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { devices, type Browser, type BrowserType, type Locator, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { asUser, expect, openCanvas, test, workspaceOrigin, type Api } from './fixtures'

const ALICE = 'tok-alice'
const BOB = 'tok-bob'
const FIXTURE = resolve('../aiworkspace/fixtures/presentation/commits.json')
const WAV = resolve('test-results/aiworkspace-voice/speech.wav')
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Json = any

/** 48 kHz mono 16-bit: 1.2 s of a voice-like sound (a gliding pitch with harmonics, syllables at 4 Hz), 1.3 s of silence,
 * looped by Chromium. A steady sine would be taken for noise by the browser's noise suppression. */
function speechWav(): Buffer {
  const rate = 48_000
  const n = Math.round(rate * 2.5)
  const pcm = Buffer.alloc(n * 2)
  let phase = 0
  for (let i = 0; i < n; i++) {
    const t = i / rate
    let x = 0
    if (t < 1.2) {
      const f0 = 150 + 60 * Math.sin(2 * Math.PI * 0.7 * t)
      phase += (2 * Math.PI * f0) / rate
      for (let h = 1; h <= 6; h++) x += Math.sin(phase * h) / h
      x *= 0.22 * (0.55 + 0.45 * Math.sin(2 * Math.PI * 4 * t)) * Math.min(1, t * 20, (1.2 - t) * 20)
    }
    pcm.writeInt16LE(Math.max(-32767, Math.min(32767, Math.round(x * 32767))), i * 2)
  }
  const head = Buffer.alloc(44)
  head.write('RIFF', 0); head.writeUInt32LE(36 + pcm.length, 4); head.write('WAVE', 8); head.write('fmt ', 12)
  head.writeUInt32LE(16, 16); head.writeUInt16LE(1, 20); head.writeUInt16LE(1, 22); head.writeUInt32LE(rate, 24)
  head.writeUInt32LE(rate * 2, 28); head.writeUInt16LE(2, 32); head.writeUInt16LE(16, 34); head.write('data', 36); head.writeUInt32LE(pcm.length, 40)
  return Buffer.concat([head, pcm])
}

/** A Chromium whose microphone plays the recording. */
async function speaker(chromium: BrowserType): Promise<Browser> {
  mkdirSync(dirname(WAV), { recursive: true })
  writeFileSync(WAV, speechWav())
  return chromium.launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream', `--use-file-for-fake-audio-capture=${WAV}`, '--autoplay-policy=no-user-gesture-required'] })
}

/** The presentation fixture without its guide (播放 then opens "开始放映" at once). */
async function courseware(api: Api, title: string): Promise<{ workspace_id: string; epoch: string }> {
  const ws = await api.rpc(ALICE, 'ws.create', { title }) as { workspace_id: string; epoch: string }
  const fixture = JSON.parse(readFileSync(FIXTURE, 'utf8')) as { commits: { operations: Json[] }[] }
  for (const [index, commit] of fixture.commits.entries()) {
    const operations = commit.operations.filter((op) => op.entity_id !== 'path-guide')
    if (operations.length === 0) continue
    expect((await api.commit(ALICE, ws, operations, `voice-fx/${index + 1}`)).status).toBe('accepted')
  }
  return ws
}

function roomUrl(workspaceId: string, sessionId: string, link?: string): string {
  return `${workspaceOrigin(workspaceId)}/w?session=${encodeURIComponent(sessionId)}${link ? `#k=${link}` : ''}`
}

async function open(browser: Browser, token: string, url: string): Promise<Page> {
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } })
  await context.grantPermissions(['microphone'])
  const page = await context.newPage()
  await asUser(page, token)
  await page.goto(url)
  return page
}

async function join(page: Page, name: string) {
  await expect(page.getByTestId('aiws-session-gate')).toBeVisible({ timeout: 30_000 })
  await page.getByTestId('aiws-session-name').fill(name)
  await page.getByTestId('aiws-session-join').click()
}

function person(page: Page, principal: string): Locator {
  return page.getByTestId('aiws-session-people-panel').locator(`[data-testid="aiws-session-person"][data-principal="${principal}"]`)
}

async function voiceEvents(api: Api, sid: string): Promise<Json[]> {
  const events = await api.rpc(ALICE, 'session.events', { session_id: sid, after: 0, limit: 2000 })
  return (events.events as Json[]).filter((e) => String(e.type).startsWith('voice.'))
}

test('V-01 one by one: a request, the grant in the room\'s mode, push to talk, the auto gate, mute, mute all, the right taken back (多人发言 T3)', async ({ api, playwright }) => {
  test.setTimeout(300_000)
  const [one, two] = [await speaker(playwright.chromium), await speaker(playwright.chromium)]
  const ws = await courseware(api, `voices ${Date.now()}`)
  const sid = (await api.rpc(ALICE, 'session.create', { workspace_id: ws.workspace_id, path_id: 'path-intro' })).session.session_id as string
  const at = (await api.rpc(ALICE, 'session.invite', { session_id: sid, kind: 'at' })).invite.token as string
  const presenter = await open(one, ALICE, roomUrl(ws.workspace_id, sid))
  await expect(presenter.getByTestId('aiws-session-start')).toBeVisible({ timeout: 30_000 })
  await presenter.getByTestId('aiws-session-start').click()
  // the presenter's voice: connected, in auto mode
  await presenter.getByTestId('aiws-session-voice').click()
  await expect(presenter.getByTestId('aiws-session-voice')).toHaveAttribute('aria-pressed', 'true', { timeout: 15_000 })
  await expect(presenter.getByTestId('aiws-session-voice-modes')).toHaveAttribute('data-mode', 'auto')

  const bob = await open(two, BOB, roomUrl(ws.workspace_id, sid, at))
  await join(bob, '小明')
  // listening, not speaking: asks to
  await expect(bob.getByTestId('aiws-session-listen')).toBeVisible({ timeout: 20_000 })
  await expect(bob.getByTestId('aiws-session-voice-modes')).toHaveCount(0)
  await bob.getByTestId('aiws-session-voice-request').click()
  await expect(bob.getByTestId('aiws-session-voice-request')).toHaveAttribute('aria-pressed', 'true')
  await presenter.getByTestId('aiws-session-people').click()
  await expect(person(presenter, 'bob').getByTestId('aiws-session-person-request')).toBeVisible()
  await person(presenter, 'bob').getByTestId('aiws-session-person-speak').check()
  // granted: the room's mode (manual by default), the recording said once
  await expect(bob.getByTestId('aiws-session-voice-modes')).toHaveAttribute('data-mode', 'manual', { timeout: 15_000 })
  await expect(bob.getByTestId('aiws-session-voice-note')).toContainText('录音')
  await expect(person(presenter, 'bob')).toHaveAttribute('data-speak', 'true')
  await expect(person(presenter, 'bob').getByTestId('aiws-session-person-request')).toHaveCount(0)

  // push to talk: the gate is open while it is held, and the presenter's stage names who speaks
  const level = bob.getByTestId('aiws-session-voice-level')
  await expect(level).toHaveAttribute('data-open', 'false')
  const talk = bob.getByTestId('aiws-session-voice-talk')
  await talk.hover()
  await bob.mouse.down()
  await expect(level).toHaveAttribute('data-open', 'true')
  await expect(level).toHaveAttribute('data-sending', 'true')
  await expect(presenter.getByTestId('aiws-session-speaking')).toContainText('小明', { timeout: 15_000 })
  await expect(person(presenter, 'bob')).toHaveAttribute('data-speaking', 'true')
  await bob.mouse.up()
  await expect(level).toHaveAttribute('data-open', 'false')
  await expect(level).toHaveAttribute('data-sending', 'false')

  // auto: the gate opens on the voice and closes in the pauses; the presenter sees the mode
  await bob.getByTestId('aiws-session-voice-mode-auto').click()
  await expect.poll(() => level.getAttribute('data-open'), { timeout: 10_000 }).toBe('true')
  await expect.poll(() => level.getAttribute('data-open'), { timeout: 10_000 }).toBe('false')
  await expect(person(presenter, 'bob').getByTestId('aiws-session-person-mode')).toHaveAttribute('data-mode', 'auto')

  // mute: the microphone goes off, the voice stays
  await bob.getByTestId('aiws-session-voice-mode-mute').click()
  await expect(bob.getByTestId('aiws-session-voice-modes')).toHaveAttribute('data-mode', 'mute')
  await expect(level).toHaveCount(0)
  await expect(bob.getByTestId('aiws-session-listen')).toBeVisible()

  // mute all: everyone but the presenter, the right stays
  await bob.getByTestId('aiws-session-voice-mode-auto').click()
  await expect(level).toBeVisible()
  await presenter.getByTestId('aiws-session-mute-all').click()
  await expect(bob.getByTestId('aiws-session-voice-modes')).toHaveAttribute('data-mode', 'mute', { timeout: 15_000 })
  await expect(bob.getByTestId('aiws-session-voice-note')).toBeVisible()
  await expect(presenter.getByTestId('aiws-session-voice-modes')).toHaveAttribute('data-mode', 'auto')

  // the right taken back: listening again, asking again possible
  await person(presenter, 'bob').getByTestId('aiws-session-person-speak').uncheck()
  await expect(bob.getByTestId('aiws-session-voice-modes')).toHaveCount(0, { timeout: 15_000 })
  await expect(bob.getByTestId('aiws-session-voice-request')).toBeVisible()
  await expect(bob.getByTestId('aiws-session-listen')).toBeVisible()

  const kinds = (await voiceEvents(api, sid)).map((e) => `${e.type}:${e.data.principal ?? e.actor ?? ''}`)
  for (const k of ['voice.start:alice', 'voice.request:bob', 'voice.allow:bob', 'voice.mode:bob', 'voice.mute_all:alice', 'voice.speaking:bob', 'voice.speaking:alice']) expect(kinds).toContain(k)
  expect(kinds.filter((k) => k === 'voice.allow:bob')).toHaveLength(2)
  // out of the mix with his right: the host decoded his voice, and the browser's packets carried their audio level (V4)
  await expect.poll(async () => (await voiceEvents(api, sid)).find((e) => e.type === 'voice.mix' && e.data.principal === 'bob')?.data ?? null).not.toBeNull()
  const mix = (await voiceEvents(api, sid)).find((e) => e.type === 'voice.mix' && e.data.principal === 'bob').data
  expect(mix.decoded).toBeGreaterThan(20)
  expect(mix.with_level).toBeGreaterThan(20)
  await one.close()
  await two.close()
})

test('V-02 a small class ticked in the start panel: whoever joins speaks at once in auto mode; one of them can still be stopped (多人发言 §2 裁决 5)', async ({ page, api, playwright, browser }) => {
  test.setTimeout(240_000)
  const ws = await courseware(api, `small class ${Date.now()}`)
  await openCanvas(page, ALICE, ws.workspace_id)
  await page.getByTestId('aiws-collab-play').click()
  await page.getByTestId('aiws-start-kind-session').click()
  const panel = page.getByTestId('aiws-session-create')
  await panel.getByTestId('aiws-session-create-small').check()
  await expect(panel.getByTestId('aiws-session-create-speak')).toHaveValue('all')
  await expect(panel.getByTestId('aiws-session-create-speak-mode')).toHaveValue('auto')
  await panel.getByTestId('aiws-session-create-title').fill('小班课')
  const [room] = await Promise.all([page.context().waitForEvent('page'), panel.getByTestId('aiws-session-create-go').click()])
  await expect.poll(() => room.url()).toContain('session=')
  await room.close()
  const session = ((await api.rpc(ALICE, 'session.list', { workspace_id: ws.workspace_id })).sessions as Json[]).find((s) => s.title === '小班课')
  expect(session.policy.speak).toBe('all')
  expect(session.policy.speak_mode).toBe('auto')
  const sid = session.session_id as string
  const at = (await api.rpc(ALICE, 'session.invite', { session_id: sid, kind: 'at' })).invite.token as string
  const presenter = await open(browser, ALICE, roomUrl(ws.workspace_id, sid))
  await expect(presenter.getByTestId('aiws-session-start')).toBeVisible({ timeout: 30_000 })
  await presenter.getByTestId('aiws-session-start').click()

  const media = await speaker(playwright.chromium)
  const bob = await open(media, BOB, roomUrl(ws.workspace_id, sid, at))
  await join(bob, '小红')
  await expect(bob.getByTestId('aiws-session-voice-modes')).toHaveAttribute('data-mode', 'auto', { timeout: 20_000 })
  await expect(bob.getByTestId('aiws-session-voice-note')).toContainText('所有人都可以发言')
  await expect.poll(() => bob.getByTestId('aiws-session-voice-level').getAttribute('data-open'), { timeout: 10_000 }).toBe('true')
  await expect(presenter.getByTestId('aiws-session-speaking')).toContainText('小红', { timeout: 15_000 })
  // nothing special about it once it runs: the presenter stops this one person
  await presenter.getByTestId('aiws-session-people').click()
  await expect(presenter.getByTestId('aiws-session-room-speak')).toBeChecked()
  await person(presenter, 'bob').getByTestId('aiws-session-person-speak').uncheck()
  await expect(bob.getByTestId('aiws-session-voice-modes')).toHaveCount(0, { timeout: 15_000 })
  await expect(presenter.getByTestId('aiws-session-speaking')).toHaveCount(0, { timeout: 15_000 })
  // the replay shows when she spoke (Q8)
  await api.rpc(ALICE, 'session.end', { session_id: sid })
  await expect(presenter.getByTestId('aiws-session-timeline')).toBeVisible({ timeout: 30_000 })
  expect(await presenter.getByTestId('aiws-session-speaking-span').count()).toBeGreaterThan(0)
  await media.close()
})

test('V-03 a phone in a small class: the voice folds into one button, listening works, speaking is suggested from a computer (多人发言 §2 裁决 4, C10)', async ({ api, playwright }) => {
  test.setTimeout(180_000)
  const ws = await courseware(api, `phone ${Date.now()}`)
  const sid = (await api.rpc(ALICE, 'session.create', { workspace_id: ws.workspace_id, path_id: 'path-intro', policy: { speak: 'all', speak_mode: 'auto' } })).session.session_id as string
  const at = (await api.rpc(ALICE, 'session.invite', { session_id: sid, kind: 'at' })).invite.token as string
  await api.rpc(ALICE, 'session.start', { session_id: sid })
  const media = await speaker(playwright.chromium)
  // eslint-disable-next-line @typescript-eslint/no-unused-vars -- the browser is the speaker's, the rest is the phone
  const { defaultBrowserType, ...pixel } = devices['Pixel 7']
  const context = await media.newContext(pixel)
  await context.grantPermissions(['microphone'])
  const phone = await context.newPage()
  await asUser(phone, BOB)
  await phone.goto(roomUrl(ws.workspace_id, sid, at))
  await join(phone, '手机')
  await expect(phone.getByTestId('aiws-session-voice-note')).toContainText('建议用电脑发言', { timeout: 20_000 })
  await expect(phone.getByTestId('aiws-session-voice-modes')).toHaveCount(0)
  await phone.getByTestId('aiws-session-voice-menu').click()
  const pop = phone.getByTestId('aiws-session-voice-pop')
  await expect(pop.getByTestId('aiws-session-voice-modes')).toHaveAttribute('data-mode', 'auto')
  await expect(pop.getByTestId('aiws-session-listen')).toBeVisible()
  // the bar keeps its place for the stage and the reactions
  await expect(phone.getByTestId('aiws-session-symbol-👍')).toBeVisible()
  await media.close()
})
