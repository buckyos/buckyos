import { test, expect, type Page } from '@playwright/test'
import { routeAppHost } from '../fixtures/desktop'

// The fixture App is fulfilled by Playwright (no address) and frames the dev server on 127.0.0.1; Chrome's Local
// Network Access check would treat that as a public page reaching into the local network. In a Zone, the App host
// and the Zone root resolve to the same gateway.
test.use({ launchOptions: { args: ['--disable-features=LocalNetworkAccessChecks,PrivateNetworkAccessForNavigations,PrivateNetworkAccessRespectPreflightResults'] } })

const SYSTEM = 'http://127.0.0.1:4173'
const APP = 'http://chat-fixture.localhost:4173/'
const SOURCES = 'buckyos.conversation.mock.sources.v1'
const SELF = 'did:buckyos:user:self'
/** CodeAssistant asks a question there (mock seed). */
const ASKING = 'session-coder-ask'
/** CodeAssistant handles the message in front there (mock seed). */
const WORK = 'session-coder-work'

/**
 * E6 of notepads/conversation-ui-component-todo.md: a third-party App shows a conversation with the owner's Agent
 * inside its own page (`/embed/conversation`, `buckyos.conversation-embed`). The fixture is a plain page that speaks
 * the protocol on the wire, as any App would: it learns the state and the Turn, never the messages.
 */
const fixture = (target: unknown) => `<!doctype html><title>Chat fixture</title>
<output id="state"></output><output id="session"></output><output id="turns"></output><output id="errors"></output>
<iframe id="embed" style="width:420px;height:640px;border:0"></iframe><script>
  const system = ${JSON.stringify(SYSTEM)}
  const nonce = crypto.randomUUID()
  const embed = document.querySelector('#embed')
  const post = (type, payload) => embed.contentWindow.postMessage({ protocol: 'buckyos.conversation-embed', version: 1, type, payload }, system)
  const out = (id, text) => { document.querySelector(id).textContent = text }
  const turns = [], errors = []
  window.sendToAgent = (text, attachments) => post('embed.send', attachments ? { text, attachments } : { text })
  addEventListener('message', e => {
    const m = e.data
    if (e.source !== embed.contentWindow || e.origin !== system || m.protocol !== 'buckyos.conversation-embed') return
    if (m.type === 'embed.hello' && m.payload.nonce === nonce) post('embed.init', { nonce, target: ${JSON.stringify(target)}, theme: 'light', locale: 'en', placeholder: 'Ask about this page' })
    if (m.type === 'embed.state') { out('#state', m.payload.status); out('#session', m.payload.sessionId ?? '') }
    if (m.type === 'embed.turn') { turns.push(m.payload.mode); out('#turns', turns.join(',')) }
    if (m.type === 'embed.error') { errors.push(m.payload.code); out('#errors', errors.join(',')) }
  })
  embed.src = system + '/embed/conversation#bce=' + nonce + '&po=' + encodeURIComponent(location.origin)
</script>`

async function openFixture(page: Page, target: unknown, sources: Record<string, string> = {}) {
  await page.addInitScript(([key, value, origin]) => {
    if (location.origin === origin && !sessionStorage.getItem('seeded')) { localStorage.setItem(key, value); sessionStorage.setItem('seeded', '1') }
  }, [SOURCES, JSON.stringify(sources), SYSTEM] as const)
  await routeAppHost(page, APP, fixture(target))
  await page.goto(APP)
  return page.frameLocator('#embed')
}

const embedFrame = (page: Page) => page.frames().find((frame) => frame.url().startsWith(`${SYSTEM}/embed/conversation`))!

