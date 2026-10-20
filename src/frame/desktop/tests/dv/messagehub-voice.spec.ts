/**
 * Voice notes and emoji on a real Zone (UI_DATAMODEL §3.5.7).
 *
 *   BUCKYOS_TEST_ZONE_HOST=test.buckyos.io npx playwright test --config=playwright.dv.config.ts tests/dv/messagehub-voice.spec.ts
 *
 * Signs in as `bob` (password `BUCKYOS_TEST_USER_PASSWORD`, default `buckyos2026`) with Chromium's fake
 * capture device as the microphone (a beeping tone), records a voice note to `alice` and checks what
 * msg-center stored: the `voice` extension, an audio file that is not silent, readable by both sides.
 * Alice then plays it in her MessageHub. The messages stay in the conversation.
 */

import { expect, test, type Browser, type BrowserContext, type Page } from '@playwright/test'

const zone = process.env.BUCKYOS_TEST_ZONE_HOST || 'test.buckyos.io'
const origin = `https://${zone}`
const sender = process.env.BUCKYOS_TEST_VOICE_USER || 'bob'
const peer = process.env.BUCKYOS_TEST_VOICE_PEER || 'alice'
const password = process.env.BUCKYOS_TEST_USER_PASSWORD || 'buckyos2026'
const didOf = (user: string) => `did:web:${user}.${zone}`

test.use({
  launchOptions: { args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] },
})

