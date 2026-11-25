import { expect, test } from '@playwright/test'
import { AGENT, ARTICLE, ARTICLE_URL, GOAL, HN, SITE_KEY, YAHOO, asks, browse, frame, home, moment, noHorizontalScroll, openApp, put, recordAsks, rpc, sample, seed, shot, siteNow, siteObjects, type MockWindow, type SiteJson, type Source } from '../fixtures/agentBrowserSite'
import { openFromAllApps } from '../fixtures/desktop'
import { pickZone, putZoneFile, selectDialog } from '../fixtures/selectDialog'

/**
 * Home mode of the Agent browser on a phone: the owner's personal site (Personal Site Protocol), on the mock runtime.
 * The site's data is the shared samples of doc/opendan/protocol/fixtures/site/, put into the mock bucket before the
 * App opens (`window.__agentBrowserMockSeed`); the page in the frame is the mock's stand-in for the site's template,
 * which speaks the `site.*` messages. The specs play the Agent's side by writing the bucket
 * (`window.__agentBrowserMock.site`) and replace `askAgent` to see what the Agent is asked.
 */

test.use({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true })

test('home opens without the browser: an empty site offers the three ways to start, the address bar and a task for the Agent', async ({ page }, testInfo) => {
  await openApp(page)
  const app = page.getByTestId('agent-browser')
  await expect(app).toHaveAttribute('data-mode', 'home')
  await expect(page.getByRole('tab')).toHaveText(['Home', 'Page', 'Chat'])
  await expect(home(page)).toHaveAttribute('data-status', 'absent')
  const start = page.getByTestId('ab-site-start')
  await expect(start).toContainText('Your reading site')
  await expect(start.getByTestId('ab-start-points').getByRole('button')).toHaveText([/Import bookmarks/, /Open a website/, /Tell CodeAssistant/])
  await expect(page.getByTestId('ab-address-input')).toHaveValue('')
  await expect(page.getByTestId('ab-back')).toBeDisabled()
  await expect(page.getByTestId('ab-status-agent')).toHaveText('CodeAssistant is idle')
  await expect(page.getByTestId('ab-status-operator')).toHaveCount(0)
  await noHorizontalScroll(page)
  await shot(page, testInfo, 'home-empty')

  // Nothing started the browser, connected its screen or wrote to the Agent.
  expect(await rpc(page, 'browser.status')).not.toHaveLength(0)
  expect(await rpc(page, 'browser.ensure')).toEqual([])
  expect(await page.evaluate(() => (window as unknown as MockWindow).__agentBrowserMock.log.length)).toBe(0)
  expect(await asks(page)).toEqual([])
  await expect(page.getByTestId('ab-screen')).toHaveCount(0)

  // A task is given in the conversation, which comes up over home; so does "tell the Agent what to read".
  await page.getByTestId('ab-give-task').tap()
  await expect(page.getByTestId('ab-chat-sheet')).toBeVisible()
  await expect(page.getByTestId('ab-give-task')).toHaveCount(0)
  await page.getByTestId('ab-chat-sheet-close').tap()
  await page.getByTestId('ab-start-chat').tap()
  await expect(page.getByTestId('ab-chat-sheet')).toBeVisible()
  await page.getByTestId('ab-chat-sheet-close').tap()
  expect(await rpc(page, 'browser.ensure')).toEqual([])

  // "Open a website" and an address entered at home are the browser's: only now is it started.
  await page.getByTestId('ab-start-open').tap()
  await expect(app).toHaveAttribute('data-mode', 'page')
  await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open')
  expect(await rpc(page, 'browser.ensure')).toHaveLength(1)
  await page.getByTestId('ab-mode-home').tap()
  await browse(page, 'example.com')
  await expect.poll(async () => page.evaluate(() => (window as unknown as MockWindow).__agentBrowserMock.log.filter((entry) => entry.message.type === 'tab').map((entry) => entry.message.url))).toEqual(['https://example.com'])
})

