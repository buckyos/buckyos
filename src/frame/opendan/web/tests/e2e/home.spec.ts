import { expect, test, type Page } from '@playwright/test'

const AGENT = 'did:bns:jarvis.devtest'
const UI = 'ui-7f3a2c91'
const WORK_RUNNING = 'work-b41d09e2'
const WORK_PENDING = 'work-5c8e17aa'
const WORK_FAILED = 'work-e02b66f4'
const CHECK = 'sc-daily'

const open = async (page: Page) => {
  const errors: string[] = []
  page.on('pageerror', (e) => errors.push(e.message))
  await page.goto('/?data=mock')
  await expect(page.getByTestId('data-source')).toHaveText('data: mock')
  return errors
}

test('home shows the agent card and the work overview', async ({ page }) => {
  const errors = await open(page)
  const card = page.getByTestId('agent-card')
  await expect(card.getByRole('heading', { name: 'Jarvis' })).toBeVisible()
  await expect(card).toContainText('Your personal agent')
  await expect(card.getByTestId('agent-did')).toHaveText(AGENT)
  await expect(card.getByRole('link', { name: /Detailed settings/ })).toHaveAttribute('href', '#/agent')

  const overview = page.getByTestId('work-overview')
  await expect(overview.getByTestId('stat-working')).toContainText('1')
  await expect(overview.getByTestId('stat-pending')).toContainText('1')
  await expect(overview.getByTestId('stat-total')).toContainText('5')
  const models = overview.getByTestId('model-row')
  await expect(models).toHaveCount(3)
  await expect(models.first()).toContainText('claude-sonnet-5-5')
  await expect(models.first()).toContainText('45k')
  await expect(models.first()).toContainText('660k')
  await expect(models.first()).toContainText('4.1M')
  const recent = overview.getByTestId('recent-items').getByRole('listitem')
  await expect(recent).toHaveCount(4)
  await expect(recent.first()).toContainText('Downloads')
  await expect(overview.getByTestId('recent-items')).toContainText('art-weekly-report')
  expect(errors).toEqual([])
})

test('sessions open in MessageHub when they are conversations, else on their detail page', async ({ page }) => {
  await open(page)
  const list = page.getByTestId('session-list')
  await expect(list.getByTestId('open-default-session')).toHaveAttribute(
    'href',
    `https://test.buckyos.io/messagehub?entityId=${encodeURIComponent(AGENT)}`,
  )
  // Only root sessions are listed until a tree is expanded.
  await expect(list.getByRole('listitem')).toHaveCount(2)
  const ui = list.getByTestId(`home-session-${UI}`)
  await expect(ui).toContainText('Chat with devtest')
  await expect(ui.getByRole('link')).toHaveAttribute(
    'href',
    `https://test.buckyos.io/messagehub?entityId=${encodeURIComponent(AGENT)}&sessionId=${encodeURIComponent(`dm:${AGENT}`)}`,
  )
  await expect(list.getByTestId(`home-session-${CHECK}`).getByRole('link')).toHaveAttribute('href', `#/session/${CHECK}`)

  await ui.getByRole('button', { name: /Show sub sessions \(2\)/ }).click()
  const running = list.getByTestId(`home-session-${WORK_RUNNING}`)
  await expect(running).toContainText('Working')
  await expect(list.getByTestId(`home-session-${WORK_PENDING}`)).toContainText('Needs review')
  await expect(list.getByTestId(`home-session-${WORK_FAILED}`)).toHaveCount(0)
  await running.getByRole('button', { name: /Show sub sessions \(1\)/ }).click()
  await expect(list.getByTestId(`home-session-${WORK_FAILED}`)).toContainText('Failed')
  await ui.getByRole('button', { name: /Hide sub sessions/ }).click()
  await expect(list.getByRole('listitem')).toHaveCount(2)

  await ui.getByRole('button', { name: /Show sub sessions/ }).click()
  await running.getByRole('link').click()
  await expect(page.getByTestId('session-summary')).toContainText(WORK_RUNNING)
  await page.getByRole('link', { name: 'Home' }).click()
  await expect(page.getByTestId('agent-card')).toBeVisible()
})

test('the profile is edited from the card', async ({ page }) => {
  await open(page)
  const card = page.getByTestId('agent-card')
  await card.getByRole('button', { name: 'Edit profile' }).click()
  const dialog = page.getByRole('dialog', { name: 'Edit profile' })
  await dialog.getByLabel('Name').fill('')
  await dialog.getByRole('button', { name: 'Save' }).click()
  await expect(dialog.getByRole('alert')).toContainText('A name is required')
  await dialog.getByLabel('Name').fill('Friday')
  await dialog.getByLabel(/About/).fill('Keeps the house in order.')
  await dialog.getByLabel('Change photo').setInputFiles({
    name: 'a.png',
    mimeType: 'image/png',
    buffer: Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==', 'base64'),
  })
  await expect(dialog.getByRole('button', { name: 'Remove photo' })).toBeVisible()
  await dialog.getByRole('button', { name: 'Save' }).click()
  await expect(dialog).toBeHidden()
  await expect(card.getByRole('heading', { name: 'Friday' })).toBeVisible()
  await expect(card).toContainText('Keeps the house in order.')
  await expect(card.locator('img')).toHaveAttribute('src', /^data:image\/jpeg/)
})

test.describe('on a phone', () => {
  test.use({ viewport: { width: 375, height: 740 }, hasTouch: true, isMobile: true })

  test('home fits the screen', async ({ page }) => {
    await open(page)
    await expect(page.getByTestId('model-row')).toHaveCount(3)
    await page.getByTestId(`home-session-${UI}`).getByRole('button', { name: /Show sub sessions/ }).click()
    await expect(page.getByTestId(`home-session-${WORK_RUNNING}`)).toBeVisible()
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)
    expect(overflow).toBeLessThanOrEqual(0)
    for (const target of await page.getByTestId('session-list').getByRole('link').all()) {
      expect((await target.boundingBox())!.height).toBeGreaterThanOrEqual(44)
    }
    await page.screenshot({ path: 'test-results/home-mobile.png', fullPage: true })
    await page.goto('/?data=mock&lang=zh')
    await expect(page.getByTestId('work-overview')).toContainText('工作概况')
    await page.screenshot({ path: 'test-results/home-mobile-zh.png', fullPage: true })
  })
})

test('home on a wide screen', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 900 })
  await open(page)
  await expect(page.getByTestId('model-row')).toHaveCount(3)
  await page.screenshot({ path: 'test-results/home-desktop.png', fullPage: true })
})
