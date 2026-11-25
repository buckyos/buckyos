/* Home mode: the owner's personal site (Personal Site Protocol), read from the reflow bucket and shown from
 * `reflow.<zone>` in a frame. Nothing here starts the browser or writes to the Agent by itself: the site is read
 * with the owner's session, and the Agent is asked only when the owner adds or removes a website, accepts an upgrade
 * or wants a page tidied. What the owner switches (a source paused, synced now, the reading records) goes to the
 * maintenance Goal's jobs and `site.json` directly, without the Agent. One controller per App window. It keeps where
 * the owner is in the site (a route, or a page tidied by itself), the websites the Agent was asked to add or remove
 * until `site.json` shows it, and what the shell says about the page being read. */

import { getBrowserBackend, type BrowserAgent } from '../api/backend'
import type { PageHint } from '../api/types'
import { agentBrowserExtensions } from '../extensions'
import { readReflowState, reflowControllerOf, type ReflowController } from '../reflow/controller'
import { normalizeUrl, siteOfHost } from '../reflow/library'
import { getReflowSource } from '../reflow/source'
import type { AgentBrowserStore } from '../store/store'
import { decideUpgrade, displayState, locate, readArticle, readFeed, readSite, setReadingEnabled, setSourcePaused, sourceOf, type ArticleRef, type Feed, type Site, type SiteSource } from './data'
import { callFailure, readJobs, runJob, setJobEnabled, type Job } from './manage'
import { addSourceMessage, removeSourceMessage, upgradeMessage } from './messages'

/** Where the owner is in home mode: a page of the site (its route, the query string without `?`), or a page tidied by itself. */
export type Place =
  | { kind: 'site'; route: string }
  | { kind: 'page'; url: string; title: string }

/** A website the owner wants added, until `site.json` has it as a source that is past `preparing`. */
export interface PendingAdd {
  /** The source id a website gets by default (its host with dashes): the same website is not asked for twice. */
  key: string
  /** The Agent that is asked. */
  agentId: string
  url: string
  host: string
  title: string
  note: string
  /** `queued`: waits for its turn (one website is added at a time); `asked`: the request is in the conversation. */
  state: 'queued' | 'asked'
  /** The request could not be sent; it goes out when the owner tries again. */
  failed?: boolean
}

export type SiteSheet =
  | { kind: 'add'; url: string; title: string }
  | { kind: 'start' }
  | { kind: 'upgrade'; id: string }

export interface SiteNotice {
  id: number
  kind: 'add_unsent' | 'add_again' | 'upgrade_unsent' | 'upgrade_unsaved' | 'remove_unsent' | 'action_failed' | 'session_failed'
  /** The website or upgrade it is about; for `action_failed` the service's own sentence. */
  detail: string
}

/** The pages of "manage": the websites, the maintenance Goal, what is recorded. */
export type ManageTab = 'sources' | 'goal' | 'records'

/** What the owner just asked for a source, while it is under way. */
export type SourceAction = 'sync' | 'pause' | 'resume' | 'remove'

export interface SiteSnapshot {
  /** `absent`: the user has no site yet; `failed`: the bucket could not be read and nothing was read before. */
  status: 'loading' | 'ready' | 'absent' | 'failed'
  site: Site | null
  /** The feeds that were looked at (the source being read, the sources being added); null: the source has none yet. */
  feeds: Record<string, Feed | null>
  place: Place
  canGoBack: boolean
  /** What the site's frame loads (a route) and the count that loads it again. */
  frame: { view: string; key: number }
  /** The article of the current route. */
  article: ArticleRef | null
  adds: PendingAdd[]
  sheet: SiteSheet | null
  notice: SiteNotice | null
  /** "Manage" covers the site, on this page. */
  manage: ManageTab | null
  /** The jobs of the maintenance Goal, read while "manage" is open; null: not read (yet), or this Agent does not maintain the site. */
  jobs: Job[] | null
  /** Sources with a request of the owner under way. */
  acting: Record<string, SourceAction>
  /** Sources a sync was asked for, by when: until a run that started after it shows. */
  synced: Record<string, number>
  /** Sources the Agent was asked to remove, until `site.json` has them no more. */
  removing: string[]
}