test('the site is read at home: sources, articles, in-site links and Back, the original page, a link that is not collected', async ({ page }, testInfo) => {
  await seed(page, { objects: siteObjects() })
  await openApp(page)
  await expect(home(page)).toHaveAttribute('data-status', 'ready')
  await expect(home(page)).toHaveAttribute('data-route', '')
  await expect(frame(page).locator('#title')).toHaveText('我的阅读')
  await expect(page.getByTestId('ab-site-frame')).toHaveAttribute('data-drawn', 'true')
  // The shell's strip is the App's own, read from `_data`: the site and how many websites it has.
  await expect(page.getByTestId('ab-site-title')).toHaveText('我的阅读')
  await expect(page.getByTestId('ab-site-status')).toHaveText('Websites: 3')
  await expect(page.getByTestId('ab-site-original')).toHaveCount(0)
  await expect(page.getByTestId('ab-site-maintainer')).toHaveCount(0)
  await expect(page.getByTestId('ab-address-input')).toHaveValue('')
  await noHorizontalScroll(page)
  expect((await page.getByTestId('ab-site-frame').boundingBox())!.height).toBeGreaterThan(400)
  await shot(page, testInfo, 'home-site')

  // A source: the page says where it is (`site.location`), the strip says when it was last synced and what waits.
  await frame(page).locator(`#source-${YAHOO}`).tap()
  await expect(home(page)).toHaveAttribute('data-route', `s=${YAHOO}`)
  const strip = page.getByTestId('ab-site-strip')
  await expect(strip).toHaveAttribute('data-state', 'ok')
  await expect(page.getByTestId('ab-site-title')).toHaveText('雅虎日本')
  await expect(page.getByTestId('ab-site-status')).toHaveText(`Synced ${await moment(page, 1764001800000)}`)
  await expect(page.getByTestId('ab-site-pending')).toHaveText('2 new articles wait for translation: today’s budget is used up')
  await expect(page.getByTestId('ab-address-input')).toHaveValue('news.yahoo.co.jp')
  await shot(page, testInfo, 'strip-source-ok')

  // An article opens in the site, at once; the bar shows where it comes from.
  await frame(page).locator(`#item-${ARTICLE}`).tap()
  await expect(home(page)).toHaveAttribute('data-route', `a=${ARTICLE}`)
  await expect(frame(page).locator('#title')).toHaveText('半导体出口新规下月生效，日本设备厂商受影响')
  await expect(page.getByTestId('ab-site-title')).toHaveText('雅虎日本')
  await expect(page.getByTestId('ab-address-input')).toHaveValue(ARTICLE_URL.replace('https://', ''))
  await shot(page, testInfo, 'home-article')
  // A link to another collected article stays in the site; Back walks the routes, in the page (`site.navigate`) without loading it anew.
  await frame(page).getByText('相关：市中心初雪').tap()
  await expect(home(page)).toHaveAttribute('data-route', 'a=81c5e462be246971')
  await page.getByTestId('ab-back').tap()
  await expect(home(page)).toHaveAttribute('data-route', `a=${ARTICLE}`)
  await expect(frame(page).locator('#title')).toHaveText('半导体出口新规下月生效，日本设备厂商受影响')
  await page.getByTestId('ab-back').tap()
  await expect(home(page)).toHaveAttribute('data-route', `s=${YAHOO}`)
  await expect(frame(page).locator('#title')).toHaveText('雅虎日本')
  await page.getByTestId('ab-back').tap()
  await expect(home(page)).toHaveAttribute('data-route', '')
  await expect(page.getByTestId('ab-back')).toBeDisabled()
  await expect(frame(page).locator('body')).toHaveAttribute('data-route', '')
  expect(Number(await frame(page).locator('body').getAttribute('data-draws'))).toBeGreaterThan(3)

  // All of that without the browser, and without a word to the Agent.
  expect(await rpc(page, 'browser.ensure')).toEqual([])
  expect(await asks(page)).toEqual([])

  // A link that is not collected (`site.open`) is the original site's, in original page mode; the Agent is not asked.
  await frame(page).locator(`#source-${YAHOO}`).tap()
  await frame(page).locator(`#item-${ARTICLE}`).tap()
  await frame(page).getByText('经济产业省的公告').tap()
  const app = page.getByTestId('agent-browser')
  await expect(app).toHaveAttribute('data-mode', 'page')
  await expect(page.getByTestId('ab-address-input')).toHaveValue('www.meti.go.jp/press/2026/11/20261125001.html')
  expect(await asks(page)).toEqual([])
  // Home kept the article; "Original page" in the strip and "view original" of the page open its own address.
  await page.getByTestId('ab-mode-home').tap()
  await expect(frame(page).locator('#title')).toHaveText('半导体出口新规下月生效，日本设备厂商受影响')
  await page.getByTestId('ab-site-original').tap()
  await expect(app).toHaveAttribute('data-mode', 'page')
  await expect(page.getByTestId('ab-address-input')).toHaveValue(ARTICLE_URL.replace('https://', ''))
  await page.getByTestId('ab-mode-home').tap()
  await frame(page).locator('#original').tap()
  await expect(app).toHaveAttribute('data-mode', 'page')
  expect(await asks(page)).toEqual([])
})

