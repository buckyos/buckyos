/* Reflow mode (RF6, TODO §5.6): for the address in the bar, what the library has (`browser.reflow_state`) decides
 * what shows — the page that was made, a refresh of the site's package in the Agent's browser, or a request to the
 * Agent for a fallback page. One controller per App window. The address is kept per Agent: it starts as
 * the page the browser shows and then goes its own way (links of reflow pages, the bar), until the browser has
 * moved on when the user comes back to this mode. */

import { getBrowserBackend } from '../api/backend'
import { browserErrorCode } from '../api/types'
import { agentBrowserExtensions, type ConversationContext } from '../extensions'
import { toNavigableUrl } from '../shell/address'
import type { AgentBrowserStore } from '../store/store'
import { ReflowError, libraryState, normalizeUrl, readPreference, reflowAddress, shownMade, writePreference, type ReflowState } from './library'
import { askText, type AskTopic, type Translate } from './messages'
import { getReflowSource } from './source'

export type ReflowPhase =
  /** No address yet. */
  | 'blank'
  | 'loading'
  /** A page is shown. */
  | 'view'
  /** Nothing to show yet: the package is being refreshed (`job`), or the Agent was asked (`asked`). */
  | 'making'
  /** Nothing to show and nobody is making it (`reason`). */
  | 'unmade'
  /** The address has no reflow (not an http(s) page). */
  | 'unsupported'
  | 'failed'

export type ReflowNoticeKind = 'browser_unavailable' | 'refresh_failed' | 'action_failed' | 'agent_asked' | 'agent_unreachable' | 'preference_failed'
export interface ReflowNotice {
  id: number
  kind: ReflowNoticeKind
  /** The service's own sentence about a failure. */
  detail: string
  /** The failure was handed to the Agent. */
  asked: boolean
}

export interface ReflowSnapshot {
  /** The address being reflowed, as it was entered or taken from the browser; empty before there is one. */
  url: string
  canGoBack: boolean
  phase: ReflowPhase
  state: ReflowState | null
  /** The page the frame shows (a path of protocol §5.3) and the count that loads it again. */
  view: string | null
  frameKey: number
  /** What runs on the original page for this address. */
  job: 'refresh' | 'action' | null
  /** The Agent was asked about this address and has not delivered yet. */
  asked: boolean
  /** The library was read directly: the Agent's browser is not available, so nothing can be made. */
  offline: boolean
  /** Why nothing is being made (`unmade`). */
  reason: 'browser_unavailable' | 'agent_unreachable' | 'not_started' | null
  /** Why the last refresh failed. */
  failure: string | null
  /** The user's preference for the site of this address. */
  preference: string
  notice: ReflowNotice | null
}

/** What a reflow page tells the App (protocol §5.5). */
type PageMessage = { type?: unknown; url?: unknown; selector?: unknown }

interface Place {
  url: string
  history: string[]
  /** The browser's address this place last followed. */
  adopted: string
}

/** What the page showed when the Agent was asked: the request is met once it shows something else. */
interface Asked { view: string | null; madeAt: number }

const JOB_POLL_MS = 2000
const AGENT_POLL_MS = 4000
const CONTEXT_WAIT_MS = 6000
const LANDING_WAIT_MS = 20_000
const HISTORY_MAX = 50
const ASKED_KEY = 'buckyos.agent-browser.reflow.asked'

/** Requests to the Agent that are still open, by Agent, normalized address and subject; kept for the browser tab so a reload does not ask again. */
const asked = new Map<string, Asked>((() => {
  try { return JSON.parse(window.sessionStorage.getItem(ASKED_KEY) ?? '[]') as Array<[string, Asked]> } catch { return [] }
})())

function saveAsked() {
  try { window.sessionStorage.setItem(ASKED_KEY, JSON.stringify([...asked])) } catch { /* no storage */ }
}

const askedPrefix = (agentId: string, url: string) => `${agentId}\n${url}\n`
const isAsked = (agentId: string, url: string) => [...asked.keys()].some((key) => key.startsWith(askedPrefix(agentId, url)))

function sameAddress(a: string, b: string): boolean {
  if (a === b) return true
  try { return normalizeUrl(a).url === normalizeUrl(b).url } catch { return false }
}

