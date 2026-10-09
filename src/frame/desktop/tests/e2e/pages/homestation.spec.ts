import { expect, test, type Page } from '@playwright/test'

function trackErrors(page: Page) {
  const errors: string[] = []
  page.on('console', message => {
    if (message.type() === 'error') errors.push(message.text())
  })
  page.on('pageerror', error => errors.push(error.message))
  return errors
}

async function openHomeStation(page: Page, search = '') {
  await page.goto(`/homestation${search}`)
  await expect(page.getByTestId('hs-card').first()).toBeVisible()
}

const card = (page: Page, text: string) => page.getByTestId('hs-card').filter({ hasText: text })

async function scrollToCard(page: Page, text: string) {
  const target = card(page, text)
  const scroll = page.getByTestId('hs-feed-scroll')
  await expect.poll(async () => {
    if ((await target.count()) > 0) return true
    await scroll.evaluate(element => element.scrollTo(0, element.scrollHeight))
    return false
  }, { timeout: 15_000, intervals: [250] }).toBe(true)
  await target.first().scrollIntoViewIfNeeded()
  return target.first()
}

async function openMenuItem(page: Page, cardText: string, item: RegExp) {
  const target = await scrollToCard(page, cardText)
  await target.getByTestId('hs-card-more').click()
  await page.getByRole('menuitem', { name: item }).click()
}