test('the strip says what a source needs: a sign-in, an upgrade, why it could not be added, who maintains the site', async ({ page }, testInfo) => {
  const feed = sample('feed') as { status: Record<string, unknown>; updated_at: number }
  await seed(page, {
    readOnly: [AGENT],
    objects: {
      ...siteObjects((site) => { site.maintainer = { agent: 'did:bns:assistant.alice', goal_id: 'g-1' } }),
      // Nikkei was added in the end, and needs the owner to sign in.
      '_data/feed/www-nikkei-com.json': JSON.stringify({ ...feed, source: 'www-nikkei-com', items: [], status: { ...feed.status, state: 'needs_login', pending: null, login_url: 'https://www.nikkei.com/login', error: { kind: 'auth', message: 'the session expired', at: 1764001800000 } } }),
    },
  })
  await openApp(page, `&agent_browser=${AGENT}&route=${encodeURIComponent('s=www-nikkei-com')}`)
  const strip = page.getByTestId('ab-site-strip')
  // `site.json` says the source could not be added: that comes first, with the reason.
  await expect(strip).toHaveAttribute('data-source', 'www-nikkei-com')
  await expect(strip).toHaveAttribute('data-state', 'failed')
  await expect(page.getByTestId('ab-site-status')).toHaveText('Could not be added: 正文需要订阅，只能取到列表')
  await shot(page, testInfo, 'strip-failed')

  // The Agent got it working; the source now waits for the owner's sign-in, which is done on the original site.
  const site = await siteNow(page)
  site.sources.find((source) => source.id === 'www-nikkei-com')!.state = 'active'
  await put(page, SITE_KEY, site)
  await page.getByTestId('ab-home-reload').tap()
  await expect(strip).toHaveAttribute('data-state', 'needs_login')
  await expect(page.getByTestId('ab-site-status')).toContainText('Needs you to sign in on the original site')
  await expect(page.getByTestId('ab-site-status')).toContainText(`Synced ${await moment(page, 1764001800000)}`)
  await shot(page, testInfo, 'strip-needs-login')
  // The page was told that the data changed (`site.updated`) and drew again.
  await expect.poll(async () => Number(await frame(page).locator('body').getAttribute('data-draws'))).toBeGreaterThan(1)

  // A source with an open upgrade request shows it, whatever its feed says.
  await frame(page).locator('#front').tap()
  await expect(home(page)).toHaveAttribute('data-route', '')
  // At the front page: who maintains the site, and that this Agent cannot.
  await expect(page.getByTestId('ab-site-maintainer')).toContainText('This site is maintained by BuckyOS Assistant, not by CodeAssistant.')
  await expect(page.getByTestId('ab-site-read-only')).toContainText('CodeAssistant has read-only access')
  await noHorizontalScroll(page)
  await shot(page, testInfo, 'strip-front-notes')
  await frame(page).locator(`#source-${HN}`).tap()
  await expect(strip).toHaveAttribute('data-state', 'needs_upgrade')
  await expect(page.getByTestId('ab-site-upgrade')).toContainText('Hacker News 的评论页改版了')
  await expect(page.getByTestId('ab-site-upgrade')).toContainText('评论改成按需加载')
  await expect(page.getByTestId('ab-site-maintainer')).toHaveCount(0)
  await shot(page, testInfo, 'strip-upgrade')

  // "Sign in" opens the sign-in page in original page mode.
  await page.getByTestId('ab-back').tap()
  await page.getByTestId('ab-back').tap()
  await expect(strip).toHaveAttribute('data-source', 'www-nikkei-com')
  await expect(page.getByTestId('ab-back')).toBeDisabled()
  await page.getByTestId('ab-site-sign-in').tap()
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'page')
  await expect(page.getByTestId('ab-address-input')).toHaveValue('www.nikkei.com/login')
  // The other Agent's browser is one press away from the note.
  await page.getByTestId('ab-mode-home').tap()
  await frame(page).locator('#front').tap()
  await page.getByTestId('ab-site-maintainer-switch').tap()
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-agent', 'assistant.alice')
  await expect(page.getByTestId('ab-site-maintainer')).toHaveCount(0)
})