/** Whether the Agent whose browser this is maintains the site: its Goal's jobs are this Agent's service's. */
export function maintainedBy(site: Site | null, agent: BrowserAgent | null): 'this' | 'other' | 'none' {
  if (!site?.maintainer?.agent || !site.maintainer.goal_id) return 'none'
  return agent && site.maintainer.agent === agent.agentDid ? 'this' : 'other'
}

/** The job that syncs a source (`src-<source>` unless `site.json` names another). */
export const jobOf = (source: SiteSource): string => source.job || `src-${source.id}`

/** What the site's page tells the App (Personal Site Protocol §6.2). */
type PageMessage = { type?: unknown; route?: unknown; url?: unknown }

const ADDS_KEY = 'buckyos.agent-browser.site.adds'
const REMOVING_KEY = 'buckyos.agent-browser.site.removing'
const HISTORY_MAX = 50
const POLL_MS = 30_000
/** While a website is being added, or a sync the owner asked for is awaited. */
const WAITING_POLL_MS = 5000
/** How long a sync that was asked for is awaited before the row stops saying so. */
const SYNC_WAIT_MS = 120_000
/** A page that asks for its session again this soon after it was signed in is not signed in by trying once more. */
const SESSION_RETRY_MS = 15_000

function rememberedAdds(): PendingAdd[] {
  try {
    const list = JSON.parse(window.localStorage.getItem(ADDS_KEY) ?? '[]') as PendingAdd[]
    return Array.isArray(list) ? list.filter((item) => item && typeof item.key === 'string' && typeof item.url === 'string') : []
  } catch {
    return []
  }
}

function rememberAdds(adds: PendingAdd[]) {
  try {
    window.localStorage.setItem(ADDS_KEY, JSON.stringify(adds.map((add) => ({ ...add, failed: undefined }))))
  } catch { /* no storage */ }
}

function rememberedRemoving(): string[] {
  try {
    const list = JSON.parse(window.localStorage.getItem(REMOVING_KEY) ?? '[]') as unknown
    return Array.isArray(list) ? list.filter((item): item is string => typeof item === 'string') : []
  } catch {
    return []
  }
}

function rememberRemoving(ids: string[]) {
  try {
    window.localStorage.setItem(REMOVING_KEY, JSON.stringify(ids))
  } catch { /* no storage */ }
}

const routeOfPlace = (place: Place): string | null => (place.kind === 'site' ? place.route : null)
const samePlace = (a: Place, b: Place): boolean => (a.kind === 'site' ? b.kind === 'site' && a.route === b.route : b.kind === 'page' && a.url === b.url)
const routeParam = (route: string, name: string): string | null => new URLSearchParams(route).get(name)

/** The source `site.json` has for a website that was asked for: by what it matches, or by the default id. */
export function sourceOfAdd(site: Site | null, add: PendingAdd): SiteSource | null {
  return site ? sourceOf(site, add.url) ?? site.sources.find((source) => source.id === add.key) ?? null : null
}

/** The source the shell's strip is about: the one of the route, or of the article being read. */
export function currentSource(snapshot: SiteSnapshot): SiteSource | null {
  const route = routeOfPlace(snapshot.place)
  if (route === null || !snapshot.site) return null
  const id = routeParam(route, 's') ?? (snapshot.article && routeParam(route, 'a') === snapshot.article.id ? snapshot.article.source : null)
  return snapshot.site.sources.find((source) => source.id === id) ?? null
}