test.describe('HomeStation prototype (architecture v0.6)', () => {
  test.use({ viewport: { width: 1440, height: 900 } })

  test('A06 video without ready resources is not offered as playable', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    await page.getByTestId('hs-filter-videos').click()
    const ready = card(page, 'Building a personal AI assistant')
    await expect(ready.getByTestId('hs-play')).toBeVisible()
    const preparing = card(page, 'Procedural world generation')
    await expect(preparing.getByTestId('hs-resource-state')).toHaveAttribute('data-state', 'preparing')
    await expect(preparing.getByTestId('hs-play')).toHaveCount(0)
    const unavailable = card(page, 'Boss fight preview')
    await expect(unavailable.getByTestId('hs-resource-state')).toHaveAttribute('data-state', 'unavailable')
    await expect(unavailable.getByTestId('hs-play')).toHaveCount(0)
    await expect(unavailable.getByRole('button', { name: 'Retry' })).toBeVisible()
    expect(errors).toEqual([])
  })

  test('A74 first like explains that likes are public; bookmarks stay private', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    const target = card(page, 'New design explorations')
    await target.getByTestId('hs-like').click()
    const notice = page.getByRole('dialog', { name: 'Likes are public' })
    await expect(notice).toBeVisible()
    await expect(notice).toContainText('Bookmarks stay private')
    await page.getByTestId('hs-like-confirm').click()
    await expect(target.getByTestId('hs-like')).toHaveAttribute('aria-pressed', 'true')
    await card(page, 'Just shipped the new decentralized identity').getByTestId('hs-like').click()
    await expect(page.getByRole('dialog', { name: 'Likes are public' })).toHaveCount(0)
    await page.getByTestId('hs-nav-published').click()
    await page.getByTestId('hs-published-kind-reactions').click()
    await expect(page.getByTestId('hs-card').filter({ hasText: 'New design explorations' })).toBeVisible()
    expect(errors).toEqual([])
  })

  test('A11 A43 bookmarks are private by default and never reach the visitor view', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    const target = card(page, 'Building a personal AI assistant')
    await target.getByTestId('hs-bookmark').click()
    await expect(page.getByTestId('hs-toast').filter({ hasText: 'Bookmarked privately' })).toBeVisible()
    await page.getByTestId('hs-nav-bookmarks').click()
    const saved = card(page, 'Building a personal AI assistant')
    await expect(saved.getByTestId('hs-saved-meta')).toContainText('Private bookmark')
    await page.getByTestId('hs-nav-published').click()
    await page.getByTestId('hs-published-kind-reactions').click()
    await expect(page.getByTestId('hs-published')).toBeVisible()
    await expect(card(page, 'Building a personal AI assistant')).toHaveCount(0)
    await page.getByTestId('hs-nav-profile').click()
    for (const reader of ['anonymous', 'friend']) {
      await page.getByTestId(`hs-preview-${reader}`).click()
      const list = page.getByTestId('hs-profile-list')
      await expect(list.getByTestId('hs-card').first()).toBeVisible()
      await expect(list).not.toContainText('Building a personal AI assistant')
      await expect(list.getByTestId('hs-reason')).toHaveCount(0)
      await expect(list.getByTestId('hs-like')).toHaveCount(0)
    }
    expect(errors).toEqual([])
  })

  test('A17 A29 comments on an old version are marked; a withdrawn post keeps its bookmark', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    const updated = await scrollToCard(page, 'Just shipped the new decentralized identity')
    await updated.getByTestId('hs-updated-badge').click()
    await expect(page.getByTestId('hs-detail')).toBeVisible()
    await expect(page.getByTestId('hs-older-comments')).toBeVisible()
    await expect(page.getByTestId('hs-old-version')).toHaveCount(2)
    await expect(page.locator('[data-testid="hs-comment-row"][data-old-version="false"]')).toHaveCount(1)
    await page.getByRole('button', { name: 'Back' }).click()
    const withdrawn = await scrollToCard(page, 'Bob Zhang withdrew this post')
    await expect(withdrawn).toHaveAttribute('data-state', 'withdrawn')
    await page.getByTestId('hs-nav-bookmarks').click()
    const saved = page.getByTestId('hs-card').filter({ hasText: 'withdrew this post' })
    await expect(saved.getByTestId('hs-saved-withdrawn')).toBeVisible()
    await expect(saved).toContainText('Your bookmark is kept.')
    expect(errors).toEqual([])
  })

  test('A35 not showing a person hides them in the feed and catch-up but keeps the follow', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    await expect(page.getByTestId('hs-card-publisher').filter({ hasText: 'Bob Zhang' }).first()).toBeVisible()
    await openMenuItem(page, 'Building a personal AI assistant', /Don’t show Bob Zhang/)
    await expect(page.getByTestId('hs-toast').filter({ hasText: 'Following and friendship are unchanged' })).toBeVisible()
    await expect(page.getByTestId('hs-card-publisher').filter({ hasText: 'Bob Zhang' })).toHaveCount(0)
    await page.getByTestId('hs-nav-candidates').click()
    await expect(page.getByTestId('hs-candidate-footer').first()).toBeVisible()
    await expect(page.getByTestId('hs-candidates')).not.toContainText('Trail conditions update')
    await page.getByTestId('hs-nav-sources').click()
    const bob = page.locator('[data-testid="hs-source-row"][data-source="src-bob"]')
    await expect(bob).toContainText('From friendship')
    await expect(bob.getByTestId('hs-friend-derived')).toBeVisible()
    await page.getByTestId('hs-nav-prefs').click()
    await expect(page.getByTestId('hs-mute-list')).toContainText('Bob Zhang')
    expect(errors).toEqual([])
  })

  test('A37 A38 catch-up lists unselected candidates and drops one after it is read', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    await page.getByTestId('hs-filter-following').click()
    const entry = await scrollToCard(page, 'Reading list for the week')
    await expect(entry).toBeVisible()
    await page.getByTestId('hs-catchup-entry').click()
    for (const state of ['unscreened', 'not_selected', 'preparing']) {
      await expect(page.locator(`[data-testid="hs-candidate-footer"][data-selection="${state}"]`).first()).toBeVisible()
    }
    const coffee = card(page, 'Coffee chat notes')
    await coffee.getByTestId('hs-candidate-open').click()
    await expect(page.getByTestId('hs-detail')).toBeVisible()
    await page.getByRole('button', { name: 'Back' }).click()
    await expect(page.getByTestId('hs-candidate-footer').first()).toBeVisible()
    await expect(card(page, 'Coffee chat notes')).toHaveCount(0)
    await page.getByTestId('hs-candidates-include-read').check()
    await expect(card(page, 'Coffee chat notes')).toBeVisible()
    expect(errors).toEqual([])
  })

  test('A41 a failed publish is not shown as success and a retry creates one post', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page, '?scenario=publish-fail')
    const text = 'Testing idempotent publishing from the quick panel'
    await page.getByTestId('hs-quick-text').fill(text)
    await page.getByTestId('hs-quick-submit').click()
    await expect(page.getByTestId('hs-quick-failed')).toBeVisible()
    await expect(page.getByTestId('hs-toast').filter({ hasText: 'Published' })).toHaveCount(0)
    await page.getByTestId('hs-quick-retry').click()
    await expect(page.getByTestId('hs-toast').filter({ hasText: 'Published to your feed' })).toBeVisible()
    await expect(card(page, text)).toHaveCount(0)
    await page.getByTestId('hs-nav-published').click()
    await expect(card(page, text)).toHaveCount(1)
    await expect(card(page, text).getByTestId('hs-publish-status')).toHaveAttribute('data-stage', 'published')
    expect(errors).toEqual([])
  })

  test('A69 anonymous, follower and friend readers get different home feeds', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    await page.getByTestId('hs-nav-profile').click()
    const list = page.getByTestId('hs-profile-list')
    const counts: Record<string, number> = {}
    for (const reader of ['anonymous', 'follower', 'friend']) {
      await page.getByTestId(`hs-preview-${reader}`).click()
      await expect(page.locator(`[data-testid="hs-profile"][data-reader="${reader}"]`)).toBeVisible()
      await expect(list.getByTestId('hs-card').first()).toBeVisible()
      await expect(list.getByTestId('hs-card')).toHaveCount(reader === 'anonymous' ? 4 : reader === 'follower' ? 5 : 7)
      counts[reader] = await list.getByTestId('hs-card').count()
    }
    expect(counts.anonymous).toBeLessThan(counts.follower)
    expect(counts.follower).toBeLessThan(counts.friend)
    // The public portal of the same home feed reads as an anonymous visitor.
    await page.goto('/homestation/leo')
    await expect(page.getByTestId('hs-portal')).toHaveAttribute('data-feed', 'leo')
    await expect(page.getByTestId('hs-portal-sign-in')).toBeVisible()
    await expect(page.getByTestId('hs-profile-list').getByTestId('hs-card').first()).toBeVisible()
    await expect(page.getByTestId('hs-profile-list').getByTestId('hs-card')).toHaveCount(4)
    await expect(page.getByTestId('hs-profile-list')).not.toContainText('Saturday trail plan')
    await expect(page.getByTestId('hs-profile-list')).not.toContainText('Notes for followers')
    await page.getByTestId('hs-portal-zone').click()
    await expect(page.getByTestId('hs-portal')).toHaveAttribute('data-feed', '~zone')
    await expect(page.getByTestId('hs-zone-list').getByTestId('hs-card')).toHaveCount(1)
    expect(errors).toEqual([])
  })

  test('A70 restricted posts cannot be reposted and stay invisible inside other wrappers', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    const friendsOnly = await scrollToCard(page, 'Family BBQ this weekend')
    await expect(friendsOnly.getByTestId('hs-audience-badge')).toContainText('Friends only')
    await expect(friendsOnly.getByTestId('hs-repost')).toBeDisabled()
    const wrapper = await scrollToCard(page, 'This cave biome is gorgeous')
    await expect(wrapper.getByTestId('hs-embed-placeholder')).toHaveAttribute('data-visibility', 'not_visible')
    await expect(wrapper.getByTestId('hs-embed-placeholder')).toContainText('Original not visible')
    expect(errors).toEqual([])
  })

  test('A71 a comment on a restricted post goes to the author only and stays off the public homepage', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    const friendsOnly = await scrollToCard(page, 'Family BBQ this weekend')
    await friendsOnly.getByTestId('hs-comment').click()
    await expect(page.getByTestId('hs-comment-restricted')).toBeVisible()
    const text = 'Count me in for the next one'
    await page.getByTestId('hs-comment-form').getByRole('textbox').fill(text)
    await page.getByTestId('hs-comment-submit').click()
    await expect(page.getByTestId('hs-toast').filter({ hasText: 'to the author only' })).toBeVisible()
    await expect(page.getByTestId('hs-comment-row').filter({ hasText: text })).toBeVisible()
    await page.getByTestId('hs-nav-published').click()
    const mine = card(page, text)
    await expect(mine.getByTestId('hs-published-audience')).toContainText('Only Alice Chen')
    await expect(mine).toContainText('Not on your public homepage')
    await page.getByTestId('hs-nav-profile').click()
    for (const reader of ['anonymous', 'friend']) {
      await page.getByTestId(`hs-preview-${reader}`).click()
      await expect(page.getByTestId('hs-profile-list').getByTestId('hs-card').first()).toBeVisible()
      await expect(page.getByTestId('hs-profile-list')).not.toContainText(text)
    }
    expect(errors).toEqual([])
  })

  test('A72 a private capture is not published until it is shared', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    await expect(card(page, 'OpenAI announces GPT-5').getByTestId('hs-capture-line')).toContainText('Captured by your Spider')
    await page.getByTestId('hs-nav-published').click()
    await expect(page.getByTestId('hs-published-meta').first()).toBeVisible()
    await expect(card(page, 'OpenAI announces GPT-5')).toHaveCount(0)
    await page.getByTestId('hs-nav-feed').click()
    await openMenuItem(page, 'OpenAI announces GPT-5', /Share to my homepage/)
    await page.getByTestId('hs-share-confirm').click()
    await expect(page.getByTestId('hs-toast').filter({ hasText: 'Shared to your homepage' })).toBeVisible()
    await page.getByTestId('hs-nav-published').click()
    await expect(card(page, 'OpenAI announces GPT-5')).toHaveCount(1)
    await expect(card(page, 'OpenAI announces GPT-5').getByTestId('hs-capture-line')).toContainText('Shared by Leo Wang')
    expect(errors).toEqual([])
  })

  test('A56 filtered items can be shown on request and a rule change keeps the scroll anchor', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    await expect(card(page, 'Daily AI Digest')).toHaveCount(0)
    const anchor = await scrollToCard(page, 'Show HN: A 200-line static site generator')
    await anchor.evaluate(element => element.scrollIntoView({ block: 'center' }))
    await page.waitForTimeout(250)
    const before = (await anchor.boundingBox())!.y
    await page.getByTestId('hs-info-rule-rule-ai-full').uncheck()
    await expect(card(page, 'Daily AI Digest')).toHaveCount(1)
    await page.waitForTimeout(150)
    const after = (await anchor.boundingBox())!.y
    expect(Math.abs(after - before)).toBeLessThan(6)
    await page.getByTestId('hs-info-rule-rule-ai-full').check()
    await expect(card(page, 'Daily AI Digest')).toHaveCount(0)
    await page.getByTestId('hs-toggle-filtered').click()
    const filtered = card(page, 'Daily AI Digest')
    await expect(filtered.getByTestId('hs-filtered-badge')).toBeVisible()
    await expect(filtered.locator('[data-testid="hs-tag"][data-tag="ai_full"]')).toContainText('Possibly AI-generated')
    expect(errors).toEqual([])
  })

  test('A44 immersive mode keeps the current filter and topic', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    await page.getByTestId('hs-filter-videos').click()
    await page.getByTestId('hs-topic-topic-tech').click()
    await expect(page.getByTestId('hs-view-scope')).toContainText('Tech')
    await expect(page.getByTestId('hs-card')).toHaveCount(2)
    await page.getByTestId('hs-mode-immersive').click()
    await expect(page.getByTestId('hs-immersive-scope')).toContainText('Videos')
    await expect(page.getByTestId('hs-immersive-scope')).toContainText('tech')
    await expect(page.getByTestId('hs-immersive-counter')).toHaveText('1 / 2')
    await page.keyboard.press('ArrowDown')
    await expect(page.getByTestId('hs-immersive-counter')).toHaveText('2 / 2')
    await page.getByTestId('hs-immersive-close').click()
    await expect(page.getByTestId('hs-filter-videos')).toHaveAttribute('aria-pressed', 'true')
    await expect(page.getByTestId('hs-topic-topic-tech')).toHaveAttribute('aria-pressed', 'true')
    expect(errors).toEqual([])
  })

  test('every view keeps its own scroll position', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    const anchor = await scrollToCard(page, 'Reading list for the week')
    await anchor.evaluate(element => element.scrollIntoView({ block: 'start' }))
    await page.waitForTimeout(250)
    const scroll = page.getByTestId('hs-feed-scroll')
    const allTop = await scroll.evaluate(element => element.scrollTop)
    expect(allTop).toBeGreaterThan(400)
    await page.getByTestId('hs-filter-images').click()
    await expect(page.getByTestId('hs-card').first()).toBeVisible()
    await expect.poll(() => scroll.evaluate(element => element.scrollTop)).toBeLessThan(50)
    await page.getByTestId('hs-filter-all').click()
    await expect(card(page, 'Reading list for the week')).toBeVisible()
    await expect.poll(() => scroll.evaluate(element => element.scrollTop)).toBeGreaterThan(allTop - 40)
    await card(page, 'Reading list for the week').getByText('Reading list for the week').click()
    await expect(page.getByTestId('hs-detail')).toBeVisible()
    await page.getByRole('button', { name: 'Back' }).click()
    await expect(card(page, 'Reading list for the week')).toBeVisible()
    await expect.poll(() => page.getByTestId('hs-feed-scroll').evaluate(element => element.scrollTop)).toBeGreaterThan(allTop - 40)
    expect(errors).toEqual([])
  })

  test('error and empty states', async ({ page }) => {
    const errors = trackErrors(page)
    await page.goto('/homestation?scenario=error')
    await expect(page.getByTestId('hs-error')).toBeVisible()
    await page.getByTestId('hs-error').getByRole('button', { name: 'Retry' }).click()
    await expect(page.getByTestId('hs-card').first()).toBeVisible()
    await page.goto('/homestation?scenario=empty')
    await expect(page.getByTestId('hs-empty')).toContainText('Your reading list is empty')
    expect(errors).toEqual([])
  })
})