test('Reflow on a page the owner has opens it at home; on any other it offers three choices, ordered by what the page looks like', async ({ page }, testInfo) => {
  await seed(page, { objects: siteObjects(), hints: { 'forms.example.org': 'task' } })
  await openApp(page)
  const app = page.getByTestId('agent-browser')
  const button = page.getByTestId('ab-reflow-button')
  await page.getByTestId('ab-mode-page').tap()
  await expect(page.getByTestId('ab-screen')).toHaveAttribute('data-status', 'open')
  // No page, no reflow.
  await expect(button).toBeDisabled()

  // A collected article: its page of the site.
  await browse(page, ARTICLE_URL)
  await expect(button).toHaveAccessibleName('Reflow')
  for (const testId of ['ab-back', 'ab-address-input', 'ab-reflow-button', 'ab-tabs-button']) {
    const box = await page.getByTestId(testId).boundingBox()
    expect(box && box.x >= 0 && box.x + box.width <= 390, testId).toBe(true)
  }
  await shot(page, testInfo, 'page-reflow-button')
  await button.tap()
  await expect(app).toHaveAttribute('data-mode', 'home')
  await expect(home(page)).toHaveAttribute('data-route', `a=${ARTICLE}`)
  await expect(frame(page).locator('#title')).toHaveText('半导体出口新规下月生效，日本设备厂商受影响')
  await expect(page.getByTestId('ab-reflow-sheet')).toHaveCount(0)
  // Another page of a collected website: the page of its source.
  await page.getByTestId('ab-mode-page').tap()
  await browse(page, 'https://news.yahoo.co.jp/categories/it')
  await button.tap()
  await expect(app).toHaveAttribute('data-mode', 'home')
  await expect(home(page)).toHaveAttribute('data-route', `s=${YAHOO}`)
  expect(await rpc(page, 'browser.page_hint')).toEqual([])
  expect(await asks(page)).toEqual([])

  // A page that reads like an article: adding the website comes first.
  await page.getByTestId('ab-mode-page').tap()
  await browse(page, 'https://news.example.net/story/7')
  await button.tap()
  const choices = page.getByTestId('ab-reflow-choices')
  await expect(choices).toHaveAttribute('data-hint', 'reading')
  await expect(choices.getByRole('button')).toHaveText([/Add to my reading/, /Ask CodeAssistant to do something here/, /Tidy this page only/])
  await expect(page.getByTestId('ab-reflow-choice-add')).toHaveAttribute('data-first', 'true')
  await expect(page.getByTestId('ab-reflow-page')).toContainText('news.example.net/story/7')
  expect(await rpc(page, 'browser.page_hint')).toEqual([{}])
  await noHorizontalScroll(page)
  await shot(page, testInfo, 'reflow-choices-reading')
  // "Tidy this page only" asks first, with room for a line to the Agent; nothing is sent before the owner confirms.
  await page.getByTestId('ab-reflow-choice-tidy').tap()
  await expect(page.getByTestId('ab-tidy-sheet')).toHaveAccessibleName('Tidy this page only')
  await expect(page.getByTestId('ab-tidy-page')).toContainText('news.example.net/story/7')
  await shot(page, testInfo, 'tidy-sheet')
  await page.getByTestId('ab-tidy-cancel').tap()
  expect(await asks(page)).toEqual([])

  // A page with a form to fill in: handing it to the Agent comes first, and is the handover of the page.
  await browse(page, 'https://forms.example.org/licence')
  await button.tap()
  await expect(choices).toHaveAttribute('data-hint', 'task')
  await expect(choices.getByRole('button')).toHaveText([/Ask CodeAssistant to do something here/, /Add to my reading/, /Tidy this page only/])
  await shot(page, testInfo, 'reflow-choices-task')
  await page.getByTestId('ab-reflow-choice-task').tap()
  await expect(page.getByTestId('ab-hand-to-agent-sheet')).toBeVisible()
  await expect(page.getByTestId('ab-handover-page')).toContainText('forms.example.org/licence')
  await page.getByTestId('ab-handover-note').fill('Renew my licence')
  await page.getByTestId('ab-handover-confirm').tap()
  await expect.poll(() => rpc(page, 'browser.hand_to_agent')).toEqual([{ group_id: 'default' }])
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'page')

  // Anything else: adding comes first.
  await browse(page, 'https://shop.example.org/')
  await button.tap()
  await expect(choices).toHaveAttribute('data-hint', 'unknown')
  await expect(page.getByTestId('ab-reflow-choice-add')).toHaveAttribute('data-first', 'true')
})

test('adding a website asks the Agent once in the fixed format; home shows it waiting, then being prepared, then lands on its page', async ({ page }, testInfo) => {
  await seed(page, { objects: siteObjects() })
  await openApp(page)
  const app = page.getByTestId('agent-browser')
  await browse(page, 'https://www3.nhk.or.jp/news/')
  await page.getByTestId('ab-reflow-button').tap()
  await page.getByTestId('ab-reflow-choice-add').tap()
  const sheet = page.getByTestId('ab-add-sheet')
  await expect(sheet).toHaveAccessibleName('Add www3.nhk.or.jp to your reading')
  await expect(page.getByTestId('ab-add-page')).toContainText('www3.nhk.or.jp/news')
  await page.getByTestId('ab-add-note').fill('Only the science and culture sections')
  await shot(page, testInfo, 'add-sheet')
  await page.getByTestId('ab-add-confirm').tap()

  // Home, with the request shown at the site's front page.
  await expect(app).toHaveAttribute('data-mode', 'home')
  await expect(home(page)).toHaveAttribute('data-route', '')
  const add = page.getByTestId('ab-site-add')
  await expect(add).toHaveAttribute('data-state', 'asked')
  await expect(add).toContainText('Asked CodeAssistant to add www3.nhk.or.jp')
  expect(await asks(page)).toEqual([[
    'Please add this website to my reading site.',
    'URL: https://www3.nhk.or.jp/news/',
    'Title: www3.nhk.or.jp',
    'Note: Only the science and culture sections',
  ].join('\n')])
  await noHorizontalScroll(page)
  await shot(page, testInfo, 'home-add-asked')

  // The same website is not asked for twice while it is pending: Reflow on it goes back to home.
  await page.getByTestId('ab-mode-page').tap()
  await browse(page, 'https://www3.nhk.or.jp/news/html/20261125/k1.html')
  await page.getByTestId('ab-reflow-button').tap()
  await expect(app).toHaveAttribute('data-mode', 'home')
  await expect(page.getByTestId('ab-reflow-sheet')).toHaveCount(0)
  await expect(add).toHaveCount(1)
  expect(await asks(page)).toHaveLength(1)
  // It survives closing the App, and is not asked for again.
  await openApp(page, '', true)
  await expect(add).toHaveAttribute('data-state', 'asked')
  await expect(add).toContainText('Asked CodeAssistant to add www3.nhk.or.jp')

  // The Work wrote the source as `preparing`: the App sees it by itself.
  const site = await siteNow(page)
  const nhk: Source = { ...site.sources[0], id: 'www3-nhk-or-jp', title: 'NHK 新闻', home_url: 'https://www3.nhk.or.jp/news/', match: [{ host: 'www3.nhk.or.jp' }], job: 'src-www3-nhk-or-jp', state: 'preparing', note: null }
  site.sources.push(nhk)
  await put(page, SITE_KEY, site)
  await expect(add).toHaveAttribute('data-state', 'preparing', { timeout: 10_000 })
  await expect(add).toContainText('CodeAssistant is preparing NHK 新闻')
  await expect(page.getByTestId('ab-site-status')).toHaveText('Websites: 4')
  await shot(page, testInfo, 'home-add-preparing')

  // Handed over and synced once: the request is done, and the owner waiting at the front page lands on the new source.
  nhk.state = 'active'
  await put(page, SITE_KEY, site)
  const feed = sample('feed') as { status: Record<string, unknown> }
  await put(page, '_data/feed/www3-nhk-or-jp.json', { ...feed, source: 'www3-nhk-or-jp', items: [], status: { ...feed.status, pending: null } })
  await expect(add).toHaveCount(0, { timeout: 10_000 })
  await expect(home(page)).toHaveAttribute('data-route', 's=www3-nhk-or-jp')
  await expect(page.getByTestId('ab-site-title')).toHaveText('NHK 新闻')
  await expect(page.getByTestId('ab-site-strip')).toHaveAttribute('data-state', 'ok')
  await expect(frame(page).locator('#title')).toHaveText('NHK 新闻')

  // A website the site has is not asked for at all: "add this website" of its own page goes to its source.
  await frame(page).locator('#add-here').tap()
  await page.getByTestId('ab-add-confirm').tap()
  await expect(home(page)).toHaveAttribute('data-route', 's=www3-nhk-or-jp')
  await frame(page).locator('#front').tap()
  await expect(home(page)).toHaveAttribute('data-route', '')
  await expect(add).toHaveCount(0)
  expect(await asks(page)).toEqual([])
})

