import { mkdirSync, readFileSync } from 'node:fs'
import http from 'node:http'
import { resolve } from 'node:path'
import { expect, test, type FrameLocator, type Page } from '@playwright/test'

const fixtures = resolve(__dirname, '../../../../../doc/opendan/protocol/fixtures/site')
const fixture = (name: string) => JSON.parse(readFileSync(resolve(fixtures, name), 'utf8'))
/** events.json as `site events put` leaves it, each entry with its title, source and time (§3.5). */
const events = () => JSON.parse(readFileSync(resolve(__dirname, 'data/base/events.json'), 'utf8'))
const shots = resolve(__dirname, 'screenshots')

/** 30 minutes after the shared feed was last checked. */
const NOW = 1764003600000
const YAHOO = 'news-yahoo-co-jp'
const CHIPS = '7eec8758d56a244a'
const SNOW = '81c5e462be246971'
const MARKUP = '8064031d77568dc9'
const EXPLAINER = '01eabe90c23c8fe3'
const STORY = 'e-20261124-chip-export'
const CHIPS_TITLE = '半导体出口新规下月生效，日本设备厂商受影响'
const CHIPS_URL = 'https://news.yahoo.co.jp/articles/0a1b2c3d4e5f60718293a4b5c6d7e8f901234567'

interface Entry { method: string; path: string; status: number; bytes: number }
interface RunState { reads: { type: string; origin: string; records: Record<string, unknown>[] }[]; log: Entry[] }

let serial = 0

/** One run of a data case on the test server: its own origin, settings, changed data and log. */
function open(baseURL: string | undefined, name = 'base') {
  const { port } = new URL(baseURL!)
  const id = `r${process.pid}x${Date.now().toString(36)}x${serial++}`
  const control = `http://127.0.0.1:${port}/__test/${id}`
  const host = `${name}.${id}.localhost:${port}`
  const origin = `http://${host}`
  return {
    origin,
    /** A request to this run made outside a browser, with the headers given as they are. */
    send: (method: string, path: string, headers: Record<string, string> = {}, body: string | Buffer = '') => new Promise<number>((done, fail) => {
      const request = http.request({ host: '127.0.0.1', port, path, method, headers: { Host: host, ...headers } }, (response) => {
        response.resume()
        response.on('end', () => done(response.statusCode ?? 0))
      })
      request.on('error', fail)
      request.end(body)
    }),
    url: (route = '') => `${origin}/${route ? `?${route}` : ''}`,
    embedded: (route = '') => `http://localhost:${port}/__parent.html?child=${encodeURIComponent(`${origin}/${route ? `?${route}` : ''}`)}`,
    set: async (settings: Record<string, unknown>) => { await fetch(control, { method: 'POST', body: JSON.stringify(settings) }) },
    state: async (): Promise<RunState> => (await fetch(control)).json(),
  }
}

const feedPath = (source: string) => `/_data/feed/${source}.json`
const rowTitles = (scope: Page | FrameLocator) => scope.locator('.row h3')
const messages = (page: Page) => page.evaluate(() => (window as unknown as { messages: Record<string, unknown>[] }).messages)
const tell = (page: Page, message: Record<string, unknown>) => page.evaluate((m) => (window as unknown as { tell(m: unknown): void }).tell(m), message)
const scrollY = (frame: FrameLocator | Page) => frame.locator('html').evaluate(() => window.scrollY)
const renewals = async (site: ReturnType<typeof open>) => (await site.state()).log.filter((entry) => entry.path === '/sso_refresh').map((entry) => entry.status)
const setVisibility = (page: Page, state: 'hidden' | 'visible') => page.evaluate((value) => {
  Object.defineProperty(document, 'visibilityState', { value, configurable: true })
  document.dispatchEvent(new Event('visibilitychange'))
}, state)

function item(id: string, title: string, at: number, extra: Record<string, unknown> = {}) {
  return { id, object_id: `cyfeed:${'0'.repeat(64)}`, url: `https://example.com/${id}`, title, summary: '', image: null, tags: [], published_at: at, fetched_at: at, rev: 1, ...extra }
}

function feed(source: string, items: unknown[]) {
  return { ...fixture('feed.json'), source, status: { state: 'ok', checked_at: 1764001800000, published_at: 1764001800000, error: null, pending: null, login_url: null }, items }
}

