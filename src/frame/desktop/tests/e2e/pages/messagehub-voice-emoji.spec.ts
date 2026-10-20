import { expect, test, type Page } from '@playwright/test'
import { MOCK_VOICE_NOTE } from '../../../src/app/messagehub/mock/objects'

declare global {
  interface Window { __messageHubMock: import('../../../src/app/messagehub/mock/store').MessageHubMockStore }
}

/**
 * Telegram-style voice notes and emoji in the MessageHub composer, on the
 * mock runtime. Chromium's fake capture device stands in for the microphone.
 */

const SELF = 'did:buckyos:user:self'
const CODER = 'did:buckyos:agent:codeassistant'
const ALICE = 'did:buckyos:person:alice'
const ALICE_SESSION = 'session-alice-1'
const OWN = { viewerDid: SELF, ownerDid: SELF, mode: 'self' as const }

test.use({
  launchOptions: { args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] },
  permissions: ['microphone'],
})

async function openSession(page: Page, title: string) {
  await page.goto(`/messagehub?entityId=${encodeURIComponent(CODER)}`)
  await expect(page.getByRole('button', { name: 'Sessions', exact: true })).toBeVisible()
  await page.getByRole('button', { name: 'New Session', exact: true }).last().click()
  const dialog = page.getByRole('dialog')
  await dialog.getByLabel('Title (optional)', { exact: true }).fill(title)
  await dialog.getByRole('combobox', { name: 'Connection', exact: true }).selectOption('native')
  await dialog.getByRole('button', { name: 'Create', exact: true }).click()
  await expect(dialog).toHaveCount(0)
  await expect(page.getByTestId('conversation-history')).toHaveAttribute('data-raw-count', '0')
  return page.evaluate(({ context, entity }) => window.__messageHubMock.sessions(context, entity)[0].id, { context: OWN, entity: CODER })
}

async function sentMessages(page: Page, id: string) {
  return page.evaluate(({ context, session }) => {
    const reader = window.__messageHubMock.reader(context, session)
    return reader.readRange(0, reader.totalCount)
  }, { context: OWN, session: id })
}

async function holdMic(page: Page, ms: number, move?: { dx: number; dy: number }) {
  const box = (await page.getByTestId('voice-record').boundingBox())!
  const x = box.x + box.width / 2
  const y = box.y + box.height / 2
  await page.mouse.move(x, y)
  await page.mouse.down()
  await expect(page.getByTestId('voice-recording')).toHaveAttribute('data-phase', 'recording')
  if (move) await page.mouse.move(x - move.dx, y - move.dy, { steps: 8 })
  await page.waitForTimeout(ms)
  await page.mouse.up()
}

test('emoji panel inserts at the caret, searches, remembers, and emoji-only messages show large', async ({ page }) => {
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  await openSession(page, 'Emoji')
  const textarea = page.locator('textarea')
  const history = page.getByTestId('conversation-history')

  await textarea.fill('Hi  there')
  await textarea.evaluate((element: HTMLTextAreaElement) => element.setSelectionRange(3, 3))
  await page.getByTestId('emoji-button').click()
  const panel = page.getByTestId('emoji-panel')
  await expect(panel).toBeVisible()
  await expect(panel.getByRole('button', { name: 'grinning face', exact: true })).toBeVisible()
  await panel.getByRole('group', { name: 'Recently used' }).getByRole('button', { name: 'thumbs up', exact: true }).click()
  await expect(textarea).toHaveValue('Hi 👍 there')
  await expect(textarea).toBeFocused()
  await expect(panel).toBeVisible()
  await page.screenshot({ path: 'test-results/messagehub-emoji-panel.png' })

  await page.getByTestId('emoji-tab-flags').click()
  await expect(page.getByTestId('emoji-tab-flags')).toHaveAttribute('aria-pressed', 'true')
  await page.getByTestId('emoji-search').fill('rock')
  await expect(panel.getByRole('group', { name: 'Search results' }).getByRole('button', { name: 'rocket', exact: true })).toBeVisible()
  await page.getByTestId('emoji-search').fill('zzzz')
  await expect(panel.getByText('No emoji found')).toBeVisible()
  await page.getByTestId('emoji-search').fill('')

  await textarea.press('Escape')
  await expect(panel).toHaveCount(0)
  await textarea.press('Enter')
  await expect(history.getByTestId('message-bubble').filter({ hasText: 'Hi 👍 there' })).toBeVisible()

  await page.getByTestId('emoji-button').click()
  await panel.getByRole('group', { name: 'Smileys & people' }).getByRole('button', { name: 'face with tears of joy', exact: true }).click()
  await panel.getByRole('group', { name: 'Smileys & people' }).getByRole('button', { name: 'red heart', exact: true }).click()
  await expect(textarea).toHaveValue('😂❤️')
  // The picks lead the recent list next time.
  const recent = panel.getByRole('group', { name: 'Recently used' }).getByRole('button')
  await expect(recent.first()).toHaveAccessibleName('red heart')
  await expect(recent.nth(1)).toHaveAccessibleName('face with tears of joy')
  await page.mouse.click(5, 5)
  await expect(panel).toHaveCount(0)
  await textarea.press('Enter')
  const big = history.getByTestId('message-emoji')
  await expect(big).toHaveCount(1)
  await expect(big.locator('.mh-big-emoji')).toHaveAttribute('data-count', '2')
  await expect(big.locator('.mh-big-emoji')).toHaveText('😂❤️')
  await page.screenshot({ path: 'test-results/messagehub-emoji-big.png' })
  expect(errors).toEqual([])
})