export class SiteController {
  private readonly store: AgentBrowserStore
  private readonly reflow: ReflowController
  private snapshot: SiteSnapshot = {
    status: 'loading', site: null, feeds: {}, place: { kind: 'site', route: '' }, canGoBack: false, frame: { view: '', key: 0 }, article: null,
    adds: rememberedAdds(), sheet: null, notice: null, manage: null, jobs: null, acting: {}, synced: {}, removing: rememberedRemoving(),
  }
  private readonly listeners = new Set<() => void>()
  private history: Place[] = []
  private agentId = ''
  private stopStore: (() => void) | null = null
  private shown = false
  private timer = 0
  private reading: Promise<void> | null = null
  private again = false
  private etag = ''
  /** The owner asked for the data to be read again: the page is told even when nothing changed. */
  private tell = false
  /** What a launch asked for beyond the route, until the site was read. */
  private wanted: { url?: string; upgrade?: string } = {}
  private post: ((message: { type: string; route?: string; theme?: string }) => void) | null = null
  /** The site's page said it is drawn: it takes `site.navigate` from here on. */
  private ready = false
  private theme: 'light' | 'dark' = 'light'
  /** When the frame was last signed in again because its page asked (`site.session`). */
  private sessionAt = 0
  /** The route the site's page shows, as it last said. */
  private shownRoute = ''
  private sending = false
  private readonly sentUpgrades = new Set<string>()
  private noticeId = 0

  constructor(store: AgentBrowserStore) {
    this.store = store
    this.reflow = reflowControllerOf(store)
    this.reflow.setOpenLink((url) => void this.openUrl(url))
  }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  getSnapshot = () => this.snapshot

  private set(patch: Partial<SiteSnapshot>) {
    this.snapshot = { ...this.snapshot, ...patch }
    for (const listener of [...this.listeners]) listener()
  }

  private notify(kind: SiteNotice['kind'], detail: string) {
    this.set({ notice: { id: ++this.noticeId, kind, detail } })
  }

  dismissNotice() {
    if (this.snapshot.notice) this.set({ notice: null })
  }

  // ── The App window ──

  /** The App shows an Agent's browser: the site is read, and websites being added are followed in every mode. */
  attach() {
    if (this.stopStore) return
    this.stopStore = this.store.subscribe(() => this.follow())
    this.follow()
    this.schedule()
  }

  detach() {
    this.stopStore?.()
    this.stopStore = null
    window.clearTimeout(this.timer)
  }

  /** Home mode is on the stage. */
  setShown(shown: boolean) {
    if (shown === this.shown) return
    this.shown = shown
    if (shown && !this.reading) void this.load()
    else this.schedule()
  }

  private follow() {
    const agentId = this.store.getSnapshot().agent?.agentId ?? ''
    if (agentId === this.agentId) return
    this.agentId = agentId
    if (!agentId) return
    const { route = '', url, upgrade } = this.store.takeLaunch()
    this.wanted = { url, upgrade }
    this.history = []
    this.ready = false
    this.set({ place: { kind: 'site', route }, canGoBack: false, frame: { view: route, key: this.snapshot.frame.key + 1 }, article: null, sheet: null, notice: null, manage: null, jobs: null, acting: {}, synced: {} })
    void this.load()
  }

  // ── The site's data ──

  /** Reads the site again; a read under way is followed by one more. */
  load(): Promise<void> {
    if (this.reading) {
      this.again = true
      return this.reading
    }
    this.reading = this.read().finally(() => {
      this.reading = null
      if (this.again) {
        this.again = false
        void this.load()
      } else {
        void this.pump()
        this.schedule()
      }
    })
    return this.reading
  }

