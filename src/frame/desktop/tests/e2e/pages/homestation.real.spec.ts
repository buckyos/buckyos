import { expect, test, type Locator, type Page } from '@playwright/test'

const DEV = 'hsDevToken=tok-me'

function trackErrors(page: Page) {
  const errors: string[] = []
  page.on('console', message => {
    if (message.type() === 'error') errors.push(message.text())
  })
  page.on('pageerror', error => errors.push(error.message))
  return errors
}

async function openHomeStation(page: Page) {
  await page.goto(`/homestation?${DEV}`)
  await expect(page.getByTestId('hs-card').first()).toBeVisible()
}

const card = (page: Page, text: string) => page.getByTestId('hs-card').filter({ hasText: text })
const post = (page: Page, text: string) => page.locator('[data-testid="hs-card"]:not([data-content-type="comment"])').filter({ hasText: text })

async function scrollToCard(page: Page, target: Locator) {
  const scroll = page.getByTestId('hs-feed-scroll')
  await expect.poll(async () => {
    if ((await target.count()) > 0) return true
    await scroll.evaluate(element => element.scrollTo(0, element.scrollHeight))
    return false
  }, { timeout: 20_000, intervals: [300] }).toBe(true)
  await target.first().scrollIntoViewIfNeeded()
  return target.first()
}