for (const scheme of ['light', 'dark'] as const) {
  test(`hold the microphone to record and release to send a voice note (${scheme})`, async ({ page }) => {
    const errors: string[] = []
    page.on('pageerror', error => errors.push(error.message))
    await page.addInitScript(value => window.localStorage.setItem('buckyos.prototype.theme.v1', value), scheme)
    const id = await openSession(page, `Voice ${scheme}`)
    const history = page.getByTestId('conversation-history')
    // An empty input offers the microphone; typing turns it back into send.
    await expect(page.getByTestId('voice-record')).toBeVisible()
    await page.locator('textarea').fill('draft')
    await expect(page.getByTestId('voice-record')).toHaveCount(0)
    await page.locator('textarea').fill('')

    const box = (await page.getByTestId('voice-record').boundingBox())!
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
    await page.mouse.down()
    const bar = page.getByTestId('voice-recording')
    await expect(bar).toHaveAttribute('data-phase', 'recording')
    await expect(bar).toHaveAttribute('data-mode', 'hold')
    await expect(bar.getByText('Slide to cancel')).toBeVisible()
    await expect(page.getByTestId('voice-lock')).toBeVisible()
    await page.waitForTimeout(1600)
    await expect(page.getByTestId('voice-timer')).toHaveText(/^0:01,\d$/)
    await page.screenshot({ path: `test-results/messagehub-voice-recording-${scheme}.png` })
    await page.mouse.up()
    await expect(bar).toHaveCount(0)

    const voice = history.getByTestId('message-voice')
    await expect(voice).toHaveCount(1)
    await expect(voice.getByTestId('voice-time')).toHaveText('0:01')
    await expect(voice.getByTestId('voice-waveform').locator('span')).toHaveCount(24)
    await expect(voice.getByTestId('voice-unplayed')).toHaveCount(0)
    await expect(voice.getByTestId('delivery-status')).not.toHaveAttribute('data-status', 'sending', { timeout: 10_000 })
    const [message] = await sentMessages(page, id)
    expect(message.content.content ?? '').toBe('')
    expect(message.content.refs).toHaveLength(1)
    expect(message.content.refs![0].label).toMatch(/^voice-\d{8}-\d{6}\.webm$/)
    const sent = message.voice as { duration_ms: number; waveform: number[] }
    expect(sent.duration_ms).toBeGreaterThanOrEqual(1500)
    expect(sent.waveform.length).toBeGreaterThan(16)
    expect(Math.max(...sent.waveform)).toBe(31)

    await voice.getByTestId('voice-play').click()
    await expect(voice.getByTestId('voice-note')).toHaveAttribute('data-state', 'playing')
    await expect(voice.getByTestId('voice-rate')).toHaveText('1×')
    await voice.getByTestId('voice-rate').click()
    await expect(voice.getByTestId('voice-rate')).toHaveText('1.5×')
    await page.screenshot({ path: `test-results/messagehub-voice-bubble-${scheme}.png` })
    await expect(voice.getByTestId('voice-note')).toHaveAttribute('data-state', 'idle', { timeout: 5_000 })
    await voice.getByTestId('voice-rate').count().then(count => expect(count).toBe(0))
    expect(errors).toEqual([])
  })
}