test('a request that cannot be sent says so and goes out when the owner tries again; the owner can stop waiting', async ({ page }) => {
  await openApp(page)
  await recordAsks(page, false)
  await browse(page, 'https://blog.example.net/')
  await page.getByTestId('ab-reflow-button').tap()
  await page.getByTestId('ab-reflow-choice-add').tap()
  await page.getByTestId('ab-add-confirm').tap()
  // An empty site shows what is being added with the ways to start.
  await expect(page.getByTestId('ab-site-start')).toBeVisible()
  const add = page.getByTestId('ab-site-add')
  await expect(add).toHaveAttribute('data-state', 'unsent')
  await expect(page.getByTestId('ab-site-notice')).toHaveAttribute('data-kind', 'add_unsent')
  await page.getByTestId('ab-site-notice-dismiss').tap()
  await recordAsks(page)
  await page.getByTestId('ab-site-add-retry').tap()
  await expect(add).toHaveAttribute('data-state', 'asked')
  expect(await asks(page)).toEqual(['Please add this website to my reading site.\nURL: https://blog.example.net/\nTitle: blog.example.net'])
  await page.getByTestId('ab-site-add-remove').tap()
  await expect(add).toHaveCount(0)
})

test('bookmarks are imported from a file of the Zone: one entry per website, none ticked, added one at a time', async ({ page }, testInfo) => {
  const bookmarks = `<!DOCTYPE NETSCAPE-Bookmark-file-1>
<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">
<TITLE>Bookmarks</TITLE><H1>Bookmarks</H1>
<DL><p>
  <DT><H3>News</H3>
  <DL><p>
    <DT><A HREF="https://www3.nhk.or.jp/news/" ADD_DATE="1700000000">NHK ニュース</A>
    <DT><A HREF="https://news.yahoo.co.jp/pickup/1">Yahoo! pickup</A>
    <DT><A HREF="https://www.itmedia.co.jp/news/articles/1.html">ITmedia article one</A>
    <DT><A HREF="https://www.itmedia.co.jp/news/articles/2.html#top">ITmedia article two</A>
    <DT><A HREF="https://arstechnica.com/">Ars Technica</A>
  </DL><p>
  <DT><A HREF="javascript:void(0)">A bookmarklet</A>
  <DT><A HREF="file:///home/alice/notes.html">Local notes</A>
</DL><p>`
  await seed(page, { objects: siteObjects() })
  await openApp(page)
  await putZoneFile(page, '/home/Documents/bookmarks.html', Buffer.from(bookmarks), 'text/html')
  // The site has websites already: its page's "add a website" offers the same three ways.
  await frame(page).locator('#add').tap()
  await expect(page.getByTestId('ab-start-sheet')).toBeVisible()
  await page.getByTestId('ab-start-bookmarks').tap()
  await pickZone(selectDialog(page), '/home/Documents/bookmarks.html')

  const sheet = page.getByTestId('ab-bookmarks-sheet')
  const rows = sheet.getByTestId('ab-bookmark')
  await expect(rows).toHaveCount(4)
  expect(await rows.evaluateAll((items) => items.map((item) => item.getAttribute('data-host')))).toEqual(['www3.nhk.or.jp', 'news.yahoo.co.jp', 'www.itmedia.co.jp', 'arstechnica.com'])
  expect(await rows.evaluateAll((items) => items.map((item) => item.getAttribute('aria-checked')))).toEqual(['false', 'false', 'false', 'false'])
  // What the site has cannot be added again; several bookmarks of one website are one entry.
  await expect(rows.nth(1)).toBeDisabled()
  await expect(rows.nth(1)).toContainText('Already in your reading')
  await expect(rows.nth(2)).toContainText('Bookmarks: 2')
  await expect(rows.nth(0)).toContainText('NHK ニュース')
  await expect(page.getByTestId('ab-bookmarks-add')).toBeDisabled()
  await rows.nth(0).tap()
  await rows.nth(2).tap()
  await expect(page.getByTestId('ab-bookmarks-add')).toHaveText('Add selected: 2')
  await noHorizontalScroll(page)
  await shot(page, testInfo, 'bookmarks')
  await page.getByTestId('ab-bookmarks-add').tap()
  await expect(sheet).toHaveCount(0)
  await expect(page.getByTestId('ab-start-sheet')).toHaveCount(0)

  // One request at a time: the first is asked, the second waits for its turn.
  const adds = page.getByTestId('ab-site-add')
  await expect(adds).toHaveCount(2)
  await expect(adds.nth(0)).toHaveAttribute('data-state', 'asked')
  await expect(adds.nth(1)).toHaveAttribute('data-state', 'queued')
  await expect(adds.nth(1)).toContainText('www.itmedia.co.jp waits for its turn')
  expect(await asks(page)).toEqual(['Please add this website to my reading site.\nURL: https://www3.nhk.or.jp/news/\nTitle: NHK ニュース'])
  await shot(page, testInfo, 'home-add-queue')

  // The first one is being prepared: the second still waits. Once the first is past that (here: it failed), the second goes out.
  const site = await siteNow(page)
  const nhk: Source = { ...site.sources[0], id: 'www3-nhk-or-jp', title: 'NHK', home_url: 'https://www3.nhk.or.jp/news/', match: [{ host: 'www3.nhk.or.jp' }], state: 'preparing', note: null }
  site.sources.push(nhk)
  await put(page, SITE_KEY, site)
  await expect(adds.nth(0)).toHaveAttribute('data-state', 'preparing', { timeout: 10_000 })
  await expect(adds.nth(1)).toHaveAttribute('data-state', 'queued')
  expect(await asks(page)).toHaveLength(1)
  nhk.state = 'failed'
  nhk.note = 'the site refuses automated requests'
  await put(page, SITE_KEY, site)
  // The one that failed is where the owner waiting at the front page lands, with the reason.
  await expect(home(page)).toHaveAttribute('data-route', 's=www3-nhk-or-jp', { timeout: 10_000 })
  await expect(page.getByTestId('ab-site-status')).toHaveText('Could not be added: the site refuses automated requests')
  await expect.poll(async () => (await asks(page))[1]).toBe('Please add this website to my reading site.\nURL: https://www.itmedia.co.jp/\nTitle: www.itmedia.co.jp')
  await frame(page).locator('#front').tap()
  await expect(adds).toHaveCount(1)
  await expect(adds.nth(0)).toHaveAttribute('data-key', 'www-itmedia-co-jp')
  await expect(adds.nth(0)).toHaveAttribute('data-state', 'asked')
})