test.describe('HomeStation on the homestation service (devnet)', () => {
  test.skip(!process.env.HS_REAL_E2E, 'run through playwright.homestation.config.ts')
  test.use({ viewport: { width: 1440, height: 900 } })

  test('the reading list shows Alice’s seeded posts with their media and article body', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    await expect(page.getByTestId('hs-card-publisher').filter({ hasText: 'Alice Chen' }).first()).toBeVisible()
    await expect(await scrollToCard(page, post(page, 'Started our balcony garden today'))).toHaveAttribute('data-content-type', 'text')
    const gallery = await scrollToCard(page, post(page, 'Before and after of the garden weekend'))
    await expect(gallery).toHaveAttribute('data-content-type', 'image')
    const images = gallery.getByTestId('hs-media-grid').locator('img')
    await expect(images).toHaveCount(2)
    await expect(images.first()).toHaveAttribute('src', /^\/home\/objects\/cyfile[^/]+\/content\?access=/)
    await expect.poll(() => images.evaluateAll(list => list.every(img => (img as HTMLImageElement).complete && (img as HTMLImageElement).naturalWidth > 0))).toBe(true)
    const article = await scrollToCard(page, post(page, 'A week of balcony gardening'))
    await expect(article).toHaveAttribute('data-content-type', 'article')
    await article.getByText('A week of balcony gardening').first().click()
    await expect(page.getByTestId('hs-detail')).toBeVisible()
    await expect(page.getByTestId('hs-article-body')).toHaveAttribute('data-state', 'ready')
    await expect(page.getByTestId('hs-article-body')).toContainText('Six hours of sun is the minimum for tomatoes.')
    await page.getByRole('button', { name: 'Back' }).click()
    await gallery.getByTestId('hs-media-grid').click()
    await expect(page.getByTestId('hs-image-preview')).toBeVisible()
    await expect(page.getByTestId('hs-image-preview').locator('img').first()).toBeVisible()
    expect(errors).toEqual([])
  })

  test('AI-tagged items are hidden by default and shown on request', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    await expect(page.getByTestId('hs-hidden-summary')).toContainText(/\d+ hidden by your filters/)
    await scrollToCard(page, post(page, 'A week of balcony gardening'))
    await expect(card(page, 'AIGC experiment: a poem about soil')).toHaveCount(0)
    await page.getByTestId('hs-toggle-filtered').click()
    const filtered = await scrollToCard(page, post(page, 'AIGC experiment: a poem about soil'))
    await expect(filtered.getByTestId('hs-filtered-badge')).toBeVisible()
    await page.getByTestId('hs-toggle-filtered').click()
    await expect(card(page, 'AIGC experiment: a poem about soil')).toHaveCount(0)
    expect(errors).toEqual([])
  })

  test('a like is stored by the service and survives a reload', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    const target = await scrollToCard(page, post(page, 'Before and after of the garden weekend'))
    await expect(target.getByTestId('hs-like')).toHaveAttribute('aria-pressed', 'false')
    await target.getByTestId('hs-like').click()
    await expect(target.getByTestId('hs-like')).toHaveAttribute('aria-pressed', 'true')
    await expect(target.getByTestId('hs-like-count')).toHaveText('1')
    await page.reload()
    await expect(page.getByTestId('hs-card').first()).toBeVisible()
    const reloaded = await scrollToCard(page, post(page, 'Before and after of the garden weekend'))
    await expect(reloaded.getByTestId('hs-like')).toHaveAttribute('aria-pressed', 'true')
    await page.getByTestId('hs-nav-published').click()
    await page.getByTestId('hs-published-kind-reactions').click()
    await expect(card(page, 'Before and after of the garden weekend')).toHaveCount(1)
    expect(errors).toEqual([])
  })

  test('a comment on Alice’s post appears in the detail view', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    const target = await scrollToCard(page, post(page, 'Started our balcony garden today'))
    await target.getByTestId('hs-comment').click()
    await expect(page.getByTestId('hs-detail')).toBeVisible()
    await expect(page.getByTestId('hs-comment-row').filter({ hasText: 'Try leafy greens first, they forgive mistakes.' })).toBeVisible()
    const text = `Basil wants the sunniest box (${Date.now().toString(36)})`
    await page.getByTestId('hs-comment-form').getByRole('textbox').fill(text)
    await page.getByTestId('hs-comment-submit').click()
    await expect(page.getByTestId('hs-toast').filter({ hasText: 'Comment published' })).toBeVisible()
    await expect(page.getByTestId('hs-comment-row').filter({ hasText: text })).toBeVisible()
    await page.getByRole('button', { name: 'Back' }).click()
    await page.getByTestId('hs-nav-published').click()
    await page.getByTestId('hs-published-kind-comments').click()
    await expect(card(page, text)).toHaveCount(1)
    expect(errors).toEqual([])
  })

  test('a published text post is listed in My publications with its delivery state', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    const text = `Hello from the HomeStation e2e suite (${Date.now().toString(36)})`
    await page.getByTestId('hs-quick-text').fill(text)
    await page.getByTestId('hs-quick-submit').click()
    await expect(page.getByTestId('hs-toast').filter({ hasText: 'Published to your feed' })).toBeVisible()
    await expect(card(page, text)).toHaveCount(0)
    await page.getByTestId('hs-nav-published').click()
    const mine = card(page, text)
    await expect(mine).toHaveCount(1)
    const status = mine.getByTestId('hs-publish-status')
    await expect(status).toHaveAttribute('data-stage', 'published')
    await expect(status).toHaveAttribute('data-delivery', /^(delivering|delivered|partially_failed)$/)
    await expect(status).toHaveAttribute('data-delivery', 'delivered', { timeout: 45_000 })
    await expect(mine.getByTestId('hs-published-audience')).toContainText('Public')
    expect(errors).toEqual([])
  })

  test('the followed-candidates view loads', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    await page.getByTestId('hs-nav-candidates').click()
    const view = page.getByTestId('hs-candidates')
    await expect(view).toBeVisible()
    await expect(view.getByTestId('hs-loading')).toHaveCount(0)
    await expect(view.getByTestId('hs-error')).toHaveCount(0)
    await expect(view).toContainText('Kept for 14 days')
    await expect(view.getByTestId('hs-candidate-footer').or(view.getByTestId('hs-empty')).first()).toBeVisible()
    expect(errors).toEqual([])
  })

  test('owner previews read as a real follower and friend', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    await page.getByTestId('hs-nav-profile').click()
    await expect(page.getByTestId('hs-preview-follower')).toHaveText('Follower (Sarah Kim)')
    await expect(page.getByTestId('hs-preview-friend')).toHaveText(/^Friend \((Alice Chen|Bob Zhang)\)$/)
    const list = page.getByTestId('hs-profile-list')
    await page.getByTestId('hs-preview-friend').click()
    await expect(page.locator('[data-testid="hs-profile"][data-reader="friend"]')).toBeVisible()
    await expect(list).toContainText('Friends only: dinner photos coming soon')
    await page.getByTestId('hs-preview-follower').click()
    await expect(page.locator('[data-testid="hs-profile"][data-reader="follower"]')).toBeVisible()
    await expect(list).toContainText('Testing my HomeStation')
    await expect(list).not.toContainText('Friends only: dinner photos coming soon')
    await expect(page.getByTestId('hs-reader-approximated')).toHaveCount(0)
    expect(errors).toEqual([])
  })

  test('mute choices come from the HomeStation’s people', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    await expect(post(page, 'Color palettes for winter interfaces')).toBeVisible()
    await page.getByTestId('hs-nav-prefs').click()
    const choice = page.getByTestId('hs-mute-choice')
    await expect(choice.locator('option[value="person|did:test:sarah"]')).toHaveText('Sarah Kim')
    await expect(choice.locator('option[value="person|did:test:alice"]')).toHaveText('Alice Chen')
    await expect(choice.locator('option[value^="person|did:bns:"]')).toHaveCount(0)
    await choice.selectOption('person|did:test:sarah')
    await page.getByTestId('hs-mute-add').click()
    await expect(page.getByTestId('hs-mute-list')).toContainText('Sarah Kim')
    await page.getByTestId('hs-nav-feed').click()
    await expect(page.getByTestId('hs-hidden-summary')).toContainText(/\d+ from people you chose not to see/)
    await expect(post(page, 'Color palettes for winter interfaces')).toHaveCount(0)
    await page.getByTestId('hs-nav-prefs').click()
    await page.getByTestId('hs-mute-list').getByTestId('hs-unmute').click()
    await expect(page.getByTestId('hs-mute-list')).not.toContainText('Sarah Kim')
    expect(errors).toEqual([])
  })

  test('change polling pauses while the page is hidden', async ({ page }) => {
    const errors = trackErrors(page)
    const polls: number[] = []
    page.on('request', request => {
      if (request.url().endsWith('/kapi/homestation') && request.postData()?.includes('"ui.versions"')) polls.push(Date.now())
    })
    await openHomeStation(page)
    await expect.poll(() => polls.length, { timeout: 10_000 }).toBeGreaterThan(0)
    const setHidden = (hidden: boolean) => page.evaluate(value => {
      Object.defineProperty(document, 'hidden', { configurable: true, get: () => value })
      document.dispatchEvent(new Event('visibilitychange'))
    }, hidden)
    await setHidden(true)
    await page.waitForTimeout(500)
    const paused = polls.length
    await page.waitForTimeout(7_000)
    expect(polls.length).toBe(paused)
    await setHidden(false)
    await expect.poll(() => polls.length, { timeout: 2_000 }).toBeGreaterThan(paused)
    expect(errors).toEqual([])
  })

  test('the visitor route shows Alice’s public posts but not her friends-only post to an anonymous reader', async ({ page }) => {
    const errors = trackErrors(page)
    await page.goto(`/homestation/u/did:test:alice?reader=anonymous&${DEV}`)
    await expect(page.getByTestId('hs-visitor')).toHaveAttribute('data-reader', 'anonymous')
    await expect(page.getByTestId('hs-profile')).toContainText('Alice Chen')
    const list = page.getByTestId('hs-profile-list')
    await expect(list).toContainText('Started our balcony garden today')
    await expect(list).toContainText('Before and after of the garden weekend')
    await expect(list).not.toContainText('Family barbecue')
    await expect(page.getByTestId('hs-reader-approximated')).toHaveCount(0)
    await page.getByTestId('hs-visitor-reader-friend').click()
    await expect(page.getByTestId('hs-visitor')).toHaveAttribute('data-reader', 'friend')
    await expect(page.getByTestId('hs-reader-approximated')).toBeVisible()
    await expect(list).toContainText('Started our balcony garden today')
    await expect(list).not.toContainText('Family barbecue')
    expect(errors).toEqual([])
  })
})