test.describe('home', () => {
  test('shows the columns in order, the latest items, the sync time and a line for every source that is not ok', async ({ page, baseURL }) => {
    const site = open(baseURL)
    await page.clock.setFixedTime(NOW)
    await page.goto(site.url())
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('我的阅读')
    await expect(page).toHaveTitle('我的阅读')
    await expect(page.locator('html')).toHaveAttribute('lang', 'zh-CN')
    await expect(page.locator('.sec-h')).toHaveText(['日本', '科技', '日经'])
    // the third item of the feed is tagged with the one topic the owner hides
    await expect(rowTitles(page)).toHaveText([CHIPS_TITLE, '东京都心今冬首次降雪，交通部分延误'])
    await expect(page.locator('.row .meta').first()).toHaveText('雅虎日本 · 1小时前 · 科技 · 经济')
    await expect(page.locator('.mast .meta')).toContainText('30分钟前同步')
    await expect(page.locator('.note')).toHaveText([
      /雅虎日本\s*2 篇待发布 · 今日翻译额度已用完/,
      /Hacker News\s*需要升级 · Hacker News 的评论页改版了/,
      /日经\s*接入没成功 · 正文需要订阅，只能取到列表/,
    ])
    const thumb = page.locator('img.thumb')
    await expect(thumb).toHaveCount(1)
    await expect(thumb).toHaveAttribute('loading', 'lazy')
    await expect(thumb).toHaveAttribute('src', '/_data/img/b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2.jpg')
    // read articles are told apart (the shared reading.json has both)
    await expect(page.locator('.row.read')).toHaveCount(2)
  })

  test('shows an upgrade request that is about the whole site', async ({ page, baseURL }) => {
    const site = open(baseURL)
    const data = fixture('site.json')
    data.upgrades.push({ ...data.upgrades[0], id: 'new-layout', source: undefined, title: '想换一种首页排法', summary: '最近读得最多的是科技，建议把它排到最前面。' })
    await site.set({ overrides: { '/_data/site.json': data } })
    await page.goto(site.url())
    await expect(page.locator('.card')).toHaveCount(1)
    await expect(page.locator('.card')).toContainText('升级请求')
    await expect(page.locator('.card h2')).toHaveText('想换一种首页排法')
    await expect(page.locator('.card')).toContainText('最近读得最多的是科技，建议把它排到最前面。')
  })

  test('follows per_column, hide_topics and collapsed', async ({ page, baseURL }) => {
    const site = open(baseURL)
    const data = fixture('site.json')
    data.template.params = { home_order: 'time', per_column: 1, hide_topics: [], collapsed: ['tech'] }
    data.columns[1].sources = [YAHOO]
    await site.set({ overrides: { '/_data/site.json': data } })
    await page.goto(site.url())
    const first = page.locator('.sec').nth(0)
    await expect(first.locator('.row h3')).toHaveText([CHIPS_TITLE])
    const folded = page.locator('.sec').nth(1)
    await expect(folded.getByRole('button', { name: '科技' })).toHaveAttribute('aria-expanded', 'false')
    await expect(folded.locator('.row')).toHaveCount(0)
    await folded.getByRole('button', { name: '科技' }).tap()
    await expect(folded.locator('.row')).toHaveCount(1)

    data.template.params = { per_column: 100, hide_topics: [], collapsed: [] }
    await site.set({ overrides: { '/_data/site.json': data } })
    await page.reload()
    await expect(page.locator('.sec').nth(0).locator('.row')).toHaveCount(3)
  })

  test('a collapsed column does not fetch its feeds until it is opened', async ({ page, baseURL }) => {
    const site = open(baseURL)
    const data = fixture('site.json')
    data.template.params.collapsed = ['jp']
    await site.set({ overrides: { '/_data/site.json': data } })
    await page.goto(site.url())
    await expect(page.locator('.note')).toHaveCount(2)
    await page.waitForLoadState('networkidle')
    expect((await site.state()).log.filter((entry) => entry.path === feedPath(YAHOO))).toHaveLength(0)
    await page.getByRole('button', { name: '日本' }).tap()
    await expect(page.locator('.row')).toHaveCount(2)
  })

  test('merges a column by time, by source, and by story', async ({ page, baseURL }) => {
    const site = open(baseURL)
    const data = fixture('site.json')
    data.sources[1].state = 'active'
    data.upgrades = []
    data.columns = [{ id: 'all', title: '全部', sources: [YAHOO, 'news-ycombinator-com'] }]
    data.template.params = { home_order: 'time', per_column: 20, hide_topics: ['体育'], collapsed: [] }
    const told = events()
    told.events[0].articles.push({ id: SNOW, adds: '第二篇报道', title: '东京都心今冬首次降雪，交通部分延误', source: YAHOO, at: 1763996400000 })
    const overrides: Record<string, unknown> = {
      '/_data/site.json': data,
      '/_data/events.json': told,
      [feedPath('news-ycombinator-com')]: feed('news-ycombinator-com', [item('aaaaaaaaaaaaaaa1', 'Between the two', 1763998000000), item('aaaaaaaaaaaaaaa2', 'Oldest', 1763990000000)]),
    }
    await site.set({ overrides })
    await page.goto(site.url())
    const snow = '东京都心今冬首次降雪，交通部分延误'
    await expect(rowTitles(page)).toHaveText([CHIPS_TITLE, 'Between the two', snow, 'Oldest'])
    await expect(page.locator('.row .meta').nth(1)).toContainText('Hacker News')

    data.template.params.home_order = 'source'
    await site.set({ overrides })
    await page.reload()
    await expect(page.locator('.sec .group, .sec .row h3')).toHaveText(['雅虎日本', CHIPS_TITLE, snow, 'Hacker News', 'Between the two', 'Oldest'])

    data.template.params.home_order = 'events'
    await site.set({ overrides })
    await page.reload()
    // the two reports of one story stand together, with the story under them
    await expect(page.locator('.sec .row h3, .sec .thread b')).toHaveText([CHIPS_TITLE, snow, '半导体设备出口新规', 'Between the two', 'Oldest'])
  })

  test('a story that got something new sits under the article the owner read last about it', async ({ page, baseURL }) => {
    const site = open(baseURL)
    await page.clock.setFixedTime(NOW)
    await page.goto(site.url())
    await expect(page.locator('.row')).toHaveCount(2)
    // with the shared reading.json the owner opened the article after the story was last updated
    await expect(page.locator('.thread')).toHaveCount(0)

    const reading = fixture('reading.json')
    reading.articles[CHIPS].last_opened_at = 1764001000000
    await site.set({ overrides: { '/_data/reading.json': reading } })
    await page.reload()
    const block = page.locator('.sec').first()
    await expect(block.locator('.sec-h')).toHaveText('后续')
    await expect(block.locator('.row h3')).toHaveText([CHIPS_TITLE])
    await expect(block.locator('.row + .thread')).toContainText('半导体设备出口新规')
    await expect(block.locator('.thread .adds')).toHaveText('逐条解释了许可的适用范围，原报道没有')
    expect((await site.state()).log.filter((entry) => entry.path.startsWith('/_data/a/'))).toEqual([])

    await block.locator('.thread').tap()
    await expect(page).toHaveURL(site.url(`e=${STORY}`))
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('半导体设备出口新规')
    await page.getByRole('button', { name: '返回' }).tap()
    // having looked at the story, it is not announced again
    await expect(page.locator('.row')).toHaveCount(2)
    await expect(page.locator('.thread')).toHaveCount(0)
  })

  test('an empty site is calm, and offers to add a source only inside the App', async ({ page, baseURL }) => {
    const site = open(baseURL, 'empty')
    await page.goto(site.url())
    await expect(page.locator('.empty h2')).toHaveText('这里还没有内容')
    await expect(page.locator('.empty')).toContainText('在 BuckyOS 的 Agent 浏览器里添加来源。')
    await expect(page.getByRole('button')).toHaveCount(0)

    await page.goto(site.embedded())
    const frame = page.frameLocator('#site')
    await frame.getByRole('button', { name: '添加来源' }).tap()
    await expect.poll(() => messages(page)).toContainEqual({ type: 'site.add' })
  })

  test('sources without articles yet show what is going on', async ({ page, baseURL }) => {
    const site = open(baseURL, 'empty')
    const data = JSON.parse(readFileSync(resolve(__dirname, 'data/empty/site.json'), 'utf8'))
    data.sources = [{ ...fixture('site.json').sources[0], state: 'preparing' }]
    await site.set({ overrides: { '/_data/site.json': data } })
    await page.goto(site.url())
    await expect(page.locator('.empty')).toContainText('来源正在接入，第一批文章很快就到。')
    await expect(page.locator('.note')).toHaveText([/雅虎日本\s*正在接入/])
    await expect(page.locator('.none')).toHaveCount(0)
  })

  test('no site yet', async ({ page, baseURL }) => {
    const site = open(baseURL)
    await site.set({ overrides: { '/_data/site.json': null } })
    await page.goto(site.url())
    await expect(page.locator('#boot')).toHaveText('There is no reading site here yet.')
  })
})

test.describe('source page', () => {
  test('lists every item with the status of the source', async ({ page, baseURL }) => {
    const site = open(baseURL)
    await page.clock.setFixedTime(NOW)
    await page.goto(site.url(`s=${YAHOO}`))
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('雅虎日本')
    await expect(page).toHaveTitle('雅虎日本 · 我的阅读')
    await expect(page.locator('.row')).toHaveCount(3)
    await expect(page.locator('.note')).toHaveText(['正常 · 上次同步：30分钟前', '2 篇待发布 · 今日翻译额度已用完'])
    await expect(page.locator('.head a')).toHaveAttribute('href', 'https://news.yahoo.co.jp/')
  })

  test('shows an open upgrade request', async ({ page, baseURL }) => {
    const site = open(baseURL)
    await page.goto(site.url('s=news-ycombinator-com'))
    await expect(page.locator('.note')).toHaveText(['需要升级 · Hacker News 的评论页改版了 · 还没有同步过'])
    await expect(page.locator('.card')).toContainText('升级请求')
    await expect(page.locator('.card h2')).toHaveText('Hacker News 的评论页改版了')
    await expect(page.locator('.card')).toContainText('评论改成按需加载，现在只能取到前 3 条。')
    await expect(page.locator('.none')).toHaveText('还没有文章。')
  })

  test('an unknown source', async ({ page, baseURL }) => {
    const site = open(baseURL)
    await page.goto(site.url('s=nowhere'))
    await expect(page.locator('.empty')).toContainText('没有这个来源。')
    await page.getByRole('link', { name: '我的阅读' }).last().tap()
    await expect(page).toHaveURL(site.url())
  })
})

