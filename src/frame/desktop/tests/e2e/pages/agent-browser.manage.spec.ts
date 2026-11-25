import { expect, test, type Page } from '@playwright/test'
import { AGENT, GOAL, HN, NIKKEI, SITE_KEY, YAHOO, asks, frame, home, moment, noHorizontalScroll, openApp, put, rpc, sample, seed, shot, siteNow, siteObjects, stored, type MockWindow } from '../fixtures/agentBrowserSite'

/**
 * "Manage" of the personal site on a phone, on the mock runtime: the websites with their states and what the owner
 * does about each (the jobs of the maintenance Goal on the mock Agent Service, src/app/agent-browser/mock/manage.ts),
 * the maintenance Goal, and what is recorded. Pausing, syncing and the records are the owner's own calls and writes;
 * only removing a website is asked of the Agent.
 */

test.use({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true })

const BLOG = 'blog-example-net'

/** The sample site with two more websites: one the owner paused, one that waits for a sign-in. */
function managedObjects(): Record<string, string> {
  const feed = sample('feed') as { status: Record<string, unknown> }
  const objects = siteObjects((site) => {
    const like = site.sources[0]
    site.sources.push({ ...like, id: BLOG, title: 'Example Blog', home_url: 'https://blog.example.net/', match: [{ host: 'blog.example.net' }], job: `src-${BLOG}`, state: 'paused', note: null })
    site.sources.push({ ...like, id: 'members-example-org', title: 'Members Weekly', home_url: 'https://members.example.org/', match: [{ host: 'members.example.org' }], job: 'src-members-example-org', state: 'active', note: null })
  })
  objects['_data/feed/members-example-org.json'] = JSON.stringify({ ...feed, source: 'members-example-org', items: [], status: { ...feed.status, state: 'needs_login', pending: null, login_url: 'https://members.example.org/login', error: { kind: 'auth', message: 'the session expired', at: 1764001800000 } } })
  objects[`_data/feed/${BLOG}.json`] = JSON.stringify({ ...feed, source: BLOG, items: [], status: { ...feed.status, pending: null } })
  objects['_read/20261124/01JDAAAAAAAAAAAAAAAAAAAAAA.json'] = JSON.stringify(sample('read_batch'))
  objects['_read/20261125/01JDBBBBBBBBBBBBBBBBBBBBBB.json'] = JSON.stringify(sample('read_batch'))
  return objects
}

const row = (page: Page, source: string) => page.locator(`[data-testid="ab-source"][data-source="${source}"]`)
const states = (page: Page) => page.getByTestId('ab-source').evaluateAll((rows) => rows.map((item) => `${item.getAttribute('data-source')}:${item.getAttribute('data-state')}`))

async function openManage(page: Page) {
  await page.getByTestId('ab-site-manage').tap()
  await expect(page.getByTestId('ab-manage')).toHaveAttribute('data-tab', 'sources')
}