test('slide left discards, slide up locks, a tap records hands-free and Escape discards', async ({ page }) => {
  const id = await openSession(page, 'Voice gestures')
  const bar = page.getByTestId('voice-recording')

  await holdMic(page, 300, { dx: 160, dy: 0 })
  await expect(bar).toHaveCount(0)

  await holdMic(page, 200, { dx: 0, dy: 110 })
  await expect(bar).toHaveAttribute('data-mode', 'locked')
  await expect(page.getByTestId('voice-record')).toHaveAttribute('aria-label', 'Send voice message')
  await page.waitForTimeout(700)
  await page.getByTestId('voice-record').click()
  await expect(bar).toHaveCount(0)
  await expect(page.getByTestId('conversation-history').getByTestId('message-voice')).toHaveCount(1)

  await page.getByTestId('voice-record').click()
  await expect(bar).toHaveAttribute('data-mode', 'locked')
  await expect(bar).toHaveAttribute('data-phase', 'recording')
  await bar.getByTestId('voice-discard').click()
  await expect(bar).toHaveCount(0)

  await page.getByTestId('voice-record').focus()
  await page.keyboard.press('Enter')
  await expect(bar).toHaveAttribute('data-phase', 'recording')
  await page.keyboard.press('Escape')
  await expect(bar).toHaveCount(0)
  await page.waitForTimeout(300)
  expect(await sentMessages(page, id)).toHaveLength(1)
})

test('a blocked microphone explains itself and records nothing', async ({ page }) => {
  await page.addInitScript(() => {
    navigator.mediaDevices.getUserMedia = () => Promise.reject(new DOMException('blocked', 'NotAllowedError'))
  })
  await openSession(page, 'Voice denied')
  await page.getByTestId('voice-record').click()
  await expect(page.getByTestId('voice-error')).toContainText('Microphone access is blocked')
  await expect(page.getByTestId('voice-recording')).toHaveCount(0)
})