test.describe('every display state of a source', () => {
  const HOME: [string, RegExp][] = [
    ['Paused Daily', /Paused$/],
    ['Upgrade Times', /Needs an upgrade · The site moved its articles$/],
    ['Preparing Post', /Being set up$/],
    ['Setup Failed News', /Could not be set up · The text needs a subscription$/],
    ['No Feed Yet', /Being set up$/],
    ['Login Journal', /Sign-in needed\s*Sign in$/],
    ['Broken Herald', /The site changed, waiting for a fix$/],
    ['Budget Weekly', /2 waiting to be published · today’s translation budget is used up$/],
    ['Model Review', /2 waiting to be published · no translation model is available$/],
    ['Retry Courier', /2 waiting to be published · the translation did not work, it is tried again at the next sync$/],
    ['Slow Gazette', /2 waiting to be published · the last sync ran out of time$/],
    ['Images Tribune', /2 waiting to be published · their images are still being fetched$/],
    ['Other Reason', /2 waiting to be published · a reason this template does not know$/],
  ]
  const PAGES: [string, string, RegExp][] = [
    ['fine', 'good', /^Up to date · Last synced /],
    ['paused', '', /^Paused · Last synced /],
    ['upgrade', 'info', /^Needs an upgrade · The site moved its articles · Last synced /],
    ['preparing', '', /^Being set up · Not synced yet$/],
    ['setup-failed', 'bad', /^Could not be set up · The text needs a subscription · Last synced /],
    ['no-feed', '', /^Being set up · Not synced yet$/],
    ['login', 'warn', /^Sign-in needed · Last synced .*Sign in$/],
    ['broken', 'bad', /^The site changed, waiting for a fix · Last synced /],
    ['flaky', 'good', /^Up to date · Last synced /],
  ]

  test('on the home', async ({ page, baseURL }) => {
    const site = open(baseURL, 'states')
    await page.goto(site.url())
    await expect(page.locator('html')).toHaveAttribute('lang', 'en')
    const notes = page.locator('.note')
    await expect(notes).toHaveCount(HOME.length)
    for (const [title, text] of HOME) await expect(notes.filter({ has: page.getByText(title, { exact: true }) })).toHaveText(text)
    await expect(page.locator('.note', { hasText: 'Login Journal' }).getByRole('link', { name: 'Sign in' })).toHaveAttribute('href', 'https://login.example.com/account/sign-in')
    // per_column is 3
    await expect(page.locator('.row')).toHaveCount(3)
  })

  for (const [id, tone, text] of PAGES) {
    test(`on the page of "${id}"`, async ({ page, baseURL }) => {
      const site = open(baseURL, 'states')
      await page.goto(site.url(`s=${id}`))
      const note = page.locator('.note').first()
      await expect(note).toHaveText(text)
      await expect(note.locator('.dot')).toHaveClass(tone ? `dot ${tone}` : /^dot\s*$/)
      if (id === 'flaky') await expect(page.locator('.detail')).toHaveText('connection reset')
      if (id === 'upgrade') await expect(page.locator('.card h2')).toHaveText('The site moved its articles')
    })
  }
})

test.describe('article page', () => {
  test('shows the translation, the pictures, the table, the comments and the links', async ({ page, baseURL }) => {
    const site = open(baseURL)
    await page.clock.setFixedTime(NOW)
    await page.goto(site.url(`a=${CHIPS}`))
    await expect(page.getByRole('heading', { level: 1 })).toHaveText(CHIPS_TITLE)
    await expect(page).toHaveTitle(`${CHIPS_TITLE} · 我的阅读`)
    await expect(page.locator('.head .meta').first()).toHaveText('科技 · 经济')
    await expect(page.locator('.head h1 + .meta')).toHaveText('共同通信 · 2025年11月25日 01:00 · 31分钟前更正')
    await expect(page.locator('.lead')).toHaveText('政府公布了先进制程设备的出口许可细则，多家设备厂商表示正在评估订单。')
    await expect(page.locator('.body > p').first()).toHaveText('政府25日公布了先进半导体制造设备出口许可的细则。')
    await expect(page.locator('.body h2')).toHaveText('涉及的设备')
    await expect(page.locator('.body h3')).toHaveText('今后的日程')
    await expect(page.locator('.body li')).toHaveText(['光刻设备', '刻蚀设备'])
    await expect(page.locator('.body blockquote')).toContainText('正在仔细评估对订单的影响')
    await expect(page.locator('.body pre')).toHaveText('HS 8486.20')
    await expect(page.locator('.body th')).toHaveText(['年度', '出口额'])
    await expect(page.locator('.body td')).toHaveText(['2025', '4.1 万亿日元', '2026', '4.6 万亿日元'])

    const images = page.locator('.body img')
    await expect(images).toHaveCount(2)
    for (const [i, [w, h]] of [[1280, 720], [960, 540]].entries()) {
      await expect(images.nth(i)).toHaveAttribute('loading', 'lazy')
      await expect(images.nth(i)).toHaveAttribute('width', String(w))
      await expect(images.nth(i)).toHaveAttribute('height', String(h))
      const box = (await images.nth(i).boundingBox())!
      expect(box.width / box.height).toBeCloseTo(w / h, 1)
    }
    await expect(images.first()).toHaveAttribute('alt', '工厂里的光刻设备')
    await expect(page.locator('.body figcaption')).toHaveText(['工厂里的光刻设备', '出口额走势图', '记者会现场'])
    await expect(page.locator('.gone')).toHaveText('图片没有取到')

    await expect(page.locator('.cmt')).toHaveCount(2)
    await expect(page.locator('.cmt').first()).toContainText('tak***')
    await expect(page.locator('.cmt').first()).toContainText('比较在意设备厂商的股价。')
    await expect(page.locator('.foot a')).toHaveAttribute('href', CHIPS_URL)
  })

  test('switches to the original and back, and remembers the choice for the session', async ({ page, baseURL }) => {
    const site = open(baseURL)
    await page.goto(site.url(`a=${CHIPS}`))
    const title = page.getByRole('heading', { level: 1 })
    await expect(title).toHaveText(CHIPS_TITLE)
    await expect(title).toHaveAttribute('lang', 'zh-CN')
    await expect(page.getByRole('button', { name: '译文' })).toHaveAttribute('aria-pressed', 'true')

    await page.getByRole('button', { name: '原文' }).tap()
    await expect(title).toHaveText('半導体の輸出新ルール、来月施行へ 国内装置メーカーに影響')
    await expect(title).toHaveAttribute('lang', 'ja')
    await expect(page.locator('.body > p').first()).toHaveText('政府は25日、先端半導体製造装置の輸出許可に関する細則を公表した。')
    await expect(page.locator('.cmt').first()).toContainText('装置メーカーの株価が気になる。')
    await expect(page.locator('.body td').nth(1)).toHaveText('4.1兆円')
    // the summary was written with the translation: the original has none
    await expect(page.locator('.lead')).toHaveCount(0)

    await page.getByRole('link', { name: '関連：都心で初雪' }).tap()
    await expect(page).toHaveURL(site.url(`a=${SNOW}`))
    await expect(title).toHaveText('東京都心で今冬初の降雪、交通の一部に遅れ')
    await page.reload()
    await expect(title).toHaveText('東京都心で今冬初の降雪、交通の一部に遅れ')

    await page.getByRole('button', { name: '译文' }).tap()
    await expect(title).toHaveText('东京都心今冬首次降雪，交通部分延误')
  })

  test('a table wider than the page scrolls inside itself', async ({ page, baseURL }) => {
    const site = open(baseURL)
    await page.goto(site.url(`a=${SNOW}`))
    const table = page.locator('.scroll')
    await expect(table.locator('th')).toHaveCount(6)
    const sizes = await table.evaluate((el) => ({ inner: el.scrollWidth, outer: el.clientWidth, page: document.documentElement.scrollWidth, view: document.documentElement.clientWidth }))
    expect(sizes.inner).toBeGreaterThan(sizes.outer)
    expect(sizes.page).toBeLessThanOrEqual(sizes.view)
  })

  test('a link to a collected article stays in the site, any other opens a new tab', async ({ page, context, baseURL }) => {
    const site = open(baseURL)
    await context.route(/^https:/, (route) => route.fulfill({ contentType: 'text/html', body: 'outside' }))
    await page.goto(site.url(`a=${CHIPS}`))
    await page.evaluate(() => { (window as unknown as { kept: boolean }).kept = true })
    const outside = page.getByRole('link', { name: '经济产业省的公告' })
    await expect(outside).toHaveAttribute('target', '_blank')
    await expect(outside).toHaveAttribute('rel', 'noopener noreferrer')
    const [tab] = await Promise.all([context.waitForEvent('page'), outside.tap()])
    await expect(tab).toHaveURL('https://www.meti.go.jp/press/2026/11/20261125001.html')
    await tab.close()
    const [original] = await Promise.all([context.waitForEvent('page'), page.locator('.foot a').tap()])
    await expect(original).toHaveURL(CHIPS_URL)
    await original.close()
    await expect(page).toHaveURL(site.url(`a=${CHIPS}`))

    await page.getByRole('link', { name: '相关：市中心初雪' }).tap()
    await expect(page).toHaveURL(site.url(`a=${SNOW}`))
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('东京都心今冬首次降雪，交通部分延误')
    expect(await page.evaluate(() => (window as unknown as { kept?: boolean }).kept)).toBe(true)
    await page.goBack()
    await expect(page.getByRole('heading', { level: 1 })).toHaveText(CHIPS_TITLE)
  })

  test('shows what else belongs to the story, and a supplement says what it adds', async ({ page, baseURL }) => {
    const site = open(baseURL)
    await page.goto(site.url(`a=${CHIPS}`))
    const more = page.locator('.sec', { has: page.getByRole('heading', { name: '后续' }) })
    await expect(more.locator('.thread')).toContainText('半导体设备出口新规')
    await expect(more.locator('.line')).toHaveText(/^逐条看半导体出口新规\s*逐条解释了许可的适用范围，原报道没有\s*example\.org$/)
    await more.locator('.line').tap()
    await expect(page).toHaveURL(site.url(`a=${EXPLAINER}`))
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('逐条看半导体出口新规')
    await expect(page.locator('.head .tag')).toHaveText('补充材料')
    await expect(page.locator('.head .adds')).toHaveText('逐条解释了许可的适用范围，原报道没有')
    await expect(page.locator('.bar-up')).toHaveText('半导体设备出口新规')
    await page.locator('.bar-up').tap()
    await expect(page).toHaveURL(site.url(`e=${STORY}`))
  })

  test('an article that is gone, and one that cannot be read right now', async ({ page, baseURL }) => {
    const site = open(baseURL)
    await page.goto(site.url('a=0000000000000000'))
    await expect(page.locator('.empty')).toContainText('这篇文章已经不在了。')

    await page.route('**/_data/a/**', (route) => route.abort())
    await page.goto(site.url(`a=${CHIPS}`))
    await expect(page.locator('.empty')).toContainText('没能读到内容。')
    await page.unroute('**/_data/a/**')
    await page.getByRole('button', { name: '重试' }).tap()
    await expect(page.getByRole('heading', { level: 1 })).toHaveText(CHIPS_TITLE)
  })
})