test('the websites are listed with what each is doing; syncing and pausing are the owner’s own, without the Agent', async ({ page }, testInfo) => {
  await seed(page, { objects: managedObjects() })
  await openApp(page)
  await openManage(page)
  await expect(row(page, 'members-example-org')).toHaveAttribute('data-state', 'needs_login')
  expect(await states(page)).toEqual([`${YAHOO}:ok`, `${HN}:needs_upgrade`, `${NIKKEI}:failed`, `${BLOG}:paused`, 'members-example-org:needs_login'])
  await expect(row(page, YAHOO)).toContainText(`Synced ${await moment(page, 1764001800000)}`)
  await expect(row(page, NIKKEI)).toContainText('Could not be added: 正文需要订阅，只能取到列表')
  await expect(row(page, BLOG)).toContainText('Paused')
  await expect(row(page, 'members-example-org')).toContainText('Needs you to sign in on the original site')
  await expect(page.getByTestId('ab-manage-maintainer')).toHaveCount(0)
  await noHorizontalScroll(page)
  await shot(page, testInfo, 'manage-sources')

  // One website: what it is doing, when its job ran and runs next, and what can be done.
  await row(page, YAHOO).tap()
  const sheet = page.getByTestId('ab-source-sheet')
  await expect(sheet).toHaveAccessibleName('雅虎日本')
  await expect(page.getByTestId('ab-source-last-run')).toContainText('done')
  await expect(page.getByTestId('ab-source-actions').getByRole('button')).toHaveText(['Read it', 'Sync now', 'Pause', 'Open the original site', 'Remove from my reading…'])
  await shot(page, testInfo, 'manage-source-sheet')

  // "Sync now" runs the source's job out of turn on the Agent's service; the state follows what the run left.
  const before = Date.now()
  await page.getByTestId('ab-source-sync').tap()
  await expect.poll(() => rpc(page, 'goals.job_run')).toEqual([{ goal_id: GOAL, job_id: `src-${YAHOO}` }])
  await page.getByTestId('ab-source-sheet-close').tap()
  await expect(row(page, YAHOO)).toHaveAttribute('data-doing', 'true')
  await expect(row(page, YAHOO)).not.toHaveAttribute('data-doing', 'true', { timeout: 15_000 })
  const checked = (JSON.parse((await stored(page, `_data/feed/${YAHOO}.json`))!) as { status: { checked_at: number } }).status.checked_at
  expect(checked).toBeGreaterThanOrEqual(before)
  await expect(row(page, YAHOO)).toContainText(`Synced ${await moment(page, checked)}`)

  // Pause: the job is switched off, then `site.json` says the source is paused; resume is the reverse.
  await row(page, YAHOO).tap()
  await page.getByTestId('ab-source-pause').tap()
  await expect(row(page, YAHOO)).toHaveAttribute('data-state', 'paused')
  expect(await rpc(page, 'goals.job_set_enabled')).toEqual([{ goal_id: GOAL, job_id: `src-${YAHOO}`, enabled: false }])
  expect((await siteNow(page)).sources.find((source) => source.id === YAHOO)?.state).toBe('paused')
  expect((await siteNow(page)).updated_by).toBe('did:bns:alice')
  await expect(page.getByTestId('ab-source-actions').getByRole('button')).toHaveText(['Read it', 'Resume', 'Open the original site', 'Remove from my reading…'])
  await expect(page.getByTestId('ab-source-next')).toHaveText('Off')
  await page.getByTestId('ab-source-resume').tap()
  await expect(row(page, YAHOO)).toHaveAttribute('data-state', 'ok')
  expect((await rpc(page, 'goals.job_set_enabled')).at(-1)).toEqual({ goal_id: GOAL, job_id: `src-${YAHOO}`, enabled: true })
  expect((await siteNow(page)).sources.find((source) => source.id === YAHOO)?.state).toBe('active')
  await page.getByTestId('ab-source-sheet-close').tap()

  // A website that waits for a sign-in: once the owner signed in on the original site, the same run tries again.
  await row(page, 'members-example-org').tap()
  await expect(page.getByTestId('ab-source-sync')).toHaveText('I have signed in — try again')
  await page.getByTestId('ab-source-sync').tap()
  await page.getByTestId('ab-source-sheet-close').tap()
  await expect(row(page, 'members-example-org')).toHaveAttribute('data-state', 'ok', { timeout: 15_000 })

  // A run that fails says why, in the service's words.
  await page.evaluate((job) => (window as unknown as MockWindow).__agentBrowserMock.manage.failJob(job, 'program', 'the list is empty'), `src-${BLOG}`)
  await row(page, BLOG).tap()
  await page.getByTestId('ab-source-resume').tap()
  await expect(row(page, BLOG)).toHaveAttribute('data-state', 'ok')
  await page.getByTestId('ab-source-sync').tap()
  await page.getByTestId('ab-source-sheet-close').tap()
  await expect(row(page, BLOG)).toHaveAttribute('data-state', 'failed', { timeout: 15_000 })
  await row(page, BLOG).tap()
  await expect(page.getByTestId('ab-source-last-run')).toContainText('failed')
  await expect(page.getByTestId('ab-source-last-run')).toContainText('the list is empty')
  await expect(page.getByTestId('ab-source-sync')).toHaveText('Try again')
  await page.getByTestId('ab-source-read').tap()
  // "Read it" leaves manage for the source's page.
  await expect(page.getByTestId('ab-manage')).toHaveCount(0)
  await expect(home(page)).toHaveAttribute('data-route', `s=${BLOG}`)

  // None of it was said to the Agent, and none of it needed the browser.
  expect(await asks(page)).toEqual([])
  expect(await rpc(page, 'browser.ensure')).toEqual([])
})