  private async read() {
    const first = this.snapshot.status === 'loading' || this.snapshot.status === 'failed'
    try {
      const library = await (await getReflowSource()).library()
      const file = await readSite(library)
      const site = file?.site ?? null
      const route = routeOfPlace(this.snapshot.place)
      const articleId = route === null ? null : routeParam(route, 'a')
      const article = articleId === null ? null : this.snapshot.article?.id === articleId ? this.snapshot.article : await readArticle(library, articleId).catch(() => null)
      const wanted = new Set<string>()
      const reading = (route === null ? null : routeParam(route, 's')) ?? article?.source
      if (site && reading && site.sources.some((source) => source.id === reading)) wanted.add(reading)
      for (const add of this.snapshot.adds) {
        const source = sourceOfAdd(site, add)
        if (source?.state === 'active') wanted.add(source.id)
      }
      // The list of websites shows each one's state, which its feed has.
      const listed = this.snapshot.manage === 'sources'
      if (listed) for (const source of site?.sources ?? []) wanted.add(source.id)
      const feeds: Record<string, Feed | null> = {}
      await Promise.all([...wanted].map(async (id) => { feeds[id] = await readFeed(library, id).catch(() => this.snapshot.feeds[id] ?? null) }))
      const { agent } = this.store.getSnapshot()
      const jobs = this.snapshot.manage && this.snapshot.manage !== 'records' && site?.maintainer && agent && maintainedBy(site, agent) === 'this'
        ? await readJobs(agent, site.maintainer.goal_id).catch(() => null)
        : this.snapshot.jobs
      const before = this.snapshot.feeds
      const changed = (file?.etag ?? '') !== this.etag || [...wanted].some((id) => id in before && before[id]?.updated_at !== feeds[id]?.updated_at)
      this.etag = file?.etag ?? ''
      // The owner may have moved on while this was read: what was read about another page is not shown for this one.
      const now = routeOfPlace(this.snapshot.place)
      this.set({ status: site ? 'ready' : 'absent', site, feeds: { ...before, ...feeds }, jobs, ...(now === route ? { article } : {}) })
      if ((changed || this.tell) && !first) this.post?.({ type: 'site.updated' })
      this.tell = false
      this.settleAdds()
      this.settleSources()
      if (first) this.takeWanted()
    } catch {
      if (first) this.set({ status: 'failed' })
    }
  }

  private schedule() {
    window.clearTimeout(this.timer)
    if (!this.stopStore) return
    const waiting = this.snapshot.adds.some((add) => add.state === 'asked') || Object.keys(this.snapshot.synced).length > 0
    if (waiting || this.shown) this.timer = window.setTimeout(() => void this.load(), waiting ? WAITING_POLL_MS : POLL_MS)
  }

  /** The owner wants what is shown read again: the site's data (the page reads its own again), or the tidied page made again. */
  reload() {
    if (this.snapshot.place.kind === 'page') { void this.reflow.redo(); return }
    this.tell = true
    void this.load()
  }

  private takeWanted() {
    const { url, upgrade } = this.wanted
    this.wanted = {}
    const found = upgrade ? this.snapshot.site?.upgrades.find((item) => item.id === upgrade) : undefined
    if (found) {
      this.go(found.source ? `s=${found.source}` : '')
      if (found.state === 'open') this.set({ sheet: { kind: 'upgrade', id: found.id } })
    } else if (url) void this.openUrl(url)
  }

  // ── Where the owner is ──

  /** A page of the site. */
  go(route: string) {
    this.moveTo({ kind: 'site', route })
  }

  /** A page tidied by itself, with what the library has for it. */
  showPage(url: string, title = '') {
    this.moveTo({ kind: 'page', url, title })
  }

  back() {
    const place = this.history.pop()
    if (place) this.enter(place)
  }

  private moveTo(place: Place) {
    const before = this.snapshot.place
    if (!samePlace(before, place)) this.history = [...this.history, before].slice(-HISTORY_MAX)
    this.enter(place)
  }