test('story page: the reports in time order with what each added, then the other material with its source', async ({ page, baseURL }) => {
  const site = open(baseURL)
  const told = events()
  told.events[0].articles.push({ id: SNOW, adds: '更早的一篇', title: '东京都心今冬首次降雪，交通部分延误', source: YAHOO, at: 1763996400000 })
  // an entry nobody filled in keeps its place and shows what it adds
  told.events[0].articles.push({ id: 'cccccccccccccccc', adds: '没有标题的一条' })
  await site.set({ overrides: { '/_data/events.json': told } })
  await page.clock.setFixedTime(NOW)
  await page.goto(site.url(`e=${STORY}`))
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('半导体设备出口新规')
  await expect(page).toHaveTitle('半导体设备出口新规 · 我的阅读')
  await expect(page.locator('.lead')).toHaveText('政府收紧先进制程设备出口，细则下月生效，厂商评估影响。')
  const reports = page.locator('.steps').first().locator('.step')
  await expect(reports.locator('.step-t')).toHaveText(['东京都心今冬首次降雪，交通部分延误', CHIPS_TITLE, '没有标题的一条'])
  await expect(reports.locator('.adds')).toHaveText(['更早的一篇', '公布了细则和生效日期'])
  await expect(reports.locator('.meta')).toHaveText(['雅虎日本 · 2025年11月25日 00:00', '雅虎日本 · 2025年11月25日 01:00', ''])
  const material = page.locator('.sec', { has: page.getByRole('heading', { name: '补充材料' }) }).locator('.step')
  await expect(material.locator('.meta')).toContainText('example.org')
  await expect(material.locator('.step-t')).toHaveText('逐条看半导体出口新规')
  await expect(material.locator('.adds')).toHaveText('逐条解释了许可的适用范围，原报道没有')
  // all of it comes from events.json: no article is fetched to draw the page
  await page.waitForLoadState('networkidle')
  expect((await site.state()).log.filter((entry) => entry.path.startsWith('/_data/a/'))).toEqual([])
  await reports.locator('.step-t').nth(1).tap()
  await expect(page).toHaveURL(site.url(`a=${CHIPS}`))

  await page.goto(site.url('e=e-none'))
  await expect(page.locator('.empty')).toContainText('没有这件事。')
})

test('what comes from a website is text: markup in it never runs', async ({ page, baseURL }) => {
  const site = open(baseURL)
  const outside: string[] = []
  page.on('dialog', () => { throw new Error('a dialog opened') })
  page.on('request', (request) => { if (!request.url().includes('.localhost:')) outside.push(request.url()) })
  await page.goto(site.url(`s=${YAHOO}`))
  await expect(rowTitles(page).last()).toHaveText('<script>alert(1)</script> 标题里的标记只当文字显示')
  await expect(page.locator('.row-sum').last()).toHaveText('用来核对页面不把内容当 HTML 插入。<img src=x onerror=alert(2)>')
  await rowTitles(page).last().locator('a').tap()
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('<script>window.__xss = 1</script> 标题里的标记只当文字显示')
  await expect(page.locator('.body > p').first()).toHaveText('<img src=x onerror="window.__xss = 4"> 正文里的标记')
  await expect(page.locator('.body pre')).toHaveText('</pre><script>window.__xss = 5</script>')
  await expect(page.locator('.body td')).toHaveText('<td onclick="window.__xss = 8">cell</td>')
  await expect(page.locator('.cmt')).toContainText('<svg onload="window.__xss = 10">')
  // neither picture is a key of the bucket: both are placeholders, and nothing is fetched from outside
  await expect(page.locator('.body img')).toHaveCount(0)
  await expect(page.locator('.gone')).toHaveCount(2)
  // only the http(s) link is a link
  const links = page.locator('.lines a')
  await expect(links).toHaveCount(1)
  await expect(links).toHaveText(/<u>https link<\/u>/)
  expect(new URL((await links.getAttribute('href'))!).protocol).toBe('https:')
  await expect(page.locator('#app').getByText('javascript link')).toHaveCount(0)
  await expect(page.locator('#app :is(script, svg:not(.icon), u, b[onmouseover], [onerror], [onclick], [onload], [onmouseover])')).toHaveCount(0)
  await page.getByRole('button', { name: '原文' }).tap()
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('<script>window.__xss = 1</script> タイトルのマークアップ')
  expect(await page.evaluate(() => (window as unknown as { __xss?: number }).__xss)).toBeUndefined()
  expect(outside).toEqual([])
})