test.describe('HomeStation on a phone', () => {
  test.use({ viewport: { width: 375, height: 812 }, hasTouch: true, isMobile: true })

  test('main flow at 375px without horizontal scrolling', async ({ page }) => {
    const errors = trackErrors(page)
    await openHomeStation(page)
    const noOverflow = async () => {
      const width = await page.evaluate(() => Math.max(document.documentElement.scrollWidth, document.body.scrollWidth))
      expect(width).toBeLessThanOrEqual(375)
    }
    await noOverflow()
    const target = card(page, 'New design explorations')
    await target.getByTestId('hs-like').tap()
    await page.getByTestId('hs-like-confirm').tap()
    await expect(target.getByTestId('hs-like')).toHaveAttribute('aria-pressed', 'true')
    await target.getByTestId('hs-comment').tap()
    await expect(page.getByTestId('hs-comments')).toBeVisible()
    await noOverflow()
    await page.getByRole('button', { name: 'Back' }).tap()
    await page.getByTestId('hs-avatar').tap()
    for (const entry of ['hs-me-profile', 'hs-me-published', 'hs-me-candidates', 'hs-me-bookmarks', 'hs-me-read-later', 'hs-me-sources', 'hs-me-prefs']) {
      await expect(page.getByTestId(entry)).toBeVisible()
    }
    await page.getByTestId('hs-me-published').tap()
    await expect(page.getByTestId('hs-published-meta').first()).toBeVisible()
    await noOverflow()
    await page.getByRole('button', { name: 'Back' }).tap()
    await page.getByTestId('hs-me-sources').tap()
    await expect(page.getByTestId('hs-source-row').first()).toBeVisible()
    await noOverflow()
    await page.getByRole('button', { name: 'Back' }).tap()
    await page.getByRole('button', { name: 'Back' }).tap()
    await page.getByTestId('hs-fab').tap()
    await page.getByTestId('hs-publish-text').fill('Posting from my phone')
    await expect(page.getByTestId('hs-publish-audience')).toBeVisible()
    await page.getByTestId('hs-publish-submit').tap()
    await expect(page.getByTestId('hs-toast').filter({ hasText: 'Published to your feed' })).toBeVisible()
    await noOverflow()
    expect(errors).toEqual([])
  })
})