test('the first message of an App starts a conversation with the Agent that carries the App as its source', async ({ page }) => {
  const embed = await openFixture(page, { kind: 'agent' })
  await expect(page.locator('#state')).toHaveText('ready')
  await expect(page.locator('#session')).toHaveText('')
  const input = embed.getByRole('textbox', { name: 'Ask about this page' })
  await expect(input).toBeVisible()

  // The App says something in the user's name; a path it names goes as text.
  await page.evaluate(() => (window as unknown as { sendToAgent: (text: string, attachments?: unknown[]) => void }).sendToAgent('What is this page about?', [{ kind: 'cyfs-path', path: 'cyfs:///home/notes/plan.md' }]))
  await expect(page.locator('#session')).not.toHaveText('')
  const session = await page.locator('#session').textContent()
  await expect(embed.getByTestId('conversation-embed-session')).toHaveAttribute('data-session', session!)
  await expect(embed.locator('.mh-row', { hasText: 'What is this page about?' })).toContainText('cyfs:///home/notes/plan.md')
  expect(await embedFrame(page).evaluate((key) => JSON.parse(localStorage.getItem(key) ?? '{}'), SOURCES)).toEqual({ [`${SELF}\n${session}`]: 'chat-fixture' })

  // The user goes on typing in the same conversation.
  await embed.getByRole('textbox', { name: 'Ask about this page' }).fill('And the second part?')
  await embed.getByRole('button', { name: 'Send', exact: true }).click()
  await expect(embed.locator('.mh-row', { hasText: 'And the second part?' })).toBeVisible()
  await expect(page.locator('#turns')).toContainText('idle')
  await expect(page.locator('#errors')).toHaveText('')
})

test('an open question of the Agent is answered in the embedded conversation', async ({ page }) => {
  const embed = await openFixture(page, { kind: 'session', sessionId: ASKING }, { [`${SELF}\n${ASKING}`]: 'chat-fixture' })
  await expect(page.locator('#session')).toHaveText(ASKING)
  const answer = embed.getByTestId('composer-answer')
  await expect(answer).toBeVisible()
  await expect(page.locator('#turns')).toHaveText(/answer$/)
  await answer.getByTestId('answer-option').first().click()
  await expect(embed.getByTestId('composer-answer')).toHaveCount(0)
  await expect(page.locator('#turns')).not.toHaveText(/answer$/)
})

test('the Turn in front is stopped from the embedded conversation, and the App hears that it stopped', async ({ page }) => {
  const embed = await openFixture(page, { kind: 'session', sessionId: WORK }, { [`${SELF}\n${WORK}`]: 'chat-fixture' })
  const blocked = embed.getByTestId('composer-blocked')
  await expect(blocked).toHaveAttribute('data-mode', 'running')
  await expect(page.locator('#turns')).toHaveText(/running$/)
  await blocked.getByTestId('turn-stop').click()
  await expect(embed.getByTestId('composer-blocked')).toHaveCount(0)
  await expect(page.locator('#turns')).toHaveText(/idle$/)
})

test('an App may only open the conversations it started, and a malformed target is refused', async ({ page }) => {
  await openFixture(page, { kind: 'session', sessionId: ASKING })
  await expect(page.locator('#errors')).toHaveText('denied')
  await expect(page.locator('#state')).toHaveText('error')

  const other = await page.context().newPage()
  await openFixture(other, { kind: 'url', href: 'https://example.com' })
  await expect(other.locator('#errors')).toHaveText('invalid_params')
  await other.close()
})

test.describe('phone', () => {
  test.use({ viewport: { width: 360, height: 640 }, isMobile: true, hasTouch: true })

  test('the embedded conversation fits a 360px wide frame', async ({ page }) => {
    await routeAppHost(page, APP, fixture({ kind: 'agent' }).replace('width:420px;height:640px', 'width:100%;height:600px'))
    await page.goto(APP)
    const embed = page.frameLocator('#embed')
    const input = embed.getByRole('textbox', { name: 'Ask about this page' })
    await expect(input).toBeVisible()
    const frame = embedFrame(page)
    expect(await frame.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true)
    const box = (await input.boundingBox())!
    const frameBox = (await page.locator('#embed').boundingBox())!
    expect(box.y + box.height).toBeLessThanOrEqual(frameBox.y + frameBox.height)
  })
})