test.describe('inside the App', () => {
  test('says when it is drawn and where it is, opens outside links through the App, and goes where the App says', async ({ page, context, baseURL }) => {
    const site = open(baseURL)
    await page.goto(site.embedded())
    const frame = page.frameLocator('#site')
    await expect(rowTitles(frame)).toHaveCount(2)
    await expect.poll(() => messages(page)).toEqual([{ type: 'site.ready' }, { type: 'site.location', route: '', title: '我的阅读' }])

    await rowTitles(frame).first().locator('a').tap()
    await expect(frame.getByRole('heading', { level: 1 })).toHaveText(CHIPS_TITLE)
    await expect.poll(async () => (await messages(page)).at(-1)).toEqual({ type: 'site.location', route: `a=${CHIPS}`, title: CHIPS_TITLE, url: CHIPS_URL, source: YAHOO })

    let tabs = 0
    context.on('page', () => { tabs += 1 })
    await frame.getByRole('link', { name: '经济产业省的公告' }).tap()
    await expect.poll(async () => (await messages(page)).at(-1)).toEqual({ type: 'site.open', url: 'https://www.meti.go.jp/press/2026/11/20261125001.html' })
    await frame.locator('.foot a').tap()
    await expect.poll(async () => (await messages(page)).at(-1)).toEqual({ type: 'site.open', url: CHIPS_URL })
    expect(tabs).toBe(0)

    await tell(page, { type: 'site.navigate', route: `s=${YAHOO}` })
    await expect(frame.getByRole('heading', { level: 1 })).toHaveText('雅虎日本')
    await expect.poll(async () => (await messages(page)).at(-1)).toEqual({ type: 'site.location', route: `s=${YAHOO}`, title: '雅虎日本', source: YAHOO })

    await tell(page, { type: 'site.navigate', route: `e=${STORY}` })
    await expect(frame.getByRole('heading', { level: 1 })).toHaveText('半导体设备出口新规')
    await expect.poll(async () => (await messages(page)).at(-1)).toEqual({ type: 'site.location', route: `e=${STORY}`, title: '半导体设备出口新规' })

    // not a route of the site, and not from the App: both are ignored
    const before = (await messages(page)).length
    await tell(page, { type: 'site.navigate', route: 'x=1' })
    await tell(page, { type: 'site.navigate', route: 'https://example.com/' })
    await frame.locator('html').evaluate(() => window.postMessage({ type: 'site.navigate', route: '' }, '*'))
    await page.waitForTimeout(300)
    await expect(frame.getByRole('heading', { level: 1 })).toHaveText('半导体设备出口新规')
    expect((await messages(page)).length).toBe(before)

    await tell(page, { type: 'site.navigate', route: '' })
    await expect(frame.getByRole('heading', { level: 1 })).toHaveText('我的阅读')
    await expect.poll(async () => (await messages(page)).at(-1)).toEqual({ type: 'site.location', route: '', title: '我的阅读' })
  })

  test('the App can name the theme; without it the system decides', async ({ page, baseURL }) => {
    const site = open(baseURL)
    const frame = page.frameLocator('#site')
    const colours = () => frame.locator('html').evaluate((root) => {
      const style = getComputedStyle(root)
      const names = ['--bg', '--wash', '--text', '--muted', '--line', '--accent', '--on-accent', '--good', '--warn', '--bad']
      return { ...Object.fromEntries(names.map((name) => [name, style.getPropertyValue(name).trim()])), page: style.backgroundColor, text: style.color }
    })
    await page.emulateMedia({ colorScheme: 'dark' })
    await page.goto(site.embedded())
    await expect(rowTitles(frame)).toHaveCount(2)
    const dark = await colours()
    await page.emulateMedia({ colorScheme: 'light' })
    const light = await colours()
    expect(dark.page).toBe('rgb(20, 20, 21)')
    expect(light.page).toBe('rgb(253, 253, 252)')

    await tell(page, { type: 'site.theme', theme: 'dark' })
    await expect(frame.locator('html')).toHaveAttribute('data-theme', 'dark')
    expect(await colours()).toEqual(dark)
    expect(await frame.locator('html').evaluate((root) => getComputedStyle(root).colorScheme)).toBe('dark')
    await page.emulateMedia({ colorScheme: 'dark' })
    await tell(page, { type: 'site.theme', theme: 'light' })
    await expect(frame.locator('html')).toHaveAttribute('data-theme', 'light')
    expect(await colours()).toEqual(light)
    expect(await frame.locator('html').evaluate((root) => getComputedStyle(root).colorScheme)).toBe('light')
    // the theme stays over a change of page, and anything but the two names is ignored
    await rowTitles(frame).first().locator('a').tap()
    await expect(frame.getByRole('heading', { level: 1 })).toHaveText(CHIPS_TITLE)
    await tell(page, { type: 'site.theme', theme: 'sepia' })
    await tell(page, { type: 'site.theme' })
    await page.waitForTimeout(200)
    await expect(frame.locator('html')).toHaveAttribute('data-theme', 'light')
    expect(await colours()).toEqual(light)
  })

  test('standing alone, it takes no messages', async ({ page, baseURL }) => {
    const site = open(baseURL)
    await page.goto(site.url())
    await expect(rowTitles(page)).toHaveCount(2)
    await page.evaluate((route) => window.postMessage({ type: 'site.navigate', route }, '*'), `s=${YAHOO}`)
    await page.evaluate(() => window.postMessage({ type: 'site.theme', theme: 'dark' }, '*'))
    await page.waitForTimeout(300)
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('我的阅读')
    expect(await page.locator('html').getAttribute('data-theme')).toBeNull()
  })

  test('the App’s way back keeps the place in the list', async ({ page, baseURL }) => {
    const site = open(baseURL, 'big')
    await page.goto(site.embedded())
    const frame = page.frameLocator('#site')
    await expect(rowTitles(frame).first()).toBeVisible()
    const target = frame.locator('.row').nth(9)
    await target.scrollIntoViewIfNeeded()
    const y = await scrollY(frame)
    expect(y).toBeGreaterThan(300)
    await target.locator('a').tap()
    await expect.poll(async () => (await messages(page)).at(-1)).toMatchObject({ type: 'site.location', route: expect.stringMatching(/^a=/) })
    await tell(page, { type: 'site.navigate', route: '' })
    await expect.poll(async () => (await messages(page)).at(-1)).toMatchObject({ type: 'site.location', route: '' })
    await expect.poll(() => scrollY(frame)).toBeGreaterThan(y - 3)
    expect(await scrollY(frame)).toBeLessThan(y + 3)
  })

  test('site.updated reads the data again and keeps what the owner was looking at in place', async ({ page, baseURL }) => {
    const site = open(baseURL, 'big')
    await page.goto(site.embedded())
    const frame = page.frameLocator('#site')
    await expect(rowTitles(frame).first()).toBeVisible()
    const kept = frame.locator('.row').nth(7)
    await kept.scrollIntoViewIfNeeded()
    const title = await kept.locator('h3').textContent()
    const top = (await kept.boundingBox())!.y
    const y = await scrollY(frame)

    const current = await frame.locator('html').evaluate(async () => (await fetch('/_data/feed/source-01.json')).json())
    current.items.unshift(item('bbbbbbbbbbbbbbb1', '刚刚发布的一篇', current.items[0].published_at + 60000, { summary: '它排在最前面。' }), item('bbbbbbbbbbbbbbb2', '刚刚发布的另一篇', current.items[0].published_at + 30000))
    await site.set({ overrides: { '/_data/feed/source-01.json': current } })
    await tell(page, { type: 'site.updated' })
    await expect(rowTitles(frame).first()).toHaveText('刚刚发布的一篇')
    const same = frame.locator('.row', { hasText: title! })
    expect(Math.abs((await same.boundingBox())!.y - top)).toBeLessThan(2)
    expect(await scrollY(frame)).toBeGreaterThan(y + 100)
    // a page that did not change is not announced again
    expect((await messages(page)).filter((message) => message.type === 'site.location')).toHaveLength(1)

    // the site's own definition is read again too; another template version takes a reload
    const data = await frame.locator('html').evaluate(async () => (await fetch('/_data/site.json')).json())
    data.title = '改了名字'
    await site.set({ overrides: { '/_data/site.json': data } })
    await tell(page, { type: 'site.updated' })
    await expect(frame.getByRole('heading', { level: 1 })).toHaveText('改了名字')
    expect(await scrollY(frame)).toBeGreaterThan(y + 100)
    data.template.version = 't2'
    await site.set({ overrides: { '/_data/site.json': data } })
    await tell(page, { type: 'site.updated' })
    await expect.poll(async () => (await site.state()).log.some((entry) => entry.path === '/_app/t2/app.js')).toBe(true)
    await expect(frame.getByRole('heading', { level: 1 })).toHaveText('改了名字')
  })
})