/** The service's sentence in a `browser.*` error (the wire error is JSON inside the RPC error's message). */
function errorMessage(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error ?? '')
  const start = message.indexOf('{')
  if (start >= 0) {
    try {
      const wire = JSON.parse(message.slice(start)) as { message?: unknown }
      if (typeof wire.message === 'string') return wire.message
    } catch { /* not the wire form */ }
  }
  return message
}

const BLANK: ReflowSnapshot = {
  url: '', canGoBack: false, phase: 'blank', state: null, view: null, frameKey: 0, job: null, asked: false, offline: false, reason: null,
  failure: null, preference: '', notice: null,
}

export class ReflowController {
  private readonly store: AgentBrowserStore
  private snapshot: ReflowSnapshot = BLANK
  private readonly listeners = new Set<() => void>()
  private readonly places = new Map<string, Place>()
  private key = ''
  private active = false
  private t: Translate = (key, fallback) => fallback ?? key
  private stopStore: (() => void) | null = null
  private timer = 0
  private run = 0
  private noticeId = 0
  private shown: { view: string; madeAt: number } | null = null
  private selector = ''
  private post: ((message: { type: string }) => void) | null = null

  constructor(store: AgentBrowserStore) {
    this.store = store
  }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  getSnapshot = () => this.snapshot

  private set(patch: Partial<ReflowSnapshot>) {
    this.snapshot = { ...this.snapshot, ...patch }
    for (const listener of [...this.listeners]) listener()
  }

  private notify(kind: ReflowNoticeKind, detail = '', askedAgent = false) {
    this.set({ notice: { id: ++this.noticeId, kind, detail, asked: askedAgent } })
  }

  dismissNotice() {
    if (this.snapshot.notice) this.set({ notice: null })
  }

  // ── The address ──

  /** The language the Agent is written to in. */
  setTranslate(t: Translate) {
    this.t = t
  }

  /** The panel is shown: follow the store's Agent, and look up the address in the bar. */
  activate() {
    if (this.active) return
    this.active = true
    this.stopStore = this.store.subscribe(() => this.follow(false))
    this.follow(true)
  }

  deactivate() {
    this.active = false
    this.stopStore?.()
    this.stopStore = null
    window.clearTimeout(this.timer)
    this.run += 1
  }

  private place(): Place {
    let place = this.places.get(this.key)
    if (!place) {
      place = { url: '', history: [], adopted: '' }
      this.places.set(this.key, place)
    }
    return place
  }

