import { readFileSync } from 'node:fs'
import { expect, type Page, type TestInfo } from '@playwright/test'
import { openFromAllApps } from './desktop'

/**
 * The personal site of the Agent browser on the mock runtime, for the specs: the shared samples of
 * doc/opendan/protocol/fixtures/site/ put into the mock bucket before the App opens (`window.__agentBrowserMockSeed`),
 * the Agent's side played by writing the bucket (`window.__agentBrowserMock.site`), and `askAgent` replaced by a
 * recorder to see what the Agent is asked.
 */

export const AGENT = 'codeassistant'
export const AGENT_DID = 'did:buckyos:agent:codeassistant'
/** A Goal of the mock Agent (components/conversation/mock/goals.ts), standing for the site's maintenance Goal. */
export const GOAL = 'goal-weekly-report'
export const ARTICLE = '7eec8758d56a244a'
export const ARTICLE_URL = 'https://news.yahoo.co.jp/articles/0a1b2c3d4e5f60718293a4b5c6d7e8f901234567'
export const YAHOO = 'news-yahoo-co-jp'
export const HN = 'news-ycombinator-com'
export const NIKKEI = 'www-nikkei-com'
export const SITE_KEY = '_data/site.json'

export const sample = (name: string) => JSON.parse(readFileSync(new URL(`../../../../../../doc/opendan/protocol/fixtures/site/${name}.json`, import.meta.url), 'utf8')) as Record<string, unknown>

export interface Source { id: string; title: string; home_url: string; match: Array<{ host: string }>; state: string; note: string | null; [field: string]: unknown }
export interface Upgrade { id: string; state: string; decided_at?: number; [field: string]: unknown }
export interface SiteJson { title: string; sources: Source[]; upgrades: Upgrade[]; reading?: { enabled: boolean }; maintainer: { agent: string; goal_id: string } | null; [field: string]: unknown }
export interface Seed { objects?: Record<string, string>; readOnly?: string[]; hints?: Record<string, string> }
interface Ask { text: string }
interface MockControl {
  rpc: Array<{ method: string; params: Record<string, unknown> }>
  log: Array<{ message: { type: string; url?: string } }>
  reflow: { extensions: { askAgent?: (request: Ask) => Promise<void> }; keys(prefix?: string): string[]; opened(): number }
  site: { read(key: string): string | null; put(key: string, text: string | null): void }
  manage: { failJob(jobId: string, kind: string, summary: string): void; job(goalId: string, jobId: string): { enabled: boolean } | null; visits(agentId: string): { enabled: boolean; visits: unknown[] } }
}
export type MockWindow = { __agentBrowserMock: MockControl; __agentBrowserMockSeed?: Seed; __asked?: Ask[] }

/** The shared samples as the bucket holds them, maintained by the mock Agent with one of its Goals; `change` adjusts `site.json`. */
export function siteObjects(change: (site: SiteJson) => void = () => undefined): Record<string, string> {
  const site = sample('site') as SiteJson
  site.maintainer = { agent: AGENT_DID, goal_id: GOAL }
  for (const upgrade of site.upgrades) upgrade.goal_id = GOAL
  change(site)
  return {
    [SITE_KEY]: JSON.stringify(site),
    [`_data/feed/${YAHOO}.json`]: JSON.stringify(sample('feed')),
    [`_data/a/${ARTICLE}.json`]: JSON.stringify(sample('article')),
    '_data/events.json': JSON.stringify(sample('events')),
    '_data/reading.json': JSON.stringify(sample('reading')),
  }
}

export const seed = (page: Page, value: Seed) => page.addInitScript((given) => { (window as unknown as MockWindow).__agentBrowserMockSeed = given }, value)
export const rpc = (page: Page, method: string) =>
  page.evaluate((m) => (window as unknown as MockWindow).__agentBrowserMock.rpc.filter((call) => call.method === m).map((call) => call.params), method)
export const asks = (page: Page) => page.evaluate(() => ((window as unknown as MockWindow).__asked ?? []).map((ask) => ask.text))
export const stored = (page: Page, key: string) => page.evaluate((k) => (window as unknown as MockWindow).__agentBrowserMock.site.read(k), key)
export const put = (page: Page, key: string, value: unknown) =>
  page.evaluate(([k, text]) => (window as unknown as MockWindow).__agentBrowserMock.site.put(k, text), [key, value === null ? null : JSON.stringify(value)] as const)
export const siteNow = async (page: Page) => JSON.parse((await stored(page, SITE_KEY)) ?? 'null') as SiteJson
export const frame = (page: Page) => page.frameLocator('[data-testid="ab-site-frame"]')
export const home = (page: Page) => page.getByTestId('ab-home')
export const moment = (page: Page, ms: number) => page.evaluate((at) => new Intl.DateTimeFormat('en', { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(at)), ms)

/** A picture of the screen once what just came up has stopped moving. */
export async function shot(page: Page, testInfo: TestInfo, name: string) {
  await page.waitForTimeout(350)
  await page.screenshot({ path: testInfo.outputPath(`${name}.png`) })
}

/** What the App asks the Agent is recorded instead of sent (or cannot be sent at all). */
export async function recordAsks(page: Page, available = true) {
  await page.evaluate((on) => {
    const host = window as unknown as MockWindow
    host.__asked ??= []
    if (on) host.__agentBrowserMock.reflow.extensions.askAgent = async (request) => { host.__asked!.push({ text: request.text }) }
    else delete host.__agentBrowserMock.reflow.extensions.askAgent
  }, available)
}

/** The App on the mock Agent's browser on a phone, as it opens: at home. `link`: opened by a Desktop link; `again`: the Agent is remembered from before. */
export async function openApp(page: Page, link = '', again = false) {
  await page.goto(`/?scenario=normal${link}`)
  await expect(page.getByTestId('mobile-taskbar')).toBeAttached()
  if (!link) {
    await openFromAllApps(page, 'agent-browser')
    if (!again) await page.getByTestId(`ab-agent-${AGENT}`).tap()
  }
  const app = page.getByTestId('agent-browser')
  await expect(app).toHaveAttribute('data-agent', AGENT)
  if (await app.getAttribute('data-mode') === 'home') await expect(home(page)).not.toHaveAttribute('data-status', 'loading')
  await recordAsks(page)
}

/** The original page of an address, entered in the bar of whichever mode is on. */
export async function browse(page: Page, address: string) {
  const input = page.getByTestId('ab-address-input')
  await input.tap()
  await input.fill(address)
  await input.press('Enter')
  await expect(page.getByTestId('agent-browser')).toHaveAttribute('data-mode', 'page')
  await expect(page.getByTestId('ab-screen-canvas')).toHaveAttribute('data-seq', /\d+/)
  await expect(input).toHaveValue(address.replace(/^https:\/\//, '').replace(/\/$/, ''))
}

export const noHorizontalScroll = async (page: Page) => expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(390)