test('removing a website is asked of the Agent once, in the fixed format; a website that was not added can be tried again', async ({ page }, testInfo) => {
  await seed(page, { objects: managedObjects() })
  await openApp(page)
  await openManage(page)
  await row(page, HN).tap()
  await page.getByTestId('ab-source-remove').tap()
  const confirm = page.getByTestId('ab-source-remove-confirm')
  await expect(confirm).toHaveAccessibleName('Remove Hacker News?')
  await shot(page, testInfo, 'manage-remove-confirm')
  await confirm.getByTestId('ab-source-remove-cancel').tap()
  expect(await asks(page)).toEqual([])
  await page.getByTestId('ab-source-remove').tap()
  await confirm.getByTestId('ab-source-remove-accept').tap()
  await expect(row(page, HN)).toContainText('CodeAssistant was asked to remove it')
  expect(await asks(page)).toEqual([[
    'Please remove this website from my reading site.',
    `Source: ${HN}`,
    'Title: Hacker News',
    'URL: https://news.ycombinator.com/',
  ].join('\n')])
  // It is not asked for twice, also after the App was opened again.
  await row(page, HN).tap()
  await expect(page.getByTestId('ab-source-remove')).toBeDisabled()
  await expect(page.getByTestId('ab-source-remove')).toHaveText('CodeAssistant was asked to remove it')
  await openApp(page, '', true)
  await openManage(page)
  await expect(row(page, HN)).toContainText('CodeAssistant was asked to remove it')
  // The Agent took it out of `site.json`: it is gone from the list.
  const site = await siteNow(page)
  site.sources = site.sources.filter((source) => source.id !== HN)
  await put(page, SITE_KEY, site)
  await page.getByTestId('ab-manage-back').tap()
  await page.getByTestId('ab-home-reload').tap()
  await openManage(page)
  await expect(row(page, HN)).toHaveCount(0)

  // A website that could not be added has no job: trying again is the Agent's, with the message of the first time.
  await row(page, NIKKEI).tap()
  await expect(page.getByTestId('ab-source-actions').getByRole('button')).toHaveText(['Read it', 'Ask CodeAssistant to try adding it again', 'Open the original site', 'Remove from my reading…'])
  await page.getByTestId('ab-source-retry').tap()
  await expect(page.getByTestId('ab-site-notice')).toHaveAttribute('data-kind', 'add_again')
  expect(await asks(page)).toEqual(['Please add this website to my reading site.\nURL: https://www.nikkei.com/\nTitle: 日经'])
  expect(await rpc(page, 'goals.job_run')).toEqual([])
})

test('a site another Agent maintains, or nobody yet, is listed but not changed from here', async ({ page }) => {
  await seed(page, { objects: siteObjects((site) => { site.maintainer = { agent: 'did:bns:assistant.alice', goal_id: 'g-1' } }) })
  await openApp(page)
  await openManage(page)
  const note = page.getByTestId('ab-manage-maintainer')
  await expect(note).toHaveAttribute('data-maintainer', 'other')
  await expect(note).toContainText('This site is maintained by BuckyOS Assistant.')
  await row(page, YAHOO).tap()
  for (const action of ['sync', 'pause', 'remove']) await expect(page.getByTestId(`ab-source-${action}`)).toBeDisabled()
  await expect(page.getByTestId('ab-source-read')).toBeEnabled()
  await page.getByTestId('ab-source-sheet-close').tap()
  expect(await rpc(page, 'goals.jobs')).toEqual([])
  await page.getByTestId('ab-manage-tab-goal').tap()
  await expect(page.getByTestId('ab-manage-goal').getByTestId('ab-manage-maintainer')).toBeVisible()

  const site = await siteNow(page)
  site.maintainer = null
  await put(page, SITE_KEY, site)
  await page.getByTestId('ab-manage-tab-sources').tap()
  await expect(note).toHaveAttribute('data-maintainer', 'none')
  await expect(note).toContainText('No Agent maintains a reading site for you yet.')
  // The other Agent's browser is one press away.
  site.maintainer = { agent: 'did:bns:assistant.alice', goal_id: 'g-1' }
  await put(page, SITE_KEY, site)
  await page.getByTestId('ab-manage-tab-goal').tap()
  await page.getByTestId('ab-manage-switch').tap()
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-agent', 'assistant.alice')
})

