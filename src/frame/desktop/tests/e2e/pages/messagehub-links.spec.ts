import { expect, test, type BrowserContext, type Page } from '@playwright/test'

/** URLs in message text open on click: plain text keeps its characters, Markdown keeps its formatting. */

declare global {
  interface Window {
    __messageHubMock: import('../../../src/app/messagehub/mock/store').MessageHubMockStore
    __openedExternally?: string[]
  }
}

const SELF = 'did:buckyos:user:self'
const OWN = { viewerDid: SELF, ownerDid: SELF, mode: 'self' as const }
const BOB = 'did:buckyos:person:bob'

type Content = { format: string; content: string; refs?: unknown[] }

async function openBob(page: Page, context: BrowserContext) {
  await context.route('https://example.com/**', route => route.fulfill({ contentType: 'text/html', body: '<title>target</title>' }))
  await page.route('https://upload.wikimedia.org/**', route => route.fulfill({ contentType: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600"/>' }))
  await page.goto(`/messagehub?entityId=${encodeURIComponent(BOB)}`)
  await expect(page.getByTestId('conversation-history')).toBeVisible()
}

async function receive(page: Page, id: string, content: Content) {
  await page.evaluate(({ context, peer, id, content }) => {
    const mock = window.__messageHubMock
    mock.injectMessage(context.ownerDid, mock.defaultSession(context, peer)!.id, { from: peer, to: [context.ownerDid], kind: 'chat', created_at_ms: Date.now(), ui_message_id: id, ui_sender_name: 'Bob', content } as never)
  }, { context: OWN, peer: BOB, id, content })
}

const hrefs = (page: Page, testId: string, text: string) => page.getByTestId('conversation-history').getByTestId(testId).filter({ hasText: text }).getByRole('link').evaluateAll(links => links.map(link => link.getAttribute('href')))

test('plain text links every URL and leaves the rest as typed', async ({ page, context }) => {
  await openBob(page, context)
  const text = [
    '看看https://example.com/zh，然后 *not emphasis* __raw__',
    'wiki https://example.com/a_(b) and (https://example.com/c?x=1&y=2).',
    '“https://example.com/d” Https://EXAMPLE.com/e! {"u":"https://example.com/f","n":1}',
    'not a link: ftp://example.com https:// www.example.com',
  ].join('\n')
  await receive(page, 'msg-links', { format: 'text/plain', content: text })

  const bubble = page.getByTestId('conversation-history').getByTestId('message-bubble').filter({ hasText: 'not emphasis' })
  await expect(bubble.getByRole('link')).toHaveCount(6)
  expect(await hrefs(page, 'message-bubble', 'not emphasis')).toEqual([
    'https://example.com/zh',
    'https://example.com/a_(b)',
    'https://example.com/c?x=1&y=2',
    'https://example.com/d',
    'Https://EXAMPLE.com/e',
    'https://example.com/f',
  ])
  await expect(bubble.locator('p', { hasText: 'not emphasis' })).toHaveText(text)
  await expect(bubble.locator('em, strong')).toHaveCount(0)
})

test('clicking a link opens it in a new page and does not open the bubble actions', async ({ page, context }) => {
  await openBob(page, context)
  await receive(page, 'msg-click', { format: 'text/plain', content: 'open https://example.com/click please' })
  const bubble = page.getByTestId('conversation-history').getByTestId('message-bubble').filter({ hasText: 'open https' })

  const opened = context.waitForEvent('page')
  await bubble.getByRole('link').click()
  await expect.poll(async () => (await opened).url()).toBe('https://example.com/click')
  await expect(bubble).not.toHaveAttribute('data-actions-open', 'true')
  await expect(page).toHaveURL(/\/messagehub/)
})

test('own messages link URLs in the self bubble link colour', async ({ page, context }) => {
  await openBob(page, context)
  const composer = page.getByTestId('message-composer').locator('textarea')
  await composer.fill('mine https://example.com/mine')
  await composer.press('Enter')

  const link = page.getByTestId('conversation-history').getByTestId('message-bubble').filter({ hasText: 'mine https' }).getByRole('link')
  await expect(link).toHaveAttribute('href', 'https://example.com/mine')
  await expect(link).toHaveCSS('text-decoration-line', 'underline')
  const colours = await link.evaluate(element => {
    const probe = document.createElement('span')
    probe.style.color = 'var(--cp-message-self-link)'
    element.parentElement!.append(probe)
    const expected = getComputedStyle(probe).color
    probe.remove()
    return { actual: getComputedStyle(element).color, expected }
  })
  expect(colours.actual).toBe(colours.expected)
})

test('attachment captions and the message details link URLs too', async ({ page, context }) => {
  await openBob(page, context)
  await receive(page, 'msg-caption', { format: 'text/plain', content: 'caption https://example.com/caption', refs: [{ role: 'input', label: 'design-brief.pdf', target: { type: 'data_obj', obj_id: 'cyfile:mock-doc-brief', uri_hint: 'cyfs://cyfile:mock-doc-brief' } }] })
  await expect(page.getByTestId('conversation-history').getByTestId('message-attachments').filter({ hasText: 'caption https' }).getByRole('link', { name: 'https://example.com/caption' })).toHaveAttribute('href', 'https://example.com/caption')

  await receive(page, 'msg-details', { format: 'text/plain', content: 'details https://example.com/details' })
  const bubble = page.getByTestId('conversation-history').getByTestId('message-bubble').filter({ hasText: 'details https' })
  await bubble.hover()
  await bubble.getByTestId('message-details').click()
  await expect(page.getByTestId('detail-effective').getByRole('link')).toHaveAttribute('href', 'https://example.com/details')
})

test('Markdown keeps its formatting, labelled links and the same URL recognition', async ({ page, context }) => {
  await openBob(page, context)
  await receive(page, 'msg-markdown', { format: 'text/markdown', content: '**bold** https://example.com/md_(x)。[label](https://example.com/label)' })
  const bubble = page.getByTestId('conversation-history').getByTestId('message-bubble').filter({ hasText: 'bold' })
  await expect(bubble.locator('strong')).toHaveText('bold')
  expect(await hrefs(page, 'message-bubble', 'bold')).toEqual(['https://example.com/md_(x)', 'https://example.com/label'])
  await expect(bubble.getByRole('link', { name: 'label' })).toHaveCSS('text-decoration-line', 'underline')
})

test('inside the App shell a link opens through the native bridge', async ({ page, context }) => {
  await page.addInitScript(() => {
    window.__openedExternally = []
    Object.assign(window, { BuckyApi: { openExternalUrl: (url: string) => { window.__openedExternally!.push(url) } } })
  })
  await openBob(page, context)
  await receive(page, 'msg-shell', { format: 'text/plain', content: 'shell https://example.com/shell' })
  await page.getByTestId('conversation-history').getByTestId('message-bubble').filter({ hasText: 'shell https' }).getByRole('link').click()

  await expect.poll(() => page.evaluate(() => window.__openedExternally)).toEqual(['https://example.com/shell'])
  expect(context.pages()).toHaveLength(1)
  await expect(page).toHaveURL(/\/messagehub/)
})

test.describe('on a phone', () => {
  test.use({ viewport: { width: 375, height: 812 }, hasTouch: true, isMobile: true })

  test('a long URL wraps inside its bubble and a tap opens it', async ({ page, context }) => {
    await openBob(page, context)
    const url = `https://example.com/${'long-path-segment/'.repeat(12)}end?token=${'a'.repeat(120)}`
    await receive(page, 'msg-phone', { format: 'text/plain', content: `手机 ${url}` })
    const bubble = page.getByTestId('conversation-history').getByTestId('message-bubble').filter({ hasText: '手机' })
    const link = bubble.getByRole('link')
    await expect(link).toHaveAttribute('href', url)

    const box = await bubble.evaluate(element => {
      const lines = [...element.querySelector('a')!.getClientRects()]
      return { lines: lines.length, linkRight: Math.max(...lines.map(line => line.right)), bubbleRight: element.getBoundingClientRect().right, pageOverflow: document.documentElement.scrollWidth - window.innerWidth }
    })
    expect(box.lines).toBeGreaterThan(1)
    expect(box.linkRight).toBeLessThanOrEqual(box.bubbleRight)
    expect(box.bubbleRight).toBeLessThanOrEqual(375)
    expect(box.pageOverflow).toBe(0)
    await page.screenshot({ path: 'test-results/messagehub-links-phone.png' })

    const opened = context.waitForEvent('page')
    await link.tap()
    await expect.poll(async () => (await opened).url()).toBe(url)
    await expect(bubble).not.toHaveAttribute('data-actions-open', 'true')
  })
})