test('an upgrade request is answered from its link: "Upgrade" asks the Agent in the fixed format and records it, "Not now" only records it', async ({ page }, testInfo) => {
  await seed(page, {
    objects: siteObjects((site) => {
      site.upgrades.push({ id: 'new-template', title: '首页版式要重做', summary: '栏目超过 8 个之后首页在手机上太长，建议改成按主题折叠。', file: 'upgrades/2026-11-25-new-template.md', goal_id: GOAL, opened_at: 1764003000000, state: 'open' })
    }),
  })
  // The notification's link: home, at the source the request is about, with the request open.
  await openApp(page, `&agent_browser=${AGENT}&mode=home&upgrade=hn-comments-layout`)
  expect(new URL(page.url()).search).toBe('?scenario=normal')
  await expect(home(page)).toHaveAttribute('data-route', `s=${HN}`)
  const sheet = page.getByTestId('ab-upgrade-sheet')
  await expect(sheet).toBeVisible()
  await expect(sheet).toContainText('Hacker News')
  await expect(sheet).toContainText('Hacker News 的评论页改版了')
  await expect(sheet).toContainText('评论改成按需加载，现在只能取到前 3 条。要改成用浏览器取，或者不再收评论。')
  await noHorizontalScroll(page)
  await shot(page, testInfo, 'upgrade-sheet')
  expect(await rpc(page, 'browser.ensure')).toEqual([])

  const before = Date.now()
  await page.getByTestId('ab-upgrade-accept').tap()
  await expect(sheet).toHaveCount(0)
  expect(await asks(page)).toEqual([[
    'Please go ahead with this upgrade of my reading site.',
    'Upgrade: hn-comments-layout',
    'File: upgrades/2026-11-24-hn-comments-layout.md',
    `Goal: ${GOAL}`,
  ].join('\n')])
  let site = await siteNow(page)
  const accepted = site.upgrades.find((upgrade) => upgrade.id === 'hn-comments-layout')!
  expect(accepted.state).toBe('accepted')
  expect(accepted.decided_at).toBeGreaterThanOrEqual(before)
  // The rest of the file is as it was.
  expect(site.sources).toHaveLength(3)
  expect(site.template).toEqual((sample('site') as SiteJson).template)
  expect(site.updated_by).toBe('did:bns:alice')
  // The source is no longer waiting for the owner.
  await expect(page.getByTestId('ab-site-upgrade')).toHaveCount(0)
  await expect(page.getByTestId('ab-site-strip')).not.toHaveAttribute('data-state', 'needs_upgrade')

  // A request about no single source shows at the front page; "Not now" writes nothing to the Agent.
  await frame(page).locator('#front').tap()
  const note = page.getByTestId('ab-site-upgrade')
  await expect(note).toHaveAttribute('data-upgrade', 'new-template')
  await expect(note).toContainText('首页版式要重做')
  await shot(page, testInfo, 'strip-front-upgrade')
  await page.getByTestId('ab-site-upgrade-open').tap()
  await page.getByTestId('ab-upgrade-dismiss').tap()
  await expect(sheet).toHaveCount(0)
  await expect(note).toHaveCount(0)
  site = await siteNow(page)
  expect(site.upgrades.map((upgrade) => upgrade.state)).toEqual(['accepted', 'dismissed'])
  expect(await asks(page)).toHaveLength(1)
})