test('the maintenance Goal: its jobs with their last and next run, its recent changes, and the Goal itself as a conversation', async ({ page }, testInfo) => {
  await seed(page, { objects: managedObjects() })
  await openApp(page)
  // Reachable from the menu as well, whatever is on the stage.
  await page.getByTestId('ab-mode-chat').tap()
  await page.getByTestId('ab-menu-button').tap()
  await page.getByTestId('ab-menu-manage').tap()
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'home')
  await page.getByTestId('ab-manage-tab-goal').tap()
  const tab = page.getByTestId('ab-manage-goal')
  await expect(tab).toHaveAttribute('data-goal', GOAL)
  await expect(page.getByTestId('ab-goal-open')).toContainText('Weekly project report')
  await expect(page.getByTestId('ab-goal-open').getByTestId('goal-status')).toHaveAttribute('data-status', 'active')

  // The jobs: one for each website that was installed, and the hourly summary of the reading records.
  const jobs = page.getByTestId('ab-job')
  expect(await jobs.evaluateAll((items) => items.map((item) => `${item.getAttribute('data-job')}:${item.getAttribute('data-enabled')}`))).toEqual([
    `src-${YAHOO}:true`, `src-${HN}:true`, `src-${BLOG}:false`, 'src-members-example-org:true', 'site-reading:true',
  ])
  await expect(jobs.nth(0)).toContainText('Sync 雅虎日本')
  await expect(jobs.nth(0)).toContainText(`Last run ${await moment(page, 1764001800000)}: done`)
  await expect(jobs.nth(0)).toContainText('Next run')
  await expect(jobs.nth(2)).toContainText('Off')
  await expect(jobs.nth(3)).toContainText('failed')
  await expect(jobs.nth(3)).toContainText('the session expired')
  expect(await rpc(page, 'goals.jobs')).toContainEqual({ goal_id: GOAL })

  // The recent changes, newest first: what, when, by whom, why.
  const changes = page.getByTestId('ab-change')
  expect(await rpc(page, 'goals.changes')).toContainEqual({ goal_id: GOAL, limit: 12 })
  expect(await changes.evaluateAll((items) => items.map((item) => item.getAttribute('data-kind')))).toEqual(['revised', 'job', 'revised', 'job', 'created'])
  await expect(changes.nth(0)).toContainText('Revised: constraints')
  await expect(changes.nth(0)).toContainText('from a conversation')
  await expect(changes.nth(0)).toContainText('The owner does not want sports news')
  await expect(changes.nth(1)).toContainText(`Job changed: src-${HN}: put`)
  await expect(changes.nth(1)).toContainText('by a Work of CodeAssistant')
  await noHorizontalScroll(page)
  await shot(page, testInfo, 'manage-goal')

  // The Goal itself is the system's Goal conversation; Back returns to the tab.
  await page.getByTestId('ab-goal-open').tap()
  const conversation = page.getByTestId('goal-conversation')
  await expect(conversation).toHaveAttribute('data-goal', GOAL)
  await expect(page.getByTestId('goal-title')).toHaveText('Weekly project report')
  await shot(page, testInfo, 'manage-goal-conversation')
  await page.getByTestId('goal-back').tap()
  await expect(tab).toBeVisible()
  await page.getByTestId('ab-manage-back').tap()
  await expect(page.getByTestId('ab-manage')).toHaveCount(0)
  await expect(frame(page).locator('#title')).toHaveText('我的阅读')
})