test('a microphone that delivers only silence is flagged while recording and nothing is sent', async ({ page }) => {
  // What a muted input, a closed laptop lid or a virtual default device gives: a live track of exact zeros.
  await page.addInitScript(() => {
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
  const id = await openSession(page, 'Voice silent')
  const box = (await page.getByTestId('voice-record').boundingBox())!
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
  await page.mouse.down()
  const bar = page.getByTestId('voice-recording')
  await expect(bar).toHaveAttribute('data-phase', 'recording')
  await expect(bar.getByTestId('voice-no-signal')).toContainText('No sound from', { timeout: 4_000 })
  await page.screenshot({ path: 'test-results/messagehub-voice-no-signal.png' })
  await page.mouse.up()
  await expect(bar).toHaveCount(0)
  await expect(page.getByTestId('voice-error')).toContainText('so the recording was not sent')
  await page.waitForTimeout(500)
  expect(await sentMessages(page, id)).toHaveLength(0)
  await expect(page.getByTestId('conversation-history').getByTestId('message-voice')).toHaveCount(0)
})

test('a received voice note shows unplayed until played, seeks on its waveform and previews as a voice message', async ({ page }) => {
  await page.goto(`/messagehub?entityId=${encodeURIComponent(ALICE)}`)
  const history = page.getByTestId('conversation-history')
  await expect(history).toBeVisible()
  await page.evaluate(({ owner, id, note }) => window.__messageHubMock.injectMessage(owner, id, {
    from: 'did:buckyos:person:alice',
    to: [owner],
    kind: 'chat',
    created_at_ms: Date.now(),
    ui_message_id: 'msg-alice-voice',
    ui_sender_name: 'Alice',
    voice: note.voice,
    content: { format: 'text/plain', content: '', refs: [{ role: 'input', label: note.label, target: { type: 'data_obj', obj_id: note.objId, uri_hint: `cyfs://${note.objId}` } }] },
  }), { owner: SELF, id: ALICE_SESSION, note: MOCK_VOICE_NOTE })

  const voice = history.getByTestId('message-voice')
  await expect(voice).toBeVisible()
  await expect(voice.getByTestId('voice-time')).toHaveText('0:03')
  await expect(voice.getByTestId('voice-unplayed')).toBeVisible()
  const preview = await page.evaluate(({ context, id }) => window.__messageHubMock.sessions(context).find(item => item.id === id)?.lastMessage?.text, { context: OWN, id: ALICE_SESSION })
  expect(preview).toBe('🎤 0:03')

  await voice.getByTestId('voice-play').click()
  await expect(voice.getByTestId('voice-note')).toHaveAttribute('data-state', 'playing')
  await expect(voice.getByTestId('voice-unplayed')).toHaveCount(0)
  const wave = voice.getByTestId('voice-waveform')
  const box = (await wave.boundingBox())!
  await page.mouse.click(box.x + box.width * 0.75, box.y + box.height / 2)
  await expect(voice.getByTestId('voice-time')).toHaveText('0:02')
  await expect(wave.locator('span[data-played]')).not.toHaveCount(0)
  await voice.getByTestId('voice-play').click()
  await expect(voice.getByTestId('voice-note')).toHaveAttribute('data-state', 'paused')
  await page.screenshot({ path: 'test-results/messagehub-voice-received.png' })

  // Played stays remembered across reloads.
  await page.reload()
  await page.waitForFunction(() => !!window.__messageHubMock)
  await expect(history).toBeVisible()
  await page.evaluate(({ owner, id, note }) => window.__messageHubMock.injectMessage(owner, id, {
    from: 'did:buckyos:person:alice', to: [owner], kind: 'chat', created_at_ms: Date.now(), ui_message_id: 'msg-alice-voice', ui_sender_name: 'Alice', voice: note.voice,
    content: { format: 'text/plain', content: '', refs: [{ role: 'input', label: note.label, target: { type: 'data_obj', obj_id: note.objId } }] },
  }), { owner: SELF, id: ALICE_SESSION, note: MOCK_VOICE_NOTE })
  await expect(history.getByTestId('message-voice')).toBeVisible()
  await expect(history.getByTestId('voice-unplayed')).toHaveCount(0)
})

test('phone: the emoji panel spans the composer and the recording bar fits', async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem('buckyos.prototype.theme.v1', 'dark'))
  await page.setViewportSize({ width: 375, height: 760 })
  await page.goto(`/messagehub?entityId=${encodeURIComponent(ALICE)}`)
  const history = page.getByTestId('conversation-history')
  await expect(history).toBeVisible()
  await page.evaluate(({ owner, id, note }) => window.__messageHubMock.injectMessage(owner, id, {
    from: 'did:buckyos:person:alice', to: [owner], kind: 'chat', created_at_ms: Date.now(), ui_message_id: 'msg-alice-voice-phone', ui_sender_name: 'Alice', voice: note.voice,
    content: { format: 'text/plain', content: '', refs: [{ role: 'input', label: note.label, target: { type: 'data_obj', obj_id: note.objId } }] },
  }), { owner: SELF, id: ALICE_SESSION, note: MOCK_VOICE_NOTE })
  await expect(history.getByTestId('message-voice')).toBeVisible()
  await page.getByTestId('emoji-button').click()
  const panel = page.getByTestId('emoji-panel')
  await expect(panel.getByRole('button', { name: 'grinning face', exact: true })).toBeVisible()
  const composer = (await page.getByTestId('message-composer').boundingBox())!
  const panelBox = (await panel.boundingBox())!
  expect(panelBox.width).toBeGreaterThan(composer.width - 24)
  expect(panelBox.x).toBeGreaterThanOrEqual(composer.x)
  expect(panelBox.x + panelBox.width).toBeLessThanOrEqual(composer.x + composer.width)
  await page.screenshot({ path: 'test-results/messagehub-emoji-panel-phone.png' })
  await page.getByTestId('emoji-button').click()
  await expect(panel).toHaveCount(0)

  await page.getByTestId('voice-record').click()
  const bar = page.getByTestId('voice-recording')
  await expect(bar).toHaveAttribute('data-phase', 'recording')
  await page.waitForTimeout(600)
  const barBox = (await bar.boundingBox())!
  expect(barBox.x).toBeGreaterThanOrEqual(composer.x)
  expect(barBox.x + barBox.width).toBeLessThanOrEqual(composer.x + composer.width)
  await page.screenshot({ path: 'test-results/messagehub-voice-locked-phone.png' })
  await bar.getByTestId('voice-discard').click()
  await expect(bar).toHaveCount(0)
})