test('an answer is written into site.json as it is now: what the Agent wrote meanwhile is kept', async ({ page }) => {
  await seed(page, { objects: siteObjects() })
  await openApp(page, `&agent_browser=${AGENT}&upgrade=hn-comments-layout`)
  await expect(page.getByTestId('ab-upgrade-sheet')).toBeVisible()
  // The Goal changed a parameter after the App read the file.
  const site = await siteNow(page)
  site.title = '晨读'
  await put(page, SITE_KEY, site)
  await page.getByTestId('ab-upgrade-dismiss').tap()
  await expect(page.getByTestId('ab-upgrade-sheet')).toHaveCount(0)
  const now = await siteNow(page)
  expect(now.title).toBe('晨读')
  expect(now.upgrades[0].state).toBe('dismissed')
})

test('a link opens home where it says: a page of the site, the site’s page for an address, or the original page', async ({ page }) => {
  await seed(page, { objects: siteObjects() })
  await openApp(page, `&agent_browser=${AGENT}&route=${encodeURIComponent(`a=${ARTICLE}`)}`)
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'home')
  await expect(home(page)).toHaveAttribute('data-route', `a=${ARTICLE}`)
  await expect(frame(page).locator('#title')).toHaveText('半导体出口新规下月生效，日本设备厂商受影响')
  await expect(page.getByTestId('ab-site-title')).toHaveText('雅虎日本')

  await openApp(page, `&agent_browser=${AGENT}&url=${encodeURIComponent(ARTICLE_URL)}`)
  await expect(home(page)).toHaveAttribute('data-route', `a=${ARTICLE}`)
  expect(await rpc(page, 'browser.ensure')).toEqual([])

  // A sign-in page a notification links to: original page mode on that address.
  await openApp(page, `&agent_browser=${AGENT}&mode=page&url=${encodeURIComponent('https://www.nikkei.com/login')}`)
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'page')
  await expect(page.getByTestId('ab-address-input')).toHaveValue('www.nikkei.com/login')
})

test('home follows the language', async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem('buckyos.prototype.locale.v1', 'zh-CN'))
  await openApp(page)
  await expect(page.getByRole('tab')).toHaveText(['首页', '原始页面', '对话'])
  await expect(page.getByTestId('ab-site-start')).toContainText('你的阅读站')
  await expect(page.getByTestId('ab-start-points').getByRole('button')).toHaveText([/导入书签/, /打开网站/, /告诉 CodeAssistant/])
  await expect(page.getByTestId('ab-give-task')).toHaveText('交代任务')
})