test('records: what is recorded is said plainly, shown, switched off and cleared by the owner', async ({ page }, testInfo) => {
  await seed(page, { objects: managedObjects() })
  await openApp(page)
  await openManage(page)
  await page.getByTestId('ab-manage-tab-records').tap()
  await expect(page.getByTestId('ab-records-intro')).toContainText('so that the Agent maintaining your reading site knows what you read. Nobody else uses them.')
  await expect(page.getByTestId('ab-records-intro')).toContainText('90 days')

  // Reading records: on by default; the summary says what was read most.
  const reading = page.getByTestId('ab-reading-switch')
  await expect(reading).toHaveAttribute('aria-checked', 'true')
  await expect(reading).toContainText('when you opened it, how long it stayed in front of you, and how far you read')
  await expect(page.getByTestId('ab-reading-summary')).toContainText('Articles read since')
  await expect(page.getByTestId('ab-reading-sources').locator('li')).toHaveText([/雅虎日本.*2 min · opened 3/])
  await expect(page.getByTestId('ab-reading-tags').locator('li').first()).toContainText('科技')
  // Browsing records: the pages of the Agent's browser, newest first.
  const visits = page.getByTestId('ab-visits-switch')
  await expect(visits).toHaveAttribute('aria-checked', 'true')
  await expect(visits).toContainText('Not while you enter a password, and not while CodeAssistant operates.')
  await expect(page.getByTestId('ab-visits-list').locator('li')).toHaveCount(4)
  await expect(page.getByTestId('ab-visits-list').locator('li').first()).toContainText('NHKニュース')
  await expect(page.getByTestId('ab-visits-list').locator('li').first()).toContainText('www3.nhk.or.jp/news')
  expect(await rpc(page, 'browser.visits')).toContainEqual({})
  await noHorizontalScroll(page)
  await shot(page, testInfo, 'manage-records')

  // Off: `site.json` says so (as it is now), and the Agent's service stops recording pages.
  await reading.tap()
  await expect(reading).toHaveAttribute('aria-checked', 'false')
  expect((await siteNow(page)).reading).toEqual({ enabled: false })
  expect((await siteNow(page)).sources).toHaveLength(5)
  await visits.tap()
  await expect(visits).toHaveAttribute('aria-checked', 'false')
  expect(await rpc(page, 'browser.visits_set_enabled')).toEqual([{ enabled: false }])
  // What is there stays until it is cleared.
  await expect(page.getByTestId('ab-visits-list').locator('li')).toHaveCount(4)
  await shot(page, testInfo, 'manage-records-off')

  // Clearing asks first, then deletes: every reported batch and the summary; the list of pages.
  expect(await page.evaluate(() => (window as unknown as MockWindow).__agentBrowserMock.reflow.keys('_read/'))).toHaveLength(2)
  await page.getByTestId('ab-reading-clear').tap()
  await expect(page.getByTestId('ab-records-confirm')).toHaveAccessibleName('Clear reading records?')
  await page.getByTestId('ab-records-cancel').tap()
  expect(await stored(page, '_data/reading.json')).not.toBeNull()
  await page.getByTestId('ab-reading-clear').tap()
  await page.getByTestId('ab-records-accept').tap()
  await expect(page.getByTestId('ab-reading-empty')).toBeVisible()
  expect(await page.evaluate(() => (window as unknown as MockWindow).__agentBrowserMock.reflow.keys('_read/'))).toEqual([])
  expect(await stored(page, '_data/reading.json')).toBeNull()
  expect(await stored(page, SITE_KEY)).not.toBeNull()
  await page.getByTestId('ab-visits-clear').tap()
  await expect(page.getByTestId('ab-records-confirm')).toHaveAccessibleName('Clear browsing records?')
  await page.getByTestId('ab-records-accept').tap()
  await expect(page.getByTestId('ab-visits-empty')).toBeVisible()
  expect(await rpc(page, 'browser.visits_clear')).toEqual([{}])
  expect(await page.evaluate((agent) => (window as unknown as MockWindow).__agentBrowserMock.manage.visits(agent), AGENT)).toEqual({ enabled: false, visits: [] })

  // On again.
  await reading.tap()
  await expect(reading).toHaveAttribute('aria-checked', 'true')
  expect((await siteNow(page)).reading).toEqual({ enabled: true })
  expect(await asks(page)).toEqual([])
  expect(await rpc(page, 'browser.ensure')).toEqual([])
})

test('without a site there is nothing to list, and the browsing records are still the owner’s to see and clear', async ({ page }) => {
  await openApp(page)
  await page.getByTestId('ab-menu-button').tap()
  await page.getByTestId('ab-menu-manage').tap()
  await expect(page.getByTestId('ab-manage-sources-empty')).toBeVisible()
  await page.getByTestId('ab-manage-tab-records').tap()
  await expect(page.getByTestId('ab-reading-switch')).toBeDisabled()
  await expect(page.getByTestId('ab-reading-empty')).toHaveText('There is no reading site yet.')
  await expect(page.getByTestId('ab-visits-list').locator('li')).toHaveCount(4)
  await page.getByTestId('ab-manage-tab-sources').tap()
  // "Add a website" offers the ways to start.
  await page.getByTestId('ab-manage-add').tap()
  await expect(page.getByTestId('ab-start-sheet')).toBeVisible()
})

test('manage follows the language', async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem('buckyos.prototype.locale.v1', 'zh-CN'))
  await seed(page, { objects: managedObjects() })
  await openApp(page)
  await openManage(page)
  await expect(page.getByTestId('ab-manage').getByRole('tab')).toHaveText(['网站', '维护', '记录'])
  await expect(row(page, BLOG)).toContainText('已暂停')
  await page.getByTestId('ab-manage-tab-records').tap()
  await expect(page.getByTestId('ab-reading-switch')).toContainText('阅读记录')
  await expect(page.getByTestId('ab-visits-switch')).toContainText('浏览记录')
})