  private enter(place: Place) {
    let { frame } = this.snapshot
    if (place.kind === 'page') this.reflow.show(place.url, place.title)
    // A page that is drawn goes there itself and keeps what it has loaded; before that, the frame loads the route.
    else if (this.post && this.ready) { if (this.shownRoute !== place.route) this.post({ type: 'site.navigate', route: place.route }) }
    else if (frame.view !== place.route) frame = { view: place.route, key: frame.key + 1 }
    const kept = place.kind === 'site' && this.snapshot.article && routeParam(place.route, 'a') === this.snapshot.article.id
    this.set({ place, frame, canGoBack: this.history.length > 0, ...(kept ? {} : { article: null }) })
    void this.load()
  }

  /** Whether the site already shows this address (an article, or the page of its source). */
  private async routeOf(url: string): Promise<string | null> {
    const { site } = this.snapshot
    if (!site) return null
    try {
      return await locate(await (await getReflowSource()).library(), site, url)
    } catch {
      return null
    }
  }

  /** Whether a page was tidied for this address before (`browser.reflow_state`, which starts no browser). */
  private async hasPage(url: string): Promise<boolean> {
    const { agent } = this.store.getSnapshot()
    if (!agent) return false
    try {
      return (await readReflowState(agent, url)).view !== null
    } catch {
      return false
    }
  }

  /** An address from a link or a launch: its page of the site, the page tidied for it, or the original site. The Agent is not asked. */
  async openUrl(url: string) {
    await this.settled()
    const route = await this.routeOf(url)
    if (route !== null) { this.go(route); return }
    if (await this.hasPage(url)) { this.showPage(url); return }
    this.store.openInPage(url)
  }

  /** The first read of the site is over. */
  private async settled() {
    while (this.snapshot.status === 'loading') await this.load()
  }

  /**
   * The Reflow button of the original page. What the owner already has opens in home mode: the site's page for the
   * address, the website being added, the page tidied for it (or asked of the Agent). Otherwise the owner chooses, and
   * `browser.page_hint` says what to offer first.
   */
  async reflowPage(url: string, title: string): Promise<'opened' | PageHint['hint']> {
    await this.settled()
    const open = (go: () => void) => { go(); this.store.setMode('home'); return 'opened' as const }
    const route = await this.routeOf(url)
    if (route !== null) return open(() => this.go(route))
    if (this.addOf(url)) return open(() => this.go(''))
    if (this.reflow.waitsFor(url) || await this.hasPage(url)) return open(() => this.showPage(url, title))
    const { agent } = this.store.getSnapshot()
    if (!agent) return 'unknown'
    try {
      return (await (await getBrowserBackend()).call<PageHint>(agent, 'browser.page_hint', {})).hint
    } catch {
      return 'unknown'
    }
  }

  /** The owner wants this page tidied by itself, maybe with a line for the Agent: it is asked of the Agent and waited for in home mode. */
  tidyPage(url: string, title: string, note: string) {
    this.showPage(url, title)
    this.store.setMode('home')
    void this.reflow.request(note)
  }

  // ── The site's page ──

  /** The frame's way to the site's page, while it shows one. */
  setPage(post: ((message: { type: string; route?: string; theme?: string }) => void) | null) {
    this.post = post
    this.ready = false
    if (post) this.shownRoute = this.snapshot.frame.view
  }

  /** The App's light or dark theme: the page is told once it is drawn, and when it changes. */
  setTheme(theme: 'light' | 'dark') {
    if (theme === this.theme) return
    this.theme = theme
    if (this.ready) this.post?.({ type: 'site.theme', theme })
  }

  /** Signs the frame in on the pages' origin again and loads the page the owner is on. */
  reopen() {
    const route = routeOfPlace(this.snapshot.place) ?? this.snapshot.frame.view
    this.sessionAt = Date.now()
    this.set({ frame: { view: route, key: this.snapshot.frame.key + 1 }, notice: null })
  }