test.describe('reading records', () => {
  const read = async (page: Page, ms: number) => {
    await expect(page.getByRole('heading', { level: 1 })).toHaveText(CHIPS_TITLE)
    await page.waitForTimeout(ms)
  }
  test('leaving an article reports the visible time and how far it was read', async ({ page, baseURL }) => {
    const site = open(baseURL)
    const started = Date.now()
    await page.goto(site.url(`a=${CHIPS}`))
    await read(page, 2200)
    await page.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight))
    await page.waitForTimeout(200)
    await page.getByRole('link', { name: '相关：市中心初雪' }).tap()
    await expect.poll(async () => (await site.state()).reads.length).toBe(1)
    const { reads, log } = await site.state()
    // the host took it: a JSON body from this origin, one whole record
    expect(log.filter((entry) => entry.path === '/_read').map((entry) => entry.status)).toEqual([204])
    expect(reads[0].type).toBe('application/json')
    expect(reads[0].origin).toBe(site.origin)
    expect(reads[0].records).toHaveLength(1)
    expect(Object.keys(reads[0].records[0]).sort()).toEqual(['article', 'dwell_ms', 'max_scroll', 'opened_at', 'source'])
    const [record] = reads[0].records as { article: string; source: string; opened_at: number; dwell_ms: number; max_scroll: number }[]
    expect(record).toMatchObject({ article: CHIPS, source: YAHOO, max_scroll: 100 })
    expect(record.opened_at).toBeGreaterThanOrEqual(started)
    expect(record.opened_at).toBeLessThanOrEqual(Date.now())
    expect(record.dwell_ms).toBeGreaterThanOrEqual(2200)
    expect(record.dwell_ms).toBeLessThan(6000)
  })

  test('a page that is unloaded reports too, with how far the article was on screen', async ({ page, baseURL }) => {
    const site = open(baseURL)
    await page.goto(site.url(`a=${CHIPS}`))
    await read(page, 2100)
    await page.goto('about:blank')
    await expect.poll(async () => (await site.state()).reads.length).toBe(1)
    const record = (await site.state()).reads[0].records[0] as { max_scroll: number }
    expect(record.max_scroll).toBeGreaterThan(5)
    expect(record.max_scroll).toBeLessThan(60)
  })

  test('time the page was hidden does not count, and a visit that goes on reports only what was added', async ({ page, baseURL }) => {
    const site = open(baseURL)
    await page.goto(site.url(`a=${CHIPS}`))
    await read(page, 2100)
    await setVisibility(page, 'hidden')
    await expect.poll(async () => (await site.state()).reads.length).toBe(1)
    await page.waitForTimeout(1500)
    await setVisibility(page, 'visible')
    await page.waitForTimeout(600)
    await page.getByRole('button', { name: '返回' }).tap()
    await expect.poll(async () => (await site.state()).reads.length).toBe(2)
    const [first, second] = (await site.state()).reads.map((batch) => batch.records[0] as { opened_at: number; dwell_ms: number })
    expect(second.opened_at).toBe(first.opened_at)
    expect(first.dwell_ms).toBeGreaterThanOrEqual(2100)
    expect(second.dwell_ms).toBeGreaterThanOrEqual(600)
    expect(second.dwell_ms).toBeLessThan(1400)
  })

  test('nothing is reported for a glance, or when reading records are off', async ({ page, baseURL }) => {
    const site = open(baseURL)
    await page.goto(site.url(`a=${CHIPS}`))
    await read(page, 500)
    await page.getByRole('button', { name: '返回' }).tap()
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('雅虎日本')

    const data = fixture('site.json')
    data.reading.enabled = false
    await site.set({ overrides: { '/_data/site.json': data } })
    await page.goto(site.url(`a=${CHIPS}`))
    await read(page, 2300)
    await setVisibility(page, 'hidden')
    await page.goto('about:blank')
    await page.waitForTimeout(500)
    const state = await site.state()
    expect(state.reads).toEqual([])
    expect(state.log.filter((entry) => entry.path === '/_read')).toEqual([])
  })

  test('an added piece is reported under _supplement, and an article with another kind of id is not reported', async ({ page, baseURL }) => {
    const site = open(baseURL)
    const odd = { ...fixture('article.json'), id: 'not-sixteen-hex' }
    await site.set({ overrides: { '/_data/a/not-sixteen-hex.json': odd } })
    await page.goto(site.url(`a=${EXPLAINER}`))
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('逐条看半导体出口新规')
    await page.waitForTimeout(2100)
    await page.goto(site.url('a=not-sixteen-hex'))
    await expect.poll(async () => (await site.state()).reads.length).toBe(1)
    expect((await site.state()).reads[0].records[0]).toMatchObject({ article: EXPLAINER, source: '_supplement' })
    await read(page, 2100)
    await page.goto('about:blank')
    await page.waitForTimeout(400)
    expect((await site.state()).log.filter((entry) => entry.path === '/_read').map((entry) => entry.status)).toEqual([204])
  })

  test('the host takes whole records from its own pages and nothing else', async ({ baseURL }) => {
    const site = open(baseURL)
    const { hostname } = new URL(site.origin)
    const record = { article: CHIPS, source: YAHOO, opened_at: NOW, dwell_ms: 42000, max_scroll: 100 }
    const report = (body: unknown, headers: Record<string, string> = { Origin: site.origin }) =>
      site.send('POST', '/_read', headers, typeof body === 'string' || Buffer.isBuffer(body) ? body : JSON.stringify(body))
    for (const good of [[record], fixture('read_batch.json'), [{ ...record, source: '_supplement' }], Array(50).fill(record)]) expect(await report(good)).toBe(204)
    // whose page it is, is told by the host name alone
    expect(await report([record], { Origin: `https://${hostname}:1` })).toBe(204)
    for (const headers of [{}, { Origin: 'null' }, { Origin: 'http://localhost' }, { Origin: hostname }]) expect(await report([record], headers), JSON.stringify(headers)).toBe(403)
    expect(await report(Buffer.alloc((16 << 10) + 1, ' '))).toBe(413)
    const refused = [
      [], {}, 'not json', record, [record, null], Array(51).fill(record),
      [{ ...record, dwell_ms: 1.5 }], [{ ...record, dwell_ms: -1 }], [{ ...record, opened_at: String(NOW) }], [{ ...record, max_scroll: 101 }],
      [{ ...record, article: CHIPS.toUpperCase() }], [{ ...record, article: CHIPS.slice(1) }], [{ ...record, source: '-x' }], [{ ...record, source: 'Yahoo' }],
      [{ ...record, note: 'more' }], [{ article: CHIPS, source: YAHOO, opened_at: NOW, dwell_ms: 1 }],
    ]
    for (const body of refused) expect(await report(body), JSON.stringify(body).slice(0, 80)).toBe(400)
    // nothing else is written on this host, and without a session nothing is looked at
    expect(await site.send('PUT', '/_read', { Origin: site.origin }, JSON.stringify([record]))).toBe(405)
    expect(await site.send('POST', '/', { Origin: site.origin }, JSON.stringify([record]))).toBe(405)
    await site.set({ auth: 'locked' })
    expect(await report([record], {})).toBe(401)
    expect((await site.state()).reads).toHaveLength(5)
  })

  test('a record the host refuses does not disturb the page', async ({ page, baseURL }) => {
    const site = open(baseURL)
    const errors: string[] = []
    page.on('pageerror', (error) => errors.push(error.message))
    await page.route('**/_read', (route) => route.abort())
    await page.goto(site.url(`a=${CHIPS}`))
    await read(page, 2100)
    await page.getByRole('button', { name: '返回' }).tap()
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('雅虎日本')
    await expect(page.locator('.row')).toHaveCount(3)
    expect(errors).toEqual([])
  })
})