test('the page is told the theme before it shows, and is signed in again when it says its session ran out', async ({ page }) => {
  await page.addInitScript(() => window.localStorage.setItem('buckyos.prototype.theme.v1', 'dark'))
  await seed(page, { objects: siteObjects() })
  await openApp(page)
  await expect(page.getByTestId('ab-site-frame')).toHaveAttribute('data-drawn', 'true')
  const root = frame(page).locator('html')
  await expect(root).toHaveAttribute('data-theme', 'dark')

  await frame(page).locator(`#source-${YAHOO}`).tap()
  await expect(home(page)).toHaveAttribute('data-route', `s=${YAHOO}`)
  const opened = Number(await root.getAttribute('data-opened'))
  // `site.session`: the App signs the frame in again and comes back to the page the owner was on.
  await root.evaluate(() => (window as unknown as { loseSession(): void }).loseSession())
  await expect(root).toHaveAttribute('data-opened', String(opened + 1))
  await expect(frame(page).locator('body')).toHaveAttribute('data-route', `s=${YAHOO}`)
  await expect(frame(page).locator('#title')).toHaveText('雅虎日本')
  await expect(root).toHaveAttribute('data-theme', 'dark')
  await expect(page.getByTestId('ab-site-frame')).toHaveAttribute('data-drawn', 'true')
  await expect(home(page)).toHaveAttribute('data-route', `s=${YAHOO}`)
  // A page that loses it again right away is not signed in by trying once more: the owner is told, and decides.
  await root.evaluate(() => (window as unknown as { loseSession(): void }).loseSession())
  await expect(page.getByTestId('ab-site-notice')).toHaveAttribute('data-kind', 'session_failed')
  await expect(root).toHaveAttribute('data-opened', String(opened + 1))
  await page.getByTestId('ab-site-session-retry').tap()
  await expect(root).toHaveAttribute('data-opened', String(opened + 2))
  await expect(page.getByTestId('ab-site-notice')).toHaveCount(0)
})

test.describe('in a window', () => {
  test.use({ viewport: { width: 1280, height: 860 }, hasTouch: false, isMobile: false })

  const LINK = `/?agent_browser=${AGENT}&mode=home&upgrade=hn-comments-layout`

  test('the page follows the Desktop’s theme when it changes', async ({ page }) => {
    await seed(page, { objects: siteObjects() })
    await page.goto(`/?scenario=normal&agent_browser=${AGENT}`)
    const root = page.frameLocator('[data-testid="ab-site-frame"]').locator('html')
    await expect(root).toHaveAttribute('data-theme', 'light')
    await openFromAllApps(page, 'settings')
    const settings = page.getByTestId('window-settings')
    await settings.getByRole('button', { name: 'Appearance', exact: true }).click()
    await settings.getByTestId('settings-theme').selectOption('dark')
    await expect(root).toHaveAttribute('data-theme', 'dark')
    await settings.getByTestId('settings-theme').selectOption('light')
    await expect(root).toHaveAttribute('data-theme', 'light')
  })

  test('the App’s own page (a browser tab, the BuckyOS client) opens where its address says', async ({ page }) => {
    await seed(page, { objects: siteObjects() })
    await page.goto(`/app/agent-browser?agent=${AGENT}&mode=home&upgrade=hn-comments-layout`)
    await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-agent', AGENT)
    await expect(home(page)).toHaveAttribute('data-route', `s=${HN}`)
    await expect(page.getByTestId('ab-upgrade-sheet')).toContainText('Hacker News 的评论页改版了')
    await page.goto(`/app/agent-browser?agent=${AGENT}&route=${encodeURIComponent(`a=${ARTICLE}`)}`)
    await expect(home(page)).toHaveAttribute('data-route', `a=${ARTICLE}`)
    await page.goto(`/app/agent-browser?agent=${AGENT}&mode=page&url=${encodeURIComponent('https://www.nikkei.com/login')}`)
    await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'page')
    await expect(page.getByTestId('ab-address-input')).toHaveValue('www.nikkei.com/login')
  })

  test('a notification’s link to the App opens the App’s page from MessageHub’s own page, not the Desktop in its place', async ({ page }) => {
    await page.context().addInitScript((objects) => { (window as unknown as MockWindow).__agentBrowserMockSeed = { objects } }, siteObjects())
    const session = `/messagehub?entityId=${encodeURIComponent('did:web:zone.buckyos.mock')}&sessionId=${encodeURIComponent('sysnotify:app')}`
    await page.goto(session)
    await expect(page.getByTestId('conversation-history')).toBeVisible()
    // The Goal's notification carries the Desktop link of the upgrade request (TODO §4.4).
    await page.evaluate((link) => {
      type Notify = { view: { source: { link?: string } } }
      const backend = (window as unknown as { __messageHubMockNotify: { record(id: string): [Record<string, Notify>, Notify]; save(state: Record<string, Notify>, id: string): void } }).__messageHubMockNotify
      const [state, notify] = backend.record('nt-app-backup')
      notify.view.source.link = link
      backend.save(state, 'nt-app-backup')
    }, LINK)
    await page.goto(session)
    const link = page.locator('[data-testid="notify-card"][data-notify-id="nt-app-backup"]').getByTestId('notify-link')
    await expect(link).toHaveAttribute('href', LINK)
    const opening = page.waitForEvent('popup')
    await link.click()
    const tab = await opening
    await expect(tab.getByTestId('agent-browser')).toHaveAttribute('data-agent', AGENT)
    expect(new URL(tab.url()).pathname).toBe('/app/agent-browser')
    await expect(tab.getByTestId('ab-home')).toHaveAttribute('data-route', `s=${HN}`)
    await expect(tab.getByTestId('ab-upgrade-sheet')).toBeVisible()
    // MessageHub's page stayed where it was.
    expect(new URL(page.url()).pathname).toBe('/messagehub')
  })
})