async function login(browser: Browser, userId: string): Promise<{ context: BrowserContext; page: Page; token: string }> {
  const context = await browser.newContext({ ignoreHTTPSErrors: true, viewport: { width: 1280, height: 900 }, permissions: ['microphone'] })
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

async function openConversation(page: Page, peerDid: string) {
  await page.goto(`${origin}/messagehub?entityId=${encodeURIComponent(peerDid)}`)
  await expect(page.getByTestId('message-composer')).toBeVisible({ timeout: 30_000 })
  return page.getByTestId('conversation-history')
}

/** Peak sample of an audio file decoded by the page (0 = digital silence). */
async function peakOf(page: Page, url: string, token: string): Promise<{ bytes: number; seconds: number; peak: number }> {
  return page.evaluate(async ({ url, token }) => {
    const response = await fetch(url, { headers: { Authorization: `Bearer ${token}` } })
    if (!response.ok) throw new Error(`content ${response.status}`)
    const data = await response.arrayBuffer()
    const audio = await new OfflineAudioContext(1, 48_000, 48_000).decodeAudioData(data.slice(0))
    let peak = 0
    for (const value of audio.getChannelData(0)) peak = Math.max(peak, Math.abs(value))
    return { bytes: data.byteLength, seconds: audio.duration, peak }
  }, { url, token })
}

test('a held microphone sends a voice note the peer can play', async ({ browser }) => {
  const me = await login(browser, sender)
  const history = await openConversation(me.page, didOf(peer))
  const before = await history.getByTestId('message-voice').count()

  const mic = me.page.getByTestId('voice-record')
  await expect(mic).toBeVisible()
  const box = (await mic.boundingBox())!
  await me.page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
  await me.page.mouse.down()
  await expect(me.page.getByTestId('voice-recording')).toHaveAttribute('data-phase', 'recording')
  await me.page.waitForTimeout(2600)
  await me.page.mouse.up()

  const bubble = history.getByTestId('message-voice').nth(before)
  await expect(bubble).toBeVisible({ timeout: 30_000 })
  await expect(bubble.getByTestId('delivery-status')).not.toHaveAttribute('data-status', 'sending', { timeout: 60_000 })
  const msgId = await bubble.getAttribute('data-message-id')
  expect(msgId).toMatch(/^cymsg:/)

  // What msg-center stored: the voice extension and a non-silent audio file both sides can read.
  const message = await me.page.evaluate(async ({ origin, token, owner, msgId }) => {
    const response = await fetch(`${origin}/kapi/msg-center`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', Authorization: `Bearer ${token}` },
      body: JSON.stringify({ method: 'msg.list_sessions', params: { owner, limit: 50, with_object: true, order_by: 'activity' }, sys: [1, token] }),
    })
    const body = await response.json() as { result?: { items?: Array<{ last_record?: { record: { msg_id: string }; msg?: unknown } }> }; error?: unknown }
    if (!body.result) throw new Error(`msg.list_sessions: ${JSON.stringify(body.error ?? body)}`)
    return body.result.items?.find(item => item.last_record?.record.msg_id === msgId)?.last_record?.msg ?? null
  }, { origin, token: me.token, owner: didOf(sender), msgId: msgId! }) as { voice?: { duration_ms: number; waveform: number[] }; content: { refs: Array<{ label: string; target: { obj_id: string } }> } } | null
  if (!message) throw new Error(`${msgId} is not the last record of any of ${sender}'s sessions`)
  console.log(`stored voice: ${JSON.stringify(message.voice)}`)
  expect(message.voice?.duration_ms ?? 0).toBeGreaterThan(2000)
  expect(Math.max(...(message.voice?.waveform ?? [0]))).toBe(31)
  const fileId = message.content.refs[0].target.obj_id
  expect(message.content.refs[0].label).toMatch(/^voice-\d{8}-\d{6}\.(webm|ogg|m4a)$/)
  const mine = await peakOf(me.page, `${origin}/kapi/msg-center/objects/${fileId}/content`, me.token)
  console.log(`sender reads ${fileId}: ${JSON.stringify(mine)}`)
  expect(mine.peak).toBeGreaterThan(0.01)
  await me.context.close()

  const them = await login(browser, peer)
  const theirs = await peakOf(them.page, `${origin}/kapi/msg-center/objects/${fileId}/content`, them.token)
  expect(theirs).toEqual(mine)
  const theirHistory = await openConversation(them.page, didOf(sender))
  const received = theirHistory.locator(`[data-testid="message-voice"][data-message-id="${msgId}"]`)
  await expect(received).toBeVisible({ timeout: 30_000 })
  await expect(received.getByTestId('voice-unplayed')).toBeVisible()
  await received.getByTestId('voice-play').click()
  await expect(received.getByTestId('voice-note')).toHaveAttribute('data-state', 'playing', { timeout: 30_000 })
  await expect(received.getByTestId('voice-unplayed')).toHaveCount(0)
  await them.page.screenshot({ path: 'test-results/dv-messagehub-voice-received.png' })
  await expect(received.getByTestId('voice-note')).toHaveAttribute('data-state', 'idle', { timeout: 15_000 })
  await them.context.close()
})

test('a microphone that delivers only silence is flagged and nothing reaches the peer', async ({ browser }) => {
  const me = await login(browser, sender)
  // A muted input, a closed laptop lid or a virtual default device: a live track of exact zeros.
  await me.context.addInitScript(() => {
    navigator.mediaDevices.getUserMedia = async () => {
      const context = new AudioContext()
      const gain = context.createGain()
      gain.gain.value = 0
      const destination = context.createMediaStreamDestination()
      const oscillator = context.createOscillator()
      oscillator.connect(gain).connect(destination)
      oscillator.start()
      return destination.stream
    }
  })
  const history = await openConversation(me.page, didOf(peer))
  await expect(me.page.getByTestId('voice-record')).toBeVisible()
  const before = await history.getByTestId('message-voice').count()
  const box = (await me.page.getByTestId('voice-record').boundingBox())!
  await me.page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
  await me.page.mouse.down()
  await expect(me.page.getByTestId('voice-no-signal')).toBeVisible({ timeout: 5_000 })
  await me.page.mouse.up()
  await expect(me.page.getByTestId('voice-error')).toContainText('so the recording was not sent')
  await me.page.screenshot({ path: 'test-results/dv-messagehub-voice-silent.png' })
  await me.page.waitForTimeout(3000)
  await expect(history.getByTestId('message-voice')).toHaveCount(before)
  await me.context.close()
})