  private follow(entered: boolean) {
    const { agent, state, group } = this.store.getSnapshot()
    if (!agent) return
    const key = agent.agentId
    const switched = key !== this.key
    this.key = key
    const place = this.place()
    // While a page is being read here the browser may go elsewhere (the Agent works in it): the address stays.
    if (!switched && !entered && place.url) return
    const browserUrl = [state?.url, group?.tabs.find((tab) => tab.active)?.url].find((url) => !!url && /^https?:\/\//i.test(url)) ?? ''
    let moved = false
    if (browserUrl && !sameAddress(browserUrl, place.adopted)) {
      place.adopted = browserUrl
      moved = !sameAddress(browserUrl, place.url)
      if (moved) this.move(place, browserUrl)
    }
    if (switched || entered || moved) void this.evaluate()
  }

  private move(place: Place, url: string) {
    if (place.url) place.history = [...place.history, place.url].slice(-HISTORY_MAX)
    place.url = url
  }

  /** The address bar: an address as it is, anything else a web search. False when there is nothing to open. */
  go(text: string): boolean {
    const url = toNavigableUrl(text)
    if (!url) return false
    this.open(url)
    return true
  }

  private open(url: string) {
    const place = this.place()
    if (!sameAddress(url, place.url)) this.move(place, url)
    void this.evaluate()
  }

  back() {
    const place = this.place()
    const url = place.history.pop()
    if (url === undefined) return
    place.url = url
    void this.evaluate()
  }

  retry() {
    void this.evaluate()
  }

  // ── What the library has ──

  private async call(method: string, params: Record<string, unknown>): Promise<ReflowState> {
    const { agent } = this.store.getSnapshot()
    if (!agent) throw Error('no Agent')
    return (await getBrowserBackend()).call<ReflowState>(agent, method, params)
  }

  /** `browser.reflow_state`; when the Agent's browser is not available the library itself is read, so what was made still shows. */
  private async readState(url: string): Promise<{ state: ReflowState; offline: boolean }> {
    try {
      return { state: await this.call('browser.reflow_state', { url }), offline: false }
    } catch (error) {
      if (browserErrorCode(error) !== 'browser_unavailable') throw error
      const library = await (await getReflowSource()).library()
      return { state: { ...await libraryState(library, await reflowAddress(url)), refresh: { status: 'idle', error: null, at_ms: null } }, offline: true }
    }
  }

  private async evaluate() {
    const run = ++this.run
    window.clearTimeout(this.timer)
    this.shown = null
    const place = this.place()
    const cleared = { url: place.url, canGoBack: place.history.length > 0, state: null, view: null, job: null, asked: false, offline: false, reason: null, failure: null, preference: '', notice: null }
    if (!place.url) { this.set({ ...cleared, phase: 'blank' }); return }
    this.set({ ...cleared, phase: 'loading' })
    try {
      const { state, offline } = await this.readState(place.url)
      if (run !== this.run) return
      const preference = state.package ? state.package.preference : await readPreference(await (await getReflowSource()).library(), state.site).catch(() => '')
      if (run !== this.run) return
      this.set({ preference, offline })
      this.apply(state)
      if (state.view || state.refresh.status === 'running') this.schedule()
      else if (offline) this.set({ phase: 'unmade', reason: 'browser_unavailable' })
      else if (state.package) await this.refresh()
      else if (await this.ask('fallback') === 'unreachable' && run === this.run) this.set({ phase: 'unmade', reason: 'agent_unreachable' })
    } catch (error) {
      if (run !== this.run) return
      this.set({ phase: error instanceof ReflowError || browserErrorCode(error) === 'invalid_argument' ? 'unsupported' : 'failed' })
    }
  }

  /** Takes a new state of the address: shows what it has, tells the page when its result changed, and settles what was waited for. */
  private apply(state: ReflowState) {
    const before = this.snapshot
    const agentId = this.store.getSnapshot().agent?.agentId ?? ''
    const view = state.view
    const madeAt = view ? shownMade(state)?.made_at ?? 0 : 0
    const running = state.refresh.status === 'running'
    const ended = before.job !== null && !running
    let frameKey = before.frameKey
    if (view) {
      if (this.shown?.view !== view) frameKey += 1
      else if (this.shown.madeAt !== madeAt || (ended && state.refresh.status === 'idle')) {
        // A package's page reads its result again; a fallback page is a new document.
        if (state.package && state.result) this.post?.({ type: 'reflow.updated' })
        else frameKey += 1
      }
      this.shown = { view, madeAt }
      let settled = false
      for (const [key, was] of asked) {
        if (key.startsWith(askedPrefix(agentId, state.url)) && (was.view !== view || was.madeAt !== madeAt)) settled = asked.delete(key)
      }
      if (settled) saveAsked()
    } else {
      this.shown = null
    }
    const job = running ? before.job ?? 'refresh' : null
    const waiting = isAsked(agentId, state.url)
    this.set({ state, view, frameKey, job, asked: waiting, phase: view ? 'view' : job || waiting ? 'making' : before.phase })
    if (ended && (state.refresh.status === 'failed' || !view)) void this.jobFailed(before.job ?? 'refresh', state.refresh.error ?? '')
  }

  private schedule() {
    window.clearTimeout(this.timer)
    const { job, asked: waiting, offline } = this.snapshot
    if (!this.active || offline || (!job && !waiting)) return
    this.timer = window.setTimeout(() => void this.poll(), job ? JOB_POLL_MS : AGENT_POLL_MS)
  }

  private async poll() {
    const run = this.run
    const url = this.snapshot.state?.url
    if (!url) return
    try {
      const state = await this.call('browser.reflow_state', { url })
      if (run !== this.run) return
      this.apply(state)
    } catch {
      /* the service did not answer: asked again at the next turn */
    }
    if (run === this.run) this.schedule()
  }

  // ── Making it ──

  /** Makes the page again: the site's package is refreshed in the Agent's browser; a page without a package is the Agent's to make. */
  async refresh() {
    const { state, job, offline, view } = this.snapshot
    if (!state || job) return
    if (offline) { this.unavailable(); return }
    const run = this.run
    this.set({ job: 'refresh', notice: null, reason: null, phase: view ? 'view' : 'making' })
    try {
      const next = await this.call('browser.reflow_refresh', { url: state.url })
      if (run !== this.run) return
      this.apply(next)
      this.schedule()
    } catch (error) {
      if (run !== this.run) return
      this.set({ job: null })
      const code = browserErrorCode(error)
      if (code === 'browser_unavailable') this.unavailable()
      else if (code === 'reflow_no_package') {
        const outcome = await this.ask(view ? 'redo' : 'fallback')
        if (run !== this.run) return
        if (outcome === 'unreachable') this.unreachable()
        else if (view) this.notify('agent_asked')
      } else if (view) this.notify('refresh_failed', errorMessage(error))
      // The refresh did not start (the service did not take it): that is not the package's fault, so the Agent is not asked.
      else this.set({ phase: 'unmade', reason: 'not_started', failure: errorMessage(error) })
    }
  }

  /** A control of the reflow page that only the original page can carry out: the Agent's browser presses it there and makes the page again. */
  async action(selector: string) {
    const { state, job, offline, view } = this.snapshot
    if (!state || !view || job) return
    if (offline) { this.notify('browser_unavailable'); return }
    const run = this.run
    this.selector = selector
    this.set({ job: 'action', notice: null })
    try {
      const next = await this.call('browser.reflow_action', { url: state.url, selector })
      if (run !== this.run) return
      this.apply(next)
      this.schedule()
    } catch (error) {
      if (run !== this.run) return
      this.set({ job: null })
      const code = browserErrorCode(error)
      if (code === 'browser_unavailable') this.notify('browser_unavailable')
      else if (code === null) this.notify('action_failed', errorMessage(error))
      else await this.jobFailed('action', errorMessage(error))
    }
  }

  private unavailable() {
    if (this.snapshot.view) this.notify('browser_unavailable')
    else this.set({ phase: 'unmade', reason: 'browser_unavailable' })
  }

  private unreachable() {
    if (this.snapshot.view) this.notify('agent_unreachable')
    else this.set({ phase: 'unmade', reason: 'agent_unreachable' })
  }

  /** A refresh or an action did not make the page: the Agent takes it from here (a fallback page, and the package repaired). */
  private async jobFailed(job: 'refresh' | 'action', error: string) {
    const run = this.run
    this.set({ failure: job === 'refresh' ? error || null : this.snapshot.failure })
    const outcome = await this.ask(job === 'action' ? 'action' : 'refresh_failed', error)
    if (run !== this.run) return
    if (this.snapshot.view) this.notify(job === 'action' ? 'action_failed' : 'refresh_failed', error, outcome !== 'unreachable')
    else if (outcome === 'unreachable') this.set({ phase: 'unmade', reason: 'agent_unreachable' })
  }

  // ── The Agent ──

  /** The conversation side comes up a moment after the App opens. */
  private conversationContext(): Promise<ConversationContext | null> {
    const started = Date.now()
    return new Promise((resolve) => {
      const look = () => {
        const context = this.store.conversationContext()
        if (context || Date.now() - started >= CONTEXT_WAIT_MS) resolve(context)
        else window.setTimeout(look, 200)
      }
      look()
    })
  }

  /** Writes to the Agent in the browser conversation about this address, once per subject until the page changes. */
  private async ask(topic: AskTopic, reason = '', again = false): Promise<'asked' | 'waiting' | 'unreachable'> {
    const { state, preference } = this.snapshot
    const agent = this.store.getSnapshot().agent
    const send = agentBrowserExtensions.askAgent
    if (!state || !agent) return 'unreachable'
    const key = `${askedPrefix(agent.agentId, state.url)}${topic === 'action' ? `action ${this.selector}` : 'page'}`
    const run = this.run
    let outcome: 'asked' | 'waiting' = 'waiting'
    if (again || !asked.has(key)) {
      if (!send) return 'unreachable'
      asked.set(key, this.shown ?? { view: null, madeAt: 0 })
      try {
        const context = await this.conversationContext()
        if (!context) throw Error('the conversation is not available')
        if (run !== this.run && (topic === 'fallback' || topic === 'redo')) {
          asked.delete(key)
          return outcome
        }
        const text = askText(this.t, topic, { url: state.url, site: state.site, host: new URL(state.url).hostname, preference, reason, selector: this.selector })
        await send({ text }, context)
      } catch {
        asked.delete(key)
        return 'unreachable'
      }
      saveAsked()
      outcome = 'asked'
    }
    if (run !== this.run) return outcome
    this.set({ asked: true, reason: null, phase: this.snapshot.view ? 'view' : 'making' })
    this.schedule()
    return outcome
  }

  /** The user asks once more for a page that is still not there. */
  async askAgain() {
    const run = this.run
    const outcome = await this.ask(this.snapshot.failure ? 'refresh_failed' : 'fallback', this.snapshot.failure ?? '', true)
    if (run === this.run && outcome === 'unreachable') this.unreachable()
  }

  // ── Preference ──

  /** Keeps the user's preference for this site and tells the Agent, who makes the site's pages by it. */
  async savePreference(text: string): Promise<boolean> {
    const { state, offline } = this.snapshot
    if (!state) return false
    const preference = text.trim()
    if (preference === this.snapshot.preference) return true
    const run = this.run
    try {
      await writePreference(await (await getReflowSource()).library(), state.site, preference)
    } catch {
      if (run === this.run) this.notify('preference_failed')
      return false
    }
    if (run !== this.run) return true
    this.set({ preference })
    if (preference && !offline) {
      void this.ask('preference', '', true).then((outcome) => {
        if (run === this.run) this.notify(outcome === 'unreachable' ? 'agent_unreachable' : 'agent_asked')
      })
    }
    return true
  }

  /** The library changed under the page (a package was imported): look the address up again. */
  libraryChanged(site: string) {
    if (this.active && this.snapshot.state?.site === site) void this.evaluate()
  }

  // ── The page ──

  /** The frame's way to the page it shows, while it shows one. */
  setPage(post: ((message: { type: string }) => void) | null) {
    this.post = post
  }

  /** A message of the page in the frame; the frame has checked where it came from. */
  onPageMessage(data: unknown) {
    const message = data as PageMessage | null
    if (!message || typeof message !== 'object') return
    switch (message.type) {
      case 'reflow.open':
        if (typeof message.url === 'string') this.openLink(message.url)
        break
      case 'reflow.action':
        if (typeof message.selector === 'string' && message.selector) void this.action(message.selector)
        break
      case 'reflow.refresh':
        void this.refresh()
        break
      case 'reflow.source':
        this.openSource()
        break
    }
  }

  /** A link of the page: an http(s) address, a relative one against the original page. */
  private openLink(link: string) {
    let url: URL
    try {
      url = new URL(link, this.snapshot.state?.url)
    } catch {
      return
    }
    if (url.protocol === 'http:' || url.protocol === 'https:') this.open(url.href)
  }

  /** Back to the original page: the browser opens what this page was made from. */
  openSource() {
    const store = this.store
    const { state } = this.snapshot
    const url = (state && shownMade(state)?.source_url) || state?.url || this.snapshot.url
    const place = this.place()
    const { agent } = store.getSnapshot()
    store.setMode('page')
    if (!url) return
    place.adopted = url
    let sent = false
    let loading = false
    const finish = () => { stop(); window.clearTimeout(timer) }
    const look = () => {
      const now = store.getSnapshot()
      if (now.mode !== 'page' || now.agent !== agent) { finish(); return }
      if (now.screenStatus !== 'open' || !now.state) return
      if (!sent) {
        sent = true
        if (sameAddress(now.state.url, url) || !store.navigate(url)) finish()
        return
      }
      // Where the site took the browser (a redirect) is still this page: coming back to reflow keeps it.
      if (now.state.loading) loading = true
      else if (loading && now.state.url) { place.adopted = now.state.url; finish() }
    }
    const stop = store.subscribe(look)
    const timer = window.setTimeout(finish, LANDING_WAIT_MS)
    look()
  }
}

const controllers = new WeakMap<AgentBrowserStore, ReflowController>()

/** The reflow controller of an App window. */
export function reflowControllerOf(store: AgentBrowserStore): ReflowController {
  let controller = controllers.get(store)
  if (!controller) {
    controller = new ReflowController(store)
    controllers.set(store, controller)
  }
  return controller
}