  /** A message of the site's page; the frame has checked where it came from. */
  onPageMessage(data: unknown) {
    const message = data as PageMessage | null
    if (!message || typeof message !== 'object') return
    switch (message.type) {
      case 'site.ready':
        this.ready = true
        this.post?.({ type: 'site.theme', theme: this.theme })
        break
      case 'site.session':
        // The page's session ran out and it could not renew it: the sign-in is the App's to do again.
        if (Date.now() - this.sessionAt < SESSION_RETRY_MS) this.notify('session_failed', '')
        else this.reopen()
        break
      case 'site.location':
        if (typeof message.route === 'string') this.located(message.route)
        break
      case 'site.open':
        if (typeof message.url === 'string' && /^https?:\/\//i.test(message.url)) this.store.openInPage(message.url)
        break
      case 'site.add':
        this.set({ sheet: typeof message.url === 'string' && message.url ? { kind: 'add', url: message.url, title: '' } : { kind: 'start' } })
        break
    }
  }

  /** The page says where it is: a link the owner followed in it, or its own way back. */
  private located(route: string) {
    this.shownRoute = route
    const { place } = this.snapshot
    if (place.kind !== 'site' || place.route === route) return
    const last = this.history.at(-1)
    if (last?.kind === 'site' && last.route === route) this.history = this.history.slice(0, -1)
    else this.history = [...this.history, place].slice(-HISTORY_MAX)
    this.set({ place: { kind: 'site', route }, canGoBack: this.history.length > 0, article: null })
    void this.load()
  }

  openSheet(sheet: SiteSheet | null) {
    this.set({ sheet })
  }

  // ── Managing the site ──

  /** "Manage" over the site: the websites with their states, the maintenance Goal, what is recorded. */
  openManage(tab: ManageTab) {
    this.set({ manage: tab })
    this.store.setMode('home')
    void this.load()
  }

  closeManage() {
    this.set({ manage: null })
  }

  /** The Goal and job of a source, when this Agent maintains the site. */
  private jobTarget(id: string): { agent: BrowserAgent; goalId: string; jobId: string; source: SiteSource } | null {
    const { site } = this.snapshot
    const { agent } = this.store.getSnapshot()
    const source = site?.sources.find((item) => item.id === id)
    if (!site?.maintainer || !agent || !source || maintainedBy(site, agent) !== 'this') return null
    return { agent, goalId: site.maintainer.goal_id, jobId: jobOf(source), source }
  }

  /** Runs what the owner asked for a source; while it runs the source says so, and a failure shows the service's reason. */
  private async act(id: string, action: SourceAction, run: () => Promise<void>) {
    if (this.snapshot.acting[id]) return
    this.set({ acting: { ...this.snapshot.acting, [id]: action } })
    try {
      await run()
    } catch (error) {
      this.notify('action_failed', callFailure(error))
    }
    this.set({ acting: Object.fromEntries(Object.entries(this.snapshot.acting).filter(([key]) => key !== id)) })
    await this.load()
  }

  /** A sync of the source now (after a failure, or once the owner signed in): its job runs out of turn, without the Agent. */
  async syncSource(id: string) {
    const target = this.jobTarget(id)
    if (!target) return
    await this.act(id, 'sync', async () => {
      const asked = Date.now()
      await runJob(target.agent, target.goalId, target.jobId)
      this.set({ synced: { ...this.snapshot.synced, [id]: asked } })
    })
  }

  /** A website that could not be added: the Agent is asked to try again, with the message it was asked with the first time. */
  async retrySource(id: string) {
    const target = this.jobTarget(id)
    if (!target) return
    await this.act(id, 'sync', async () => {
      const { title, home_url: url } = target.source
      try {
        const send = agentBrowserExtensions.askAgent
        const context = await this.store.whenConversation()
        if (!send || !context) throw Error('the conversation is not available')
        await send({ text: addSourceMessage({ url, title, note: '' }) }, context)
      } catch {
        this.notify('add_unsent', title)
        return
      }
      this.notify('add_again', title)
    })
  }

  /** Pauses a source or resumes it: its job is switched, then `site.json` says so. */
  async pauseSource(id: string, paused: boolean) {
    const target = this.jobTarget(id)
    if (!target) return
    await this.act(id, paused ? 'pause' : 'resume', async () => {
      await setJobEnabled(target.agent, target.goalId, target.jobId, !paused)
      await setSourcePaused(await (await getReflowSource()).library(), id, paused)
    })
  }

  /** Removing a source takes its program, its job and its place in the Goal with it: the Agent is asked, once. */
  async removeSource(id: string) {
    const target = this.jobTarget(id)
    if (!target || this.snapshot.removing.includes(id)) return
    await this.act(id, 'remove', async () => {
      try {
        const send = agentBrowserExtensions.askAgent
        const context = await this.store.whenConversation()
        if (!send || !context) throw Error('the conversation is not available')
        await send({ text: removeSourceMessage({ id, title: target.source.title, url: target.source.home_url }) }, context)
      } catch {
        this.notify('remove_unsent', target.source.title)
        return
      }
      this.setRemoving([...this.snapshot.removing, id])
    })
  }

  private setRemoving(removing: string[]) {
    rememberRemoving(removing)
    this.set({ removing })
  }

  /** The reading records switched on or off in `site.json`; false when it was not written. */
  async setReading(enabled: boolean): Promise<boolean> {
    try {
      await setReadingEnabled(await (await getReflowSource()).library(), enabled)
    } catch (error) {
      this.notify('action_failed', callFailure(error))
      return false
    }
    await this.load()
    return true
  }

  /** What was asked for is settled once the site shows it: a source that is gone, a run that started after the sync was asked for. */
  private settleSources() {
    const { site, jobs, removing, synced } = this.snapshot
    if (!site) return
    const left = removing.filter((id) => site.sources.some((source) => source.id === id))
    if (left.length !== removing.length) this.setRemoving(left)
    const now = Date.now()
    const waiting = Object.fromEntries(Object.entries(synced).filter(([id, asked]) => {
      const source = site.sources.find((item) => item.id === id)
      const job = source && jobs?.find((item) => item.job_id === jobOf(source))
      return !!source && now - asked < SYNC_WAIT_MS && !(job?.last_run && !job.running && job.last_run.at_ms >= asked)
    }))
    if (Object.keys(waiting).length !== Object.keys(synced).length) this.set({ synced: waiting })
  }

  // ── Adding websites ──

  private addOf(url: string): PendingAdd | null {
    try {
      const key = siteOfHost(normalizeUrl(url).host)
      return this.snapshot.adds.find((add) => add.key === key) ?? null
    } catch {
      return null
    }
  }

  private setAdds(adds: PendingAdd[]) {
    rememberAdds(adds)
    this.set({ adds })
  }

  /** Puts a website in the queue unless the site has it or it is waiting already. */
  private enqueue({ url, title, note }: { url: string; title: string; note: string }): 'queued' | 'known' | 'pending' | 'invalid' {
    const agentId = this.agentId
    let address: { url: string; host: string }
    let key: string
    try {
      address = normalizeUrl(url)
      key = siteOfHost(address.host)
    } catch {
      return 'invalid'
    }
    if (!agentId) return 'invalid'
    if (this.snapshot.site && sourceOf(this.snapshot.site, address.url)) return 'known'
    if (this.snapshot.adds.some((add) => add.key === key)) return 'pending'
    this.setAdds([...this.snapshot.adds, { key, agentId, url: address.url, host: address.host, title: title.trim(), note: note.trim(), state: 'queued' }])
    return 'queued'
  }

  /** The owner adds a website to the site: the Agent is asked in the conversation (after the ones asked before), and home shows it waiting. */
  requestAdd(entry: { url: string; title: string; note: string }) {
    const outcome = this.enqueue(entry)
    const source = outcome === 'known' && this.snapshot.site ? sourceOf(this.snapshot.site, entry.url) : null
    this.go(source ? `s=${source.id}` : '')
    this.store.setMode('home')
    void this.pump()
  }

  /** Websites chosen from a bookmarks file: asked of the Agent one at a time. */
  queueAdds(entries: Array<{ url: string; title: string }>) {
    for (const entry of entries) this.enqueue({ ...entry, note: '' })
    void this.pump()
  }

  /** The owner stops waiting for a website (what the Agent already started goes on). */
  removeAdd(key: string) {
    this.setAdds(this.snapshot.adds.filter((add) => add.key !== key))
    void this.pump()
  }

  retryAdd(key: string) {
    this.setAdds(this.snapshot.adds.map((add) => (add.key === key ? { ...add, failed: false } : add)))
    void this.pump()
  }

  /** A website that was asked for is done once its source is past `preparing`: the next one goes out, and the owner waiting at the site's front page lands on the new source. */
  private settleAdds() {
    const { site, feeds, adds, place } = this.snapshot
    if (!site) return
    let landed: string | null = null
    const left = adds.filter((add) => {
      const source = sourceOfAdd(site, add)
      if (!source) return true
      if (add.state === 'asked' && displayState(site, source, feeds[source.id] ?? null) === 'preparing') return true
      if (add.state === 'asked') landed = source.id
      return false
    })
    if (left.length === adds.length) return
    this.setAdds(left)
    if (landed && this.shown && place.kind === 'site' && place.route === '') this.go(`s=${landed}`)
  }

  /** Sends the next request when none is under way; not before the site was read, which may have the website already. */
  private async pump() {
    if (this.sending || this.snapshot.status === 'loading') return
    const { adds } = this.snapshot
    const { agent } = this.store.getSnapshot()
    if (!agent || adds.some((add) => add.state === 'asked')) return
    const next = adds.find((add) => add.state === 'queued' && !add.failed && add.agentId === agent.agentId)
    if (!next) return
    this.sending = true
    let sent = false
    try {
      const send = agentBrowserExtensions.askAgent
      const context = await this.store.whenConversation()
      if (!send || !context) throw Error('the conversation is not available')
      await send({ text: addSourceMessage(next) }, context)
      sent = true
    } catch {
      this.notify('add_unsent', next.host)
    }
    this.sending = false
    this.setAdds(this.snapshot.adds.map((add) => (add.key === next.key ? (sent ? { ...add, state: 'asked' } : { ...add, failed: true }) : add)))
    if (sent) this.schedule()
  }

  // ── Upgrade requests ──

  /**
   * The owner's answer to an upgrade request. "Upgrade" asks the Agent in the conversation (once) and then records
   * `accepted`; "Not now" records `dismissed`. False when it did not work: the request stays open.
   */
  async decide(id: string, decision: 'accepted' | 'dismissed'): Promise<boolean> {
    const upgrade = this.snapshot.site?.upgrades.find((item) => item.id === id)
    if (!upgrade) return false
    if (decision === 'accepted' && !this.sentUpgrades.has(id)) {
      try {
        const send = agentBrowserExtensions.askAgent
        const context = await this.store.whenConversation()
        if (!send || !context) throw Error('the conversation is not available')
        await send({ text: upgradeMessage({ id, file: upgrade.file, goalId: upgrade.goal_id }) }, context)
        this.sentUpgrades.add(id)
      } catch {
        this.notify('upgrade_unsent', upgrade.title)
        return false
      }
    }
    try {
      await decideUpgrade(await (await getReflowSource()).library(), id, decision)
    } catch {
      this.notify('upgrade_unsaved', upgrade.title)
      return false
    }
    this.set({ sheet: null })
    await this.load()
    return true
  }
}

const controllers = new WeakMap<AgentBrowserStore, SiteController>()

/** The personal site's controller of an App window. */
export function siteControllerOf(store: AgentBrowserStore): SiteController {
  let controller = controllers.get(store)
  if (!controller) {
    controller = new SiteController(store)
    controllers.set(store, controller)
  }
  return controller
}
