import { expect, test, type Page } from '@playwright/test'

/**
 * buckyos#638: splitter drag state, message reactions in direct sessions,
 * and the entity list hiding `root` and the logged-in account.
 */

declare global {
  interface Window { __messageHubMock: import('../../../src/app/messagehub/mock/store').MessageHubMockStore }
}

const SELF = 'did:buckyos:user:self'
const OWN = { viewerDid: SELF, ownerDid: SELF, mode: 'self' as const }
const BOB = 'did:buckyos:person:bob'
const BOB_SESSION = 'session-bob-1'

async function openHub(page: Page, entityId?: string) {
  await page.route('https://upload.wikimedia.org/**', route => route.fulfill({ contentType: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600"/>' }))
  await page.goto(entityId ? `/messagehub?entityId=${encodeURIComponent(entityId)}` : '/messagehub')
  await expect(entityId ? page.getByTestId('message-composer') : page.getByRole('button', { name: 'New group', exact: true }).first()).toBeVisible()
}

test('reactions in a direct session: quick bar, picker, own toggle, peer counts', async ({ page }) => {
  await openHub(page, BOB)
  const history = page.getByTestId('conversation-history')
  const bubble = history.locator('[data-testid="message-bubble"]', { hasText: 'Ah makes sense' })
  await expect(bubble).toBeVisible()
  // No reaction yet: the chips row is absent; the hover bar carries the quick reactions.
  await expect(bubble.getByTestId('reactions')).toHaveCount(0)
  await bubble.hover()
  await expect(bubble.getByTestId('message-hover-bar')).toBeVisible()
  await bubble.getByRole('button', { name: 'React with 👍' }).click()
  await expect(bubble.getByTestId('reactions')).toContainText('👍 1')
  const own = bubble.getByTestId('reactions').getByRole('button', { name: /^👍 1/ })
  await expect(own).toHaveAttribute('aria-pressed', 'true')
  // Reacting again with the same key is a no-op (one count per person and key).
  await bubble.getByRole('button', { name: 'Remove your 👍 reaction' }).hover()
  await expect(bubble.getByTestId('reactions')).toContainText('👍 1')

  // Bob reacts too, then the viewer's own click on the chip removes only theirs.
  await page.evaluate(({ owner, id, from, target }) => window.__messageHubMock.injectRelation(owner, id, from, target, { rel: 'reaction', key: '👍' }, '👍'), { owner: SELF, id: BOB_SESSION, from: BOB, target: 'msg-b1-3' })
  await expect(bubble.getByTestId('reactions')).toContainText('👍 2')
  await expect(bubble.getByTestId('reactions').getByRole('button', { name: /^👍 2/ })).toHaveAttribute('title', /You, Bob Zhang reacted with 👍|Bob Zhang, You reacted with 👍/)
  await bubble.getByTestId('reactions').getByRole('button', { name: /^👍 2/ }).click()
  await expect(bubble.getByTestId('reactions')).toContainText('👍 1')
  await expect(bubble.getByTestId('reactions').getByRole('button', { name: /^👍 1/ })).toHaveAttribute('aria-pressed', 'false')

  // The picker offers more than the quick four.
  await bubble.hover()
  await bubble.getByTestId('add-reaction').click()
  const picker = page.getByTestId('reaction-picker')
  await expect(picker).toBeVisible()
  await picker.getByRole('menuitem', { name: 'React with 🔥' }).click()
  await expect(picker).toHaveCount(0)
  await expect(bubble.getByTestId('reactions')).toContainText('🔥 1')
  await page.screenshot({ path: 'test-results/messagehub-reactions-direct.png' })

  // Own messages can be reacted to as well, and the reaction is a relation message, not a row.
  const mine = history.locator('[data-testid="message-bubble"]', { hasText: 'push the fix tonight' })
  await mine.hover()
  await mine.getByRole('button', { name: 'React with 🎉' }).click()
  await expect(mine.getByTestId('reactions')).toContainText('🎉 1')
  await expect(history.locator('[data-testid="message-bubble"]')).toHaveCount(4)
  const sent = await page.evaluate(async ({ context, id }) => (await window.__messageHubMock.reader(context, id).readRange(0, 500)).map(message => message.content.content), { context: OWN, id: BOB_SESSION })
  expect(sent).not.toContain('🎉')
  // The session preview is not the reaction.
  await expect(page.getByRole('button', { name: /^Bob Zhang/ })).not.toContainText('🎉')
})

test('touch devices get the reactions from the footer menu', async ({ browser }) => {
  const context = await browser.newContext({ viewport: { width: 412, height: 839 }, hasTouch: true, isMobile: true })
  const page = await context.newPage()
  await openHub(page, BOB)
  const bubble = page.getByTestId('conversation-history').locator('[data-testid="message-bubble"]', { hasText: 'Ah makes sense' })
  await expect(bubble.getByTestId('message-hover-bar')).toBeHidden()
  await bubble.getByTestId('message-actions-touch').getByRole('button', { name: 'Message actions' }).click()
  await bubble.getByRole('menuitem', { name: 'React with ❤️' }).click()
  await expect(bubble.getByTestId('reactions')).toContainText('❤️ 1')
  await bubble.getByTestId('message-actions-touch').getByRole('button', { name: 'Message actions' }).click()
  await bubble.getByRole('menuitem', { name: /^Add reaction/ }).click()
  await page.getByTestId('reaction-picker').getByRole('menuitem', { name: 'React with 🚀' }).click()
  await expect(bubble.getByTestId('reactions')).toContainText('🚀 1')
  await context.close()
})

test('the entity list hides root and the logged-in account', async ({ page }) => {
  await openHub(page)
  const list = page.getByTestId('entity-filters').locator('..')
  await expect(page.getByRole('button', { name: /^Bob Zhang/ })).toBeVisible()
  await expect(list.getByText('root', { exact: true })).toHaveCount(0)
  await expect(page.getByRole('button', { name: /^root\b/ })).toHaveCount(0)
  const listed = await page.evaluate(context => window.__messageHubMock.entities(context).map(entity => entity.id), OWN)
  expect(listed).not.toContain('did:bns:root')
  expect(listed).not.toContain(SELF)
  expect(listed).toContain(BOB)
})

test('splitter drag resizes without per-move renders and always releases its state', async ({ page }) => {
  await page.setViewportSize({ width: 1400, height: 900 })
  await openHub(page, BOB)
  const splitter = page.getByTestId('entity-list-splitter')
  await expect(splitter).toHaveAttribute('aria-valuenow', '340')
  const pane = splitter.locator('xpath=preceding-sibling::div[1]')
  const box = (await splitter.boundingBox())!
  const x = box.x + box.width / 2, y = box.y + box.height / 2
  await page.mouse.move(x, y)
  await page.mouse.down()
  await page.mouse.move(x + 60, y, { steps: 4 })
  await expect(splitter).toHaveAttribute('data-active', 'true')
  await expect.poll(() => page.evaluate(() => document.body.style.cursor)).toBe('col-resize')
  // The pane follows the pointer while the committed state has not changed yet.
  await expect.poll(async () => Math.round((await pane.boundingBox())!.width)).toBe(400)
  await expect(splitter).toHaveAttribute('aria-valuenow', '340')
  await page.mouse.move(x + 100, y, { steps: 4 })
  await page.mouse.up()
  await expect(splitter).not.toHaveAttribute('data-active', 'true')
  await expect(splitter).toHaveAttribute('aria-valuenow', '440')
  await expect.poll(() => page.evaluate(() => document.body.style.cursor)).toBe('')
  // Focus stays where it was: the splitter is not a button.
  await page.locator('textarea').focus()
  await page.mouse.move(x + 100, y)
  await page.mouse.down()
  await page.mouse.move(x + 60, y, { steps: 3 })
  await expect(splitter).toHaveAttribute('data-active', 'true')
  await expect(page.locator('textarea')).toBeFocused()
  // Losing the window focus mid-drag ends the drag, commits the width and restores the cursor.
  await page.evaluate(() => window.dispatchEvent(new Event('blur')))
  await expect(splitter).not.toHaveAttribute('data-active', 'true')
  await expect(splitter).toHaveAttribute('aria-valuenow', '400')
  await expect.poll(() => page.evaluate(() => document.body.style.cursor)).toBe('')
  await page.mouse.up()
  // Escape restores the width from before the drag.
  await page.mouse.move(x + 60, y)
  await page.mouse.down()
  await page.mouse.move(x + 120, y, { steps: 3 })
  await expect.poll(async () => Math.round((await pane.boundingBox())!.width)).toBe(460)
  await page.keyboard.press('Escape')
  await expect(splitter).toHaveAttribute('aria-valuenow', '400')
  await expect.poll(async () => Math.round((await pane.boundingBox())!.width)).toBe(400)
  await page.mouse.up()
  // Keyboard resizing on the focused separator.
  await splitter.focus()
  await page.keyboard.press('ArrowLeft')
  await expect(splitter).toHaveAttribute('aria-valuenow', '384')
  // The bounds hold (the keyboard step animates, so wait for the pane to settle first).
  await expect.poll(async () => Math.round((await pane.boundingBox())!.width)).toBe(384)
  await page.mouse.move(x + 44, y)
  await page.mouse.down()
  await page.mouse.move(x + 600, y, { steps: 3 })
  await page.mouse.up()
  await expect(splitter).toHaveAttribute('aria-valuenow', '520')
})