test.describe('weak network', () => {
  test('the first screen of ten sources with twenty items each is within 100 KB without the pictures', async ({ page, baseURL }) => {
    const site = open(baseURL, 'big')
    await page.goto(site.url())
    await expect(rowTitles(page).first()).toBeVisible()
    await page.waitForLoadState('networkidle')
    const { log } = await site.state()
    const counted = log.filter((entry) => !entry.path.startsWith('/_data/img/'))
    const total = counted.reduce((sum, entry) => sum + entry.bytes, 0)
    console.log(`first screen: ${total} bytes\n${counted.map((entry) => `  ${String(entry.bytes).padStart(6)}  ${entry.status}  ${entry.path}`).join('\n')}`)
    expect(total).toBeLessThanOrEqual(100 * 1024)
    // only the first column was fetched, each feed once
    expect(counted.filter((entry) => entry.path.startsWith('/_data/feed/')).map((entry) => entry.path).sort())
      .toEqual(['source-01', 'source-02', 'source-03'].map(feedPath))
    expect(counted.some((entry) => entry.path.endsWith('/article.js'))).toBe(false)
    await expect(page.locator('.sec').first().locator('.row')).toHaveCount(20)

    // the next columns come as the owner gets near them
    expect((await site.state()).log.some((entry) => entry.path === feedPath('source-04'))).toBe(false)
    await expect(async () => {
      await page.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight))
      await expect(page.locator('.row')).toHaveCount(80, { timeout: 500 })
    }).toPass()
  })

  test('with every source in one column the first screen needs every feed', async ({ page, baseURL }) => {
    const site = open(baseURL, 'wide')
    await page.goto(site.url())
    await expect(page.locator('.row')).toHaveCount(20)
    await page.waitForLoadState('networkidle')
    const counted = (await site.state()).log.filter((entry) => !entry.path.startsWith('/_data/img/'))
    console.log(`one column of ten sources: ${counted.reduce((sum, entry) => sum + entry.bytes, 0)} bytes`)
    expect(counted.filter((entry) => entry.path.startsWith('/_data/feed/'))).toHaveLength(10)
  })

  test('what was read last time shows at once, and is replaced when the host answers', async ({ page, baseURL }) => {
    const site = open(baseURL)
    await page.goto(site.url())
    await expect(rowTitles(page).first()).toHaveText(CHIPS_TITLE)
    await page.waitForLoadState('networkidle')

    const next = fixture('feed.json')
    next.items[0].title = '改过的标题'
    await site.set({ delay: 2500, slow: '/_data/feed/', overrides: { [feedPath(YAHOO)]: next } })
    await page.reload()
    await expect(rowTitles(page).first()).toHaveText(CHIPS_TITLE, { timeout: 1500 })
    await expect(page.locator('.mast .meta')).toContainText('正在更新…')
    await expect(rowTitles(page).first()).toHaveText('改过的标题')
    await expect(page.locator('.mast .meta')).not.toContainText('正在更新…')
  })

  test('when the host cannot be reached the old list stays, with a way to try again', async ({ page, baseURL }) => {
    const site = open(baseURL)
    await page.goto(site.embedded())
    const frame = page.frameLocator('#site')
    await expect(frame.locator('.sec[data-done]')).toHaveCount(3)
    await page.route('**/_data/**', (route) => route.abort())
    await tell(page, { type: 'site.updated' })
    await expect(frame.locator('.note', { hasText: '没能更新，这是之前读到的内容。' })).toBeVisible()
    await expect(rowTitles(frame)).toHaveCount(2)
    await page.unroute('**/_data/**')
    await frame.getByRole('button', { name: '重试' }).tap()
    await expect(frame.locator('.note', { hasText: '没能更新' })).toHaveCount(0)
    await expect(rowTitles(frame)).toHaveCount(2)
  })

  test('going back to the list returns to where the owner was', async ({ page, baseURL }) => {
    const site = open(baseURL, 'big')
    await page.goto(site.url())
    const target = page.locator('.row').nth(12)
    await target.scrollIntoViewIfNeeded()
    const y = await scrollY(page)
    await target.locator('a').tap()
    await expect(page.locator('.bar')).toBeVisible()
    expect(await scrollY(page)).toBe(0)
    await page.getByRole('button', { name: '返回' }).tap()
    await expect(page.locator('.row').nth(12)).toBeVisible()
    await expect.poll(() => scrollY(page)).toBe(y)
  })
})

test.describe('session', () => {
  test('a 401 is answered by renewing the session and asking again', async ({ page, baseURL }) => {
    const site = open(baseURL)
    // the session ends between the page and its first file
    await site.set({ drop: true })
    await page.goto(site.url())
    await expect(rowTitles(page)).toHaveCount(2)
    expect(await renewals(site)).toEqual([204])

    // later, in the middle of reading: the next file the page needs is the article module
    await expect(page.locator('.sec[data-done]')).toHaveCount(3)
    await site.set({ auth: 'expired' })
    await rowTitles(page).first().locator('a').tap()
    await expect(page.getByRole('heading', { level: 1 })).toHaveText(CHIPS_TITLE)
    expect(await renewals(site)).toEqual([204, 204])

    // opening an address without a session: the host's own page renews it and comes back
    await site.set({ auth: 'expired' })
    await page.goto(site.url(`s=${YAHOO}`))
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('雅虎日本')
  })

  test('standing alone, a session that cannot be renewed reloads the page, and the host takes it to sign in', async ({ page, baseURL }) => {
    const site = open(baseURL)
    await page.goto(site.url())
    await expect(page.locator('.sec[data-done]')).toHaveCount(3)
    await site.set({ auth: 'locked' })
    await rowTitles(page).first().locator('a').tap()
    await expect(page.locator('#login-stub')).toBeVisible()
    expect(new URL(page.url()).pathname).toBe('/login')
    expect(new URL(page.url()).searchParams.get('redirect_url')).toBe(site.url(`a=${CHIPS}`))
  })

  test('inside the App it says so once, and keeps what it shows', async ({ page, baseURL }) => {
    const site = open(baseURL)
    await page.goto(site.embedded())
    const frame = page.frameLocator('#site')
    const asked = async () => (await messages(page)).filter((message) => message.type === 'site.session').length
    await expect(frame.locator('.sec[data-done]')).toHaveCount(3)
    await frame.locator('html').evaluate(() => { (window as unknown as { kept: boolean }).kept = true })
    await site.set({ auth: 'locked' })
    await tell(page, { type: 'site.updated' })
    await expect.poll(asked).toBe(1)
    await expect(frame.locator('.note', { hasText: '没能更新，这是之前读到的内容。' })).toBeVisible()
    await expect(rowTitles(frame)).toHaveCount(2)
    await rowTitles(frame).first().locator('a').tap()
    await expect(frame.locator('.empty')).toContainText('没能读到内容。')
    expect(await frame.locator('html').evaluate(() => (window as unknown as { kept?: boolean }).kept)).toBe(true)
    expect(await asked()).toBe(1)

    // the App signed the frame in again: what was missing is read, and a later loss is said again
    await site.set({ auth: 'open' })
    await frame.getByRole('button', { name: '重试' }).tap()
    await expect(frame.getByRole('heading', { level: 1 })).toHaveText(CHIPS_TITLE)
    await site.set({ auth: 'locked' })
    await tell(page, { type: 'site.updated' })
    await expect.poll(asked).toBe(2)
    await expect(frame.getByRole('heading', { level: 1 })).toHaveText(CHIPS_TITLE)
  })

  test('inside the App, a page that cannot start for want of a session says that and not that it is drawn', async ({ page, baseURL }) => {
    const site = open(baseURL)
    await site.set({ drop: 'locked' })
    await page.goto(site.embedded())
    await expect(page.frameLocator('#site').locator('#boot')).toBeVisible()
    expect(await messages(page)).toEqual([{ type: 'site.session' }])
  })

  test('the session is renewed every ten minutes while the page is visible', async ({ page, baseURL }) => {
    const site = open(baseURL)
    const minutes = (count: number) => page.clock.fastForward(count * 60_000)
    await page.clock.install({ time: new Date(NOW) })
    await page.goto(site.url())
    await expect(rowTitles(page)).toHaveCount(2)
    await minutes(9)
    await page.waitForTimeout(200)
    expect(await renewals(site)).toEqual([])
    await minutes(1.1)
    await expect.poll(() => renewals(site)).toEqual([204])
    await minutes(10.1)
    await expect.poll(() => renewals(site)).toEqual([204, 204])

    // hidden, nothing is renewed; coming back after more than ten minutes renews at once
    await setVisibility(page, 'hidden')
    await minutes(25)
    await page.waitForTimeout(200)
    expect(await renewals(site)).toHaveLength(2)
    await setVisibility(page, 'visible')
    await expect.poll(() => renewals(site)).toHaveLength(3)

    // coming back sooner waits for the rest of the ten minutes
    await setVisibility(page, 'hidden')
    await minutes(4)
    await setVisibility(page, 'visible')
    await minutes(5)
    await page.waitForTimeout(200)
    expect(await renewals(site)).toHaveLength(3)
    await minutes(1.1)
    await expect.poll(() => renewals(site)).toHaveLength(4)
  })
})

test.describe('languages', () => {
  const LANGUAGES = ['en', 'zh-CN', 'zh-TW', 'ja', 'ko', 'fr', 'de', 'es', 'ru', 'ar']
  // a string, so that the test runner does not rewrite the import
  const words = (page: Page, name: string): Promise<Record<string, string>> =>
    page.evaluate(`import('/_app/t1/${name === 'en' ? 'i18n.js' : `lang/${name}.js`}').then((module) => module.default ?? module.en)`)

  test('every language has every text', async ({ page, baseURL }) => {
    const site = open(baseURL)
    await page.goto(site.url())
    const english = Object.keys(await words(page, 'en')).sort()
    for (const name of LANGUAGES) expect(Object.keys(await words(page, name)).sort(), name).toEqual(english)
  })

  for (const language of [...LANGUAGES, 'zh-Hant-HK', 'pt-BR']) {
    test(`the interface follows the site language: ${language}`, async ({ page, baseURL }) => {
      const site = open(baseURL, 'states')
      const data = JSON.parse(readFileSync(resolve(__dirname, 'data/states/site.json'), 'utf8'))
      data.language = language
      await site.set({ overrides: { '/_data/site.json': data } })
      await page.clock.setFixedTime(NOW)
      await page.goto(site.url())
      const file = language === 'zh-Hant-HK' ? 'zh-TW' : language === 'pt-BR' ? 'en' : language
      const text = await words(page, file)
      await expect(page.locator('html')).toHaveAttribute('lang', language)
      await expect(page.locator('html')).toHaveAttribute('dir', language === 'ar' ? 'rtl' : 'ltr')
      await expect(page.locator('.note', { hasText: 'Login Journal' })).toContainText(text['st.needs_login'])
      await expect(page.locator('.note', { hasText: 'Login Journal' }).getByRole('link').last()).toHaveText(text.signIn)
      await expect(page.locator('.note', { hasText: 'Budget Weekly' })).toContainText(text['why.translate_budget'])
      await expect(page.locator('.note', { hasText: 'Retry Courier' })).toContainText(text['why.translate_failed'])
      // the date and the times are written by Intl in that language
      const expected = await page.evaluate((tag) => new Intl.DateTimeFormat(tag, { weekday: 'long', month: 'long', day: 'numeric' }).format(Date.now()), language)
      await expect(page.locator('.mast .meta')).toContainText(expected)
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth)).toBe(true)
    })
  }
})

const PAGES: [name: string, data: string, route: string][] = [
  ['home', 'base', ''],
  ['source', 'base', `s=${YAHOO}`],
  ['source-upgrade', 'base', 's=news-ycombinator-com'],
  ['article', 'base', `a=${CHIPS}`],
  ['article-table', 'base', `a=${SNOW}`],
  ['article-markup', 'base', `a=${MARKUP}`],
  ['article-supplement', 'base', `a=${EXPLAINER}`],
  ['story', 'base', `e=${STORY}`],
  ['empty', 'empty', ''],
  ['states', 'states', ''],
  ['states-login', 'states', 's=login'],
  ['big', 'big', ''],
]

async function settle(page: Page) {
  await page.waitForLoadState('networkidle')
  await expect(page.locator('.wait').first()).not.toBeInViewport()
  await page.evaluate(() => Promise.all([...document.images].filter((image) => image.loading !== 'lazy' || image.getBoundingClientRect().top < 4000).map((image) => image.decode().catch(() => {}))))
}

test.describe('layout', () => {
  for (const [name, data, route] of PAGES) {
    test(`${name}: nothing is wider than a 375 px screen, and everything to tap is at least 44 px`, async ({ page, baseURL }) => {
      const site = open(baseURL, data)
      await page.goto(site.url(route))
      await settle(page)
      expect(await page.evaluate(() => document.documentElement.clientWidth)).toBe(375)
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(375)
      const small = await page.evaluate(() => [...document.querySelectorAll<HTMLElement>('#app a[href], #app button')].map((el) => {
        const box = (el.classList.contains('row-link') ? el.closest('.row')! : el).getBoundingClientRect()
        return { text: (el.textContent || el.getAttribute('aria-label') || '').trim().slice(0, 30), width: Math.round(box.width), height: Math.round(box.height) }
      }).filter((box) => box.width < 44 || box.height < 44))
      expect(small).toEqual([])
    })
  }

  for (const scheme of ['light', 'dark'] as const) {
    test(`screenshots: ${scheme}`, async ({ page, baseURL }) => {
      test.setTimeout(90_000)
      mkdirSync(shots, { recursive: true })
      await page.emulateMedia({ colorScheme: scheme })
      await page.clock.setFixedTime(NOW)
      for (const [name, data, route] of PAGES) {
        const site = open(baseURL, data)
        if (name === 'home') {
          const reading = fixture('reading.json')
          reading.articles[CHIPS].last_opened_at = 1764001000000
          delete reading.articles[SNOW]
          await site.set({ overrides: { '/_data/reading.json': reading } })
        }
        await page.goto(site.url(route))
        await settle(page)
        await page.screenshot({ path: resolve(shots, `${name}-${scheme}.png`), fullPage: true })
        if (name === 'article') {
          await page.getByRole('button', { name: '原文' }).tap()
          await page.screenshot({ path: resolve(shots, `article-original-${scheme}.png`) })
          await page.getByRole('button', { name: '译文' }).tap()
        }
      }
      const background = await page.evaluate(() => getComputedStyle(document.documentElement).backgroundColor)
      expect(background).toBe(scheme === 'dark' ? 'rgb(20, 20, 21)' : 'rgb(253, 253, 252)')

      await page.setViewportSize({ width: 1280, height: 800 })
      for (const [name, route] of [['home', ''], ['article', `a=${CHIPS}`]]) {
        await page.goto(open(baseURL).url(route))
        await settle(page)
        await page.screenshot({ path: resolve(shots, `wide-${name}-${scheme}.png`), fullPage: true })
        const column = (await page.locator('#app').boundingBox())!
        expect(column.width).toBeLessThanOrEqual(712)
        expect(Math.abs(column.x - (1280 - column.width) / 2)).toBeLessThan(10)
      }
    })
  }

  for (const [theme, system] of [['dark', 'light'], ['light', 'dark']] as const) {
    test(`screenshots: the App's ${theme} theme on a ${system} system`, async ({ page, baseURL }) => {
      mkdirSync(shots, { recursive: true })
      await page.setViewportSize({ width: 375, height: 1500 })
      await page.emulateMedia({ colorScheme: system })
      await page.clock.setFixedTime(NOW)
      for (const [name, data, route] of [['home', 'base', ''], ['article', 'base', `a=${CHIPS}`], ['story', 'base', `e=${STORY}`], ['states', 'states', '']]) {
        await page.goto(open(baseURL, data).embedded(route))
        const frame = page.frameLocator('#site')
        await expect(frame.getByRole('heading', { level: 1 })).toBeVisible()
        await tell(page, { type: 'site.theme', theme })
        await expect(frame.locator('html')).toHaveAttribute('data-theme', theme)
        await page.waitForLoadState('networkidle')
        await expect(frame.locator('.wait').first()).not.toBeInViewport()
        await page.screenshot({ path: resolve(shots, `theme-${theme}-on-${system}-${name}.png`) })
      }
    })
  }

  for (const language of ['de', 'ru', 'ar']) {
    test(`an article with the interface in ${language} fits a 375 px screen`, async ({ page, baseURL }) => {
      const site = open(baseURL)
      await site.set({ overrides: { '/_data/site.json': { ...fixture('site.json'), language } } })
      await page.goto(site.url(`a=${CHIPS}`))
      await settle(page)
      await expect(page.locator('.seg button')).toHaveCount(2)
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(375)
      for (const route of ['', `s=${YAHOO}`, `e=${STORY}`]) {
        await page.goto(site.url(route))
        await settle(page)
        expect(await page.evaluate(() => document.documentElement.scrollWidth), route).toBeLessThanOrEqual(375)
      }
    })
  }

  test('right to left', async ({ page, baseURL }) => {
    mkdirSync(shots, { recursive: true })
    const site = open(baseURL, 'states')
    const data = JSON.parse(readFileSync(resolve(__dirname, 'data/states/site.json'), 'utf8'))
    data.language = 'ar'
    await site.set({ overrides: { '/_data/site.json': data } })
    await page.clock.setFixedTime(NOW)
    await page.goto(site.url())
    await settle(page)
    const dot = (await page.locator('.note .dot').first().boundingBox())!
    const text = (await page.locator('.note .note-text').first().boundingBox())!
    expect(dot.x).toBeGreaterThan(text.x)
    await page.screenshot({ path: resolve(shots, 'states-ar-light.png'), fullPage: true })
  })
})
